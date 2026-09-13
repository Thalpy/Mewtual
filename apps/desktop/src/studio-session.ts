// One channel's connected Studio: the Index view, the open flipnote, the pixels held for it, the
// saves in flight and the recovery rail, all read through the typed native adapter. Plain data
// plus a change signal, so the two surfaces re-read it the way they read the old fixture, and the
// unit tests drive it with a fake bridge.
//
// Rules this class exists to keep (docs/FLIPNOTE-UI-HOOKS.md):
// - A save is one complete request. A retry resends exactly it, nonce and epoch included; only an
//   explicit "save again as a new edit" mints a new nonce against the current epoch. Recovery
//   applies are saves too: their echoed preview payload is retained and retried the same way.
// - Saves to one document run in order. An uncertain save blocks the later saves of its lane
//   until it is retried or discarded on purpose; other documents keep moving.
// - Unsaved pixels survive every failure. They stay on the record until the save lands or the
//   member discards them on purpose.
// - Every asynchronous result is fenced by scope generation (server, channel, session) and by a
//   per-target request generation; a late result is dropped even after IPC delivered it, and a
//   landed write supersedes the reads that were issued before it.
// - A preview (`awaitingTenureReceipt`) is read-only: its epoch never authorizes an edit.
// - One fetch scheduler: thumbnails, the open frame, conflict actions and recovery all go through
//   the same bounded, prioritized, deduplicated queue. A cid that was unavailable or failed
//   transiently is asked again only after an invalidation or an explicit retry; content the
//   decoder rejected is kept apart from that.
// - State the surfaces read is replaced, never mutated in place, so a revision bridge sees a new
//   reference whenever something changed.

import { PixError, decodePix } from "./pix.ts";
import { canonicalJson, randomElementId, type FlipnoteOp, type IndexOp } from "./studio-contract.ts";
import {
  DEFAULT_FPS,
  SETTLEMENT_CHANGED_EVENT,
  STUDIO_RECEIVE_PAUSED_EVENT,
  STUDIO_UPDATED_EVENT,
  StudioNativeError,
  expiryForBody,
  flipnoteModel,
  indexModel,
  parseReceivePaused,
  parseSettlementChanged,
  parseStudioUpdated,
  publishPix,
  recoveryAcknowledge,
  recoveryApply,
  recoveryExport,
  recoveryList,
  recoveryPreview,
  recoveryRead,
  recoveryRestorePointer,
  requestBlobBounded,
  studioApply,
  studioApplyIndex,
  studioCreate,
  studioList,
  studioRead,
  type ApplyIndexRequest,
  type ApplyRequest,
  type CreateRequest,
  type Decimal,
  type FlipnoteModel,
  type FlipnoteView,
  type FrameConflictValue,
  type Hex32,
  type Hex64,
  type IndexModel,
  type IndexView,
  type NativeExpiry,
  type PixPublication,
  type PointerRestored,
  type RecoveryApplied,
  type RecoveryApplyEdit,
  type RecoveryChoice,
  type RecoveryDisposition,
  type RecoveryExport,
  type RecoveryListing,
  type RecoveryMode,
  type RecoveryVersionRead,
  type StudioIpc,
  type StudioPhase,
  type StudioUnlisten,
} from "./studio-native.ts";

export type StudioScope = { server: number; channel: Decimal };

export type SaveStatus = "queued" | "inflight" | "uncertain";
export type SaveOutcome = "landed" | "uncertain" | "discarded";
type SaveBase = {
  id: number;
  label: string;
  status: SaveStatus;
  attempts: number;
  error: string;
  /// Set when a later read of the same document carries a different epoch than the frozen
  /// request: the backend decides on retry; the member may instead re-author explicitly.
  epochChanged: boolean;
  /// Resolved once, when the record lands, goes uncertain or is discarded (used by callers
  /// that await one save, such as the recovery walk).
  settle?: (outcome: SaveOutcome) => void;
};
export type SaveRecord = SaveBase &
  (
    | { kind: "create"; request: CreateRequest; object: Hex32 }
    | { kind: "frame"; object: Hex32; frame: Hex32; op: "insert" | "replace"; after: Hex32 | null; pix: Uint8Array; published: PixPublication | null; apply: ApplyRequest | null; force?: boolean }
    | { kind: "apply"; object: Hex32; request: ApplyRequest; op: FlipnoteOp }
    | { kind: "applyIndex"; request: ApplyIndexRequest; op: IndexOp; object: Hex32 }
    | { kind: "recoveryApply"; object: Hex32 | null; edit: RecoveryApplyEdit; applied: RecoveryApplied | null }
  );

/// held: decoded bytes are cached. fetching/queued: a job for this scope exists. unavailable:
/// the backend answered null. failed: the request itself failed (busy, transport); retryable.
/// invalid: the body failed the length or PIX1 check; content, not availability. idle: not asked.
export type BlobState = "held" | "fetching" | "queued" | "unavailable" | "failed" | "invalid" | "idle";

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;
type NewSave = DistributiveOmit<SaveRecord, "id" | "status" | "attempts" | "error" | "epochChanged">;

export type RecoveryItemState = "pending" | "previewing" | "fetching" | "applying" | "applied" | "alreadySaved" | "uncertain" | RecoveryDisposition | "error";
export type RecoveryItem = {
  choice: RecoveryChoice;
  label: string;
  state: RecoveryItemState;
  error: string;
  originalAuthor: Hex64 | null;
};
export type RecoveryRun = {
  snapshot: Hex64;
  mode: RecoveryMode;
  object: Hex32 | null;
  items: RecoveryItem[];
  status: "running" | "done" | "stopped";
  error: string;
};
export type PointerStep = { status: "idle" | "running" | "done" | "blocked"; error: string; result: PointerRestored | null };

export type DocState = {
  object: Hex32;
  view: FlipnoteView | null;
  model: FlipnoteModel | null;
  loading: boolean;
  /// `studio_read` answered null: local absence, not a deletion or proof nobody has it.
  absent: boolean;
  error: string;
};

