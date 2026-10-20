/**
 * The clip maker's document: everything the person has put on a clip before posting it.
 *
 * It grows the cut list (`video-edit.ts`) into a small composition, in the spirit of a cartoon
 * maker rather than a full editor:
 *
 * - `layers[0]` is the video the person added, with its cut list. Its kept time IS the clip: the
 *   output is exactly as long as what layer 0 keeps, and every other time in this document is in
 *   that OUTPUT timeline.
 * - `layers[1..2]` are extra clips (at most `MAX_VIDEO_LAYERS` in all), each with its own cut list,
 *   placed at an output time and drawn picture-in-picture in a box. Any layer can be muted.
 * - `overlays` are text (in one of the app's text effects) or images, shown between a start and an
 *   end, and moved by keyframes. Keyframes come from dragging the overlay: while paused a drag sets
 *   where it is at that moment; while playing the whole drag is RECORDED as its path (see
 *   `recordPath`), Cartoon Maker style.
 * - `audio` items are music or sound effects placed at an output time, each with its own volume.
 *
 * Like the cut list, every function is pure and returns the same object when nothing changed, so
 * the shared history can tell an edit from a refusal by identity. Media is referenced by `source`
 * ids; the bytes live outside the document (the editor's media bin), so the document stays small,
 * serialisable and safe to keep in undo history.
 *
 * Bounds (`MAX_*`) keep every per-frame loop in the renderer short and the history small.
 */
import { initialEdit, isUnedited, keptDuration, outputToSource, type VideoEdit } from "./video-edit.ts";

export const MAX_VIDEO_LAYERS = 3;
export const MAX_OVERLAYS = 24;
export const MAX_AUDIO_ITEMS = 32;
/** Keyframes per overlay; a recorded path is simplified to fit (see `recordPath`). */
export const MAX_KEYFRAMES = 900;
/** How long a newly added overlay shows when the clip has room for it. */
export const DEFAULT_OVERLAY_SECONDS = 3;
/** The shortest anything may last on the timeline. */
export const MIN_ITEM_SECONDS = 0.1;
/** A recorded path keeps a point only where leaving it out would move the overlay this far (in frame
 * fractions: 0.4%, under 3 px on a 640-wide clip, so the motion looks the same with few keys). */
export const PATH_TOLERANCE = 0.004;
/** Two keyframes closer than this are the same moment (one 60 Hz frame). */
const SAME_MOMENT = 1 / 60;

/** Where an overlay is at one moment: its centre as fractions of the frame, its height as a
 * fraction of the frame height, and its rotation in degrees (clockwise). */
export type Placement = { readonly x: number; readonly y: number; readonly scale: number; readonly rotation: number };
export type Keyframe = Placement & { readonly t: number };

export type TextContent = {
  readonly kind: "text";
  readonly text: string;
  /** A text effect id from `message-effects.ts`, or null for plain text. */
  readonly effect: string | null;
  /** A `#rrggbb` colour. Effects that colour their own text ignore it. */
  readonly color: string;
};
export type ImageContent = {
  readonly kind: "image";
  readonly source: string;
  /** Width over height of the image, so its box keeps the picture's shape. */
  readonly aspect: number;
};
export type OverlayContent = TextContent | ImageContent;

export type Overlay = {
  readonly id: string;
  readonly start: number;
  readonly end: number;
  readonly content: OverlayContent;
  /** Ascending by `t`; never empty. One keyframe means the overlay stands still. */
  readonly keyframes: readonly Keyframe[];
};

/** A rectangle in frame fractions: left, top, width, height. */
export type Box = { readonly x: number; readonly y: number; readonly w: number; readonly h: number };

export type VideoLayer = {
  readonly id: string;
  readonly source: string;
  /** The source video's own length, which its cut list is drawn against. */
  readonly duration: number;
  readonly edit: VideoEdit;
  /** Output time where its first kept frame shows. Always 0 for layer 0. */
  readonly start: number;
  readonly muted: boolean;
  /** 0..1 */
  readonly volume: number;
  /** Where it is drawn. Layer 0 always fills the frame. */
  readonly box: Box;
};

export type AudioKind = "music" | "sfx";
export type AudioItem = {
  readonly id: string;
  readonly kind: AudioKind;
  readonly source: string;
  /** The sound's own length. */
  readonly duration: number;
  /** Output time it starts at. */
  readonly start: number;
  /** How far into the sound it starts (a trimmed head). */
  readonly offset: number;
  /** How long it plays, from `offset`. */
  readonly length: number;
  readonly volume: number;
  readonly muted: boolean;
};

