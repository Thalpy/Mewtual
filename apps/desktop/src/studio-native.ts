// The native Studio contract as the renderer reads it (docs/FLIPNOTE-UI-HOOKS.md). Every result
// that crosses the bridge is checked here into a typed, conflict-preserving read model before any
// surface sees it. Nothing is cast into the fixture roots in studio-store.ts: the native view keeps
// every register's selected value AND its conflicts, every insertion, every tombstone, the overflow
// and deleted Index entries, and the exact decimal/hex identifiers the backend chose.
//
// Two trust states share one envelope. `awaitingTenureReceipt: true` is a read-only history preview
// with no phase and no publication claim; an ordinary view carries `publication: "local"` and a
// stored phase. Only an ordinary view's `epochId` may prepare an edit. `provisional: true` appears
// on both and is never the read-only discriminator.

import type { NativeInvoker } from "./native-download.ts";
import { PIX_MAX_BYTES, SCORE_MAX_STEPS, SCORE_ROWS, SCORE_VOLUME_MAX, SCORE_VOLUME_MIN, layerOf, parseNoteKey, type ScoreCell, type FlipnoteOp } from "./studio-contract.ts";
import { admittedSounds, distinctCells, scoreVoices, type ScoreVoices, type ScoreVoiceSlot } from "./score-voices.ts";
import { PIXA_MAX_BYTES } from "./pixa.ts";
import type { JamPatch } from "./jam-contract.ts";
import { validateJamPatch } from "./jam-patch.ts";
import { studioSoundModel, type StudioSoundModel } from "./studio-sound.ts";

export type Hex32 = string;
export type Hex64 = string;
/// A u64/u128 the backend rendered as a decimal string; never round-trip it through Number.
export type Decimal = string;

export class StudioNativeError extends Error {
  readonly reason: string;
  constructor(reason: string) {
    super(`studio native: ${reason}`);
    this.reason = reason;
  }
}

// --- Bridge ------------------------------------------------------------------------------------

export type StudioUnlisten = () => void;
export type StudioIpc = {
  invoke: NativeInvoker;
  listen: <T>(name: string, handler: (event: { payload: T }) => void) => Promise<StudioUnlisten>;
};

export const STUDIO_UPDATED_EVENT = "studio-updated";
export const STUDIO_RECEIVE_PAUSED_EVENT = "studio-receive-paused";
export const STUDIO_SYNC_STATE_EVENT = "studio-sync-state";
export const SETTLEMENT_CHANGED_EVENT = "settlement-changed";

// --- Read model --------------------------------------------------------------------------------

export type NativeExpiry = { kind: "unrecorded" } | { kind: "never" } | { kind: "at"; ms: number };
export type IndexSource = { opId: Hex64; author: Hex64; nonce: Hex32 };
export type FrameSource = IndexSource & { ts: number };
export type Valued<T, S> = { value: T; source: S };
export type Register<T, S> = { selected: Valued<T, S>; conflicts: Valued<T, S>[] };

export type IndexCreation = { kind: "flipnote" | "score"; title: string; createdBy: Hex64; ts: number; expiry: NativeExpiry };
export type IndexEntry = {
  creations: Valued<IndexCreation, IndexSource>[];
  title: Register<string, IndexSource>;
  expiry: Register<NativeExpiry, IndexSource>;
};
export type IndexContent = {
  kind: "index";
  objects: Record<Hex32, IndexEntry>;
  overflow: Record<Hex32, IndexEntry>;
  deletedObjects: Record<Hex32, IndexEntry>;
  tombstones: Record<Hex32, IndexSource[]>;
};

export type FrameBlob = { cid: Hex64; bytes: number };
export type FrameInsertion = { checkpoint: boolean; after: Hex32 | null; anchor: Hex64 | null; before: Hex64 | null; blob: FrameBlob };
export type FrameEntry = { pixels: Register<FrameBlob, FrameSource>; insertions: Valued<FrameInsertion, FrameSource>[] };
export type FrameLimits = { count: boolean; bytes: boolean };
export type FrameSoundOp = Exclude<FlipnoteOp, { op: "insert_frame" | "remove_frame" | "replace_frame" | "set_header" }>
  | { op: "set_header"; field: "score"; value: Hex32 | null };
export type FrameSoundReadiness = {
  sfxOverflow: Hex32[]; patchOverflow: Hex64[]; exportOverflow: Hex32[];
  missingFrames: Hex32[]; missingPatches: Hex32[]; conflicts: boolean;
};
export type FrameSoundContent = { records: Record<string, Register<FrameSoundOp, FrameSource>>; readiness: FrameSoundReadiness };
export type FlipnoteContent = {
  kind: "flipnote";
  title: Register<string, FrameSource> | null;
  fps: Register<number, FrameSource> | null;
  timeline: Hex32[];
  frames: Record<Hex32, FrameEntry>;
  declaredFrameBytes: number;
  overCap: Record<Hex32, FrameLimits>;
  tombstones: Record<Hex32, FrameSource[]>;
  /** Omitted on older art-only replies; present sound evidence is never cast away. */
  sound?: FrameSoundContent;
};
/** Raw Automerge winner and effective remove-wins state are deliberately separate evidence. */
export type ScoreSlot<T> = { removed: boolean; register: Register<T | null, IndexSource> };
/** A layered note slot (`n/S:R/L`, docs/design-score-layers.md): add wins, so it is `cleared` only
 * when no alternative holds a value; every distinct value it holds plays. */
export type ScoreLayerSlot = { cleared: boolean; register: Register<ScoreCell | null, IndexSource> };
export type ScoreCellIssues = { count: boolean; extent: boolean; recipe: boolean; overlap: boolean; conflict: boolean };
export type ScorePatchIssues = { count: boolean; conflict: boolean };
/** `layers` is present only when the score has layered notes (native omits it otherwise). */
export type ScoreReadiness = { cells: Record<string, ScoreCellIssues>; patches: Record<Hex64, ScorePatchIssues>; headerConflicts: boolean; layers?: Record<string, ScoreCellIssues> };
export type ScoreContent = {
  kind: "score";
  title: Register<string, IndexSource> | null;
  bpm: Register<number, IndexSource> | null;
  bpb: Register<number, IndexSource> | null;
  steps: Register<number, IndexSource> | null;
  cells: Record<string, ScoreSlot<ScoreCell>>;
  /** Keyed `"S:R/L"`; present only when the score has layered notes. */
  layers?: Record<string, ScoreLayerSlot>;
  patches: Record<Hex64, ScoreSlot<JamPatch>>;
  readiness: ScoreReadiness;
};
export type StudioObjectKind = "flipnote" | "score";
export type StudioContent = IndexContent | FlipnoteContent | ScoreContent;

export type StudioPhase = "open" | "closing" | "settled" | "fault";
export type StudioTrust =
  | { awaitingTenureReceipt: true; phase: null; publication: null }
  | { awaitingTenureReceipt: false; phase: StudioPhase; publication: "local" };
export type StudioView<C extends StudioContent = StudioContent> = {
  v: 1;
  epochId: Hex32;
  epoch: Decimal;
  channel: Decimal;
  /** Stable native projection group id, never a process-local server handle. Older replies omit it. */
  group?: string;
  provisional: true;
  content: C;
} & StudioTrust;
export type IndexView = StudioView<IndexContent>;
export type FlipnoteView = StudioView<FlipnoteContent>;
export type ScoreView = StudioView<ScoreContent>;
export type StudioObjectView = FlipnoteView | ScoreView;

export type PixPublication = { cid: Hex64; bytes: number };

// --- Recovery ------------------------------------------------------------------------------------

export type RecoveryReason = "excluded" | "rewound" | "conflictOverflow" | "repair";
export type RecoveryVersion = { snapshot: Hex64; epoch: Decimal; staged: boolean; bytes: number; reason: RecoveryReason };
export type RecoveryEviction = { oldestSnapshot: Hex64; stagedSnapshot: Hex64; deadlineMs: Decimal };
export type RecoveryListing = {
  v: 1;
  kind: "recoveryList" | "recoveryAcknowledged";
  channel: Decimal;
  object: Hex32 | null;
  source: { epochId: Hex32; epoch: Decimal; phase: StudioPhase; provisional: true } | null;
  versions: RecoveryVersion[];
  evictionPending: RecoveryEviction | null;
  pendingIntents: number;
};
export type RecoveryVersionRead = { v: 1; kind: "recoveryVersion"; historical: true; version: RecoveryVersion; channel: Decimal; group?: string; content: StudioContent };
export type RecoveryExport = { v: 1; kind: "recoveryExport"; snapshot: Hex64; format: "p1-recovery-v1"; bytes: number; bytesB64: string };
export type RecoveryMode = "restore" | "copy";
export type RecoveryChoice =
  | { kind: "soundRecord"; value: Hex64 }
  | { kind: "scoreCell"; step: number; row: number; value: Hex64 }
  /** A layered note slot (docs/design-score-layers.md section 6): `layer` is "d" or the sound id;
   * native restores it with `set_note` (or `clear_note` for a chosen clear). */
  | { kind: "scoreNote"; step: number; row: number; layer: string; value: Hex64 }
  | { kind: "scorePatch"; patch: Hex64; value: Hex64 }
  | { kind: "scoreHeader"; field: "title" | "bpm" | "bpb" | "steps"; value: Hex64 }
  | { kind: "frame"; id: Hex32; value: Hex64 }
  | { kind: "frameDeletion"; id: Hex32 }
  | { kind: "title"; value: Hex64 }
  | { kind: "fps"; value: Hex64 }
  | { kind: "object"; id: Hex32 }
  | { kind: "objectTitle"; id: Hex32; value: Hex64 }
  | { kind: "objectExpiry"; id: Hex32; value: Hex64 }
  | { kind: "objectDeletion"; id: Hex32 };
export type RecoveryDisposition = "ready" | "unchanged" | "conflict" | "deleted" | "full" | "missingTarget";
export type RecoveryPreview = {
  v: 1;
  kind: "recoveryPreview";
  snapshot: Hex64;
  epochId: Hex32;
  expectedProjection: Hex64;
  disposition: RecoveryDisposition;
  body: string | null;
  originalAuthor: Hex64 | null;
};
/// Echo of a Ready preview plus a fresh nonce: the exact payload a retry resends.
export type RecoveryApplyEdit = {
  snapshot: Hex64;
  choice: RecoveryChoice;
  mode: RecoveryMode;
  epochId: Hex32;
  expectedProjection: Hex64;
  nonce: Hex32;
  body: string;
};
export type RecoveryApplied = { v: 1; kind: "recoveryApplied"; contentSaved: true; alreadySaved: boolean; provisional: true; pointerRestored: false };
export type PointerRestored = { v: 1; kind: "recoveryPointerRestored"; channel: Decimal; object: Hex32 | null; checkpointEpoch: Decimal; registryEpochId: Hex32; provisional: true };

// --- Retained local drafts ----------------------------------------------------------------------

export type OverlayManualReason =
  | "sourceMissing" | "sourceUnreadable" | "sourceNotClosing" | "sourceReplaced"
  | "sourceRewound" | "successorNotPristine" | "successorMissing" | "receiptChanged"
  | "objectMissing" | "tenureUnknown" | "tenureImported" | "preparedStuck" | "fault"
  | "notCurrentAuthor" | "unconfirmed" | "notReplayable";
export type OverlaySaveRefusal = OverlayManualReason
  | "notMember" | "sourceAdopting" | "installedSource" | "previewUnavailable"
  | "previewExpired" | "tailIncomplete" | "previewReplaced" | "membershipChanged"
  | "basisChanged" | "basisChangedDuringDetach" | "transferInProgress" | "otherKindDraft"
  | "liveBranchSuperseded" | "staleBranch" | "nonceConflict" | "ordinaryIntentPending"
  | "branchFull" | "pixelUnpublished" | "pixelInvalid" | "pixelMissing"
  | "exportUnpublished" | "exportInvalid" | "exportMissing"
  | "unconfirmedBranchesPerServer" | "unconfirmedBytesVaultWide" | "referenceCapacity"
  | "storageRefused" | "retry";
export type OverlaySaveRetry = "sameRequest" | "newTicket" | "none" | "afterExplicitAccess";
export type OverlayUnconfirmedState = "awaitingSource" | "baseConfirmed" | "baseSuperseded" | "sourceUnreadable";
export type OverlayHandoffAttempt = {
  state: "refused" | "backingOff";
  reason: OverlayManualReason | "transient";
  retryAt: Decimal;
};
export type OverlayProvenance =
  | { kind: "closing" }
  | { kind: "unconfirmed"; provider: Hex64; observedMlsEpoch: Decimal; observedAtMs: Decimal };
