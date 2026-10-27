/**
 * The cut list behind the upload video editor (`VideoEditor.svelte`).
 *
 * A video picked, dropped or pasted into a composer can be trimmed and cut before it is uploaded.
 * This module is the whole editing model and has no DOM, codec or clock in it, so every rule the
 * editor and the exporter depend on is decidable in a unit test.
 *
 * The model is deliberately the simplest one that still supports a blade:
 *
 * - `cuts` are blade positions, in source seconds, strictly inside (0, duration) and at least
 *   `MIN_PIECE_SECONDS` apart. They divide the source into `cuts.length + 1` pieces.
 * - `removed[i]` marks piece `i` as deleted. A cut on its own changes nothing in the output; it
 *   only makes a piece that can then be removed.
 * - `inPoint`/`outPoint` are the trim handles. They are separate from the cuts so that dragging a
 *   handle never litters the timeline with boundaries the user did not make.
 *
 * What gets uploaded is `keptRanges`: every piece that is not removed, clipped to the trim window,
 * with touching ranges merged. All times are in the SOURCE timeline. The exporter maps them to the
 * output timeline itself (see `exportPlan`), and the editor shows both.
 *
 * Every operation is pure and returns either a new edit or the very same object when the request
 * was refused (out of range, too close to an existing cut, and so on). Callers use that identity
 * to decide whether an undo step was made, so a refused operation must never allocate a copy.
 */

/**
 * The shortest piece the editor will create, and the closest two cuts (or a cut and an end) may
 * sit. Anything shorter is a fraction of a frame at common frame rates: unseeable on the timeline,
 * and a piece that small would only ever hold a single frame or none.
 */
export const MIN_PIECE_SECONDS = 0.1;

/**
 * Upper bound on blade cuts. It keeps the per-frame mapping in the exporter a short scan and the
 * timeline legible; nobody editing a clip before sharing it needs a hundred cuts.
 */
export const MAX_CUTS = 99;

/** Undo depth. Older steps are dropped first. */
export const HISTORY_LIMIT = 100;

/**
 * Tolerance for "the handle is at the very end". The duration comes from the demuxer and the
 * handle position from a pointer, so exact equality would make an untouched clip look trimmed.
 */
const END_EPSILON = 1e-3;

export type VideoEdit = {
  /** Source duration in seconds; finite and positive. */
  readonly duration: number;
  /** Blade positions, ascending. */
  readonly cuts: readonly number[];
  /** One flag per piece (`cuts.length + 1`); true when that piece is deleted. */
  readonly removed: readonly boolean[];
  /** Trim handles; `0 <= inPoint < outPoint <= duration`. */
  readonly inPoint: number;
  readonly outPoint: number;
};

export type TimeRange = { readonly start: number; readonly end: number };

/** One piece of the source for rendering: its span, its index, and whether it is deleted. */
export type Piece = TimeRange & { readonly index: number; readonly removed: boolean };

/** A fresh, untouched edit of a source `duration` seconds long. */
export function initialEdit(duration: number): VideoEdit {
  if (!Number.isFinite(duration) || duration <= 0) {
    throw new RangeError("a video edit needs a finite, positive duration");
  }
  return { duration, cuts: [], removed: [false], inPoint: 0, outPoint: duration };
}

function clamp(value: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, value));
}

/** The piece boundaries: 0, every cut, then the duration. */
function bounds(edit: VideoEdit): number[] {
  return [0, ...edit.cuts, edit.duration];
}

/** Every piece in order, including removed ones and ones outside the trim window. */
export function pieces(edit: VideoEdit): Piece[] {
  const b = bounds(edit);
  return edit.removed.map((removed, index) => ({ start: b[index], end: b[index + 1], index, removed }));
}

/**
 * The index of the piece that contains `t`. Pieces are half-open `[start, end)`, except that the
 * very end of the source belongs to the last piece, so a playhead parked at the end still has one.
 */
export function pieceIndexAt(edit: VideoEdit, t: number): number {
  let index = 0;
  while (index < edit.cuts.length && t >= edit.cuts[index]) index++;
  return index;
}

/** Whether a blade at `t` would be accepted; see `splitAt`. */
export function canSplitAt(edit: VideoEdit, t: number): boolean {
  if (!Number.isFinite(t) || edit.cuts.length >= MAX_CUTS) return false;
  // Inside the source first: a point far outside it is "far from every boundary" too, and a cut
  // there would make a piece with negative length.
  if (t <= 0 || t >= edit.duration) return false;
  // The nanosecond slack lets "exactly the minimum away" pass despite binary fractions (5.1 - 5
  // is 0.0999...); it is far below anything a pointer or a frame can express.
  return bounds(edit).every((b) => Math.abs(b - t) >= MIN_PIECE_SECONDS - 1e-9);
}

