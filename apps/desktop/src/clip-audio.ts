/**
 * The clip's sound: every video layer's own audio, the music and the sound effects, mixed into the
 * one audio track the posted clip carries.
 *
 * Two halves:
 *
 * - `soundPieces` (pure) turns the document into a flat list of pieces: "play this much of that
 *   sound, from here, at this output time, this loud". A video layer contributes one piece per kept
 *   range of its cut list, so cut-out sound is never in the list and never reaches the mix.
 * - `mixClipSound` (browser) renders the pieces into the output in fixed SEGMENTS, each its own
 *   short `OfflineAudioContext`, handing each finished segment on before starting the next. A
 *   whole-clip render would hold the full mix (and every decoded source) in memory at once; a
 *   segment holds `MIX_SEGMENT_SECONDS` of it.
 *
 * Because each segment starts with fresh nodes, nothing in the mix may carry state across a segment
 * boundary: no compressor or limiter (their gain reduction would jump at every boundary). Loud
 * mixes are tamed by a stateless soft clip instead (`softClipCurve`), linear for ordinary levels.
 * Piece envelopes are computed from absolute time, so a fade crossing a boundary continues exactly.
 */
import { keptRanges } from "./video-edit.ts";
import { clipDuration, type ClipDoc } from "./clip-doc.ts";
import { CLIP_SAMPLE_RATE } from "./jam-offline.ts";

/** How much of the mix is rendered at a time. */
export const MIX_SEGMENT_SECONDS = 20;
/** Fade at both ends of every piece, so a hard cut in a waveform does not click. */
export const DECLICK_SECONDS = 0.004;
/** The level the soft clip starts bending at; below it the mix passes through unchanged. */
export const SOFT_CLIP_KNEE = 0.8;

export type SoundKind = "video" | "music" | "sfx";

/** One stretch of one sound in the output. Times are seconds. */
export type SoundPiece = {
  readonly kind: SoundKind;
  readonly source: string;
  /** Where in the sound this piece starts (for a video: its source time). */
  readonly from: number;
  /** Output time it plays at. */
  readonly at: number;
  readonly length: number;
  readonly gain: number;
};

/** Everything that sounds in the clip. Muted and silent items, and anything after the end, are left out. */
export function soundPieces(doc: ClipDoc): SoundPiece[] {
  const total = clipDuration(doc);
  const pieces: SoundPiece[] = [];
  const push = (piece: SoundPiece) => {
    const length = Math.min(piece.length, total - piece.at);
    if (length > 0 && piece.gain > 0) pieces.push({ ...piece, length });
  };
  for (const layer of doc.layers) {
    if (layer.muted || !(layer.volume > 0)) continue;
    let at = layer.start;
    for (const range of keptRanges(layer.edit)) {
      push({ kind: "video", source: layer.source, from: range.start, at, length: range.end - range.start, gain: layer.volume });
      at += range.end - range.start;
    }
  }
  for (const item of doc.audio) {
    if (item.muted || !(item.volume > 0)) continue;
    push({ kind: item.kind, source: item.source, from: item.offset, at: item.start, length: item.length, gain: item.volume });
  }
  return pieces;
}

/** A piece's loudness at output time `t`: its gain, faded in and out over `DECLICK_SECONDS`. */
export function envelopeAt(piece: SoundPiece, t: number): number {
  const fade = Math.min(DECLICK_SECONDS, piece.length / 2);
  const into = t - piece.at, left = piece.at + piece.length - t;
  if (into <= 0 || left <= 0) return 0;
  return piece.gain * Math.min(1, into / fade, left / fade);
}

/** The corners of a piece's envelope as `[output time, level]`; it is straight lines between them. */
export function envelopeCorners(piece: SoundPiece): [number, number][] {
  const fade = Math.min(DECLICK_SECONDS, piece.length / 2);
  const end = piece.at + piece.length;
  return [[piece.at, 0], [piece.at + fade, piece.gain], [end - fade, piece.gain], [end, 0]];
}

/** The part of a piece inside one segment, relative to the segment's start. */
export type WindowedPiece = {
  readonly piece: SoundPiece;
  /** Where in the sound this window starts. */
  readonly from: number;
  /** Segment-relative start. */
  readonly when: number;
  readonly length: number;
};

/** The pieces that sound between output times `start` and `end`, cut to that window. */
export function piecesInWindow(pieces: readonly SoundPiece[], start: number, end: number): WindowedPiece[] {
  const out: WindowedPiece[] = [];
  for (const piece of pieces) {
    const lo = Math.max(start, piece.at), hi = Math.min(end, piece.at + piece.length);
    if (hi - lo <= 1e-9) continue;
    out.push({ piece, from: piece.from + (lo - piece.at), when: lo - start, length: hi - lo });
  }
  return out;
}

/**
 * A WaveShaper curve over [-1, 1]: unchanged up to `SOFT_CLIP_KNEE`, then bending smoothly toward
 * ±1 instead of clipping hard. Stateless, so segment boundaries cannot be heard in it.
 */