export type ClipDoc = {
  /** `[0]` is the clip itself; at most `MAX_VIDEO_LAYERS`. */
  readonly layers: readonly VideoLayer[];
  readonly overlays: readonly Overlay[];
  readonly audio: readonly AudioItem[];
  /** Counter for new ids, so ids are deterministic and never reused within a document. */
  readonly seq: number;
};

const FULL_FRAME: Box = { x: 0, y: 0, w: 1, h: 1 };
/** Where a new picture-in-picture layer goes: the top-right corner, a third of the frame wide. */
const PIP_BOX: Box = { x: 0.63, y: 0.04, w: 0.33, h: 0.33 };
const TEXT_PLACEMENT: Placement = { x: 0.5, y: 0.82, scale: 0.12, rotation: 0 };
const IMAGE_PLACEMENT: Placement = { x: 0.5, y: 0.5, scale: 0.3, rotation: 0 };

function clamp(value: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, value));
}
const finite = (n: number) => Number.isFinite(n);

/** A fresh document for a video `duration` seconds long, untouched. */
export function newClipDoc(source: string, duration: number): ClipDoc {
  return {
    layers: [{
      id: "v1", source, duration, edit: initialEdit(duration), start: 0, muted: false, volume: 1, box: FULL_FRAME,
    }],
    overlays: [],
    audio: [],
    seq: 1,
  };
}

/** The clip's own layer. */
export function baseLayer(doc: ClipDoc): VideoLayer {
  return doc.layers[0];
}

/** The output's length: what layer 0 keeps. */
export function clipDuration(doc: ClipDoc): number {
  return keptDuration(baseLayer(doc).edit);
}

/**
 * True when posting would send the original file unchanged: nothing cut, trimmed, added or muted.
 * The caller then uploads the original bytes; nothing is re-rendered.
 */
export function isUntouched(doc: ClipDoc): boolean {
  const base = baseLayer(doc);
  return isUnedited(base.edit) && !needsCompositor(doc);
}

/**
 * True when the output needs every frame drawn (anything beyond cutting layer 0): an overlay or a
 * second video that shows, a sound that is heard, or the clip's own sound turned off or down.
 * Cut-only edits keep the lighter re-encode path that copies kept frames as they are.
 *
 * Only what reaches the output counts: an overlay or video that starts after a trimmed end, or a
 * muted (or silent) added sound, changes nothing in the posted clip and must not force a full
 * re-render of it (nor its length limit).
 */
export function needsCompositor(doc: ClipDoc): boolean {
  const base = baseLayer(doc);
  const total = clipDuration(doc);
  const shows = (start: number) => start < total;
  return doc.overlays.some((o) => shows(o.start)) || doc.layers.slice(1).some((l) => shows(l.start))
    || doc.audio.some((a) => !a.muted && a.volume > 0 && shows(a.start)) || base.muted || base.volume < 1;
}

function takeId(doc: ClipDoc, prefix: string): [string, ClipDoc] {
  return [`${prefix}${doc.seq + 1}`, { ...doc, seq: doc.seq + 1 }];
}

// --- Layer 0's cuts -----------------------------------------------------------------------------

/** Replace layer 0's cut list (after a blade, trim, remove or undo of those). */
export function withBaseEdit(doc: ClipDoc, edit: VideoEdit): ClipDoc {
  const base = baseLayer(doc);
  if (edit === base.edit) return doc;
  return { ...doc, layers: [{ ...base, edit }, ...doc.layers.slice(1)] };
}

// --- Overlays -----------------------------------------------------------------------------------

/** An overlay's window starting at `at`: `DEFAULT_OVERLAY_SECONDS` long, kept inside the clip. */
function windowAt(doc: ClipDoc, at: number): [number, number] | null {
  const total = clipDuration(doc);
  if (!(total >= MIN_ITEM_SECONDS)) return null;
  const start = clamp(finite(at) ? at : 0, 0, total - MIN_ITEM_SECONDS);
  return [start, Math.min(total, start + DEFAULT_OVERLAY_SECONDS)];
}

function addOverlay(doc: ClipDoc, at: number, content: OverlayContent, placement: Placement): ClipDoc {
  if (doc.overlays.length >= MAX_OVERLAYS) return doc;
  const span = windowAt(doc, at);
  if (!span) return doc;
  const [id, next] = takeId(doc, "o");
  const overlay: Overlay = { id, start: span[0], end: span[1], content, keyframes: [{ t: span[0], ...placement }] };
  return { ...next, overlays: [...doc.overlays, overlay] };
}