/**
 * Blade: cut the piece under `t` in two. Both halves inherit the piece's removed flag, so cutting
 * a deleted piece leaves both halves deleted. Refused (same object back) within
 * `MIN_PIECE_SECONDS` of an existing cut or either end, or once `MAX_CUTS` is reached.
 */
export function splitAt(edit: VideoEdit, t: number): VideoEdit {
  if (!canSplitAt(edit, t)) return edit;
  const index = pieceIndexAt(edit, t);
  const cuts = [...edit.cuts];
  cuts.splice(index, 0, t);
  const removed = [...edit.removed];
  removed.splice(index, 0, edit.removed[index]);
  return { ...edit, cuts, removed };
}

/** Delete or restore piece `index`. Out-of-range indices and no-op changes are refused. */
export function setRemoved(edit: VideoEdit, index: number, removed: boolean): VideoEdit {
  if (!Number.isInteger(index) || index < 0 || index >= edit.removed.length) return edit;
  if (edit.removed[index] === removed) return edit;
  const next = [...edit.removed];
  next[index] = removed;
  return { ...edit, removed: next };
}

export function toggleRemoved(edit: VideoEdit, index: number): VideoEdit {
  return setRemoved(edit, index, !edit.removed[index]);
}

/**
 * Drag cut `cutIndex` (0-based into `cuts`) towards `t`. The cut stays between its neighbours with
 * `MIN_PIECE_SECONDS` to spare, so a drag can shrink a piece but never erase or reorder one.
 */
export function moveCut(edit: VideoEdit, cutIndex: number, t: number): VideoEdit {
  if (!Number.isInteger(cutIndex) || cutIndex < 0 || cutIndex >= edit.cuts.length || !Number.isFinite(t)) return edit;
  const lo = (cutIndex === 0 ? 0 : edit.cuts[cutIndex - 1]) + MIN_PIECE_SECONDS;
  const hi = (cutIndex === edit.cuts.length - 1 ? edit.duration : edit.cuts[cutIndex + 1]) - MIN_PIECE_SECONDS;
  // A neighbourhood narrower than two minimum pieces cannot happen through these functions, but a
  // cut list built some other way must not make the clamp invert and teleport the cut.
  if (lo > hi) return edit;
  const next = clamp(t, lo, hi);
  if (next === edit.cuts[cutIndex]) return edit;
  const cuts = [...edit.cuts];
  cuts[cutIndex] = next;
  return { ...edit, cuts };
}

/** Move the in-handle; it stays at least `MIN_PIECE_SECONDS` before the out-handle. */
export function setInPoint(edit: VideoEdit, t: number): VideoEdit {
  if (!Number.isFinite(t)) return edit;
  const next = clamp(t, 0, Math.max(0, edit.outPoint - MIN_PIECE_SECONDS));
  return next === edit.inPoint ? edit : { ...edit, inPoint: next };
}

/** Move the out-handle; it stays at least `MIN_PIECE_SECONDS` after the in-handle. */
export function setOutPoint(edit: VideoEdit, t: number): VideoEdit {
  if (!Number.isFinite(t)) return edit;
  const next = clamp(t, Math.min(edit.duration, edit.inPoint + MIN_PIECE_SECONDS), edit.duration);
  return next === edit.outPoint ? edit : { ...edit, outPoint: next };
}

/**
 * What will be uploaded, in source seconds: the pieces that are not removed, clipped to the trim
 * window, with ranges that touch merged into one. Sorted, disjoint, and never empty-width.
 */
export function keptRanges(edit: VideoEdit): TimeRange[] {
  const out: { start: number; end: number }[] = [];
  for (const piece of pieces(edit)) {
    if (piece.removed) continue;
    const start = Math.max(piece.start, edit.inPoint);
    const end = Math.min(piece.end, edit.outPoint);
    if (end <= start) continue;
    const last = out[out.length - 1];
    if (last && last.end === start) last.end = end;
    else out.push({ start, end });
  }
  return out;
}

/** Length of the upload in seconds. */
export function keptDuration(edit: VideoEdit): number {
  return keptRanges(edit).reduce((sum, r) => sum + (r.end - r.start), 0);
}

/**
 * True when the edit would upload exactly the source: nothing trimmed and nothing removed. Cuts on
 * their own do not count. The caller then uploads the original bytes rather than a re-encode, so
 * the person who did not edit anything loses no quality and waits for no transcode.
 */
export function isUnedited(edit: VideoEdit): boolean {
  return edit.inPoint <= END_EPSILON
    && edit.outPoint >= edit.duration - END_EPSILON
    && edit.removed.every((removed) => !removed);
}

