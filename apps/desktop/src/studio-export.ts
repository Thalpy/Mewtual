// Pure Gate 6 conversion. The result goes through the same validator as the jukebox's saved
// takes; this module owns neither playback nor publication nor permission to edit Studio state.
import { PATCH_OSC_WAVES, TAKE_ID_MAX_BYTES, type JamPatch, type JamTake, type JamTakeEvent } from "./jam-contract.ts";
import { jamPatchId, validateJamPatch } from "./jam-patch.ts";
import { validateJamTake } from "./jam-recorder.ts";
import { FLIPNOTE_MAX_PATCHES, hex, SCORE_MAX_STEPS, SCORE_ROWS, SCORE_MAX_CELLS, SCORE_VOLUME_MAX, SCORE_VOLUME_MIN, type ScoreCell } from "./studio-contract.ts";
export { SCORE_MAX_STEPS, SCORE_ROWS, SCORE_MAX_CELLS } from "./studio-contract.ts";

/** A resolved export view. Conflicts, overflow and native authority must be handled by its caller. */
export type ScoreExportInput = Readonly<{
  id: string;
  bpm: number;
  bpb: number;
  steps: number;
  patches: Readonly<Record<string, JamPatch>>;
  /** Note length is sixteenth steps; omitted len preserves the original one-step draft input. */
  cells: Readonly<Record<string, ScoreCell>>;
}>;

export type StudioExportContext = Readonly<{
  /** Stable group identifier supplied by the backend, never a device-local server number. */
  group: string;
  /** Full device identity of the exporter. This is a content label, not a signature. */
  exporter: string;
}>;

export type StudioPatchUnion = Readonly<{ ids: string[]; patches: JamPatch[] }>;
/**
 * A resolved Score take plus its authored grid end. jam-take:v1 ends at its last event, so
 * trailing rests and empty bars exist only here: Studio playback keeps them, a shared take does not.
 */
export type ScorePlayback = Readonly<{ take: JamTake; gridMs: number }>;
type PatchTable = Map<string, JamPatch>;
type Cell = { step: number; row: number; value: { k: "n"; p: string; len: number; vol?: number } | { k: "d"; pad: number } };
type ScoreSnapshot = { id: string; bpm: number; bpb: number; steps: number; patches: PatchTable; cells: Cell[] };
const encoder = new TextEncoder();

export class StudioExportError extends Error {
  constructor(reason: string) { super(`studio export: ${reason}`); this.name = "StudioExportError"; }
}

function fail(reason: string): never { throw new StudioExportError(reason); }
function integer(value: unknown, min: number, max: number): value is number {
  return Number.isSafeInteger(value) && (value as number) >= min && (value as number) <= max;
}
function record(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return fail("expected a plain record");
  const proto = Object.getPrototypeOf(value);
  if (proto !== Object.prototype && proto !== null) return fail("expected a plain record");
  if (Object.values(Object.getOwnPropertyDescriptors(value)).some((d) => d.get || d.set)) return fail("accessor fields");
  return value as Record<string, unknown>;
}
function exact(value: Record<string, unknown>, keys: string[]): void {
  if (Object.keys(value).sort().join(",") !== keys.sort().join(",")) fail("unexpected fields");
}
function id(value: unknown, length: number): string {
  if (typeof value !== "string" || !(new RegExp(`^[0-9a-f]{${length}}$`)).test(value)) fail("invalid identity");
  return value;
}
function rawId(value: string): Uint8Array {
  return Uint8Array.from(value.match(/../g)!, (pair) => Number.parseInt(pair, 16));
}

function context(input: StudioExportContext): StudioExportContext {
  const source = record(input);
  const group = source.group, exporter = source.exporter;
  // TextEncoder silently substitutes a lone surrogate. Refuse that second representation before
  // hashing/serializing, as the Studio scalar-text contract requires.
  if (typeof group !== "string" || !group.length || /[\uD800-\uDFFF]/u.test(group) ||
    encoder.encode(group).length > TAKE_ID_MAX_BYTES) fail("invalid group identifier");
  return { group, exporter: id(exporter, 64) };
}

function snapshotPatches(input: unknown): PatchTable {
  const source = record(input);
  const keys = Object.keys(source).sort();
  if (keys.length > FLIPNOTE_MAX_PATCHES) fail("too many patches");
  const result: PatchTable = new Map();
  for (const key of keys) {
    id(key, 64);
    const checked = validateJamPatch(source[key]);
    if (!checked.ok) fail(checked.error);
    // validateJamPatch reconstructs every field; nothing caller-mutable survives the first await.
    result.set(key, checked.patch);
  }
  return result;
}