/** Add a text overlay at output time `at`. Empty text is refused. */
export function addText(doc: ClipDoc, at: number, text: string, effect: string | null = null, color = "#ffffff"): ClipDoc {
  const trimmed = text.trim();
  if (!trimmed) return doc;
  return addOverlay(doc, at, { kind: "text", text: trimmed, effect, color }, TEXT_PLACEMENT);
}

/** Add an image overlay at output time `at`. `aspect` is the picture's width over its height. */
export function addImage(doc: ClipDoc, at: number, source: string, aspect: number): ClipDoc {
  if (!source || !(aspect > 0) || !finite(aspect)) return doc;
  return addOverlay(doc, at, { kind: "image", source, aspect }, IMAGE_PLACEMENT);
}

function updateOverlayById(doc: ClipDoc, id: string, update: (o: Overlay) => Overlay): ClipDoc {
  const index = doc.overlays.findIndex((o) => o.id === id);
  if (index < 0) return doc;
  const next = update(doc.overlays[index]);
  if (next === doc.overlays[index]) return doc;
  const overlays = [...doc.overlays];
  overlays[index] = next;
  return { ...doc, overlays };
}

/** Change a text overlay's words, effect or colour. Empty text is refused. */
export function editText(doc: ClipDoc, id: string, patch: Partial<Omit<TextContent, "kind">>): ClipDoc {
  return updateOverlayById(doc, id, (o) => {
    if (o.content.kind !== "text") return o;
    const text = patch.text !== undefined ? patch.text.trim() : o.content.text;
    if (!text) return o;
    const content: TextContent = {
      kind: "text",
      text,
      effect: patch.effect !== undefined ? patch.effect : o.content.effect,
      color: patch.color !== undefined && /^#[0-9a-f]{6}$/i.test(patch.color) ? patch.color : o.content.color,
    };
    const same = content.text === o.content.text && content.effect === o.content.effect && content.color === o.content.color;
    return same ? o : { ...o, content };
  });
}

/**
 * Move or resize an overlay on the timeline. Kept inside the clip and at least `MIN_ITEM_SECONDS`
 * long.
 *
 * `carry` decides what happens to its path. A MOVE carries the keyframes with the start, so a
 * recorded path plays the same way at its new time. A TRIM (dragging either end of the bar) passes
 * `carry: false`: the path stays where it was in time and the overlay simply shows less of it.
 */
export function setOverlayTimes(doc: ClipDoc, id: string, start: number, end: number, carry = true): ClipDoc {
  const total = clipDuration(doc);
  return updateOverlayById(doc, id, (o) => {
    if (!finite(start) || !finite(end)) return o;
    const s = clamp(start, 0, Math.max(0, total - MIN_ITEM_SECONDS));
    const e = clamp(end, s + MIN_ITEM_SECONDS, total);
    if (s === o.start && e === o.end) return o;
    const shift = carry ? s - o.start : 0;
    const keyframes = shift === 0 ? o.keyframes : o.keyframes.map((k) => ({ ...k, t: k.t + shift }));
    return { ...o, start: s, end: e, keyframes };
  });
}

/** Forget an overlay's recorded motion: it stands still where it is at time `t`. */
export function clearPath(doc: ClipDoc, id: string, t: number): ClipDoc {
  return updateOverlayById(doc, id, (o) => {
    if (o.keyframes.length <= 1) return o;
    return { ...o, keyframes: [{ t: o.start, ...placementAt(o, finite(t) ? t : o.start) }] };
  });
}

export function removeOverlay(doc: ClipDoc, id: string): ClipDoc {
  const overlays = doc.overlays.filter((o) => o.id !== id);
  return overlays.length === doc.overlays.length ? doc : { ...doc, overlays };
}

/** The overlays showing at output time `t`, back to front. */
export function overlaysAt(doc: ClipDoc, t: number): Overlay[] {
  return doc.overlays.filter((o) => t >= o.start && t < o.end);
}

/** Where an overlay is at time `t`: straight lines between keyframes, held before the first and
 * after the last. */