/**
 * Where preview playback should be at source time `t`, so the preview plays what the upload will
 * contain. `null` means `t` is inside a kept range and playback continues; `{ seek }` means jump
 * forward over trimmed or removed time; `{ end: true }` means there is nothing kept after `t`.
 *
 * The preview polls this once per painted frame, so playback can show up to one frame of removed
 * time before the jump lands. That is a preview artefact only: the exporter never encodes it.
 */
export function playbackJump(edit: VideoEdit, t: number): { seek: number } | { end: true } | null {
  for (const range of keptRanges(edit)) {
    if (t < range.start) return { seek: range.start };
    if (t < range.end) return null;
  }
  return { end: true };
}

/**
 * Where the upload's playhead is for source time `t`: the kept time before `t`. Inside removed or
 * trimmed time it holds at the output position of the next kept frame.
 */
export function sourceToOutput(edit: VideoEdit, t: number): number {
  let out = 0;
  for (const range of keptRanges(edit)) {
    if (t <= range.start) break;
    out += Math.min(t, range.end) - range.start;
  }
  return out;
}

/**
 * The source time shown at output time `o`: the inverse of `sourceToOutput` over kept time. Null
 * when `o` is outside the upload (negative, or past its end). The end of a range belongs to the
 * next range's start, so a join never maps into the removed time between them.
 */
export function outputToSource(edit: VideoEdit, o: number): number | null {
  if (!Number.isFinite(o) || o < 0) return null;
  let at = 0;
  for (const range of keptRanges(edit)) {
    const length = range.end - range.start;
    if (o < at + length) return range.start + (o - at);
    at += length;
  }
  return null;
}

// --- Undo/redo ----------------------------------------------------------------------------------

/**
 * Undo/redo over any immutable value: the cut list here, the whole clip document in
 * `clip-doc.ts`. Every rule below works on identity alone, so it holds for both.
 */
export type EditHistory<T = VideoEdit> = {
  readonly past: readonly T[];
  readonly present: T;
  readonly future: readonly T[];
};

export function startHistory<T = VideoEdit>(edit: T): EditHistory<T> {
  return { past: [], present: edit, future: [] };
}

/**
 * Record `next` as a new undo step. A refused operation hands back the present edit itself, and
 * that is not a step: pushing it would make Undo appear to do nothing.
 */
export function pushEdit<T>(history: EditHistory<T>, next: T): EditHistory<T> {
  if (next === history.present) return history;
  const past = [...history.past, history.present];
  if (past.length > HISTORY_LIMIT) past.splice(0, past.length - HISTORY_LIMIT);
  return { past, present: next, future: [] };
}

/**
 * Replace the present edit without making an undo step. A drag calls this for every pointer move
 * after the first, so one drag is one undo step however many moves it took.
 */
export function replaceEdit<T>(history: EditHistory<T>, next: T): EditHistory<T> {
  return next === history.present ? history : { ...history, present: next };
}

export function undo<T>(history: EditHistory<T>): EditHistory<T> {
  if (history.past.length === 0) return history;
  const past = [...history.past];
  const present = past.pop() as T;
  return { past, present, future: [history.present, ...history.future] };
}

export function redo<T>(history: EditHistory<T>): EditHistory<T> {
  if (history.future.length === 0) return history;
  const [present, ...future] = history.future;
  return { past: [...history.past, history.present], present, future };
}

// --- Export mapping -----------------------------------------------------------------------------

/**
 * How the exporter turns the edit into one trimmed conversion.
 *
 * The conversion is asked for `[trimStart, trimEnd]` and hands every decoded frame and audio chunk
 * to a callback with a timestamp RELATIVE to `trimStart`. `ranges` are the kept ranges in that same
 * relative time; the gaps between them are the removed pieces, which the callbacks drop. `outStarts`
 * is where each range begins in the output, so a kept frame is moved earlier by exactly the removed
 * time before it and audio and video stay in step across every cut.
 *
 * `trimEnd` is null when the upload runs to the end of the source. The duration measured for the
 * timeline and the timestamps of the final frames can disagree by a frame, and a finite end there
 * could drop the last frame or the tail of the sound.
 */
export type ExportPlan = {
  readonly trimStart: number;
  readonly trimEnd: number | null;
  readonly ranges: readonly TimeRange[];
  readonly outStarts: readonly number[];
  /** Expected output length in seconds, for progress and the summary line. */
  readonly outputDuration: number;
};

