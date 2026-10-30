// One channel's connected Studio: the Index view, the open flipnote, the pixels held for it, the
// saves in flight and the recovery rail, all read through the typed native adapter. Plain data
// plus a change signal, so the two surfaces re-read it the way they read the old fixture, and the
// unit tests drive it with a fake bridge.
//
// Rules this class exists to keep (docs/FLIPNOTE-UI-HOOKS.md):
// - A save is one complete request. A retry resends exactly it, nonce and epoch included; only an
//   explicit "save again as a new edit" mints a new nonce against the current epoch. Recovery
//   applies are saves too: their echoed preview payload is retained and retried the same way.
// - Saves to one document run in order. An uncertain or explicitly not-written save blocks the later saves of its lane
//   until it is retried or discarded on purpose; other documents keep moving.
// - Unsaved pixels survive every failure. They stay on the record until the save lands or the
//   member discards them on purpose. Changes to that retained work are signalled separately
//   (`onWorkChange`), so App can checkpoint it into the sealed continuity snapshot; leaving the
//   Studio surface needs that local checkpoint, not a native receipt (`preserveForDeparture`).
// - A preview-local Save answered `pending` or `busy` is continued automatically with the
//   identical request (`runUnconfirmed`): that is the native contract's progress answer, never a
//   failure for the member to Retry.
// - Every asynchronous result is fenced by scope generation (server, channel, session) and by a
//   per-target request generation; a late result is dropped even after IPC delivered it, and a
//   landed write supersedes the reads that were issued before it.
// - A preview (`awaitingTenureReceipt`) never authorizes an ordinary edit. The editor may save a
//   ticketed local-only Unconfirmed overlay, which stays visibly separate from installed history.
// - One fetch scheduler: thumbnails, the open frame, conflict actions and recovery all go through
//   the same bounded, prioritized, deduplicated queue. A cid that was unavailable or failed
//   transiently is asked again only after an invalidation or an explicit retry; content the
//   decoder rejected is kept apart from that.
// - State the surfaces read is replaced, never mutated in place, so a revision bridge sees a new
//   reference whenever something changed.

import { PixError, decodePix } from "./pix.ts";
import { workingFlipnote, workingScore, workingIndex, workingSlot } from "./studio-working.ts";
import { decodeStudioDraft, encodeStudioDraft, validateDraftOperation, type StudioDraft } from "./studio-draft.ts";
import { validateJamPatch } from "./jam-patch.ts";
import type { JamPatch, JamTake } from "./jam-contract.ts";
import { flipnotePixaFromNative, preflightFlipnoteMedia, scorePlaybackFromNative } from "./studio-resolved-export.ts";
import { exportScorePlayback, type ScorePlayback, type ScoreExportInput } from "./studio-export.ts";
import { exportStudioGif, publishStudioPixa, type StudioGifSaved } from "./studio-media.ts";
import { decodePixa, encodePixa, exportFlipnotePixa, PIXA_MAX_BYTES } from "./pixa.ts";
import { historicalSoundOperation, soundRecoveryChoices } from "./studio-sound-recovery.ts";
import { readStudioFault, repairStudioFault, type FaultScope, type FaultView } from "./studio-fault.ts";
import { canonicalJson, randomElementId, type FlipnoteOp, type IndexOp, type ScoreOp, type ScoreCell } from "./studio-contract.ts";
import {
  DEFAULT_FPS,
  SETTLEMENT_CHANGED_EVENT,
  STUDIO_RECEIVE_PAUSED_EVENT,
  STUDIO_SYNC_STATE_EVENT,
  STUDIO_UPDATED_EVENT,
  StudioNativeError,
  expiryForBody,
  flipnoteModel,
  scoreModel,
  indexModel,
  overlayArchive,
  overlayArchiveExport,
  overlayArchiveRead,
  overlayArchiveRelease,
  overlayCopyApply,
  overlayCopyPreview,
  overlayDispose,
  overlayExport,
  overlayLifecycle,
  overlayRead,
  parseReceivePaused,
  parseStudioSyncState,
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
  unconfirmedOverlaySave,
  unconfirmedOverlaySaveBegin,
  closingOverlaySave,
  closingOverlaySaveBegin,
  type ClosingOverlaySave,
  type ApplyIndexRequest,
  type ApplyRequest,
  type CreateRequest,
  type Decimal,
  type FlipnoteModel,
  type FrameConflictValue,
  type Hex32,
  type Hex64,
  type IndexModel,
  type IndexView,
  type NativeExpiry,
  type OverlayArchive,
  type OverlayCopyApplied,
  type OverlayCopyApplyEdit,
  type OverlayCopyPreview,
  type OverlayDisposalRequest,
  type OverlayInspection,
  type OverlayLifecycle,
  type OverlaySaveRetry,
  type OverlaySaveRefusal,
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
  type StudioContent,
  type StudioObjectKind,
  type StudioObjectView,
  type ScoreModel,
  type FlipnoteView,
  type ScoreView,
  type RecoveryTargetArgs,
  type StudioPhase,
  type StudioUnlisten,
  type UnconfirmedOverlaySave,
  type UnconfirmedOverlaySaveInput,
} from "./studio-native.ts";

export type StudioScope = { server: number; channel: Decimal };
export type StudioPixaPublished = { publication: PixPublication; referenceSaveId: number | null; warning: string | null };

export type SaveStatus = "queued" | "inflight" | "blocked" | "uncertain";
export type SaveOutcome = "landed" | "notWritten" | "uncertain" | "discarded";
type SaveBase = {
  id: number;
  label: string;
  status: SaveStatus;
  attempts: number;
  error: string;
  /// A delivery failure on this exact request has not been reconciled. This remains true while a
  /// later exact retry is queued or in flight, so its latest attempt cannot hide older uncertainty.
  mayHaveCommitted: boolean;
  /// Set when a later read of the same document carries a different epoch than the frozen
  /// request: the backend decides on retry; the member may instead re-author explicitly.
  epochChanged: boolean;
  /** Admission evidence for delayed first delivery; never rebound by an autosave. */
  binding?: { epochId: string; awaiting: boolean; phase: StudioPhase | null };
};
/// What a surface sees of a save: an immutable snapshot, replaced whenever the queue changes.
export type SaveRecord = SaveBase &
  (
    | { kind: "create"; request: CreateRequest<StudioObjectKind>; object: Hex32 }
    | { kind: "frame"; object: Hex32; frame: Hex32; op: "insert" | "replace"; after: Hex32 | null; pix: Uint8Array; published: PixPublication | null; route: "ordinary" | "unconfirmed" | "closing"; apply: ApplyRequest | null; overlay: UnconfirmedOverlaySaveInput | null; overlayRetry: OverlaySaveRetry | null; force?: boolean }
    | { kind: "apply"; object: Hex32; request: ApplyRequest<StudioObjectKind>; op: FlipnoteOp | ScoreOp }
    | { kind: "applyIndex"; request: ApplyIndexRequest; op: IndexOp; object: Hex32 }
    | { kind: "overlayApply"; object: Hex32 | null; objectKind?: StudioObjectKind; route?: "unconfirmed" | "closing"; request: UnconfirmedOverlaySaveInput | null; overlayRetry: OverlaySaveRetry | null; op: FlipnoteOp | IndexOp | ScoreOp }
    | { kind: "recoveryApply"; object: Hex32 | null; objectKind?: StudioObjectKind; edit: RecoveryApplyEdit; applied: RecoveryApplied | null }
  );
/// The queue's own mutable entry, with the callback of whoever awaits its first outcome.
type SaveEntry = SaveRecord & { settle?: (outcome: SaveOutcome) => void };

/// A typed Flow-S result that proves this attempt did not write. It is deliberately separate from
/// native/transport errors, whose final-fence timing means the write outcome can be uncertain.
class OverlayNoWrite extends Error {}

/// held: decoded bytes are cached. fetching/queued: a job for this scope exists. unavailable:
/// the backend answered null. failed: the request itself failed (busy, transport); retryable.
/// invalid: the body failed the length or PIX1 check; content, not availability. idle: not asked.
export type BlobState = "held" | "fetching" | "queued" | "unavailable" | "failed" | "invalid" | "idle";

type DistributiveOmit<T, K extends PropertyKey> = T extends unknown ? Omit<T, K> : never;
type NewSave = DistributiveOmit<SaveRecord, "id" | "status" | "attempts" | "error" | "mayHaveCommitted" | "epochChanged">;

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

export type OverlayState = {
  object: Hex32 | null;
  inspection: OverlayInspection | null;
  lifecycle: OverlayLifecycle | null;
  archive: OverlayArchive | null;
  exported: OverlayArchive | null;
  copy: {
    objectKind?: StudioObjectKind;
    choice: RecoveryChoice;
    label: string;
    mode: RecoveryMode;
    status: "previewing" | "applying" | "applied" | "refused" | "uncertain" | "error" | RecoveryDisposition;
    error: string;
    /// Once any Apply delivery is ambiguous, a later pre-write refusal cannot prove what the
    /// earlier attempt did. Only an applied acknowledgement may clear this uncertainty.
    mayHaveCommitted: boolean;
    preview: OverlayCopyPreview | null;
    edit: OverlayCopyApplyEdit | null;
    applied: OverlayCopyApplied | null;
  } | null;
  loading: boolean;
  error: string;
};

type OverlayCopyRun = NonNullable<OverlayState["copy"]>;

export type DocState = {
  object: Hex32;
  objectKind: StudioObjectKind;
  view: StudioObjectView | null;
  model: FlipnoteModel | null;
  score: ScoreModel | null;
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
  /** Production edits wait five seconds from the first change; zero is the immediate test seam. */
  autosaveMs?: number;
};

