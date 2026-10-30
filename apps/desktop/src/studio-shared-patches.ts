import type { JamPatch } from "./jam-contract.ts";
import { JAM_PATCH_FILE_MAX_BYTES } from "./jam-contract.ts";
import { decodeJamPatchBase64, jamPatchFileName, parseJamPatchJson } from "./jam-patch.ts";

export const STUDIO_SHARED_PATCH_LIMIT = 16;
export type SharedPatchCandidate = Readonly<{ cid: string; name: string; binding: string }>;
type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;
type Read = { cancellation: string; ready: Promise<void>; registered: boolean; cancelled: boolean };

/** One serial, cancellable shelf read. Eligibility is supplied by the current share/trust policy;
 * each change revokes old continuations, including server A→B→A and lock/unlock. Retained recipes
 * are always a subset of the current sixteen eligible candidates, never a growing CID cache.
 *
 * Reading can be paused without forgetting (`active: false`): leaving the Studio surface stops
 * the reads but keeps what was already read for the same scope, so coming back to the same channel
 * does not download every recipe again. Any scope change (channel, server, membership, lock) still
 * clears them, exactly as before. */
export class StudioSharedPatches {
  private generation = 0;
  private key = "";
  private server: number | null = null;
  private scope: string | null = null;
  private candidates: readonly SharedPatchCandidate[] = [];
  private values = new Map<string, { name: string; patch: JamPatch }>();
  private reading = false;
  private read: Read | null = null;
  private sequence = 0;
  private active = true;
  private readonly invoke: Invoke;
  private readonly nonce: string;
  private readonly changed: (values: ReadonlyMap<string, { name: string; patch: JamPatch }>) => void;
  private readonly allowed: (server: number, file: SharedPatchCandidate, scope: string | null) => boolean;
  constructor(invoke: Invoke, nonce: string, changed: (values: ReadonlyMap<string, { name: string; patch: JamPatch }>) => void,
    allowed: (server: number, file: SharedPatchCandidate, scope: string | null) => boolean = () => true) {
    this.invoke = invoke; this.nonce = nonce; this.changed = changed;
    this.allowed = allowed;
  }

  update(scope: string | null, server: number | null, candidates: readonly SharedPatchCandidate[], active = true): void {
    const next = scope === null || server === null ? [] : candidates.slice(0, STUDIO_SHARED_PATCH_LIMIT);
    const key = JSON.stringify([scope, server, next]);
    if (key === this.key) { this.setActive(active); return; }
    const sameScope = this.server === server && this.scope === scope;
    const previous = new Map(this.candidates.map(f => [f.cid, f]));
    this.key = key; this.scope = scope; this.server = server; this.candidates = next; this.generation++;
    this.active = active;
    if (this.read) this.cancel(this.read);
    const eligible = new Map(next.map(f => [f.cid, f]));
    this.values = new Map(sameScope ? [...this.values].filter(([cid]) => {
      const old = previous.get(cid), current = eligible.get(cid);
      return old && current && old.binding === current.binding && old.name === current.name;
    }) : []);
    this.changed(this.values);
    void this.drain();
  }

  /** Pause or resume reading for the same scope and candidates; held values are kept either way. */
  private setActive(active: boolean): void {
    if (active === this.active) return;
    this.active = active;
    if (!active) {
      // Retire the running loop (it checks the generation after every await) and its native read.
      this.generation++;
      if (this.read) this.cancel(this.read);
      return;
    }
    void this.drain();
  }

  private cancel(read: Read): void {
    if (read.cancelled) return;
    read.cancelled = true;
    // Cancellation cannot race ahead of native registration. At most one such continuation
    // belongs to the active read, even if the listing changes repeatedly while begin is pending.
    void read.ready.then(async () => {
      if (read.registered) await this.invoke("cancel_inline_download", { cancellation: read.cancellation });
    }).catch(() => {});
  }

  private async drain(): Promise<void> {
    if (this.reading || this.server === null || !this.active) return;
    this.reading = true;
    const generation = this.generation, server = this.server, scope = this.scope, candidates = this.candidates;
    const current = () => this.generation === generation && this.server === server;
    try {
      for (const file of candidates) {
        const currentFile = () => current() && this.allowed(server, file, scope);
        if (!currentFile()) return;
        if (this.values.has(file.cid)) continue;
        let ready!: () => void;
        const read: Read = { cancellation: `studio-patch:${this.nonce}:${++this.sequence}`, ready: new Promise(resolve => { ready = resolve; }), registered: false, cancelled: false };
        this.read = read;
        try {
          try { await this.invoke("begin_inline_download", { cancellation: read.cancellation }); read.registered = true; }
          finally { ready(); }
          if (!currentFile()) return;
          const base64 = await this.invoke<string>("download_file", { server, cid: file.cid, cancellation: read.cancellation, maxBytes: JAM_PATCH_FILE_MAX_BYTES });
          if (!currentFile()) return;
          const text = decodeJamPatchBase64(base64), checked = text === null ? null : parseJamPatchJson(text);
          if (checked?.ok) {
            this.values = new Map(this.values).set(file.cid, { name: jamPatchFileName(file.name), patch: checked.patch });
            this.changed(this.values);
          }
        } catch { /* Invalid/unavailable files are omitted; a listing change can retry them. */ }
        finally {
          ready();
          if (read.registered) await this.invoke("cancel_inline_download", { cancellation: read.cancellation }).catch(() => {});
          if (this.read === read) this.read = null;
        }
      }
    } finally {
      this.reading = false;
      // A replacement cannot allocate another read until the old one actually releases custody.
      if (generation !== this.generation && this.candidates.length) void this.drain();
    }
  }
}
