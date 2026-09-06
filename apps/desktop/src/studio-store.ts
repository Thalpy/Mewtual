// The studio's in-memory projection: every StudioIndex and StudioObject document the client
// would materialize from P1, kept as plain data and edited only through the closed domain
// operation set (design-creative-suite.md 2.9). Each edit becomes a `DomainOpEnvelope` in `ops`,
// which is exactly what the transport will send once Studio save/load is wired; until then the
// projection is the only copy, and this class is the boundary.
//
// What is deliberately NOT here: blob publication (`publish_pix`), bounded fetch, claim frames on
// the draw channel, settlement and recovery actions, export. Their inputs and outputs exist as
// fixtures so the surface can render every state; the verbs are stubs that say so.

import { encodePix, localCid } from "./pix.ts";
import {
  CLAIM_TTL_MS,
  DOC_TYPE_STUDIO_INDEX,
  DOC_TYPE_STUDIO_OBJECT,
  FLIPNOTE_FPS_MAX,
  FLIPNOTE_FPS_MIN,
  FLIPNOTE_FRAME_BYTES_PROMISE,
  FLIPNOTE_H,
  FLIPNOTE_MAX_FRAMES,
  FLIPNOTE_MAX_SFX,
  FLIPNOTE_W,
  MAX_DOMAIN_OP_BYTES,
  PIX_MAX_BYTES,
  STUDIO_INDEX_MAX_OBJECTS,
  canonicalJson,
  randomElementId,
  type DomainOpEnvelope,
  type FlipnoteOp,
  type FlipnoteRoot,
  type FrameClaim,
  type FrameRecord,
  type IndexOp,
  type LogicalDocument,
  type PixPaletteEntry,
  type RecoveryView,
  type Settlement,
  type StudioIndexRoot,
} from "./studio-contract.ts";

export class StudioError extends Error {
  readonly reason: string;
  constructor(reason: string) {
    super(`studio: ${reason}`);
    this.reason = reason;
  }
}

/// A frame's transient state on this device, outside the document (2.10, P1 section 1).
export type FrameState = "held" | "fetching" | "replaying";
/// "another version by X": a concurrent replacement of one frame, shown with both authors.
export type FrameConflict = { frame: string; mine: FrameRecord; theirs: FrameRecord; by: string };

export type Caps = { frames: number; frameBytes: number; sfx: number; objects: number };
const DEFAULT_CAPS: Caps = {
  frames: FLIPNOTE_MAX_FRAMES,
  frameBytes: FLIPNOTE_FRAME_BYTES_PROMISE,
  sfx: FLIPNOTE_MAX_SFX,
  objects: STUDIO_INDEX_MAX_OBJECTS,
};

