import assert from "node:assert/strict";
import test from "node:test";
import { addImage, addLayer, addText, newClipDoc, placeAt, transformOverlay, type Overlay } from "./clip-doc.ts";
import {
  containRect, drawClipFrame, hitsBox, overlayBox, spriteShape, TEXT_FONT_MAX_PX, TEXT_FONT_MIN_PX, TEXT_LINE_HEIGHT,
  textFontPx, type Draw2D, type FrameAssets, type TextSprite,
} from "./clip-render.ts";

const close = (a: number, b: number, eps = 1e-6) => Math.abs(a - b) <= eps;

test("pictures are letterboxed into their box, never stretched", () => {
  assert.deepEqual(containRect(1920, 1080, { x: 0, y: 0, w: 1280, h: 720 }), { x: 0, y: 0, w: 1280, h: 720 });
  const tall = containRect(1080, 1920, { x: 0, y: 0, w: 1280, h: 720 });
  assert.ok(close(tall.h, 720) && close(tall.w, 405) && close(tall.x, (1280 - 405) / 2) && tall.y === 0);
  const wide = containRect(400, 100, { x: 10, y: 20, w: 200, h: 200 });
  assert.ok(close(wide.w, 200) && close(wide.h, 50) && close(wide.y, 20 + 75));
  assert.deepEqual(containRect(0, 0, { x: 1, y: 2, w: 3, h: 4 }), { x: 1, y: 2, w: 3, h: 4 }, "an unknown size fills the box");
});

test("text is prepared at its biggest size for the frame, within bounds", () => {
  let doc = addText(newClipDoc("v", 10), 0, "Hi");
  const id = doc.overlays[0].id;
  const base = doc.overlays[0];
  assert.equal(textFontPx(base, 720), Math.ceil((base.keyframes[0].scale * 720) / TEXT_LINE_HEIGHT));
  doc = placeAt(doc, id, 1, { x: 0.5, y: 0.5 });
  doc = transformOverlay(doc, id, 2, 0);
  assert.ok(textFontPx(doc.overlays[0], 720) >= textFontPx(base, 720), "the biggest keyframe decides");
  const tiny: Overlay = { ...base, keyframes: [{ ...base.keyframes[0], scale: 0.001 }] };
  const huge: Overlay = { ...base, keyframes: [{ ...base.keyframes[0], scale: 5 }] };
  assert.equal(textFontPx(tiny, 720), TEXT_FONT_MIN_PX);
  assert.equal(textFontPx(huge, 4000), TEXT_FONT_MAX_PX);
});

test("an overlay's box is centred on its placement and one line is `scale` of the frame tall", () => {
  const box = overlayBox({ x: 0.25, y: 0.5, scale: 0.1, rotation: 0 }, 3, 1, 1000, 600);
  assert.deepEqual(box, { cx: 250, cy: 300, w: 180, h: 60 });
  assert.equal(overlayBox({ x: 0, y: 0, scale: 0.1, rotation: 0 }, 3, 2, 1000, 600).h, 120, "two lines, twice as tall");
});

test("a sprite's shape leaves its glow padding out", () => {
  const sprite: TextSprite = { image: {} as CanvasImageSource, fontPx: 100, pad: 60, width: 460 + 120, height: 115 + 120 };
  const shape = spriteShape(sprite);
  assert.ok(close(shape.aspect, 4) && shape.lines === 1);
  assert.equal(spriteShape({ ...sprite, height: 230 + 120 }).lines, 2);
});

test("picking honours rotation", () => {
  const box = { cx: 100, cy: 100, w: 200, h: 20 };
  assert.equal(hitsBox(box, 0, 190, 100), true);
  assert.equal(hitsBox(box, 0, 100, 150), false);
  assert.equal(hitsBox(box, 90, 190, 100), false, "turned upright, its far end is no longer there");
  assert.equal(hitsBox(box, 90, 100, 190), true);
});

// --- Drawing, recorded -------------------------------------------------------------------------------

function recorder() {
  const ops: unknown[][] = [];
  const ctx = {
    fillStyle: "", imageSmoothingQuality: "low",
    fillRect: (...a: unknown[]) => ops.push(["fillRect", ...a]),
    drawImage: (...a: unknown[]) => ops.push(["drawImage", ...a]),
    save: () => ops.push(["save"]),
    restore: () => ops.push(["restore"]),
    translate: (...a: unknown[]) => ops.push(["translate", ...a]),
    rotate: (...a: unknown[]) => ops.push(["rotate", ...a]),
  };
  return { ops, ctx: ctx as unknown as Draw2D };
}
const pic = (name: string, width: number, height: number) => ({ image: { name } as unknown as CanvasImageSource, width, height });
const named = (op: unknown[]) => (op[1] as { name: string }).name;