export type KnownView = { awaiting: boolean; phase: StudioPhase | null; epoch: Decimal };

export type SessionOptions = {
  ipc: StudioIpc;
  me: Hex64;
  now?: () => number;
  newId?: () => Hex32;
  /// Coalescing timer seam; returns a cancel function.
  schedule?: (fn: () => void, ms: number) => () => void;
  maxFetches?: number;
  blobCacheBytes?: number;
};

const REFRESH_COALESCE_MS = 120;
const BLOB_CACHE_BYTES = 8 * 1024 * 1024;
const BLOB_CACHE_ENTRIES = 512;
const RECENT_VIEWS = 8;

type BlobJob = {
  key: string;
  cid: Hex64;
  bytes: number;
  priority: number;
  generation: number;
  started: boolean;
  waiters: { resolve: (bytes: Uint8Array | null) => void; reject: (e: unknown) => void }[];
};

export class StudioSession {
  readonly ipc: StudioIpc;
  me: Hex64;
  readonly now: () => number;
  readonly newId: () => Hex32;
  private readonly schedule: (fn: () => void, ms: number) => () => void;

  scope: StudioScope | null = null;
  /// Bumped by every scope change and every reset; captured before each await.
  generation = 0;

  index: IndexView | null = null;
  indexModel: IndexModel | null = null;
  indexLoading = false;
  indexError = "";
  doc: DocState | null = null;
  /// Trust state of documents this session has read, for sidebar chips; never a phase claim
  /// for a document that was not read.
  readonly known = new Map<Hex32, KnownView>();
  /// The last view of each recently read document, so a queued save still has its epoch after
  /// the member moves to another flipnote. Bounded; the open document is always current.
  private readonly views = new Map<Hex32, { view: FlipnoteView; model: FlipnoteModel }>();

  readonly saves: SaveRecord[] = [];
  private saveSeq = 0;
  /// The token of the worker that owns the save lane right now (0 = none). Only that worker
  /// may release it; a worker from a cleared scope finds a different token and stands down.
  private saving = 0;
  private workerSeq = 0;

  receivePaused = false;
  readonly listeners = new Set<() => void>();

  // Blobs
  private readonly maxFetches: number;
  private readonly cacheBytesCap: number;
  private readonly blobs = new Map<Hex64, Uint8Array>();
  private cacheBytes = 0;
  private readonly jobs = new Map<string, BlobJob>();
  private inflight = 0;
  private readonly unavailable = new Set<Hex64>();
  private readonly failed = new Map<Hex64, string>();
  private readonly invalid = new Map<Hex64, string>();

  // Recovery
  recoveryTarget: Hex32 | null = null;
  recoveryListing: RecoveryListing | null = null;
  recoveryLoading = false;
  recoveryError = "";
  readonly versions = new Map<Hex64, RecoveryVersionRead>();
  readonly exports = new Map<Hex64, RecoveryExport>();
  /// An immutable snapshot of the current walk; replaced on every change.
  recoveryRun: RecoveryRun | null = null;
  pointer: PointerStep = { status: "idle", error: "", result: null };
  private activeRun: { token: number; stop: boolean } | null = null;
  private runSeq = 0;

  private indexReq = 0;
  private readonly docReq = new Map<Hex32, number>();
  private recoveryReq = 0;
  private pendingIndex = false;
  private readonly pendingObjects = new Set<Hex32>();
  private pendingRecovery = false;
  private cancelFlush: (() => void) | null = null;
  private unlisten: StudioUnlisten[] = [];

  constructor(opts: SessionOptions) {
    this.ipc = opts.ipc;
    this.me = opts.me;
    this.now = opts.now ?? (() => Date.now());
    this.newId = opts.newId ?? (() => randomElementId());
    this.schedule = opts.schedule ?? ((fn, ms) => { const t = setTimeout(fn, ms); return () => clearTimeout(t); });
    this.maxFetches = opts.maxFetches ?? 2;
    this.cacheBytesCap = opts.blobCacheBytes ?? BLOB_CACHE_BYTES;
  }

  // --- Change signal -----------------------------------------------------------------------