export type OverlayTarget = { channel: Decimal; object: Hex32 | null; objectKind?: StudioObjectKind };
export type OverlayInspection =
  | ({ v: 1; kind: "absent" } & OverlayTarget)
  | ({ v: 1; kind: "local-draft"; eligibility: "transferable" | "manual" | null;
      manualReason: OverlayManualReason | null; handoffAttempt: OverlayHandoffAttempt | null;
      unconfirmedState: OverlayUnconfirmedState | null; basis: Hex64 | null; branch: Hex64;
      contentId: Hex64 | null; generation: Decimal; accepted: number;
      transferState: "active" | "prepared"; provenance: "closing" | "unconfirmed" | null;
      replayable: boolean; readOnly: true; content: StudioContent | null;
      /** Actual rebuilt draft projection identity; old native versions omit these and cannot export a draft. */
      contentEpoch?: Decimal; group?: string } & OverlayTarget)
  | ({ v: 1; kind: "disposed"; basis: Hex64; branch: Hex64; generation: Decimal;
      accepted: number; mode: "preserved" | "discarded"; archive: Hex64 | null;
      readOnly: true } & OverlayTarget);
export type OverlayLifecycle = {
  v: 1;
  kind: "overlayLifecycle";
  channel: Decimal;
  object: Hex32 | null;
  objectKind?: StudioObjectKind;
  branch: { branch: Hex64; content: Hex64; generation: Decimal; accepted: number } | null;
  prepared: boolean;
  eligibility: "transferable" | "manual" | null;
  manualReason: OverlayManualReason | null;
  handoffAttempt: OverlayHandoffAttempt | null;
  unconfirmedState: OverlayUnconfirmedState | null;
  archive: { archive: Hex64; branch: Hex64; generation: Decimal; replayable: boolean } | null;
  disposed: ({ branch: Hex64; generation: Decimal } & ({ mode: "preserved"; archive: Hex64 } | { mode: "discarded" })) | null;
  transferred: boolean;
};
export type OverlayArchive = {
  v: 1;
  kind: "overlayArchive" | "overlayArchived" | "overlayArchiveExport" | "overlayExport";
  archive: Hex64;
  basis: Hex64;
  content: Hex64;
  branch: Hex64;
  generation: Decimal;
  author: Hex64;
  accepted: number;
  provenance: OverlayProvenance;
  replayable: boolean;
  readOnly: true;
  authority: false;
  terminal: true;
  physicalBytes?: Decimal;
  preserved?: boolean;
  notReplayable?: string | null;
  format?: "catcoms-studio-draft-v1";
  bytes?: number;
  bytesB64?: string;
};
export type OverlayArchiveReleased = { v: 1; kind: "overlayArchiveReleased"; reconcileRequired: true };
export type OverlayDisposed = {
  v: 1; kind: "overlayDisposed"; channel: Decimal; object: Hex32 | null; objectKind?: StudioObjectKind; author: Hex64;
  provenance: OverlayProvenance; basis: Hex64; branch: Hex64; content: Hex64;
  generation: Decimal; disposal: { mode: "preserved"; archive: Hex64 } | { mode: "discarded" };
  accepted: number; sequence: Decimal; atMs: Decimal; terminal: true;
};
export type OverlayCopyPreview = {
  v: 1; kind: "overlayCopyPreview"; source: OverlayTarget; destination: OverlayTarget;
  epochId: Hex32; expectedProjection: Hex64; disposition: RecoveryDisposition;
  body: string | null; originalAuthor: Hex64 | null; sourceOps: Hex64[];
  preservesBranch: false; provisional: true;
};
export type OverlayCopyApplied = {
  v: 1; kind: "overlayCopyApplied"; channel: Decimal; object: Hex32 | null; objectKind?: StudioObjectKind;
  contentSaved: true; alreadySaved: boolean; provisional: true; branchPreserved: false;
};
export type OverlayCopyRefused = {
  v: 1; kind: "overlayCopyRefused"; channel: Decimal; object: Hex32 | null; objectKind?: StudioObjectKind;
  reason: "stalePreview"; retry: "repreview"; written: false;
};
export type OverlayCopyApplyResult = OverlayCopyApplied | OverlayCopyRefused;
export type OverlaySaveProvenance = "unconfirmed" | "closing";
export type OverlaySaveTicket<P extends OverlaySaveProvenance> = {
  v: 1; kind: `${P}OverlaySaveTicket`; channel: Decimal; object: Hex32 | null; objectKind?: StudioObjectKind;
  basis: Hex64; branch: Hex64; provenance: P;
};
export type OverlaySaveResult<P extends OverlaySaveProvenance> = ({
  v: 1; kind: `${P}OverlaySave`; channel: Decimal; object: Hex32 | null; objectKind?: StudioObjectKind;
} & (
  | { state: "saved"; basis: Hex64; accepted: number; localOnly: true; provisional: true; provenance: P }
  | { state: "pending" | "busy"; retry: "sameRequest" }
  | { state: "paused"; retry: "afterExplicitAccess" }
  | { state: "refused"; reason: OverlaySaveRefusal; retry: "sameRequest" | "newTicket" | "none" }
  | { state: "disposed"; disposal: OverlayDisposed }
  | { state: "handedOff"; basis: Hex64; epoch: Decimal; docId: Hex32; accepted: number }
));
export type UnconfirmedOverlaySaveTicket = OverlaySaveTicket<"unconfirmed">;
export type UnconfirmedOverlaySave = OverlaySaveResult<"unconfirmed">;
export type ClosingOverlaySaveTicket = OverlaySaveTicket<"closing">;
export type ClosingOverlaySave = OverlaySaveResult<"closing">;
export type ClosingOverlaySaveBegin = ClosingOverlaySaveTicket | Extract<ClosingOverlaySave, { state: "refused" }>;
/// Begin normally returns a ticket, but a preview that ceased to authorize a local branch is a
/// typed, known-no-write refusal. Keeping that refusal in the public union prevents the renderer
/// from inventing ticket fields or misreporting a clean refusal as delivery uncertainty.
export type UnconfirmedOverlaySaveBegin =
  | UnconfirmedOverlaySaveTicket
  | Extract<UnconfirmedOverlaySave, { state: "refused" }>;

export type OverlayDestination = { channel: Decimal; object?: Hex32; objectKind?: StudioObjectKind };
export type OverlayCopyApplyEdit = {
  destination: OverlayDestination; choice: RecoveryChoice; mode: RecoveryMode; epochId: Hex32;
  expectedProjection: Hex64; nonce: Hex32; body: string;
};
export type OverlayDisposalRequest = {
  branch: Hex64; content: Hex64; accepted: number;
  mode: { kind: "preserve" } | { kind: "discard"; confirmation: "destroy-local-draft" };
};
export type UnconfirmedOverlaySaveInput = { basis: Hex64; branch: Hex64; nonce: Hex32; body: string };

// --- Events --------------------------------------------------------------------------------------

export type StudioUpdatedEvent = { server: number; channel: Decimal; object: Hex32 | null; objectKind?: StudioObjectKind };
export type StudioReceivePausedEvent = { server: number };
export type StudioSyncStateEvent = { server: number; instance: Decimal; sequence: Decimal; paused: boolean; cause: "background" | "incoming" | null; detail: string | null };
export type SettlementState = "open" | "closing" | "settled" | "fault" | "recoveryAvailable" | "recoveryEvictionPending" | "refreshRequired" | "repairing" | "storageRefused";
export type SettlementChangedEvent = { server: number; docType: 15 | 16; logicalKey: Hex32; channel: Decimal; object: Hex32 | null; objectKind?: StudioObjectKind; state: SettlementState };

// --- Checks --------------------------------------------------------------------------------------

const HEX32 = /^[0-9a-f]{32}$/;
const HEX64 = /^[0-9a-f]{64}$/;
const DECIMAL = /^(0|[1-9][0-9]*)$/;

export const isHex32 = (s: unknown): s is Hex32 => typeof s === "string" && HEX32.test(s);
export const isHex64 = (s: unknown): s is Hex64 => typeof s === "string" && HEX64.test(s);
export const isDecimal = (s: unknown): s is Decimal => typeof s === "string" && DECIMAL.test(s);

function fail(what: string): never {
  throw new StudioNativeError(what);
}
function record(v: unknown, what: string): Record<string, unknown> {
  if (!v || typeof v !== "object" || Array.isArray(v)) fail(`${what} is not an object`);
  return v as Record<string, unknown>;
}
function list(v: unknown, what: string): unknown[] {
  if (!Array.isArray(v)) fail(`${what} is not a list`);
  return v;
}
function text(v: unknown, what: string): string {
  if (typeof v !== "string") fail(`${what} is not a string`);
  return v;
}
function hex32(v: unknown, what: string): Hex32 {
  if (!isHex32(v)) fail(`${what} is not 32 lowercase hex`);
  return v;
}
function hex64(v: unknown, what: string): Hex64 {
  if (!isHex64(v)) fail(`${what} is not 64 lowercase hex`);
  return v;
}
function decimal(v: unknown, what: string): Decimal {
  if (!isDecimal(v)) fail(`${what} is not a canonical decimal string`);
  return v;
}
function count(v: unknown, what: string): number {
  if (typeof v !== "number" || !Number.isSafeInteger(v) || v < 0) fail(`${what} is not a safe non-negative integer`);
  return v;
}
function flag(v: unknown, what: string): boolean {
  if (typeof v !== "boolean") fail(`${what} is not a boolean`);
  return v;
}
function nullable<T>(v: unknown, f: (v: unknown) => T): T | null {
  return v === null || v === undefined ? null : f(v);
}
function mapOf<T>(v: unknown, what: string, key: (k: string) => string, f: (v: unknown, k: string) => T): Record<string, T> {
  const out: Record<string, T> = {};
  for (const [k, item] of Object.entries(record(v, what))) out[key(k)] = f(item, `${what}.${k}`);
  return out;
}

export function parseExpiry(v: unknown, what = "expiry"): NativeExpiry {
  const r = record(v, what);
  switch (r.kind) {
    case "unrecorded": return { kind: "unrecorded" };
    case "never": return { kind: "never" };
    case "at": return { kind: "at", ms: count(r.ms, `${what}.ms`) };
    default: return fail(`${what}.kind is unknown`);
  }
}
function indexSource(v: unknown, what: string): IndexSource {
  const r = record(v, what);
  return { opId: hex64(r.opId, `${what}.opId`), author: hex64(r.author, `${what}.author`), nonce: hex32(r.nonce, `${what}.nonce`) };
}
function frameSource(v: unknown, what: string): FrameSource {
  const r = record(v, what);
  return { ...indexSource(v, what), ts: count(r.ts, `${what}.ts`) };
}
function register<T, S>(v: unknown, what: string, value: (v: unknown, w: string) => T, source: (v: unknown, w: string) => S): Register<T, S> {
  const r = record(v, what);
  const valued = (x: unknown, w: string): Valued<T, S> => {
    const e = record(x, w);
    return { value: value(e.value, `${w}.value`), source: source(e.source, `${w}.source`) };
  };
  return {
    selected: valued(r.selected, `${what}.selected`),
    conflicts: list(r.conflicts, `${what}.conflicts`).map((c, i) => valued(c, `${what}.conflicts[${i}]`)),
  };
}
const asHex32Key = (k: string) => hex32(k, "object id");

function indexEntry(v: unknown, what: string): IndexEntry {
  const r = record(v, what);
  return {
    creations: list(r.creations, `${what}.creations`).map((c, i) => {
      const e = record(c, `${what}.creations[${i}]`);
      const val = record(e.value, `${what}.creations[${i}].value`);
      const kind = val.kind === "flipnote" || val.kind === "score" ? val.kind : fail(`${what}.creations[${i}].value.kind is unknown`);
      return {
        source: indexSource(e.source, `${what}.creations[${i}].source`),
        value: {
          kind,
          title: text(val.title, `${what}.creations[${i}].value.title`),
          createdBy: hex64(val.createdBy, `${what}.creations[${i}].value.createdBy`),
          ts: count(val.ts, `${what}.creations[${i}].value.ts`),
          expiry: parseExpiry(val.expiry, `${what}.creations[${i}].value.expiry`),
        },
      };
    }),
    title: register(r.title, `${what}.title`, text, indexSource),
    expiry: register(r.expiry, `${what}.expiry`, parseExpiry, indexSource),
  };
}

