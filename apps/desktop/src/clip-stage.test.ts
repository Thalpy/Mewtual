import assert from "node:assert/strict";
import test from "node:test";
import { addAudio, addImage, addLayer, addText, newClipDoc, transformOverlay, updateAudio, updateLayer } from "./clip-doc.ts";
import {
  overlayShape, overlayStageBox, PathRecorder, pickLayer, pickOverlay, RECORD_STEP_SECONDS, resizeBox, scaleTurn, stageFrame, toFraction,
} from "./clip-stage.ts";
import { layerTargets, needsResync, soundStarts } from "./clip-preview.ts";

const close = (a: number, b: number, eps = 1e-9) => Math.abs(a - b) <= eps;

test("the frame is the clip's shape letterboxed in the stage, and points become frame fractions", () => {
  const frame = stageFrame(600, 300, 1920, 1080);
  assert.ok(close(frame.h, 300) && close(frame.w, 300 * 16 / 9));
  assert.ok(close(frame.x, (600 - frame.w) / 2) && frame.y === 0);
  const mid = toFraction(frame, frame.x + frame.w / 2, frame.h / 4);
  assert.ok(close(mid.x, 0.5) && close(mid.y, 0.25));
  assert.ok(toFraction(frame, 0, 0).x < 0, "outside the frame is outside 0..1");
});

test("picking finds the front-most overlay showing now, honouring its turn", () => {
  let doc = newClipDoc("v", 10);
  doc = addImage(doc, 0, "back", 1);
  doc = addImage(doc, 0, "front", 1);
  const [back, front] = doc.overlays.map((o) => o.id);
  const frame = stageFrame(400, 400, 1000, 1000);
  const shape = (o: (typeof doc.overlays)[number]) => overlayShape(o, null);
  assert.equal(pickOverlay(doc, 1, frame, 200, 200, shape), front, "both under the point: the later one is on top");
  assert.equal(pickOverlay(doc, 1, frame, 5, 5, shape), null);
  assert.equal(pickOverlay(doc, 5, frame, 200, 200, shape), null, "neither shows at 5 s");
  // A wide sticker turned upright no longer covers a point off to its side.
  doc = addImage(newClipDoc("v", 10), 0, "wide", 4);
  const id = doc.overlays[0].id;
  const side = { x: 200 + 0.3 * 400 * 4 / 2 - 4, y: 200 };
  assert.equal(pickOverlay(doc, 1, frame, side.x, side.y, shape), id);
  doc = transformOverlay(doc, id, 1, 90);
  assert.equal(pickOverlay(doc, 1, frame, side.x, side.y, shape), null);
  void back;
});

test("text is grabbable before its picture exists, then sized by it", () => {
  const doc = addText(newClipDoc("v", 10), 0, "HELLO\nTHERE");
  const guess = overlayShape(doc.overlays[0], null);
  assert.equal(guess.lines, 2);
  assert.ok(guess.aspect > 2, "wide enough for five letters");
  const real = overlayShape(doc.overlays[0], { image: {} as CanvasImageSource, width: 500, height: 300, pad: 50, fontPx: 100 });
  assert.equal(real.lines, 2);
  const frame = stageFrame(100, 100, 100, 100);
  const box = overlayStageBox(doc.overlays[0], 0, frame, real);
  assert.ok(close(box.h, doc.overlays[0].keyframes[0].scale * 100 * 2));
});

test("picking an added layer ignores the clip itself and layers not showing yet", () => {
  let doc = addLayer(newClipDoc("v", 10), "pip", 3, 2);
  doc = updateLayer(doc, doc.layers[1].id, { box: { x: 0.5, y: 0.5, w: 0.4, h: 0.4 } });
  const frame = stageFrame(100, 100, 100, 100);
  assert.equal(pickLayer(doc, 3, frame, 70, 70), doc.layers[1].id);
  assert.equal(pickLayer(doc, 1, frame, 70, 70), null, "starts at 2 s");
  assert.equal(pickLayer(doc, 3, frame, 10, 10), null, "the clip itself is never picked");
});

