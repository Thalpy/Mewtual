import assert from "node:assert/strict";
import test from "node:test";
import {
  HISTORY_LIMIT, MAX_CUTS, MIN_PIECE_SECONDS, canSplitAt, exportPlan, formatClock, initialEdit, isUnedited,
  keptDuration, keptRanges, mapAudioChunk, mapVideoFrame, moveCut, pieceIndexAt, pieces, playbackJump, pushEdit,
  redo, replaceEdit, rulerLabel, rulerTicks, setInPoint, setOutPoint, setRemoved, sourceToOutput, splitAt,
  startHistory, toggleRemoved, undo, type VideoEdit,
} from "./video-edit.ts";

/** Floating-point sums of tenths are not exact; compare times to a microsecond. */
function near(actual: number, expected: number, message = "") {
  assert.ok(Math.abs(actual - expected) < 1e-6, `${message} expected ${expected}, got ${actual}`);
}

function ranges(edit: VideoEdit): Array<[number, number]> {
  return keptRanges(edit).map((r) => [r.start, r.end]);
}

// --- Construction and pieces --------------------------------------------------------------------

test("a fresh edit is one kept piece spanning the whole source and counts as unedited", () => {
  const edit = initialEdit(12);
  assert.deepEqual(pieces(edit), [{ start: 0, end: 12, index: 0, removed: false }]);
  assert.deepEqual(ranges(edit), [[0, 12]]);
  assert.equal(isUnedited(edit), true);
  assert.equal(keptDuration(edit), 12);
});

test("initialEdit refuses durations a timeline cannot be drawn against", () => {
  for (const bad of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
    assert.throws(() => initialEdit(bad), RangeError, String(bad));
  }
});

test("pieceIndexAt is half-open, and the very end belongs to the last piece", () => {
  const edit = splitAt(splitAt(initialEdit(10), 4), 7);
  assert.equal(pieceIndexAt(edit, 0), 0);
  assert.equal(pieceIndexAt(edit, 3.999), 0);
  assert.equal(pieceIndexAt(edit, 4), 1);
  assert.equal(pieceIndexAt(edit, 7), 2);
  assert.equal(pieceIndexAt(edit, 10), 2);
});

// --- Blade --------------------------------------------------------------------------------------

test("a blade cut makes two pieces that inherit the cut piece's state, in either order of cutting", () => {
  const a = splitAt(splitAt(initialEdit(10), 7), 4);
  const b = splitAt(splitAt(initialEdit(10), 4), 7);
  assert.deepEqual(a, b);
  assert.deepEqual(a.cuts, [4, 7]);
  const removedMiddle = setRemoved(a, 1, true);
  const cutAgain = splitAt(removedMiddle, 5.5);
  assert.deepEqual(cutAgain.removed, [false, true, true, false], "both halves of a removed piece stay removed");
});

test("a cut on its own changes nothing in the upload", () => {
  const edit = splitAt(initialEdit(10), 5);
  assert.equal(isUnedited(edit), true);
  assert.deepEqual(ranges(edit), [[0, 10]], "touching kept pieces merge into one range");
});

test("blade refusals hand back the very same object so no undo step is recorded", () => {
  const edit = splitAt(initialEdit(10), 5);
  for (const t of [0, MIN_PIECE_SECONDS / 2, 5, 5 + MIN_PIECE_SECONDS / 2, 10, 10 - MIN_PIECE_SECONDS / 2, -1, 11, Number.NaN]) {
    assert.equal(canSplitAt(edit, t), false, String(t));
    assert.equal(splitAt(edit, t), edit, String(t));
  }
  // Exactly the minimum away is allowed.
  assert.notEqual(splitAt(edit, 5 + MIN_PIECE_SECONDS), edit);
});

test("the number of cuts is bounded", () => {
  let edit = initialEdit(1000);
  for (let i = 1; i <= MAX_CUTS + 5; i++) edit = splitAt(edit, i);
  assert.equal(edit.cuts.length, MAX_CUTS);
  assert.equal(edit.removed.length, MAX_CUTS + 1);
  assert.equal(canSplitAt(edit, 500.5), false);
});

// --- Remove / restore ---------------------------------------------------------------------------