export function parseIndexContent(v: unknown, what = "content"): IndexContent {
  const r = record(v, what);
  if (r.kind !== "index") fail(`${what}.kind is not index`);
  return {
    kind: "index",
    objects: mapOf(r.objects, `${what}.objects`, asHex32Key, indexEntry),
    overflow: mapOf(r.overflow, `${what}.overflow`, asHex32Key, indexEntry),
    deletedObjects: mapOf(r.deletedObjects, `${what}.deletedObjects`, asHex32Key, indexEntry),
    tombstones: mapOf(r.tombstones, `${what}.tombstones`, asHex32Key, (t, w) => list(t, w).map((s, i) => indexSource(s, `${w}[${i}]`))),
  };
}

function blob(v: unknown, what: string): FrameBlob {
  const r = record(v, what);
  return { cid: hex64(r.cid, `${what}.cid`), bytes: count(r.bytes, `${what}.bytes`) };
}
function frameEntry(v: unknown, what: string): FrameEntry {
  const r = record(v, what);
  return {
    pixels: register(r.pixels, `${what}.pixels`, blob, frameSource),
    insertions: list(r.insertions, `${what}.insertions`).map((x, i) => {
      const e = record(x, `${what}.insertions[${i}]`);
      const val = record(e.value, `${what}.insertions[${i}].value`);
      const w = `${what}.insertions[${i}].value`;
      return {
        source: frameSource(e.source, `${what}.insertions[${i}].source`),
        value: {
          checkpoint: flag(val.checkpoint, `${w}.checkpoint`),
          after: nullable(val.after, (a) => hex32(a, `${w}.after`)),
          anchor: nullable(val.anchor, (a) => hex64(a, `${w}.anchor`)),
          before: nullable(val.before, (a) => hex64(a, `${w}.before`)),
          blob: blob(val.blob, `${w}.blob`),
        },
      };
    }),
  };
}

export function parseFlipnoteContent(v: unknown, what = "content"): FlipnoteContent {
  const r = record(v, what);
  if (r.kind !== "flipnote") fail(`${what}.kind is not flipnote`);
  const fps = (x: unknown, w: string) => {
    const n = count(x, w);
    if (n < 1 || n > 24) fail(`${w} is outside 1..24`);
    return n;
  };
  return {
    kind: "flipnote",
    title: nullable(r.title, (t) => register(t, `${what}.title`, text, frameSource)),
    fps: nullable(r.fps, (t) => register(t, `${what}.fps`, fps, frameSource)),
    timeline: list(r.timeline, `${what}.timeline`).map((id, i) => hex32(id, `${what}.timeline[${i}]`)),
    frames: mapOf(r.frames, `${what}.frames`, asHex32Key, frameEntry),
    declaredFrameBytes: count(r.declaredFrameBytes, `${what}.declaredFrameBytes`),
    overCap: mapOf(r.overCap, `${what}.overCap`, asHex32Key, (x, w) => {
      const e = record(x, w);
      return { count: flag(e.count, `${w}.count`), bytes: flag(e.bytes, `${w}.bytes`) };
    }),
    tombstones: mapOf(r.tombstones, `${what}.tombstones`, asHex32Key, (t, w) => list(t, w).map((s, i) => frameSource(s, `${w}[${i}]`))),
    ...(r.sound === undefined ? {} : { sound: parseFrameSoundContent(r.sound, `${what}.sound`) }),
  };
}

/** Validate the full sound operation and its physical-key/source binding, including losing values. */
function soundOperation(value: unknown, what: string): FrameSoundOp {
  const r = closedDataRecord(value, what);
  const exact = (keys: string[]) => { if (Object.keys(r).sort().join(",") !== keys.sort().join(",")) fail(`${what} has unknown/missing operation fields`); };
  switch (r.op) {
    case "set_sfx": {
      exact(["op", "sfx", "frame", "patch", "note", "duration_ms"]);
      const note = count(r.note, `${what}.note`), duration_ms = count(r.duration_ms, `${what}.duration_ms`);
      if (note > 127 || duration_ms < 1 || duration_ms > 600_000) fail(`${what} sound note/duration is outside its bound`);
      return { op: "set_sfx", sfx: hex32(r.sfx, `${what}.sfx`), frame: hex32(r.frame, `${what}.frame`), patch: hex64(r.patch, `${what}.patch`), note, duration_ms };
    }
    case "remove_sfx": exact(["op", "sfx"]); return { op: "remove_sfx", sfx: hex32(r.sfx, `${what}.sfx`) };
    case "set_patch": {
      exact(["op", "patch", "descriptor"]);
      const descriptor = validateJamPatch(r.descriptor);
      if (!descriptor.ok) fail(`${what}.descriptor: ${descriptor.error}`);
      return { op: "set_patch", patch: hex64(r.patch, `${what}.patch`), descriptor: descriptor.patch };
    }
    case "remove_patch": exact(["op", "patch"]); return { op: "remove_patch", patch: hex64(r.patch, `${what}.patch`) };
    case "set_export": {
      exact(["op", "export", "cid", "bytes", ...("expiry" in r ? ["expiry"] : [])]);
      const bytes = count(r.bytes, `${what}.bytes`);
      if (bytes < 1 || bytes > PIXA_MAX_BYTES) fail(`${what}.bytes is outside the pixa bound`);
      return { op: "set_export", export: hex32(r.export, `${what}.export`), cid: hex64(r.cid, `${what}.cid`), bytes,
        ...("expiry" in r ? { expiry: r.expiry === null ? null : count(r.expiry, `${what}.expiry`) } : {}) };
    }
    case "remove_export": exact(["op", "export"]); return { op: "remove_export", export: hex32(r.export, `${what}.export`) };
    case "set_header":
      exact(["op", "field", "value"]);
      if (r.field !== "score") fail(`${what} is not a sound header`);
      return { op: "set_header", field: "score", value: r.value === null ? null : hex32(r.value, `${what}.value`) };
    default: return fail(`${what} is not a known sound operation`);
  }
}

function soundOperationKey(op: FrameSoundOp, source: FrameSource): string {
  switch (op.op) {
    case "set_sfx": return `s/${op.sfx}`;
    case "remove_sfx": return `q/${op.sfx}/${source.opId}`;
    case "set_patch": case "remove_patch": return `p/${op.patch}`;
    case "set_export": return `x/${op.export}/${source.opId}`;
    case "remove_export": return `z/${op.export}/${source.opId}`;
    case "set_header": return "h/score";
  }
}

export function parseFrameSoundContent(value: unknown, what = "sound"): FrameSoundContent {
  const r = closedDataRecord(value, what, ["records", "readiness"]), rawRecords = closedDataRecord(r.records, `${what}.records`);
  let remaining = 3 * 20_000 + 8192;
  if (Object.keys(rawRecords).length > remaining) fail(`${what} sound records exceed their bound`);
  const records = mapOf(rawRecords, `${what}.records`, (key) => {
    if (key !== "h/score" && !/^(?:s\/[0-9a-f]{32}|p\/[0-9a-f]{64}|[qxz]\/[0-9a-f]{32}\/[0-9a-f]{64})$/.test(key)) fail("sound physical key is malformed");
    return key;
  }, (raw, w) => {
    const recordValue = closedDataRecord(raw, w, ["selected", "conflicts"]), conflicts = closedDataList(recordValue.conflicts, `${w}.conflicts`);
    if (conflicts.length + 1 > remaining) fail(`${what} sound values exceed their bound`);
    remaining -= conflicts.length + 1;
    closedDataRecord(recordValue.selected, `${w}.selected`, ["value", "source"]);
    for (const [index, conflict] of conflicts.entries()) closedDataRecord(conflict, `${w}.conflicts[${index}]`, ["value", "source"]);
    const result = register(recordValue, w, soundOperation, (source, name) => frameSource(closedDataRecord(source, name, ["opId", "author", "nonce", "ts"]), name));
    const key = w.slice(`${what}.records.`.length), seen = new Set<string>();
    for (const alternative of [result.selected, ...result.conflicts]) {
      if (soundOperationKey(alternative.value, alternative.source) !== key) fail(`${w} operation/key/source disagree`);
      if (seen.has(alternative.source.opId)) fail(`${w} repeats a sound source operation`);
      seen.add(alternative.source.opId);
    }
    return result;
  });
  const readiness = closedDataRecord(r.readiness, `${what}.readiness`, ["sfxOverflow", "patchOverflow", "exportOverflow", "missingFrames", "missingPatches", "conflicts"]);
  const sortedIds = (input: unknown, w: string, parse: (v: unknown, w: string) => string) => {
    const values = closedDataList(input, w);
    if (values.length > 68_192) fail(`${w} exceeds its bound`);
    const result = values.map((v, i) => parse(v, `${w}[${i}]`));
    if (result.some((v, i) => i > 0 && v <= result[i - 1])) fail(`${w} is not a sorted unique list`);
    return result;
  };
  return { records, readiness: {
    sfxOverflow: sortedIds(readiness.sfxOverflow, `${what}.readiness.sfxOverflow`, hex32),
    patchOverflow: sortedIds(readiness.patchOverflow, `${what}.readiness.patchOverflow`, hex64),
    exportOverflow: sortedIds(readiness.exportOverflow, `${what}.readiness.exportOverflow`, hex32),
    missingFrames: sortedIds(readiness.missingFrames, `${what}.readiness.missingFrames`, hex32),
    missingPatches: sortedIds(readiness.missingPatches, `${what}.readiness.missingPatches`, hex32),
    conflicts: flag(readiness.conflicts, `${what}.readiness.conflicts`),
  } };
}

/** Canonical global address, independent of the current (possibly concurrently shrunk) extent. */
function scoreAddress(value: string): { step: number; row: number } {
  const match = /^(0|[1-9][0-9]*):(0|[1-9][0-9]*)$/.exec(value);
  if (!match) fail("score cell address is not canonical");
  const step = Number(match[1]), row = Number(match[2]);
  if (step >= SCORE_MAX_STEPS || row >= SCORE_ROWS) fail("score cell address is outside its global bound");
  return { step, row };
}

/** New Score DTOs are closed data records; future automation/authority fields must not vanish. */
function closedDataRecord(value: unknown, what: string, keys?: readonly string[]): Record<string, unknown> {
  const r = record(value, what), prototype = Object.getPrototypeOf(r), own = Reflect.ownKeys(r);
  if (prototype !== Object.prototype && prototype !== null) fail(`${what} is not plain data`);
  if (own.some((key) => typeof key !== "string" || !Object.getOwnPropertyDescriptor(r, key)?.enumerable ||
    !("value" in Object.getOwnPropertyDescriptor(r, key)!))) fail(`${what} has hidden/accessor fields`);
  if (keys && (own.length !== keys.length || own.some((key) => !keys.includes(key as string)))) fail(`${what} has unknown/missing fields`);
  return r;
}

function closedDataList(value: unknown, what: string): unknown[] {
  const values = list(value, what);
  if (values.length > 68_192 || Object.getPrototypeOf(values) !== Array.prototype) fail(`${what} is not a bounded plain list`);
  if (Reflect.ownKeys(values).length !== values.length + 1) fail(`${what} is not a dense data list`);
  for (let index = 0; index < values.length; index++) {
    const descriptor = Object.getOwnPropertyDescriptor(values, String(index));
    if (!descriptor || !descriptor.enumerable || !("value" in descriptor)) fail(`${what} is not a dense data list`);
  }
  return values;
}

function scoreCell(v: unknown, what: string, step: number): ScoreCell {
  const r = closedDataRecord(v, what), keys = Object.keys(r).sort().join(",");
  if (r.k === "n") {
    if (keys !== "k,p" && keys !== "k,len,p" && keys !== "k,p,vol" && keys !== "k,len,p,vol") fail(`${what} has unknown note fields`);
    const p = hex64(r.p, `${what}.p`);
    const cell: ScoreCell = { k: "n", p };
    if ("len" in r) {
      const len = count(r.len, `${what}.len`);
      if (len < 1 || len > SCORE_MAX_STEPS - step) fail(`${what}.len is outside its global extent`);
      cell.len = len;
    }
    if ("vol" in r) {
      const vol = count(r.vol, `${what}.vol`);
      if (vol < SCORE_VOLUME_MIN || vol > SCORE_VOLUME_MAX) fail(`${what}.vol is outside ${SCORE_VOLUME_MIN}..${SCORE_VOLUME_MAX}`);
      cell.vol = vol;
    }
    return cell;
  }
  if (r.k === "d" && keys === "k,pad") {
    const pad = count(r.pad, `${what}.pad`);
    if (pad > 9) fail(`${what}.pad is outside 0..9`);
    return { k: "d", pad };
  }
  return fail(`${what} is not a known score cell`);
}

