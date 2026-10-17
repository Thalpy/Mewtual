// The byte-exact animation package from creative design 2.16. Frames stay encoded so importing
// a 999-frame file never retains 999 expanded rasters. Playback/publication belong to existing owners.
import { TAKE_MAX_BYTES, TAKE_MAX_DURATION_MS, type JamTake } from "./jam-contract.ts";
import { parseJamTakeJson, validateJamTake } from "./jam-recorder.ts";
import { decodePix } from "./pix.ts";
import {
  canonicalJson, FLIPNOTE_FPS_MAX, FLIPNOTE_FPS_MIN, FLIPNOTE_FRAME_BYTES_PROMISE,
  FLIPNOTE_H, FLIPNOTE_MAX_FRAMES, FLIPNOTE_MAX_SFX, FLIPNOTE_W, PIX_MAX_BYTES,
} from "./studio-contract.ts";
import { exportFlipnoteTake, type StudioExportContext } from "./studio-export.ts";

export const PIXA_MAX_BYTES = 9 * 1024 * 1024;
export const PIXA_MAX_SFX_BYTES = 32 * 1024;
const SFX_ENTRY_BYTES = 8;
/** Authored hold before the patch's release envelope; emoji sources supply their validated len. */
export type PixaSfx = Readonly<{ frame: number; patch: number; note: number; durationMs: number }>;
export type Pixa = Readonly<{ fps: number; frames: readonly Uint8Array[]; take: JamTake; sfx: readonly PixaSfx[] }>;
const encoder = new TextEncoder();
// Buffer.slice() aliases and typed-array subclasses may override length/slice/subarray. Read the
// intrinsic length for admission, then copy through the base constructor without caller methods.
const typedArrayLength = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), "length")!.get!;

export class PixaError extends Error {
  constructor(reason: string) { super(`pixa:v1: ${reason}`); this.name = "PixaError"; }
}
function fail(reason: string): never { throw new PixaError(reason); }
function integer(value: unknown, min: number, max: number): value is number {
  return Number.isSafeInteger(value) && (value as number) >= min && (value as number) <= max;
}
function byteLength(bytes: Uint8Array): number { return typedArrayLength.call(bytes) as number; }
function copyBytes(source: Uint8Array, admittedLength: number): Uint8Array {
  const owned = new Uint8Array(admittedLength);
  // A growable shared view can grow after admission. set() refuses a source larger than this
  // fixed destination; a constructor taking the source would silently allocate its newer size.
  owned.set(source);
  return owned;
}
function snapshotFrames(frames: readonly Uint8Array[]): Uint8Array[] {
  if (!Array.isArray(frames) || !integer(frames.length, 1, FLIPNOTE_MAX_FRAMES)) fail("frame count");
  const count = frames.length, admitted: Array<{ frame: Uint8Array; length: number }> = [];
  let total = 0;
  for (let i = 0; i < count; i++) {
    const frame = frames[i];
    if (!(frame instanceof Uint8Array)) fail("frame length");
    const length = byteLength(frame);
    if (!integer(length, 1, PIX_MAX_BYTES)) fail("frame length");
    total += length;
    if (total > FLIPNOTE_FRAME_BYTES_PROMISE) fail("frame byte budget");
    admitted.push({ frame, length });
  }
  return admitted.map(({ frame, length }) => copyBytes(frame, length));
}
function validateFrame(bytes: Uint8Array): void {
  const image = decodePix(bytes);
  if (image.w !== FLIPNOTE_W || image.h !== FLIPNOTE_H) fail("frame dimensions");
}
function checkedTake(value: unknown): JamTake {
  const checked = validateJamTake(value);
  if (!checked.ok) fail(checked.error);
  for (const text of [checked.take.group, checked.take.call, ...checked.take.parts]) {
    if (/[\uD800-\uDFFF]/u.test(text)) fail("take text is not Unicode scalar text");
  }
  return checked.take;
}
function validateSfx(sfx: readonly PixaSfx[], frames: number, patches: number): void {
  if (!Array.isArray(sfx) || sfx.length > FLIPNOTE_MAX_SFX || sfx.length * SFX_ENTRY_BYTES > PIXA_MAX_SFX_BYTES) fail("SFX count");
  for (const entry of sfx) {
    if (!entry || !integer(entry.frame, 0, frames - 1) || !integer(entry.patch, 0, patches - 1) ||
      !integer(entry.note, 0, 127)) fail("SFX index or note");
    if (!integer(entry.durationMs, 1, TAKE_MAX_DURATION_MS)) fail("SFX duration");
  }
}