  onChange(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  private notify(): void {
    for (const fn of this.listeners) fn();
  }

  // --- Scope -------------------------------------------------------------------------------

  /// Point the session at a channel. Everything scoped to the previous one is dropped, including
  /// pending saves: their identity belongs to that channel and a retry there needs its scope back.
  setScope(scope: StudioScope | null): void {
    if (this.scope && scope && this.scope.server === scope.server && this.scope.channel === scope.channel) return;
    if (!this.scope && !scope) return;
    this.clearScoped();
    this.scope = scope;
    this.generation++;
    this.notify();
    if (scope) this.invalidate({ index: true });
  }

  /// Lock or session change: forget everything, including cached pixels and unsaved work.
  reset(): void {
    this.clearScoped();
    this.scope = null;
    this.generation++;
    this.blobs.clear();
    this.cacheBytes = 0;
    this.notify();
  }

  private clearScoped(): void {
    this.index = null;
    this.indexModel = null;
    this.indexLoading = false;
    this.indexError = "";
    this.doc = null;
    this.known.clear();
    this.views.clear();
    for (const s of this.saves) s.settle?.("discarded");
    this.saves.length = 0;
    this.saving = 0;
    this.receivePaused = false;
    // Queued jobs are dropped; started ones keep their slot until they actually finish.
    for (const [key, job] of this.jobs) {
      if (job.started) continue;
      for (const w of job.waiters) w.resolve(null);
      this.jobs.delete(key);
    }
    this.unavailable.clear();
    this.failed.clear();
    this.invalid.clear();
    this.recoveryTarget = null;
    this.recoveryListing = null;
    this.recoveryLoading = false;
    this.recoveryError = "";
    this.versions.clear();
    this.exports.clear();
    this.recoveryRun = null;
    this.activeRun = null;
    this.pointer = { status: "idle", error: "", result: null };
    this.pendingIndex = false;
    this.pendingObjects.clear();
    this.pendingRecovery = false;
    if (this.cancelFlush) { this.cancelFlush(); this.cancelFlush = null; }
  }

  private current(gen: number): boolean {
    return gen === this.generation && this.scope !== null;
  }
  private target(): StudioScope {
    if (!this.scope) throw new StudioNativeError("no channel selected");
    return this.scope;
  }

  // --- Events --------------------------------------------------------------------------------

  /// Install the three native listeners once; the returned function removes them.
  async attach(): Promise<() => void> {
    const handles = await Promise.all([
      this.ipc.listen<unknown>(STUDIO_UPDATED_EVENT, (e) => this.onStudioUpdated(e.payload)),
      this.ipc.listen<unknown>(STUDIO_RECEIVE_PAUSED_EVENT, (e) => this.onReceivePaused(e.payload)),
      this.ipc.listen<unknown>(SETTLEMENT_CHANGED_EVENT, (e) => this.onSettlementChanged(e.payload)),
    ]);
    this.unlisten.push(...handles);
    return () => this.detach();
  }
  detach(): void {
    for (const u of this.unlisten) u();
    this.unlisten = [];
  }

  onStudioUpdated(payload: unknown): void {
    let ev;
    try { ev = parseStudioUpdated(payload); } catch { return; }
    if (!this.scope || ev.server !== this.scope.server || ev.channel !== this.scope.channel) return;
    // Every event invalidates the Index; a named object additionally invalidates that object.
    // A blob that was unavailable or failed may have arrived with it: let the next request try.
    if (ev.object) this.releaseTransientFor(ev.object);
    this.invalidate({ index: true, objects: ev.object ? [ev.object] : [] });
  }
  onReceivePaused(payload: unknown): void {
    let ev;
    try { ev = parseReceivePaused(payload); } catch { return; }
    if (!this.scope || ev.server !== this.scope.server) return;
    this.receivePaused = true;
    this.notify();
  }
  onSettlementChanged(payload: unknown): void {
    let ev;
    try { ev = parseSettlementChanged(payload); } catch { return; }
    if (!this.scope || ev.server !== this.scope.server || ev.channel !== this.scope.channel) return;
    // Phase and recovery observations are independent, and neither is a projection: re-read
    // the affected document's actual view and the actual recovery listing.
    const objects = ev.object ? [ev.object] : [];
    this.invalidate({ index: ev.object === null, objects, recovery: ev.object === this.recoveryTarget });
  }

  // --- Coalesced refresh ---------------------------------------------------------------------

  invalidate(what: { index?: boolean; objects?: Hex32[]; recovery?: boolean }): void {
    if (what.index) this.pendingIndex = true;
    for (const o of what.objects ?? []) this.pendingObjects.add(o);
    if (what.recovery) this.pendingRecovery = true;
    if (!this.cancelFlush) this.cancelFlush = this.schedule(() => this.flush(), REFRESH_COALESCE_MS);
  }

  /// Run the coalesced re-reads now (the timer calls this; tests call it directly).
  flush(): void {
    if (this.cancelFlush) { this.cancelFlush(); this.cancelFlush = null; }
    if (!this.scope) { this.pendingIndex = false; this.pendingObjects.clear(); this.pendingRecovery = false; return; }
    if (this.pendingIndex) { this.pendingIndex = false; void this.refreshIndex(); }
    for (const o of this.pendingObjects) {
      if (this.doc?.object === o) void this.refreshDoc(o);
      else this.known.delete(o); // its trust state is no longer known until read again
    }
    this.pendingObjects.clear();
    if (this.pendingRecovery) { this.pendingRecovery = false; void this.refreshRecovery(); }
  }

  async refreshIndex(): Promise<void> {
    const gen = this.generation;
    const req = ++this.indexReq;
    const target = this.target();
    this.indexLoading = true;
    this.notify();
    try {
      const view = await studioList(this.ipc, target);
      if (!this.current(gen) || req !== this.indexReq) return;
      this.index = view;
      this.indexModel = indexModel(view.content);
      this.indexError = "";
      this.receivePaused = false; // a successful explicit read resumes the receiver
      this.noteEpoch(null, view.epochId);
    } catch (e) {
      if (!this.current(gen) || req !== this.indexReq) return;
      this.indexError = reason(e);
    } finally {
      if (this.current(gen) && req === this.indexReq) { this.indexLoading = false; this.notify(); }
    }
  }

  /// Open (or re-read) a flipnote. Opening a different object drops the previous document but
  /// keeps its pending saves: they are addressed by object and finish or fail on their own.
  open(object: Hex32 | null, opts: { read?: boolean } = {}): void {
    if (object === null) {
      if (this.doc) { this.doc = null; this.notify(); }
      return;
    }
    if (this.doc?.object !== object) {
      this.doc = { object, view: null, model: null, loading: opts.read === false, absent: false, error: "" };
      this.notify();
    }
    // A just-created object is filled by its Create result; reading it first would only
    // report the absence that preceded the save.
    if (opts.read !== false) void this.refreshDoc(object);
  }

  async refreshDoc(object: Hex32): Promise<void> {
    const gen = this.generation;
    const req = (this.docReq.get(object) ?? 0) + 1;
    this.docReq.set(object, req);
    const target = this.target();
    this.patchDoc(object, { loading: true });
    try {
      const view = await studioRead(this.ipc, target, object);
      if (!this.current(gen) || req !== this.docReq.get(object)) return;
      const model = view ? flipnoteModel(view.content) : null;
      if (view && model) {
        this.known.set(object, { awaiting: view.awaitingTenureReceipt, phase: view.phase, epoch: view.epoch });
        this.rememberView(object, view, model);
        this.noteEpoch(object, view.epochId);
      } else { this.known.delete(object); this.views.delete(object); }
      this.receivePaused = false;
      this.patchDoc(object, { view, model, absent: view === null, error: "", loading: false });
    } catch (e) {
      if (!this.current(gen) || req !== this.docReq.get(object)) return;
      this.patchDoc(object, { error: reason(e), loading: false });
    }
  }

  /// Replace the open document's state (never mutate it) when it is this object.
  private patchDoc(object: Hex32, patch: Partial<DocState>): void {
    if (this.doc?.object !== object) return;
    this.doc = { ...this.doc, ...patch };
    this.notify();
  }

  /// A read carrying a different epoch than an uncertain save used marks that save.
  private noteEpoch(object: Hex32 | null, epochId: Hex32): void {
    for (const s of this.saves) {
      if (s.status !== "uncertain") continue;
      const used = usedEpoch(s);
      const scope = s.kind === "applyIndex" ? null : s.object;
      if (used && scope === object && used !== epochId) s.epochChanged = true;
    }
  }

  // --- Edits ---------------------------------------------------------------------------------

  /// The current editable epoch for a document, or a reason there is none.
  editableEpoch(object: Hex32 | null): { epochId: Hex32 } | { refused: string } {
    const view = object === null ? this.index : this.doc?.object === object ? this.doc.view : this.views.get(object)?.view ?? null;
    if (!view) return { refused: object === null ? "the channel index is not loaded" : "the flipnote is not loaded" };
    if (view.awaitingTenureReceipt) return { refused: "read-only preview: the current owner has not confirmed this document's history" };
    if (view.phase === "fault") return { refused: "history fault: conflicting owner receipts; this document is read-only" };
    if (view.phase === "closing") return { refused: "rotating: durable edits resume when the owner settles this rotation" };
    return { epochId: view.epochId };
  }

  canEdit(object: Hex32 | null): boolean {
    return "epochId" in this.editableEpoch(object);
  }

  createFlipnote(title: string): Hex32 {
    const object = this.newId();
    this.enqueue({ kind: "create", object, label: `new flipnote "${title}"`, request: { object, nonce: this.newId(), title, createdAtMs: this.now() } });
    return object;
  }

  saveFrame(object: Hex32, frame: Hex32, pix: Uint8Array): void {
    // A newer raster for the same frame replaces an earlier record that has no apply identity
    // yet (nothing of it can have committed); one with an identity keeps it and the newer
    // pixels queue behind it in the same lane.
    const prior = this.saves.find((s) => s.kind === "frame" && s.object === object && s.frame === frame && s.status !== "inflight" && s.apply === null);
    if (prior && prior.kind === "frame") {
      prior.pix = pix;
      prior.published = null;
      prior.error = "";
      prior.status = "queued";
      this.notify();
      this.pump();
      return;
    }
    this.enqueue({ kind: "frame", object, frame, op: "replace", after: null, pix, published: null, apply: null, label: "frame pixels" });
  }

  insertFrame(object: Hex32, after: Hex32 | null, pix: Uint8Array): Hex32 {
    const frame = this.newId();
    this.enqueue({ kind: "frame", object, frame, op: "insert", after, pix, published: null, apply: null, label: "new frame" });
    return frame;
  }

  removeFrame(object: Hex32, frame: Hex32): void {
    this.applyOp(object, { op: "remove_frame", frame }, "remove frame");
  }

  setTitle(object: Hex32, title: string): void {
    this.applyOp(object, { op: "set_header", field: "title", value: title }, "title");
    if (this.indexModel?.entries.some((e) => e.id === object) || this.indexModel?.overflow.some((e) => e.id === object)) {
      this.applyIndexOp({ op: "set_title", object, title }, object, "index title");
    }
  }

  setFps(object: Hex32, fps: number): void {
    this.applyOp(object, { op: "set_header", field: "fps", value: fps }, "fps");
  }

  /// Use one of a conflicted frame's live values as the frame's pixels: fetch it bounded, hold it
  /// locally through publication, then replace. The blob is never named by a guessed cid.
  async useVersion(object: Hex32, frame: Hex32, alt: FrameConflictValue, how: "replace" | "insertAfter"): Promise<void> {
    const gen = this.generation;
    const bytes = await this.fetchNow(alt.cid, alt.bytes);
    if (!this.current(gen)) return;
    if (!bytes) throw new StudioNativeError("that version's pixels are not available yet");
    // Re-putting a value the register already selects is still an operation here: it consumes
    // every live predecessor and so resolves the conflict on purpose.
    if (how === "replace") this.enqueue({ kind: "frame", object, frame, op: "replace", after: null, pix: bytes, published: null, apply: null, force: true, label: `frame pixels by ${alt.author.slice(0, 8)}` });
    else this.enqueue({ kind: "frame", object, frame: this.newId(), op: "insert", after: frame, pix: bytes, published: null, apply: null, label: "keep both versions" });
  }

  renameEntry(object: Hex32, title: string): void {
    this.applyIndexOp({ op: "set_title", object, title }, object, "index title");
  }
  setEntryExpiry(object: Hex32, expiry: NativeExpiry): void {
    this.applyIndexOp({ op: "set_expiry", object, ...expiryForBody(expiry) } as IndexOp, object, "expiry");
  }
  deleteEntry(object: Hex32): void {
    this.applyIndexOp({ op: "tombstone_object", object }, object, "delete flipnote");
  }

  private applyOp(object: Hex32, op: FlipnoteOp, label: string): void {
    const ep = this.editableEpoch(object);
    if (!("epochId" in ep)) throw new StudioNativeError(ep.refused);
    this.enqueue({ kind: "apply", object, op, label, request: { object, epochId: ep.epochId, nonce: this.newId(), body: canonicalJson(op) } });
  }
  private applyIndexOp(op: IndexOp, object: Hex32, label: string): void {
    const ep = this.editableEpoch(null);
    if (!("epochId" in ep)) throw new StudioNativeError(ep.refused);
    this.enqueue({ kind: "applyIndex", object, op, label, request: { epochId: ep.epochId, nonce: this.newId(), body: canonicalJson(op) } });
  }

  private enqueue(rec: NewSave): SaveRecord {
    const record = { ...rec, id: ++this.saveSeq, status: "queued", attempts: 0, error: "", epochChanged: false } as SaveRecord;
    this.saves.push(record);
    this.notify();
    this.pump();
    return record;
  }

  /// Resend the same complete request.
  retry(id: number): void {
    const s = this.saves.find((r) => r.id === id);
    if (!s || s.status !== "uncertain") return;
    s.status = "queued";
    s.error = "";
    this.notify();
    this.pump();
  }

  /// Explicit re-authoring: a NEW operation with a fresh nonce against the current epoch. Never
  /// automatic; the local save under the old identity may already have committed.
  reauthor(id: number): void {
    const s = this.saves.find((r) => r.id === id);
    if (!s || s.status !== "uncertain") return;
    if (s.kind === "frame") s.apply = null;
    else if (s.kind === "apply") {
      const ep = this.editableEpoch(s.object);
      if (!("epochId" in ep)) throw new StudioNativeError(ep.refused);
      s.request = { ...s.request, epochId: ep.epochId, nonce: this.newId() };
    } else if (s.kind === "applyIndex") {
      const ep = this.editableEpoch(null);
      if (!("epochId" in ep)) throw new StudioNativeError(ep.refused);
      s.request = { ...s.request, epochId: ep.epochId, nonce: this.newId() };
    } else return; // create and recovery apply keep their identity; retry them as they are
    s.status = "queued";
    s.error = "";
    s.epochChanged = false;
    this.notify();
    this.pump();
  }

  /// Drop a save on purpose. For a frame save this discards its unsaved pixels; the saves that
  /// waited behind it in the same lane may then run.
  discard(id: number): void {
    const at = this.saves.findIndex((r) => r.id === id && r.status !== "inflight");
    if (at < 0) return;
    const [s] = this.saves.splice(at, 1);
    this.settleRecord(s, "discarded");
    this.notify();
    this.pump();
  }

  /// The newest unsaved pixels for a frame, so an editor reopens what the member drew rather
  /// than the last saved projection.
  unsavedPix(object: Hex32, frame: Hex32): Uint8Array | null {
    for (let i = this.saves.length - 1; i >= 0; i--) {
      const s = this.saves[i];
      if (s.kind === "frame" && s.object === object && s.frame === frame) return s.pix;
    }
    return null;
  }

  pendingFor(object: Hex32 | null): SaveRecord[] {
    return this.saves.filter((s) => (s.kind === "applyIndex" ? object === null || s.object === object : s.object === object));
  }

  /// Queued saves that cannot run until this uncertain save is retried or discarded.
  blockedBehind(id: number): number {
    const s = this.saves.find((r) => r.id === id);
    if (!s || s.status !== "uncertain") return 0;
    const lane = laneOf(s);
    return this.saves.filter((r) => r.id !== id && r.status === "queued" && laneOf(r) === lane).length;
  }

  /// One save at a time, in queue order within each lane (a document, or the Index). An
  /// uncertain or in-flight save blocks the later saves of its lane; other lanes proceed.
  private pump(): void {
    if (this.saving) return;
    const blocked = new Set<string>();
    let next: SaveRecord | undefined;
    for (const s of this.saves) {
      const lane = laneOf(s);
      if (blocked.has(lane)) continue;
      if (s.status === "queued") { next = s; break; }
      blocked.add(lane); // uncertain (or, defensively, inflight): nothing later in this lane runs
    }
    if (!next) return;
    const token = ++this.workerSeq;
    this.saving = token;
    void this.run(next).finally(() => {
      if (this.saving !== token) return; // a cleared scope owns a different lane now
      this.saving = 0;
      this.pump();
    });
  }

  private async run(s: SaveRecord): Promise<void> {
    const gen = this.generation;
    const target = this.target();
    s.status = "inflight";
    s.attempts++;
    this.notify();
    try {
      if (s.kind === "create") {
        const view = await studioCreate(this.ipc, target, s.request);
        if (!this.current(gen)) return;
        this.landed(s.object, view);
        this.invalidate({ index: true });
      } else if (s.kind === "frame") {
        if (!s.published) {
          const p = await publishPix(this.ipc, target.server, s.pix);
          if (!this.current(gen)) return;
          s.published = p;
          this.remember(p.cid, s.pix);
        }
        const current = this.doc?.object === s.object ? this.doc.model : this.views.get(s.object)?.model ?? null;
        if (s.op === "replace" && !s.force && current?.frames.find((f) => f.id === s.frame)?.cid === s.published.cid) {
          this.finish(s); // already the selected value: no operation for an unchanged value
          return;
        }
        if (!s.apply) {
          const ep = this.editableEpoch(s.object);
          if (!("epochId" in ep)) throw new StudioNativeError(ep.refused);
          const op: FlipnoteOp = s.op === "insert"
            ? { op: "insert_frame", frame: s.frame, after: s.after, cid: s.published.cid, bytes: s.published.bytes }
            : { op: "replace_frame", frame: s.frame, cid: s.published.cid, bytes: s.published.bytes };
          s.apply = { object: s.object, epochId: ep.epochId, nonce: this.newId(), body: canonicalJson(op) };
        }
        const view = await studioApply(this.ipc, target, s.apply);
        if (!this.current(gen)) return;
        this.landed(s.object, view);
      } else if (s.kind === "apply") {
        const view = await studioApply(this.ipc, target, s.request);
        if (!this.current(gen)) return;
        this.landed(s.object, view);
      } else if (s.kind === "applyIndex") {
        const view = await studioApplyIndex(this.ipc, target, s.request);
        if (!this.current(gen)) return;
        this.landedIndex(view);
      } else {
        const applied = await recoveryApply(this.ipc, { ...target, object: s.object }, s.edit);
        if (!this.current(gen)) return;
        s.applied = applied;
        if (s.object) this.invalidate({ objects: [s.object] });
        else this.invalidate({ index: true });
      }
      this.receivePaused = false;
      this.finish(s);
    } catch (e) {
      if (!this.current(gen)) return;
      // Timeout, cancellation, busy, capacity refusal, epoch mismatch: all leave the local save
      // uncertain. Keep the request and the pixels; the member retries or discards.
      s.status = "uncertain";
      s.error = reason(e);
      this.settleRecord(s, "uncertain");
      this.notify();
    }
  }

  private landed(object: Hex32, view: FlipnoteView): void {
    // A read issued before this save answered is older than this view: supersede it.
    this.docReq.set(object, (this.docReq.get(object) ?? 0) + 1);
    this.known.set(object, { awaiting: view.awaitingTenureReceipt, phase: view.phase, epoch: view.epoch });
    const model = flipnoteModel(view.content);
    this.rememberView(object, view, model);
    this.patchDoc(object, { view, model, absent: false, error: "", loading: false });
  }
  /// The Index equivalent: a landed write is newer than any list still in flight.
  private landedIndex(view: IndexView): void {
    this.indexReq++;
    this.index = view;
    this.indexModel = indexModel(view.content);
    this.indexError = "";
    this.indexLoading = false;
  }
  private rememberView(object: Hex32, view: FlipnoteView, model: FlipnoteModel): void {
    this.views.delete(object);
    this.views.set(object, { view, model });
    while (this.views.size > RECENT_VIEWS) this.views.delete(this.views.keys().next().value as Hex32);
  }
  private finish(s: SaveRecord): void {
    const at = this.saves.indexOf(s);
    if (at >= 0) this.saves.splice(at, 1);
    this.settleRecord(s, "landed");
    this.notify();
  }
  private settleRecord(s: SaveRecord, outcome: SaveOutcome): void {
    const settle = s.settle;
    s.settle = undefined;
    settle?.(outcome);
  }

  // --- Blobs ---------------------------------------------------------------------------------

  blob(cid: Hex64): Uint8Array | undefined {
    const b = this.blobs.get(cid);
    if (b) { this.blobs.delete(cid); this.blobs.set(cid, b); } // LRU touch
    return b;
  }
  blobState(cid: Hex64): BlobState {
    if (this.blobs.has(cid)) return "held";
    const job = this.jobs.get(`${this.generation}:${cid}`);
    if (job) return job.started ? "fetching" : "queued";
    if (this.invalid.has(cid)) return "invalid";
    if (this.failed.has(cid)) return "failed";
    if (this.unavailable.has(cid)) return "unavailable";
    return "idle";
  }
  blobProblem(cid: Hex64): string {
    return this.invalid.get(cid) ?? this.failed.get(cid) ?? (this.unavailable.has(cid) ? "not available from any reachable member yet" : "");
  }

  /// Ask for a frame's pixels. Lower priority runs first (0 = the frame on screen). A cid that
  /// answered unavailable, failed or was rejected is not asked again until an invalidation or
  /// an explicit retry clears it.
  want(cid: Hex64, bytes: number, priority = 2): void {
    if (this.unavailable.has(cid) || this.failed.has(cid) || this.invalid.has(cid)) return;
    this.scheduleFetch(cid, bytes, priority);
  }

  /// Explicit retry from the member: clears every hold on this cid and asks first.
  retryBlob(cid: Hex64, bytes: number): void {
    this.unavailable.delete(cid);
    this.failed.delete(cid);
    this.invalid.delete(cid);
    this.scheduleFetch(cid, bytes, 0);
  }

  /// One bounded fetch for an action that needs the bytes now: it takes the front of the same
  /// queue and the same in-flight slots, and shares an existing job for the cid.
  fetchNow(cid: Hex64, bytes: number): Promise<Uint8Array | null> {
    const held = this.blobs.get(cid);
    if (held) return Promise.resolve(held);
    this.unavailable.delete(cid);
    this.failed.delete(cid);
    const job = this.scheduleFetch(cid, bytes, -1);
    if (!job) return Promise.resolve(this.blobs.get(cid) ?? null);
    return new Promise((resolve, reject) => { job.waiters.push({ resolve, reject }); });
  }

  private scheduleFetch(cid: Hex64, bytes: number, priority: number): BlobJob | null {
    if (this.blobs.has(cid) || !this.scope) return null;
    const key = `${this.generation}:${cid}`;
    let job = this.jobs.get(key);
    if (job) {
      if (!job.started && priority < job.priority) job.priority = priority;
      return job;
    }
    job = { key, cid, bytes, priority, generation: this.generation, started: false, waiters: [] };
    this.jobs.set(key, job);
    this.pumpFetches();
    return job;
  }

  private pumpFetches(): void {
    while (this.inflight < this.maxFetches) {
      let next: BlobJob | null = null;
      for (const job of this.jobs.values()) {
        if (job.started || job.generation !== this.generation) continue;
        if (!next || job.priority < next.priority) next = job;
      }
      if (!next) return;
      void this.fetchOne(next);
    }
  }

  private async fetchOne(job: BlobJob): Promise<void> {
    job.started = true;
    this.inflight++;
    const target = this.target();
    this.notify();
    let result: Uint8Array | null = null;
    let failure: unknown = null;
    try {
      const got = await requestBlobBounded(this.ipc, target.server, job.cid, job.bytes);
      if (this.current(job.generation)) {
        if (!got) this.unavailable.add(job.cid);
        else {
          decodePix(got);
          this.remember(job.cid, got);
          result = got;
        }
      }
    } catch (e) {
      failure = e;
      if (this.current(job.generation)) {
        // Length/base64/PIX1 failures are about the content named by this cid; anything else
        // (busy, cancelled, transport) is about this attempt and may succeed next time.
        if (e instanceof StudioNativeError || e instanceof PixError) this.invalid.set(job.cid, reason(e));
        else this.failed.set(job.cid, reason(e));
      }
    } finally {
      // Release exactly this job's slot and marker; a same-cid job of a newer scope is its own.
      this.inflight--;
      if (this.jobs.get(job.key) === job) this.jobs.delete(job.key);
      for (const w of job.waiters) {
        if (failure && this.current(job.generation)) w.reject(failure);
        else w.resolve(result);
      }
      this.notify();
      this.pumpFetches();
    }
  }

  private remember(cid: Hex64, bytes: Uint8Array): void {
    if (this.blobs.has(cid)) return;
    this.unavailable.delete(cid);
    this.failed.delete(cid);
    this.invalid.delete(cid);
    this.blobs.set(cid, bytes);
    this.cacheBytes += bytes.length;
    while ((this.cacheBytes > this.cacheBytesCap || this.blobs.size > BLOB_CACHE_ENTRIES) && this.blobs.size > 1) {
      const oldest = this.blobs.keys().next().value as Hex64;
      this.cacheBytes -= this.blobs.get(oldest)!.length;
      this.blobs.delete(oldest);
    }
  }

  /// A document changed: what was unavailable or failed for its frames may be reachable now.
  /// Rejected content stays rejected until an explicit retry.
  private releaseTransientFor(object: Hex32): void {
    this.failed.clear();
    if (this.doc?.object !== object || !this.doc.model) { this.unavailable.clear(); return; }
    for (const f of this.doc.model.frames) this.unavailable.delete(f.cid);
  }

  // --- Recovery ------------------------------------------------------------------------------

  /// Point the rail at a document (null = the Index). The same target is a no-op; the listing
  /// is read on a change of target, on an invalidation, or through an explicit refresh.
  watchRecovery(object: Hex32 | null): void {
    if (this.recoveryTarget === object && (this.recoveryListing || this.recoveryLoading || this.recoveryError)) return;
    this.recoveryTarget = object;
    this.recoveryListing = null;
    this.recoveryError = "";
    this.recoveryRun = null;
    this.activeRun = null;
    this.pointer = { status: "idle", error: "", result: null };
    this.notify();
    void this.refreshRecovery();
  }

  async refreshRecovery(): Promise<void> {
    const gen = this.generation;
    const req = ++this.recoveryReq;
    const object = this.recoveryTarget;
    const target = this.target();
    this.recoveryLoading = true;
    this.notify();
    try {
      const listing = await recoveryList(this.ipc, { ...target, object });
      if (!this.current(gen) || req !== this.recoveryReq || object !== this.recoveryTarget) return;
      this.recoveryListing = listing;
      this.recoveryError = "";
    } catch (e) {
      if (!this.current(gen) || req !== this.recoveryReq || object !== this.recoveryTarget) return;
      this.recoveryError = reason(e);
    } finally {
      if (this.current(gen) && req === this.recoveryReq) { this.recoveryLoading = false; this.notify(); }
    }
  }

  async readVersion(snapshot: Hex64): Promise<RecoveryVersionRead | null> {
    const cached = this.versions.get(snapshot);
    if (cached) return cached;
    const gen = this.generation;
    const object = this.recoveryTarget;
    const v = await recoveryRead(this.ipc, { ...this.target(), object }, snapshot);
    if (!this.current(gen) || object !== this.recoveryTarget) return null;
    this.versions.set(snapshot, v);
    this.notify();
    return v;
  }

  async exportVersion(snapshot: Hex64): Promise<RecoveryExport | null> {
    const gen = this.generation;
    const object = this.recoveryTarget;
    const x = await recoveryExport(this.ipc, { ...this.target(), object }, snapshot);
    if (!this.current(gen) || object !== this.recoveryTarget) return null;
    this.exports.set(snapshot, x);
    this.notify();
    return x;
  }

  async acknowledgeEviction(): Promise<void> {
    const w = this.recoveryListing?.evictionPending;
    if (!w) throw new StudioNativeError("no eviction warning to acknowledge");
    const gen = this.generation;
    const object = this.recoveryTarget;
    const listing = await recoveryAcknowledge(this.ipc, { ...this.target(), object }, { oldestSnapshot: w.oldestSnapshot, stagedSnapshot: w.stagedSnapshot });
    if (!this.current(gen) || object !== this.recoveryTarget) return;
    this.recoveryReq++; // a list still in flight is older than this answer
    this.recoveryListing = listing;
    this.recoveryLoading = false;
    this.notify();
  }

  /// The choices a historical version offers, in the order Restore walks them. Headers never
  /// restore automatically: they are listed for Copy only; deletions are Copy only.
  choicesFor(v: RecoveryVersionRead, mode: RecoveryMode): { choice: RecoveryChoice; label: string }[] {
    const out: { choice: RecoveryChoice; label: string }[] = [];
    if (v.content.kind === "flipnote") {
      const c = v.content;
      c.timeline.forEach((id, i) => {
        const e = c.frames[id];
        if (e) out.push({ choice: { kind: "frame", id, value: e.pixels.selected.source.opId }, label: `frame ${i + 1}` });
      });
      if (mode === "copy") {
        if (c.title) out.push({ choice: { kind: "title", value: c.title.selected.source.opId }, label: `title "${c.title.selected.value}"` });
        if (c.fps) out.push({ choice: { kind: "fps", value: c.fps.selected.source.opId }, label: `${c.fps.selected.value} fps` });
        for (const id of Object.keys(c.tombstones)) out.push({ choice: { kind: "frameDeletion", id }, label: `deletion of frame ${id.slice(0, 8)}` });
      }
    } else {
      const c = v.content;
      for (const [id, e] of [...Object.entries(c.objects), ...Object.entries(c.overflow)]) {
        out.push({ choice: { kind: "object", id }, label: `entry "${e.title.selected.value}"` });
        if (mode === "copy") {
          out.push({ choice: { kind: "objectTitle", id, value: e.title.selected.source.opId }, label: `title "${e.title.selected.value}"` });
          out.push({ choice: { kind: "objectExpiry", id, value: e.expiry.selected.source.opId }, label: `expiry of "${e.title.selected.value}"` });
        }
      }
      if (mode === "copy") for (const id of Object.keys(c.tombstones)) out.push({ choice: { kind: "objectDeletion", id }, label: `deletion of entry ${id.slice(0, 8)}` });
    }
    return out;
  }

  /// Walk a version's choices: preview, apply what is Ready through the save queue, re-read,
  /// next. A partial walk is saved content; the run reports each item's disposition and never
  /// calls itself a Restore. An uncertain apply stops the walk: its exact payload stays on the
  /// save record for retry, and a later walk previews afresh only after that is resolved.
  async runRecovery(snapshot: Hex64, mode: RecoveryMode): Promise<void> {
    const gen = this.generation;
    const object = this.recoveryTarget;
    const version = await this.readVersion(snapshot);
    if (!version || !this.current(gen)) return;
    const token = ++this.runSeq;
    const control = { token, stop: false };
    this.activeRun = control;
    const run: RecoveryRun = {
      snapshot, mode, object, status: "running", error: "",
      items: this.choicesFor(version, mode).map(({ choice, label }) => ({ choice, label, state: "pending", error: "", originalAuthor: null })),
    };
    const live = () => this.current(gen) && this.activeRun === control;
    const publish = () => {
      if (!live()) return;
      this.recoveryRun = { ...run, items: run.items.map((i) => ({ ...i })) };
      this.notify();
    };
    publish();
    const target = { ...this.target(), object };
    for (const item of run.items) {
      if (!live()) return;
      if (control.stop) { run.status = "stopped"; publish(); return; }
      item.state = "previewing";
      publish();
      try {
        const preview = await recoveryPreview(this.ipc, target, snapshot, item.choice, mode);
        if (!live()) return;
        item.originalAuthor = preview.originalAuthor;
        if (preview.disposition !== "ready" || preview.body === null) { item.state = preview.disposition; publish(); continue; }
        if (item.choice.kind === "frame") {
          // Pixels must be held locally before the Save that references them: fetch the exact
          // historical value bounded by its declared size, then hold it through publication.
          const held = historicalBlob(version, item.choice.id, item.choice.value);
          if (!held) { item.state = "error"; item.error = "the historical frame value is not in this version"; publish(); continue; }
          item.state = "fetching";
          publish();
          const bytes = await this.fetchNow(held.cid, held.bytes);
          if (!live()) return;
          if (!bytes) { item.state = "error"; item.error = "pixels unavailable; not applied"; publish(); continue; }
          const p = await publishPix(this.ipc, target.server, bytes);
          if (!live()) return;
          if (p.cid !== held.cid) { item.state = "error"; item.error = "held pixels do not name the historical cid"; publish(); continue; }
        }
        item.state = "applying";
        publish();
        const edit: RecoveryApplyEdit = { snapshot, choice: item.choice, mode, epochId: preview.epochId, expectedProjection: preview.expectedProjection, nonce: this.newId(), body: preview.body };
        const record = this.enqueue({ kind: "recoveryApply", object, edit, applied: null, label: `${mode}: ${item.label}` });
        const outcome = await new Promise<SaveOutcome>((resolve) => { record.settle = resolve; });
        if (!live()) return;
        if (outcome !== "landed") {
          // The complete payload is on the save record; only its exact retry may finish it.
          item.state = "uncertain";
          item.error = outcome === "uncertain" ? "apply uncertain; retry the same request from the save card, then run again" : "discarded";
          run.status = "stopped";
          run.error = "stopped at an uncertain apply";
          publish();
          return;
        }
        const applied = record.kind === "recoveryApply" ? record.applied : null;
        item.state = applied?.alreadySaved ? "alreadySaved" : "applied";
        publish();
        // Re-read before the next preview so its projection fingerprint is the current one.
        if (object) await this.refreshDoc(object); else await this.refreshIndex();
      } catch (e) {
        if (!live()) return;
        item.state = "error";
        item.error = reason(e);
        publish();
      }
    }
    run.status = "done";
    publish();
    this.invalidate({ index: true, objects: object ? [object] : [], recovery: true });
  }

  /// Stop after the item in progress; the walk reports "stopped".
  stopRecovery(): void {
    if (this.activeRun && this.recoveryRun?.status === "running") this.activeRun.stop = true;
  }

  /// The separate, separately retryable Registry pointer step for a document that was saved.
  async restorePointer(object: Hex32 | null = this.recoveryTarget): Promise<void> {
    const gen = this.generation;
    this.pointer = { status: "running", error: "", result: null };
    this.notify();
    try {
      const r = await recoveryRestorePointer(this.ipc, { ...this.target(), object });
      if (!this.current(gen)) return;
      this.pointer = { status: "done", error: "", result: r };
    } catch (e) {
      if (!this.current(gen)) return;
      this.pointer = { status: "blocked", error: reason(e), result: null };
    }
    this.notify();
  }
}

function laneOf(s: SaveRecord): string {
  return s.kind === "applyIndex" ? "index" : s.object ?? "index";
}

function usedEpoch(s: SaveRecord): Hex32 | null {
  switch (s.kind) {
    case "frame": return s.apply?.epochId ?? null;
    case "apply": case "applyIndex": return s.request.epochId;
    case "recoveryApply": return s.edit.epochId;
    default: return null;
  }
}

function historicalBlob(v: RecoveryVersionRead, frame: Hex32, opId: Hex64): { cid: Hex64; bytes: number } | null {
  if (v.content.kind !== "flipnote") return null;
  const e = v.content.frames[frame];
  if (!e) return null;
  for (const x of [e.pixels.selected, ...e.pixels.conflicts]) if (x.source.opId === opId) return x.value;
  for (const x of e.insertions) if (x.source.opId === opId) return x.value.blob;
  return null;
}

export function reason(e: unknown): string {
  if (e instanceof StudioNativeError) return e.reason;
  if (e instanceof Error) return e.message;
  return String(e);
}

export { DEFAULT_FPS };