/** Preserve all values, while checking the native effective flag against the removal evidence. */
export function parseScoreContent(v: unknown, what = "content"): ScoreContent {
  // `layers` appears only for a score with layered notes, so a score without them is the exact DTO
  // older builds received (docs/design-score-layers.md section 4).
  const hasLayers = typeof v === "object" && v !== null && Object.hasOwn(v, "layers");
  const r = closedDataRecord(v, what, ["kind", "title", "bpm", "bpb", "steps", "cells", ...(hasLayers ? ["layers"] : []), "patches", "readiness"]);
  if (r.kind !== "score") fail(`${what}.kind is not score`);
  // Matches the typed snapshot's value rail; malformed native replies cannot allocate arbitrary
  // conflict vectors or let a tiny displayed winner hide an unbounded evidence collection.
  let remaining = 3 * 20_000 + 8192;
  const checkedRegister = <T>(value: unknown, w: string, parse: (v: unknown, w: string) => T): Register<T, IndexSource> => {
    const raw = closedDataRecord(value, w, ["selected", "conflicts"]), conflicts = closedDataList(raw.conflicts, `${w}.conflicts`);
    if (conflicts.length + 1 > remaining) fail("score register evidence exceeds its bound");
    remaining -= conflicts.length + 1;
    closedDataRecord(raw.selected, `${w}.selected`, ["value", "source"]);
    for (let i = 0; i < conflicts.length; i++) {
      const descriptor = Object.getOwnPropertyDescriptor(conflicts, String(i));
      if (!descriptor || !("value" in descriptor)) fail(`${w}.conflicts is not a dense data list`);
      closedDataRecord(descriptor.value, `${w}.conflicts[${i}]`, ["value", "source"]);
    }
    const result = register(raw, w, parse, (source, name) => indexSource(closedDataRecord(source, name, ["opId", "author", "nonce"]), name));
    const seen = new Set<string>();
    for (const alternative of [result.selected, ...result.conflicts]) {
      if (seen.has(alternative.source.opId)) fail(`${w} repeats a source operation`);
      seen.add(alternative.source.opId);
    }
    return result;
  };
  const boundedNumber = (min: number, max: number) => (value: unknown, w: string) => {
    const n = count(value, w);
    if (n < min || n > max) fail(`${w} is outside ${min}..${max}`);
    return n;
  };
  const titleValue = (value: unknown, w: string) => {
    const result = text(value, w);
    if (result.length > 65536 || new TextEncoder().encode(result).length > 65536 || /[\uD800-\uDFFF]/u.test(result)) fail(`${w} is not bounded Unicode scalar text`);
    return result;
  };
  const slot = <T>(value: unknown, w: string, parse: (v: unknown, w: string) => T): ScoreSlot<T> => {
    const raw = closedDataRecord(value, w, ["removed", "register"]), removed = flag(raw.removed, `${w}.removed`);
    const result = checkedRegister<T | null>(raw.register, `${w}.register`, (x, name) => x === null ? null : parse(x, name));
    const effectiveRemoved = [result.selected, ...result.conflicts].some((alternative) => alternative.value === null);
    if (removed !== effectiveRemoved) fail(`${w}.removed disagrees with retained removal evidence`);
    return { removed, register: result };
  };
  const rawCells = closedDataRecord(r.cells, `${what}.cells`), rawPatches = closedDataRecord(r.patches, `${what}.patches`);
  if (Object.keys(rawCells).length > SCORE_MAX_STEPS * SCORE_ROWS || Object.keys(rawPatches).length > 20_064) fail("score slot collection exceeds its bound");
  const cells = mapOf(rawCells, `${what}.cells`, (key) => { scoreAddress(key); return key; },
    (value, w) => slot(value, w, (cell, name) => scoreCell(cell, name, scoreAddress(w.slice(w.lastIndexOf(".") + 1)).step)));
  const patches = mapOf(rawPatches, `${what}.patches`, (key) => hex64(key, "score patch id"), (value, w) => slot(value, w, (patch, name) => {
    const result = validateJamPatch(patch);
    if (!result.ok) fail(`${name}: ${result.error}`);
    return result.patch;
  }));
  // Layered notes: add wins, so a slot is `cleared` exactly when none of its alternatives holds a
  // value, and every value it holds must belong to the slot's own layer (the key's sound, or `d`).
  let layers: Record<string, ScoreLayerSlot> | undefined;
  if (hasLayers) {
    const rawLayers = closedDataRecord(r.layers, `${what}.layers`);
    const count = Object.keys(rawLayers).length;
    if (count === 0) fail(`${what}.layers is present but empty`);
    if (count > 68_192) fail("score layer collection exceeds its bound");
    layers = mapOf(rawLayers, `${what}.layers`, (key) => {
      const parsed = parseNoteKey(key);
      if (!parsed || parsed.layer === null) fail("score layer address is not canonical");
      return key;
    }, (value, w) => {
      const key = w.slice(w.lastIndexOf(".") + 1), { step, layer } = parseNoteKey(key)!;
      const raw = closedDataRecord(value, w, ["cleared", "register"]), cleared = flag(raw.cleared, `${w}.cleared`);
      const result = checkedRegister<ScoreCell | null>(raw.register, `${w}.register`, (x, name) => x === null ? null : scoreCell(x, name, step));
      const values = [result.selected, ...result.conflicts].map((alternative) => alternative.value);
      if (values.some((cell) => cell !== null && layerOf(cell) !== layer)) fail(`${w} holds a note of another layer`);
      if (cleared !== values.every((cell) => cell === null)) fail(`${w}.cleared disagrees with the values it holds`);
      return { cleared, register: result };
    });
  }
  const ready = closedDataRecord(r.readiness, `${what}.readiness`, ["cells", "patches", "headerConflicts", ...(hasLayers ? ["layers"] : [])]);
  const issueMap = <T>(input: unknown, w: string, keys: readonly string[], present: Record<string, unknown>, key: (v: string) => string): Record<string, T> =>
    mapOf(closedDataRecord(input, w), w, key, (value, name) => {
      const address = name.slice(name.lastIndexOf(".") + 1);
      if (!(address in present)) fail(`${name} names an absent slot`);
      const raw = closedDataRecord(value, name, keys), result: Record<string, boolean> = {};
      if (Object.keys(raw).sort().join(",") !== [...keys].sort().join(",")) fail(`${name} has unknown readiness flags`);
      for (const field of keys) result[field] = flag(raw[field], `${name}.${field}`);
      return result as T;
    });
  const title = nullable(r.title, (x) => checkedRegister(x, `${what}.title`, titleValue));
  const bpm = nullable(r.bpm, (x) => checkedRegister(x, `${what}.bpm`, boundedNumber(40, 240)));
  const bpb = nullable(r.bpb, (x) => checkedRegister(x, `${what}.bpb`, boundedNumber(1, 8)));
  const steps = nullable(r.steps, (x) => checkedRegister(x, `${what}.steps`, boundedNumber(1, SCORE_MAX_STEPS)));
  const headerConflicts = flag(ready.headerConflicts, `${what}.readiness.headerConflicts`);
  // A header conflict is more than one DISTINCT value (identical concurrent values are not one).
  const distinctHeader = (header: Register<unknown, IndexSource> | null) =>
    !!header && new Set([header.selected, ...header.conflicts].map((alternative) => JSON.stringify(alternative.value))).size > 1;
  if (headerConflicts !== [title, bpm, bpb, steps].some(distinctHeader)) fail("score header readiness disagrees with retained conflicts");
  const issueKeys = ["count", "extent", "recipe", "overlap", "conflict"] as const;
  const layerIssues = layers ? issueMap<ScoreCellIssues>(ready.layers, `${what}.readiness.layers`, issueKeys, layers, (key) => key) : undefined;
  // The composer refuses to move or rewrite a layered slot whose `conflict` says it holds several
  // different live values (score-editor.ts `scoreOpSeveralValues`), so that flag must agree with
  // the slot's own evidence, as `headerConflicts` must above: an under-reported flag would let a
  // grid edit drop versions the member never saw (re-review of e7250d48, LOW-2).
  if (layers) for (const [key, slot] of Object.entries(layers)) {
    const several = distinctCells([slot.register.selected.value, ...slot.register.conflicts.map((c) => c.value)]).length > 1;
    if (several !== (layerIssues![key]?.conflict ?? false)) fail(`${what}.readiness.layers conflict disagrees with the values ${key} holds`);
  }
  return { kind: "score", title, bpm, bpb, steps, cells, ...(layers ? { layers } : {}), patches, readiness: {
    cells: issueMap<ScoreCellIssues>(ready.cells, `${what}.readiness.cells`, issueKeys, cells, (key) => { scoreAddress(key); return key; }),
    patches: issueMap<ScorePatchIssues>(ready.patches, `${what}.readiness.patches`, ["count", "conflict"], patches, (key) => hex64(key, "score patch issue id")),
    headerConflicts,
    ...(layerIssues ? { layers: layerIssues } : {}),
  } };
}

function parseContent(v: unknown, what: string): StudioContent {
  const r = record(v, what);
  if (r.kind === "index") return parseIndexContent(r, what);
  if (r.kind === "flipnote") return parseFlipnoteContent(r, what);
  if (r.kind === "score") return parseScoreContent(r, what);
  return fail(`${what}.kind is unknown`);
}

function phase(v: unknown, what: string): StudioPhase {
  if (v === "open" || v === "closing" || v === "settled" || v === "fault") return v;
  return fail(`${what} is not a known phase`);
}

/// One non-null Studio response into the two-state union. The preview branch must carry neither
/// phase nor publication; the ordinary branch must carry both.
export function parseStudioView(v: unknown): StudioView {
  const r = record(v, "view");
  if (r.v !== 1) fail("view.v is not 1");
  if (r.provisional !== true) fail("view.provisional is not true");
  const base = {
    v: 1 as const,
    epochId: hex32(r.epochId, "view.epochId"),
    epoch: decimal(r.epoch, "view.epoch"),
    channel: decimal(r.channel, "view.channel"),
    provisional: true as const,
    content: parseContent(r.content, "view.content"),
    ...(r.group === undefined ? {} : { group: typeof r.group === "string" && /^(?:[0-9a-f]{2}){1,256}$/.test(r.group) ? r.group : fail("view.group is not a bounded stable group id") }),
  };
  if (r.awaitingTenureReceipt === true) {
    if (r.phase !== undefined && r.phase !== null) fail("a preview must not claim a phase");
    if (r.publication !== undefined && r.publication !== null) fail("a preview must not claim a publication");
    return { ...base, awaitingTenureReceipt: true, phase: null, publication: null };
  }
  if (r.awaitingTenureReceipt !== undefined && r.awaitingTenureReceipt !== null && r.awaitingTenureReceipt !== false) {
    fail("view.awaitingTenureReceipt is not a boolean");
  }
  if (r.publication !== "local") fail("view.publication is not local");
  return { ...base, awaitingTenureReceipt: false, phase: phase(r.phase, "view.phase"), publication: "local" };
}

export function parseIndexView(v: unknown): IndexView {
  const view = parseStudioView(v);
  if (view.content.kind !== "index") fail("expected an Index view");
  return view as IndexView;
}
export function parseFlipnoteView(v: unknown): FlipnoteView {
  const view = parseStudioView(v);
  if (view.content.kind !== "flipnote") fail("expected a Flipnote view");
  return view as FlipnoteView;
}
export function parseScoreView(v: unknown): ScoreView {
  const view = parseStudioView(v);
  if (view.content.kind !== "score") fail("expected a Score view");
  return view as ScoreView;
}

export function parsePixPublication(v: unknown): PixPublication {
  const r = record(v, "publication");
  return { cid: hex64(r.cid, "publication.cid"), bytes: count(r.bytes, "publication.bytes") };
}

function recoveryVersion(v: unknown, what: string): RecoveryVersion {
  const r = record(v, what);
  const reason = r.reason;
  if (reason !== "excluded" && reason !== "rewound" && reason !== "conflictOverflow" && reason !== "repair") fail(`${what}.reason is unknown`);
  return {
    snapshot: hex64(r.snapshot, `${what}.snapshot`),
    epoch: decimal(r.epoch, `${what}.epoch`),
    staged: flag(r.staged, `${what}.staged`),
    bytes: count(r.bytes, `${what}.bytes`),
    reason,
  };
}