/** Encode only validated content. The caller orders SFX by source id before ids leave the format. */
export function encodePixa(input: Pixa): Uint8Array {
  const fps = input.fps;
  if (!integer(fps, FLIPNOTE_FPS_MIN, FLIPNOTE_FPS_MAX)) fail("fps");
  const sourceFrames = input.frames, sourceSfx = input.sfx;
  const frames = snapshotFrames(sourceFrames);
  const totalFrames = frames.reduce((sum, frame) => sum + frame.length, 0);
  const take = checkedTake(input.take);
  const takeBytes = encoder.encode(canonicalJson(take));
  if (takeBytes.length > TAKE_MAX_BYTES) fail("take byte budget");
  if (!Array.isArray(sourceSfx) || sourceSfx.length > FLIPNOTE_MAX_SFX) fail("SFX count");
  const sfx = sourceSfx.map(({ frame, patch, note, durationMs }) => ({ frame, patch, note, durationMs }));
  validateSfx(sfx, frames.length, take.patches.length);
  const length = 7 + 4 * frames.length + totalFrames + 4 + takeBytes.length + 2 + SFX_ENTRY_BYTES * sfx.length;
  if (length > PIXA_MAX_BYTES) fail("whole byte budget");
  for (const frame of frames) validateFrame(frame);
  const bytes = new Uint8Array(length), view = new DataView(bytes.buffer);
  bytes.set([0x50, 0x49, 0x58, 0x41, fps]);
  view.setUint16(5, frames.length, true);
  let offset = 7;
  for (const frame of frames) {
    view.setUint32(offset, frame.length, true); offset += 4;
    bytes.set(frame, offset); offset += frame.length;
  }
  view.setUint32(offset, takeBytes.length, true); offset += 4;
  bytes.set(takeBytes, offset); offset += takeBytes.length;
  view.setUint16(offset, sfx.length, true); offset += 2;
  for (const entry of sfx) {
    view.setUint16(offset, entry.frame, true);
    bytes[offset + 2] = entry.patch; bytes[offset + 3] = entry.note;
    view.setUint32(offset + 4, entry.durationMs, true); offset += SFX_ENTRY_BYTES;
  }
  return bytes;
}

/** Check all length framing before decoding a raster or copying any frame body. */
export function decodePixa(input: Uint8Array): Pixa {
  if (!(input instanceof Uint8Array)) fail("whole byte budget");
  const admittedLength = byteLength(input);
  if (admittedLength > PIXA_MAX_BYTES) fail("whole byte budget");
  // An admitted shared-memory input may change concurrently. Parse and validate exclusively this
  // owned snapshot, so bytes copied into the result are exactly the bytes that passed validation.
  const bytes = copyBytes(input, admittedLength);
  if (bytes.length < 7) fail("truncated header");
  if (bytes[0] !== 0x50 || bytes[1] !== 0x49 || bytes[2] !== 0x58 || bytes[3] !== 0x41) fail("magic");
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const fps = bytes[4], count = view.getUint16(5, true);
  if (!integer(fps, FLIPNOTE_FPS_MIN, FLIPNOTE_FPS_MAX)) fail("fps");
  if (!integer(count, 1, FLIPNOTE_MAX_FRAMES)) fail("frame count");
  let offset = 7, totalFrames = 0;
  const spans: Array<{ offset: number; length: number }> = [];
  const require = (length: number) => { if (length > bytes.length - offset) fail("truncated body"); };
  for (let i = 0; i < count; i++) {
    require(4);
    const length = view.getUint32(offset, true); offset += 4;
    if (!integer(length, 1, PIX_MAX_BYTES)) fail("frame length");
    totalFrames += length;
    if (totalFrames > FLIPNOTE_FRAME_BYTES_PROMISE) fail("frame byte budget");
    require(length); spans.push({ offset, length }); offset += length;
  }
  require(4);
  const takeLength = view.getUint32(offset, true); offset += 4;
  if (!integer(takeLength, 1, TAKE_MAX_BYTES)) fail("take byte budget");
  require(takeLength);
  const takeStart = offset; offset += takeLength;
  require(2);
  const sfxCount = view.getUint16(offset, true); offset += 2;
  if (sfxCount > FLIPNOTE_MAX_SFX || sfxCount * SFX_ENTRY_BYTES > PIXA_MAX_SFX_BYTES) fail("SFX count");
  require(sfxCount * SFX_ENTRY_BYTES);
  if (offset + sfxCount * SFX_ENTRY_BYTES !== bytes.length) fail("trailing bytes");

  let text: string;
  try { text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes.subarray(takeStart, takeStart + takeLength)); }
  catch { return fail("take UTF-8"); }
  // Reuse the existing bounded parser. Equality to its normalized canonical result also rejects
  // duplicate JSON fields, alternate number spellings, whitespace and noncanonical event order.
  const parsed = parseJamTakeJson(text);
  if (!parsed.ok) fail(parsed.error);
  const take = checkedTake(parsed.take);
  if (canonicalJson(take) !== text) fail("noncanonical take");
  const sfx: PixaSfx[] = [];
  for (let i = 0; i < sfxCount; i++, offset += SFX_ENTRY_BYTES) {
    sfx.push({ frame: view.getUint16(offset, true), patch: bytes[offset + 2], note: bytes[offset + 3], durationMs: view.getUint32(offset + 4, true) });
  }
  validateSfx(sfx, count, take.patches.length);
  for (const span of spans) validateFrame(bytes.subarray(span.offset, span.offset + span.length));
  return { fps, take, sfx, frames: spans.map((span) => bytes.slice(span.offset, span.offset + span.length)) };
}