/// The authored palette every new flipnote starts from. Entry 0 is the paper and entry 1 the
/// ink, both literal, so a drawing reads as a drawing under every theme. Then the four role
/// entries (bg, fg, accent, muted) that follow the viewer's theme when they opt in, with
/// Nightshade's values as the fallback, and ten literal colours that never move. The fixed
/// tones (t0..t3) are roles a palette may carry, not ones it must.
export const DEFAULT_PALETTE: readonly PixPaletteEntry[] = [
  { role: 0, r: 0xf2, g: 0xec, b: 0xdf }, // 0 paper
  { role: 0, r: 0x2a, g: 0x26, b: 0x33 }, // 1 ink
  { role: 1, r: 0x13, g: 0x12, b: 0x18 }, // 2 bg role
  { role: 2, r: 0xe8, g: 0xe6, b: 0xf0 }, // 3 fg role
  { role: 3, r: 0x97, g: 0x7d, b: 0xf2 }, // 4 accent role
  { role: 4, r: 0x8f, g: 0x8b, b: 0xa3 }, // 5 muted role
  { role: 0, r: 0x00, g: 0x00, b: 0x00 }, // 6 black
  { role: 0, r: 0xff, g: 0xff, b: 0xff }, // 7 white
  { role: 0, r: 0xe0, g: 0x7a, b: 0xb8 }, // 8 pink
  { role: 0, r: 0xd8, g: 0xa6, b: 0x57 }, // 9 gold
  { role: 0, r: 0xe0, g: 0x57, b: 0x4b }, // 10 red
  { role: 0, r: 0xe8, g: 0x8a, b: 0x3a }, // 11 orange
  { role: 0, r: 0xf2, g: 0xe2, b: 0x7a }, // 12 yellow
  { role: 0, r: 0x5e, g: 0xc9, b: 0x6e }, // 13 green
  { role: 0, r: 0x4f, g: 0xc1, b: 0xb8 }, // 14 teal
  { role: 0, r: 0x6c, g: 0xa0, b: 0xd8 }, // 15 blue
];
export const PALETTE_LABELS: readonly string[] = ["pa", "in", "bg", "fg", "ac", "mu", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15"];

export class StudioStore {
  readonly serverId: string;
  readonly channel: string;
  /// This device's full identity, lowercase hex (2.18): the author of every local op.
  readonly me: string;
  readonly now: () => number;
  readonly caps: Caps;
  readonly index: StudioIndexRoot;
  readonly objects = new Map<string, FlipnoteRoot>();
  /// Local stand-in for the sealed blob store: cid -> pix bytes held on this device.
  readonly blobs = new Map<string, Uint8Array>();
  /// Every domain operation this device produced, in order. The transport's outbox.
  readonly ops: DomainOpEnvelope[] = [];
  readonly settlement = new Map<string, Settlement>();
  readonly recovery = new Map<string, RecoveryView>();
  readonly claims = new Map<string, FrameClaim>();
  readonly conflicts = new Map<string, FrameConflict>();
  readonly frameState = new Map<string, FrameState>();
  /// Element ids whose tombstone wins over any later insertion (P1 generic rule).
  private readonly tombstones = new Set<string>();
  private readonly newId: () => string;

  constructor(opts: { serverId: string; channel: string; me: string; now?: () => number; caps?: Partial<Caps>; newId?: () => string }) {
    this.serverId = opts.serverId;
    this.channel = opts.channel;
    this.me = opts.me;
    this.now = opts.now ?? (() => Date.now());
    this.caps = { ...DEFAULT_CAPS, ...(opts.caps ?? {}) };
    this.newId = opts.newId ?? (() => randomElementId());
    this.index = { v: 1, kind: "index", channel: opts.channel, epoch: 0, objects: {} };
  }

  // --- Logical names, as the backend derives document ids from them -------------------------

  indexDocument(): LogicalDocument {
    return { serverId: this.serverId, docType: DOC_TYPE_STUDIO_INDEX, logicalKey: this.channel };
  }

  objectDocument(objectId: string): LogicalDocument {
    return { serverId: this.serverId, docType: DOC_TYPE_STUDIO_OBJECT, logicalKey: objectId };
  }

  // --- Blobs ---------------------------------------------------------------------------------

  /// Stage-promote-publish stand-in (2.10): keep the bytes, name them. Rejects over the cap the
  /// way `publish_pix` would refuse before promotion.
  holdBlob(bytes: Uint8Array): { cid: string; bytes: number } {
    if (bytes.length > PIX_MAX_BYTES) throw new StudioError("pix over 64 KiB");
    const cid = localCid(bytes);
    if (!this.blobs.has(cid)) this.blobs.set(cid, bytes);
    return { cid, bytes: bytes.length };
  }

  // --- Index ---------------------------------------------------------------------------------

  createFlipnote(title: string, fps = 12, w = FLIPNOTE_W, h = FLIPNOTE_H, expiry = 0): string {
    const live = Object.values(this.index.objects).filter((o) => !o.deleted).length;
    if (live >= this.caps.objects) throw new StudioError("channel studio full");
    const id = this.newId();
    this.applyIndex({ op: "put_object", object: id, kind: "flipnote", title, created_by: this.me, ts: this.now(), expiry });
    this.objects.set(id, {
      v: 1, kind: "flipnote", id, channel: this.channel, epoch: 0, title, fps, w, h,
      frames: [], frame: {}, sfx: {}, patches: {}, exports: {},
    });
    // No settlement entry: a new document has no receipt, and the surface says "new" until
    // P1 reports one. (The demo fixture sets its own.)
    this.recovery.set(id, { retained: [], staged: null, evictionDeadline: 0 });
    return id;
  }

  applyIndex(op: IndexOp): DomainOpEnvelope {
    const objs = this.index.objects;
    switch (op.op) {
      case "put_object":
        if (this.tombstones.has(op.object)) break; // a tombstone wins over any insertion
        objs[op.object] = { kind: op.kind, title: op.title, created_by: op.created_by, ts: op.ts, expiry: op.expiry };
        break;
      case "tombstone_object":
        this.tombstones.add(op.object);
        if (objs[op.object]) objs[op.object].deleted = true;
        break;
      case "set_title":
        if (objs[op.object]) objs[op.object].title = op.title;
        if (this.objects.has(op.object)) this.objects.get(op.object)!.title = op.title;
        break;
      case "set_expiry":
        if (objs[op.object]) objs[op.object].expiry = op.expiry;
        break;
    }
    return this.record(this.indexDocument(), op);
  }

  // --- Flipnote domain operations ------------------------------------------------------------

  root(objectId: string): FlipnoteRoot {
    const r = this.objects.get(objectId);
    if (!r) throw new StudioError("no such object");
    return r;
  }

  /// Apply one operation to the projection and record its envelope. Validation is the editor's
  /// own preflight (caps, ranges); P1's projection preflight repeats it on every peer.
  apply(objectId: string, op: FlipnoteOp): DomainOpEnvelope {
    const root = this.root(objectId);
    if (this.overCap(root).size && op.op !== "remove_frame" && op.op !== "set_header") {
      throw new StudioError("document full"); // editing is refused while over-cap frames exist
    }
    switch (op.op) {
      case "insert_frame": {
        if (this.tombstones.has(op.frame)) break;
        if (root.frame[op.frame]) break; // idempotent
        if (root.frames.length >= this.caps.frames) throw new StudioError("document full");
        root.frame[op.frame] = { cid: op.cid, bytes: op.bytes, author: this.me, ts: this.now() };
        const at = op.after === null ? -1 : root.frames.indexOf(op.after);
        // After its recorded predecessor if present, else at the end (2.9 restore refinement).
        if (op.after !== null && at < 0) root.frames.push(op.frame);
        else root.frames.splice(at + 1, 0, op.frame);
        break;
      }
      case "remove_frame":
        this.tombstones.add(op.frame);
        root.frames = root.frames.filter((f) => f !== op.frame);
        delete root.frame[op.frame];
        for (const [sid, s] of Object.entries(root.sfx)) if (s.fr === op.frame) delete root.sfx[sid];
        this.claims.delete(op.frame);
        this.conflicts.delete(op.frame);
        break;
      case "replace_frame": {
        const rec = root.frame[op.frame];
        if (!rec) throw new StudioError("no such frame");
        root.frame[op.frame] = { cid: op.cid, bytes: op.bytes, author: this.me, ts: this.now() };
        break;
      }
      case "set_sfx":
        if (!root.frame[op.frame]) throw new StudioError("no such frame");
        if (!root.sfx[op.sfx] && Object.keys(root.sfx).length >= this.caps.sfx) throw new StudioError("too many sfx");
        if (!Number.isInteger(op.note) || op.note < 0 || op.note > 127) throw new StudioError("note range");
        root.sfx[op.sfx] = { fr: op.frame, p: op.patch, n: op.note };
        break;
      case "remove_sfx":
        delete root.sfx[op.sfx];
        break;
      case "set_patch":
        root.patches[op.patch] = op.descriptor;
        break;
      case "remove_patch":
        delete root.patches[op.patch];
        break;
      case "set_export":
        root.exports[op.export] = { cid: op.cid, bytes: op.bytes, author: this.me, ts: this.now(), expiry: op.expiry };
        break;
      case "remove_export":
        if (root.exports[op.export]) root.exports[op.export].deleted = true;
        break;
      case "set_header":
        if (op.field === "fps") {
          const v = Number(op.value);
          if (!Number.isInteger(v) || v < FLIPNOTE_FPS_MIN || v > FLIPNOTE_FPS_MAX) throw new StudioError("fps range");
          root.fps = v;
        } else if (op.field === "title") {
          root.title = String(op.value ?? "");
          if (this.index.objects[objectId]) this.index.objects[objectId].title = root.title;
        } else if (op.field === "score") {
          if (op.value === null) delete root.score;
          else root.score = String(op.value);
        }
        break;
    }
    return this.record(this.objectDocument(objectId), op);
  }

  // --- Convenience for the editor ------------------------------------------------------------

  /// Insert a new frame holding these pixels after `after` (null = at the front... no: null means
  /// the list end per the op contract; pass the last frame id to append explicitly).
  insertFrame(objectId: string, after: string | null, pixBytes: Uint8Array): string {
    const { cid, bytes } = this.holdBlob(pixBytes);
    const frame = this.newId();
    this.apply(objectId, { op: "insert_frame", frame, after, cid, bytes });
    this.frameState.set(frame, "held");
    return frame;
  }

  /// Replace a frame's pixels, producing an op only when the bytes actually changed.
  replaceFrame(objectId: string, frame: string, pixBytes: Uint8Array): boolean {
    const root = this.root(objectId);
    const { cid, bytes } = this.holdBlob(pixBytes);
    if (root.frame[frame]?.cid === cid) return false;
    this.apply(objectId, { op: "replace_frame", frame, cid, bytes });
    this.frameState.set(frame, "held");
    return true;
  }

  frameBytes(objectId: string, frame: string): Uint8Array | undefined {
    const rec = this.root(objectId).frame[frame];
    return rec ? this.blobs.get(rec.cid) : undefined;
  }

  /// Frames past the list cap or the 8 MiB promise, summed over declared bytes in list order:
  /// they grey out, playback skips them, and editing is refused while any exist (2.10).
  overCap(root: FlipnoteRoot): Set<string> {
    const out = new Set<string>();
    let sum = 0;
    root.frames.forEach((f, i) => {
      sum += root.frame[f]?.bytes ?? 0;
      if (i >= this.caps.frames || sum > this.caps.frameBytes) out.add(f);
    });
    return out;
  }

  frameBytesTotal(root: FlipnoteRoot): number {
    return root.frames.reduce((n, f) => n + (root.frame[f]?.bytes ?? 0), 0);
  }

  // --- Claims: ephemeral, advisory, 90 s from last receipt -----------------------------------

  claim(frame: string, by: string, ask = false): void {
    this.claims.set(frame, { frame, by, ask, seenTs: this.now() });
  }

  /// The live claim on a frame, if its last receipt is inside the TTL.
  claimOn(frame: string): FrameClaim | null {
    const c = this.claims.get(frame);
    if (!c) return null;
    if (this.now() - c.seenTs > CLAIM_TTL_MS) { this.claims.delete(frame); return null; }
    return c;
  }

  /// Renew this device's claim (the 30 s resend the surface schedules while editing).
  renewClaim(frame: string): void {
    const c = this.claims.get(frame);
    if (c && c.by === this.me) c.seenTs = this.now();
  }

  /// Pass: hand the claim to whoever asked. Nothing on the wire yet; the surface reflects it.
  passClaim(frame: string, to: string): void {
    this.claims.set(frame, { frame, by: to, ask: false, seenTs: this.now() });
  }

  releaseClaim(frame: string): void {
    const c = this.claims.get(frame);
    if (c && c.by === this.me) this.claims.delete(frame);
  }

  /// Secondsleft on a claim's TTL, for the countdown.
  claimSecondsLeft(frame: string): number {
    const c = this.claimOn(frame);
    return c ? Math.max(0, Math.ceil((CLAIM_TTL_MS - (this.now() - c.seenTs)) / 1000)) : 0;
  }

  // --- Boundary stubs: shared behaviour that is not connected yet ----------------------------

  /// Where Restore/Copy/Export will go (P1 recovery actions). Until the bridge carries them,
  /// the surface offers the buttons and this says why nothing happens.
  recoveryAction(_objectId: string, action: "restore" | "copy" | "export"): string {
    return `${action}: recovery actions are not connected to the epoch store yet`;
  }

  exportPixa(_objectId: string): string {
    return "export: .pixa export is not connected yet";
  }

  /// Posting a frame into the channel is a chat doodle attachment `{cid, bytes, expiry}` on a
  /// message (2.11, C3b). The pixels are ready here; the send path is not connected yet.
  postToChat(objectId: string, frame: string): string {
    const rec = this.root(objectId).frame[frame];
    if (!rec) return "post to chat: no frame open";
    return `post to chat: frame ${this.root(objectId).frames.indexOf(frame) + 1} (${Math.round(rec.bytes / 102.4) / 10} KiB) would attach to a message here; the doodle attachment path is not connected yet`;
  }

  // --- Internals -----------------------------------------------------------------------------

  private record(doc: LogicalDocument, op: FlipnoteOp | IndexOp): DomainOpEnvelope {
    const body = canonicalJson(op);
    if (new TextEncoder().encode(body).length > MAX_DOMAIN_OP_BYTES) throw new StudioError("operation over 64 KiB");
    const env: DomainOpEnvelope = { nonce: this.newId(), docType: doc.docType, logicalKey: doc.logicalKey, body };
    this.ops.push(env);
    return env;
  }
}

// --- Fixtures: the cat scene the mockups use, in every state the surface has to render ----------

const SCENE_W = 48, SCENE_H = 36, CELL = 4;
/// Scene marks to palette index: '.' paper, F ink (the cat, the moon), A accent (eyes), M muted
/// (cloud); '0' ground (muted), '1' the ground line (ink), '2' grass (green), '3' stars (gold);
/// P pink, G gold.
const ROLE_INDEX: Record<string, number> = { ".": 0, F: 1, A: 4, M: 5, "0": 5, "1": 1, "2": 13, "3": 9, P: 8, G: 9 };

function sceneCells(tail: "A" | "B", moonShift: number): string[][] {
  const g: string[][] = Array.from({ length: SCENE_H }, () => Array(SCENE_W).fill("."));
  const put = (r: number, c: number, s: string) => { for (let i = 0; i < s.length; i++) if (c + i < SCENE_W) g[r][c + i] = s[i]; };
  const P = (list: [number, number, string][]) => list.forEach(([r, c, s]) => put(r, c, s));
  P([[1, 10, "3"], [2, 32, "3"], [5, 3, "3"], [2, 6, "G"], [8, 44, "3"], [12, 40, "3"], [16, 4, "3"]]);
  const m = moonShift;
  P([[3, 36 + m, "FFF"], [4, 35 + m, "FFFFF"], [5, 34 + m, "FFF"], [6, 34 + m, "FF"], [7, 34 + m, "FF"], [8, 34 + m, "FFF"], [9, 35 + m, "FFFFF"], [10, 36 + m, "FFF"]]);
  P([[6, 4, "MMMM"], [7, 3, "MMMMMM"], [8, 5, "MMM"]]);
  for (let c = 0; c < SCENE_W; c++) g[29][c] = "1";
  for (let r = 30; r < SCENE_H; r++) for (let c = 0; c < SCENE_W; c++) g[r][c] = "0";
  P([[30, 8, "2"], [30, 27, "2"], [30, 41, "2"], [31, 20, "2"], [32, 36, "2"]]);
  P([[11, 14, "F"], [11, 25, "F"], [12, 14, "FF"], [12, 24, "FF"], [13, 14, "FFF"], [13, 23, "FFF"], [14, 14, "FFFF"], [14, 22, "FFFF"],
    [15, 14, "FFFFFFFFFFFF"], [16, 14, "FFFFFFFFFFFF"], [17, 14, "FFAAFFFFAAFF"], [18, 14, "FFAAFFFFAAFF"], [19, 14, "FFFFFFPPFFFF"],
    [20, 15, "FFFFFFFFFF"], [21, 16, "FFFFFFFF"]]);
  P([[22, 15, "FFFFFFFFFF"], [23, 14, "FFFFFFFFFFFF"], [24, 13, "FFFFFFFFFFFFFF"], [25, 13, "FFFFFFFFFFFFFF"], [26, 13, "FFFFFFFFFFFFFF"],
    [27, 13, "FFFFFFFFFFFFFFF"], [28, 13, "FFFFFFFFFFFFFFF"], [29, 13, "FFFF"], [29, 19, "FFFF"]]);
  if (tail === "A") P([[23, 34, "F"], [24, 33, "FF"], [25, 32, "FF"], [26, 31, "FF"], [27, 29, "FFF"], [28, 28, "FFF"]]);
  else P([[27, 33, "FF"], [28, 28, "FFFFFFF"]]);
  return g;
}

/// One 192x144 frame of the cat scene as pix:v1 bytes under the default palette.
export function sceneFrame(tail: "A" | "B", moonShift = 0): Uint8Array {
  const g = sceneCells(tail, moonShift);
  const pixels = new Uint8Array(FLIPNOTE_W * FLIPNOTE_H);
  for (let y = 0; y < FLIPNOTE_H; y++) for (let x = 0; x < FLIPNOTE_W; x++) {
    pixels[y * FLIPNOTE_W + x] = ROLE_INDEX[g[Math.floor(y / CELL)][Math.floor(x / CELL)]] ?? 0;
  }
  return encodePix({ w: FLIPNOTE_W, h: FLIPNOTE_H, palette: DEFAULT_PALETTE.map((e) => ({ ...e })), pixels });
}

export type DemoPeople = { me: string; rook: string; mika: string; wren: string; owner: string };

/// The channel studio the mockups show: "moon cat" (24 frames, settled, a claim, an ask, a
/// conflict, two fetching frames, one replaying), "rook's loop" (rotating, eviction warning),
/// and "lullaby" (a score entry in the index, no editor yet).
export function demoStudio(people: DemoPeople, now: () => number = () => Date.now()): { store: StudioStore; moonCat: string; rooksLoop: string; lullaby: string } {
  let seq = 0;
  const newId = () => (++seq).toString(16).padStart(32, "0");
  const store = new StudioStore({ serverId: "a".repeat(32), channel: "art", me: people.me, now, newId });
  const t0 = now();

  const moonCat = store.createFlipnote("moon cat", 12);
  let last: string | null = null;
  const authors = [people.me, people.me, people.mika, people.mika, people.me, people.me, people.me, people.rook];
  for (let i = 0; i < 24; i++) {
    const bytes = sceneFrame(i % 2 === 0 ? "A" : "B", Math.floor(i / 6) % 2);
    last = store.insertFrame(moonCat, last, bytes);
    const rec = store.root(moonCat).frame[last];
    rec.author = authors[i % authors.length];
    rec.ts = t0 - (24 - i) * 60_000;
  }
  const frames = store.root(moonCat).frames;
  // Frame 12 (index 11) is ours and claimed; mika asked for it.
  store.claim(frames[11], people.me);
  store.claims.get(frames[11])!.ask = true;
  // Frame 15 is rook's claim, a little way into its TTL.
  store.claim(frames[14], people.rook);
  store.claims.get(frames[14])!.seenTs = t0 - 48_000;
  // Frame 9: another version by mika (concurrent replace).
  const f9 = frames[8];
  store.conflicts.set(f9, {
    frame: f9,
    mine: store.root(moonCat).frame[f9],
    theirs: { cid: localCid(sceneFrame("B", 1)), bytes: 4400, author: people.mika, ts: t0 - 30_000 },
    by: people.mika,
  });
  // Frames 17 and 18: records arrived, blobs not yet fetched. Their cids name bytes this device
  // does not hold (blobs dedupe by content, so the scene frames' own cids must stay held).
  [frames[16], frames[17]].forEach((f, i) => {
    const rec = store.root(moonCat).frame[f];
    rec.cid = (i === 0 ? "f0" : "f1").padEnd(64, "0");
    rec.author = people.rook;
    store.frameState.set(f, "fetching");
  });
  // Frame 13: our edit, excluded by a checkpoint, replaying from its intent.
  store.frameState.set(frames[12], "replaying");
  // Sfx on frames 8, 12 and 16; a linked score.
  store.apply(moonCat, { op: "set_patch", patch: "meow", descriptor: { v: 1, name: "meow" } });
  store.apply(moonCat, { op: "set_sfx", sfx: newId(), frame: frames[7], patch: "meow", note: 79 });
  store.apply(moonCat, { op: "set_sfx", sfx: newId(), frame: frames[11], patch: "meow", note: 72 });
  store.apply(moonCat, { op: "set_sfx", sfx: newId(), frame: frames[15], patch: "meow", note: 36 });
  store.apply(moonCat, { op: "set_export", export: newId(), cid: "e".repeat(64), bytes: 1_258_291, expiry: t0 + 21 * 86_400_000 });
  store.settlement.set(moonCat, { gate: "settled", epoch: 3, label: "settled", receiptBy: people.owner, receiptTs: t0 - 2 * 3_600_000 });
  store.recovery.set(moonCat, {
    retained: [
      { id: "r2".padEnd(64, "0"), epoch: 2, closedTs: t0 - 2 * 86_400_000, bytes: 39_000 },
      { id: "r1".padEnd(64, "0"), epoch: 1, closedTs: t0 - 9 * 86_400_000, bytes: 21_000 },
    ],
    staged: null,
    evictionDeadline: 0,
  });

  const rooksLoop = store.createFlipnote("rook's loop", 8);
  last = null;
  for (let i = 0; i < 8; i++) last = store.insertFrame(rooksLoop, last, sceneFrame(i % 2 === 0 ? "B" : "A"));
  for (const f of store.root(rooksLoop).frames) store.root(rooksLoop).frame[f].author = people.rook;
  store.index.objects[rooksLoop].created_by = people.rook;
  store.settlement.set(rooksLoop, { gate: "closing", epoch: 1, label: "rotating", receiptBy: people.owner, receiptTs: t0 - 86_400_000 });
  store.recovery.set(rooksLoop, {
    retained: [
      { id: "s2".padEnd(64, "0"), epoch: 1, closedTs: t0 - 86_400_000, bytes: 9_000 },
      { id: "s1".padEnd(64, "0"), epoch: 0, closedTs: t0 - 12 * 86_400_000, bytes: 8_000 },
    ],
    staged: { id: "s3".padEnd(64, "0"), epoch: 2, closedTs: t0 - 3_600_000, bytes: 9_200 },
    evictionDeadline: t0 + 5 * 86_400_000,
  });

  const lullaby = newId();
  store.applyIndex({ op: "put_object", object: lullaby, kind: "score", title: "lullaby", created_by: people.mika, ts: t0 - 3 * 86_400_000, expiry: 0 });
  store.apply(moonCat, { op: "set_header", field: "score", value: lullaby });
  store.settlement.set(lullaby, { gate: "open", epoch: 0, label: "current owner has not yet confirmed this document's history", receiptBy: "", receiptTs: 0 });

  // The demo's own edits are not real history: start the outbox clean.
  store.ops.length = 0;
  return { store, moonCat, rooksLoop, lullaby };
}