export function parseRecoveryListing(v: unknown): RecoveryListing {
  const r = record(v, "listing");
  if (r.v !== 1) fail("listing.v is not 1");
  if (r.kind !== "recoveryList" && r.kind !== "recoveryAcknowledged") fail("listing.kind is unknown");
  return {
    v: 1,
    kind: r.kind,
    channel: decimal(r.channel, "listing.channel"),
    object: nullable(r.object, (o) => hex32(o, "listing.object")),
    source: nullable(r.source, (s) => {
      const e = record(s, "listing.source");
      if (e.provisional !== true) fail("listing.source.provisional is not true");
      return { epochId: hex32(e.epochId, "listing.source.epochId"), epoch: decimal(e.epoch, "listing.source.epoch"), phase: phase(e.phase, "listing.source.phase"), provisional: true as const };
    }),
    versions: list(r.versions, "listing.versions").map((x, i) => recoveryVersion(x, `listing.versions[${i}]`)),
    evictionPending: nullable(r.evictionPending, (w) => {
      const e = record(w, "listing.evictionPending");
      return {
        oldestSnapshot: hex64(e.oldestSnapshot, "listing.evictionPending.oldestSnapshot"),
        stagedSnapshot: hex64(e.stagedSnapshot, "listing.evictionPending.stagedSnapshot"),
        deadlineMs: decimal(e.deadlineMs, "listing.evictionPending.deadlineMs"),
      };
    }),
    pendingIntents: count(r.pendingIntents, "listing.pendingIntents"),
  };
}

export function parseRecoveryVersionRead(v: unknown): RecoveryVersionRead {
  const r = record(v, "version");
  if (r.v !== 1 || r.kind !== "recoveryVersion") fail("not a recovery version");
  if (r.historical !== true) fail("version.historical is not true");
  return { v: 1, kind: "recoveryVersion", historical: true, version: recoveryVersion(r.version, "version.version"), channel: decimal(r.channel, "version.channel"),
    ...(r.group === undefined ? {} : { group: typeof r.group === "string" && /^(?:[0-9a-f]{2}){1,256}$/.test(r.group) ? r.group : fail("version.group is not a bounded stable group id") }),
    content: parseContent(r.content, "version.content") };
}

export function parseRecoveryExport(v: unknown): RecoveryExport {
  const r = record(v, "export");
  if (r.v !== 1 || r.kind !== "recoveryExport") fail("not a recovery export");
  if (r.format !== "p1-recovery-v1") fail("export.format is unknown");
  const bytesB64 = text(r.bytesB64, "export.bytesB64");
  const bytes = count(r.bytes, "export.bytes");
  if (base64Length(bytesB64) !== bytes) fail("export.bytes does not match its payload");
  return { v: 1, kind: "recoveryExport", snapshot: hex64(r.snapshot, "export.snapshot"), format: "p1-recovery-v1", bytes, bytesB64 };
}

export function parseRecoveryPreview(v: unknown): RecoveryPreview {
  const r = record(v, "preview");
  if (r.v !== 1 || r.kind !== "recoveryPreview") fail("not a recovery preview");
  const d = r.disposition;
  if (d !== "ready" && d !== "unchanged" && d !== "conflict" && d !== "deleted" && d !== "full" && d !== "missingTarget") fail("preview.disposition is unknown");
  const body = nullable(r.body, (b) => text(b, "preview.body"));
  if (d === "ready" && body === null) fail("a ready preview carries a body");
  return {
    v: 1,
    kind: "recoveryPreview",
    snapshot: hex64(r.snapshot, "preview.snapshot"),
    epochId: hex32(r.epochId, "preview.epochId"),
    expectedProjection: hex64(r.expectedProjection, "preview.expectedProjection"),
    disposition: d,
    body,
    originalAuthor: nullable(r.originalAuthor, (a) => hex64(a, "preview.originalAuthor")),
  };
}

export function parseRecoveryApplied(v: unknown): RecoveryApplied {
  const r = record(v, "applied");
  if (r.v !== 1 || r.kind !== "recoveryApplied") fail("not a recovery apply result");
  if (r.contentSaved !== true || r.provisional !== true || r.pointerRestored !== false) fail("applied claims are not the documented ones");
  return { v: 1, kind: "recoveryApplied", contentSaved: true, alreadySaved: flag(r.alreadySaved, "applied.alreadySaved"), provisional: true, pointerRestored: false };
}

export function parsePointerRestored(v: unknown): PointerRestored {
  const r = record(v, "pointer");
  if (r.v !== 1 || r.kind !== "recoveryPointerRestored") fail("not a pointer restoration");
  if (r.provisional !== true) fail("pointer.provisional is not true");
  return {
    v: 1,
    kind: "recoveryPointerRestored",
    channel: decimal(r.channel, "pointer.channel"),
    object: nullable(r.object, (o) => hex32(o, "pointer.object")),
    checkpointEpoch: decimal(r.checkpointEpoch, "pointer.checkpointEpoch"),
    registryEpochId: hex32(r.registryEpochId, "pointer.registryEpochId"),
    provisional: true,
  };
}

const OVERLAY_MANUAL_REASONS = new Set<OverlayManualReason>([
  "sourceMissing", "sourceUnreadable", "sourceNotClosing", "sourceReplaced", "sourceRewound",
  "successorNotPristine", "successorMissing", "receiptChanged", "objectMissing", "tenureUnknown",
  "tenureImported", "preparedStuck", "fault", "notCurrentAuthor", "unconfirmed", "notReplayable",
]);
const OVERLAY_SAVE_REFUSALS = new Set<OverlaySaveRefusal>([
  ...OVERLAY_MANUAL_REASONS,
  "notMember", "sourceAdopting", "installedSource", "previewUnavailable", "previewExpired",
  "tailIncomplete", "previewReplaced", "membershipChanged", "basisChanged",
  "basisChangedDuringDetach", "transferInProgress", "otherKindDraft", "liveBranchSuperseded",
  "staleBranch", "nonceConflict", "ordinaryIntentPending", "branchFull", "pixelUnpublished",
  "pixelInvalid", "pixelMissing", "unconfirmedBranchesPerServer", "unconfirmedBytesVaultWide",
  "exportUnpublished", "exportInvalid", "exportMissing",
  "referenceCapacity", "storageRefused", "retry",
]);

function overlayManualReason(v: unknown, what: string): OverlayManualReason {
  if (typeof v !== "string" || !OVERLAY_MANUAL_REASONS.has(v as OverlayManualReason)) fail(`${what} is unknown`);
  return v as OverlayManualReason;
}
function overlaySaveRefusal(v: unknown, what: string): OverlaySaveRefusal {
  if (typeof v !== "string" || !OVERLAY_SAVE_REFUSALS.has(v as OverlaySaveRefusal)) fail(`${what} is unknown`);
  return v as OverlaySaveRefusal;
}
function overlayTarget(v: unknown, what: string): OverlayTarget {
  const r = record(v, what);
  const object = nullable(r.object, (x) => hex32(x, `${what}.object`));
  if (r.objectKind !== undefined && (object === null || (r.objectKind !== "score" && r.objectKind !== "flipnote"))) fail(`${what}.objectKind is invalid for this target`);
  return { channel: decimal(r.channel, `${what}.channel`), object,
    ...(r.objectKind === undefined ? {} : { objectKind: r.objectKind as StudioObjectKind }) };
}
function overlayUnconfirmedState(v: unknown, what: string): OverlayUnconfirmedState {
  if (v !== "awaitingSource" && v !== "baseConfirmed" && v !== "baseSuperseded" && v !== "sourceUnreadable") fail(`${what} is unknown`);
  return v;
}
function overlayHandoffAttempt(v: unknown, what: string): OverlayHandoffAttempt {
  const r = record(v, what);
  if (r.state !== "refused" && r.state !== "backingOff") fail(`${what}.state is unknown`);
  const reason = r.reason === "transient" ? "transient" : overlayManualReason(r.reason, `${what}.reason`);
  return { state: r.state, reason, retryAt: decimal(r.retryAt, `${what}.retryAt`) };
}
function overlayProvenance(v: unknown, what: string): OverlayProvenance {
  const r = record(v, what);
  if (r.kind === "closing") return { kind: "closing" };
  if (r.kind !== "unconfirmed") fail(`${what}.kind is unknown`);
  return {
    kind: "unconfirmed",
    provider: hex64(r.provider, `${what}.provider`),
    observedMlsEpoch: decimal(r.observedMlsEpoch, `${what}.observedMlsEpoch`),
    observedAtMs: decimal(r.observedAtMs, `${what}.observedAtMs`),
  };
}
function overlayEligibility(r: Record<string, unknown>, what: string): {
  eligibility: "transferable" | "manual" | null;
  manualReason: OverlayManualReason | null;
} {
  const eligibility = nullable(r.eligibility, (v) => {
    if (v !== "transferable" && v !== "manual") fail(`${what}.eligibility is unknown`);
    return v;
  });
  const manualReason = nullable(r.manualReason, (v) => overlayManualReason(v, `${what}.manualReason`));
  if ((eligibility === "manual") !== (manualReason !== null)) fail(`${what} must carry a reason exactly when manual`);
  return { eligibility, manualReason };
}

export function parseOverlayInspection(v: unknown): OverlayInspection {
  const r = record(v, "overlay inspection");
  if (r.v !== 1) fail("overlay inspection.v is not 1");
  const target = overlayTarget(r, "overlay inspection");
  if (r.kind === "absent") return { v: 1, kind: "absent", ...target };
  if (r.kind === "disposed") {
    if (r.readOnly !== true) fail("disposed overlay is not read-only");
    if (r.mode !== "preserved" && r.mode !== "discarded") fail("disposed overlay mode is unknown");
    const archive = nullable(r.archive, (x) => hex64(x, "overlay inspection.archive"));
    if ((r.mode === "preserved") !== (archive !== null)) fail("disposed overlay archive disagrees with its mode");
    return { v: 1, kind: "disposed", ...target, basis: hex64(r.basis, "overlay inspection.basis"), branch: hex64(r.branch, "overlay inspection.branch"), generation: decimal(r.generation, "overlay inspection.generation"), accepted: count(r.accepted, "overlay inspection.accepted"), mode: r.mode, archive, readOnly: true };
  }
  if (r.kind !== "local-draft") fail("overlay inspection.kind is unknown");
  if (r.readOnly !== true) fail("local draft is not read-only");
  if (r.transferState !== "active" && r.transferState !== "prepared") fail("overlay inspection.transferState is unknown");
  if (r.provenance !== null && r.provenance !== "closing" && r.provenance !== "unconfirmed") fail("overlay inspection.provenance is unknown");
  const classified = overlayEligibility(r, "overlay inspection");
  const content = nullable(r.content, (x) => parseContent(x, "overlay inspection.content"));
  const replayable = flag(r.replayable, "overlay inspection.replayable");
  if (replayable !== (content !== null)) fail("overlay inspection replayability disagrees with content");
  if ((r.contentEpoch === undefined) !== (r.group === undefined) || (r.contentEpoch !== undefined && content === null)) fail("overlay media identity must describe actual rebuilt content");
  return {
    v: 1, kind: "local-draft", ...target, ...classified,
    handoffAttempt: nullable(r.handoffAttempt, (x) => overlayHandoffAttempt(x, "overlay inspection.handoffAttempt")),
    unconfirmedState: nullable(r.unconfirmedState, (x) => overlayUnconfirmedState(x, "overlay inspection.unconfirmedState")),
    basis: nullable(r.basis, (x) => hex64(x, "overlay inspection.basis")),
    branch: hex64(r.branch, "overlay inspection.branch"),
    contentId: nullable(r.contentId, (x) => hex64(x, "overlay inspection.contentId")),
    generation: decimal(r.generation, "overlay inspection.generation"),
    accepted: count(r.accepted, "overlay inspection.accepted"), transferState: r.transferState,
    provenance: r.provenance, replayable, readOnly: true, content,
    ...(r.contentEpoch === undefined ? {} : { contentEpoch: decimal(r.contentEpoch, "overlay inspection.contentEpoch") }),
    ...(r.group === undefined ? {} : { group: typeof r.group === "string" && /^(?:[0-9a-f]{2}){1,256}$/.test(r.group) ? r.group : fail("overlay inspection.group is not a bounded stable group id") }),
  };
}