test("removing a middle piece leaves two ranges and restoring it merges them back", () => {
  const cut = splitAt(splitAt(initialEdit(10), 3), 6);
  const removed = toggleRemoved(cut, 1);
  assert.deepEqual(ranges(removed), [[0, 3], [6, 10]]);
  near(keptDuration(removed), 7);
  assert.equal(isUnedited(removed), false);
  const restored = toggleRemoved(removed, 1);
  assert.deepEqual(ranges(restored), [[0, 10]]);
  assert.equal(isUnedited(restored), true);
});

test("setRemoved refuses out-of-range indices and no-op changes", () => {
  const edit = splitAt(initialEdit(10), 5);
  assert.equal(setRemoved(edit, -1, true), edit);
  assert.equal(setRemoved(edit, 2, true), edit);
  assert.equal(setRemoved(edit, 0.5, true), edit);
  assert.equal(setRemoved(edit, 0, false), edit);
});

test("removing every piece keeps nothing and has no export plan", () => {
  const edit = setRemoved(setRemoved(splitAt(initialEdit(10), 5), 0, true), 1, true);
  assert.deepEqual(keptRanges(edit), []);
  assert.equal(keptDuration(edit), 0);
  assert.equal(exportPlan(edit), null);
});

// --- Dragging cuts and trim handles -------------------------------------------------------------

test("a dragged cut is held between its neighbours with a minimum piece to spare", () => {
  const edit = splitAt(splitAt(initialEdit(10), 3), 6);
  near(moveCut(edit, 0, 4).cuts[0], 4);
  near(moveCut(edit, 0, 100).cuts[0], 6 - MIN_PIECE_SECONDS, "cannot cross the next cut");
  near(moveCut(edit, 0, -5).cuts[0], MIN_PIECE_SECONDS, "cannot reach the start");
  near(moveCut(edit, 1, 100).cuts[1], 10 - MIN_PIECE_SECONDS, "cannot reach the end");
  assert.equal(moveCut(edit, 2, 4), edit, "no such cut");
  assert.equal(moveCut(edit, 0, Number.NaN), edit);
  assert.equal(moveCut(edit, 0, 3), edit, "no movement is no step");
});

test("trim handles clamp to the source and never cross each other", () => {
  const edit = initialEdit(10);
  near(setInPoint(edit, 2).inPoint, 2);
  near(setInPoint(edit, -3).inPoint, 0);
  assert.equal(setInPoint(edit, -3), edit, "already at 0");
  near(setInPoint(edit, 50).inPoint, 10 - MIN_PIECE_SECONDS);
  near(setOutPoint(edit, 8).outPoint, 8);
  near(setOutPoint(edit, 50).outPoint, 10);
  const narrow = setOutPoint(setInPoint(edit, 4), 0);
  near(narrow.outPoint, 4 + MIN_PIECE_SECONDS, "out stays after in");
  assert.equal(setInPoint(edit, Number.NaN), edit);
  assert.equal(setOutPoint(edit, Number.POSITIVE_INFINITY), edit);
});

test("trimming clips the kept ranges and does not add cuts", () => {
  const edit = setOutPoint(setInPoint(splitAt(initialEdit(10), 5), 2), 8);
  assert.deepEqual(edit.cuts, [5]);
  assert.deepEqual(ranges(edit), [[2, 8]]);
  assert.equal(isUnedited(edit), false);
  // A piece wholly outside the trim window contributes nothing even when kept.
  const outside = setOutPoint(splitAt(initialEdit(10), 5), 4);
  assert.deepEqual(ranges(outside), [[0, 4]]);
});

test("handles within a millisecond of the ends still count as unedited", () => {
  const edit = setOutPoint(setInPoint(initialEdit(10), 0.0004), 9.9996);
  assert.equal(isUnedited(edit), true);
  assert.equal(isUnedited(setInPoint(initialEdit(10), 0.01)), false);
});

// --- Preview playback ---------------------------------------------------------------------------