test("layers draw in order, extra ones only while showing and inside their box", () => {
  let doc = newClipDoc("base", 10);
  doc = addLayer(doc, "pip", 3, 2);
  const assets: FrameAssets = {
    layer: (id) => (id === doc.layers[0].id ? pic("base", 1280, 720) : pic("pip", 640, 360)),
    image: () => null,
    text: () => null,
  };
  const at = (t: number) => { const r = recorder(); drawClipFrame(r.ctx, 1280, 720, doc, t, assets); return r.ops; };
  const before = at(1).filter((op) => op[0] === "drawImage");
  assert.deepEqual(before.map(named), ["base"], "the second video has not started yet");
  const during = at(3).filter((op) => op[0] === "drawImage");
  assert.deepEqual(during.map(named), ["base", "pip"]);
  const pipBox = doc.layers[1].box;
  const [, , x, y, w, h] = during[1] as number[];
  assert.ok(x >= pipBox.x * 1280 - 1e-6 && y >= pipBox.y * 720 - 1e-6, "inside its box");
  assert.ok(x + w <= (pipBox.x + pipBox.w) * 1280 + 1e-6 && y + h <= (pipBox.y + pipBox.h) * 720 + 1e-6);
  assert.deepEqual(at(5.5).filter((op) => op[0] === "drawImage").map(named), ["base"], "ended after its 3 s");
  const first = at(0)[0];
  assert.deepEqual(first, ["fillRect", 0, 0, 1280, 720], "the frame is cleared to black first");
});

test("overlays draw above the videos, in document order, only inside their time", () => {
  let doc = newClipDoc("base", 10);
  doc = addImage(doc, 1, "sticker", 2);
  doc = addText(doc, 1, "Hello");
  const assets: FrameAssets = {
    layer: () => pic("base", 100, 100),
    image: () => pic("sticker", 200, 100),
    text: () => ({ ...pic("text", 300, 100), pad: 10, fontPx: 50 }),
  };
  const r = recorder();
  drawClipFrame(r.ctx, 1000, 500, doc, 2, assets);
  assert.deepEqual(r.ops.filter((op) => op[0] === "drawImage").map(named), ["base", "sticker", "text"]);
  const early = recorder();
  drawClipFrame(early.ctx, 1000, 500, doc, 0.5, assets);
  assert.deepEqual(early.ops.filter((op) => op[0] === "drawImage").map(named), ["base"]);
});

test("text is scaled so one line is `scale` of the frame tall, and its glow spills past the box", () => {
  const doc = addText(newClipDoc("base", 10), 0, "Hello");
  const overlay = doc.overlays[0];
  const scale = overlay.keyframes[0].scale;
  const sprite: TextSprite = { ...pic("text", 400, 135), pad: 20, fontPx: 82 };
  let seenLocal = -1;
  const r = recorder();
  drawClipFrame(r.ctx, 1280, 720, doc, 1.25, { layer: () => null, image: () => null, text: (_, local) => { seenLocal = local; return sprite; } });
  assert.equal(seenLocal, 1.25 - overlay.start, "effects are timed from when the text appears");
  const draw = r.ops.find((op) => op[0] === "drawImage") as number[];
  const k = (scale * 720) / (82 * TEXT_LINE_HEIGHT);
  assert.ok(close(draw[4], 400 * k) && close(draw[5], 135 * k));
  assert.ok(close(draw[2], -200 * k) && close(draw[3], -67.5 * k), "drawn centred on the placement");
  const translate = r.ops.find((op) => op[0] === "translate") as number[];
  assert.ok(close(translate[1], overlay.keyframes[0].x * 1280) && close(translate[2], overlay.keyframes[0].y * 720));
});

test("a rotated picture turns about its centre and the context is restored after", () => {
  let doc = addImage(newClipDoc("base", 10), 0, "s", 1);
  doc = transformOverlay(doc, doc.overlays[0].id, 1, 30);
  const r = recorder();
  drawClipFrame(r.ctx, 100, 100, doc, 0.5, { layer: () => null, image: () => pic("s", 10, 10), text: () => null });
  const kinds = r.ops.map((op) => op[0]);
  assert.deepEqual(kinds.slice(1), ["save", "translate", "rotate", "drawImage", "restore"]);
  assert.ok(close(r.ops[3][1] as number, (30 * Math.PI) / 180));
});

test("missing pictures are skipped, not drawn as holes or errors", () => {
  let doc = addImage(newClipDoc("base", 10), 0, "gone", 1);
  doc = addText(doc, 0, "x");
  const r = recorder();
  drawClipFrame(r.ctx, 100, 100, doc, 0.5, { layer: () => null, image: () => null, text: () => null });
  assert.deepEqual(r.ops.map((op) => op[0]), ["fillRect"]);
});
