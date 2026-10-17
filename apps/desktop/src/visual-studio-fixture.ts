// Native-shaped, read-only Studio data for the localhost development visual fixture. These are
// explicit mocked responses, never a fallback when a real native command fails. Fixture CIDs
// are stable lookup keys, not claims that this renderer published encrypted content.
import { encodePix } from "./pix.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { FLIPNOTE_H, FLIPNOTE_W, type PixPaletteEntry, type ScoreCell } from "./studio-contract.ts";
import {
  bytesToBase64, parseFlipnoteView, parseIndexView, parseScoreView,
  type FlipnoteContent, type IndexContent, type IndexSource, type Register, type ScoreContent, type ScoreSlot,
} from "./studio-native.ts";

export const VISUAL_STUDIO_CHANNEL = "1";
export const VISUAL_STUDIO_GROUP = "deadbeef";
export const VISUAL_STUDIO_ME = "a4f29c110b7d8365a4f29c110b7d8365".repeat(2);
export const VISUAL_STUDIO_ART = "11".repeat(16);
export const VISUAL_STUDIO_SCORE = "22".repeat(16);
export const VISUAL_STUDIO_CONFLICT_SCORE = "33".repeat(16);
export const VISUAL_STUDIO_CONFLICT_ART = "44".repeat(16);
export const VISUAL_STUDIO_PATCH = "6ef42672dfee8b336c82d5466a84dceaa6dd18f405e5a4710ea24c9db47eaf6c";
const epochId = "ee".repeat(16);
const id32 = (n: number) => n.toString(16).padStart(32, "0");
const id64 = (n: number) => n.toString(16).padStart(64, "0");
const source = (n: number): IndexSource => ({ opId: id64(n), author: VISUAL_STUDIO_ME, nonce: id32(n) });
const reg = <T>(value: T, n: number, conflicts: { value: T; source: IndexSource }[] = []): Register<T, IndexSource> => ({ selected: { value, source: source(n) }, conflicts });
const slot = <T>(value: T | null, n: number, conflicts: { value: T | null; source: IndexSource }[] = []): ScoreSlot<T> => ({ removed: value === null || conflicts.some(v => v.value === null), register: reg(value, n, conflicts) });
const patch = legacyJamPatch("triangle"); // Hash is pinned by the shared score-op golden vector.
const PALETTE: PixPaletteEntry[] = [
  { role: 1, r: 19, g: 18, b: 24 }, { role: 2, r: 232, g: 230, b: 240 },
  { role: 3, r: 151, g: 125, b: 242 }, { role: 0, r: 224, g: 122, b: 184 },
];

/** A small deterministic cat scene at the real 192×144 frame size, encoded by the real codec. */
function scene(frame: number): Uint8Array {
  const pixels = new Uint8Array(FLIPNOTE_W * FLIPNOTE_H);
  const dot = (x: number, y: number, color: number) => { if (x >= 0 && x < FLIPNOTE_W && y >= 0 && y < FLIPNOTE_H) pixels[y * FLIPNOTE_W + x] = color; };
  const box = (x: number, y: number, w: number, h: number, color: number) => { for (let yy = y; yy < y + h; yy++) for (let xx = x; xx < x + w; xx++) dot(xx, yy, color); };
  for (let x = 0; x < FLIPNOTE_W; x++) { dot(x, 115, 2); dot(x, 116, 2); }
  for (let i = 0; i < 9; i++) { const x = 16 + i * 19, y = 16 + (i % 3) * 9; dot(x, y, 1); dot(x - 1, y, 1); dot(x + 1, y, 1); dot(x, y - 1, 1); dot(x, y + 1, 1); }
  const lift = frame === 1 ? -4 : 0, x = 68 + frame * 3, y = 66 + lift;
  box(x, y, 48, 39, 2); box(x - 5, y + 10, 58, 23, 2); box(x, y - 10, 12, 15, 2); box(x + 36, y - 10, 12, 15, 2);
  box(x + 5, y - 5, 4, 8, 3); box(x + 39, y - 5, 4, 8, 3); box(x + 10, y + 14, 5, frame === 2 ? 2 : 6, 0); box(x + 33, y + 14, 5, frame === 2 ? 2 : 6, 0);
  box(x + 22, y + 24, 5, 3, 3); box(x + 24, y + 27, 1, 5, 0); box(x + 19, y + 31, 11, 1, 0);
  box(x + 5, y + 37, 11, 10, 2); box(x + 33, y + 37, 11, 10, 2); box(x + 47, y + 28, 21, 5, 2); box(x + 63, y + 18, 5, 12, 2);
  return encodePix({ w: FLIPNOTE_W, h: FLIPNOTE_H, palette: PALETTE, pixels });
}
const frames = [0, 1, 2].map((frame) => ({ id: id32(100 + frame), cid: id64(100 + frame), bytes: scene(frame) }));