export function parseOverlayLifecycle(v: unknown): OverlayLifecycle {
  const r = record(v, "overlay lifecycle");
  if (r.v !== 1 || r.kind !== "overlayLifecycle") fail("not an overlay lifecycle");
  const target = overlayTarget(r, "overlay lifecycle");
  const classified = overlayEligibility(r, "overlay lifecycle");
  const branch = nullable(r.branch, (x) => {
    const b = record(x, "overlay lifecycle.branch");
    return { branch: hex64(b.branch, "overlay lifecycle.branch.branch"), content: hex64(b.content, "overlay lifecycle.branch.content"), generation: decimal(b.generation, "overlay lifecycle.branch.generation"), accepted: count(b.accepted, "overlay lifecycle.branch.accepted") };
  });
  const archive = nullable(r.archive, (x) => {
    const a = record(x, "overlay lifecycle.archive");
    return { archive: hex64(a.archive, "overlay lifecycle.archive.archive"), branch: hex64(a.branch, "overlay lifecycle.archive.branch"), generation: decimal(a.generation, "overlay lifecycle.archive.generation"), replayable: flag(a.replayable, "overlay lifecycle.archive.replayable") };
  });
  const disposed = nullable(r.disposed, (x) => {
    const d = record(x, "overlay lifecycle.disposed");
    const common = { branch: hex64(d.branch, "overlay lifecycle.disposed.branch"), generation: decimal(d.generation, "overlay lifecycle.disposed.generation") };
    if (d.mode === "preserved") return { ...common, mode: "preserved" as const, archive: hex64(d.archive, "overlay lifecycle.disposed.archive") };
    if (d.mode === "discarded") return { ...common, mode: "discarded" as const };
    return fail("overlay lifecycle.disposed.mode is unknown");
  });
  return {
    v: 1, kind: "overlayLifecycle", ...target, branch,
    prepared: flag(r.prepared, "overlay lifecycle.prepared"), ...classified,
    handoffAttempt: nullable(r.handoffAttempt, (x) => overlayHandoffAttempt(x, "overlay lifecycle.handoffAttempt")),
    unconfirmedState: nullable(r.unconfirmedState, (x) => overlayUnconfirmedState(x, "overlay lifecycle.unconfirmedState")),
    archive, disposed, transferred: flag(r.transferred, "overlay lifecycle.transferred"),
  };
}

export function parseOverlayArchive(v: unknown): OverlayArchive {
  const r = record(v, "overlay archive");
  if (r.v !== 1 || (r.kind !== "overlayArchive" && r.kind !== "overlayArchived" && r.kind !== "overlayArchiveExport" && r.kind !== "overlayExport")) fail("not an overlay archive");
  if (r.readOnly !== true || r.authority !== false || r.terminal !== true) fail("overlay archive authority labels are invalid");
  const out: OverlayArchive = {
    v: 1, kind: r.kind, archive: hex64(r.archive, "overlay archive.archive"),
    basis: hex64(r.basis, "overlay archive.basis"), content: hex64(r.content, "overlay archive.content"),
    branch: hex64(r.branch, "overlay archive.branch"), generation: decimal(r.generation, "overlay archive.generation"),
    author: hex64(r.author, "overlay archive.author"), accepted: count(r.accepted, "overlay archive.accepted"),
    provenance: overlayProvenance(r.provenance, "overlay archive.provenance"),
    replayable: flag(r.replayable, "overlay archive.replayable"), readOnly: true, authority: false, terminal: true,
  };
  if (r.physicalBytes !== undefined) out.physicalBytes = decimal(r.physicalBytes, "overlay archive.physicalBytes");
  if (r.preserved !== undefined) out.preserved = flag(r.preserved, "overlay archive.preserved");
  if (r.notReplayable !== undefined) out.notReplayable = nullable(r.notReplayable, (x) => text(x, "overlay archive.notReplayable"));
  if (r.format !== undefined || r.bytes !== undefined || r.bytesB64 !== undefined) {
    if (r.format !== "catcoms-studio-draft-v1") fail("overlay archive.format is unknown");
    out.format = "catcoms-studio-draft-v1";
    out.bytes = count(r.bytes, "overlay archive.bytes");
    out.bytesB64 = text(r.bytesB64, "overlay archive.bytesB64");
    if (base64Length(out.bytesB64) !== out.bytes) fail("overlay archive bytes do not match the payload");
  }
  return out;
}

export function parseOverlayArchiveReleased(v: unknown): OverlayArchiveReleased {
  const r = record(v, "overlay archive release");
  if (r.v !== 1 || r.kind !== "overlayArchiveReleased" || r.reconcileRequired !== true) fail("not an overlay archive release");
  return { v: 1, kind: "overlayArchiveReleased", reconcileRequired: true };
}

function overlayDisposal(v: unknown, what: string): OverlayDisposed["disposal"] {
  const r = record(v, what);
  if (r.mode === "preserved") return { mode: "preserved", archive: hex64(r.archive, `${what}.archive`) };
  if (r.mode === "discarded") return { mode: "discarded" };
  return fail(`${what}.mode is unknown`);
}
export function parseOverlayDisposed(v: unknown): OverlayDisposed {
  const r = record(v, "overlay disposal");
  if (r.v !== 1 || r.kind !== "overlayDisposed" || r.terminal !== true) fail("not an overlay disposal");
  const target = overlayTarget(r, "overlay disposal");
  return { v: 1, kind: "overlayDisposed", ...target, author: hex64(r.author, "overlay disposal.author"), provenance: overlayProvenance(r.provenance, "overlay disposal.provenance"), basis: hex64(r.basis, "overlay disposal.basis"), branch: hex64(r.branch, "overlay disposal.branch"), content: hex64(r.content, "overlay disposal.content"), generation: decimal(r.generation, "overlay disposal.generation"), disposal: overlayDisposal(r.disposal, "overlay disposal.disposal"), accepted: count(r.accepted, "overlay disposal.accepted"), sequence: decimal(r.sequence, "overlay disposal.sequence"), atMs: decimal(r.atMs, "overlay disposal.atMs"), terminal: true };
}

export function parseOverlayCopyPreview(v: unknown): OverlayCopyPreview {
  const r = record(v, "overlay copy preview");
  if (r.v !== 1 || r.kind !== "overlayCopyPreview" || r.preservesBranch !== false || r.provisional !== true) fail("not an overlay copy preview");
  const d = r.disposition;
  if (d !== "ready" && d !== "unchanged" && d !== "conflict" && d !== "deleted" && d !== "full" && d !== "missingTarget") fail("overlay copy preview.disposition is unknown");
  const body = nullable(r.body, (x) => text(x, "overlay copy preview.body"));
  if (d === "ready" && body === null) fail("a ready overlay copy preview carries a body");
  return { v: 1, kind: "overlayCopyPreview", source: overlayTarget(r.source, "overlay copy preview.source"), destination: overlayTarget(r.destination, "overlay copy preview.destination"), epochId: hex32(r.epochId, "overlay copy preview.epochId"), expectedProjection: hex64(r.expectedProjection, "overlay copy preview.expectedProjection"), disposition: d, body, originalAuthor: nullable(r.originalAuthor, (x) => hex64(x, "overlay copy preview.originalAuthor")), sourceOps: list(r.sourceOps, "overlay copy preview.sourceOps").map((x, i) => hex64(x, `overlay copy preview.sourceOps[${i}]`)), preservesBranch: false, provisional: true };
}
export function parseOverlayCopyApplied(v: unknown): OverlayCopyApplied {
  const r = record(v, "overlay copy applied");
  if (r.v !== 1 || r.kind !== "overlayCopyApplied" || r.contentSaved !== true || r.provisional !== true || r.branchPreserved !== false) fail("not an overlay copy apply result");
  return { v: 1, kind: "overlayCopyApplied", ...overlayTarget(r, "overlay copy applied"), contentSaved: true, alreadySaved: flag(r.alreadySaved, "overlay copy applied.alreadySaved"), provisional: true, branchPreserved: false };
}
export function parseOverlayCopyApplyResult(v: unknown): OverlayCopyApplyResult {
  const r = record(v, "overlay copy apply result");
  if (r.kind === "overlayCopyApplied") return parseOverlayCopyApplied(v);
  if (r.v !== 1 || r.kind !== "overlayCopyRefused" || r.reason !== "stalePreview" || r.retry !== "repreview" || r.written !== false) fail("not an overlay copy refusal");
  return { v: 1, kind: "overlayCopyRefused", ...overlayTarget(r, "overlay copy refused"), reason: "stalePreview", retry: "repreview", written: false };
}
export function parseUnconfirmedOverlaySaveTicket(v: unknown): UnconfirmedOverlaySaveTicket {
  return parseOverlaySaveTicket(v, "unconfirmed");
}
function parseOverlaySaveTicket<P extends OverlaySaveProvenance>(v: unknown, provenance: P): OverlaySaveTicket<P> {
  const r = record(v, "unconfirmed overlay save ticket");
  if (r.v !== 1 || r.kind !== `${provenance}OverlaySaveTicket` || r.provenance !== provenance) fail(`not a ${provenance} overlay save ticket`);
  return { v: 1, kind: `${provenance}OverlaySaveTicket`, ...overlayTarget(r, "overlay save ticket"), basis: hex64(r.basis, "overlay save ticket.basis"), branch: hex64(r.branch, "overlay save ticket.branch"), provenance };
}
export function parseUnconfirmedOverlaySave(v: unknown): UnconfirmedOverlaySave {
  return parseOverlaySave(v, "unconfirmed");
}
function parseOverlaySave<P extends OverlaySaveProvenance>(v: unknown, provenance: P): OverlaySaveResult<P> {
  const r = record(v, "unconfirmed overlay save");
  if (r.v !== 1 || r.kind !== `${provenance}OverlaySave`) fail(`not a ${provenance} overlay save result`);
  const common = { v: 1 as const, kind: `${provenance}OverlaySave` as const, ...overlayTarget(r, "overlay save") };
  if (r.state === "saved") {
    if (r.localOnly !== true || r.provisional !== true || r.provenance !== provenance) fail("overlay saved claims are invalid");
    return { ...common, state: "saved", basis: hex64(r.basis, "overlay save.basis"), accepted: count(r.accepted, "overlay save.accepted"), localOnly: true, provisional: true, provenance };
  }
  if (r.state === "pending" || r.state === "busy") {
    if (r.retry !== "sameRequest") fail("unconfirmed overlay save retry is invalid");
    return { ...common, state: r.state, retry: "sameRequest" };
  }
  if (r.state === "paused") {
    if (r.retry !== "afterExplicitAccess") fail("paused overlay save retry is invalid");
    return { ...common, state: "paused", retry: "afterExplicitAccess" };
  }
  if (r.state === "refused") {
    if (r.retry !== "sameRequest" && r.retry !== "newTicket" && r.retry !== "none") fail("refused overlay save retry is invalid");
    return { ...common, state: "refused", reason: overlaySaveRefusal(r.reason, "unconfirmed overlay save.reason"), retry: r.retry };
  }
  if (r.state === "disposed") return { ...common, state: "disposed", disposal: parseOverlayDisposed(r.disposal) };
  if (r.state === "handedOff") return { ...common, state: "handedOff", basis: hex64(r.basis, "unconfirmed overlay save.basis"), epoch: decimal(r.epoch, "unconfirmed overlay save.epoch"), docId: hex32(r.docId, "unconfirmed overlay save.docId"), accepted: count(r.accepted, "unconfirmed overlay save.accepted") };
  return fail("unconfirmed overlay save.state is unknown");
}
export function parseUnconfirmedOverlaySaveBegin(v: unknown): UnconfirmedOverlaySaveBegin {
  const r = record(v, "unconfirmed overlay save begin");
  if (r.kind === "unconfirmedOverlaySaveTicket") return parseUnconfirmedOverlaySaveTicket(v);
  const result = parseUnconfirmedOverlaySave(v);
  if (result.state !== "refused") fail("an unconfirmed overlay save begin result must be a ticket or refusal");
  return result;
}