export function softClipCurve(points = 4097): Float32Array<ArrayBuffer> {
  const curve = new Float32Array(points);
  const room = 1 - SOFT_CLIP_KNEE;
  for (let i = 0; i < points; i++) {
    const x = (i / (points - 1)) * 2 - 1;
    const a = Math.abs(x);
    const y = a <= SOFT_CLIP_KNEE ? a : SOFT_CLIP_KNEE + room * Math.tanh((a - SOFT_CLIP_KNEE) / room);
    curve[i] = Math.sign(x) * y;
  }
  return curve;
}

/**
 * Where a decoded chunk's samples go in a buffer that starts at time `start` and is `length`
 * frames long at `rate`: copy `count` frames from chunk frame `from` to buffer frame `to`. Null when
 * the chunk lies wholly outside. Decoders hand out chunks that straddle the requested window; the
 * parts outside it are dropped here, and a gap between chunks stays silent.
 */
export function chunkPlacement(start: number, rate: number, length: number, chunkTime: number, chunkFrames: number): { from: number; to: number; count: number } | null {
  const at = Math.round((chunkTime - start) * rate);
  const from = Math.max(0, -at), to = Math.max(0, at);
  const count = Math.min(chunkFrames - from, length - to);
  return count > 0 ? { from, to, count } : null;
}

/**
 * The decoded audio for one window of a piece: `buffer`, with the window's first moment `offset`
 * seconds in. Null means the source has no sound (a video without an audio track).
 */
export type LoadedSound = { readonly buffer: AudioBuffer; readonly offset: number } | null;
/** Loads one window of a piece's sound. `signal` is the mix's own, so a decoder can stop early. */
export type SoundLoader = (piece: SoundPiece, from: number, length: number, signal?: AbortSignal) => Promise<LoadedSound>;

export type MixOptions = {
  readonly sampleRate?: number;
  readonly signal?: AbortSignal;
  /** Test seam. */
  readonly createContext?: (channels: number, frames: number, sampleRate: number) => OfflineAudioContext;
};

/**
 * Render the clip's sound segment by segment, calling `onSegment` with each finished one in order.
 * Resolves false (and calls nothing) when nothing sounds at all: the clip then has no audio track.
 * A loader that throws fails the mix; a source that turns out silent (`null`) is just silent.
 */
export async function mixClipSound(
  pieces: readonly SoundPiece[],
  duration: number,
  load: SoundLoader,
  onSegment: (segment: AudioBuffer) => Promise<void> | void,
  options: MixOptions = {},
): Promise<boolean> {
  if (pieces.length === 0 || !(duration > 0)) return false;
  const rate = options.sampleRate ?? CLIP_SAMPLE_RATE;
  const make = options.createContext ?? ((channels, length, sampleRate) => new OfflineAudioContext({ numberOfChannels: channels, length, sampleRate }));
  const totalFrames = Math.round(duration * rate);
  const segmentFrames = Math.round(MIX_SEGMENT_SECONDS * rate);
  const curve = softClipCurve();
  // Checked before every stage that starts new work (each load, each render), not only once a
  // segment: a cancel during the first of several loads must not start the rest.
  const stop = () => { if (options.signal?.aborted) throw new DOMException("mix aborted", "AbortError"); };
  for (let first = 0; first < totalFrames; first += segmentFrames) {
    stop();
    const frames = Math.min(segmentFrames, totalFrames - first);
    // Frame-exact segment edges, so consecutive segments tile the output with no gap or overlap.
    const start = first / rate, end = (first + frames) / rate;
    const context = make(2, frames, rate);
    const shaper = context.createWaveShaper();
    shaper.curve = curve;
    shaper.connect(context.destination);
    for (const windowed of piecesInWindow(pieces, start, end)) {
      const sound = await load(windowed.piece, windowed.from, windowed.length, options.signal);
      stop();
      if (!sound) continue;
      const source = context.createBufferSource();
      source.buffer = sound.buffer;
      const gain = context.createGain();
      // The envelope from absolute time: its value where this window begins, then each corner.
      gain.gain.setValueAtTime(envelopeAt(windowed.piece, start + windowed.when), windowed.when);
      const close = windowed.when + windowed.length;
      let last = windowed.when;
      for (const [time, level] of envelopeCorners(windowed.piece)) {
        const local = time - start;
        if (local > windowed.when && local <= close) { gain.gain.linearRampToValueAtTime(level, local); last = local; }
      }
      // A fade cut by the segment's end: carry the line on to where the window stops, so the next
      // segment (which starts from `envelopeAt` there) continues it without a step.
      if (close - last > 1e-9) gain.gain.linearRampToValueAtTime(envelopeAt(windowed.piece, start + close), close);
      source.connect(gain).connect(shaper);
      source.start(windowed.when, sound.offset, windowed.length);
    }
    stop();
    const rendered = await context.startRendering();
    stop();
    await onSegment(rendered);
  }
  return true;
}
