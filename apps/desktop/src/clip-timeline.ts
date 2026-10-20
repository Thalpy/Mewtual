/**
 * Geometry for the clip maker's timeline. Pure: the component turns these numbers into positions.
 *
 * The horizontal axis is the clip's own SOURCE time (layer 0, what the V1 lane and its filmstrip
 * show), so trims and cuts read the way the video does, with removed stretches still visible and
 * dimmed. Everything else in the document lives in OUTPUT time (the clip as posted, see
 * `clip-doc.ts`), so overlays, sounds and extra layers are drawn at the source time where their
 * output time falls. A bar that spans a cut therefore stretches across the dimmed gap, which is
 * honest: the gap is not in the clip, and the bar is not interrupted by it.
 */
import { baseLayer, clipDuration, layerSpan, type ClipDoc, type Overlay } from "./clip-doc.ts";
import { keptRanges, sourceToOutput, type VideoEdit } from "./video-edit.ts";

/** Source (axis) time for output time `o`. `edge: "end"` picks the end of a kept range when `o`
 * falls exactly where two kept ranges meet, so a bar ending there ends before the gap. */
export function outputToAxis(edit: VideoEdit, o: number, edge: "start" | "end" = "start"): number {
  const ranges = keptRanges(edit);
  if (!ranges.length) return edit.inPoint;
  let acc = 0;
  for (const r of ranges) {
    const len = r.end - r.start;
    const inside = edge === "end" ? o <= acc + len : o < acc + len;
    if (inside) return r.start + Math.max(0, o - acc);
    acc += len;
  }
  return ranges[ranges.length - 1].end;
}

/** Output time for axis time `s` (time in removed stretches lands on the next kept frame). */
export function axisToOutput(edit: VideoEdit, s: number): number {
  return sourceToOutput(edit, s);
}

/** Where an output-time item `[start, end)` sits on the axis. */
export function axisSpan(doc: ClipDoc, start: number, end: number): { from: number; to: number } {
  const edit = baseLayer(doc).edit;
  const total = clipDuration(doc);
  const s = Math.min(Math.max(0, start), total), e = Math.min(Math.max(s, end), total);
  return { from: outputToAxis(edit, s, "start"), to: outputToAxis(edit, e, "end") };
}

/**
 * Pack items into rows so none overlaps another in its row: greedy by start time, each into the
 * first row that is free by then. Returns each item's row, in the items' own order.
 */
export function packRows(items: readonly { readonly start: number; readonly end: number }[]): number[] {
  const order = items.map((_, i) => i).sort((a, b) => items[a].start - items[b].start || a - b);
  const rowEnds: number[] = [];
  const rows = new Array<number>(items.length).fill(0);
  for (const i of order) {
    let row = rowEnds.findIndex((end) => end <= items[i].start + 1e-9);
    if (row < 0) { row = rowEnds.length; rowEnds.push(0); }
    rowEnds[row] = items[i].end;
    rows[i] = row;
  }
  return rows;
}

/** Output times of an overlay's keyframes that show on its bar (a still overlay shows none). */
export function keyframeMarks(overlay: Overlay): number[] {
  if (overlay.keyframes.length < 2) return [];
  return overlay.keyframes.map((k) => k.t).filter((t) => t >= overlay.start - 1e-9 && t <= overlay.end + 1e-9);
}

/**
 * Thin marks (ascending positions, in percent of a bar) to at most one per `minGap`: a recorded
 * path can hold dozens of keyframes in a short stretch, which would draw as a solid smear. Ends are
 * always shown, so a path's start and finish stay visible.
 */
export function thinMarks(positions: readonly number[], minGap: number): number[] {
  if (positions.length <= 2) return positions.slice();
  const out = [positions[0]];
  const last = positions[positions.length - 1];
  for (const p of positions.slice(1, -1)) {
    if (p - out[out.length - 1] >= minGap && last - p >= minGap) out.push(p);
  }
  out.push(last);
  return out;
}

/** Heights of the timeline's parts, in px. */
export const LANE = { ruler: 16, v1: 40, layer: 20, row: 13, gap: 3 } as const;

export type LaneLayout = {
  /** Top of each lane, from the top of the lanes area (below the ruler). */
  readonly v1: number;
  readonly layers: readonly number[];
  readonly overlays: number;
  readonly audio: number;
  readonly overlayHeight: number;
  readonly audioHeight: number;
  /** Everything, ruler included. */
  readonly height: number;
};

/** Stack the lanes: V1, one lane per added layer, then text and stickers, then sound. Each of the
 * last two is at least two rows tall so a lone item never looks like a full-height block. */
export function laneLayout(addedLayers: number, overlayRows: number, audioRows: number): LaneLayout {
  const rowsHeight = (rows: number) => Math.max(2, rows) * (LANE.row + 1) + 2;
  let y = 0;
  const v1 = y; y += LANE.v1 + LANE.gap;
  const layers: number[] = [];
  for (let i = 0; i < addedLayers; i++) { layers.push(y); y += LANE.layer + LANE.gap; }
  const overlayHeight = rowsHeight(overlayRows);
  const overlays = y; y += overlayHeight + LANE.gap;
  const audioHeight = rowsHeight(audioRows);
  const audio = y; y += audioHeight;
  return { v1, layers, overlays, audio, overlayHeight, audioHeight, height: LANE.ruler + 4 + y };
}

/** An added layer's bar on the axis. */
export function layerAxisSpan(doc: ClipDoc, layerIndex: number): { from: number; to: number } {
  const layer = doc.layers[layerIndex];
  const span = layerSpan(doc, layer);
  return axisSpan(doc, span.start, span.end);
}

/** A drag along the axis, from `from` to `to` (axis seconds), as a change in output seconds. */
export function outputDelta(edit: VideoEdit, from: number, to: number): number {
  return axisToOutput(edit, to) - axisToOutput(edit, from);
}