/** A resolved Flipnote view, supplied only once all source conflicts/overflow have been handled. */
export type FlipnoteExportInput = Readonly<{
  id: string; epoch: bigint; fps: number;
  frames: readonly Readonly<{ id: string; bytes: Uint8Array }>[];
  patches: unknown; score?: unknown;
  sfx: readonly Readonly<{ id: string; frame: string; patch: string; note: number; durationMs: number }>[];
}>;

/** Resolve stable frame/patch ids to wire indices and serialize SFX in ascending source-id order. */
export async function exportFlipnotePixa(input: FlipnoteExportInput, context: StudioExportContext): Promise<Uint8Array> {
  const fps = input.fps;
  if (!integer(fps, FLIPNOTE_FPS_MIN, FLIPNOTE_FPS_MAX)) fail("fps");
  const sourceFrames = input.frames, sourceEntries = input.sfx;
  if (!Array.isArray(sourceFrames) || sourceFrames.length > FLIPNOTE_MAX_FRAMES) fail("frame count");
  const frameEntries = sourceFrames.map(({ id, bytes }) => ({ id, bytes }));
  const frames = snapshotFrames(frameEntries.map((f) => f.bytes));
  const frameIds = new Map<string, number>();
  for (const [index, frame] of frameEntries.entries()) {
    if (!/^[0-9a-f]{32}$/.test(frame.id) || frameIds.has(frame.id)) fail("frame identity");
    frameIds.set(frame.id, index);
    validateFrame(frames[index]);
  }
  if (!Array.isArray(sourceEntries) || sourceEntries.length > FLIPNOTE_MAX_SFX) fail("SFX count");
  const sfxIds = new Set<string>();
  // Capture every scalar before awaiting recipe digests. Later caller edits must produce a new
  // export, never silently change the bytes of this validated attempt.
  const sourceSfx = sourceEntries.map(({ id, frame, patch, note, durationMs }) => {
    const entry = { id, frame, patch, note, durationMs };
    if (!/^[0-9a-f]{32}$/.test(entry.id) || sfxIds.has(entry.id) || !frameIds.has(entry.frame) ||
      !/^[0-9a-f]{64}$/.test(entry.patch) || !integer(entry.note, 0, 127) ||
      !integer(entry.durationMs, 1, TAKE_MAX_DURATION_MS)) fail("SFX source");
    sfxIds.add(entry.id);
    return { ...entry };
  }).sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
  const { take, patchIds } = await exportFlipnoteTake(input, context);
  const sfx = sourceSfx.map((entry) => ({ frame: frameIds.get(entry.frame)!, patch: patchIds.indexOf(entry.patch), note: entry.note, durationMs: entry.durationMs }));
  return encodePixa({ fps, frames, take, sfx });
}