export function placementAt(overlay: Overlay, t: number): Placement {
  const ks = overlay.keyframes;
  if (t <= ks[0].t) return pick(ks[0]);
  const last = ks[ks.length - 1];
  if (t >= last.t) return pick(last);
  // Binary search: recorded paths can hold hundreds of keyframes and this runs every frame.
  let lo = 0, hi = ks.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (ks[mid].t <= t) lo = mid; else hi = mid;
  }
  const a = ks[lo], b = ks[hi];
  const f = b.t === a.t ? 0 : (t - a.t) / (b.t - a.t);
  return {
    x: a.x + (b.x - a.x) * f,
    y: a.y + (b.y - a.y) * f,
    scale: a.scale + (b.scale - a.scale) * f,
    rotation: a.rotation + (b.rotation - a.rotation) * f,
  };
}

function pick(k: Keyframe): Placement {
  return { x: k.x, y: k.y, scale: k.scale, rotation: k.rotation };
}

function fitPlacement(p: Placement): Placement {
  return {
    x: clamp(p.x, -0.5, 1.5),
    y: clamp(p.y, -0.5, 1.5),
    scale: clamp(p.scale, 0.02, 2),
    rotation: clamp(p.rotation, -720, 720),
  };
}

/**
 * Move an overlay to `to` at time `t` (a drag while paused). An overlay that stands still (one
 * keyframe) simply moves; one with a path gets a keyframe at `t` (replacing one at that moment), so
 * the path bends through the new place.
 */
export function placeAt(doc: ClipDoc, id: string, t: number, to: { x: number; y: number }): ClipDoc {
  return updateOverlayById(doc, id, (o) => {
    if (!finite(t) || !finite(to.x) || !finite(to.y)) return o;
    const here = placementAt(o, t);
    const p = fitPlacement({ ...here, x: to.x, y: to.y });
    if (o.keyframes.length === 1) {
      const k = o.keyframes[0];
      return k.x === p.x && k.y === p.y ? o : { ...o, keyframes: [{ ...k, x: p.x, y: p.y }] };
    }
    const at = clamp(t, o.start, o.end);
    const keyframes = o.keyframes.filter((k) => Math.abs(k.t - at) >= SAME_MOMENT);
    keyframes.push({ t: at, ...p });
    keyframes.sort((a, b) => a.t - b.t);
    return { ...o, keyframes: capKeyframes(keyframes) };
  });
}

/** Scale (by `factor`) and turn (by `degrees`) an overlay everywhere along its path. */
export function transformOverlay(doc: ClipDoc, id: string, factor: number, degrees: number): ClipDoc {
  return updateOverlayById(doc, id, (o) => {
    if (!finite(factor) || !(factor > 0) || !finite(degrees) || (factor === 1 && degrees === 0)) return o;
    return { ...o, keyframes: o.keyframes.map((k) => ({ ...k, ...fitPlacement({ ...k, scale: k.scale * factor, rotation: k.rotation + degrees }) })) };
  });
}

/** One sample of a drag being recorded: output time and the pointer's place in frame fractions. */
export type PathSample = { readonly t: number; readonly x: number; readonly y: number };

/**
 * Record a drag made while playing as the overlay's path between the first and last sample.
 * Keyframes inside that stretch are replaced; the rest of the path is kept, so a person can record
 * a clip a stretch at a time. Size and rotation follow what the overlay already had at each moment.
 * The samples are simplified (`simplifyPath`) so a long recording stays within `MAX_KEYFRAMES`
 * without visibly changing the motion.
 */
export function recordPath(doc: ClipDoc, id: string, samples: readonly PathSample[]): ClipDoc {
  return updateOverlayById(doc, id, (o) => {
    const inside = samples
      .filter((s) => finite(s.t) && finite(s.x) && finite(s.y) && s.t >= o.start && s.t <= o.end)
      .slice()
      .sort((a, b) => a.t - b.t);
    if (inside.length === 0) return o;
    const from = inside[0].t;
    const to = inside[inside.length - 1].t;
    const kept = simplifyPath(inside, PATH_TOLERANCE);
    const recorded: Keyframe[] = kept.map((s) => ({ t: s.t, ...fitPlacement({ ...placementAt(o, s.t), x: s.x, y: s.y }) }));
    const outside = o.keyframes.filter((k) => k.t < from - SAME_MOMENT || k.t > to + SAME_MOMENT);
    const keyframes = [...outside, ...recorded].sort((a, b) => a.t - b.t);
    return { ...o, keyframes: capKeyframes(keyframes) };
  });
}

/**
 * Drop samples that a straight line between their neighbours already describes, within `tolerance`
 * (in frame fractions). Time is the line's parameter, so speed along the path is preserved, not
 * just its shape. The first and last samples are always kept.
 */
