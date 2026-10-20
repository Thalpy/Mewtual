/**
 * How the editor's live preview follows the playhead: which source time each extra video should
 * show, when a playing element has drifted far enough to be sent back, and which sounds to start
 * when playback starts. Pure decisions; the component owns the media elements and AudioContext.
 *
 * The base video is the clock (its element plays and the playhead follows it, jumping over removed
 * pieces). Everything else chases the clip's OUTPUT time derived from it: extra videos are seeked
 * when they drift, and music and sound effects are scheduled on the AudioContext from the output
 * time playback started at. That is good enough to judge a clip by eye and ear; the export does
 * not depend on any of it (it draws and mixes every frame offline, see `clip-export.ts`).
 */
import { layerSourceTime, type ClipDoc } from "./clip-doc.ts";
import { soundPieces } from "./clip-audio.ts";

/** A playing extra video further than this from where it should be is seeked back into place. */
export const DRIFT_SECONDS = 0.18;
/** A paused one is placed exactly (a frame and a bit), so a still frame matches the export. */
export const STILL_SECONDS = 0.02;

/** For each added layer, the source time it shows at output time `t`, or null when it is not showing. */
export function layerTargets(doc: ClipDoc, t: number): Map<string, number | null> {
  const out = new Map<string, number | null>();
  for (const layer of doc.layers.slice(1)) out.set(layer.id, layerSourceTime(doc, layer, t));
  return out;
}

/** Whether an element at `current` must be sent to `target`. */
export function needsResync(current: number, target: number, playing: boolean): boolean {
  return !(Math.abs(current - target) <= (playing ? DRIFT_SECONDS : STILL_SECONDS));
}

/** One added sound to start: its buffer `source`, after `delay` s, from `offset` s in, for `duration` s. */
export type SoundStart = { readonly source: string; readonly delay: number; readonly offset: number; readonly duration: number; readonly gain: number };

/**
 * The music and sound effects to start when playback starts at output time `t`: everything still
 * to come, and the rest of anything already sounding. Videos play their own sound and are left out.
 */
export function soundStarts(doc: ClipDoc, t: number): SoundStart[] {
  const starts: SoundStart[] = [];
  for (const piece of soundPieces(doc)) {
    if (piece.kind === "video") continue;
    const end = piece.at + piece.length;
    if (end <= t) continue;
    const late = Math.max(0, t - piece.at);
    starts.push({ source: piece.source, delay: Math.max(0, piece.at - t), offset: piece.from + late, duration: piece.length - late, gain: piece.gain });
  }
  return starts;
}