test("playback jumps over trimmed and removed time and ends after the last kept frame", () => {
  const edit = setInPoint(toggleRemoved(splitAt(splitAt(initialEdit(10), 4), 6), 1), 1);
  // kept: [1,4] and [6,10]
  assert.deepEqual(playbackJump(edit, 0), { seek: 1 });
  assert.equal(playbackJump(edit, 1), null);
  assert.equal(playbackJump(edit, 3.99), null);
  assert.deepEqual(playbackJump(edit, 4), { seek: 6 });
  assert.deepEqual(playbackJump(edit, 5), { seek: 6 });
  assert.equal(playbackJump(edit, 9.99), null);
  assert.deepEqual(playbackJump(edit, 10), { end: true });
  const trimmedEnd = setOutPoint(edit, 8);
  assert.deepEqual(playbackJump(trimmedEnd, 8.5), { end: true });
});

test("sourceToOutput counts only kept time and holds inside removed time", () => {
  const edit = toggleRemoved(splitAt(splitAt(initialEdit(10), 4), 6), 1);
  near(sourceToOutput(edit, 2), 2);
  near(sourceToOutput(edit, 4), 4);
  near(sourceToOutput(edit, 5), 4, "inside removed time");
  near(sourceToOutput(edit, 7), 5);
  near(sourceToOutput(edit, 10), 8);
  near(sourceToOutput(setInPoint(edit, 1), 0.5), 0, "before the in-point");
});

// --- History ------------------------------------------------------------------------------------

test("undo and redo walk the history, and a new edit clears the redo branch", () => {
  const a = initialEdit(10);
  const b = splitAt(a, 5);
  const c = toggleRemoved(b, 0);
  let h = pushEdit(pushEdit(startHistory(a), b), c);
  h = undo(h);
  assert.equal(h.present, b);
  h = undo(h);
  assert.equal(h.present, a);
  assert.equal(undo(h), h, "nothing left to undo");
  h = redo(h);
  assert.equal(h.present, b);
  const d = setInPoint(b, 1);
  h = pushEdit(h, d);
  assert.equal(h.future.length, 0);
  assert.equal(redo(h), h, "nothing left to redo");
});

test("a refused operation is not an undo step", () => {
  const a = splitAt(initialEdit(10), 5);
  const h = startHistory(a);
  assert.equal(pushEdit(h, splitAt(a, 5)), h);
});

test("a drag is one undo step however many moves it makes", () => {
  const a = splitAt(initialEdit(10), 5);
  let h = startHistory(a);
  h = pushEdit(h, moveCut(h.present, 0, 5.5));
  for (const t of [6, 6.5, 7]) h = replaceEdit(h, moveCut(h.present, 0, t));
  near(h.present.cuts[0], 7);
  h = undo(h);
  assert.equal(h.present, a);
});

test("history depth is bounded, dropping the oldest steps", () => {
  let h = startHistory(initialEdit(1000));
  for (let i = 1; i <= HISTORY_LIMIT + 20; i++) h = pushEdit(h, setInPoint(h.present, i));
  assert.equal(h.past.length, HISTORY_LIMIT);
  near(h.past[0].inPoint, 20, "the first twenty steps were dropped");
});

// --- Export plan and mapping --------------------------------------------------------------------

test("the export plan is relative to the first kept frame and runs to the end when the tail is kept", () => {
  const edit = setInPoint(toggleRemoved(splitAt(splitAt(initialEdit(10), 4), 6), 1), 1);
  const plan = exportPlan(edit);
  assert.ok(plan);
  near(plan.trimStart, 1);
  assert.equal(plan.trimEnd, null, "the tail is kept, so no finite end");
  near(plan.ranges[0].start, 0);
  near(plan.ranges[0].end, 3);
  near(plan.ranges[1].start, 5);
  assert.equal(plan.ranges[1].end, Number.POSITIVE_INFINITY);
  assert.deepEqual(plan.outStarts, [0, 3]);
  near(plan.outputDuration, 7);
});

test("the export plan has a finite end when the tail is trimmed or removed", () => {
  const trimmed = exportPlan(setOutPoint(initialEdit(10), 8));
  assert.equal(trimmed?.trimEnd, 8);
  assert.equal(trimmed?.ranges[0].end, 8);
  const removedTail = exportPlan(toggleRemoved(splitAt(initialEdit(10), 9), 1));
  assert.equal(removedTail?.trimEnd, 9);
});