export function simplifyPath(samples: readonly PathSample[], tolerance: number): PathSample[] {
  if (samples.length <= 2) return samples.slice();
  const keep = new Uint8Array(samples.length);
  keep[0] = 1;
  keep[samples.length - 1] = 1;
  const stack: Array<[number, number]> = [[0, samples.length - 1]];
  while (stack.length) {
    const [a, b] = stack.pop()!;
    const sa = samples[a], sb = samples[b];
    let worst = -1, index = -1;
    for (let i = a + 1; i < b; i++) {
      const s = samples[i];
      const f = sb.t === sa.t ? 0 : (s.t - sa.t) / (sb.t - sa.t);
      const d = Math.hypot(s.x - (sa.x + (sb.x - sa.x) * f), s.y - (sa.y + (sb.y - sa.y) * f));
      if (d > worst) { worst = d; index = i; }
    }
    if (index > 0 && worst > tolerance) {
      keep[index] = 1;
      stack.push([a, index], [index, b]);
    }
  }
  return samples.filter((_, i) => keep[i]);
}

/** Over the cap, thin the path evenly, always keeping both ends. */
function capKeyframes(keyframes: Keyframe[]): Keyframe[] {
  if (keyframes.length <= MAX_KEYFRAMES) return keyframes;
  const out: Keyframe[] = [];
  const step = (keyframes.length - 1) / (MAX_KEYFRAMES - 1);
  for (let i = 0; i < MAX_KEYFRAMES; i++) out.push(keyframes[Math.round(i * step)]);
  return out;
}

// --- Extra video layers -------------------------------------------------------------------------

/** Add a picture-in-picture clip (a video `duration` long) at output time `at`. */
export function addLayer(doc: ClipDoc, source: string, duration: number, at: number): ClipDoc {
  if (doc.layers.length >= MAX_VIDEO_LAYERS || !source || !(duration > 0) || !finite(duration)) return doc;
  const total = clipDuration(doc);
  const start = clamp(finite(at) ? at : 0, 0, Math.max(0, total - MIN_ITEM_SECONDS));
  const [id, next] = takeId(doc, "v");
  const layer: VideoLayer = { id, source, duration, edit: initialEdit(duration), start, muted: false, volume: 1, box: PIP_BOX };
  return { ...next, layers: [...doc.layers, layer] };
}

export type LayerPatch = Partial<Pick<VideoLayer, "muted" | "volume" | "box" | "start" | "edit">>;

/** Change a layer. Layer 0 never moves, never shrinks and always fills the frame. */
export function updateLayer(doc: ClipDoc, id: string, patch: LayerPatch): ClipDoc {
  const index = doc.layers.findIndex((l) => l.id === id);
  if (index < 0) return doc;
  const layer = doc.layers[index];
  const base = index === 0;
  const next: VideoLayer = {
    ...layer,
    muted: patch.muted ?? layer.muted,
    volume: patch.volume !== undefined && finite(patch.volume) ? clamp(patch.volume, 0, 1) : layer.volume,
    edit: patch.edit ?? layer.edit,
    start: base || patch.start === undefined || !finite(patch.start)
      ? layer.start
      : clamp(patch.start, 0, Math.max(0, clipDuration(doc) - MIN_ITEM_SECONDS)),
    box: base || !patch.box ? layer.box : fitBox(patch.box),
  };
  const same = next.muted === layer.muted && next.volume === layer.volume && next.edit === layer.edit
    && next.start === layer.start && next.box === layer.box;
  if (same) return doc;
  const layers = [...doc.layers];
  layers[index] = next;
  return { ...doc, layers };
}

function fitBox(b: Box): Box {
  const w = clamp(finite(b.w) ? b.w : 0.33, 0.05, 1);
  const h = clamp(finite(b.h) ? b.h : 0.33, 0.05, 1);
  return { x: clamp(finite(b.x) ? b.x : 0, -w + 0.05, 0.95), y: clamp(finite(b.y) ? b.y : 0, -h + 0.05, 0.95), w, h };
}

/**
 * Draw an added layer one step further forward (`+1`) or back (`-1`) among the added layers.
 * Layer 0 is always the backdrop: nothing moves behind it and it moves nowhere.
 */
export function moveLayer(doc: ClipDoc, id: string, step: 1 | -1): ClipDoc {
  const index = doc.layers.findIndex((l) => l.id === id);
  const to = index + step;
  if (index <= 0 || to <= 0 || to >= doc.layers.length) return doc;
  const layers = [...doc.layers];
  [layers[index], layers[to]] = [layers[to], layers[index]];
  return { ...doc, layers };
}

