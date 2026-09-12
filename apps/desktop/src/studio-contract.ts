// The creative-suite contract as the desktop client sees it: the names, tags and caps the
// backend already defines (catcoms-wire `DocType`, catcoms-replication `epoch.rs`, the P1 storage
// recovery record) and the studio document shapes from docs/design-creative-suite.md (2.1, 2.9,
// 2.10). Constants and types only: this file is the single source for every limit the studio UI
// enforces, the same way jam-contract.ts is for the jam layer.
//
// Nothing here talks to the bridge. The editor produces domain operations against an in-memory
// projection (studio-store.ts); when Studio save/load lands, the same operations go out through
// `EncryptedDoc::edit_domain_gated` and the projection is replaced by the materialized document.

// --- Document tags (catcoms-wire DocType; stable, append-only) --------------------------------
export const DOC_TYPE_STUDIO_INDEX = 15;
export const DOC_TYPE_STUDIO_OBJECT = 16;
export const DOC_TYPE_POST_REPLIES = 17;
export const DOC_TYPE_DOC_REGISTRY = 18;
export type StudioDocType =
  | typeof DOC_TYPE_STUDIO_INDEX
  | typeof DOC_TYPE_STUDIO_OBJECT
  | typeof DOC_TYPE_POST_REPLIES;

/// `catcoms_replication::epoch::LogicalDocument`, with the server id and key as hex strings the
/// bridge would carry. Epoch document ids derive from this stable name; the UI never sees them.
export type LogicalDocument = { serverId: string; docType: StudioDocType; logicalKey: string };

// --- Replication bounds (catcoms-replication epoch.rs, mirrored by name) ------------------------
export const MAX_DOMAIN_OP_BYTES = 64 * 1024;
export const MAX_RECOVERY_SNAPSHOT_BYTES = 6 * 1024 * 1024;
export const RECOVERY_GRACE_MS = 7 * 24 * 60 * 60 * 1000;
export const RECOVERY_RETAINED_SLOTS = 2; // two retained versions plus one staged (RecoverySlots)

/// `EpochGate`: the persisted, server/document-bound settlement boundary of one epoch.
export type GateState = "open" | "closing" | "settled" | "fault";

/// What the Studio surface says about a document, driven by P1's settlement event (2.9). The
/// `label` strings are the ones the design fixes; the UI renders them verbatim.
export type Settlement = {
  gate: GateState;
  epoch: number;
  /// Which of the fixed labels applies, or none while everything is ordinary.
  label:
    | "settled"
    | "rotating"
    | "local edits only until the owner settles this rotation"
    | "current owner has not yet confirmed this document's history"
    | "history fault: conflicting owner receipts"
    | "document full"
    | "storage limit reached";
  /// Who issued the last verified receipt and when (ms epoch), when there is one.
  receiptBy: string;
  receiptTs: number;
};

/// `catcoms_app::store::EpochRecoveryState`, as the UI needs it: newest-first retained versions,
/// the staged one behind an eviction warning, and the warning's deadline.
export type RecoverySnapshotView = { id: string; epoch: number; closedTs: number; bytes: number };
export type RecoveryView = {
  retained: RecoverySnapshotView[];
  staged: RecoverySnapshotView | null;
  /// ms epoch by which the oldest retained version is removed unless exported; 0 = no warning.
  evictionDeadline: number;
};

// --- pix:v1 (2.1) ------------------------------------------------------------------------------
export const PIX_MAGIC = "PIX1";
export const PIX_MAX_BYTES = 64 * 1024; // on the encoded length, checked before any decode
export const PIX_MAX_SIDE = 256;
export const PIX_MAX_PIXELS = 65536;
export const PIX_MIN_PALETTE = 4;
export const PIX_MAX_PALETTE = 16;
export const PIX_MAX_RUN = 256;

/// Palette roles. Every entry also carries an RGB fallback; role entries may be recoloured by a
/// viewer who opted into "adapt drawings to my theme".
export const PIX_ROLE_LITERAL = 0;
export const PIX_ROLE_BG = 1;
export const PIX_ROLE_FG = 2;
export const PIX_ROLE_ACCENT = 3;
export const PIX_ROLE_MUTED = 4;
export const PIX_ROLE_T0 = 5;
export const PIX_ROLE_T1 = 6;
export const PIX_ROLE_T2 = 7;
export const PIX_ROLE_T3 = 8;
export const PIX_ROLE_MAX = 8;
export type PixRole = 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8;
export const PIX_ROLE_NAMES: readonly string[] = ["", "bg", "fg", "acc", "mut", "t0", "t1", "t2", "t3"];

export type PixPaletteEntry = { role: PixRole; r: number; g: number; b: number };
export type PixImage = { w: number; h: number; palette: PixPaletteEntry[]; pixels: Uint8Array };

// --- Canvas modes (2.2) --------------------------------------------------------------------------
export const FLIPNOTE_W = 192;
export const FLIPNOTE_H = 144;
export const DOODLE_W = 128;
export const DOODLE_H = 96;
export const STAMP_W = 32;
export const STAMP_H = 32;
export const BRUSH_MIN = 1;
export const BRUSH_MAX = 8;
/// Editor layers are a local convenience (docs note "layers"): a frame on the wire is one flat
/// pix blob until the design says otherwise.
export const LAYER_COUNT = 3;
export const LAYER_NAMES: readonly string[] = ["bg", "mid", "fg"];
export const UNDO_DEPTH = 64;