test("video frames inside removed time are dropped and later frames move up by the removed time", () => {
  // Keep [0,4] and [6,10]; 25 fps.
  const plan = exportPlan(toggleRemoved(splitAt(splitAt(initialEdit(10), 4), 6), 1));
  assert.ok(plan);
  const dt = 0.04;
  const kept = mapVideoFrame(plan, 1, dt);
  near(kept?.timestamp ?? -1, 1);
  near(kept?.duration ?? -1, dt);
  assert.equal(mapVideoFrame(plan, 4, dt), null);
  assert.equal(mapVideoFrame(plan, 5, dt), null);
  const after = mapVideoFrame(plan, 6, dt);
  assert.ok(after);
  near(after.timestamp, 4);
  near(after.duration, dt);
  const late = mapVideoFrame(plan, 9.96, dt);
  near(late?.timestamp ?? -1, 7.96);
});

test("a frame straddling a cut keeps only its kept part, so the picture changes exactly at the cut", () => {
  const plan = exportPlan(toggleRemoved(splitAt(splitAt(initialEdit(10), 4.01), 6.01), 1));
  assert.ok(plan);
  // Frame [4.00, 4.04) is kept for 0.01 s before the removed piece.
  const before = mapVideoFrame(plan, 4.0, 0.04);
  near(before?.timestamp ?? -1, 4.0);
  near(before?.duration ?? -1, 0.01);
  // Frame [6.00, 6.04) starts in removed time; only [6.01, 6.04) is kept, placed at the join.
  const after = mapVideoFrame(plan, 6.0, 0.04);
  near(after?.timestamp ?? -1, 4.01);
  near(after?.duration ?? -1, 0.03);
});

test("a zero-duration frame is kept only when its instant is kept", () => {
  const plan = exportPlan(toggleRemoved(splitAt(splitAt(initialEdit(10), 4), 6), 1));
  assert.ok(plan);
  assert.deepEqual(mapVideoFrame(plan, 2, 0), { timestamp: 2, duration: 0 });
  assert.equal(mapVideoFrame(plan, 5, 0), null);
  near(mapVideoFrame(plan, 7, 0)?.timestamp ?? -1, 5);
});

test("audio chunks are sliced at cuts to the sample and placed back to back", () => {
  // Keep [0,1] and [1.5,3] of a 48 kHz source, chunks of 1024 frames.
  const plan = exportPlan(toggleRemoved(splitAt(splitAt(initialEdit(3), 1), 1.5), 1));
  assert.ok(plan);
  const rate = 48000;
  const whole = mapAudioChunk(plan, 0, 1024, rate);
  assert.deepEqual(whole, [{ startFrame: 0, endFrame: 1024, timestamp: 0 }]);
  // A chunk crossing 1.0 s keeps only its frames before the cut.
  const crossing = mapAudioChunk(plan, 0.99, 1024, rate);
  assert.equal(crossing.length, 1);
  assert.equal(crossing[0].startFrame, 0);
  assert.equal(crossing[0].endFrame, Math.round(0.01 * rate));
  // Wholly removed.
  assert.deepEqual(mapAudioChunk(plan, 1.2, 1024, rate), []);
  // A chunk crossing 1.5 s keeps its tail, placed at output 1.0.
  const resuming = mapAudioChunk(plan, 1.49, 1024, rate);
  assert.equal(resuming.length, 1);
  assert.equal(resuming[0].startFrame, Math.round(0.01 * rate));
  assert.equal(resuming[0].endFrame, 1024);
  near(resuming[0].timestamp, 1);
});

test("an audio chunk spanning a removed sliver survives on both sides of it", () => {
  // Remove [1.00, 1.01): shorter than one 1024-frame chunk at 48 kHz (21.3 ms).
  const plan = exportPlan(toggleRemoved(splitAt(splitAt(initialEdit(3), 1.0), 1.0 + MIN_PIECE_SECONDS), 1));
  assert.ok(plan);
  const slices = mapAudioChunk(plan, 0.995, 9600, 48000); // 0.2 s chunk covers the whole gap
  assert.equal(slices.length, 2);
  near(slices[0].timestamp, 0.995);
  assert.equal(slices[0].endFrame, Math.round(0.005 * 48000));
  near(slices[1].timestamp, 1.0, "the far side lands right after the near side");
  assert.equal(slices[1].startFrame, Math.round((0.005 + MIN_PIECE_SECONDS) * 48000));
  assert.equal(slices[1].endFrame, 9600);
});