async function verifyPatches(table: PatchTable): Promise<void> {
  for (const [key, patch] of table) if (key !== await jamPatchId(patch)) fail("patch identity mismatch");
}

function mergePatches(score: PatchTable, sfx: PatchTable): StudioPatchUnion {
  const union = new Map(score);
  for (const [key, patch] of sfx) {
    const previous = union.get(key);
    if (previous && JSON.stringify(previous) !== JSON.stringify(patch)) fail("patch identity equivocation");
    if (!previous) union.set(key, patch);
  }
  if (union.size > FLIPNOTE_MAX_PATCHES) fail("score and SFX patch union exceeds 64");
  return { ids: [...union.keys()], patches: [...union.values()] };
}

/** Score recipes retain their indices; distinct SFX recipes are appended in ascending id order. */
export async function studioPatchUnion(scorePatches: unknown, sfxPatches: unknown): Promise<StudioPatchUnion> {
  const score = snapshotPatches(scorePatches), sfx = snapshotPatches(sfxPatches);
  const union = mergePatches(score, sfx);
  await verifyPatches(score);
  await verifyPatches(sfx);
  return union;
}

function snapshotScore(input: unknown): ScoreSnapshot {
  const source = record(input);
  exact(source, ["id", "bpm", "bpb", "steps", "patches", "cells"]);
  const object = id(source.id, 32);
  if (!integer(source.bpm, 40, 240) || !integer(source.bpb, 1, 8) || !integer(source.steps, 1, SCORE_MAX_STEPS)) {
    fail("score header bounds");
  }
  const patches = snapshotPatches(source.patches);
  const values = record(source.cells), keys = Object.keys(values);
  if (keys.length > SCORE_MAX_CELLS) fail("too many score cells");
  const cells: Cell[] = [];
  for (const key of keys) {
    if (!/^(0|[1-9][0-9]*):(0|[1-9][0-9]*)$/.test(key)) fail("invalid cell address");
    const [step, row] = key.split(":").map(Number);
    if (!integer(step, 0, source.steps - 1) || !integer(row, 0, SCORE_ROWS - 1)) fail("cell outside score");
    const value = record(values[key]);
    if (value.k === "n") {
      exact(value, ["k", "p", ...(Object.hasOwn(value, "len") ? ["len"] : []), ...(Object.hasOwn(value, "vol") ? ["vol"] : [])]);
      const patch = id(value.p, 64);
      const len = Object.hasOwn(value, "len") ? value.len : 1;
      if (!integer(len, 1, source.steps - step)) fail("note length outside score");
      if (!patches.has(patch)) fail("cell patch missing");
      // The authored level is carried, never defaulted in: an absent `vol` stays absent so the
      // take event for a legacy note is byte-identical to what it was before levels existed.
      const vol = Object.hasOwn(value, "vol") ? value.vol : undefined;
      if (vol !== undefined && !integer(vol, SCORE_VOLUME_MIN, SCORE_VOLUME_MAX)) fail("note volume outside 1..100");
      cells.push({ step, row, value: { k: "n", p: patch, len, ...(vol === undefined ? {} : { vol }) } });
    } else if (value.k === "d") {
      exact(value, ["k", "pad"]);
      if (!integer(value.pad, 0, 9)) fail("invalid drum pad");
      cells.push({ step, row, value: { k: "d", pad: value.pad } });
    } else fail("invalid cell kind");
  }
  cells.sort((a, b) => a.step - b.step || a.row - b.row);
  const ends = new Map<number, number>();
  for (const { step, row, value } of cells) if (value.k === "n") {
    // The existing take engine has one held voice per lane/pitch. A resolved export cannot
    // silently truncate or merge overlapping authored notes; let its caller present the conflict.
    if ((ends.get(row) ?? 0) > step) fail("overlapping score notes");
    ends.set(row, step + value.len);
  }
  return { id: object, bpm: source.bpm, bpb: source.bpb, steps: source.steps, patches, cells };
}

/** Start of a sixteenth step; `scoreStepMs(steps, bpm)` is the authored grid end. */
export function scoreStepMs(step: number, bpm: number): number {
  return Math.floor(step * 15000 / bpm);
}