const REFRESH_COALESCE_MS = 120;
const BLOB_CACHE_BYTES = 8 * 1024 * 1024;
const BLOB_CACHE_ENTRIES = 512;
const RECENT_VIEWS = 8;
const RETAINED_OVERLAY_COPIES = 8;
/// Saves in flight at once across lanes (still one per lane). Two is the least that lets a document
/// lane and the shared Index lane both be in flight, so a departure's document Save does not wait
/// on its own Index half's receipt. Workers are not reserved per lane: two other lanes (a Create
/// and another document's Save, say) can still hold both, and the next Save then waits for one of
/// them. More buys nothing natively: the server actor runs Studio operations one at a time, and
/// every Studio IPC call (reads, blob fetches and the native receiver's passes too) holds one of
/// the bridge's four shared inline slots for its whole round trip.
const SAVE_WORKERS = 2;
/// Backoff before each automatic resend of a request the bridge refused before dispatch
/// (nativeNoWrite): the same request, no attempt counted unless an earlier attempt was already
/// uncertain (resendLater). The backoffs total 1.75 s, but the
/// refusal itself is not always instant: "Studio actor busy" arrives only after its 5 s rendezvous,
/// so four such sends take about 22 s and outlast a departure's 10 s timeout, which then reports
/// the Save as still running.
const BUSY_BACKOFF_MS = [250, 500, 1000];
/// Pacing for the automatic continuation of a preview-local (Unconfirmed or Closing) Save. Native
/// answers the first visit of new authoring with `pending` (its plan is prepared off the actor) or
/// `busy` (the one overlay slot holds other work), and its contract is that the caller repeats the
/// IDENTICAL request until it is saved (receiver/unconfirmed.rs `save_unconfirmed_overlay`). That
/// is progress, not failure, so the worker repeats it itself: quickly at first, because a small
/// edit's plan is usually ready within a few hundred milliseconds, then once a second. The budget
/// stays well inside native's 30 s park (catchup.rs `OVERLAY_PARK_MS`): a plan left parked longer
/// is discarded and the next visit has to prepare it again.
const OVERLAY_CONTINUE_MS = [100, 200, 400, 800];
const OVERLAY_CONTINUE_STEADY_MS = 1000;
const OVERLAY_CONTINUE_BUDGET_MS = 20_000;
/// How long a departure that only needs local preservation (`preserveForDeparture`) gives queued
/// Saves to land before it stops waiting. Their outcome is not a condition of leaving: whatever is
/// still held after this is kept in the session and in the sealed continuity snapshot.
const DEPARTURE_SETTLE_MS = 1500;

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
  // Unlike docReq/indexReq, these advance ONLY for actual read admission. Navigation without
  // a read (especially Create→open→first frame) cannot supersede the Create receipt's view.
  private readonly docReadSerial = new Map<Hex32, number>();
  private indexReadSerial = 0;
  private readonly documentCloseHandlers = new Set<() => void>();
  private navigationSerial = 0;
  private closeHeld = false;
  /** Stage before freezing input. Existing requests may drain while native close seals state;
   * no new raster or navigation may escape that final snapshot. endClose releases a failed close. */
  beginClose(): void {
    this.cancelNavigation(); this.prepareDocumentClose(); this.stageDrawing();
    this.closeHeld = true; this.notify();
  }
  endClose(): void { if (this.closeHeld) { this.closeHeld = false; this.notify(); } }
  cancelNavigation(): void { this.navigationSerial++; }
  /** App and both editor surfaces share this lease. Check it after the last await, immediately
   * before mutation; checking only in an awaited helper leaves a microtask race. */
  claimNavigation(): () => boolean {
    const request = ++this.navigationSerial, generation = this.generation;
    return () => request === this.navigationSerial && generation === this.generation;
  }
  /** Sidebar and Music share one navigation owner; a newer action wins across both surfaces.
   *
   * The departure Save covers only the document being left. Records queued for any other document
   * (a lock/unlock restore leaves them offscreen, as does deleting a document from the Index while
   * its edits are still queued; neither passes through a departure) stay exactly as they are:
   * their Retry, reauthor and discard controls live in that document's own editor, so requiring
   * them to save first would make the one route to those controls depend on the controls. With no
   * document open nothing is being left, so nothing is saved and restored work stays held until
   * an explicit Save or retry. Leaving the Studio, a channel or the window still saves everything
   * through `saveBeforeLeaving`. */
  async navigate(action: () => void): Promise<void> {
    if (this.closeHeld) throw new StudioNativeError("The window is closing. Wait for saving to finish.");
    const current = this.claimNavigation();
    const leaving = this.doc?.object ?? null;
    if (leaving !== null) {
      const generation = this.generation;
      await this.saveUntilClear(s => laneOf(s) === leaving);
      // The document's records in the shared Index lane (the Index half of a rename, an expiry)
      // are its work too: send them, but never wait on them. That lane can sit behind another
      // document's held work, and waiting there would bring back the deadlock this scoping removes.
      // Released only AFTER the document's own lane drained: the save workers are few (and the
      // actor serves one Studio operation at a time), so Index work released first would compete
      // with the document's own Save for them, the lane the departure must not wait on. Released even if a newer
      // action superseded this navigation: the document half has been sent, and holding the Index
      // half back would split the rename. Not across a reset, though (a lock and restore landing in
      // the resume gap would otherwise release freshly restored work nobody chose to send).
      if (generation === this.generation) this.releaseQueued(true, s => laneOf(s) !== leaving && affectedObject(s) === leaving);
    }
    if (!current()) return;
    action();
  }
  private readonly draftHandlers = new Set<() => void>();
  beforeSave(handler: () => void): () => void {
    this.draftHandlers.add(handler); return () => { this.draftHandlers.delete(handler); };
  }
  stageDrawing(): void { for (const handler of this.draftHandlers) handler(); }

  /** Flush the mounted editor before replacing its document. Throwing vetoes the change;
   * queued saves retain their original object, pixels and retry identity offscreen. */
  beforeDocumentClose(handler: () => void): () => void {
    this.documentCloseHandlers.add(handler);
    return () => { this.documentCloseHandlers.delete(handler); };
  }

  prepareDocumentClose(): void {
    if (this.doc) for (const handler of this.documentCloseHandlers) handler();
  }

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
  indexOverlay: OverlayState | null = null;
  /// The retained local branch is a separate truth from the ordinary/preview projection. It is
  /// read-only evidence here; edits reach it only through the ticketed Unconfirmed Save below.
  overlay: OverlayState | null = null;
  /// Trust state of documents this session has read, for sidebar chips; never a phase claim
  /// for a document that was not read.
  readonly known = new Map<Hex32, KnownView>();
  /// The last view of each recently read document, so a queued save still has its epoch after
  /// the member moves to another flipnote. Bounded; the open document is always current.
  private readonly views = new Map<Hex32, { view: StudioObjectView; model: FlipnoteModel | null; score: ScoreModel | null }>();
  /// Exact retained-draft Copy requests are session work, not panel state. Keeping them here means
  /// navigation cannot lose an uncertain nonce/body or let a late response overwrite a newer run.
  /// The map is bounded; unresolved entries are never evicted merely to make room.
  private readonly overlayCopies = new Map<string, OverlayCopyRun>();

  /// Mutable queue entries (with their settle callbacks); never handed to a surface.
  private readonly records: SaveEntry[] = [];
  /// Immutable snapshots of the queue, rebuilt on every change: what the surfaces read.
  saves: SaveRecord[] = [];
  private saveSeq = 0;
  /// Tokens of the save workers running now (at most SAVE_WORKERS). Only a worker whose token is
  /// still here may re-pump when it ends; a worker from a cleared scope finds its token gone and
  /// stands down.
  private readonly workers = new Set<number>();
  /// Records a worker has taken, by id. A record stays `queued` through its admission read, so
  /// this, not its status, is what keeps a second worker off it and its lane.
  private readonly running = new Set<number>();
  /// Records waiting out a backoff after a pre-dispatch refusal; each blocks its lane until resent.
  private readonly cooling = new Set<number>();
  /// Automatic resends used per record (see BUSY_BACKOFF_MS).
  private readonly busyResends = new Map<number, number>();
  private workerSeq = 0;
  private readonly autosaveMs: number;
  get buffering(): boolean { return this.autosaveMs > 0; }
  private cancelAutosave: (() => void) | null = null;
  private readonly eligible = new Set<number>();
  private readonly restoredHeld = new Set<number>();
  private editorEpoch = 0;
  private editorSnapshot: { key: string; frames: { id: string; cid: string; bytes: Uint8Array | undefined }[] } | null = null;
  get editorMediaGeneration(): number { return this.editorEpoch; }
  // Hold the original scope even if unlock initially selects another channel. New authoring
  // cannot overwrite this sole retained queue before its original channel is selected.
  private restoredDraft: StudioDraft | null = null;
  private draftGroup: string | null = null;
  private readonly draftCosts = new WeakMap<object, number>();
  workingModel: FlipnoteModel | null = null;
  workingScore: ScoreModel | null = null;
  workingIndex: IndexModel | null = null;

  /// Local-preservation bookkeeping (see `workRevision`, `authorRevision` and `onWorkChange`).
  private workKey = "";
  private workRev = 0;
  private authorRev = 0;
  private readonly workListeners = new Set<() => void>();
  /// Per-array identity for the pixels a frame record holds, so the work key notices a coalesced
  /// stroke (the same record with new pixels) without hashing or comparing the bytes themselves.
  private readonly pixIds = new WeakMap<Uint8Array, number>();
  private pixSeq = 0;
  /// Coalesced refreshes in flight, by target: at most one read per target at a time, plus one
  /// "read again" flag set when another invalidation arrives meanwhile (see `coalesceRefresh`).
  private readonly refreshes = new Map<string, { again: boolean }>();
  /// Frame CIDs the mounted editor has on screen (the open frame, its onion neighbours, visible
  /// thumbnails). Cache eviction skips them so a mounted thumbnail cannot evict, refetch and evict
  /// again in a loop when a long timeline exceeds the cache bounds.
  private blobFocus = new Set<Hex64>();

  receivePaused = false;
  receivePauseDetail = "";
  private syncObservation: ReturnType<typeof parseStudioSyncState> | null = null;
  // Navigation must not make an older server event new again. These are locally selected
  // server ids, with a bounded recent-server cache; a cold/evicted scope refreshes its Index
  // immediately and the backend emits its current observation on that successful read.
  private readonly syncByServer = new Map<number, ReturnType<typeof parseStudioSyncState>>();
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
  private recoveryFamily: StudioObjectKind | "index" = "index";
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
  private indexOverlayReq = 0;
  private readonly docReq = new Map<Hex32, number>();
  private readonly overlayReq = new Map<Hex32, number>();
  private recoveryReq = 0;
  private pendingIndex = false;
  private readonly pendingObjects = new Set<Hex32>();
  private pendingRecovery = false;
  private cancelFlush: (() => void) | null = null;
  private unlisten: StudioUnlisten[] = [];
  // One preparation and one publication/save action may be in flight, even across navigation.
  // Old work must actually drain before another scope can allocate a second full package.
  private mediaTask: Promise<unknown> | null = null;
  private mediaWrite = false;
  private mediaEpoch = 0;
  private faultEpoch = 0;
  /** Observation invalidations include Index/Registry notices, without treating loading as news. */
  get faultGeneration(): number { return this.faultEpoch; }
  /** Immediate invalidation edge used to revoke an already prepared UI playback snapshot. */
  get mediaGeneration(): number { return this.mediaEpoch; }

  constructor(opts: SessionOptions) {
    this.ipc = opts.ipc;
    this.me = opts.me;
    this.now = opts.now ?? (() => Date.now());
    this.newId = opts.newId ?? (() => randomElementId());
    this.schedule = opts.schedule ?? ((fn, ms) => { const t = setTimeout(fn, ms); return () => clearTimeout(t); });
    this.maxFetches = opts.maxFetches ?? 2;
    this.cacheBytesCap = opts.blobCacheBytes ?? BLOB_CACHE_BYTES;
    this.autosaveMs = opts.autosaveMs ?? 0;
  }

  // --- Change signal -----------------------------------------------------------------------

  onChange(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /// Bumped whenever something the continuity snapshot (`draftSnapshot`) would record about the
  /// retained work changes: a record added, replaced or removed, new pixels, a status, an attempt,
  /// uncertainty, a minted retry identity, or a restored draft arriving or leaving. Blob arrivals,
  /// reads and every other display-only change leave it alone, so a listener can checkpoint the
  /// work without serializing the queue on each redraw.
  get workRevision(): number { return this.workRev; }
  /// Bumped only by new authoring (an edit queued, a stroke folded into a queued frame, a
  /// re-authored replacement). A departure compares this with the revision of the last sealed
  /// snapshot: background progress on work already preserved never counts as unpreserved work.
  get authorRevision(): number { return this.authorRev; }
  /// Called (synchronously, after the surfaces' change listeners) when `workRevision` moves.
  onWorkChange(fn: () => void): () => void {
    this.workListeners.add(fn);
    return () => this.workListeners.delete(fn);
  }
  private authored(): void { this.authorRev++; }

  private notify(): void {
    // The queue entries are mutated in place (their identity is what a retry resends); the
    // surfaces get fresh copies so a keyed row or a `find` never holds a stale object.
    this.saves = this.records.map(({ settle: _settle, ...rest }) => rest as SaveRecord);
    const pending = this.doc ? this.saves.filter(s => s.kind !== "applyIndex" && s.object === this.doc!.object) : [];
    this.workingModel = this.doc?.model ? (pending.length ? workingFlipnote(this.doc.model, pending, this.me) : this.doc.model) : null;
    this.workingScore = this.doc?.score ? (pending.length ? workingScore(this.doc.score, pending) : this.doc.score) : null;
    this.workingIndex = this.indexModel ? (this.saves.length ? workingIndex(this.indexModel, this.saves) : this.indexModel) : null;
    this.updateEditorMedia();
    const work = this.workFingerprint(), moved = work !== this.workKey;
    if (moved) { this.workKey = work; this.workRev++; }
    for (const fn of this.listeners) fn();
    if (moved) for (const fn of this.workListeners) fn();
  }

  /// A compact identity of what `draftSnapshot` would encode. Cheap on purpose (no pixel bytes,
  /// no JSON of operation bodies): it runs on every notify, including during drawing.
  private workFingerprint(): string {
    // No work at all is one state, whichever channel is selected: switching channels with nothing
    // retained must not look like a change that needs a checkpoint.
    if (!this.records.length && !this.restoredDraft) return "";
    const parts: string[] = [];
    for (const s of this.records) {
      let identity = "";
      if (s.kind === "frame") {
        let pix = this.pixIds.get(s.pix);
        if (pix === undefined) { pix = ++this.pixSeq; this.pixIds.set(s.pix, pix); }
        identity = `${pix}:${s.published?.cid ?? ""}:${s.apply?.nonce ?? ""}:${s.overlay?.nonce ?? ""}:${s.overlayRetry ?? ""}`;
      } else if (s.kind === "overlayApply") identity = `${s.request?.nonce ?? ""}:${s.overlayRetry ?? ""}`;
      else if (s.kind === "recoveryApply") identity = s.applied ? "applied" : "";
      parts.push(`${s.id}:${s.status}:${s.attempts}:${s.mayHaveCommitted ? 1 : 0}:${s.epochChanged ? 1 : 0}:${identity}`);
    }
    const draft = this.restoredDraft;
    return `${this.scope?.server ?? ""}/${this.scope?.channel ?? ""}|${draft ? `${draft.scope.server}/${draft.scope.channel}/${draft.records.length}/${draft.records[0]?.id ?? 0}` : ""}|${parts.join(",")}`;
  }

  private updateEditorMedia(): void {
    const m = this.workingModel, score = this.workingScore, view = this.doc?.view;
    const inspection = this.overlay?.inspection;
    const snapshot = { key: canonicalJson({ object: this.doc?.object, trust: view ? [view.epochId, view.group, view.phase, view.awaitingTenureReceipt] : null,
      overlay: inspection?.kind === "local-draft" ? [inspection.contentEpoch, inspection.group, inspection.branch, inspection.basis] : null,
      flipnote: m ? [m.title, m.fps, m.titleConflicts, m.fpsConflicts, m.overCapCount, m.sound.patches, m.sound.score,
        [...m.sound.sfx].sort((a, b) => a.id.localeCompare(b.id)).map(s => [s.id, s.frame, s.patch, s.note, s.durationMs, s.conflicts]), m.sound.readiness,
        m.frames.map(f => [f.id, f.bytes, f.conflicts, f.insertions, f.overCap])] : null,
      score: score ? [score.title, score.bpm, score.bpb, score.steps, score.cells, score.patches, score.readiness] : null }),
      frames: (m?.frames ?? []).map(f => ({ id: f.id, cid: f.cid, bytes: this.doc ? this.unsavedPix(this.doc.object, f.id) ?? this.blobs.get(f.cid) : undefined })) };
    const previous = this.editorSnapshot;
    const changed = !previous || previous.key !== snapshot.key || snapshot.frames.some((f, i) => {
      const old = previous.frames[i];
      if (!old || old.id !== f.id) return true;
      // A successful Save substitutes an immutable native CID for the same owned PIX bytes.
      // Delivery status and that substitution must not restart/stop an already playing draft.
      if (old.bytes && f.bytes) return old.bytes !== f.bytes && (old.bytes.length !== f.bytes.length || old.bytes.some((b, n) => b !== f.bytes![n]));
      return old.cid !== f.cid;
    });
    this.editorSnapshot = snapshot;
    if (changed) this.editorEpoch++;
  }

  // --- Scope -------------------------------------------------------------------------------

  /// Point the session at a channel. Production callers must await saveBeforeLeaving first;
  /// retained work vetoes a scope replacement. Lock uses snapshot + reset instead. The immediate
  /// test seam retains its original reset behavior for scope-fencing regressions.
  setScope(scope: StudioScope | null): void {
    if (this.scope && scope && this.scope.server === scope.server && this.scope.channel === scope.channel) return;
    if (!this.scope && !scope) return;
    if (this.closeHeld) throw new StudioNativeError("The window is closing. Wait for saving to finish.");
    if (this.autosaveMs && this.records.length) throw new StudioNativeError("Save your Studio changes before changing channels.");
    const sync = scope ? this.syncByServer.get(scope.server) ?? null : null;
    this.clearScoped();
    // Sync belongs to the server, not a channel or editor mount. Restore its ordering watermark
    // across both channel navigation and A → B → A server navigation.
    this.syncObservation = sync;
    this.receivePaused = sync?.paused ?? false;
    this.receivePauseDetail = sync?.detail ?? "";
    this.scope = scope;
    this.generation++;
    this.installRestoredDraft();
    this.notify();
    if (scope) this.invalidate({ index: true });
  }

  /// The server this session holds unsaved work for: the scope's, when it has retained records (or
  /// `unstaged` says a stroke could not be staged into them), or else a restored draft's, which
  /// waits until its channel is the scope again. Null when there is none.
  workServer(unstaged = false): { server: number; restored: boolean } | null {
    if (this.scope && (this.records.length || unstaged)) return { server: this.scope.server, restored: false };
    if (this.restoredDraft) return { server: this.restoredDraft.scope.server, restored: true };
    return null;
  }

  /// The native server behind `server` is gone: the member left it, or it closed under this session
  /// (its actor stopped). Nothing can accept a Save to it now, so leaving it must never wait on one
  /// (review of d2548a37: the switch off a removed server ran the Save gate against the stopped
  /// actor, refused, and left the app on a server that no longer existed). This retires that scope
  /// without saving; scope changes are otherwise vetoed while records remain (setScope).
  ///
  /// `keep` turns the retained records into a restored draft, exactly as lock's snapshot does: it
  /// waits until that channel is open again, and an interrupted delivery comes back uncertain. A
  /// server whose actor closed is not restarted by a lock and unlock (the node stays online and
  /// lists it again, still stopped), so in practice that is after Mewtual restarts; the draft is in
  /// the continuity snapshot until then. Like any waiting draft it refuses new Studio edits
  /// meanwhile, and `discardWaitingDraft` is the member's way out. Without `keep` the records are
  /// dropped, and so is a waiting restored draft for that server, which a server the member left
  /// could never install. "partial" means kept, except a stroke the full queue refused to stage.
  /// Lock resets the session itself, so callers skip this while locked. Window close may be under
  /// way: its hold on new input is kept, and a snapshot it takes later includes the kept draft.
  retireServer(server: number, keep: boolean): "none" | "kept" | "partial" | "dropped" {
    let outcome: "none" | "kept" | "partial" | "dropped" = "none";
    if (this.scope?.server === server) {
      let lostStroke = false;
      try { this.stageDrawing(); } catch { lostStroke = true; } // a stroke the full queue refused goes with the scope
      let draft: StudioDraft | null = null;
      if (keep && this.records.length) {
        // The same encoding lock uses, so what is kept is exactly what a lock would have kept.
        try { const json = this.draftSnapshot(); draft = json ? decodeStudioDraft(json) : null; } catch { draft = null; }
      }
      if (draft) outcome = lostStroke ? "partial" : "kept";
      else if (this.records.length || lostStroke) outcome = "dropped";
      const closing = this.closeHeld;
      this.clearScoped(); // settles awaited Saves as discarded and retires the old workers
      this.closeHeld = closing; // clearScoped releases a window close's input hold; this is not that close ending
      this.scope = null;
      this.generation++; // ends every navigation lease and in-flight read of the old scope
      // Records and a waiting draft never coexist (restoreDraft and enqueue refuse it), so this
      // cannot replace another server's draft.
      if (draft) this.restoredDraft = draft;
    }
    if (!keep && this.restoredDraft?.scope.server === server) { this.restoredDraft = null; outcome = "dropped"; }
    this.syncByServer.delete(server);
    this.notify();
    return outcome;
  }

  /// A restored draft still waiting for its channel (null when there is none or it has installed):
  /// where it belongs, its first record's id (an identity for confirmations), how many edits it
  /// holds and how many of those were interrupted deliveries that may already have landed. While
  /// one waits, new edits are refused.
  get waitingDraft(): { server: number; channel: string; first: number; edits: number; uncertain: number } | null {
    const draft = this.restoredDraft;
    return draft ? { server: draft.scope.server, channel: draft.scope.channel, first: draft.records[0]?.id ?? 0, edits: draft.records.length,
      uncertain: draft.records.filter(r => r.status === "inflight" || r.mayHaveCommitted).length } : null;
  }

  /// The member's explicit discard of a waiting restored draft: its channel may never open again
  /// (a stopped server lists again after unlock but takes nothing until Mewtual restarts; a deleted
  /// channel never does), and while it waits every new Studio edit is refused. Nothing was sent.
  discardWaitingDraft(): void {
    if (!this.restoredDraft) return;
    this.restoredDraft = null;
    this.notify();
  }

  /// Lock or session change: forget everything, including cached pixels and unsaved work.
  reset(): void {
    this.clearScoped();
    this.syncByServer.clear(); // pause details are private session state, cleared on lock
    this.scope = null;
    this.restoredDraft = null;
    this.generation++;
    this.blobs.clear();
    this.cacheBytes = 0;
    this.notify();
  }

  private clearScoped(): void {
    this.closeHeld = false;
    this.preferring.length = 0; // every awaited Save of the old scope rejects on the generation change
    this.cancelAutosave?.(); this.cancelAutosave = null; this.eligible.clear();
    this.restoredHeld.clear(); this.editorSnapshot = null;
    this.draftGroup = null;
    this.docReadSerial.clear(); this.indexReadSerial = 0;
    this.mediaEpoch++;
    this.index = null;
    this.indexModel = null;
    this.indexLoading = false;
    this.indexError = "";
    this.doc = null;
    this.indexOverlay = null;
    this.overlay = null;
    this.known.clear();
    this.views.clear();
    this.overlayCopies.clear();
    for (const s of this.records) s.settle?.("discarded");
    this.records.length = 0;
    this.workers.clear(); this.running.clear(); // old workers find their tokens gone and stand down
    this.cooling.clear(); this.busyResends.clear();
    this.receivePaused = false;
    this.receivePauseDetail = ""; this.syncObservation = null;
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
    this.recoveryFamily = "index";
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
    this.refreshes.clear(); // a read of the old scope still out must not hold back the new scope's
    this.blobFocus = new Set();
    if (this.cancelFlush) { this.cancelFlush(); this.cancelFlush = null; }
  }

  private current(gen: number): boolean {
    return gen === this.generation && this.scope !== null;
  }
  private target(): StudioScope {
    if (!this.scope) throw new StudioNativeError("no channel selected");
    return this.scope;
  }

  /** Kind comes from the selected/pinned Index target or retained request, never operation-body guessing. */
  objectKind(object: Hex32): StudioObjectKind {
    if (this.doc?.object === object) return this.doc.objectKind;
    const copy = this.overlayCopyFor(object);
    if (copy) return copy.objectKind ?? "flipnote";
    return this.retainedObjectKind(object) ?? this.views.get(object)?.view.content.kind
      ?? (this.recoveryTarget === object && this.recoveryFamily !== "index" ? this.recoveryFamily : undefined)
      ?? [...(this.indexModel?.entries ?? []), ...(this.indexModel?.overflow ?? [])].find((entry) => entry.id === object)?.kind ?? "flipnote";
  }

  private retainedObjectKind(object: Hex32): StudioObjectKind | undefined {
    const retained = this.records.find((save) => save.kind !== "applyIndex" && save.object === object);
    if (retained?.kind === "apply" || retained?.kind === "create") return retained.request.objectKind ?? "flipnote";
    if (retained?.kind === "overlayApply" || retained?.kind === "recoveryApply") return retained.objectKind ?? "flipnote";
    return retained?.kind === "frame" ? "flipnote" : undefined;
  }

  private objectTarget(object: Hex32 | null): RecoveryTargetArgs {
    return object === null ? { ...this.target(), object } : { ...this.target(), object,
      ...(this.objectKind(object) === "score" ? { objectKind: "score" as const } : {}) };
  }

  // --- Events --------------------------------------------------------------------------------

  /// Install the three native listeners once; the returned function removes them.
  async attach(): Promise<() => void> {
    const handles = await Promise.all([
      this.ipc.listen<unknown>(STUDIO_UPDATED_EVENT, (e) => this.onStudioUpdated(e.payload)),
      this.ipc.listen<unknown>(STUDIO_RECEIVE_PAUSED_EVENT, (e) => this.onReceivePaused(e.payload)),
      this.ipc.listen<unknown>(STUDIO_SYNC_STATE_EVENT, (e) => this.onStudioSyncState(e.payload)),
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
    // Native names the target the accepted operation changed (actor.rs, `AppEvent::StudioUpdated`):
    // `object: null` is the channel Index, a named object is that document alone. A note or a
    // frame therefore does not re-read the Index, which costs a list, lifecycle and overlay read
    // (and, natively, a cold Index reconstruction while a document owns the warm graph). The one
    // document event that changes the Index too is a Create, whose event names only the new
    // document; an object the Index does not list yet re-reads it, and so does our own Create
    // (run() invalidates the Index after its receipt).
    // A blob that was unavailable or failed may have arrived with it: let the next request try.
    if (ev.object) this.releaseTransientFor(ev.object);
    const listed = ev.object !== null && [...(this.indexModel?.entries ?? []), ...(this.indexModel?.overflow ?? [])].some((entry) => entry.id === ev.object);
    // Data notices may echo our just-acknowledged Save. Re-read them, then compare effective
    // content before stopping local playback. Settlement/membership invalidations still revoke
    // immediately; an unchanged delivery echo is not a new animation source.
    this.invalidate({ index: !listed, objects: ev.object ? [ev.object] : [], dataNotice: true });
  }
  onReceivePaused(payload: unknown): void {
    let ev;
    try { ev = parseReceivePaused(payload); } catch { return; }
    if (!this.scope || ev.server !== this.scope.server || this.syncObservation) return;
    this.receivePaused = true;
    this.notify();
  }
  onStudioSyncState(payload: unknown): void {
    let event;
    try { event = parseStudioSyncState(payload); } catch { return; }
    if (!this.scope || event.server !== this.scope.server) return;
    const previous = this.syncObservation;
    // Incarnations are native monotonic ids; actor sequences restart only with a new actor.
    // A delayed old pause or Read ACK must never overwrite the latest observed sync state.
    if (previous && (BigInt(event.instance) < BigInt(previous.instance) ||
      (event.instance === previous.instance && BigInt(event.sequence) <= BigInt(previous.sequence)))) return;
    this.syncObservation = event;
    this.syncByServer.delete(event.server);
    this.syncByServer.set(event.server, event);
    if (this.syncByServer.size > 128) this.syncByServer.delete(this.syncByServer.keys().next().value!);
    this.receivePaused = event.paused;
    this.receivePauseDetail = event.detail ?? "";
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

  invalidate(what: { index?: boolean; objects?: Hex32[]; recovery?: boolean; dataNotice?: boolean }): void {
    if (what.objects?.length) { this.mediaEpoch++; if (!what.dataNotice) this.editorEpoch++; }
    if (what.index || what.objects?.length || what.recovery) { this.faultEpoch++; this.notify(); }
    if (what.index) this.pendingIndex = true;
    for (const o of what.objects ?? []) this.pendingObjects.add(o);
    if (what.recovery) this.pendingRecovery = true;
    if (!this.cancelFlush) this.cancelFlush = this.schedule(() => this.flush(), REFRESH_COALESCE_MS);
  }

  /// Run the coalesced re-reads now (the timer calls this; tests call it directly).
  flush(): void {
    if (this.cancelFlush) { this.cancelFlush(); this.cancelFlush = null; }
    if (!this.scope) { this.pendingIndex = false; this.pendingObjects.clear(); this.pendingRecovery = false; return; }
    if (this.pendingIndex) { this.pendingIndex = false; this.coalesceRefresh("index", () => this.refreshIndex()); }
    for (const o of this.pendingObjects) {
      if (this.doc?.object === o) this.coalesceRefresh(`doc:${o}`, () => this.doc?.object === o ? this.refreshDoc(o) : Promise.resolve());
      else this.known.delete(o); // its trust state is no longer known until read again
    }
    this.pendingObjects.clear();
    if (this.pendingRecovery) { this.pendingRecovery = false; void this.refreshRecovery(); }
  }

  /// One invalidation-driven read per target at a time. The coalescing timer above only merges
  /// events that arrive within one window; it never knew whether the previous window's reads had
  /// come back, so a slow native answer let every later window start another full read of the
  /// same target (list or document, then lifecycle, then overlay). A read already out records the
  /// newer invalidation and runs exactly once more after it ends, so the final read still starts
  /// after the last event. Direct callers (admission reads, receipt reconciliation, explicit
  /// access) still call refreshIndex/refreshDoc themselves: they need a read that starts now, and
  /// their request generation supersedes this one exactly as before.
  private coalesceRefresh(key: string, read: () => Promise<void>): void {
    const held = this.refreshes.get(key);
    if (held) { held.again = true; return; }
    const entry = { again: false };
    this.refreshes.set(key, entry);
    void read().catch(() => {}).finally(() => {
      // A cleared scope owns a different map entry (or none); this read's flag is not its business.
      if (this.refreshes.get(key) !== entry) return;
      this.refreshes.delete(key);
      if (entry.again && this.scope) this.coalesceRefresh(key, read);
    });
  }

  async refreshIndex(): Promise<void> {
    this.indexReadSerial++;
    const gen = this.generation;
    const req = ++this.indexReq;
    const target = this.target();
    this.indexLoading = true;
    this.notify();
    try {
      const view = await this.unlessBusy(() => studioList(this.ipc, target), () => this.current(gen) && req === this.indexReq);
      if (!this.current(gen) || req !== this.indexReq) return;
      this.index = view;
      this.indexModel = indexModel(view.content);
      this.indexError = "";
      if (!this.syncObservation) this.receivePaused = false; // legacy bridge only
      this.noteEpoch(null, view.epochId);
    } catch (e) {
      if (!this.current(gen) || req !== this.indexReq) return;
      this.indexError = reason(e);
    } finally {
      if (this.current(gen) && req === this.indexReq) { this.indexLoading = false; this.notify(); }
    }
    if (this.current(gen) && req === this.indexReq) await this.refreshIndexOverlay();
  }

  private async refreshIndexOverlay(): Promise<void> {
    const gen = this.generation;
    const req = ++this.indexOverlayReq;
    const target = { ...this.target(), object: null };
    this.indexOverlay = this.indexOverlay
      ? { ...this.indexOverlay, loading: true }
      : { object: null, inspection: null, lifecycle: null, archive: null, exported: null, copy: this.overlayCopyFor(null), loading: true, error: "" };
    this.notify();
    try {
      const lifecycle = await overlayLifecycle(this.ipc, target);
      const inspection = await overlayRead(this.ipc, target);
      const archive = lifecycle.archive ? await overlayArchiveRead(this.ipc, target) : null;
      if (!this.current(gen) || req !== this.indexOverlayReq) return;
      this.indexOverlay = {
        object: null,
        lifecycle,
        inspection,
        archive,
        exported: this.indexOverlay?.exported ?? null,
        copy: this.overlayCopyFor(null),
        loading: false,
        error: "",
      };
      if (inspection.kind === "local-draft" && inspection.content) {
        if (inspection.content.kind !== "index") throw new StudioNativeError("local draft has Flipnote content for the Index target");
        this.indexModel = indexModel(inspection.content);
      } else if (this.index) {
        this.indexModel = indexModel(this.index.content);
      }
      this.notify();
    } catch (e) {
      if (!this.current(gen) || req !== this.indexOverlayReq) return;
      this.indexOverlay = { ...(this.indexOverlay ?? { object: null, inspection: null, lifecycle: null, archive: null, exported: null, copy: this.overlayCopyFor(null) }), object: null, copy: this.overlayCopyFor(null), loading: false, error: reason(e) };
      this.notify();
    }
  }

  /// Open (or re-read) a flipnote. Opening a different object drops the previous document but
  /// keeps its pending saves: they are addressed by object and finish or fail on their own.
  open(object: Hex32 | null, opts: { read?: boolean; objectKind?: StudioObjectKind } = {}): void {
    if (this.closeHeld && this.doc?.object !== object) throw new StudioNativeError("The window is closing. Wait for saving to finish.");
    if (this.doc?.object !== object) this.cancelNavigation();
    if (this.doc && this.doc.object !== object) this.prepareDocumentClose();
    if (this.doc?.object !== object) this.mediaEpoch++;
    if (object === null) {
      if (this.doc || this.overlay) { this.doc = null; this.overlay = null; this.notify(); }
      return;
    }
    const objectKind = opts.objectKind ?? this.objectKind(object);
    const retainedKind = this.retainedObjectKind(object);
    const retainedCopy = this.overlayCopyFor(object);
    const knownKind = this.views.get(object)?.view.content.kind
      ?? retainedKind
      ?? (retainedCopy ? retainedCopy.objectKind ?? "flipnote" : undefined)
      ?? (this.overlay?.object === object && this.overlay.inspection?.kind === "local-draft" && this.overlay.inspection.content?.kind !== "index" ? this.overlay.inspection.content?.kind : undefined)
      ?? (this.overlay?.object === object && this.overlay.copy ? this.overlay.copy.objectKind ?? "flipnote" : undefined)
      ?? [...(this.indexModel?.entries ?? []), ...(this.indexModel?.overflow ?? [])].find((entry) => entry.id === object)?.kind;
    if (knownKind !== undefined && knownKind !== objectKind) throw new StudioNativeError("an object's known immutable family cannot be changed");
    if (this.doc?.object !== object || this.doc.objectKind !== objectKind) {
      this.docReq.set(object, (this.docReq.get(object) ?? 0) + 1);
      this.overlayReq.set(object, (this.overlayReq.get(object) ?? 0) + 1);
      if (this.recoveryTarget === object && this.recoveryFamily !== objectKind) {
        this.recoveryFamily = objectKind;
        this.recoveryReq++; this.activeRun = null; this.recoveryRun = null; this.recoveryListing = null;
        this.recoveryError = ""; this.recoveryLoading = false;
        this.versions.clear(); this.exports.clear();
        this.pointer = { status: "idle", error: "", result: null };
      }
      this.doc = { object, objectKind, view: null, model: null, score: null, loading: opts.read === false, absent: false, error: "" };
      this.overlay = { object, inspection: null, lifecycle: null, archive: null, exported: null, copy: this.overlayCopyFor(object), loading: opts.read !== false, error: "" };
      this.notify();
    }
    // A just-created object is filled by its Create result; reading it first would only
    // report the absence that preceded the save.
    if (opts.read !== false) void this.refreshDoc(object);
  }

  async refreshDoc(object: Hex32): Promise<void> {
    this.docReadSerial.set(object, (this.docReadSerial.get(object) ?? 0) + 1);
    const gen = this.generation;
    const req = (this.docReq.get(object) ?? 0) + 1;
    this.docReq.set(object, req);
    const target = this.target();
    this.patchDoc(object, { loading: true });
    try {
      const objectKind = this.objectKind(object);
      const view = await this.unlessBusy<StudioObjectView | null>(() => objectKind === "score" ? studioRead(this.ipc, target, object, "score") : studioRead(this.ipc, target, object),
        () => this.current(gen) && req === this.docReq.get(object));
      if (!this.current(gen) || req !== this.docReq.get(object)) return;
      const model = view?.content.kind === "flipnote" ? flipnoteModel(view.content) : null;
      const score = view?.content.kind === "score" ? scoreModel(view.content) : null;
      if (view) {
        this.known.set(object, { awaiting: view.awaitingTenureReceipt, phase: view.phase, epoch: view.epoch });
        this.rememberView(object, view, model, score);
        this.noteEpoch(object, view.epochId);
      } else { this.known.delete(object); this.views.delete(object); }
      if (!this.syncObservation) this.receivePaused = false;
      this.patchDoc(object, { view, model, score, absent: view === null, error: "", loading: false });
    } catch (e) {
      if (!this.current(gen) || req !== this.docReq.get(object)) return;
      this.patchDoc(object, { error: reason(e), loading: false });
    }
    // `studio_read` and `studio_overlay_read` share the native latest-view fence. Awaiting this
    // second read, rather than starting both from the surface, ensures neither legitimate result
    // suppresses the other merely because the renderer raced them.
    if (this.current(gen) && req === this.docReq.get(object) && this.doc?.object === object) {
      await this.refreshOverlay(object);
    }
  }

  /// A read the bridge refused before dispatch (nativeNoWrite: slots full, actor busy, storage busy)
  /// was never served, so it is asked again after the same short backoffs a Save uses, while the
  /// caller's read is still the current one. Any other failure, or a superseded read, ends it.
  private async unlessBusy<T>(call: () => Promise<T>, live: () => boolean): Promise<T> {
    for (let round = 0; ; round++) {
      try { return await call(); }
      catch (e) {
        if (!nativeNoWrite(e) || round >= BUSY_BACKOFF_MS.length || !live()) throw e;
        await new Promise<void>((resolve) => { this.schedule(resolve, BUSY_BACKOFF_MS[round]); });
        if (!live()) throw e;
      }
    }
  }

  /// Replace the open document's state (never mutate it) when it is this object.
  private patchDoc(object: Hex32, patch: Partial<DocState>): void {
    if (this.doc?.object !== object) return;
    if (patch.view && patch.view.content.kind !== this.doc.objectKind) return;
    this.doc = { ...this.doc, ...patch };
    this.notify();
  }

  // --- Retained local draft -----------------------------------------------------------------

  /// Reconcile the live branch, lifecycle terminal and optional archive as one UI observation.
  /// These are deliberately not inferred from the ordinary view: a branch may outlive a source,
  /// and a terminal disposition must remain visible instead of collapsing to "absent".
  async refreshOverlay(object: Hex32): Promise<void> {
    const gen = this.generation;
    const req = (this.overlayReq.get(object) ?? 0) + 1;
    this.overlayReq.set(object, req);
    const target = this.objectTarget(object);
    const objectKind = target.objectKind ?? "flipnote";
    const live = () => this.current(gen) && req === this.overlayReq.get(object) && this.doc?.object === object && this.doc.objectKind === objectKind;
    if (this.overlay?.object !== object) {
      this.overlay = { object, inspection: null, lifecycle: null, archive: null, exported: null, copy: this.overlayCopyFor(object), loading: true, error: "" };
    } else {
      this.overlay = { ...this.overlay, loading: true };
    }
    this.notify();
    try {
      // The lifecycle row tells us whether an archive exists. Read it before the branch view so
      // the latter remains the final latest-view operation at native delivery.
      const lifecycle = await overlayLifecycle(this.ipc, target);
      if (!live()) return;
      const inspection = await overlayRead(this.ipc, target);
      if (!live()) return;
      const archive = lifecycle.archive ? await overlayArchiveRead(this.ipc, target) : null;
      if (!live()) return;
      this.overlay = {
        object,
        lifecycle,
        inspection,
        archive,
        exported: this.overlay?.object === object ? this.overlay.exported : null,
        copy: this.overlayCopyFor(object),
        loading: false,
        error: "",
      };
      // A replayable local draft is what the member most recently saved. Display that projection
      // while keeping the ordinary preview/source view untouched for trust and routing decisions.
      if (inspection.kind === "local-draft" && inspection.content) {
        if (inspection.content.kind !== this.objectKind(object)) throw new StudioNativeError("local draft has the wrong object family");
        this.patchDoc(object, { model: inspection.content.kind === "flipnote" ? flipnoteModel(inspection.content) : null,
          score: inspection.content.kind === "score" ? scoreModel(inspection.content) : null });
      } else if (this.doc?.object === object) {
        const content = this.doc.view?.content;
        this.patchDoc(object, { model: content?.kind === "flipnote" ? flipnoteModel(content) : null,
          score: content?.kind === "score" ? scoreModel(content) : null });
      }
      this.notify();
    } catch (e) {
      if (!this.current(gen) || req !== this.overlayReq.get(object) || this.doc?.object !== object || this.doc.objectKind !== objectKind) return;
      this.overlay = { ...(this.overlay ?? { object, inspection: null, lifecycle: null, archive: null, exported: null, copy: this.overlayCopyFor(object) }), object, copy: this.overlayCopyFor(object), loading: false, error: reason(e) };
      this.notify();
    }
  }

  private overlayFor(object: Hex32 | null): OverlayState | null {
    return object === null ? this.indexOverlay : this.overlay?.object === object ? this.overlay : null;
  }

  private overlayIsCurrent(object: Hex32 | null): boolean {
    return object === null
      ? this.indexOverlay?.object === null
      : this.doc?.object === object && this.overlay?.object === object;
  }

  private replaceOverlay(object: Hex32 | null, state: OverlayState): void {
    if (object === null) this.indexOverlay = state;
    else if (this.doc?.object === object) this.overlay = state;
  }

  private overlayCopyKey(object: Hex32 | null): string {
    return object ?? "index";
  }

  private overlayCopyFor(object: Hex32 | null): OverlayCopyRun | null {
    return this.overlayCopies.get(this.overlayCopyKey(object)) ?? null;
  }

  private retainOverlayCopy(object: Hex32 | null, run: OverlayCopyRun): void {
    const key = this.overlayCopyKey(object);
    const prior = this.overlayCopies.get(key);
    if (prior && (prior.status === "previewing" || prior.status === "applying" || prior.status === "refused" || prior.status === "uncertain")) {
      throw new StudioNativeError("the previous local draft copy is unresolved; retry it before starting another");
    }
    if (!prior && this.overlayCopies.size >= RETAINED_OVERLAY_COPIES) {
      const disposable = [...this.overlayCopies].find(([, copy]) =>
        copy.status !== "previewing" && copy.status !== "applying" && copy.status !== "refused" && copy.status !== "uncertain");
      if (disposable) this.overlayCopies.delete(disposable[0]);
    }
    if (!prior && this.overlayCopies.size >= RETAINED_OVERLAY_COPIES) {
      throw new StudioNativeError("too many unresolved local draft copies; resolve one before starting another");
    }
    this.overlayCopies.delete(key);
    this.overlayCopies.set(key, run);
  }

  private publishOverlayCopy(object: Hex32 | null, run: OverlayCopyRun): void {
    if (this.overlayCopyFor(object) !== run) return;
    const overlay = this.overlayFor(object);
    if (overlay) this.replaceOverlay(object, { ...overlay, copy: run });
    this.notify();
  }

  private async refreshOverlayTarget(object: Hex32 | null): Promise<void> {
    if (object === null) await this.refreshIndexOverlay();
    else await this.refreshOverlay(object);
  }

  async exportOverlay(object: Hex32 | null): Promise<OverlayArchive | null> {
    const gen = this.generation;
    const exported = await overlayExport(this.ipc, this.objectTarget(object));
    if (!this.current(gen) || !this.overlayIsCurrent(object)) return null;
    const overlay = this.overlayFor(object);
    if (overlay) this.replaceOverlay(object, { ...overlay, exported, error: "" });
    this.notify();
    return exported;
  }

  async archiveOverlay(object: Hex32 | null): Promise<void> {
    const gen = this.generation;
    try {
      await overlayArchive(this.ipc, this.objectTarget(object));
    } finally {
      // Archive may have committed even when final delivery failed. Reconcile instead of
      // resending a write whose outcome is explicitly uncertain.
      if (this.current(gen) && this.overlayIsCurrent(object)) await this.refreshOverlayTarget(object);
    }
  }

  async exportOverlayArchive(object: Hex32 | null): Promise<OverlayArchive | null> {
    const gen = this.generation;
    const exported = await overlayArchiveExport(this.ipc, this.objectTarget(object));
    if (!this.current(gen) || !this.overlayIsCurrent(object)) return null;
    const overlay = this.overlayFor(object);
    if (overlay) this.replaceOverlay(object, { ...overlay, exported, error: "" });
    this.notify();
    return exported;
  }

  async releaseOverlayArchive(object: Hex32 | null): Promise<void> {
    const archive = this.overlayFor(object)?.archive?.archive;
    if (!archive) throw new StudioNativeError("no retained draft archive to release");
    const gen = this.generation;
    try {
      await overlayArchiveRelease(this.ipc, this.objectTarget(object), archive, "release-local-archive");
    } finally {
      if (this.current(gen) && this.overlayIsCurrent(object)) await this.refreshOverlayTarget(object);
    }
  }

  async disposeOverlay(object: Hex32 | null, mode: "preserve" | "discard"): Promise<void> {
    const branch = this.overlayFor(object)?.lifecycle?.branch;
    if (!branch) throw new StudioNativeError("no live local draft to dispose");
    const disposal: OverlayDisposalRequest = {
      branch: branch.branch,
      content: branch.content,
      accepted: branch.accepted,
      mode: mode === "preserve" ? { kind: "preserve" } : { kind: "discard", confirmation: "destroy-local-draft" },
    };
    const gen = this.generation;
    try {
      await overlayDispose(this.ipc, this.objectTarget(object), disposal);
    } finally {
      if (this.current(gen) && this.overlayIsCurrent(object)) await this.refreshOverlayTarget(object);
    }
  }

  overlayChoices(mode: RecoveryMode, object: Hex32 | null = this.overlay?.object ?? null): { choice: RecoveryChoice; label: string }[] {
    const inspection = this.overlayFor(object)?.inspection;
    return inspection?.kind === "local-draft" && inspection.content
      ? choicesForContent(inspection.content, mode)
      : [];
  }

  /// Preview and apply one retained value into the current ordinary document. The preview body,
  /// projection fingerprint and nonce are frozen before Apply; if delivery fails, `retryOverlayCopy`
  /// resends that exact edit and never asks for a different preview under the same UI action.
  async copyOverlayChoice(object: Hex32 | null, choice: RecoveryChoice, label: string, mode: RecoveryMode): Promise<void> {
    if (!this.overlayIsCurrent(object)) throw new StudioNativeError("local draft is not open");
    const gen = this.generation;
    const source = this.objectTarget(object);
    const destination = object === null ? { channel: source.channel } : { channel: source.channel, object,
      ...(source.objectKind === "score" ? { objectKind: "score" as const } : {}) };
    const run: OverlayCopyRun = {
      ...(source.objectKind === "score" ? { objectKind: "score" as const } : {}),
      choice, label, mode, status: "previewing", error: "", mayHaveCommitted: false, preview: null, edit: null, applied: null,
    };
    this.retainOverlayCopy(object, run);
    this.publishOverlayCopy(object, run);
    try {
      const inspection = this.overlayFor(object)?.inspection;
      const preview = await overlayCopyPreview(this.ipc, source, destination, choice, mode);
      if (!this.current(gen) || this.overlayCopyFor(object) !== run) return;
      run.preview = preview;
      if (preview.disposition !== "ready" || preview.body === null) {
        run.status = preview.disposition;
        this.publishOverlayCopy(object, run);
        return;
      }
      if (choice.kind === "soundRecord") {
        if (inspection?.kind !== "local-draft" || !inspection.content) throw new StudioNativeError("the local sound draft cannot be rebuilt");
        await this.holdRecoveryExport(inspection.content, choice, preview.body, source.server, inspection.group,
          () => this.current(gen) && this.overlayCopyFor(object) === run && this.overlayFor(object)?.inspection === inspection);
        if (!this.current(gen) || this.overlayCopyFor(object) !== run) return;
      }
      run.edit = {
        destination,
        choice,
        mode,
        epochId: preview.epochId,
        expectedProjection: preview.expectedProjection,
        nonce: this.newId(),
        body: preview.body,
      };
      await this.applyOverlayCopy(object, run, gen);
    } catch (e) {
      if (!this.current(gen) || this.overlayCopyFor(object) !== run) return;
      if (run.edit) run.mayHaveCommitted = true;
      run.status = run.mayHaveCommitted ? "uncertain" : "error";
      run.error = reason(e);
      this.publishOverlayCopy(object, run);
    }
  }

  /// Start a deliberately new Copy decision after C4 proved that the frozen proposal wrote
  /// nothing. This is not an exact retry: it obtains a fresh preview and later mints a fresh nonce.
  async repreviewOverlayCopy(object: Hex32 | null): Promise<void> {
    const run = this.overlayCopyFor(object);
    if (!run || run.status !== "refused" || run.mayHaveCommitted) return;
    if (!this.overlayIsCurrent(object)) throw new StudioNativeError("local draft is not open");
    this.overlayCopies.delete(this.overlayCopyKey(object));
    await this.copyOverlayChoice(object, run.choice, run.label, run.mode);
  }

  async retryOverlayCopy(object: Hex32 | null): Promise<void> {
    const run = this.overlayCopyFor(object);
    if (!run?.edit || run.status !== "uncertain") return;
    const gen = this.generation;
    try {
      await this.applyOverlayCopy(object, run, gen);
    } catch (e) {
      if (!this.current(gen) || this.overlayCopyFor(object) !== run) return;
      run.mayHaveCommitted = true;
      run.status = "uncertain";
      run.error = reason(e);
      this.publishOverlayCopy(object, run);
    }
  }

  /// Cancel a Copy only after C4 conclusively refused its exact proposal without writing it.
  /// This is intentionally separate from `forgetOverlayCopy`: an uncertain request may have
  /// committed and needs explicit stop-tracking consent, while a refused request has no native
  /// state to undo. The expected nonce fences a delayed click from cancelling a replacement run.
  cancelRefusedOverlayCopy(object: Hex32 | null, expectedNonce: string): boolean {
    const key = this.overlayCopyKey(object);
    const run = this.overlayCopies.get(key);
    if (!run || run.status !== "refused" || run.mayHaveCommitted || run.edit?.nonce !== expectedNonce) return false;
    this.overlayCopies.delete(key);
    const overlay = this.overlayFor(object);
    if (overlay?.copy === run) this.replaceOverlay(object, { ...overlay, copy: null });
    this.notify();
    return true;
  }

  /// Forget only the frontend's retained identity for an ambiguous Copy. This deliberately sends
  /// no native command: the operation may already have landed, and forgetting it neither undoes
  /// nor disproves that write. The explicit UI confirmation is the user's decision to stop exact
  /// retries and accept that uncertainty before this bounded slot can be reused.
  forgetOverlayCopy(object: Hex32 | null, expectedNonce: string): boolean {
    const key = this.overlayCopyKey(object);
    const run = this.overlayCopies.get(key);
    // Bind the confirmation to the exact operation the user saw. A late DOM event must not forget
    // a different uncertain run that replaced it under the same document key.
    if (!run || run.status !== "uncertain" || run.edit?.nonce !== expectedNonce) return false;
    this.overlayCopies.delete(key);
    const overlay = this.overlayFor(object);
    if (overlay?.copy === run) this.replaceOverlay(object, { ...overlay, copy: null });
    this.notify();
    return true;
  }

  private async applyOverlayCopy(object: Hex32 | null, run: OverlayCopyRun, gen: number): Promise<void> {
    if (!run.edit) throw new StudioNativeError("local draft copy has no prepared edit");
    run.status = "applying";
    run.error = "";
    this.publishOverlayCopy(object, run);
    const applied = await overlayCopyApply(this.ipc, { ...this.target(), object,
      ...(run.objectKind === "score" ? { objectKind: "score" as const } : {}) }, run.edit);
    if (!this.current(gen) || this.overlayCopyFor(object) !== run) return;
    if (applied.kind === "overlayCopyRefused") {
      if (run.mayHaveCommitted) {
        run.status = "uncertain";
        run.error = "the current proposal was refused before writing, but an earlier delivery may already have committed; retry the exact request or discard it";
      } else {
        run.status = "refused";
        run.error = "the destination changed after preview; preview again or cancel this copy";
      }
      this.publishOverlayCopy(object, run);
      return;
    }
    run.status = "applied";
    run.applied = applied;
    this.publishOverlayCopy(object, run);
    if (this.overlayIsCurrent(object)) {
      if (object === null) await this.refreshIndex();
      else await this.refreshDoc(object);
    }
  }

  /// A read carrying a different epoch than an uncertain save used marks that save.
  private noteEpoch(object: Hex32 | null, epochId: Hex32): void {
    for (const s of this.records) {
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
    return !this.closeHeld && !("refused" in this.editRoute(object));
  }

  /// Why the editor must be locked. Awaiting-tenure content is intentionally not a refusal now:
  /// edits are saved into a bounded local-only branch through the ticketed route, while the
  /// preview itself remains non-authoritative and cannot supply an ordinary epoch.
  editRefusal(object: Hex32 | null): string {
    if (this.closeHeld) return "Saving before closing…";
    const route = this.editRoute(object);
    return "refused" in route ? route.refused : "";
  }

  private editRoute(object: Hex32 | null): { kind: "ordinary"; epochId: Hex32 } | { kind: "unconfirmed" | "closing" } | { refused: string } {
    const view = object === null ? this.index : this.doc?.object === object ? this.doc.view : this.views.get(object)?.view ?? null;
    if (!view) return { refused: object === null ? "the channel index is not loaded" : "the flipnote is not loaded" };
    if (view.awaitingTenureReceipt) return { kind: "unconfirmed" };
    if (view.phase === "fault") return { refused: "history fault: conflicting owner receipts; this document is read-only" };
    if (view.phase === "closing") return { kind: "closing" };
    return { kind: "ordinary", epochId: view.epochId };
  }

  createFlipnote(title: string): Hex32 {
    return this.createObject(title, "flipnote");
  }

  createScore(title: string): Hex32 {
    return this.createObject(title, "score");
  }

  private createObject(title: string, objectKind: StudioObjectKind): Hex32 {
    // Create is a coordinated Index + new-document transaction, not one domain operation. The
    // Unconfirmed overlay route deliberately accepts only a single target operation, so a preview
    // cannot make Create appear atomic by saving only half of it.
    const epoch = this.editableEpoch(null);
    if (!("epochId" in epoch)) throw new StudioNativeError(epoch.refused);
    const object = this.newId();
    this.enqueue({ kind: "create", object, label: `new ${objectKind} "${title}"`, request: { object, nonce: this.newId(), title, createdAtMs: this.now(),
      ...(objectKind === "score" ? { objectKind } : {}) } });
    return object;
  }

  saveFrame(object: Hex32, frame: Hex32, pix: Uint8Array): void {
    if (this.closeHeld) throw new StudioNativeError("The window is closing. Wait for saving to finish.");
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("pixels require a Flipnote target");
    const route = this.editRoute(object);
    const routeKind = "refused" in route && this.records.some((s) => s.kind === "create" && s.object === object && (s.request.objectKind ?? "flipnote") === "flipnote")
      ? "ordinary"
      : "refused" in route ? null : route.kind;
    if (!routeKind) throw new StudioNativeError((route as { refused: string }).refused);
    // A newer raster coalesces only into the lane's TAIL when that is a save of this frame with
    // no apply identity yet (nothing of it can have committed). Anything else queued after it
    // in this lane (another frame, a chosen conflict value, an insert) keeps its place: the new
    // pixels are appended behind it, never moved ahead of it.
    let tail: SaveEntry | undefined;
    for (const s of this.records) if (laneOf(s) === object) tail = s;
    // A forced record is an explicit conflict choice: never fold new strokes into it.
    const prior = tail && tail.kind === "frame" && tail.frame === frame && tail.route === routeKind && !tail.force && tail.status === "queued" && tail.attempts === 0 && !this.eligible.has(tail.id) && !this.restoredHeld.has(tail.id) && tail.apply === null && tail.overlay === null ? tail : undefined;
    if (prior && prior.kind === "frame") {
      this.checkDraftBudget({ ...prior, pix }, prior);
      prior.pix = pix.slice();
      this.mediaEpoch++;
      prior.published = null;
      prior.error = "";
      prior.status = "queued";
      this.authored();
      this.notify();
      this.markDirty();
      return;
    }
    this.enqueue({ kind: "frame", object, frame, op: "replace", after: null, pix, published: null, route: routeKind, apply: null, overlay: null, overlayRetry: null, label: "frame pixels" });
  }

  insertFrame(object: Hex32, after: Hex32 | null, pix: Uint8Array): Hex32 {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("frames require a Flipnote target");
    const route = this.editRoute(object);
    const routeKind = "refused" in route && this.records.some((s) => s.kind === "create" && s.object === object && (s.request.objectKind ?? "flipnote") === "flipnote")
      ? "ordinary"
      : "refused" in route ? null : route.kind;
    if (!routeKind) throw new StudioNativeError((route as { refused: string }).refused);
    const frame = this.newId();
    this.enqueue({ kind: "frame", object, frame, op: "insert", after, pix, published: null, route: routeKind, apply: null, overlay: null, overlayRetry: null, label: "new frame" });
    return frame;
  }

  removeFrame(object: Hex32, frame: Hex32): void {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("frames require a Flipnote target");
    this.applyOp(object, { op: "remove_frame", frame }, "remove frame");
  }

  setTitle(object: Hex32, title: string): void {
    this.applyOp(object, { op: "set_header", field: "title", value: title }, "title");
    if (this.indexModel?.entries.some((e) => e.id === object) || this.indexModel?.overflow.some((e) => e.id === object)) {
      this.applyIndexOp({ op: "set_title", object, title }, object, "index title");
    }
  }

  setFps(object: Hex32, fps: number): void {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("fps requires a Flipnote target");
    this.applyOp(object, { op: "set_header", field: "fps", value: fps }, "fps");
  }

  setScoreHeader(object: Hex32, field: "bpm" | "bpb" | "steps", value: number): void {
    this.scoreOp(object, { op: "set_header", field, value }, `score ${field}`);
  }
  setScoreCell(object: Hex32, step: number, row: number, value: ScoreCell): void {
    this.scoreOp(object, { op: "set_cell", step, row, value }, `score cell ${step}:${row}`);
  }
  clearScoreCell(object: Hex32, step: number, row: number): void {
    this.scoreOp(object, { op: "clear_cell", step, row }, `clear score cell ${step}:${row}`);
  }
  setScorePatch(object: Hex32, patch: Hex64, descriptor: unknown): void {
    const checked = validateJamPatch(descriptor);
    if (!checked.ok) throw new StudioNativeError(checked.error);
    this.scoreOp(object, { op: "set_patch", patch, descriptor: checked.patch }, "score recipe");
  }
  removeScorePatch(object: Hex32, patch: Hex64): void {
    this.scoreOp(object, { op: "remove_patch", patch }, "remove score recipe");
  }
  private scoreOp(object: Hex32, op: ScoreOp, label: string): void {
    if (this.objectKind(object) !== "score") throw new StudioNativeError("score edit requires a pinned Score target");
    this.applyOp(object, op, label);
  }

  /** A component emits typed Score operations; family and exact retry identity stay session-owned. */
  applyScoreOp(object: Hex32, op: ScoreOp): void {
    if (this.objectKind(object) !== "score") throw new StudioNativeError("score edit requires a pinned Score target");
    if (op.op === "set_header" && op.field === "title") this.setTitle(object, op.value as string);
    else this.scoreOp(object, op, "score edit");
  }

  setSoundPatch(object: Hex32, patch: Hex64, descriptor: JamPatch): void {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("sound requires a Flipnote target");
    const checked = validateJamPatch(descriptor);
    if (!checked.ok) throw new StudioNativeError(checked.error);
    this.applyOp(object, { op: "set_patch", patch, descriptor: checked.patch }, "sound recipe");
  }
  removeSoundPatch(object: Hex32, patch: Hex64): void {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("sound requires a Flipnote target");
    this.applyOp(object, { op: "remove_patch", patch }, "remove sound recipe");
  }
  setSfx(object: Hex32, input: { id: Hex32; frame: Hex32; patch: Hex64; note: number; durationMs: number }): void {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("SFX requires a Flipnote target");
    if (!Number.isInteger(input.note) || input.note < 0 || input.note > 127 || !Number.isInteger(input.durationMs) || input.durationMs < 1 || input.durationMs > 600_000) {
      throw new StudioNativeError("SFX needs a MIDI note and an authored duration of 1–600000ms");
    }
    this.applyOp(object, { op: "set_sfx", sfx: input.id, frame: input.frame, patch: input.patch, note: input.note, duration_ms: input.durationMs }, "frame sound");
  }
  removeSfx(object: Hex32, sfx: Hex32): void {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("SFX requires a Flipnote target");
    this.applyOp(object, { op: "remove_sfx", sfx }, "remove frame sound");
  }
  linkScore(object: Hex32, score: Hex32 | null): void {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("a Score link requires a Flipnote target");
    this.applyOp(object, { op: "set_header", field: "score", value: score }, "linked Score");
  }

  /** Whether the document's own content has unsaved edits: exactly what its working model applies
   * (see notify). Index records about it (its listing title, its expiry) change no media, so they
   * never put playback or export on the draft path; their failures still refuse via pendingFor. */
  private hasDocumentDraft(object: Hex32): boolean {
    return this.records.some(s => s.kind !== "applyIndex" && s.object === object);
  }

  /** Capture the displayed projection, not a silently substituted ordinary base beneath a draft. */
  private mediaSource(object: Hex32, kind: StudioObjectKind): { view: StudioObjectView; server: number; assertCurrent: () => void } {
    const doc = this.doc, scope = this.target(), gen = this.generation, epoch = this.mediaEpoch, editor = this.editorEpoch;
    const draft = this.hasDocumentDraft(object);
    if (draft && !this.autosaveMs) throw new StudioNativeError("finish or reconcile this document's pending saves before playback/export");
    if (!doc || doc.object !== object || doc.objectKind !== kind || !doc.view || doc.loading || this.overlay?.loading) throw new StudioNativeError("wait for this document to finish loading");
    if (this.pendingFor(object).some(s => s.status === "blocked" || s.status === "uncertain" || s.mayHaveCommitted)) throw new StudioNativeError("resolve this document's failed save before playback/export");
    const base = doc.view, model = doc.model, score = doc.score, inspection = this.overlay?.inspection;
    let view = base;
    if (inspection?.kind === "local-draft") {
      if (!inspection.replayable || !inspection.content) throw new StudioNativeError("the retained local draft cannot be rebuilt; export its private recovery archive instead");
      if (!inspection.contentEpoch || !inspection.group) throw new StudioNativeError("reopen this local draft to obtain its actual native epoch/group before playback/export");
      if (inspection.content.kind !== kind) throw new StudioNativeError("local draft has the wrong media family");
      view = { ...base, epoch: inspection.contentEpoch, group: inspection.group, content: inspection.content } as StudioObjectView;
    }
    const assertCurrent = () => {
      if (!this.current(gen) || this.scope?.server !== scope.server || this.scope?.channel !== scope.channel || editor !== this.editorEpoch ||
          this.doc?.object !== object || this.doc.objectKind !== kind || this.pendingFor(object).some(s => s.status === "blocked" || s.status === "uncertain" || s.mayHaveCommitted) || (!draft && (epoch !== this.mediaEpoch || this.doc.view !== base || this.doc.model !== model || this.doc.score !== score ||
          this.overlay?.inspection !== inspection || this.doc.loading || this.overlay?.loading))) {
        throw new StudioNativeError("the displayed source changed; start playback/export again");
      }
    };
    assertCurrent();
    return { view, server: scope.server, assertCurrent };
  }

  private oneMedia<T>(run: () => Promise<T>): Promise<T> {
    if (this.mediaTask) return Promise.reject(new StudioNativeError("another media preparation is still running"));
    const task = run();
    this.mediaTask = task;
    void task.finally(() => { if (this.mediaTask === task) this.mediaTask = null; }).catch(() => {});
    return task;
  }

  /** Recovery declarations do not prove blob availability. Hold the exact historical PIXA through
   * the existing native validator/publication before Apply; never put packages in the PIX cache.
   * Navigation may leave an idempotently published historical blob, but can never redirect Save. */
  private async holdRecoveryExport(content: StudioContent, choice: RecoveryChoice, body: string, server: number, group: string | undefined, live: () => boolean): Promise<void> {
    const op = historicalSoundOperation(content, choice);
    if (choice.kind === "soundRecord" && !op) throw new StudioNativeError("the historical sound operation is not in this version");
    if (op?.op !== "set_export") return;
    if (canonicalJson(JSON.parse(body)) !== canonicalJson(op)) throw new StudioNativeError("the export preview differs from its historical value");
    if (!group) throw new StudioNativeError("reopen this historical sound source to obtain its native group before recovering PIXA");
    if (op.bytes < 1 || op.bytes > PIXA_MAX_BYTES) throw new StudioNativeError("the historical PIXA declaration exceeds its media bound");
    if (this.mediaWrite) throw new StudioNativeError("another media save is still running");
    const assertLive = () => { if (!live()) throw new StudioNativeError("the recovery source changed; no export reference was applied"); };
    this.mediaWrite = true;
    try {
      await this.oneMedia(async () => {
        assertLive();
        const bytes = await requestBlobBounded(this.ipc, server, op.cid, op.bytes);
        assertLive();
        if (!bytes) throw new StudioNativeError("historical PIXA unavailable; reference not applied");
        const value = decodePixa(bytes);
        if (value.take.group !== group) throw new StudioNativeError("historical PIXA belongs to another group; reference not applied");
        const publication = await publishStudioPixa(this.ipc, server, bytes);
        assertLive();
        if (publication.cid !== op.cid || publication.bytes !== op.bytes) throw new StudioNativeError("held PIXA does not name the historical CID and length; reference not applied");
      });
    } finally { this.mediaWrite = false; }
  }

  async prepareScoreTake(object: Hex32): Promise<JamTake> {
    return (await this.prepareScorePlayback(object)).take;
  }

  /** The same take plus its authored grid end, which Studio looping must honour. */
  prepareScorePlayback(object: Hex32): Promise<ScorePlayback> {
    return this.oneMedia(async () => {
      const source = this.mediaSource(object, "score");
      const playback = this.hasDocumentDraft(object) && this.workingScore
        ? await exportScorePlayback(this.scoreInput(object, this.workingScore), this.mediaContext(source.view))
        : await scorePlaybackFromNative(object, source.view as ScoreView, this.me);
      source.assertCurrent();
      return playback;
    });
  }

  private async preparePixa(object: Hex32): Promise<{ bytes: Uint8Array; server: number; assertCurrent: () => void }> {
    const source = this.mediaSource(object, "flipnote"), view = source.view as FlipnoteView;
    const draft = this.hasDocumentDraft(object);
    const model = draft ? this.workingModel! : flipnoteModel(view.content);
    if (!draft || view.content.timeline.length) preflightFlipnoteMedia(view);
    else if (Object.keys(view.content.frames).length) throw new StudioNativeError("The animation timeline is incomplete.");
    if (!model || model.overCapCount || model.titleConflicts.length || model.fpsConflicts.length || !model.sound.ready ||
        model.frames.some(f => f.conflicts.length || f.insertions > 1)) throw new StudioNativeError("Resolve animation conflicts before playing or exporting.");
    // The unsaved pixels the captured model names, taken in this same turn, before the first await.
    // A draft frame's cid is a `draft:` placeholder that only the live queue can resolve, and an
    // autosave receipt landing during an await below removes that record; looking it up then fell
    // through to fetching the placeholder as a CID (review of d2548a37). The bytes are immutable.
    const unsaved = new Map(model.frames.map(f => [f.id, this.unsavedPix(object, f.id)] as const));
    let linked: { object: string; view: ScoreView } | undefined;
    if (model.sound.score) {
      if (this.hasDocumentDraft(model.sound.score)) throw new StudioNativeError("Save your music changes before playing this flipnote.");
      const score = await studioRead(this.ipc, this.target(), model.sound.score, "score");
      source.assertCurrent();
      if (!score) throw new StudioNativeError("the linked Score is not held locally; no shorter silent export was made");
      if (score.channel !== view.channel || !score.group || score.group !== view.group) throw new StudioNativeError("The linked music belongs to a different server or channel.");
      linked = { object: model.sound.score, view: score };
    }
    const held = new Map<string, Uint8Array>();
    for (const frame of model.frames) {
      const key = contentKey(frame.cid, frame.bytes);
      if (held.has(key)) continue;
      const bytes = unsaved.get(frame.id) ?? await this.fetchNow(frame.cid, frame.bytes);
      source.assertCurrent();
      if (!bytes) throw new StudioNativeError("an animation frame is unavailable; no partial export was made");
      held.set(key, bytes);
    }
    const bytes = draft ? await exportFlipnotePixa({ id: object, epoch: BigInt(view.epoch), fps: model.fps,
      frames: model.frames.map(f => ({ id: f.id, bytes: held.get(contentKey(f.cid, f.bytes))! })),
      patches: model.sound.patches, ...(linked ? { score: this.scoreInput(linked.object, scoreModel(linked.view.content)) } : {}),
      sfx: model.sound.sfx.map(s => ({ id: s.id, frame: s.frame, patch: s.patch, note: s.note, durationMs: s.durationMs })) }, this.mediaContext(view))
      : await flipnotePixaFromNative(object, view, held, this.me, linked);
    source.assertCurrent();
    return { bytes, server: source.server, assertCurrent: source.assertCurrent };
  }

  private mediaContext(view: StudioObjectView): { group: string; exporter: string } {
    if (!view.group) throw new StudioNativeError("Reopen this document before playing or exporting.");
    return { group: view.group, exporter: this.me };
  }
  private scoreInput(object: string, model: ScoreModel): ScoreExportInput {
    if (!model.ready || Object.values(model.cellSlots).some(s => !s.removed && s.register.conflicts.length) ||
        Object.values(model.patchSlots).some(s => !s.removed && s.register.conflicts.length)) throw new StudioNativeError("Resolve music conflicts before playing or exporting.");
    return { id: object, bpm: model.bpm, bpb: model.bpb, steps: model.steps, cells: model.cells, patches: model.patches };
  }

  prepareFlipnotePixa(object: Hex32): Promise<Uint8Array> {
    return this.oneMedia(async () => (await this.preparePixa(object)).bytes);
  }

  /** Read the actual immutable record bytes; its sealed blob CID is not a fileshare file CID. */
  preparePublishedPixa(object: Hex32, exportId: Hex32): Promise<Uint8Array> {
    return this.oneMedia(async () => {
      const source = this.mediaSource(object, "flipnote"), view = source.view as FlipnoteView;
      const record = flipnoteModel(view.content).sound.exports.find((record) => record.id === exportId);
      if (!record) throw new StudioNativeError("the published animation reference is no longer active");
      // The one-preparation slot is the bound for this larger media request. Do not feed PIXA
      // through the thumbnail scheduler: that cache deliberately accepts PIX1 frames only.
      const bytes = await requestBlobBounded(this.ipc, source.server, record.cid, record.bytes);
      source.assertCurrent();
      if (!bytes) throw new StudioNativeError("the exact published animation package is unavailable");
      const packageValue = decodePixa(bytes);
      if (!view.group || packageValue.take.group !== view.group) throw new StudioNativeError("published animation package belongs to another group");
      return encodePixa(packageValue);
    });
  }
  removePublishedPixa(object: Hex32, exportId: Hex32): void {
    if (this.objectKind(object) !== "flipnote") throw new StudioNativeError("an export reference requires a Flipnote target");
    this.applyOp(object, { op: "remove_export", export: exportId }, "remove published animation reference");
  }

  /** Blob custody and the editable export reference are deliberately two different outcomes. */
  async publishFlipnotePixa(object: Hex32): Promise<StudioPixaPublished> {
    if (this.mediaWrite) throw new StudioNativeError("another media save is still running");
    this.mediaWrite = true;
    try {
      const source = await this.oneMedia(() => this.preparePixa(object));
      source.assertCurrent();
      const publication = await publishStudioPixa(this.ipc, source.server, source.bytes);
      try {
        source.assertCurrent();
        const save = this.applyOp(object, { op: "set_export", export: this.newId(), cid: publication.cid, bytes: publication.bytes }, "published animation reference");
        return { publication, referenceSaveId: save.id, warning: null };
      } catch (e) {
        return { publication, referenceSaveId: null, warning: `blob published, but its Studio reference was not saved: ${reason(e)}` };
      }
    } finally { this.mediaWrite = false; }
  }

  async exportFlipnoteGif(object: Hex32): Promise<StudioGifSaved> {
    if (this.mediaWrite) throw new StudioNativeError("another media save is still running");
    this.mediaWrite = true;
    try {
      const source = await this.oneMedia(() => this.preparePixa(object));
      source.assertCurrent();
      // Native owns server-incarnation and worker fences; a successful file remains a saved file
      // even if the surface navigates away while the operating system reveals it.
      return await exportStudioGif(this.ipc, source.server, source.bytes);
    } finally { this.mediaWrite = false; }
  }

  /// Use one of a conflicted frame's live values as the frame's pixels: fetch it bounded, hold it
  /// locally through publication, then replace. The blob is never named by a guessed cid.
  async useVersion(object: Hex32, frame: Hex32, alt: FrameConflictValue, how: "replace" | "insertAfter"): Promise<void> {
    const gen = this.generation;
    const route = this.editRoute(object);
    if ("refused" in route) throw new StudioNativeError(route.refused);
    const bytes = await this.fetchNow(alt.cid, alt.bytes);
    if (!this.current(gen)) return;
    if (!bytes) throw new StudioNativeError("that version's pixels are not available yet");
    // Re-putting a value the register already selects is still an operation here: it consumes
    // every live predecessor and so resolves the conflict on purpose.
    if (how === "replace") this.enqueue({ kind: "frame", object, frame, op: "replace", after: null, pix: bytes, published: null, route: route.kind, apply: null, overlay: null, overlayRetry: null, force: true, label: `frame pixels by ${alt.author.slice(0, 8)}` });
    else this.enqueue({ kind: "frame", object, frame: this.newId(), op: "insert", after: frame, pix: bytes, published: null, route: route.kind, apply: null, overlay: null, overlayRetry: null, label: "keep both versions" });
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

  private applyOp(object: Hex32, op: FlipnoteOp | ScoreOp, label: string): SaveEntry {
    const route = this.editRoute(object);
    if ("refused" in route) throw new StudioNativeError(route.refused);
    // Cell/recipe objects now cross this seam. Freeze their operation identity immediately rather
    // than keeping a mutable caller descriptor beside an otherwise exact request/re-author path.
    const body = canonicalJson(op);
    op = JSON.parse(body) as FlipnoteOp | ScoreOp;
    if (route.kind === "ordinary") {
      return this.enqueue({ kind: "apply", object, op, label, request: { object, epochId: route.epochId, nonce: this.newId(), body,
        ...(this.objectKind(object) === "score" ? { objectKind: "score" as const } : {}) } });
    } else {
      return this.enqueue({ kind: "overlayApply", object, route: route.kind, ...(this.objectKind(object) === "score" ? { objectKind: "score" as const } : {}), op, label, request: null, overlayRetry: null });
    }
  }
  private applyIndexOp(op: IndexOp, object: Hex32, label: string): void {
    const route = this.editRoute(null);
    if ("refused" in route) throw new StudioNativeError(route.refused);
    if (route.kind === "ordinary") {
      this.enqueue({ kind: "applyIndex", object, op, label, request: { epochId: route.epochId, nonce: this.newId(), body: canonicalJson(op) } });
    } else {
      this.enqueue({ kind: "overlayApply", object: null, route: route.kind, op, label, request: null, overlayRetry: null });
    }
  }

  private enqueue(rec: NewSave): SaveEntry {
    if (this.closeHeld) throw new StudioNativeError("The window is closing. Wait for saving to finish.");
    if (this.restoredDraft) throw new StudioNativeError("Open the channel with your restored Studio edits, or discard them in the Studio sidebar, before making more changes.");
    this.mediaEpoch++;
    const record = { ...rec, label: rec.label.slice(0, 256), id: ++this.saveSeq, status: "queued", attempts: 0, error: "", epochChanged: false, mayHaveCommitted: false } as SaveEntry;
    if (record.kind === "apply" || record.kind === "applyIndex" || record.kind === "overlayApply") {
      const family = record.kind === "applyIndex" || record.object === null ? "index" : this.objectKind(record.object);
      validateDraftOperation(record.op, family);
      if (new TextEncoder().encode(canonicalJson(record.op)).length > 65536) throw new StudioNativeError("This edit is too large to save.");
    }
    const object = rec.kind === "applyIndex" ? null : rec.object;
    const base = object === null ? this.index : this.doc?.object === object ? this.doc.view : this.views.get(object)?.view;
    if (base) record.binding = { epochId: base.epochId, awaiting: base.awaitingTenureReceipt, phase: base.phase };
    this.draftGroup ??= base?.group ?? this.index?.group ?? null;
    if (record.kind === "frame") record.pix = record.pix.slice();
    const tail = this.records.filter(s => laneOf(s) === laneOf(record)).at(-1);
    if (this.autosaveMs && tail && tail.status === "queued" && tail.attempts === 0 && !this.eligible.has(tail.id) && !this.restoredHeld.has(tail.id) &&
        workingSlot(record) !== null && workingSlot(record) === workingSlot(tail) && tail.kind === record.kind) {
      this.checkDraftBudget(record, tail);
      this.records.splice(this.records.indexOf(tail), 1, record); this.settleRecord(tail, "discarded");
    } else {
      this.checkDraftBudget(record);
      this.records.push(record);
    }
    this.authored();
    this.notify();
    // Creating a document and explicitly recovering history are save actions themselves. Ordinary
    // canvas, sound, music, header and Index edits use the same delayed queue afterward.
    if (!this.autosaveMs || record.kind === "create" || record.kind === "recoveryApply") this.eligible.add(record.id);
    else this.markDirty();
    this.pump();
    return record;
  }

  /** Begin a fixed first-change deadline, including a stroke which has not had pointer-up yet. */
  markDirty(): void {
    if (!this.autosaveMs) { this.pump(); return; }
    if (this.cancelAutosave) return;
    const generation = this.generation;
    this.cancelAutosave = this.schedule(() => {
      this.cancelAutosave = null;
      if (!this.current(generation)) return;
      try { this.stageDrawing(); this.releaseQueued(); } catch (e) { this.indexError = reason(e); this.notify(); }
      // Staging an interrupted stroke may enqueue its first raster. It belongs to this deadline,
      // and must not leave an empty timer behind or postpone a subsequent edit's deadline.
      this.cancelAutosaveTimer();
    }, this.autosaveMs);
  }

  private releaseQueued(explicit = false, covers: (s: SaveRecord) => boolean = () => true): void {
    // Snapshot identities now. Later edits, including a new revision of an in-flight frame, get
    // their own deadline; a worker's finally must not drain them as part of this Save.
    for (const s of this.records) if (covers(s) && s.status === "queued" && (explicit || !this.restoredHeld.has(s.id))) { this.eligible.add(s.id); this.restoredHeld.delete(s.id); }
    this.pump();
  }

  private cancelAutosaveTimer(): void { this.cancelAutosave?.(); this.cancelAutosave = null; }

  /** The immutable argument captured before native lock revokes the current UI generation. */
  draftSnapshot(): string | null {
    if (this.restoredDraft) return encodeStudioDraft(this.restoredDraft);
    if (!this.scope || !this.records.length) return null;
    const group = this.draftGroup ?? this.index?.group ?? this.doc?.view?.group;
    if (!group) throw new StudioNativeError("Studio could not identify the server for these unsaved edits.");
    return encodeStudioDraft({ version: 1, scope: { ...this.scope }, group, records: this.records });
  }

  restoreDraft(draft: StudioDraft): void {
    if (this.records.length || this.restoredDraft) throw new StudioNativeError("Studio already has unsaved edits.");
    this.restoredDraft = draft;
    this.installRestoredDraft(); this.notify();
  }

  private installRestoredDraft(): void {
    const draft = this.restoredDraft;
    if (!draft || this.scope?.server !== draft.scope.server || this.scope.channel !== draft.scope.channel) return;
    this.restoredDraft = null; this.draftGroup = draft.group;
    for (const record of draft.records) {
      // An interrupted delivery may have committed. Never automatically turn it into a new edit.
      const interrupted = record.status === "inflight" || record.mayHaveCommitted;
      const restored = { ...record, status: interrupted ? "uncertain" as const : record.status,
        mayHaveCommitted: interrupted || record.mayHaveCommitted };
      this.records.push(restored); this.saveSeq = Math.max(this.saveSeq, record.id);
      this.restoredHeld.add(record.id);
    }
    // No timer, eligible identity or worker is installed: only Save/retry can submit restored work.
  }

  /** Restored work that only an explicit Save, retry or discard (or leaving its document) will
   * move: the deadline never releases it. Surfaces offer that Save and discard next to it. */
  awaitsExplicitSave(id: number): boolean {
    return this.restoredHeld.has(id) && this.records.some(s => s.id === id && s.status === "queued");
  }

  /** Save a fixed revision and await definite receipts. Failure/timeout keeps the editor intact. */
  async save(): Promise<void> { await this.saveCovered(() => true); }

  /** `save()` for one document's records (including its Index records): what a Save button inside
   * that document's editor means. Other documents' restored work stays held for its own explicit
   * action, and another lane's failure cannot refuse this document's Save. */
  async saveDocument(object: Hex32): Promise<void> { await this.saveCovered(s => affectedObject(s) === object); }

  /** The records each awaited Save is draining, most recent last; pump prefers the most recent
   * one's (see pump). A list, not one slot: overlapping Saves can finish in any order. */
  private readonly preferring: ((s: SaveRecord) => boolean)[] = [];

  /** `save()` restricted to the records `covers` selects; the rest keep their queue state. */
  private async saveCovered(covers: (s: SaveRecord) => boolean): Promise<void> {
    this.preferring.push(covers);
    try { await this.saveCoveredNow(covers); }
    finally { const at = this.preferring.lastIndexOf(covers); if (at >= 0) this.preferring.splice(at, 1); }
  }

  private async saveCoveredNow(covers: (s: SaveRecord) => boolean): Promise<void> {
    this.stageDrawing();
    const generation = this.generation, ids = new Set(this.records.filter(covers).map(s => s.id));
    // The shared deadline is cancelled only when every record is being released now. A partial
    // (departure) Save leaves it running for the uncovered records, such as an Index rename made
    // with the title, which would otherwise wait for an unrelated later edit to schedule them.
    if (ids.size === this.records.length) { this.cancelAutosave?.(); this.cancelAutosave = null; }
    this.releaseQueued(true, covers);
    if (!ids.size) return;
    // A covered record can sit in a lane behind an uncovered one this Save will never move (the
    // Index lane is shared by every document). Waiting would only time out with "try again", which
    // never helps; say at once what it waits for. What could run has been released and still runs.
    const stuck = (blocker: SaveEntry) => {
      const entry = affectedObject(blocker), listed = [...(this.indexModel?.entries ?? []), ...(this.indexModel?.overflow ?? [])].find(e => e.id === entry);
      const choice = blocker.status === "queued" ? "Save or discard" : "Retry or discard";
      return new StudioNativeError(`Part of this Save waits behind an unsaved edit (${blocker.label}${listed?.title ? ` of "${listed.title}"` : ""}). ${choice} that edit first; this document's edits are kept.`);
    };
    const blocker = this.stuckBehind(ids);
    if (blocker) throw stuck(blocker);
    await new Promise<void>((resolve, reject) => {
      let detach = () => {}, cancel = () => {};
      const check = () => {
        const remaining = this.records.filter(s => ids.has(s.id));
        const failure = remaining.find(s => s.status === "blocked" || s.status === "uncertain");
        // Asked again on every change: a record ahead that was in flight can fail meanwhile.
        const behind = failure ? null : this.stuckBehind(ids);
        if (generation !== this.generation || failure || behind || !remaining.length) {
          detach(); cancel();
          if (generation !== this.generation) reject(new StudioNativeError("Studio changed before saving finished"));
          else if (failure) reject(new StudioNativeError("Could not confirm the save. Your edits are still here; retry before leaving."));
          else if (behind) reject(stuck(behind));
          else resolve();
        }
      };
      detach = this.onChange(check);
      cancel = this.schedule(() => { detach(); reject(new StudioNativeError("Studio is still saving. Your edits are still here; wait and try again.")); }, 10_000);
      check();
    });
  }

  /** The first uncovered record that keeps a covered, queued one of `ids` from ever running: it is
   * ahead of it in its lane and only the member can move it (failed, uncertain, or restored and
   * held). A record waiting for its deadline or already in flight will move on its own. */
  private stuckBehind(ids: Set<number>): SaveEntry | null {
    for (const s of this.records) {
      if (!ids.has(s.id) || s.status !== "queued") continue;
      const lane = laneOf(s);
      for (const r of this.records) {
        if (r === s) break;
        if (ids.has(r.id) || laneOf(r) !== lane) continue;
        if (r.status === "blocked" || r.status === "uncertain" || (r.status === "queued" && this.restoredHeld.has(r.id))) return r;
      }
    }
    return null;
  }

  /** Leaving a channel (or the window, while its continuity snapshot cannot carry the work): every
   * retained edit must reach a definite receipt, because the session's scope is about to change
   * under it. Leaving only the Studio SURFACE uses `preserveForDeparture` instead. */
  async saveBeforeLeaving(): Promise<void> { await this.saveUntilClear(() => true); }

  /** The local-preservation half of a departure. It stages the mounted drawing (a stroke the queue
   * refuses to stage throws, exactly as a Save does: that work exists only on the canvas, so the
   * departure must not proceed) and sends every queued edit now, as leaving always has. It then
   * waits at most `settleMs` for those Saves to land, and never fails because of their outcome: a
   * failed, uncertain or still-running Save stays on this session's queue with its exact retry
   * identity, where the editor shows it again, and in the sealed continuity snapshot the caller
   * writes next. That snapshot, not a native receipt, is what makes leaving safe; a caller that
   * cannot write one must use `saveBeforeLeaving`. Resolves with how many records are still held.
   *
   * The session is not reset by leaving the surface: workers keep running, so a Save in flight or
   * queued behind its deadline still completes in the background, and the change it makes to the
   * queue schedules the next checkpoint through `onWorkChange`. */
  async preserveForDeparture(settleMs = 0): Promise<number> {
    this.prepareDocumentClose();
    this.stageDrawing();
    const generation = this.generation;
    const ids = new Set(this.records.map(s => s.id));
    this.cancelAutosaveTimer();
    this.releaseQueued(true);
    if (settleMs > 0 && ids.size) {
      await new Promise<void>((resolve) => {
        let detach = () => {}, cancel = () => {};
        const check = () => {
          // Done once nothing of what was sent is still queued or in flight: landed, refused or
          // uncertain are all outcomes, and none of them is a reason to keep the member here.
          const moving = this.records.some(s => ids.has(s.id) && (s.status === "queued" || s.status === "inflight") && !this.stuckBehind(new Set([s.id])));
          if (generation !== this.generation || !moving) { detach(); cancel(); resolve(); }
        };
        detach = this.onChange(check);
        cancel = this.schedule(() => { detach(); resolve(); }, settleMs);
        check();
      });
    }
    // A stroke made while the Saves above were awaited is new work for the snapshot, not lost.
    if (generation === this.generation) this.stageDrawing();
    return this.records.length;
  }

  private async saveUntilClear(covers: (s: SaveRecord) => boolean): Promise<void> {
    this.prepareDocumentClose();
    const started = this.now(), generation = this.generation;
    do {
      await this.saveCovered(covers);
      if (generation !== this.generation) throw new StudioNativeError("Studio changed while saving.");
      this.stageDrawing(); // capture a stroke made while the previous revision awaited its ACK
      if (this.now() - started > 10_000 && this.records.some(covers)) throw new StudioNativeError("Studio is still saving. Please wait before leaving.");
    } while (this.records.some(covers));
  }

  private checkDraftBudget(next: SaveRecord, replacing?: SaveRecord): void {
    const entries = [...this.records.filter(s => s !== replacing), next];
    if (entries.length > 512) throw new StudioNativeError("Too many unsaved edits. Save before continuing.");
    if (next.kind === "frame") decodePix(next.pix);
    let pixels = 0, encoded = 2048; // version/scope/group, separators and JSON wrapper
    for (const s of entries) {
      if (s.kind === "frame") { pixels += s.pix.length; encoded += Math.ceil(s.pix.length / 3) * 4 + 9; }
      // Reserve future frozen frame/ticket fields and bounded diagnostics before accepting an edit.
      // Count UTF-8 rather than UTF-16 so Unicode titles cannot make a later lock snapshot overflow.
      let cost = this.draftCosts.get(s);
      if (cost === undefined) {
        // Account for the outer UI envelope's string escaping too. Cached per owned entry: large
        // PIX bodies are never base64-encoded/decoded again for each subsequent editor action.
        // A local-only ticket freezes a second copy of the operation body, which may itself
        // approach the 64 KiB domain cap. Reserve that exact shape, not merely a small nonce.
        const budgetRecord = s.kind === "overlayApply" && s.request === null
          ? { ...s, request: { basis: "0".repeat(64), branch: "0".repeat(64), nonce: "0".repeat(32), body: canonicalJson(s.op) } } : s;
        cost = new TextEncoder().encode(JSON.stringify(JSON.stringify(budgetRecord, (key, value) => key === "pix" || key === "settle" ? undefined : value))).length + 4096;
        this.draftCosts.set(s, cost);
      }
      encoded += cost;
    }
    if (pixels > 8 * 1024 * 1024 || encoded > 12 * 1024 * 1024) throw new StudioNativeError("Unsaved edits are full. Save before continuing.");
  }

  /// Resend the same complete request.
  retry(id: number): void {
    const s = this.records.find((r) => r.id === id);
    if (!s || (s.status !== "uncertain" && s.status !== "blocked")) return;
    const retry = s.kind === "overlayApply" || s.kind === "frame" ? s.overlayRetry : null;
    if (retry === "newTicket" || retry === "none") return;
    s.status = "queued";
    s.error = "";
    this.eligible.add(s.id); this.restoredHeld.delete(s.id);
    this.notify();
    this.pump();
  }

  /// Whether the retained operation is proven not to have written and can therefore be replaced
  /// by a deliberately new authoring decision. Ambiguous operations keep their exact identity;
  /// the member must reconcile/retry or explicitly discard them rather than having this method
  /// silently erase their uncertainty.
  canReauthor(id: number): boolean {
    const s = this.records.find((r) => r.id === id);
    return !!s && s.status === "blocked" && !s.mayHaveCommitted
      && (s.kind === "frame" || s.kind === "apply" || s.kind === "applyIndex" || s.kind === "overlayApply");
  }

  /// Explicit re-authoring: replace one proven-no-write entry with a NEW operation selected by
  /// the document's route *now*. This is deliberately not available for an ambiguous request.
  /// The replacement gets a fresh queue identity and, on the ordinary route, a fresh nonce.
  reauthor(id: number): void {
    const at = this.records.findIndex((r) => r.id === id);
    const s = this.records[at];
    if (!s || !this.canReauthor(id)) return;
    if (s.kind !== "frame" && s.kind !== "apply" && s.kind !== "applyIndex" && s.kind !== "overlayApply") return;
    const object = s.kind === "applyIndex" ? null : s.object;
    const originalKind = s.kind === "apply" ? s.request.objectKind ?? "flipnote" : s.kind === "overlayApply" ? s.objectKind ?? "flipnote" : "flipnote";
    if (object !== null && this.objectKind(object) !== originalKind) throw new StudioNativeError("the object family changed; the retained edit cannot be redirected");
    const route = this.editRoute(object);
    if ("refused" in route) throw new StudioNativeError(route.refused);
    let next: NewSave;
    if (s.kind === "frame") {
      // A ticket refusal can mean the publication disappeared or failed validation. Re-publish
      // retained pixels even when the new route is ordinary; identical valid bytes are
      // content-addressed, and no stale receipt crosses the route transition.
      next = { kind: "frame", object: s.object, frame: s.frame, op: s.op, after: s.after,
        pix: s.pix, published: s.route !== "ordinary" ? null : s.published,
        route: route.kind, apply: null, overlay: null, overlayRetry: null,
        ...(s.force === undefined ? {} : { force: s.force }), label: s.label };
    } else if (route.kind !== "ordinary") {
      next = { kind: "overlayApply", object, route: route.kind, ...(object && this.objectKind(object) === "score" ? { objectKind: "score" as const } : {}), op: s.op, label: s.label, request: null, overlayRetry: null };
    } else if (object === null) {
      const op = s.op as IndexOp;
      next = { kind: "applyIndex", object: op.object, op, label: s.label,
        request: { epochId: route.epochId, nonce: this.newId(), body: canonicalJson(op) } };
    } else {
      next = { kind: "apply", object, op: s.op as FlipnoteOp | ScoreOp, label: s.label,
        request: { object, epochId: route.epochId, nonce: this.newId(), body: canonicalJson(s.op),
          ...(this.objectKind(object) === "score" ? { objectKind: "score" as const } : {}) } };
    }
    const replacement = { ...next, id: ++this.saveSeq, status: "queued", attempts: 0,
      error: "", epochChanged: false, mayHaveCommitted: false } as SaveEntry;
    const base = object === null ? this.index : this.doc?.object === object ? this.doc.view : this.views.get(object)?.view;
    if (base) replacement.binding = { epochId: base.epochId, awaiting: base.awaitingTenureReceipt, phase: base.phase };
    this.records[at] = replacement;
    this.eligible.delete(s.id); this.restoredHeld.delete(s.id);
    this.eligible.add(replacement.id);
    this.settleRecord(s, "discarded");
    this.authored();
    this.notify();
    this.pump();
  }

  /// Drop a save on purpose. For a frame save this discards its unsaved pixels; the saves that
  /// waited behind it in the same lane may then run.
  discard(id: number): void {
    const at = this.records.findIndex((r) => r.id === id && r.status !== "inflight");
    if (at < 0) return;
    const [s] = this.records.splice(at, 1);
    this.eligible.delete(s.id); this.restoredHeld.delete(s.id); this.cooling.delete(s.id); this.busyResends.delete(s.id);
    this.settleRecord(s, "discarded");
    this.notify();
    this.pump();
  }

  /// The newest unsaved pixels for a frame, so an editor reopens what the member drew rather
  /// than the last saved projection.
  unsavedPix(object: Hex32, frame: Hex32): Uint8Array | null {
    for (let i = this.records.length - 1; i >= 0; i--) {
      const s = this.records[i];
      if (s.kind === "frame" && s.object === object && s.frame === frame) return s.pix;
    }
    return null;
  }

  /// `null` is the channel Index's own lane (every Index record, of any route); a document is every
  /// record that changes it, including its Index records (see `affectedObject`).
  pendingFor(object: Hex32 | null): SaveRecord[] {
    return this.saves.filter((s) => object === null ? s.kind === "applyIndex" || s.object === null : affectedObject(s) === object);
  }

  /// Queued saves that cannot run until this uncertain or known-not-written save is handled.
  blockedBehind(id: number): number {
    const s = this.records.find((r) => r.id === id);
    if (!s || (s.status !== "uncertain" && s.status !== "blocked")) return 0;
    const lane = laneOf(s);
    return this.records.filter((r) => r.id !== id && r.status === "queued" && laneOf(r) === lane).length;
  }

  /// Up to SAVE_WORKERS saves at a time, at most one per lane (a document, or the Index), in queue
  /// order within each lane. An uncertain, known-not-written or in-flight save blocks later saves of
  /// its lane; other lanes proceed on the other workers, so one slow receipt (an Index Save, say)
  /// no longer holds every document's next Save behind it. Across lanes the next save is the
  /// earliest runnable lane head, except that the most recently started Save still being awaited
  /// (`preferring`) goes first, so a departure draining its document gets a worker ahead of other
  /// work. Native Studio operations for one server still run one at a time in its actor; workers
  /// change only what the renderer has sent, never native ordering within a lane.
  private pump(): void {
    while (this.workers.size < SAVE_WORKERS) {
      const seen = new Set<string>(), heads: SaveEntry[] = [];
      for (const s of this.records) {
        const lane = laneOf(s);
        if (seen.has(lane)) continue;
        seen.add(lane); // a lane's first record decides it: runnable now, or nothing later runs
        if (!this.running.has(s.id) && !this.cooling.has(s.id) && s.status === "queued" && (!this.autosaveMs || this.eligible.has(s.id))) heads.push(s);
      }
      const preferred = this.preferring.at(-1);
      const next = (preferred && heads.find(preferred)) || heads[0];
      if (!next) return;
      const token = ++this.workerSeq;
      this.workers.add(token); this.running.add(next.id);
      void this.run(next).finally(() => {
        if (!this.workers.delete(token)) return; // a cleared scope owns different workers now
        this.running.delete(next.id);
        this.pump();
      });
    }
  }

  private async run(s: SaveEntry): Promise<void> {
    const gen = this.generation;
    const target = this.target();
    // Admission compares the record's authoring binding with the CURRENT view of its document. A
    // record restored after lock, or one whose view left the bounded cache, can reach the lane
    // before this session has read that document; read it now rather than refusing on the absence,
    // which would misreport "shared history changed" and force a needless reauthor.
    // The read runs while the record is still `queued` and before `attempts` counts it: nothing
    // has been sent, so a lock snapshot taken meanwhile must not restore it as an interrupted
    // delivery that "may have committed" (which would also disable reauthor), and Discard must
    // stay available. This worker holds the lane, so nothing else can start the record.
    const unread = this.admissionRead(s);
    if (unread !== null) {
      const read = (this.docReq.get(unread) ?? 0) + 1;
      await this.refreshDoc(unread).catch(() => {});
      if (!this.current(gen) || !this.records.includes(s) || s.status !== "queued") return;
      if (!this.heldView(unread)) {
        // Still nothing to compare against, so nothing is sent and nothing counts as an attempt:
        // a retry reads and compares again instead of skipping admission as a repeat delivery.
        // "Still reading" only while a newer read of the open document is actually still out.
        const stillReading = (this.docReq.get(unread) ?? 0) !== read && this.doc?.object === unread && this.doc.loading;
        this.settleFailure(s, new OverlayNoWrite(stillReading
          ? "Studio was still reading this document, so nothing was sent. Retry in a moment."
          : "Studio could not read this document before saving, so nothing was sent. Open it and retry."));
        return;
      }
    }
    // The channel-level preconditions are checked before the attempt counts, for the same reason.
    // Without the Index (not read yet after unlock, or its read failed) neither the server group nor
    // an Index record's binding can be compared, so nothing is sent; claiming the server changed
    // would be untrue, and counting the attempt would let a later retry skip admission.
    const indexUnread = this.autosaveMs && !this.index && (!!this.draftGroup || (!!s.binding && (s.kind === "applyIndex" || s.object === null)));
    if (indexUnread || (this.autosaveMs && this.draftGroup && this.index && this.index.group !== this.draftGroup)) {
      this.settleFailure(s, new OverlayNoWrite(indexUnread
        ? "Studio has not read this channel's index yet, so nothing was sent. Retry once it has loaded."
        : "The server for these saved edits changed. They were not sent."));
      return;
    }
    s.status = "inflight";
    s.attempts++;
    this.notify();
    // Native preserves a durable Save receipt even if a refresh overtakes its projection.
    // Separate acknowledgement from display currency: a read begun after this Save may carry
    // newer remote/rotation state, so an older ACK must resolve the queue without rolling it back.
    // Captured after the admission read above, which is older than this Save's acknowledgement.
    const docRead = this.docReadSerial.get(s.object ?? "") ?? 0, indexRead = this.indexReadSerial;
    // Whether this visit actually handed a write to native. `attempts` counts deliveries: it decides
    // whether the next visit repeats first-delivery admission, and a restored draft reads it as
    // evidence. A refusal raised before anything was sent (the binding check, a missing editable
    // epoch, a refused local-draft ticket, a failed explicit access) is not a delivery, and counting
    // it made the member's Retry skip admission and send the frozen request to a retired epoch,
    // whose untyped native rejection then read as "may have committed".
    let dispatched = false;
    const sent = () => { dispatched = true; };
    try {
      if (s.attempts === 1 || s.kind === "frame" && !s.apply && !s.overlay || s.kind === "overlayApply" && !s.request) this.assertAdmission(s);
      if (s.kind === "create") {
        sent();
        const view = await studioCreate(this.ipc, target, s.request);
        if (!this.current(gen)) return;
        await this.landed(s.object, view, docRead);
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
        const op: FlipnoteOp = s.op === "insert"
          ? { op: "insert_frame", frame: s.frame, after: s.after, cid: s.published.cid, bytes: s.published.bytes }
          : { op: "replace_frame", frame: s.frame, cid: s.published.cid, bytes: s.published.bytes };
        if (s.route !== "ordinary") {
          await this.runUnconfirmed(s, s.object, canonicalJson(op), gen, sent);
          if (!this.current(gen)) return;
          await this.refreshDoc(s.object);
          if (!this.current(gen)) return;
        } else {
          if (!s.apply) {
            this.assertAdmission(s);
            const ep = this.editableEpoch(s.object);
            if (!("epochId" in ep)) throw new OverlayNoWrite(ep.refused);
            s.apply = { object: s.object, epochId: ep.epochId, nonce: this.newId(), body: canonicalJson(op) };
            // The minted identity is retry evidence: checkpoint it before the request leaves.
            this.notify();
          }
          sent();
          const view = await studioApply(this.ipc, target, s.apply);
          if (!this.current(gen)) return;
          await this.landed(s.object, view, docRead);
        }
      } else if (s.kind === "apply") {
        sent();
        const view = await studioApply(this.ipc, target, s.request);
        if (!this.current(gen)) return;
        await this.landed(s.object, view, docRead);
      } else if (s.kind === "applyIndex") {
        sent();
        const view = await studioApplyIndex(this.ipc, target, s.request);
        if (!this.current(gen)) return;
        await this.landedIndex(view, indexRead);
      } else if (s.kind === "overlayApply") {
        await this.runUnconfirmed(s, s.object, canonicalJson(s.op), gen, sent);
        if (!this.current(gen)) return;
        if (s.object) await this.refreshDoc(s.object);
        else await this.refreshIndex();
        if (!this.current(gen)) return;
      } else {
        sent();
        const applied = await recoveryApply(this.ipc, { ...target, object: s.object,
          ...(s.objectKind === "score" ? { objectKind: "score" as const } : {}) }, s.edit);
        if (!this.current(gen)) return;
        s.applied = applied;
        if (s.object) this.invalidate({ objects: [s.object] });
        else this.invalidate({ index: true });
      }
      if (!this.current(gen)) return;
      if (!this.syncObservation) this.receivePaused = false;
      this.finish(s);
    } catch (e) {
      if (!this.current(gen)) return;
      // Undo only this visit's count, and never below an earlier uncertain delivery (a restored
      // draft must keep attempts >= 1 whenever mayHaveCommitted is set; studio-draft.ts).
      if (!dispatched && e instanceof OverlayNoWrite && !s.mayHaveCommitted) s.attempts = Math.max(0, s.attempts - 1);
      if (nativeNoWrite(e) && this.resendLater(s)) return;
      this.settleFailure(s, e);
    }
  }

  /// A request the bridge refused before the execution lease was transferred (slots full, actor
  /// busy, storage busy) never ran, so resending the SAME request is safe: back it off and requeue
  /// it, a bounded number of times. Its lane waits (FIFO holds); other lanes carry on. False when
  /// the resends are used up, and settleFailure then decides the outcome.
  /// The visit is not counted, and the record settles as not written, only while no earlier
  /// attempt was uncertain. After one that may have committed, attempts are left as they are
  /// (never lowered, so a restored draft keeps attempts >= 1) and the record settles uncertain
  /// again: a later refusal cannot show what that earlier attempt did.
  private resendLater(s: SaveEntry): boolean {
    const used = this.busyResends.get(s.id) ?? 0;
    if (used >= BUSY_BACKOFF_MS.length) return false;
    this.busyResends.set(s.id, used + 1);
    if (!s.mayHaveCommitted) s.attempts = Math.max(0, s.attempts - 1);
    s.status = "queued";
    this.cooling.add(s.id);
    const generation = this.generation;
    this.schedule(() => {
      if (generation !== this.generation) return;
      this.cooling.delete(s.id);
      this.pump();
    }, BUSY_BACKOFF_MS[used]);
    this.notify();
    return true;
  }

  private settleFailure(s: SaveEntry, e: unknown): void {
    this.busyResends.delete(s.id);
    // Typed Flow-S refusals and pre-Apply epoch refusals prove this attempt did not write, and so
    // do the native refusals raised before the execution lease is transferred (nativeNoWrite). Other
    // native errors remain uncertain because they may have crossed a final delivery fence after commit.
    const noWrite = e instanceof OverlayNoWrite || nativeNoWrite(e);
    const earlierAttemptWasUncertain = s.mayHaveCommitted;
    // A pre-dispatch refusal is not a delivery: it does not use up the first-attempt admission
    // check, so the member's retry reads and compares the document again before sending.
    if (nativeNoWrite(e) && !earlierAttemptWasUncertain) s.attempts = Math.max(0, s.attempts - 1);
    if (!noWrite) s.mayHaveCommitted = true;
    s.status = s.mayHaveCommitted ? "uncertain" : "blocked";
    s.error = (noWrite && earlierAttemptWasUncertain
      ? `${reason(e)}; an earlier attempt may already have committed, so the exact request remains uncertain`
      : reason(e)).slice(0, 512);
    this.settleRecord(s, s.mayHaveCommitted ? "uncertain" : "notWritten");
    this.notify();
  }

  private heldView(object: Hex32): StudioObjectView | null {
    return (this.doc?.object === object ? this.doc.view : this.views.get(object)?.view) ?? null;
  }

  /** The document `run` must read before admitting `s`, or null when admission needs no read:
   * no binding to check, an Index record (the Index is read with the scope), or a view in hand.
   * Called before `run` counts the attempt, so a first delivery has `attempts === 0` here. */
  private admissionRead(s: SaveEntry): Hex32 | null {
    if (!this.autosaveMs || !s.binding || s.kind === "applyIndex" || s.object === null) return null;
    // Mirrors the conditions under which run() and runUnconfirmed() call assertAdmission.
    const admits = s.attempts === 0 || (s.kind === "frame" && !s.apply && !s.overlay) || (s.kind === "overlayApply" && !s.request);
    if (!admits) return null;
    return this.heldView(s.object) ? null : s.object;
  }

  private assertAdmission(s: SaveEntry): void {
    if (!this.autosaveMs || !s.binding) return;
    const object = s.kind === "applyIndex" ? null : s.object;
    const view = object === null ? this.index : this.doc?.object === object ? this.doc.view : this.views.get(object)?.view;
    // Still no view after run()'s admission read: the document could not be read (or is absent
    // on this device). Say so; "history changed" would send the member to reauthor for nothing.
    if (!view) throw new OverlayNoWrite(object === null
      ? "Studio could not read the channel index before saving, so nothing was sent. Retry once it has loaded."
      : "Studio could not read this document before saving, so nothing was sent. Open it and retry.");
    if (view.epochId !== s.binding.epochId || view.awaitingTenureReceipt !== s.binding.awaiting || view.phase !== s.binding.phase) {
      throw new OverlayNoWrite("Shared history changed before this edit was saved. Review it and save it as a new edit.");
    }
  }

  /// Execute a preview-local Save while retaining the exact request on the queue.
  /// `pending` and `busy` are the native contract's "repeat the identical request" answers, so
  /// they are continued here automatically (see OVERLAY_CONTINUE_MS), never surfaced as a failure
  /// the member has to Retry: new authoring is always answered `pending` on its first visit, and a
  /// renderer that stopped there turned every ordinary preview or Closing edit into a save error
  /// that held its whole lane. Each visit resends the same basis, branch, nonce and body. `busy`
  /// proves the visit began no write; `pending` means a detached plan of THIS request exists and
  /// may still be committed by any later Save visit, so once one was seen, running out of budget,
  /// a refusal, a pause or a transport failure all leave the record uncertain (mayHaveCommitted)
  /// rather than "not written". A paused actor is first resumed by an explicit ordinary document
  /// access, exactly as the native contract requires, then receives the same request.
  /// `sent` is called immediately before the first Save visit: the Begin ticket and the explicit
  /// access writes nothing, so a refusal before that point is not a delivery (see run()).
  private async runUnconfirmed(
    s: Extract<SaveEntry, { kind: "frame" }> | Extract<SaveEntry, { kind: "overlayApply" }>,
    object: Hex32 | null,
    body: string,
    gen: number,
    sent: () => void = () => {},
  ): Promise<UnconfirmedOverlaySave | ClosingOverlaySave> {
    const closing = s.route === "closing";
    const target: RecoveryTargetArgs = { ...this.target(), object,
      ...(s.kind === "overlayApply" && s.objectKind === "score" ? { objectKind: "score" as const } : {}) };
    if (!this.current(gen)) throw new StudioNativeError("Studio session changed before the local draft Save");
    let request = s.kind === "frame" ? s.overlay : s.request;
    if (s.overlayRetry === "afterExplicitAccess") {
      if (object) await this.refreshDoc(object);
      else await this.refreshIndex();
      if (!this.current(gen)) throw new StudioNativeError("Studio session changed before the local draft retry");
      const accessError = object
        ? this.doc?.object === object ? this.doc.error : "the flipnote is no longer open"
        : this.indexError;
      if (accessError) {
        throw new OverlayNoWrite(`explicit document access failed; the paused Save was not retried (${accessError})`);
      }
    }
    if (!request) {
      this.assertAdmission(s);
      const begun = closing ? await closingOverlaySaveBegin(this.ipc, target) : await unconfirmedOverlaySaveBegin(this.ipc, target);
      if (!this.current(gen)) throw new StudioNativeError("Studio session changed while obtaining a local draft ticket");
      this.assertAdmission(s);
      if (begun.kind === "unconfirmedOverlaySave" || begun.kind === "closingOverlaySave") {
        s.overlayRetry = begun.retry;
        throw new OverlayNoWrite(`local draft refused (${overlayRefusalText(begun.reason)}); retry=${begun.retry}`);
      }
      request = { basis: begun.basis, branch: begun.branch, nonce: this.newId(), body };
      if (s.kind === "frame") s.overlay = request;
      else s.request = request;
      // Retain the request before invoking Save: a native delivery failure may follow its write.
      this.notify();
    }
    if (!this.current(gen)) throw new StudioNativeError("Studio session changed before the local draft Save");
    const frozen = request;
    let scheduled = false, waited = 0;
    try {
      for (let visit = 0; ; visit++) {
        sent();
        const result = closing ? await closingOverlaySave(this.ipc, target, frozen) : await unconfirmedOverlaySave(this.ipc, target, frozen);
        if (result.state === "saved" || result.state === "disposed" || result.state === "handedOff") {
          s.overlayRetry = null;
          return result;
        }
        s.overlayRetry = result.retry;
        if (result.state === "refused") {
          throw new OverlayNoWrite(`local draft refused (${overlayRefusalText(result.reason)}); retry=${result.retry}`);
        }
        if (result.state === "paused") {
          throw new OverlayNoWrite("local draft Save paused; explicit document access is required before retrying the same request");
        }
        if (result.state === "pending") scheduled = true;
        // Out of budget: hand the record to the member with its exact request intact, as before.
        // A plan parked natively past its 30 s bound is discarded, and a later exact retry simply
        // prepares it again.
        if (waited >= OVERLAY_CONTINUE_BUDGET_MS) {
          if (result.state === "busy" && !scheduled) throw new OverlayNoWrite("local draft Save stayed busy; retry the same request");
          throw new StudioNativeError("local draft Save is still being prepared and may still complete; retry the same request exactly");
        }
        const delay = OVERLAY_CONTINUE_MS[visit] ?? OVERLAY_CONTINUE_STEADY_MS;
        await new Promise<void>((resolve) => { this.schedule(resolve, delay); });
        waited += delay;
        // A reset or scope change ends this worker (run() drops the outcome); the native plan, if
        // any, is committed by a later visit or expires on its own bound.
        if (!this.current(gen) || !this.records.includes(s)) throw new StudioNativeError("Studio session changed while the local draft Save was continuing");
      }
    } catch (e) {
      // Whatever ends the continuation after a `pending`, a detached plan of this exact request
      // may still be committed by a later visit; no later answer can disprove that.
      if (scheduled) s.mayHaveCommitted = true;
      throw e;
    }
  }

  private async landed(object: Hex32, view: StudioObjectView, readAtSave: number): Promise<void> {
    if ((this.docReadSerial.get(object) ?? 0) !== readAtSave) {
      // Reconcile before the queue runs dependent work, such as Create's first frame. A
      // pre-Create absence read must not leave that frame without an authenticated epoch.
      await this.refreshDoc(object);
      return;
    }
    // Offscreen exact requests can complete, but a same-id different-family surface owns its
    // newer read fence. Never let an older ACK supersede that read or patch mismatched content.
    const familyMismatch = this.doc?.object === object && this.doc.objectKind !== view.content.kind;
    // Reads already in flight when the Save began are older than its acknowledged projection.
    if (!familyMismatch) {
      this.docReq.set(object, (this.docReq.get(object) ?? 0) + 1);
      this.known.set(object, { awaiting: view.awaitingTenureReceipt, phase: view.phase, epoch: view.epoch });
    }
    const model = view.content.kind === "flipnote" ? flipnoteModel(view.content) : null;
    const score = view.content.kind === "score" ? scoreModel(view.content) : null;
    this.rememberView(object, view, model, score);
    if (!familyMismatch) this.patchDoc(object, { view, model, score, absent: false, error: "", loading: false });
  }
  /// The Index equivalent: a landed write is newer than any list still in flight.
  private async landedIndex(view: IndexView, readAtSave: number): Promise<void> {
    if (this.indexReadSerial !== readAtSave) { await this.refreshIndex(); return; }
    this.indexReq++;
    this.index = view;
    this.indexModel = indexModel(view.content);
    this.indexError = "";
    this.indexLoading = false;
  }
  private rememberView(object: Hex32, view: StudioObjectView, model: FlipnoteModel | null, score: ScoreModel | null): void {
    this.views.delete(object);
    this.views.set(object, { view, model, score });
    while (this.views.size > RECENT_VIEWS) this.views.delete(this.views.keys().next().value as Hex32);
  }
  private finish(s: SaveEntry): void {
    this.eligible.delete(s.id); this.restoredHeld.delete(s.id); this.busyResends.delete(s.id);
    const at = this.records.indexOf(s);
    if (at >= 0) this.records.splice(at, 1);
    this.settleRecord(s, "landed");
    this.notify();
  }
  private settleRecord(s: SaveEntry, outcome: SaveOutcome): void {
    const settle = s.settle;
    s.settle = undefined;
    settle?.(outcome);
  }

  // --- Blobs ---------------------------------------------------------------------------------

  /// Held bytes for a cid, only when they are the size the caller's record declares. A record
  /// whose declaration disagrees with what this cid actually holds is not shown as that record.
  blob(cid: Hex64, bytes?: number): Uint8Array | undefined {
    if (cid.startsWith("draft:")) {
      const [, object, frame] = cid.split(":");
      const held = this.unsavedPix(object, frame);
      return held && (bytes === undefined || held.length === bytes) ? held : undefined;
    }
    const b = this.blobs.get(cid);
    if (!b) return undefined;
    if (bytes !== undefined && b.length !== bytes) return undefined;
    this.blobs.delete(cid);
    this.blobs.set(cid, b); // LRU touch
    return b;
  }
  /// State for one record's (cid, declared bytes). Availability is per cid; a content rejection
  /// is per declaration, because another record may declare this cid correctly.
  blobState(cid: Hex64, bytes?: number): BlobState {
    const held = this.blobs.get(cid);
    if (held) return bytes === undefined || held.length === bytes ? "held" : "invalid";
    const job = bytes === undefined ? this.jobFor(cid) : this.jobs.get(jobKey(this.generation, cid, bytes));
    if (job) return job.started ? "fetching" : "queued";
    if (this.invalidFor(cid, bytes) !== undefined) return "invalid";
    if (this.failed.has(cid)) return "failed";
    if (this.unavailable.has(cid)) return "unavailable";
    return "idle";
  }
  blobProblem(cid: Hex64, bytes?: number): string {
    const held = this.blobs.get(cid);
    if (held && bytes !== undefined && held.length !== bytes) return `held pixels are ${held.length} bytes, not the ${bytes} this record declares`;
    return this.invalidFor(cid, bytes) ?? this.failed.get(cid) ?? (this.unavailable.has(cid) ? "not available from any reachable member yet" : "");
  }
  /// The rejection recorded for this declaration, or for any declaration of the cid when the
  /// caller has none to check against.
  private invalidFor(cid: Hex64, bytes: number | undefined): string | undefined {
    if (bytes !== undefined) return this.invalid.get(contentKey(cid, bytes));
    for (const [key, why] of this.invalid) if (key.startsWith(`${cid}:`)) return why;
    return undefined;
  }
  private jobFor(cid: Hex64): BlobJob | undefined {
    for (const job of this.jobs.values()) if (job.cid === cid && job.generation === this.generation) return job;
    return undefined;
  }

  /// Ask for a frame's pixels. Lower priority runs first (0 = the frame on screen). A cid that
  /// answered unavailable or failed, or a declaration that was rejected, is not asked again
  /// until an invalidation or an explicit retry clears it.
  want(cid: Hex64, bytes: number, priority = 2): void {
    if (this.unavailable.has(cid) || this.failed.has(cid) || this.invalid.has(contentKey(cid, bytes))) return;
    this.scheduleFetch(cid, bytes, priority);
  }

  /// Explicit retry from the member: clears every hold on this record and asks first.
  retryBlob(cid: Hex64, bytes: number): void {
    this.unavailable.delete(cid);
    this.failed.delete(cid);
    this.invalid.delete(contentKey(cid, bytes));
    this.scheduleFetch(cid, bytes, 0);
  }

  /// One bounded fetch for an action that needs the bytes now: it takes the front of the same
  /// queue and the same in-flight slots, and shares an existing job for the same (cid, bytes).
  /// Held bytes of another size than declared are a rejection, not an answer.
  fetchNow(cid: Hex64, bytes: number): Promise<Uint8Array | null> {
    const held = this.blobs.get(cid);
    if (held) {
      if (held.length === bytes) return Promise.resolve(held);
      return Promise.reject(new StudioNativeError(`held pixels are ${held.length} bytes, not the ${bytes} declared`));
    }
    this.unavailable.delete(cid);
    this.failed.delete(cid);
    const job = this.scheduleFetch(cid, bytes, -1);
    if (!job) return Promise.resolve(null);
    return new Promise((resolve, reject) => { job.waiters.push({ resolve, reject }); });
  }

  /// One job per (scope, cid, declared bytes): two records that declare one cid differently get
  /// two bounded requests, each checked against its own declaration, never a shared answer.
  private scheduleFetch(cid: Hex64, bytes: number, priority: number): BlobJob | null {
    if (this.blobs.has(cid) || !this.scope) return null;
    const key = jobKey(this.generation, cid, bytes);
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
        if (e instanceof StudioNativeError || e instanceof PixError) this.invalid.set(contentKey(job.cid, job.bytes), reason(e));
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

  /// The frame CIDs the mounted editor shows right now. Eviction keeps them while they stay in
  /// focus; the surface replaces the set as its view moves. Bounded by what one screen can show,
  /// which is why it may hold the cache above its byte or entry bound rather than evict them: a
  /// timeline may name more frames (999) than the cache holds (512 entries, 8 MiB), and a mounted
  /// thumbnail whose bytes were evicted asks for them again at once.
  focusBlobs(cids: Iterable<Hex64>): void {
    this.blobFocus = new Set(cids);
  }

  private remember(cid: Hex64, bytes: Uint8Array): void {
    if (this.blobs.has(cid)) return;
    this.unavailable.delete(cid);
    this.failed.delete(cid);
    this.invalid.delete(contentKey(cid, bytes.length));
    this.blobs.set(cid, bytes);
    this.cacheBytes += bytes.length;
    if (this.cacheBytes <= this.cacheBytesCap && this.blobs.size <= BLOB_CACHE_ENTRIES) return;
    // Oldest first (Map order is the LRU order; `blob()` touches), skipping what is in focus and
    // the bytes just stored. Stops when the bounds hold or only protected entries remain.
    for (const [key, held] of this.blobs) {
      if (this.cacheBytes <= this.cacheBytesCap && this.blobs.size <= BLOB_CACHE_ENTRIES) break;
      if (key === cid || this.blobFocus.has(key)) continue;
      this.cacheBytes -= held.length;
      this.blobs.delete(key);
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

  async readFault(object: Hex32 | null, scope: FaultScope): Promise<FaultView | null> {
    const gen = this.generation, observation = this.faultEpoch, target = this.objectTarget(object);
    const value = await readStudioFault(this.ipc, target, scope);
    return this.current(gen) && observation === this.faultEpoch && this.objectTarget(object).objectKind === target.objectKind ? value : null;
  }
  async repairFault(object: Hex32 | null, scope: FaultScope, view: FaultView, selected: string): Promise<"scheduled" | "busy" | null> {
    const gen = this.generation, target = this.objectTarget(object);
    const result = await repairStudioFault(this.ipc, target, scope, view, selected);
    if (!this.current(gen) || this.objectTarget(object).objectKind !== target.objectKind) return null;
    this.invalidate({ index: object === null, objects: object ? [object] : [], recovery: true });
    return result;
  }

  /// Point the rail at a document (null = the Index). The same target is a no-op; the listing
  /// is read on a change of target, on an invalidation, or through an explicit refresh.
  watchRecovery(object: Hex32 | null): void {
    const family = object === null ? "index" : this.objectKind(object);
    if (this.recoveryTarget === object && this.recoveryFamily === family && (this.recoveryListing || this.recoveryLoading || this.recoveryError)) return;
    this.recoveryTarget = object;
    this.recoveryFamily = family;
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
    const family = this.recoveryFamily;
    const target = this.objectTarget(object);
    const live = () => this.current(gen) && req === this.recoveryReq && object === this.recoveryTarget && this.recoveryFamily === family &&
      (object === null ? "index" : this.objectKind(object)) === family;
    this.recoveryLoading = true;
    this.notify();
    try {
      const listing = await recoveryList(this.ipc, target);
      if (!live()) return;
      this.recoveryListing = listing;
      this.recoveryError = "";
    } catch (e) {
      if (!live()) return;
      this.recoveryError = reason(e);
    } finally {
      if (live()) { this.recoveryLoading = false; this.notify(); }
    }
  }

  async readVersion(snapshot: Hex64): Promise<RecoveryVersionRead | null> {
    const object = this.recoveryTarget;
    const objectKind = object === null ? "index" : this.objectKind(object);
    const cached = this.versions.get(snapshot);
    if (cached?.content.kind === objectKind) return cached;
    const gen = this.generation;
    const v = await recoveryRead(this.ipc, this.objectTarget(object), snapshot);
    if (!this.current(gen) || object !== this.recoveryTarget || (object === null ? "index" : this.objectKind(object)) !== objectKind) return null;
    this.versions.set(snapshot, v);
    this.notify();
    return v;
  }

  async exportVersion(snapshot: Hex64): Promise<RecoveryExport | null> {
    const gen = this.generation;
    const object = this.recoveryTarget;
    const objectKind = object === null ? "index" : this.objectKind(object);
    const x = await recoveryExport(this.ipc, this.objectTarget(object), snapshot);
    if (!this.current(gen) || object !== this.recoveryTarget || (object === null ? "index" : this.objectKind(object)) !== objectKind) return null;
    this.exports.set(snapshot, x);
    this.notify();
    return x;
  }

  async acknowledgeEviction(): Promise<void> {
    const w = this.recoveryListing?.evictionPending;
    if (!w) throw new StudioNativeError("no eviction warning to acknowledge");
    const gen = this.generation;
    const object = this.recoveryTarget;
    const objectKind = object === null ? "index" : this.objectKind(object);
    const listing = await recoveryAcknowledge(this.ipc, this.objectTarget(object), { oldestSnapshot: w.oldestSnapshot, stagedSnapshot: w.stagedSnapshot });
    if (!this.current(gen) || object !== this.recoveryTarget || (object === null ? "index" : this.objectKind(object)) !== objectKind) return;
    this.recoveryReq++; // a list still in flight is older than this answer
    this.recoveryListing = listing;
    this.recoveryLoading = false;
    this.notify();
  }

  /// The choices a historical version offers, in the order Restore walks them. Headers never
  /// restore automatically: they are listed for Copy only; deletions are Copy only.
  choicesFor(v: RecoveryVersionRead, mode: RecoveryMode): { choice: RecoveryChoice; label: string }[] {
    return choicesForContent(v.content, mode);
  }

  /// Walk a version's choices: preview, apply what is Ready through the save queue, re-read,
  /// next. A partial walk is saved content; the run reports each item's disposition and never
  /// calls itself a Restore. An uncertain apply stops the walk: its exact payload stays on the
  /// save record for retry, and a later walk previews afresh only after that is resolved.
  async runRecovery(snapshot: Hex64, mode: RecoveryMode): Promise<void> {
    const gen = this.generation;
    const object = this.recoveryTarget;
    const family = this.recoveryFamily;
    const version = await this.readVersion(snapshot);
    if (!version || !this.current(gen) || this.recoveryTarget !== object || this.recoveryFamily !== family ||
      (object === null ? "index" : this.objectKind(object)) !== family) return;
    const token = ++this.runSeq;
    const control = { token, stop: false };
    this.activeRun = control;
    const run: RecoveryRun = {
      snapshot, mode, object, status: "running", error: "",
      items: this.choicesFor(version, mode).map(({ choice, label }) => ({ choice, label, state: "pending", error: "", originalAuthor: null })),
    };
    const live = () => this.current(gen) && this.activeRun === control && this.recoveryTarget === object && this.recoveryFamily === family &&
      (object === null ? "index" : this.objectKind(object)) === family;
    const publish = () => {
      if (!live()) return;
      this.recoveryRun = { ...run, items: run.items.map((i) => ({ ...i })) };
      this.notify();
    };
    publish();
    const target = this.objectTarget(object);
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
        if (item.choice.kind === "soundRecord") {
          item.state = "fetching";
          publish();
          await this.holdRecoveryExport(version.content, item.choice, preview.body, target.server, version.group, live);
          if (!live()) return;
        }
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
        const record = this.enqueue({ kind: "recoveryApply", object,
          ...(target.objectKind === "score" ? { objectKind: "score" as const } : {}), edit, applied: null, label: `${mode}: ${item.label}` });
        const outcome = await new Promise<SaveOutcome>((resolve) => { record.settle = resolve; });
        if (!live()) return;
        if (outcome !== "landed") {
          // The complete payload is on the save record; only its exact retry may finish it. Each
          // outcome says what is true of that record: an uncertain apply may have landed, one that
          // was not written is still queued (blocked) for retry or discard, a discard is gone.
          item.state = outcome === "uncertain" ? "uncertain" : "error";
          item.error = outcome === "uncertain" ? "apply uncertain; retry the same request from the save card, then run again"
            : outcome === "notWritten" ? "apply not written; retry or discard it from the save card, then run again" : "discarded";
          run.status = "stopped";
          run.error = outcome === "uncertain" ? "stopped at an uncertain apply" : outcome === "notWritten" ? "stopped at an apply that was not written" : "stopped at a discarded apply";
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
    const objectKind = object === null ? "index" : this.objectKind(object);
    const live = () => this.current(gen) && (object === null ? "index" : this.objectKind(object)) === objectKind;
    this.pointer = { status: "running", error: "", result: null };
    this.notify();
    try {
      const r = await recoveryRestorePointer(this.ipc, this.objectTarget(object));
      if (!live()) return;
      this.pointer = { status: "done", error: "", result: r };
    } catch (e) {
      if (!live()) return;
      this.pointer = { status: "blocked", error: reason(e), result: null };
    }
    this.notify();
  }
}

/// Domain choices shared by historical recovery and retained-draft copy. Keeping one enumerator is
/// important: a newly supported operation must not silently appear in one recovery surface while
/// remaining impossible to preserve from the other.
function choicesForContent(content: StudioContent, mode: RecoveryMode): { choice: RecoveryChoice; label: string }[] {
  const out: { choice: RecoveryChoice; label: string }[] = [];
  if (content.kind === "flipnote") {
    content.timeline.forEach((id, i) => {
      const entry = content.frames[id];
      if (entry) out.push({ choice: { kind: "frame", id, value: entry.pixels.selected.source.opId }, label: `frame ${i + 1}` });
    });
    out.push(...soundRecoveryChoices(content.sound, mode));
    if (mode === "copy") {
      if (content.title) out.push({ choice: { kind: "title", value: content.title.selected.source.opId }, label: `title "${content.title.selected.value}"` });
      if (content.fps) out.push({ choice: { kind: "fps", value: content.fps.selected.source.opId }, label: `${content.fps.selected.value} fps` });
      for (const id of Object.keys(content.tombstones)) out.push({ choice: { kind: "frameDeletion", id }, label: `deletion of frame ${id.slice(0, 8)}` });
    }
  } else if (content.kind === "score") {
    // Recipes precede cells so a copied/restored note can use its prerequisite. A whole-version
    // walk follows the effective selection; raw alternatives remain available in the read model.
    const selected = <T>(slot: { removed: boolean; register: { selected: { value: T | null; source: { opId: string } }; conflicts: { value: T | null; source: { opId: string } }[] } }) =>
      slot.removed ? [slot.register.selected, ...slot.register.conflicts].filter((value) => value.value === null)
        .sort((a, b) => a.source.opId < b.source.opId ? -1 : 1)[0] : slot.register.selected;
    for (const [patch, slot] of Object.entries(content.patches).sort(([a], [b]) => a < b ? -1 : 1)) {
      const value = selected(slot);
      if (value && (mode === "copy" || value.value !== null)) out.push({ choice: { kind: "scorePatch", patch, value: value.source.opId },
        label: `${value.value === null ? "remove" : "recipe"} ${patch.slice(0, 8)}` });
    }
    if (mode === "copy") for (const field of ["title", "bpm", "bpb", "steps"] as const) {
      const header = content[field];
      if (header) out.push({ choice: { kind: "scoreHeader", field, value: header.selected.source.opId }, label: `score ${field}` });
    }
    for (const [key, slot] of Object.entries(content.cells).sort(([a], [b]) => {
      const [as, ar] = a.split(":").map(Number), [bs, br] = b.split(":").map(Number); return as - bs || ar - br;
    })) {
      const value = selected(slot), [step, row] = key.split(":").map(Number);
      if (value && (mode === "copy" || value.value !== null)) out.push({ choice: { kind: "scoreCell", step, row, value: value.source.opId },
        label: `${value.value === null ? "clear" : "cell"} ${key}` });
    }
  } else if (content.kind === "index") {
    for (const [id, entry] of [...Object.entries(content.objects), ...Object.entries(content.overflow)]) {
      out.push({ choice: { kind: "object", id }, label: `entry "${entry.title.selected.value}"` });
      if (mode === "copy") {
        out.push({ choice: { kind: "objectTitle", id, value: entry.title.selected.source.opId }, label: `title "${entry.title.selected.value}"` });
        out.push({ choice: { kind: "objectExpiry", id, value: entry.expiry.selected.source.opId }, label: `expiry of "${entry.title.selected.value}"` });
      }
    }
    if (mode === "copy") for (const id of Object.keys(content.tombstones)) out.push({ choice: { kind: "objectDeletion", id }, label: `deletion of entry ${id.slice(0, 8)}` });
  }
  return out;
}

function laneOf(s: SaveRecord): string {
  return s.kind === "applyIndex" ? "index" : s.object ?? "index";
}

/** The document a record changes, for "this document's work" (its Save, its pending indicators, the
 * sidebar's held rows), never for routing. A document record names it as `object`, and so does an
 * ordinary Index record. A Closing or Unconfirmed Index record keeps the channel Index as its
 * native target (`object: null`, which must not change: the local-draft branch is the Index's),
 * and names the entry it edits only inside its operation. A title edit therefore spans both kinds,
 * and a document's Save must cover both. Null: an Index record about no one entry. */
export function affectedObject(s: SaveRecord): Hex32 | null {
  if (s.object !== null) return s.object;
  if (s.kind === "recoveryApply") {
    // An Index recovery (copying one entry's listing, title, expiry or deletion back from history)
    // names that entry in its choice; a whole-Index recovery names none.
    const choice = s.edit.choice;
    return choice.kind === "object" || choice.kind === "objectTitle" || choice.kind === "objectExpiry" || choice.kind === "objectDeletion" ? choice.id : null;
  }
  if (s.kind !== "overlayApply") return null;
  const entry = (s.op as { object?: unknown }).object;
  return typeof entry === "string" ? entry : null;
}

const jobKey = (generation: number, cid: Hex64, bytes: number) => `${generation}:${cid}:${bytes}`;
const contentKey = (cid: Hex64, bytes: number) => `${cid}:${bytes}`;

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

/// Native refusals raised before the execution lease is transferred to the server actor, so the
/// operation never ran and nothing of it was written.
/// Exact bridge messages (apps/desktop/src-tauri; studio-native-no-write.test.ts pins them against
/// that source): the actor's five-second rendezvous was lost, after which the actor never receives
/// the request's lease and drops it; the bridge's shared inline slots were all taken, which is
/// refused before the actor is contacted; and the vault-lease locks were busy, which `authorize`
/// reports before the lease is sent (the actor then drops the request). Matching is exact on
/// purpose: other texts, such as "superseded", are raised both before and after a write and must
/// stay uncertain.
export const NATIVE_NO_WRITE = ["Studio actor busy; retry", "too many inline downloads are already active", "Studio storage busy; retry the same request"] as const;
export function nativeNoWrite(e: unknown): boolean {
  const text = reason(e);
  return NATIVE_NO_WRITE.some((message) => text === message);
}

export function reason(e: unknown): string {
  if (e instanceof StudioNativeError) return e.reason;
  if (e instanceof Error) return e.message;
  return String(e);
}

function overlayRefusalText(reason: OverlaySaveRefusal): string {
  switch (reason) {
    case "exportUnpublished": return "exportUnpublished: the PIXA package has no matching native publication receipt; explicitly publish again before a new Save";
    case "exportInvalid": return "exportInvalid: the PIXA package failed complete media validation; no reference was written";
    case "exportMissing": return "exportMissing: the published PIXA package is no longer held; explicitly publish again before a new Save";
    default: return reason;
  }
}

export { DEFAULT_FPS };
