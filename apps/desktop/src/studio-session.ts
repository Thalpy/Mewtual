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
//   until it is retried (on its own, scheduleAutoRetry, or by the member) or discarded on purpose; other documents
//   keep moving. Navigation never waits on a Save: leaving hands the edits to this queue and goes.
// - Unsaved pixels survive every failure. They stay on the record until the save lands or the
//   member discards them on purpose.
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
import { busyRetrying } from "./studio-busy.ts";
import type { SaveTally } from "./studio-save-words.ts";
import { workingFlipnote, workingScore, workingIndex, workingSlot } from "./studio-working.ts";
import { encodeStudioDraft, validateDraftOperation, type StudioDraft } from "./studio-draft.ts";
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
  studioApplyBatch,
  STUDIO_APPLY_BATCH_MAX,
  STUDIO_APPLY_BATCH_MAX_BYTES,
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
/** Records the draft queue holds before an edit is refused; a lock snapshot must fit them all. */
const DRAFT_QUEUE_MAX = 512;
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
/// This is the second of two layers. Every call through `this.ipc` first repeats a storage-busy
/// refusal itself (busyRetrying in studio-busy.ts: five quick repeats, 1.24 s of pauses), so only a
/// storage-busy that outlasts those reaches this layer. A Save refused that way throughout makes
/// 4 × 6 calls of its request over about 6.7 s before it settles, inside the departure timeout
/// unless a first-delivery admission read (no view of the document held) is refused the same way
/// and adds as much again. The other two no-write texts are not repeated by that wrapper and
/// arrive here at once.
const BUSY_BACKOFF_MS = [250, 500, 1000];
/// Pauses before each automatic exact resend of a Save that failed (see scheduleAutoRetry), in
/// order; their sum (about two minutes) bounds how long a failure stays "trying again" before it is
/// handed to the member. This is the third layer, above the two busy repeats: it covers what those
/// cannot, such as a refusal that outlasts their 1.75 s, a lost receipt, or a batch refused as a
/// whole. An exact resend is always safe on the routes it covers: native answers an operation it
/// already holds as saved (`validate_local_edit` returns early on the held exact op), and a request
/// refused before dispatch never ran. Production only (`autosaveMs > 0`); the immediate test seam
/// keeps every failure where the test left it.
const AUTO_RETRY_MS = [1000, 2000, 4000, 8000, 15_000, 30_000, 60_000];
/// Refusals that no resend of the same request can change: the member decides (save as a new edit,
/// where allowed, or discard). The native texts come from catcoms-app (store/epoch_studio.rs,
/// studio.rs, studio_exchange.rs). An exact retry after "Shared history changed" would also skip
/// admission (it is no longer a first delivery) and send the stale request, which native then
/// refuses as a retired epoch and which can no longer be saved as a new edit, so `retry` refuses
/// these outright.
const NEEDS_DECISION = [/Shared history changed/, /server for these saved edits changed/, /edit belongs to a retired epoch/,
  /object already exists/, /already occurs in the index/, /Studio requires current membership/, /local device is not a current member/,
  /unknown Studio channel/];

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
    const request = ++this.navigationSerial, lease = this.leaseGeneration;
    return () => request === this.navigationSerial && lease === this.leaseGeneration;
  }
  /// What a navigation lease is fenced by: `generation`, except that a deferred channel switch
  /// (setScope with `deferred`, made when the previous channel's queue finally drains) does not
  /// advance it. That switch was decided when the Studio mounted on the new channel, so an App
  /// route the member started while it waited must not be retired by its arrival.
  private leaseGeneration = 0;
  get closing(): boolean { return this.closeHeld; }
  /** Sidebar and Music share one navigation owner; a newer action wins across both surfaces.
   *
   * Leaving a document hands its edits to the background Save and navigates at once. It never waits
   * for a receipt: a slow or failed Save of the document being left must not keep the member in it.
   * Its edits stay on their records (pixels included), keep their lane order, and are retried on
   * their own (scheduleAutoRetry) or shown in that document's editor when only the member can move
   * them. Staging the mounted drawing still runs first and can refuse (a full draft queue), which
   * keeps the member and the stroke where they are.
   *
   * Only the document being left is sent. Records queued for any other document (a lock/unlock
   * restore leaves them offscreen, as does deleting a document from the Index while its edits are
   * still queued; neither passes through a departure) stay exactly as they are: their Retry,
   * reauthor and discard controls live in that document's own editor. With no document open
   * nothing is being left, so nothing is sent and restored work stays held until an explicit Save
   * or retry. Leaving the Studio surface sends everything (`sendBeforeLeaving`); closing the window
   * still waits for every receipt (`saveBeforeLeaving`). */
  async navigate(action: () => void): Promise<void> {
    if (this.closeHeld) throw new StudioNativeError("The window is closing. Wait for saving to finish.");
    const current = this.claimNavigation();
    const leaving = this.doc?.object ?? null;
    if (leaving !== null) {
      this.prepareDocumentClose();
      this.stageDrawing();
      this.releaseQueued(true, s => laneOf(s) === leaving);
      // The document's records in the shared Index lane (the Index half of a rename, an expiry) are
      // its work too, but they go only once the document's own lane has drained: a refused document
      // half then leaves a restored Index half held with it, so this navigation does not split the
      // rename. (An ordinary Index half still goes at its own five-second deadline, and leaving the
      // Studio sends everything, as before; either can split a rename whose document half fails.)
      // Released even if a newer action superseded this navigation (the document half has been
      // sent), but not across a reset (releaseAfterDrain records the generation).
      this.releaseIndexHalfAfterDrain(leaving);
    }
    if (!current()) return;
    action();
  }

  /** Release `object`'s Index-lane records now if its own lane is empty, or once it drains. */
  private releaseIndexHalfAfterDrain(object: Hex32): void {
    if (this.records.some(s => laneOf(s) === object)) { this.releaseAfterDrain.set(object, this.generation); return; }
    this.releaseAfterDrain.delete(object);
    this.releaseQueued(true, s => laneOf(s) !== object && affectedObject(s) === object);
  }

  /** Leaving the Studio surface: hand every retained edit to the background Save, without waiting
   * for any receipt. Throws only when the mounted drawing could not be staged (the draft queue is
   * full); the caller must then keep the member, and that stroke, where they are. */
  sendBeforeLeaving(): void {
    if (this.closeHeld) throw new StudioNativeError("The window is closing. Wait for saving to finish.");
    this.prepareDocumentClose();
    this.stageDrawing();
    this.cancelAutosaveTimer();
    this.releaseQueued(true);
  }

  /** Edits a departure must still hand over: queued records not yet released for sending (waiting
   * for their deadline, or restored and held). Records in flight, cooling or retrying on their own
   * are already the background Save's, and a failed one only the member can move is shown, never
   * waited on, so neither keeps anyone in the Studio. */
  hasUnsentWork(): boolean {
    return this.records.some(s => s.status === "queued" && !this.eligible.has(s.id));
  }

  /** True while the session still holds edits of the channel it is on. A production scope change
   * then waits for them (setScope refuses; studio-state defers the switch until they land). */
  get retainsWork(): boolean { return this.autosaveMs > 0 && this.records.length > 0; }
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
  /// Automatic resends of failed Saves already made, per record (see AUTO_RETRY_MS). Kept across a
  /// manual retry, so a failure that keeps coming back waits longer each time.
  private readonly autoRounds = new Map<number, number>();
  /// The next automatic resend of a failed Save, per record: when it is due and how to cancel it.
  private readonly autoTimers = new Map<number, { at: number; cancel: () => void }>();
  /// Documents left while their own lane still held edits: once that lane drains, their records in
  /// the shared Index lane (the Index half of a rename) are released too. Valued by the generation
  /// the departure happened in, so a reset never releases restored work nobody chose to send.
  private readonly releaseAfterDrain = new Map<Hex32, number>();
  /// Saves the member discarded on purpose, ever (`discard`, not a coalesced replacement): a
  /// watcher compares it before and after, so a queue emptied by discarding is never read as saved.
  private discardedTotal = 0;
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
    this.schedule = opts.schedule ?? ((fn, ms) => { const t = setTimeout(fn, ms); return () => clearTimeout(t); });
    // Every native call of this session repeats a storage-busy refusal a bounded number of times
    // before reporting it (studio-busy.ts); the generation check ends a repeat that a reset or
    // scope change has orphaned. The timer seam is the session's own, so tests drive the pauses.
    this.ipc = busyRetrying(opts.ipc, this.schedule, () => this.generation);
    this.me = opts.me;
    this.now = opts.now ?? (() => Date.now());
    this.newId = opts.newId ?? (() => randomElementId());
    this.maxFetches = opts.maxFetches ?? 2;
    this.cacheBytesCap = opts.blobCacheBytes ?? BLOB_CACHE_BYTES;
    this.autosaveMs = opts.autosaveMs ?? 0;
  }

  // --- Change signal -----------------------------------------------------------------------

  onChange(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
  private notify(): void {
    // The queue entries are mutated in place (their identity is what a retry resends); the
    // surfaces get fresh copies so a keyed row or a `find` never holds a stale object.
    this.saves = this.records.map(({ settle: _settle, ...rest }) => rest as SaveRecord);
    const pending = this.doc ? this.saves.filter(s => s.kind !== "applyIndex" && s.object === this.doc!.object) : [];
    this.workingModel = this.doc?.model ? (pending.length ? workingFlipnote(this.doc.model, pending, this.me) : this.doc.model) : null;
    this.workingScore = this.doc?.score ? (pending.length ? workingScore(this.doc.score, pending) : this.doc.score) : null;
    this.workingIndex = this.indexModel ? (this.saves.length ? workingIndex(this.indexModel, this.saves) : this.indexModel) : null;
    this.updateEditorMedia();
    for (const fn of this.listeners) fn();
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

  /// Point the session at a channel. Production callers must not call this while `retainsWork`
  /// (studio-state's setStudioScope defers the switch until the queue drains, then calls this with
  /// `deferred`); retained work refuses a scope replacement. Lock uses snapshot + reset instead. The
  /// immediate test seam retains its original reset behavior for scope-fencing regressions.
  setScope(scope: StudioScope | null, opts: { deferred?: boolean } = {}): void {
    if (this.scope && scope && this.scope.server === scope.server && this.scope.channel === scope.channel) return;
    if (!this.scope && !scope) return;
    if (this.closeHeld) throw new StudioNativeError("The window is closing. Wait for saving to finish.");
    // Production callers check `retainsWork` first and defer the switch (studio-state), so this is
    // a guard rather than a message anyone normally reads.
    if (this.autosaveMs && this.records.length) throw new StudioNativeError("Studio edits from this channel are still saving.");
    const sync = scope ? this.syncByServer.get(scope.server) ?? null : null;
    this.clearScoped();
    // Sync belongs to the server, not a channel or editor mount. Restore its ordering watermark
    // across both channel navigation and A → B → A server navigation.
    this.syncObservation = sync;
    this.receivePaused = sync?.paused ?? false;
    this.receivePauseDetail = sync?.detail ?? "";
    this.scope = scope;
    this.generation++;
    if (!opts.deferred) this.leaseGeneration++;
    this.installRestoredDraft();
    this.notify();
    if (scope) this.invalidate({ index: true });
  }

  /// Lock or session change: forget everything, including cached pixels and unsaved work.
  reset(): void {
    this.clearScoped();
    this.syncByServer.clear(); // pause details are private session state, cleared on lock
    this.scope = null;
    this.restoredDraft = null;
    this.generation++; this.leaseGeneration++;
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
    for (const timer of this.autoTimers.values()) timer.cancel();
    this.autoTimers.clear(); this.autoRounds.clear(); this.releaseAfterDrain.clear();
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
    // Every event invalidates the Index; a named object additionally invalidates that object.
    // A blob that was unavailable or failed may have arrived with it: let the next request try.
    if (ev.object) this.releaseTransientFor(ev.object);
    // Data notices may echo our just-acknowledged Save. Re-read them, then compare effective
    // content before stopping local playback. Settlement/membership invalidations still revoke
    // immediately; an unchanged delivery echo is not a new animation source.
    this.invalidate({ index: true, objects: ev.object ? [ev.object] : [], dataNotice: true });
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
    if (this.pendingIndex) { this.pendingIndex = false; void this.refreshIndex(); }
    for (const o of this.pendingObjects) {
      if (this.doc?.object === o) void this.refreshDoc(o);
      else this.known.delete(o); // its trust state is no longer known until read again
    }
    this.pendingObjects.clear();
    if (this.pendingRecovery) { this.pendingRecovery = false; void this.refreshRecovery(); }
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

  /** Capture the displayed projection, not a silently substituted ordinary base beneath a draft.
   *
   * `playback` is local listening and watching: it plays exactly what the editor shows (the working
   * projection, which includes every edit still on the queue), so whether those edits have saved
   * yet, or failed to, is not its concern and never refuses it. An export or publication hands the
   * content to someone else and still refuses while any of this document's saves has failed. */
  private mediaSource(object: Hex32, kind: StudioObjectKind, playback = false): { view: StudioObjectView; server: number; assertCurrent: () => void } {
    const doc = this.doc, scope = this.target(), gen = this.generation, epoch = this.mediaEpoch, editor = this.editorEpoch;
    const draft = this.hasDocumentDraft(object);
    const failedSave = () => !playback && this.pendingFor(object).some(s => s.status === "blocked" || s.status === "uncertain" || s.mayHaveCommitted);
    if (draft && !this.autosaveMs) throw new StudioNativeError("finish or reconcile this document's pending saves before playback/export");
    if (!doc || doc.object !== object || doc.objectKind !== kind || !doc.view || doc.loading || this.overlay?.loading) throw new StudioNativeError("wait for this document to finish loading");
    if (failedSave()) throw new StudioNativeError("This document has an edit that did not save. Retry or discard it before exporting.");
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
          this.doc?.object !== object || this.doc.objectKind !== kind || failedSave() || (!draft && (epoch !== this.mediaEpoch || this.doc.view !== base || this.doc.model !== model || this.doc.score !== score ||
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
    return (await this.prepareScorePlayback(object, { playback: false })).take;
  }

  /** The same take plus its authored grid end, which Studio looping must honour. Playback reads the
   * working projection and so is never refused by save status; the take export (`prepareScoreTake`,
   * for download, share and chat) passes `playback: false` and still is. */
  prepareScorePlayback(object: Hex32, opts: { playback?: boolean } = { playback: true }): Promise<ScorePlayback> {
    return this.oneMedia(async () => {
      const source = this.mediaSource(object, "score", opts.playback ?? true);
      const playback = this.hasDocumentDraft(object) && this.workingScore
        ? await exportScorePlayback(this.scoreInput(object, this.workingScore), this.mediaContext(source.view))
        : await scorePlaybackFromNative(object, source.view as ScoreView, this.me);
      source.assertCurrent();
      return playback;
    });
  }

  /** `playback` as in mediaSource: Play builds from what is on screen whatever its save status,
   * including a linked Score's edits that have not saved yet. */
  private async preparePixa(object: Hex32, playback = false): Promise<{ bytes: Uint8Array; server: number; assertCurrent: () => void }> {
    const source = this.mediaSource(object, "flipnote", playback), view = source.view as FlipnoteView;
    let draft = this.hasDocumentDraft(object);
    const model = draft ? this.workingModel! : flipnoteModel(view.content);
    if (!draft || view.content.timeline.length) preflightFlipnoteMedia(view);
    else if (Object.keys(view.content.frames).length) throw new StudioNativeError("The animation timeline is incomplete.");
    if (!model || model.overCapCount || model.titleConflicts.length || model.fpsConflicts.length || !model.sound.ready ||
        model.frames.some(f => f.conflicts.length || f.insertions > 1)) throw new StudioNativeError("Resolve animation conflicts before playing or exporting.");
    let linked: { object: string; view: ScoreView } | undefined, linkedWorking: ScoreModel | undefined;
    if (model.sound.score) {
      const linkedDraft = this.hasDocumentDraft(model.sound.score);
      if (linkedDraft && !playback) throw new StudioNativeError("The linked music has edits that are still saving. Export again once they have saved.");
      const score = await studioRead(this.ipc, this.target(), model.sound.score, "score");
      source.assertCurrent();
      if (!score) throw new StudioNativeError("the linked Score is not held locally; no shorter silent export was made");
      if (score.channel !== view.channel || !score.group || score.group !== view.group) throw new StudioNativeError("The linked music belongs to a different server or channel.");
      linked = { object: model.sound.score, view: score };
      // The linked Score's own queued edits (it was edited, then left, and they are still saving):
      // play them as its editor showed them. That needs the projection export below, which builds
      // from models rather than from the native views.
      if (linkedDraft) {
        const queued = this.saves.filter(s => s.kind !== "applyIndex" && s.object === model.sound.score);
        linkedWorking = workingScore(scoreModel(score.content), queued);
        draft = true;
      }
    }
    const held = new Map<string, Uint8Array>();
    for (const frame of model.frames) {
      const key = contentKey(frame.cid, frame.bytes);
      if (held.has(key)) continue;
      const bytes = this.unsavedPix(object, frame.id) ?? await this.fetchNow(frame.cid, frame.bytes);
      source.assertCurrent();
      if (!bytes) throw new StudioNativeError("an animation frame is unavailable; no partial export was made");
      held.set(key, bytes);
    }
    const bytes = draft ? await exportFlipnotePixa({ id: object, epoch: BigInt(view.epoch), fps: model.fps,
      frames: model.frames.map(f => ({ id: f.id, bytes: held.get(contentKey(f.cid, f.bytes))! })),
      patches: model.sound.patches, ...(linked ? { score: this.scoreInput(linked.object, linkedWorking ?? scoreModel(linked.view.content)) } : {}),
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

  /** The flipnote's package. Play passes `playback: true` (never refused by save status); a
   * download or a post to chat leaves it false. */
  prepareFlipnotePixa(object: Hex32, opts: { playback?: boolean } = {}): Promise<Uint8Array> {
    return this.oneMedia(async () => (await this.preparePixa(object, opts.playback ?? false)).bytes);
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
    if (this.restoredDraft) throw new StudioNativeError("Open the channel with your restored Studio edits before making more changes.");
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

  /** The channel a restored lock snapshot belongs to while it waits for that channel to open, or
   * null. While it waits, every new edit is refused (enqueue), so a snapshot whose channel can no
   * longer be opened (the server was left, the channel deleted) would block the Studio for good
   * without `discardRestoredDraft`. */
  get pendingRestoredScope(): StudioScope | null { return this.restoredDraft ? { ...this.restoredDraft.scope } : null; }

  /** Throw away a restored snapshot that is still waiting for its channel. Its edits were never
   * sent from this session; nothing native changes. The member's explicit choice, behind the
   * surface's confirmation. */
  discardRestoredDraft(): void {
    if (!this.restoredDraft) return;
    this.discardedTotal += this.restoredDraft.records.length;
    this.restoredDraft = null;
    this.notify();
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
      return new StudioNativeError(`This Save waits behind an unsaved edit (${blocker.label}${listed?.title ? ` of "${listed.title}"` : ""}). ${choice} that edit first.`);
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
          // An awaited Save answers at the first failure, even one the session will resend on its
          // own: the caller (an explicit Save, the window close) reports it now, and the edit stays
          // queued for that resend whatever the caller does next.
          else if (failure) reject(new StudioNativeError(this.autoTimers.has(failure.id)
            ? `A Studio edit is ${failure.mayHaveCommitted ? "not confirmed" : "not saved"} yet. The Studio is trying again on its own.`
            : failure.mayHaveCommitted
              ? "A Studio edit is not confirmed. Retry or discard it in the Studio."
              : "A Studio edit was not saved. Retry or discard it in the Studio."));
          else if (behind) reject(stuck(behind));
          else resolve();
        }
      };
      detach = this.onChange(check);
      cancel = this.schedule(() => { detach(); reject(new StudioNativeError("Still saving. Try again in a moment.")); }, 10_000);
      check();
    });
  }

  /** The first uncovered record that keeps a covered, queued one of `ids` from ever running: it is
   * ahead of it in its lane and only the member can move it (failed with no automatic resend due,
   * or restored and held). A record waiting for its deadline, in flight, or failed but due to be
   * resent on its own will move without the member. */
  private stuckBehind(ids: Set<number>): SaveEntry | null {
    for (const s of this.records) {
      if (!ids.has(s.id) || s.status !== "queued") continue;
      const lane = laneOf(s);
      for (const r of this.records) {
        if (r === s) break;
        if (ids.has(r.id) || laneOf(r) !== lane) continue;
        if (((r.status === "blocked" || r.status === "uncertain") && !this.autoTimers.has(r.id)) || (r.status === "queued" && this.restoredHeld.has(r.id))) return r;
      }
    }
    return null;
  }

  /** Every retained edit must reach a definite receipt: closing the window, and any caller that
   * must know the edits landed. Navigation does not use this; it hands edits to the background
   * Save (`navigate`, `sendBeforeLeaving`). */
  async saveBeforeLeaving(): Promise<void> { await this.saveUntilClear(() => true); }

  private async saveUntilClear(covers: (s: SaveRecord) => boolean): Promise<void> {
    this.prepareDocumentClose();
    const started = this.now(), generation = this.generation;
    do {
      await this.saveCovered(covers);
      if (generation !== this.generation) throw new StudioNativeError("Studio changed while saving.");
      this.stageDrawing(); // capture a stroke made while the previous revision awaited its ACK
      if (this.now() - started > 10_000 && this.records.some(covers)) throw new StudioNativeError("Still saving. Try again in a moment.");
    } while (this.records.some(covers));
  }

  private checkDraftBudget(next: SaveRecord, replacing?: SaveRecord): void {
    const entries = [...this.records.filter(s => s !== replacing), next];
    if (entries.length > DRAFT_QUEUE_MAX) throw new StudioNativeError("Too many unsaved edits. Save before continuing.");
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

  /// Resend the same complete request: the member's "Retry now", and every automatic resend.
  retry(id: number): void {
    const s = this.records.find((r) => r.id === id);
    if (!s || (s.status !== "uncertain" && s.status !== "blocked")) return;
    const retry = s.kind === "overlayApply" || s.kind === "frame" ? s.overlayRetry : null;
    if (retry === "newTicket" || retry === "none") return;
    if (this.needsDecision(id)) return; // resending cannot change it; see NEEDS_DECISION
    this.cancelAutoRetry(s.id); // this resend replaces the one that was due
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
    this.eligible.delete(s.id); this.restoredHeld.delete(s.id); this.forgetAutoRetry(s.id);
    this.eligible.add(replacement.id);
    this.settleRecord(s, "discarded");
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
    this.forgetAutoRetry(s.id);
    // A departure waiting for this lane to drain gives up when the member empties it by discarding:
    // its Index half (a rename's other half) then stays where it was, for the member to decide.
    if (!this.records.some(r => laneOf(r) === laneOf(s))) this.releaseAfterDrain.delete(laneOf(s));
    this.discardedTotal++;
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

  /** How many more edits the draft queue takes before `enqueue` refuses: a multi-step gesture
   *  asks before it starts, so it is never left half queued. */
  draftRoom(): number {
    return this.closeHeld || this.restoredDraft ? 0 : Math.max(0, DRAFT_QUEUE_MAX - this.records.length);
  }

  /// `null` is the channel Index's own lane (every Index record, of any route); a document is every
  /// record that changes it, including its Index records (see `affectedObject`).
  pendingFor(object: Hex32 | null): SaveRecord[] {
    return this.saves.filter((s) => object === null ? s.kind === "applyIndex" || s.object === null : affectedObject(s) === object);
  }

  /// Whether a failed save was refused for a reason no resend can change (NEEDS_DECISION): its
  /// only ways forward are "Save as a new edit" (where `canReauthor`) and discard. `retry` refuses
  /// it, and surfaces should not offer Retry for it.
  needsDecision(id: number): boolean {
    const s = this.records.find(r => r.id === id);
    return !!s && (s.status === "blocked" || s.status === "uncertain") && NEEDS_DECISION.some(pattern => pattern.test(s.error));
  }

  /// Whether the member's retry would resend anything: a failed save whose retry identity is not a
  /// spent local-draft ticket and whose refusal a resend could change.
  canRetry(id: number): boolean {
    const s = this.records.find(r => r.id === id);
    if (!s || (s.status !== "uncertain" && s.status !== "blocked")) return false;
    const retry = s.kind === "overlayApply" || s.kind === "frame" ? s.overlayRetry : null;
    return retry !== "newTicket" && retry !== "none" && !this.needsDecision(id);
  }

  /// When a failed save will next be resent on its own (a `now()` timestamp), or null when it will
  /// not be: it is not failed, it is restored and held, its refusal needs the member, or its
  /// automatic resends are used up. Display only; `retry` is what resends.
  autoRetryAt(id: number): number | null {
    return this.autoTimers.get(id)?.at ?? null;
  }

  /// Outstanding saves of this session, for the background-save toast and the scope wait.
  saveTally(): SaveTally {
    let retrying = 0, stuck = 0;
    for (const s of this.records) {
      if (s.status !== "blocked" && s.status !== "uncertain") continue;
      if (this.autoTimers.has(s.id)) retrying++; else stuck++;
    }
    return { left: this.records.length, retrying, stuck, discarded: this.discardedTotal };
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
    try {
      if (s.attempts === 1 || s.kind === "frame" && !s.apply && !s.overlay || s.kind === "overlayApply" && !s.request) this.assertAdmission(s);
      if (s.kind === "create") {
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
          await this.runUnconfirmed(s, s.object, canonicalJson(op), gen);
          if (!this.current(gen)) return;
          await this.refreshDoc(s.object);
          if (!this.current(gen)) return;
        } else {
          if (!s.apply) {
            this.assertAdmission(s);
            const ep = this.editableEpoch(s.object);
            if (!("epochId" in ep)) throw new OverlayNoWrite(ep.refused);
            s.apply = { object: s.object, epochId: ep.epochId, nonce: this.newId(), body: canonicalJson(op) };
          }
          const view = await studioApply(this.ipc, target, s.apply);
          if (!this.current(gen)) return;
          await this.landed(s.object, view, docRead);
        }
      } else if (s.kind === "apply") {
        const batch = this.batchable(s);
        if (batch.length) {
          // The members share this record's lane, epoch and authoring binding, so the admission
          // this record passed above covers them. Each still counts its own attempt: a lock
          // snapshot taken now must restore every member as an interrupted delivery, and a later
          // retry of any member must not skip admission, exactly as for the first record.
          for (const r of batch) { r.status = "inflight"; r.attempts++; }
          this.notify();
          try {
            const view = await studioApplyBatch(this.ipc, target, { object: s.object, epochId: s.request.epochId,
              operations: [s, ...batch].map((r) => ({ nonce: r.request.nonce, body: r.request.body })) });
            if (!this.current(gen)) return;
            await this.landed(s.object, view, docRead);
            for (const r of batch) this.finish(r);
          } catch (e) {
            if (!this.current(gen)) return;
            // A pre-dispatch refusal (nativeNoWrite) wrote no item. While `s` has resends left, its
            // members go back to queued behind it, uncounted exactly as resendLater leaves `s`, and
            // travel again with its resend. Settling them here instead would strand them as "not
            // written" cards behind a head that is quietly resent, and the lane would then wait on
            // the member's explicit retry. Otherwise there is one native outcome for the whole
            // batch: every member settles the same way `s` does below (known not written once the
            // resends are used up, otherwise uncertain, since native may have written the items
            // before the one it names). resendLater changes nothing when it returns false, so the
            // outer catch asking it again settles `s` alike.
            if (nativeNoWrite(e) && this.resendLater(s)) {
              for (const r of batch) { r.status = "queued"; if (!r.mayHaveCommitted) r.attempts = Math.max(0, r.attempts - 1); }
              this.notify();
              return;
            }
            for (const r of batch) this.settleFailure(r, e);
            throw e;
          }
        } else {
          const view = await studioApply(this.ipc, target, s.request);
          if (!this.current(gen)) return;
          await this.landed(s.object, view, docRead);
        }
      } else if (s.kind === "applyIndex") {
        const view = await studioApplyIndex(this.ipc, target, s.request);
        if (!this.current(gen)) return;
        await this.landedIndex(view, indexRead);
      } else if (s.kind === "overlayApply") {
        await this.runUnconfirmed(s, s.object, canonicalJson(s.op), gen);
        if (!this.current(gen)) return;
        if (s.object) await this.refreshDoc(s.object);
        else await this.refreshIndex();
        if (!this.current(gen)) return;
      } else {
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
      if (nativeNoWrite(e) && this.resendLater(s)) return;
      this.settleFailure(s, e);
    }
  }

  /**
   * The queued Score edits that travel with `s` in one native transaction: the records after it
   * in its lane, in order, while each is an ordinary Score Apply against the same epoch with the
   * same authoring binding, released for sending, and the batch stays inside native's count and
   * byte bounds. Gathering stops at the first record that does not qualify, so the lane's order
   * is kept exactly. A lone edit still travels as `studio_apply`, which also keeps every
   * single-edit expectation (and test) unchanged. Flipnote and Index edits never batch: their
   * native admission inspects each body (blob custody, link evidence) one request at a time.
   */
  private batchable(s: Extract<SaveEntry, { kind: "apply" }>): Extract<SaveEntry, { kind: "apply" }>[] {
    const out: Extract<SaveEntry, { kind: "apply" }>[] = [];
    if (s.request.objectKind !== "score") return out;
    // Only a first delivery has just passed admission on the members' behalf. An exact retry
    // skips admission on purpose, and a member riding with it would skip the "history changed"
    // check it gets alone, losing its known-not-written outcome. A retried record travels alone.
    if (s.attempts !== 1) return out;
    const lane = laneOf(s);
    let bytes = utf8Length(s.request.body), after = false;
    for (const r of this.records) {
      if (r === s) { after = true; continue; }
      if (!after || laneOf(r) !== lane) continue;
      // A member that was already sent once (it rode a batch that failed after dispatch) travels
      // alone from then on, like a retried head. Whatever refused that batch (one bad item, a
      // bridge that will not take batches) then cannot refuse the same batch again: each item is
      // resent on its own, the ones native already holds land as saved, and only the item that is
      // actually refused stays behind.
      if (r.kind !== "apply" || r.status !== "queued" || r.attempts !== 0 || r.mayHaveCommitted || r.request.objectKind !== "score" || r.request.epochId !== s.request.epochId
        || (this.autosaveMs !== 0 && !this.eligible.has(r.id)) || this.restoredHeld.has(r.id) || !sameBinding(r.binding, s.binding)) break;
      bytes += utf8Length(r.request.body);
      if (out.length + 1 >= STUDIO_APPLY_BATCH_MAX || bytes > STUDIO_APPLY_BATCH_MAX_BYTES) break;
      out.push(r);
    }
    return out;
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
    // do the native refusals raised before the execution lease is transferred (nativeNoWrite) and the
    // retired-epoch refusal ahead of native's intent barrier (nativeRefusedBeforeWrite). Other native
    // errors remain uncertain because they may have crossed a final delivery fence after commit.
    const noWrite = e instanceof OverlayNoWrite || nativeNoWrite(e) || nativeRefusedBeforeWrite(e);
    const earlierAttemptWasUncertain = s.mayHaveCommitted;
    // A pre-dispatch refusal is not a delivery: it does not use up the first-attempt admission
    // check, so the member's retry reads and compares the document again before sending.
    if (nativeNoWrite(e) && !earlierAttemptWasUncertain) s.attempts = Math.max(0, s.attempts - 1);
    if (!noWrite) s.mayHaveCommitted = true;
    s.status = s.mayHaveCommitted ? "uncertain" : "blocked";
    s.error = (noWrite && earlierAttemptWasUncertain
      ? `${reason(e)}; an earlier attempt may already have committed, so the exact request remains uncertain`
      : reason(e)).slice(0, 512);
    this.scheduleAutoRetry(s);
    this.settleRecord(s, s.mayHaveCommitted ? "uncertain" : "notWritten");
    this.notify();
  }

  /// Resend a failed Save on its own after the next pause of AUTO_RETRY_MS, so a failure resolves
  /// without the member pressing anything. The resend is `retry`: the same complete request, nonce,
  /// epoch and body, with the lane's order kept (a later edit still waits behind it). Native answers
  /// an operation it already holds as saved, so resending an uncertain request never applies it
  /// twice, and a not-written one never ran. Covered: Create, document and Index Applies, and
  /// ordinary-route frames (their PIX publication is content-addressed). Not covered, and left to
  /// the member as before: restored work (only an explicit Save or retry sends it), the local-draft
  /// and recovery routes (their retry identity is a ticket the member reconciles), and refusals a
  /// resend cannot change (NEEDS_DECISION). Once the pauses are used up the record stays failed
  /// and is shown as the member's to retry or discard.
  private scheduleAutoRetry(s: SaveEntry): void {
    this.cancelAutoRetry(s.id);
    if (!this.autoRetryable(s)) return;
    const round = this.autoRounds.get(s.id) ?? 0;
    if (round >= AUTO_RETRY_MS.length) return;
    this.autoRounds.set(s.id, round + 1);
    const generation = this.generation, ms = AUTO_RETRY_MS[round];
    const cancel = this.schedule(() => {
      if (generation !== this.generation || this.autoTimers.get(s.id)?.cancel !== cancel) return;
      this.autoTimers.delete(s.id);
      this.retry(s.id);
    }, ms);
    this.autoTimers.set(s.id, { at: this.now() + ms, cancel });
  }

  private autoRetryable(s: SaveEntry): boolean {
    if (!this.autosaveMs || this.restoredHeld.has(s.id)) return false;
    if (s.status !== "blocked" && s.status !== "uncertain") return false;
    const route = s.kind === "create" || s.kind === "apply" || s.kind === "applyIndex" || (s.kind === "frame" && s.route === "ordinary");
    return route && !NEEDS_DECISION.some(pattern => pattern.test(s.error));
  }

  /** Drop the pending automatic resend of `id`, keeping its round count. */
  private cancelAutoRetry(id: number): void {
    this.autoTimers.get(id)?.cancel();
    this.autoTimers.delete(id);
  }

  /** The record left the queue (landed, discarded or replaced): forget its resend state. */
  private forgetAutoRetry(id: number): void {
    this.cancelAutoRetry(id);
    this.autoRounds.delete(id);
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

  /// Execute one visit of a preview-local Save while retaining the exact request on the queue.
  /// A successful `pending`/`busy` response is not spun here: the member chooses when to repeat
  /// the same bytes. `busy` proves no write began, while `pending` means a detached job may still
  /// land and is therefore uncertain. A paused actor is first resumed by an explicit ordinary
  /// document access, exactly as the native contract requires, then receives the same request.
  private async runUnconfirmed(
    s: Extract<SaveEntry, { kind: "frame" }> | Extract<SaveEntry, { kind: "overlayApply" }>,
    object: Hex32 | null,
    body: string,
    gen: number,
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
    const result = closing ? await closingOverlaySave(this.ipc, target, request) : await unconfirmedOverlaySave(this.ipc, target, request);
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
    if (result.state === "busy") {
      throw new OverlayNoWrite("local draft Save busy; retry the same request");
    }
    throw new StudioNativeError("local draft Save pending and may still complete; retry the same request exactly");
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
    this.eligible.delete(s.id); this.restoredHeld.delete(s.id); this.busyResends.delete(s.id); this.forgetAutoRetry(s.id);
    const at = this.records.indexOf(s);
    if (at >= 0) this.records.splice(at, 1);
    this.settleRecord(s, "landed");
    this.releaseDrained();
    this.notify();
  }
  /** A departure's Index half goes once the document it left has landed everything in its own
   * lane (see releaseIndexHalfAfterDrain). Only a landing drains a lane for this: a lane emptied by
   * the member's discard drops the entry instead (see discard), so a rename whose document half was
   * thrown away is not then half sent. */
  private releaseDrained(): void {
    for (const [object, generation] of this.releaseAfterDrain) {
      if (generation !== this.generation) { this.releaseAfterDrain.delete(object); continue; }
      if (this.records.some(r => laneOf(r) === object)) continue;
      this.releaseAfterDrain.delete(object);
      this.releaseQueued(true, r => laneOf(r) !== object && affectedObject(r) === object);
    }
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

  private remember(cid: Hex64, bytes: Uint8Array): void {
    if (this.blobs.has(cid)) return;
    this.unavailable.delete(cid);
    this.failed.delete(cid);
    this.invalid.delete(contentKey(cid, bytes.length));
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

/** Two records were authored against the same view (or both without one, in the immediate seam). */
function sameBinding(a: SaveRecord["binding"], b: SaveRecord["binding"]): boolean {
  if (!a || !b) return !a && !b;
  return a.epochId === b.epochId && a.awaiting === b.awaiting && a.phase === b.phase;
}

function utf8Length(text: string): number {
  return new TextEncoder().encode(text).length;
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

/// Native refusals of an Apply made before anything of it is journalled, other than the busy texts
/// above (which are resent; these are not, see NEEDS_DECISION). A lone Apply in a retired epoch is
/// refused by `store/epoch_studio.rs` ahead of its intent barrier (`prepare_epoch_intent_with_io`).
/// A batch is applied item by item in order (`studio.rs`), so when its FIRST item meets that refusal
/// nothing of the batch ran. Both settle known not written, so the member can save the edit again as
/// a new one instead of only discarding it. Matching is exact, like NATIVE_NO_WRITE: any other
/// wording, and a later item of a batch, stays uncertain, which is the safe side.
/// studio-native-no-write.test.ts pins these texts and that ordering to the Rust sources.
export const NATIVE_REFUSED_BEFORE_WRITE: readonly RegExp[] = [
  /^epoch studio: edit belongs to a retired epoch$/,
  /^Studio: batched Apply item 1 of \d+ was refused \(epoch studio: edit belongs to a retired epoch\); the items before it are saved, so retry the same batch exactly or discard that item$/,
];
function nativeRefusedBeforeWrite(e: unknown): boolean {
  const text = reason(e);
  return NATIVE_REFUSED_BEFORE_WRITE.some((pattern) => pattern.test(text));
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
