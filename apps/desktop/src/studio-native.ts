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
import { PIX_MAX_BYTES } from "./studio-contract.ts";

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
export type FlipnoteContent = {
  kind: "flipnote";
  title: Register<string, FrameSource> | null;
  fps: Register<number, FrameSource> | null;
  timeline: Hex32[];
  frames: Record<Hex32, FrameEntry>;
  declaredFrameBytes: number;
  overCap: Record<Hex32, FrameLimits>;
  tombstones: Record<Hex32, FrameSource[]>;
};
export type StudioContent = IndexContent | FlipnoteContent;

export type StudioPhase = "open" | "closing" | "settled" | "fault";
export type StudioTrust =
  | { awaitingTenureReceipt: true; phase: null; publication: null }
  | { awaitingTenureReceipt: false; phase: StudioPhase; publication: "local" };
export type StudioView<C extends StudioContent = StudioContent> = {
  v: 1;
  epochId: Hex32;
  epoch: Decimal;
  channel: Decimal;
  provisional: true;
  content: C;
} & StudioTrust;
export type IndexView = StudioView<IndexContent>;
export type FlipnoteView = StudioView<FlipnoteContent>;

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
export type RecoveryVersionRead = { v: 1; kind: "recoveryVersion"; historical: true; version: RecoveryVersion; channel: Decimal; content: StudioContent };
export type RecoveryExport = { v: 1; kind: "recoveryExport"; snapshot: Hex64; format: "p1-recovery-v1"; bytes: number; bytesB64: string };
export type RecoveryMode = "restore" | "copy";
export type RecoveryChoice =
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

// --- Events --------------------------------------------------------------------------------------

export type StudioUpdatedEvent = { server: number; channel: Decimal; object: Hex32 | null };
export type StudioReceivePausedEvent = { server: number };
export type SettlementState = "open" | "closing" | "settled" | "fault" | "recoveryAvailable" | "recoveryEvictionPending" | "refreshRequired";
export type SettlementChangedEvent = { server: number; docType: 15 | 16; logicalKey: Hex32; channel: Decimal; object: Hex32 | null; state: SettlementState };

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
  };
}

function parseContent(v: unknown, what: string): StudioContent {
  const r = record(v, what);
  if (r.kind === "index") return parseIndexContent(r, what);
  if (r.kind === "flipnote") return parseFlipnoteContent(r, what);
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
  return { v: 1, kind: "recoveryVersion", historical: true, version: recoveryVersion(r.version, "version.version"), channel: decimal(r.channel, "version.channel"), content: parseContent(r.content, "version.content") };
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

export function parseStudioUpdated(v: unknown): StudioUpdatedEvent {
  const r = record(v, "studio-updated");
  return { server: count(r.server, "event.server"), channel: decimal(r.channel, "event.channel"), object: nullable(r.object, (o) => hex32(o, "event.object")) };
}
export function parseReceivePaused(v: unknown): StudioReceivePausedEvent {
  return { server: count(record(v, "studio-receive-paused").server, "event.server") };
}
export function parseSettlementChanged(v: unknown): SettlementChangedEvent {
  const r = record(v, "settlement-changed");
  const s = r.state;
  if (s !== "open" && s !== "closing" && s !== "settled" && s !== "fault" && s !== "recoveryAvailable" && s !== "recoveryEvictionPending" && s !== "refreshRequired") fail("event.state is unknown");
  if (r.docType !== 15 && r.docType !== 16) fail("event.docType is unknown");
  return {
    server: count(r.server, "event.server"),
    docType: r.docType,
    logicalKey: hex32(r.logicalKey, "event.logicalKey"),
    channel: decimal(r.channel, "event.channel"),
    object: nullable(r.object, (o) => hex32(o, "event.object")),
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

export async function studioRead(ipc: StudioIpc, target: StudioTargetArgs, object: Hex32): Promise<FlipnoteView | null> {
  const raw = await ipc.invoke<unknown>("studio_read", { server: target.server, channel: target.channel, object });
  return raw === null || raw === undefined ? null : parseFlipnoteView(raw);
}

export type CreateRequest = { object: Hex32; nonce: Hex32; title: string; createdAtMs: number };
export async function studioCreate(ipc: StudioIpc, target: StudioTargetArgs, req: CreateRequest): Promise<FlipnoteView> {
  if (!Number.isSafeInteger(req.createdAtMs) || req.createdAtMs < 0) fail("createdAtMs must be a non-negative safe integer");
  const raw = await ipc.invoke<unknown>("studio_create", { server: target.server, channel: target.channel, object: req.object, nonce: req.nonce, title: req.title, createdAtMs: req.createdAtMs });
  if (raw === null || raw === undefined) fail("studio_create returned nothing");
  return parseFlipnoteView(raw);
}

export type ApplyRequest = { object: Hex32; epochId: Hex32; nonce: Hex32; body: string };
export async function studioApply(ipc: StudioIpc, target: StudioTargetArgs, req: ApplyRequest): Promise<FlipnoteView> {
  const raw = await ipc.invoke<unknown>("studio_apply", { server: target.server, channel: target.channel, object: req.object, epochId: req.epochId, nonce: req.nonce, body: req.body });
  if (raw === null || raw === undefined) fail("studio_apply returned nothing");
  return parseFlipnoteView(raw);
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

export type RecoveryTargetArgs = StudioTargetArgs & { object: Hex32 | null };
function recoveryArgs(t: RecoveryTargetArgs): Record<string, unknown> {
  return t.object === null ? { server: t.server, channel: t.channel } : { server: t.server, channel: t.channel, object: t.object };
}

export async function recoveryList(ipc: StudioIpc, t: RecoveryTargetArgs): Promise<RecoveryListing> {
  return parseRecoveryListing(await ipc.invoke<unknown>("studio_recovery_list", recoveryArgs(t)));
}
export async function recoveryRead(ipc: StudioIpc, t: RecoveryTargetArgs, snapshot: Hex64): Promise<RecoveryVersionRead> {
  return parseRecoveryVersionRead(await ipc.invoke<unknown>("studio_recovery_read", { ...recoveryArgs(t), snapshot }));
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
};

export const DEFAULT_TITLE = "";
export const DEFAULT_FPS = 12;

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
