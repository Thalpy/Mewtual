/**
 * The clip maker's stage: where the person picks, drags, scales and turns what is on the clip.
 * Pure geometry, so the component only wires pointer events to these functions.
 *
 * Coordinates: the stage is the element on screen; the FRAME is the clip's picture letterboxed
 * inside it (`stageFrame`). The document stores positions as fractions of the frame (see
 * `clip-doc.ts`), so a pointer is turned into frame fractions (`toFraction`) before anything is
 * changed, and the same document draws identically on the small stage and in the full-size export.
 */
import { layerSpan, placementAt, type Box, type ClipDoc, type Overlay, type PathSample } from "./clip-doc.ts";
import { containRect, hitsBox, overlayBox, spriteShape, type Rect, type TextSprite } from "./clip-render.ts";

/** The clip's frame inside a `stageW` x `stageH` stage: its own shape, centred and letterboxed. */
export function stageFrame(stageW: number, stageH: number, clipW: number, clipH: number): Rect {
  return containRect(clipW > 0 ? clipW : 16, clipH > 0 ? clipH : 9, { x: 0, y: 0, w: Math.max(0, stageW), h: Math.max(0, stageH) });
}

/** A stage point as fractions of the frame (outside the frame gives values outside 0..1). */
export function toFraction(frame: Rect, px: number, py: number): { x: number; y: number } {
  return { x: frame.w > 0 ? (px - frame.x) / frame.w : 0.5, y: frame.h > 0 ? (py - frame.y) / frame.h : 0.5 };
}

/** The shape `overlayBox` needs: an image's own; text's from its rendered sprite, or a guess from
 * its length until the sprite exists (wide enough to grab, corrected on the next draw). */
export function overlayShape(overlay: Overlay, sprite: TextSprite | null): { aspect: number; lines: number } {
  if (overlay.content.kind === "image") return { aspect: overlay.content.aspect, lines: 1 };
  if (sprite) return spriteShape(sprite);
  const lines = overlay.content.text.split("\n");
  return { aspect: Math.max(1, ...lines.map((l) => l.length)) * 0.62, lines: lines.length };
}

/** An overlay's box in stage pixels at output time `t`, and its rotation. */
export function overlayStageBox(overlay: Overlay, t: number, frame: Rect, shape: { aspect: number; lines: number }) {
  const at = placementAt(overlay, t);
  const box = overlayBox(at, shape.aspect, shape.lines, frame.w, frame.h);
  return { cx: frame.x + box.cx, cy: frame.y + box.cy, w: box.w, h: box.h, rotation: at.rotation };
}

/** The front-most overlay showing at `t` under stage point (px, py), or null. Later overlays draw
 * on top, so they are tried first. */
export function pickOverlay(
  doc: ClipDoc, t: number, frame: Rect, px: number, py: number,
  shapeOf: (o: Overlay) => { aspect: number; lines: number },
): string | null {
  for (let i = doc.overlays.length - 1; i >= 0; i--) {
    const o = doc.overlays[i];
    if (!(t >= o.start && t < o.end)) continue;
    const box = overlayStageBox(o, t, frame, shapeOf(o));
    if (hitsBox(box, box.rotation, px, py)) return o.id;
  }
  return null;
}

/** An added layer's box in stage pixels. */
export function layerStageBox(box: Box, frame: Rect): Rect {
  return { x: frame.x + box.x * frame.w, y: frame.y + box.y * frame.h, w: box.w * frame.w, h: box.h * frame.h };
}

/** The front-most added layer showing at `t` under (px, py), or null. Layer 0 is never picked. */
export function pickLayer(doc: ClipDoc, t: number, frame: Rect, px: number, py: number): string | null {
  for (let i = doc.layers.length - 1; i >= 1; i--) {
    const layer = doc.layers[i];
    const span = layerSpan(doc, layer);
    if (!(t >= span.start && t < span.end)) continue;
    const r = layerStageBox(layer.box, frame);
    if (px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h) return layer.id;
  }
  return null;
}

/**
 * The scale-and-turn handle: dragging it from `from` to `to` around the overlay's centre scales by
 * the change in distance and turns by the change in angle. A drag that starts on the centre
 * itself has no direction to measure, and changes nothing.
 */
export function scaleTurn(cx: number, cy: number, fromX: number, fromY: number, toX: number, toY: number): { factor: number; degrees: number } {
  const d0 = Math.hypot(fromX - cx, fromY - cy), d1 = Math.hypot(toX - cx, toY - cy);
  if (d0 < 4 || d1 < 1) return { factor: 1, degrees: 0 };
  let degrees = ((Math.atan2(toY - cy, toX - cx) - Math.atan2(fromY - cy, fromX - cx)) * 180) / Math.PI;
  if (degrees > 180) degrees -= 360;
  if (degrees < -180) degrees += 360;
  return { factor: d1 / d0, degrees };
}

export type Corner = "nw" | "ne" | "sw" | "se";

/**
 * Resize an added layer's box by dragging one corner to (fx, fy) in frame fractions. The opposite
 * corner stays put and the box keeps its shape (the width decides; the height follows).
 */
export function resizeBox(box: Box, corner: Corner, fx: number, fy: number): Box {
  const ratio = box.w > 0 ? box.h / box.w : 1;
  const anchorX = corner === "nw" || corner === "sw" ? box.x + box.w : box.x;
  const anchorY = corner === "nw" || corner === "ne" ? box.y + box.h : box.y;
  const w = Math.min(1, Math.max(0.05, Math.abs(fx - anchorX)));
  const h = w * ratio;
  return {
    x: corner === "nw" || corner === "sw" ? anchorX - w : anchorX,
    y: corner === "nw" || corner === "ne" ? anchorY - h : anchorY,
    w, h,
  };
}

/** Samples closer together than this are one moment (twice a 60 Hz frame). */
export const RECORD_STEP_SECONDS = 1 / 120;

/**
 * Collects the samples of a drag made while the clip plays (Cartoon Maker recording). Samples must
 * move forward in time: a playback that jumps backwards (a loop, a seek) ends the usable stretch,
 * and later samples are ignored rather than folded into the path out of order.
 */
export class PathRecorder {
  readonly samples: PathSample[] = [];
  add(t: number, x: number, y: number): boolean {
    if (!Number.isFinite(t) || !Number.isFinite(x) || !Number.isFinite(y)) return false;
    const last = this.samples[this.samples.length - 1];
    if (last && t < last.t + RECORD_STEP_SECONDS) return false;
    this.samples.push({ t, x: Math.min(1.5, Math.max(-0.5, x)), y: Math.min(1.5, Math.max(-0.5, y)) });
    return true;
  }
}