function scoreContent(conflicted = false): ScoreContent {
  const content: ScoreContent = {
    kind: "score", title: reg(conflicted ? "Recovery sketch" : "Moonwalk melody", 10), bpm: reg(112, 11), bpb: reg(4, 12), steps: reg(32, 13),
    cells: {}, patches: { [VISUAL_STUDIO_PATCH]: slot(patch, 14) }, readiness: { cells: {}, patches: {}, headerConflicts: false },
  };
  for (const [index, row] of [0, 4, 7, 12, 7, 4, 0, 7].entries()) content.cells[`${index * 4}:${row}`] = slot<ScoreCell>({ k: "n", p: VISUAL_STUDIO_PATCH, len: 3 }, 20 + index);
  for (let step = 0; step < 32; step += 4) content.cells[`${step}:24`] = slot<ScoreCell>({ k: "d", pad: step % 8 === 0 ? 0 : 1 }, 40 + step);
  if (conflicted) {
    content.bpm = reg(112, 70, [{ value: 126, source: source(71) }]); content.readiness.headerConflicts = true;
    content.cells["8:7"] = slot<ScoreCell>({ k: "n", p: VISUAL_STUDIO_PATCH, len: 3 }, 72, [{ value: null, source: source(73) }]);
    content.cells["255:24"] = slot<ScoreCell>({ k: "d", pad: 9 }, 74);
    // Native readiness skips effectively removed slots, even when their raw setter conflicts
    // with a live clear. Keep that evidence below without inventing an effective-cell issue.
    content.readiness.cells["255:24"] = { count: false, extent: true, recipe: false, overlap: false, conflict: false };
  }
  return content;
}

function artContent(now: number, conflicted = false): FlipnoteContent {
  const fsource = (n: number) => ({ ...source(n), ts: now - 60_000 + n });
  const freg = <T>(value: T, n: number) => ({ selected: { value, source: fsource(n) }, conflicts: [] as { value: T; source: ReturnType<typeof fsource> }[] });
  const content: FlipnoteContent = { kind: "flipnote", title: freg(conflicted ? "Sound alternatives" : "Moon cat", 1), fps: freg(8, 2), timeline: frames.map(f => f.id), frames: {}, declaredFrameBytes: frames.reduce((total, f) => total + f.bytes.length, 0), overCap: {}, tombstones: {}, sound: {
    records: {}, readiness: { sfxOverflow: [], patchOverflow: [], exportOverflow: [], missingFrames: [], missingPatches: [], conflicts: conflicted },
  } };
  for (const [index, frame] of frames.entries()) content.frames[frame.id] = {
    pixels: freg({ cid: frame.cid, bytes: frame.bytes.length }, 100 + index),
    insertions: [{ value: { checkpoint: false, after: index ? frames[index - 1].id : null, anchor: index ? id64(100 + index - 1) : null, before: null, blob: { cid: frame.cid, bytes: frame.bytes.length } }, source: fsource(100 + index) }],
  };
  content.sound!.records[`p/${VISUAL_STUDIO_PATCH}`] = freg({ op: "set_patch", patch: VISUAL_STUDIO_PATCH, descriptor: patch }, 110);
  content.sound!.records["h/score"] = freg({ op: "set_header", field: "score", value: VISUAL_STUDIO_SCORE }, 111);
  const sfx = { op: "set_sfx" as const, sfx: id32(120), frame: frames[1].id, patch: VISUAL_STUDIO_PATCH, note: 72, duration_ms: 375 };
  content.sound!.records[`s/${sfx.sfx}`] = { selected: { value: sfx, source: fsource(120) }, conflicts: conflicted ? [{ value: { ...sfx, note: 79, duration_ms: 750 }, source: fsource(121) }] : [] };
  // Retain a deleted sound alongside its setter so permanent deletion is visible, not guessed.
  const removed = { ...sfx, sfx: id32(130), frame: frames[0].id, note: 60, duration_ms: 125 };
  content.sound!.records[`s/${removed.sfx}`] = freg(removed, 130);
  content.sound!.records[`q/${removed.sfx}/${id64(131)}`] = freg({ op: "remove_sfx", sfx: removed.sfx }, 131);
  return content;
}

function indexContent(now: number): IndexContent {
  const content: IndexContent = { kind: "index", objects: {}, overflow: {}, deletedObjects: {}, tombstones: {} };
  for (const [index, [id, kind, title]] of [
    [VISUAL_STUDIO_ART, "flipnote", "Moon cat"], [VISUAL_STUDIO_SCORE, "score", "Moonwalk melody"],
    [VISUAL_STUDIO_CONFLICT_SCORE, "score", "Recovery sketch"], [VISUAL_STUDIO_CONFLICT_ART, "flipnote", "Sound alternatives"],
  ].entries()) {
    const expiry = { kind: "unrecorded" as const }, value = { kind: kind as "flipnote" | "score", title, createdBy: VISUAL_STUDIO_ME, ts: now - 120_000, expiry };
    content.objects[id] = { creations: [{ value, source: source(200 + index) }], title: reg(title, 200 + index), expiry: reg(expiry, 200 + index) };
  }
  return content;
}