test("the handle scales by distance and turns by angle, and a grab on the centre does nothing", () => {
  const twice = scaleTurn(0, 0, 10, 0, 20, 0);
  assert.ok(close(twice.factor, 2) && close(twice.degrees, 0));
  const quarter = scaleTurn(0, 0, 10, 0, 0, 10);
  assert.ok(close(quarter.factor, 1) && close(quarter.degrees, 90));
  const wrap = scaleTurn(0, 0, -10, 1, -10, -1);
  assert.ok(Math.abs(wrap.degrees) < 20, "turning across the back edge is a small turn, not 350 degrees");
  assert.deepEqual(scaleTurn(0, 0, 1, 1, 50, 50), { factor: 1, degrees: 0 });
});

test("resizing a layer box anchors the opposite corner and keeps the shape", () => {
  const box = { x: 0.6, y: 0.1, w: 0.3, h: 0.2 };
  const grown = resizeBox(box, "se", 1.05, 0.9);
  assert.ok(close(grown.x, 0.6) && close(grown.y, 0.1));
  assert.ok(close(grown.w, 0.45) && close(grown.h, 0.3));
  const fromNw = resizeBox(box, "nw", 0.8, 0);
  assert.ok(close(fromNw.x + fromNw.w, 0.9) && close(fromNw.y + fromNw.h, 0.3), "the south-east corner stayed put");
  assert.ok(close(resizeBox(box, "se", 0.6, 0.1).w, 0.05), "never smaller than the minimum");
});

test("a recording keeps forward-moving samples only, spaced at least a step apart", () => {
  const r = new PathRecorder();
  assert.equal(r.add(1, 0.2, 0.2), true);
  assert.equal(r.add(1 + RECORD_STEP_SECONDS / 2, 0.3, 0.3), false, "too soon after the last");
  assert.equal(r.add(1.1, 9, -9), true);
  assert.deepEqual(r.samples[1], { t: 1.1, x: 1.5, y: -0.5 }, "held near the frame");
  assert.equal(r.add(0.5, 0.5, 0.5), false, "a jump back in time does not fold into the path");
  assert.equal(r.add(Number.NaN, 0, 0), false);
});

test("extra videos follow the output time, and only real drift is corrected", () => {
  let doc = addLayer(newClipDoc("v", 10), "pip", 3, 2);
  const id = doc.layers[1].id;
  assert.equal(layerTargets(doc, 1).get(id), null);
  assert.ok(close(layerTargets(doc, 3.5).get(id)!, 1.5));
  assert.equal(layerTargets(doc, 5.5).get(id), null, "ended");
  doc = updateLayer(doc, id, { start: 0 });
  assert.equal(needsResync(1.0, 1.1, true), false, "small drift while playing is left alone");
  assert.equal(needsResync(1.0, 1.5, true), true);
  assert.equal(needsResync(1.0, 1.05, false), true, "a paused frame is placed exactly");
});

test("starting playback mid-clip starts what is still to come and the rest of what is sounding", () => {
  let doc = newClipDoc("v", 10);
  doc = addAudio(doc, "music", "song", 30, 1);
  doc = addAudio(doc, "sfx", "pow", 0.5, 6);
  doc = addAudio(doc, "sfx", "zap", 0.3, 2);
  doc = updateAudio(doc, doc.audio[0].id, { offset: 2 });
  const starts = soundStarts(doc, 4);
  assert.deepEqual(starts.map((s) => s.source), ["song", "pow"], "the zap at 2 s is over");
  const song = starts[0];
  assert.ok(close(song.delay, 0) && close(song.offset, 2 + 3) && close(song.duration, 9 - 3));
  assert.ok(close(starts[1].delay, 2) && close(starts[1].offset, 0));
});