/** The design's H(domain, parts...) with u32 BE framing; patch identities use their existing exception. */
async function framedHash(domain: string, ...parts: Uint8Array[]): Promise<string> {
  const all = [encoder.encode(domain), ...parts];
  const bytes = new Uint8Array(all.reduce((size, part) => size + 4 + part.length, 0));
  const view = new DataView(bytes.buffer);
  let offset = 0;
  for (const part of all) {
    view.setUint32(offset, part.length, false); offset += 4;
    bytes.set(part, offset); offset += part.length;
  }
  return hex(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)));
}
async function laneNonce(domain: string, object: string): Promise<string> {
  const value = (await framedHash(domain, rawId(object))).slice(0, 16);
  return value === "0000000000000000" ? "0000000000000001" : value;
}
function checkedTake(take: JamTake): JamTake {
  const checked = validateJamTake(take);
  if (!checked.ok) fail(checked.error);
  return checked.take;
}

async function scoreTake(score: ScoreSnapshot, ctx: StudioExportContext, union: StudioPatchUnion): Promise<JamTake> {
  const indices = new Map(union.ids.map((key, index) => [key, index]));
  const pending: Array<{ ms: number; order: number; row: number; event: JamTakeEvent }> = [];
  for (const { step, row, value } of score.cells) {
    const ms = scoreStepMs(step, score.bpm);
    if (value.k === "d") {
      pending.push({ ms, order: 1, row, event: { ms, lane: 0, n: value.pad, d: 1, q: 0 } });
    } else {
      // `p` selects the full recipe in the existing engine. Its required legacy fallback follows
      // the first oscillator, making the byte representation deterministic for every recipe.
      const w = PATCH_OSC_WAVES[score.patches.get(value.p)!.o[0].w];
      pending.push({ ms, order: 1, row, event: { ms, lane: 0, n: row + 60, on: 1, w, p: indices.get(value.p)!, q: 0, ...(value.vol === undefined ? {} : { v: value.vol }) } });
      const off = scoreStepMs(step + value.len, score.bpm) - 1;
      pending.push({ ms: off, order: 0, row, event: { ms: off, lane: 0, n: row + 60, on: 0, q: 0 } });
    }
  }
  pending.sort((a, b) => a.ms - b.ms || a.order - b.order || a.row - b.row);
  return checkedTake({
    v: 1, group: ctx.group, call: await framedHash("catcoms-score-export:v1", rawId(score.id)),
    met: { bpm: score.bpm, bpb: score.bpb }, parts: [ctx.exporter],
    lanes: [{ src: 0, sn: await laneNonce("catcoms-score-lane:v1", score.id) }],
    patches: union.patches, events: pending.map(({ event }, q) => ({ ...event, q })),
  });
}

/** Export a resolved score through the jukebox's validator. No calls, timers or audio are started. */
export async function exportScoreTake(input: unknown, inputContext: StudioExportContext): Promise<JamTake> {
  return (await exportScorePlayback(input, inputContext)).take;
}

/** The same export plus the grid end, taken from the validated snapshot rather than the caller. */
export async function exportScorePlayback(input: unknown, inputContext: StudioExportContext): Promise<ScorePlayback> {
  const score = snapshotScore(input), ctx = context(inputContext);
  await verifyPatches(score.patches);
  return { take: await scoreTake(score, ctx, mergePatches(score.patches, new Map())), gridMs: scoreStepMs(score.steps, score.bpm) };
}

/** Package a linked score or derive the exact no-score take; epoch is lossless all the way to H(). */
export async function exportFlipnoteTake(input: Readonly<{
  id: string; epoch: bigint; patches: unknown; score?: unknown;
}>, inputContext: StudioExportContext): Promise<Readonly<{ take: JamTake; patchIds: string[] }>> {
  record(input); // Wrapper accessors must not substitute new ids/recipes after validation.
  const object = id(input.id, 32), epoch = input.epoch, ctx = context(inputContext);
  if (typeof epoch !== "bigint" || epoch < 0n || epoch > 0xffff_ffff_ffff_ffffn) fail("invalid epoch");
  const score = input.score === undefined ? null : snapshotScore(input.score);
  const sfx = snapshotPatches(input.patches);
  const union = mergePatches(score?.patches ?? new Map(), sfx);
  if (score) await verifyPatches(score.patches);
  await verifyPatches(sfx);
  if (score) return { take: await scoreTake(score, ctx, union), patchIds: union.ids };
  const epochBytes = new Uint8Array(8);
  new DataView(epochBytes.buffer).setBigUint64(0, epoch, false);
  return { patchIds: union.ids, take: checkedTake({
    v: 1, group: ctx.group,
    call: await framedHash("catcoms-flipnote-export:v1", rawId(object), epochBytes),
    met: { bpm: 120, bpb: 4 }, parts: [ctx.exporter],
    lanes: [{ src: 0, sn: await laneNonce("catcoms-flipnote-lane:v1", object) }],
    patches: union.patches, events: [],
  }) };
}