export function parseStudioUpdated(v: unknown): StudioUpdatedEvent {
  const r = record(v, "studio-updated");
  return { server: count(r.server, "event.server"), ...overlayTarget(r, "event") };
}
export function parseReceivePaused(v: unknown): StudioReceivePausedEvent {
  return { server: count(record(v, "studio-receive-paused").server, "event.server") };
}
export function parseStudioSyncState(v: unknown): StudioSyncStateEvent {
  const r = record(v, "studio-sync-state"), paused = flag(r.paused, "event.paused");
  const sequence = (value: unknown, name: string): Decimal => {
    if (typeof value !== "string" || value.length > 20) fail(`${name} is not a u64`);
    const result = decimal(value, name);
    if (BigInt(result) > (1n << 64n) - 1n) fail(`${name} is not a u64`);
    return result;
  };
  if (r.cause !== null && r.cause !== "background" && r.cause !== "incoming") fail("event.cause is unknown");
  if (r.detail !== null && (typeof r.detail !== "string" || r.detail.length > 2048)) fail("event.detail is invalid");
  if (!paused && (r.cause !== null || r.detail !== null)) fail("a resumed event cannot retain a pause reason");
  return { server: count(r.server, "event.server"), instance: sequence(r.instance, "event.instance"), sequence: sequence(r.sequence, "event.sequence"),
    paused, cause: r.cause as StudioSyncStateEvent["cause"], detail: r.detail as string | null };
}
export function parseSettlementChanged(v: unknown): SettlementChangedEvent {
  const r = record(v, "settlement-changed");
  const s = r.state;
  // These notifications invalidate observations; accepting runtime repair/storage states
  // grants no renderer repair authority. Do not depend on a companion refreshRequired event.
  if (s !== "open" && s !== "closing" && s !== "settled" && s !== "fault" && s !== "recoveryAvailable" && s !== "recoveryEvictionPending" && s !== "refreshRequired" && s !== "repairing" && s !== "storageRefused") fail("event.state is unknown");
  if (r.docType !== 15 && r.docType !== 16) fail("event.docType is unknown");
  return {
    server: count(r.server, "event.server"),
    docType: r.docType,
    logicalKey: hex32(r.logicalKey, "event.logicalKey"),
    ...overlayTarget(r, "event"),
    state: s,
  };
}

// --- Base64 --------------------------------------------------------------------------------------

export function bytesToBase64(bytes: Uint8Array): string {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}
export function base64ToBytes(b64: string): Uint8Array<ArrayBuffer> {
  const s = atob(b64);
  const out = new Uint8Array(new ArrayBuffer(s.length));
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}
function base64Length(b64: string): number {
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(b64) || b64.length % 4 !== 0) fail("payload is not base64");
  const pad = b64.endsWith("==") ? 2 : b64.endsWith("=") ? 1 : 0;
  return (b64.length / 4) * 3 - pad;
}

// --- Commands: literal names, camelCase arguments, typed results -----------------------------------

export type StudioTargetArgs = { server: number; channel: Decimal };

export async function studioList(ipc: StudioIpc, target: StudioTargetArgs): Promise<IndexView> {
  const raw = await ipc.invoke<unknown>("studio_list", { server: target.server, channel: target.channel });
  if (raw === null || raw === undefined) fail("studio_list returned nothing");
  return parseIndexView(raw);
}

type ObjectViewFor<K extends StudioObjectKind> = K extends "score" ? ScoreView : FlipnoteView;
function objectKindArgs(kind: StudioObjectKind | undefined): Record<string, unknown> {
  if (kind !== undefined && kind !== "score" && kind !== "flipnote") fail("objectKind is unknown");
  return kind === undefined ? {} : { objectKind: kind };
}
function parseObjectView<K extends StudioObjectKind>(raw: unknown, kind: K | undefined): ObjectViewFor<K> {
  // The trusted target kind selects the decoder. Never try whichever body parser succeeds.
  return (kind === "score" ? parseScoreView(raw) : parseFlipnoteView(raw)) as ObjectViewFor<K>;
}
export async function studioRead<K extends StudioObjectKind = "flipnote">(ipc: StudioIpc, target: StudioTargetArgs, object: Hex32, objectKind?: K): Promise<ObjectViewFor<K> | null> {
  const raw = await ipc.invoke<unknown>("studio_read", { server: target.server, channel: target.channel, object, ...objectKindArgs(objectKind) });
  return raw === null || raw === undefined ? null : parseObjectView(raw, objectKind);
}

export type CreateRequest<K extends StudioObjectKind = "flipnote"> = { object: Hex32; nonce: Hex32; title: string; createdAtMs: number; objectKind?: K };
export async function studioCreate<K extends StudioObjectKind = "flipnote">(ipc: StudioIpc, target: StudioTargetArgs, req: CreateRequest<K>): Promise<ObjectViewFor<K>> {
  if (!Number.isSafeInteger(req.createdAtMs) || req.createdAtMs < 0) fail("createdAtMs must be a non-negative safe integer");
  const raw = await ipc.invoke<unknown>("studio_create", { server: target.server, channel: target.channel, object: req.object, nonce: req.nonce, title: req.title, createdAtMs: req.createdAtMs, ...objectKindArgs(req.objectKind) });
  if (raw === null || raw === undefined) fail("studio_create returned nothing");
  return parseObjectView(raw, req.objectKind);
}

export type ApplyRequest<K extends StudioObjectKind = "flipnote"> = { object: Hex32; epochId: Hex32; nonce: Hex32; body: string; objectKind?: K };
export async function studioApply<K extends StudioObjectKind = "flipnote">(ipc: StudioIpc, target: StudioTargetArgs, req: ApplyRequest<K>): Promise<ObjectViewFor<K>> {
  const raw = await ipc.invoke<unknown>("studio_apply", { server: target.server, channel: target.channel, object: req.object, epochId: req.epochId, nonce: req.nonce, body: req.body, ...objectKindArgs(req.objectKind) });
  if (raw === null || raw === undefined) fail("studio_apply returned nothing");
  return parseObjectView(raw, req.objectKind);
}

/** Native's bounds for one batched Score Apply (`StudioRequest::ApplyBatch`): operations and body bytes. */
export const STUDIO_APPLY_BATCH_MAX = 64;
export const STUDIO_APPLY_BATCH_MAX_BYTES = 256 * 1024;
export type ApplyBatchRequest = { object: Hex32; epochId: Hex32; operations: readonly { nonce: Hex32; body: string }[] };
/// Several Score edits in ONE native transaction (one durable server snapshot, one storage scan,
/// one publication window, one update event) instead of one of each per edit. Every operation
/// keeps its own nonce, so an exact retry of the whole batch is as safe as retrying each edit:
/// accepted items are recognised and resealed, never applied twice. The batch is not atomic; an
/// item native refuses leaves the items before it saved, and the error names it. Score only.
export async function studioApplyBatch(ipc: StudioIpc, target: StudioTargetArgs, req: ApplyBatchRequest): Promise<ScoreView> {
  if (req.operations.length < 1 || req.operations.length > STUDIO_APPLY_BATCH_MAX) fail(`studio_apply_batch holds 1 to ${STUDIO_APPLY_BATCH_MAX} operations`);
  const raw = await ipc.invoke<unknown>("studio_apply_batch", { server: target.server, channel: target.channel, object: req.object, epochId: req.epochId,
    objectKind: "score", operations: req.operations.map((o) => ({ nonce: o.nonce, body: o.body })) });
  if (raw === null || raw === undefined) fail("studio_apply_batch returned nothing");
  return parseScoreView(raw);
}

export type ApplyIndexRequest = { epochId: Hex32; nonce: Hex32; body: string };
export async function studioApplyIndex(ipc: StudioIpc, target: StudioTargetArgs, req: ApplyIndexRequest): Promise<IndexView> {
  const raw = await ipc.invoke<unknown>("studio_apply_index", { server: target.server, channel: target.channel, epochId: req.epochId, nonce: req.nonce, body: req.body });
  if (raw === null || raw === undefined) fail("studio_apply_index returned nothing");
  return parseIndexView(raw);
}

/// Stage, promote, publish. The returned cid is the only name these bytes have; a frontend hash
/// is never a cid. `bytes` must echo the exact encoded length.
export async function publishPix(ipc: StudioIpc, server: number, pix: Uint8Array): Promise<PixPublication> {
  if (pix.length > PIX_MAX_BYTES) fail("pix over 64 KiB");
  const published = parsePixPublication(await ipc.invoke<unknown>("publish_pix", { server, bytesB64: bytesToBase64(pix) }));
  if (published.bytes !== pix.length) fail("publish_pix reported a different length than the bytes sent");
  return published;
}

/// Bounded by the record's declared size; `null` is unavailable (or still fetching), and a body of
/// any other length than declared is rejected before decoding.
export async function requestBlobBounded(ipc: StudioIpc, server: number, cid: Hex64, maxBytes: number): Promise<Uint8Array | null> {
  if (!isHex64(cid)) fail("cid is not 64 lowercase hex");
  if (!Number.isSafeInteger(maxBytes) || maxBytes < 0) fail("maxBytes is not a safe integer");
  const raw = await ipc.invoke<unknown>("request_blob_bounded", { server, cid, maxBytes });
  if (raw === null || raw === undefined) return null;
  const r = record(raw, "blob");
  const declared = count(r.bytes, "blob.bytes");
  const bytes = base64ToBytes(text(r.bytes_b64, "blob.bytes_b64"));
  if (bytes.length !== declared || bytes.length !== maxBytes) fail("fetched blob length differs from the declared size");
  return bytes;
}

export type RecoveryTargetArgs = StudioTargetArgs & { object: Hex32 | null; objectKind?: StudioObjectKind };
function recoveryArgs(t: RecoveryTargetArgs): Record<string, unknown> {
  if (t.object === null && t.objectKind !== undefined) fail("objectKind requires an object target");
  return t.object === null ? { server: t.server, channel: t.channel } : { server: t.server, channel: t.channel, object: t.object, ...objectKindArgs(t.objectKind) };
}

function checkTargetContent(content: StudioContent | null, target: RecoveryTargetArgs): void {
  if (content && content.kind !== (target.object === null ? "index" : target.objectKind ?? "flipnote")) fail("native content kind disagrees with the requested target");
}

