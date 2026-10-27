/**
 * Draws one frame of a clip: the videos, then the text and pictures on top.
 *
 * This one function draws both the editor's stage (from `<video>` elements, while the person
 * works) and every frame of the export (from decoded frames). Sharing it is what makes the stage
 * truthful: where an overlay sits on the stage is where it is in the posted clip.
 *
 * Everything the frame needs comes in through `FrameAssets`, so this module never decodes or
 * fetches anything and stays a plain function of (document, time, pictures).
 *
 * Geometry, in frame pixels:
 * - Layer 0 fills the frame, letterboxed if its shape differs. Other layers fill their `box`
 *   (fractions of the frame), also letterboxed, so a picture is never stretched.
 * - An overlay is centred on its placement's (x, y). Its HEIGHT is `scale` times the frame height:
 *   for a picture the picture's height; for text the height of one line (two lines are twice as
 *   tall). Its width follows from its shape. It turns `rotation` degrees clockwise about its centre.
 */
import { layerSpan, placementAt, type ClipDoc, type Overlay, type Placement } from "./clip-doc.ts";
import { TEXT_LINE_HEIGHT } from "./text-fx-raster.ts";

export { TEXT_LINE_HEIGHT };
/** Bounds on the font size text is prepared at, in px: big enough to stay crisp, small enough to stay cheap. */
export const TEXT_FONT_MIN_PX = 12;
export const TEXT_FONT_MAX_PX = 400;

export type Rect = { readonly x: number; readonly y: number; readonly w: number; readonly h: number };

/** A picture with its own pixel size (a `<video>`'s `videoWidth`, a canvas's width...). */
export type Picture = { readonly image: CanvasImageSource; readonly width: number; readonly height: number };

/** A text overlay rendered to an image: `pad` px of room for glows on every side, at `fontPx`. */
export type TextSprite = Picture & { readonly pad: number; readonly fontPx: number };

export type FrameAssets = {
  /** The picture a video layer shows at this frame, or null while it has none (not decoded yet). */
  readonly layer: (layerId: string) => Picture | null;
  /** A picture overlay's image, by source. */
  readonly image: (source: string) => Picture | null;
  /** A text overlay drawn at `local` seconds since it appeared. */
  readonly text: (overlay: Overlay, local: number) => TextSprite | null;
};

/** The largest rectangle of shape `w:h` inside `box`, centred: letterboxing, never stretching. */
export function containRect(w: number, h: number, box: Rect): Rect {
  if (!(w > 0) || !(h > 0)) return box;
  const k = Math.min(box.w / w, box.h / h);
  const cw = w * k, ch = h * k;
  return { x: box.x + (box.w - cw) / 2, y: box.y + (box.h - ch) / 2, w: cw, h: ch };
}

/** The font size a text overlay is prepared at for a frame `frameHeight` px tall: its biggest moment. */
export function textFontPx(overlay: Overlay, frameHeight: number): number {
  const biggest = Math.max(...overlay.keyframes.map((k) => k.scale));
  const px = Math.ceil((biggest * frameHeight) / TEXT_LINE_HEIGHT);
  return Math.min(TEXT_FONT_MAX_PX, Math.max(TEXT_FONT_MIN_PX, px));
}

/**
 * An overlay's box at one placement, in frame pixels, before rotation: centre, width, height.
 * `aspect` is the content's width over its one-line height (an image's own shape; for text, its
 * laid-out width over one line). `lines` is the text's line count (1 for pictures).
 */
export function overlayBox(p: Placement, aspect: number, lines: number, W: number, H: number): { cx: number; cy: number; w: number; h: number } {
  const h = p.scale * H * Math.max(1, lines);
  return { cx: p.x * W, cy: p.y * H, w: p.scale * H * aspect, h };
}

/** The shape (`aspect`, `lines`) of a text sprite, for `overlayBox`. */
export function spriteShape(sprite: TextSprite): { aspect: number; lines: number } {
  const lineH = sprite.fontPx * TEXT_LINE_HEIGHT;
  const innerW = Math.max(1, sprite.width - sprite.pad * 2), innerH = Math.max(lineH, sprite.height - sprite.pad * 2);
  return { aspect: innerW / lineH, lines: Math.max(1, Math.round(innerH / lineH)) };
}

/** Whether frame point (px, py) is on an overlay's (rotated) box. For picking things on the stage. */
export function hitsBox(box: { cx: number; cy: number; w: number; h: number }, rotation: number, px: number, py: number): boolean {
  const r = (-rotation * Math.PI) / 180;
  const dx = px - box.cx, dy = py - box.cy;
  const lx = dx * Math.cos(r) - dy * Math.sin(r), ly = dx * Math.sin(r) + dy * Math.cos(r);
  return Math.abs(lx) <= box.w / 2 && Math.abs(ly) <= box.h / 2;
}

/** The minimal 2D context this module draws with (a real one, or a recorder in tests). */
export type Draw2D = Pick<CanvasRenderingContext2D,
  "fillStyle" | "fillRect" | "drawImage" | "save" | "restore" | "translate" | "rotate" | "imageSmoothingQuality">;

/** Draw the clip at output time `t` into a `W` x `H` frame. */
export function drawClipFrame(ctx: Draw2D, W: number, H: number, doc: ClipDoc, t: number, assets: FrameAssets): void {
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, W, H);
  ctx.imageSmoothingQuality = "high";
  doc.layers.forEach((layer, index) => {
    const span = layerSpan(doc, layer);
    if (index > 0 && !(t >= span.start && t < span.end)) return;
    const picture = assets.layer(layer.id);
    if (!picture) return;
    const box: Rect = index === 0 ? { x: 0, y: 0, w: W, h: H } : { x: layer.box.x * W, y: layer.box.y * H, w: layer.box.w * W, h: layer.box.h * H };
    const r = containRect(picture.width, picture.height, box);
    ctx.drawImage(picture.image, r.x, r.y, r.w, r.h);
  });
  for (const overlay of doc.overlays) {
    if (!(t >= overlay.start && t < overlay.end)) continue;
    const at = placementAt(overlay, t);
    let picture: Picture | null;
    let box: { cx: number; cy: number; w: number; h: number };
    if (overlay.content.kind === "image") {
      picture = assets.image(overlay.content.source);
      if (!picture) continue;
      box = overlayBox(at, overlay.content.aspect, 1, W, H);
      ctx.save();
      ctx.translate(box.cx, box.cy);
      ctx.rotate((at.rotation * Math.PI) / 180);
      ctx.drawImage(picture.image, -box.w / 2, -box.h / 2, box.w, box.h);
      ctx.restore();
    } else {
      const sprite = assets.text(overlay, t - overlay.start);
      if (!sprite) continue;
      // The sprite was laid out at `fontPx`; scale it so one line is `scale * H` tall. Its padding
      // (room for glows) scales with it and spills past the box, as the glow does in chat.
      const k = (at.scale * H) / (sprite.fontPx * TEXT_LINE_HEIGHT);
      ctx.save();
      ctx.translate(at.x * W, at.y * H);
      ctx.rotate((at.rotation * Math.PI) / 180);
      ctx.drawImage(sprite.image, (-sprite.width * k) / 2, (-sprite.height * k) / 2, sprite.width * k, sprite.height * k);
      ctx.restore();
    }
  }
}
