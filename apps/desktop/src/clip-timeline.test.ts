import assert from "node:assert/strict";
import test from "node:test";
import { addLayer, addText, newClipDoc, recordPath, withBaseEdit } from "./clip-doc.ts";
import { initialEdit, setInPoint, setRemoved, splitAt } from "./video-edit.ts";
import {
  axisSpan, axisToOutput, keyframeMarks, LANE, laneLayout, layerAxisSpan, outputDelta, outputToAxis, packRows, thinMarks,
} from "./clip-timeline.ts";

// 12 s clip: trimmed to start at 1, blade at 4 and 7, the middle piece removed. Kept: [1,4) [7,12).
const edit = setRemoved(splitAt(splitAt(setInPoint(initialEdit(12), 1), 4), 7), 1, true);

test("output time maps onto the clip's own time, skipping trimmed and removed stretches", () => {
  assert.equal(outputToAxis(edit, 0), 1);
  assert.equal(outputToAxis(edit, 2), 3);
  assert.equal(outputToAxis(edit, 3), 7, "a start at the seam is the first frame after the gap");
  assert.equal(outputToAxis(edit, 3, "end"), 4, "an end at the seam is the last frame before it");
  assert.equal(outputToAxis(edit, 5), 9);
  assert.equal(outputToAxis(edit, 99), 12, "past the end holds at the end");
  for (const o of [0, 0.5, 2.9, 3, 4.2, 7.9]) assert.ok(Math.abs(axisToOutput(edit, outputToAxis(edit, o)) - o) < 1e-9, `round trip ${o}`);
  assert.equal(axisToOutput(edit, 5), 3, "a point in the removed stretch is where the clip resumes");
});

test("a bar over a cut stretches across the dimmed gap", () => {
  const doc = withBaseEdit(newClipDoc("v", 12), edit);
  assert.deepEqual(axisSpan(doc, 2, 5), { from: 3, to: 9 });
  assert.deepEqual(axisSpan(doc, 1, 3), { from: 2, to: 4 });
  assert.deepEqual(axisSpan(doc, 6, 99), { from: 10, to: 12 }, "cut at the clip's end");
});

test("rows are packed so nothing overlaps within a row, touching ends included", () => {
  assert.deepEqual(packRows([]), []);
  // [0,2] row 0; [0.5,1] row 1; [1,3] row 1 again (it starts as [0.5,1] ends); [2,4] back in row 0.
  assert.deepEqual(packRows([{ start: 0, end: 2 }, { start: 1, end: 3 }, { start: 2, end: 4 }, { start: 0.5, end: 1 }]), [0, 1, 0, 1]);
  assert.deepEqual(packRows([{ start: 0, end: 5 }, { start: 1, end: 5 }, { start: 2, end: 5 }]), [0, 1, 2], "true overlaps get rows of their own");
  assert.deepEqual(packRows([{ start: 5, end: 6 }, { start: 0, end: 1 }]), [0, 0], "packed by start, reported in the given order");
});

test("keyframes show on a bar only when the overlay moves", () => {
  let doc = addText(newClipDoc("v", 10), 1, "Hi");
  const id = doc.overlays[0].id;
  assert.deepEqual(keyframeMarks(doc.overlays[0]), []);
  doc = recordPath(doc, id, [{ t: 1, x: 0, y: 0 }, { t: 1.5, x: 0.5, y: 0.9 }, { t: 2, x: 1, y: 0 }]);
  assert.deepEqual(keyframeMarks(doc.overlays[0]), [1, 1.5, 2]);
});

test("crowded keyframe marks are thinned to one per gap, ends always kept", () => {
  assert.deepEqual(thinMarks([], 5), []);
  assert.deepEqual(thinMarks([10, 11], 5), [10, 11], "two marks are both ends");
  assert.deepEqual(thinMarks([0, 1, 2, 6, 7, 12, 13, 14, 20], 5), [0, 6, 12, 20]);
  assert.deepEqual(thinMarks([0, 3, 4, 5], 5), [0, 5], "a mark too close to the last end yields to it");
});

test("lanes stack in order and grow with their rows", () => {
  const plain = laneLayout(0, 0, 0);
  assert.equal(plain.v1, 0);
  assert.equal(plain.overlays, LANE.v1 + LANE.gap);
  assert.ok(plain.audio > plain.overlays);
  const busy = laneLayout(2, 4, 3);
  assert.deepEqual(busy.layers, [LANE.v1 + LANE.gap, LANE.v1 + LANE.gap + LANE.layer + LANE.gap]);
  assert.ok(busy.overlayHeight > plain.overlayHeight);
  assert.ok(busy.height > plain.height);
  assert.equal(laneLayout(0, 1, 1).overlayHeight, plain.overlayHeight, "never thinner than two rows");
});

test("an added layer's bar and a drag's output change follow the clip's cuts", () => {
  let doc = withBaseEdit(newClipDoc("v", 12), edit);
  doc = addLayer(doc, "pip", 3, 2);
  assert.deepEqual(layerAxisSpan(doc, 1), { from: 3, to: 9 });
  assert.equal(outputDelta(edit, 2, 3), 1);
  assert.equal(outputDelta(edit, 3, 8), 2, "dragging across the gap moves by the kept time only");
});