// --- flipnote:v1 root and caps (2.10) ---------------------------------------------------------
export const FLIPNOTE_FPS_MIN = 1;
export const FLIPNOTE_FPS_MAX = 24;
export const FLIPNOTE_MAX_FRAMES = 999;
export const FLIPNOTE_FRAME_BYTES_PROMISE = 8 * 1024 * 1024; // summed over declared bytes, list order
export const FLIPNOTE_MAX_SFX = 4096;
export const FLIPNOTE_MAX_PATCHES = 64; // union of the linked score's patches and the sfx patches
export const FLIPNOTE_MAX_EXPORTS_VISIBLE = 8;
export const STUDIO_INDEX_MAX_OBJECTS = 64;
export const ELEMENT_ID_HEX = 32;

export type FrameRecord = { cid: string; bytes: number; author: string; ts: number };
export type SfxRecord = { fr: string; p: string; n: number };
export type ExportRecord = { cid: string; bytes: number; author: string; ts: number; expiry: number; deleted?: boolean };
export type FlipnoteRoot = {
  v: 1;
  kind: "flipnote";
  id: string;
  channel: string;
  epoch: number;
  title: string;
  fps: number;
  w: number;
  h: number;
  /// Ordered set of frame ids (projection of insert/remove by stable element id).
  frames: string[];
  frame: Record<string, FrameRecord>;
  score?: string;
  sfx: Record<string, SfxRecord>;
  patches: Record<string, unknown>;
  exports: Record<string, ExportRecord>;
};

export type StudioIndexEntry = {
  kind: "flipnote" | "score";
  title: string;
  created_by: string;
  ts: number;
  expiry: number;
  deleted?: boolean;
};
export type StudioIndexRoot = {
  v: 1;
  kind: "index";
  channel: string;
  epoch: number;
  objects: Record<string, StudioIndexEntry>;
};

// --- Domain operations (2.9 closed set for `flipnote`; every edit is one of these) ------------
export type FlipnoteOp =
  | { op: "insert_frame"; frame: string; after: string | null; cid: string; bytes: number }
  | { op: "remove_frame"; frame: string }
  | { op: "replace_frame"; frame: string; cid: string; bytes: number }
  | { op: "set_sfx"; sfx: string; frame: string; patch: string; note: number }
  | { op: "remove_sfx"; sfx: string }
  | { op: "set_patch"; patch: string; descriptor: unknown }
  | { op: "remove_patch"; patch: string }
  | { op: "set_export"; export: string; cid: string; bytes: number; expiry: number }
  | { op: "remove_export"; export: string }
  | { op: "set_header"; field: "title" | "fps" | "score"; value: string | number | null };

export type IndexOp =
  | { op: "put_object"; object: string; kind: "flipnote" | "score"; title: string; created_by: string; ts: number; expiry: number }
  | { op: "tombstone_object"; object: string }
  | { op: "set_title"; object: string; title: string }
  | { op: "set_expiry"; object: string; expiry: number };

/// `catcoms_replication::epoch::DomainOp`: the envelope every edit travels in. `body` is the
/// canonical JSON of a `FlipnoteOp` or `IndexOp`; the operation id is derived by every receiver
/// from (logical key, verified author, nonce) and never carried.
export type DomainOpEnvelope = {
  nonce: string; // 16 bytes as 32 lowercase hex
  docType: StudioDocType;
  logicalKey: string;
  body: string;
};

// --- Claims (2.10): ephemeral, on the draw channel, advisory ---------------------------------
export const CLAIM_TTL_MS = 90_000; // live 90 s from the receiver's last receipt
export const CLAIM_RESEND_MS = 30_000; // re-sent every 30 s while editing
export type FrameClaim = { frame: string; by: string; ask: boolean; seenTs: number };

/// Canonical JSON as the jam patch validator already defines it: keys sorted by UTF-16 code
/// unit, no whitespace, integers only. Strings are escaped by JSON.stringify, which emits the
/// shortest RFC 8259 form for everything but a handful of control characters it escapes as \uXXXX,
/// which the validator accepts.
export function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== "object") {
    if (typeof value === "number" && !Number.isInteger(value)) throw new Error("canonical JSON carries integers only");
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  const obj = value as Record<string, unknown>;
  const keys = Object.keys(obj).filter((k) => obj[k] !== undefined).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalJson(obj[k])}`).join(",")}}`;
}

/// 32 lowercase hex from the sanctioned RNG; the same shape as every studio element id.
export function randomElementId(rng: (n: number) => Uint8Array = cryptoBytes): string {
  return hex(rng(ELEMENT_ID_HEX / 2));
}

export function cryptoBytes(n: number): Uint8Array {
  const out = new Uint8Array(n);
  crypto.getRandomValues(out);
  return out;
}

export function hex(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += b.toString(16).padStart(2, "0");
  return s;
}
