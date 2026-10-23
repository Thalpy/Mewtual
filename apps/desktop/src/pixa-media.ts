import { decodePixa, PIXA_MAX_BYTES, type Pixa } from "./pixa.ts";
import { JamTakeLoadCoordinator } from "./jam-playback.ts";

export const PIXA_MIME = "application/x-mewtual-pixa";
export const PIXA_EXT = ".pixa";

/** Listing hints select a decoder; only the complete package decoder admits content. */
export function mayFetchPixa(size: number): boolean {
  return Number.isSafeInteger(size) && size > 0 && size <= PIXA_MAX_BYTES;
}

/** Check both transport and decoded bounds before parsing peer-controlled framing. */
export function decodePixaBase64(encoded: unknown): Pixa {
  if (typeof encoded !== "string" || encoded.length > Math.ceil(PIXA_MAX_BYTES / 3) * 4) throw new Error("animation exceeds its encoded size limit");
  const binary = atob(encoded);
  if (!mayFetchPixa(binary.length)) throw new Error("animation exceeds its decoded size limit");
  return decodePixa(Uint8Array.from(binary, char => char.charCodeAt(0)));
}

/** Loop pictures independently of a long authored sound tail; one-shot decks hold the end. */
export function pixaFrameAt(pixa: Pixa, elapsedMs: number, loop = false): number {
  return animationFrameAt(pixa.frames.length, pixa.fps, elapsedMs, loop);
}

/** Shared frame arithmetic for the editor and package viewers; sound owns its longer horizon. */
export function animationFrameAt(frameCount: number, fps: number, elapsedMs: number, loop: boolean): number {
  const frame = Math.floor(Math.max(0, Number.isFinite(elapsedMs) ? elapsedMs : 0) * fps / 1000);
  return loop ? frame % frameCount : Math.min(frameCount - 1, frame);
}

/** Two bounded encoded packages per call; expanded rasters are never cached. */
export class PixaMediaCache {
  private readonly values = new Map<string, Pixa>();
  get(cid: string): Pixa | undefined {
    const value = this.values.get(cid);
    if (value) { this.values.delete(cid); this.values.set(cid, value); }
    return value;
  }
  set(cid: string, value: Pixa): void {
    this.values.delete(cid); this.values.set(cid, value);
    while (this.values.size > 2) this.values.delete(this.values.keys().next().value!);
  }
  clear(): void { this.values.clear(); }
}

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;
/**
 * Global file viewer's one active/one replacement read. Cancel waits for registration so an
 * immediate close cannot race ahead of begin_inline_download and strand a native permit.
 * The native command independently enforces the requested cap against its authenticated plan.
 */
export class PixaFileReader {
  private readonly loads = new JamTakeLoadCoordinator<Pixa | null>();
  private sequence = 0;
  private readonly invoke: Invoke;
  private readonly nonce: string;
  constructor(invoke: Invoke, nonce: string) { this.invoke = invoke; this.nonce = nonce; }
  cancel(): void { this.loads.invalidate(); }
  read(server: number, cid: string, size: number, current: () => boolean): Promise<Pixa | null> {
    this.cancel();
    if (!mayFetchPixa(size)) return Promise.reject(new Error("animation must fit within 9 MiB"));
    const cancellation = `pixa:${this.nonce}:${++this.sequence}`;
    let registered = false, ready!: () => void;
    const registration = new Promise<void>(resolve => { ready = resolve; });
    return this.loads.submit(`${server}:${cid}:${this.sequence}`, async owned => {
      try {
        if (!owned() || !current()) return null;
        try { await this.invoke("begin_inline_download", { cancellation }); registered = true; }
        finally { ready(); }
        if (!owned() || !current()) return null;
        const encoded = await this.invoke<string>("download_file", { server, cid, cancellation, maxBytes: PIXA_MAX_BYTES });
        return owned() && current() ? decodePixaBase64(encoded) : null;
      } finally {
        ready();
        if (registered) await this.invoke("cancel_inline_download", { cancellation }).catch(() => {});
      }
    }, async () => { await registration; await this.invoke("cancel_inline_download", { cancellation }); });
  }
}