/** Remove an added layer. Layer 0 cannot be removed. */
export function removeLayer(doc: ClipDoc, id: string): ClipDoc {
  const index = doc.layers.findIndex((l) => l.id === id);
  if (index <= 0) return doc;
  return { ...doc, layers: doc.layers.filter((_, i) => i !== index) };
}

/** Output times where a layer shows: [start, start + what it keeps), cut to the clip's end. */
export function layerSpan(doc: ClipDoc, layer: VideoLayer): { start: number; end: number } {
  const end = Math.min(clipDuration(doc), layer.start + keptDuration(layer.edit));
  return { start: layer.start, end: Math.max(layer.start, end) };
}

/** The layer's own source time at output time `t`, or null when it is not showing then. */
export function layerSourceTime(doc: ClipDoc, layer: VideoLayer, t: number): number | null {
  const span = layerSpan(doc, layer);
  if (!(t >= span.start && t < span.end)) return null;
  return outputToSource(layer.edit, t - layer.start);
}

// --- Audio --------------------------------------------------------------------------------------

/** Add music or a sound effect (a sound `duration` long) at output time `at`, cut to the clip. */
export function addAudio(doc: ClipDoc, kind: AudioKind, source: string, duration: number, at: number): ClipDoc {
  if (doc.audio.length >= MAX_AUDIO_ITEMS || !source || !(duration > 0) || !finite(duration)) return doc;
  const total = clipDuration(doc);
  if (!(total >= MIN_ITEM_SECONDS)) return doc;
  const start = clamp(finite(at) ? at : 0, 0, total - MIN_ITEM_SECONDS);
  const length = Math.min(duration, total - start);
  const [id, next] = takeId(doc, "a");
  const item: AudioItem = { id, kind, source, duration, start, offset: 0, length, volume: kind === "music" ? 0.8 : 1, muted: false };
  return { ...next, audio: [...doc.audio, item] };
}

export type AudioPatch = Partial<Pick<AudioItem, "start" | "offset" | "length" | "volume" | "muted">>;

/**
 * Change an audio item: move it, trim it (offset and length stay inside the sound), set volume.
 *
 * Its place and length are re-fitted to the clip only when the patch moves or trims it. A volume or
 * mute change leaves them alone: the clip may be shorter than the sound right now (cut down, to be
 * trimmed back out later), and re-fitting on every change would cut the sound to that shorter clip
 * for good. What sounds past the clip's end is cut at mix time (`soundPieces`), not here.
 */
export function updateAudio(doc: ClipDoc, id: string, patch: AudioPatch): ClipDoc {
  const index = doc.audio.findIndex((a) => a.id === id);
  if (index < 0) return doc;
  const a = doc.audio[index];
  const total = clipDuration(doc);
  let start = a.start, offset = a.offset, length = a.length;
  if (patch.start !== undefined || patch.offset !== undefined || patch.length !== undefined) {
    start = patch.start !== undefined && finite(patch.start) ? clamp(patch.start, 0, Math.max(0, total - MIN_ITEM_SECONDS)) : a.start;
    offset = patch.offset !== undefined && finite(patch.offset) ? clamp(patch.offset, 0, Math.max(0, a.duration - MIN_ITEM_SECONDS)) : a.offset;
    const wanted = patch.length !== undefined && finite(patch.length) ? patch.length : a.length;
    // Never below zero, whatever the clip's length: a sound placed past a cut-down clip's end
    // keeps a real length and sounds again once the clip is long enough.
    const room = Math.max(0, Math.min(a.duration - offset, total - start));
    length = clamp(wanted, Math.min(MIN_ITEM_SECONDS, room), room);
  }
  const volume = patch.volume !== undefined && finite(patch.volume) ? clamp(patch.volume, 0, 1) : a.volume;
  const muted = patch.muted ?? a.muted;
  if (start === a.start && offset === a.offset && length === a.length && volume === a.volume && muted === a.muted) return doc;
  const audio = [...doc.audio];
  audio[index] = { ...a, start, offset, length, volume, muted };
  return { ...doc, audio };
}

export function removeAudio(doc: ClipDoc, id: string): ClipDoc {
  const audio = doc.audio.filter((a) => a.id !== id);
  return audio.length === doc.audio.length ? doc : { ...doc, audio };
}