export function exportPlan(edit: VideoEdit): ExportPlan | null {
  const kept = keptRanges(edit);
  if (kept.length === 0) return null;
  const trimStart = kept[0].start;
  const last = kept[kept.length - 1];
  const toEnd = last.end >= edit.duration - END_EPSILON;
  const ranges = kept.map((r, i) => ({
    start: r.start - trimStart,
    end: i === kept.length - 1 && toEnd ? Number.POSITIVE_INFINITY : r.end - trimStart,
  }));
  const outStarts: number[] = [];
  let out = 0;
  for (const r of kept) {
    outStarts.push(out);
    out += r.end - r.start;
  }
  return { trimStart, trimEnd: toEnd ? null : last.end, ranges, outStarts, outputDuration: out };
}

/**
 * Where a decoded video frame lands in the output, or null to drop it. `timestamp` and `duration`
 * are in the conversion's relative time (see `ExportPlan`).
 *
 * A frame that straddles a cut keeps only the part inside the first kept range it touches, so the
 * picture changes exactly at the cut. A frame wholly inside removed time is dropped: it is never
 * encoded, which is what makes "removed" mean the frames are absent from the uploaded bytes.
 */
export function mapVideoFrame(
  plan: ExportPlan,
  timestamp: number,
  duration: number,
): { timestamp: number; duration: number } | null {
  const end = timestamp + Math.max(0, duration);
  for (let i = 0; i < plan.ranges.length; i++) {
    const r = plan.ranges[i];
    if (duration <= 0) {
      // A frame with no duration is a point in time: keep it only if that point is kept.
      if (timestamp >= r.start && timestamp < r.end) {
        return { timestamp: plan.outStarts[i] + (timestamp - r.start), duration: 0 };
      }
      continue;
    }
    const a = Math.max(timestamp, r.start);
    const b = Math.min(end, r.end);
    if (a < b) return { timestamp: plan.outStarts[i] + (a - r.start), duration: b - a };
  }
  return null;
}

/** One kept slice of an audio chunk: frames `[startFrame, endFrame)` placed at `timestamp`. */
export type AudioSlice = { startFrame: number; endFrame: number; timestamp: number };

/**
 * Which frames of a decoded audio chunk survive the edit, and where each surviving slice lands.
 * Unlike video a chunk can survive in several slices: a removed piece shorter than the chunk (a
 * few tens of milliseconds) leaves sound on both sides of it. Empty when the chunk is all removed.
 */
export function mapAudioChunk(
  plan: ExportPlan,
  timestamp: number,
  numberOfFrames: number,
  sampleRate: number,
): AudioSlice[] {
  if (!(numberOfFrames > 0) || !(sampleRate > 0)) return [];
  const end = timestamp + numberOfFrames / sampleRate;
  const slices: AudioSlice[] = [];
  for (let i = 0; i < plan.ranges.length; i++) {
    const r = plan.ranges[i];
    const a = Math.max(timestamp, r.start);
    const b = Math.min(end, r.end);
    if (a >= b) continue;
    const startFrame = clamp(Math.round((a - timestamp) * sampleRate), 0, numberOfFrames);
    const endFrame = clamp(Math.round((b - timestamp) * sampleRate), 0, numberOfFrames);
    if (endFrame > startFrame) slices.push({ startFrame, endFrame, timestamp: plan.outStarts[i] + (a - r.start) });
  }
  return slices;
}

// --- Presentation helpers -----------------------------------------------------------------------

/** Ruler spacings in seconds, smallest first; the first that fits the tick budget wins. */
const RULER_STEPS = [0.1, 0.2, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

/**
 * Ruler ticks for a timeline `duration` seconds long: multiples of the smallest round step that
 * gives at most `maxTicks` intervals, starting at 0 and never past the end. Returns the step too so
 * labels can drop the tenths when every tick is a whole second.
 */
export function rulerTicks(duration: number, maxTicks: number): { step: number; ticks: number[] } {
  if (!(duration > 0) || !Number.isFinite(duration) || !(maxTicks >= 1)) return { step: 0, ticks: [] };
  const step = RULER_STEPS.find((s) => duration / s <= maxTicks)
    ?? Math.ceil(duration / maxTicks / 3600) * 3600;
  const ticks: number[] = [];
  for (let i = 0; i * step <= duration + 1e-9; i++) ticks.push(Math.round(i * step * 1000) / 1000);
  return { step, ticks };
}

/** A ruler label: `formatClock` without the tenths when the step is whole seconds. */
export function rulerLabel(t: number, step: number): string {
  const text = formatClock(t);
  return step >= 1 ? text.slice(0, -2) : text;
}

/** `m:ss.t`, or `h:mm:ss.t` from an hour up. Negative and non-finite input reads as zero. */
export function formatClock(seconds: number): string {
  const tenths = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds * 10 + 1e-6) : 0;
  const t = tenths % 10;
  const total = Math.floor(tenths / 10);
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}.${t}` : `${m}:${ss}.${t}`;
}