export async function overlayRead(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<OverlayInspection> {
  const result = parseOverlayInspection(await ipc.invoke<unknown>("studio_overlay_read", recoveryArgs(t)));
  if (result.kind === "local-draft") checkTargetContent(result.content, t);
  return result;
}
export async function overlayLifecycle(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<OverlayLifecycle> {
  return parseOverlayLifecycle(await ipc.invoke<unknown>("studio_overlay_lifecycle", recoveryArgs(t)));
}
export async function overlayExport(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<OverlayArchive> {
  return parseOverlayArchive(await ipc.invoke<unknown>("studio_overlay_export", recoveryArgs(t)));
}
export async function overlayArchive(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<OverlayArchive> {
  return parseOverlayArchive(await ipc.invoke<unknown>("studio_overlay_archive", recoveryArgs(t)));
}
export async function overlayArchiveRead(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<OverlayArchive> {
  return parseOverlayArchive(await ipc.invoke<unknown>("studio_overlay_archive_read", recoveryArgs(t)));
}
export async function overlayArchiveExport(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<OverlayArchive> {
  return parseOverlayArchive(await ipc.invoke<unknown>("studio_overlay_archive_export", recoveryArgs(t)));
}
export async function overlayArchiveRelease(ipc: StudioIpc, t: RecoveryTargetArgs, archive: Hex64, confirmation: "release-local-archive"): Promise<OverlayArchiveReleased> {
  return parseOverlayArchiveReleased(await ipc.invoke<unknown>("studio_overlay_archive_release", { ...recoveryArgs(t), archive, confirmation }));
}
export async function overlayCopyPreview(ipc: StudioIpc, source: RecoveryTargetArgs, destination: OverlayDestination, choice: RecoveryChoice, mode: RecoveryMode): Promise<OverlayCopyPreview> {
  return parseOverlayCopyPreview(await ipc.invoke<unknown>("studio_overlay_copy_preview", { ...recoveryArgs(source), destination, choice, mode }));
}
export async function overlayCopyApply(ipc: StudioIpc, source: RecoveryTargetArgs, edit: OverlayCopyApplyEdit): Promise<OverlayCopyApplyResult> {
  return parseOverlayCopyApplyResult(await ipc.invoke<unknown>("studio_overlay_copy_apply", { ...recoveryArgs(source), edit }));
}
export async function overlayDispose(ipc: StudioIpc, t: RecoveryTargetArgs, disposal: OverlayDisposalRequest): Promise<OverlayDisposed> {
  return parseOverlayDisposed(await ipc.invoke<unknown>("studio_overlay_dispose", { ...recoveryArgs(t), disposal }));
}
export async function unconfirmedOverlaySaveBegin(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<UnconfirmedOverlaySaveBegin> {
  return parseUnconfirmedOverlaySaveBegin(await ipc.invoke<unknown>("studio_unconfirmed_overlay_save_begin", recoveryArgs(t)));
}
export async function unconfirmedOverlaySave(ipc: StudioIpc, t: RecoveryTargetArgs, save: UnconfirmedOverlaySaveInput): Promise<UnconfirmedOverlaySave> {
  return parseUnconfirmedOverlaySave(await ipc.invoke<unknown>("studio_unconfirmed_overlay_save", { ...recoveryArgs(t), save }));
}
export async function closingOverlaySaveBegin(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<ClosingOverlaySaveBegin> {
  const value = await ipc.invoke<unknown>("studio_closing_overlay_save_begin", recoveryArgs(t));
  if (record(value, "closing Save begin").kind === "closingOverlaySaveTicket") return parseOverlaySaveTicket(value, "closing");
  const result = parseOverlaySave(value, "closing");
  if (result.state !== "refused") fail("Closing Save begin must return a ticket or refusal");
  return result;
}
export async function closingOverlaySave(ipc: StudioIpc, t: RecoveryTargetArgs, save: UnconfirmedOverlaySaveInput): Promise<ClosingOverlaySave> {
  return parseOverlaySave(await ipc.invoke<unknown>("studio_closing_overlay_save", { ...recoveryArgs(t), save }), "closing");
}

export async function recoveryList(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<RecoveryListing> {
  return parseRecoveryListing(await ipc.invoke<unknown>("studio_recovery_list", recoveryArgs(t)));
}
export async function recoveryRead(ipc: StudioIpc, t: RecoveryTargetArgs, snapshot: Hex64): Promise<RecoveryVersionRead> {
  const result = parseRecoveryVersionRead(await ipc.invoke<unknown>("studio_recovery_read", { ...recoveryArgs(t), snapshot }));
  checkTargetContent(result.content, t);
  return result;
}
export async function recoveryExport(ipc: StudioIpc, t: RecoveryTargetArgs, snapshot: Hex64): Promise<RecoveryExport> {
  return parseRecoveryExport(await ipc.invoke<unknown>("studio_recovery_export", { ...recoveryArgs(t), snapshot }));
}
export async function recoveryAcknowledge(ipc: StudioIpc, t: RecoveryTargetArgs, pair: { oldestSnapshot: Hex64; stagedSnapshot: Hex64 }): Promise<RecoveryListing> {
  return parseRecoveryListing(await ipc.invoke<unknown>("studio_recovery_acknowledge", { ...recoveryArgs(t), oldestSnapshot: pair.oldestSnapshot, stagedSnapshot: pair.stagedSnapshot }));
}
export async function recoveryPreview(ipc: StudioIpc, t: RecoveryTargetArgs, snapshot: Hex64, choice: RecoveryChoice, mode: RecoveryMode): Promise<RecoveryPreview> {
  return parseRecoveryPreview(await ipc.invoke<unknown>("studio_recovery_preview", { ...recoveryArgs(t), snapshot, choice, mode }));
}
export async function recoveryApply(ipc: StudioIpc, t: RecoveryTargetArgs, edit: RecoveryApplyEdit): Promise<RecoveryApplied> {
  return parseRecoveryApplied(await ipc.invoke<unknown>("studio_recovery_apply", { ...recoveryArgs(t), edit }));
}
export async function recoveryRestorePointer(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<PointerRestored> {
  return parsePointerRestored(await ipc.invoke<unknown>("studio_recovery_restore_pointer", recoveryArgs(t)));
}

// --- Derived, display-oriented projections (lossless: evidence rides along) -----------------------

export type FrameConflictValue = { cid: Hex64; bytes: number; author: Hex64; ts: number; opId: Hex64 };
export type FrameView = {
  id: Hex32;
  cid: Hex64;
  bytes: number;
  author: Hex64;
  ts: number;
  opId: Hex64;
  /// Every other live replacement value: "another version by X", never resolved silently.
  conflicts: FrameConflictValue[];
  overCap: FrameLimits | null;
  /// Insertion alternatives beyond the winning one (same id inserted concurrently).
  insertions: number;
};
export type FlipnoteModel = {
  title: string;
  titleConflicts: Valued<string, FrameSource>[];
  fps: number;
  fpsConflicts: Valued<number, FrameSource>[];
  frames: FrameView[];
  declaredFrameBytes: number;
  overCapCount: number;
  deletedFrames: Hex32[];
  sound: StudioSoundModel;
};

export const DEFAULT_TITLE = "";
export const DEFAULT_FPS = 12;

export type ScoreModel = {
  title: string; bpm: number; bpb: number; steps: number;
  /** Every raw slot, including clear/remove winners and losing alternatives. */
  cellSlots: ScoreContent["cells"]; patchSlots: ScoreContent["patches"];
  /** Layered note slots keyed `"S:R/L"` (add wins); empty for a score without layers. */
  layerSlots: Record<string, ScoreLayerSlot>;
  /** Legacy effective values only; a selected setter does not defeat a concurrent removal. */
  cells: Record<string, ScoreCell>; patches: Record<Hex64, JamPatch>;
  /** Every note the composer draws, one per note key: legacy cells at `"S:R"` (their effective
   * value) and layered notes at `"S:R/L"` (`layerShownValue`). A layer slot's other distinct values
   * still play (`voices`) and its `conflict` flag says there is more than one. */
  notes: Record<string, ScoreCell>;
  /** What plays, from every slot (score-voices.ts). Nothing refuses playback; issues are review. */
  voices: ScoreVoices;
  /** Diagnostics only: true when there is nothing to review. Never gates playback or export. */
  readiness: ScoreReadiness; ready: boolean;
  headers: Pick<ScoreContent, "title" | "bpm" | "bpb" | "steps">;
};

/** The voice slots of a score's raw evidence, every alternative selected first. Which of them play
 * (removal wins for legacy cells, add wins for layers) is decided in score-voices.ts alone. */
export function scoreVoiceSlots(cellSlots: ScoreContent["cells"], layerSlots: Record<string, ScoreLayerSlot>): ScoreVoiceSlot[] {
  const out: ScoreVoiceSlot[] = [];
  const add = (key: string, register: Register<ScoreCell | null, IndexSource>) => {
    const parsed = parseNoteKey(key);
    if (parsed) out.push({ key, step: parsed.step, row: parsed.row, layer: parsed.layer, values: [register.selected.value, ...register.conflicts.map((c) => c.value)] });
  };
  for (const [key, slot] of Object.entries(cellSlots)) add(key, slot.register);
  for (const [key, slot] of Object.entries(layerSlots)) add(key, slot.register);
  return out;
}

/**
 * The one value a layer slot shows and edits from (docs/design-score-layers.md section 2): the
 * selected value when it is live, else the live alternative with the lowest source op id, the same
 * rule compaction uses to promote a value over a clear. `null` only for a cleared slot. Never use
 * the removal-wins helpers on a layer slot: a clear winning the register does not remove the note.
 */
export function layerShownValue(slot: ScoreLayerSlot): ScoreCell | null {
  return layerShownAlternative(slot)?.value ?? null;
}

/** The alternative `layerShownValue` takes its value from, with the source a restore names. */
export function layerShownAlternative(slot: ScoreLayerSlot): Valued<ScoreCell, IndexSource> | null {
  const selected = slot.register.selected;
  if (selected.value !== null) return { value: selected.value, source: selected.source };
  let best: Valued<ScoreCell, IndexSource> | null = null;
  for (const alternative of slot.register.conflicts) {
    if (alternative.value !== null && (!best || alternative.source.opId < best.source.opId)) best = { value: alternative.value, source: alternative.source };
  }
  return best;
}

/** Whether a readiness has anything to review (diagnostics; nothing gates on it). */
export function scoreReadinessClear(readiness: ScoreReadiness): boolean {
  const none = (issues: Record<string, Record<string, boolean>>) => !Object.values(issues).some((flags) => Object.values(flags).some(Boolean));
  return !readiness.headerConflicts && none(readiness.cells) && none(readiness.patches) && none(readiness.layers ?? {});
}

/** Defaults are display fallback, never a fabricated operation/source or authority to save. */
export function scoreModel(content: ScoreContent): ScoreModel {
  const cells: Record<string, ScoreCell> = {}, patches: Record<Hex64, JamPatch> = {}, notes: Record<string, ScoreCell> = {};
  for (const [key, slot] of Object.entries(content.cells)) if (!slot.removed && slot.register.selected.value !== null) cells[key] = notes[key] = slot.register.selected.value;
  const layerSlots = content.layers ?? {};
  for (const [key, slot] of Object.entries(layerSlots)) {
    const shown = layerShownValue(slot);
    if (shown) notes[key] = shown;
  }
  for (const [key, slot] of Object.entries(content.patches)) if (!slot.removed && slot.register.selected.value !== null) patches[key] = slot.register.selected.value;
  const steps = content.steps?.selected.value ?? 16;
  return {
    title: content.title?.selected.value ?? "", bpm: content.bpm?.selected.value ?? 120,
    bpb: content.bpb?.selected.value ?? 4, steps,
    cellSlots: content.cells, patchSlots: content.patches, layerSlots, cells, notes, patches,
    voices: scoreVoices(scoreVoiceSlots(content.cells, layerSlots), steps, admittedSounds(Object.keys(patches))),
    readiness: content.readiness,
    ready: scoreReadinessClear(content.readiness),
    headers: { title: content.title, bpm: content.bpm, bpb: content.bpb, steps: content.steps },
  };
}

/// The timeline in the backend's order (never map enumeration), each frame with its selected
/// pixels and everything the register kept beside them.
export function flipnoteModel(c: FlipnoteContent): FlipnoteModel {
  const frames: FrameView[] = [];
  for (const id of c.timeline) {
    const e = c.frames[id];
    if (!e) continue; // the timeline names frames the map must hold; a gap is left visible as absence
    const sel = e.pixels.selected;
    frames.push({
      id,
      cid: sel.value.cid,
      bytes: sel.value.bytes,
      author: sel.source.author,
      ts: sel.source.ts,
      opId: sel.source.opId,
      conflicts: e.pixels.conflicts.map((v) => ({ cid: v.value.cid, bytes: v.value.bytes, author: v.source.author, ts: v.source.ts, opId: v.source.opId })),
      overCap: c.overCap[id] ?? null,
      insertions: e.insertions.length,
    });
  }
  return {
    title: c.title?.selected.value ?? DEFAULT_TITLE,
    titleConflicts: c.title?.conflicts ?? [],
    fps: c.fps?.selected.value ?? DEFAULT_FPS,
    fpsConflicts: c.fps?.conflicts ?? [],
    frames,
    declaredFrameBytes: c.declaredFrameBytes,
    overCapCount: Object.keys(c.overCap).length,
    deletedFrames: Object.keys(c.tombstones),
    sound: studioSoundModel(c.sound),
  };
}

export type IndexEntryView = {
  id: Hex32;
  kind: "flipnote" | "score";
  title: string;
  titleConflicts: number;
  createdBy: Hex64;
  ts: number;
  expiry: NativeExpiry;
  expiryConflicts: number;
  /// More than one creation record: concurrent creations of one id, all retained.
  creations: number;
  where: "visible" | "overflow" | "deleted";
};
export type IndexModel = { entries: IndexEntryView[]; overflow: IndexEntryView[]; deleted: IndexEntryView[] };

function entryView(id: Hex32, e: IndexEntry, where: IndexEntryView["where"]): IndexEntryView {
  const first = e.creations[0]?.value;
  return {
    id,
    kind: first?.kind ?? "flipnote",
    title: e.title.selected.value,
    titleConflicts: e.title.conflicts.length,
    createdBy: first?.createdBy ?? "",
    ts: first?.ts ?? 0,
    expiry: e.expiry.selected.value,
    expiryConflicts: e.expiry.conflicts.length,
    creations: e.creations.length,
    where,
  };
}

/// Visible entries first (the 64 the backend shows), then overflow and deleted kept apart so a
/// surface can say what it is not showing rather than hide it.
export function indexModel(c: IndexContent): IndexModel {
  const sortByCreation = (a: IndexEntryView, b: IndexEntryView) => a.ts - b.ts || (a.id < b.id ? -1 : 1);
  const entries = Object.entries(c.objects).map(([id, e]) => entryView(id, e, "visible")).sort(sortByCreation);
  const overflow = Object.entries(c.overflow).map(([id, e]) => entryView(id, e, "overflow")).sort(sortByCreation);
  const deleted = Object.entries(c.deletedObjects).map(([id, e]) => entryView(id, e, "deleted")).sort(sortByCreation);
  return { entries, overflow, deleted };
}

/// Operation-body expiry from the view's discriminated object: omitted = unrecorded, null = never,
/// integer = at. Zero is a timestamp.
export function expiryForBody(e: NativeExpiry): { expiry?: number | null } {
  if (e.kind === "unrecorded") return {};
  if (e.kind === "never") return { expiry: null };
  return { expiry: e.ms };
}