test("mapAudioChunk ignores malformed chunks rather than producing slices", () => {
  const plan = exportPlan(initialEdit(3));
  assert.ok(plan);
  assert.deepEqual(mapAudioChunk(plan, 0, 0, 48000), []);
  assert.deepEqual(mapAudioChunk(plan, 0, 1024, 0), []);
  assert.deepEqual(mapAudioChunk(plan, 0, Number.NaN, 48000), []);
});

test("mapped output time is continuous across every cut for both media kinds", () => {
  // Walk 25 fps video and 1024-frame audio across a three-range plan and check the output
  // timelines advance without overlap and by exactly the kept time.
  let edit = initialEdit(6);
  edit = splitAt(splitAt(splitAt(splitAt(edit, 1), 2), 3.5), 4.25);
  edit = toggleRemoved(toggleRemoved(edit, 1), 3);
  const plan = exportPlan(edit);
  assert.ok(plan);
  near(plan.outputDuration, 1 + 1.5 + 1.75);

  let videoOut = 0;
  for (let i = 0; i < 6 * 25; i++) {
    const placed = mapVideoFrame(plan, i / 25, 1 / 25);
    if (!placed) continue;
    near(placed.timestamp, videoOut, `video frame ${i}`);
    videoOut = placed.timestamp + placed.duration;
  }
  near(videoOut, plan.outputDuration);

  const rate = 48000;
  const total = 6 * rate;
  let audioFrames = 0;
  for (let start = 0; start < total; start += 1024) {
    // The decoder's last chunk is short: a source does not have sound past its own end.
    for (const slice of mapAudioChunk(plan, start / rate, Math.min(1024, total - start), rate)) {
      assert.ok(Math.abs(slice.timestamp * rate - audioFrames) <= 1, `audio at frame ${start}`);
      audioFrames += slice.endFrame - slice.startFrame;
    }
  }
  assert.ok(Math.abs(audioFrames - plan.outputDuration * rate) <= 3, `audio frames ${audioFrames}`);
});

// --- Presentation -------------------------------------------------------------------------------

test("ruler ticks use the smallest round step within the budget and stop at the end", () => {
  assert.deepEqual(rulerTicks(10, 8), { step: 2, ticks: [0, 2, 4, 6, 8, 10] });
  assert.deepEqual(rulerTicks(0.9, 8), { step: 0.2, ticks: [0, 0.2, 0.4, 0.6, 0.8] });
  const long = rulerTicks(7200, 8);
  assert.equal(long.step, 900);
  assert.equal(long.ticks.at(-1), 7200);
  const huge = rulerTicks(100 * 3600, 8);
  assert.equal(huge.step, 13 * 3600, "past the table the step is whole hours");
  assert.ok(huge.ticks.length - 1 <= 8);
  assert.deepEqual(rulerTicks(0, 8), { step: 0, ticks: [] });
  assert.deepEqual(rulerTicks(Number.POSITIVE_INFINITY, 8), { step: 0, ticks: [] });
  assert.deepEqual(rulerTicks(10, 0), { step: 0, ticks: [] });
});

test("ruler labels drop the tenths only when every tick is a whole second", () => {
  assert.equal(rulerLabel(65, 5), "1:05");
  assert.equal(rulerLabel(0.4, 0.2), "0:00.4");
  assert.equal(rulerLabel(3600, 900), "1:00:00");
});

test("formatClock reads minutes and tenths, with hours only when needed", () => {
  assert.equal(formatClock(0), "0:00.0");
  assert.equal(formatClock(5.25), "0:05.2");
  assert.equal(formatClock(65.3), "1:05.3");
  assert.equal(formatClock(3723.45), "1:02:03.4");
  assert.equal(formatClock(-4), "0:00.0");
  assert.equal(formatClock(Number.NaN), "0:00.0");
  assert.equal(formatClock(0.3), "0:00.3", "a tenth that is not exact in binary still floors to itself");
});