function view<C>(content: C) { return { v: 1, epochId, epoch: "3", channel: VISUAL_STUDIO_CHANNEL, group: VISUAL_STUDIO_GROUP, provisional: true, awaitingTenureReceipt: false, phase: "open", publication: "local", content }; }
function scope(args: Record<string, unknown>) {
  if (args.server !== 1 || args.channel !== VISUAL_STUDIO_CHANNEL) throw new Error("Visual Studio fixture only implements server 1, decimal channel 1.");
  const object = args.object ?? null, objectKind = args.objectKind;
  if (object !== null && (typeof object !== "string" || !/^[0-9a-f]{32}$/.test(object))) throw new Error("Visual Studio fixture object id is malformed.");
  if ((objectKind !== undefined && objectKind !== "flipnote" && objectKind !== "score") || (object === null && objectKind !== undefined)) throw new Error("Visual Studio fixture object kind is invalid.");
  const knownScore = object === VISUAL_STUDIO_SCORE || object === VISUAL_STUDIO_CONFLICT_SCORE;
  const knownArt = object === VISUAL_STUDIO_ART || object === VISUAL_STUDIO_CONFLICT_ART;
  if ((knownScore && objectKind !== "score") || (knownArt && objectKind === "score")) throw new Error("Visual Studio fixture rejects a mismatched explicit object family.");
  return { object, ...(objectKind === "score" ? { objectKind } : {}) };
}

export type VisualStudioResponse = { handled: false } | { handled: true; value: unknown };
/** Exact supported read commands; all writes/unknown Studio commands fail loudly. */
export function visualStudioFixtureResponse(command: string, args: Record<string, unknown>, now: number): VisualStudioResponse {
  if (command === "get_own_device_id") return { handled: true, value: VISUAL_STUDIO_ME };
  if (command === "request_blob_bounded") {
    if (args.server !== 1 || typeof args.cid !== "string" || !/^[0-9a-f]{64}$/.test(args.cid) || typeof args.maxBytes !== "number" || !Number.isSafeInteger(args.maxBytes) || args.maxBytes < 0) throw new Error("Visual Studio fixture blob request is malformed.");
    const frame = frames.find(f => f.cid === args.cid);
    if (!frame) return { handled: true, value: null };
    if (args.maxBytes !== frame.bytes.length) throw new Error("Visual Studio fixture refuses a wrong declared blob length.");
    return { handled: true, value: { bytes_b64: bytesToBase64(frame.bytes), bytes: frame.bytes.length } };
  }
  if (["publish_pix", "publish_pixa", "export_studio_gif"].includes(command)) throw new Error(`Read-only visual Studio fixture does not simulate publication or saved files: ${command}`);
  if (!command.startsWith("studio_")) return { handled: false };
  const target = scope(args);
  switch (command) {
    case "studio_list":
      if (target.object !== null) throw new Error("Visual Studio fixture Index read cannot carry an object.");
      return { handled: true, value: parseIndexView(view(indexContent(now))) };
    case "studio_read": {
      if (target.object === null) throw new Error("Visual Studio fixture object read requires an object.");
      if (target.object === VISUAL_STUDIO_SCORE || target.object === VISUAL_STUDIO_CONFLICT_SCORE) return { handled: true, value: parseScoreView(view(scoreContent(target.object === VISUAL_STUDIO_CONFLICT_SCORE))) };
      if (target.object === VISUAL_STUDIO_ART || target.object === VISUAL_STUDIO_CONFLICT_ART) return { handled: true, value: parseFlipnoteView(view(artContent(now, target.object === VISUAL_STUDIO_CONFLICT_ART))) };
      return { handled: true, value: null };
    }
    case "studio_overlay_read": return { handled: true, value: { v: 1, kind: "absent", channel: VISUAL_STUDIO_CHANNEL, ...target } };
    case "studio_overlay_lifecycle": return { handled: true, value: { v: 1, kind: "overlayLifecycle", channel: VISUAL_STUDIO_CHANNEL, ...target, branch: null, prepared: false, eligibility: null, manualReason: null, handoffAttempt: null, unconfirmedState: null, archive: null, disposed: null, transferred: false } };
    case "studio_recovery_list": return { handled: true, value: { v: 1, kind: "recoveryList", channel: VISUAL_STUDIO_CHANNEL, ...target, source: { epochId, epoch: "3", phase: "open", provisional: true }, versions: [], evictionPending: null, pendingIntents: 0 } };
    case "studio_fault_read": {
      if (args.scope !== "source" && args.scope !== "registry") throw new Error("Visual Studio fixture refuses unknown repair scope.");
      return { handled: true, value: { v: 1, kind: "fault", channel: VISUAL_STUDIO_CHANNEL, ...target, scope: args.scope === "source" ? { kind: "source" } : { kind: "registryBucket", bucket: 0 }, source: { epochId, epoch: "3", phase: "open" }, candidates: null, repair: null, mayDecide: false, blockedBy: "noFault", lastAttempt: null, waiting: 0, preservedOperations: 0 } };
    }
    default: throw new Error(`Read-only visual Studio fixture does not implement Tauri command: ${command}`);
  }
}
