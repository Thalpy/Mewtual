import assert from "node:assert/strict";
import test from "node:test";
import { AUDITION_CALL, AUDITION_LANE_NONCE, auditionPlayback } from "./studio-audition.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { validateJamTake } from "./jam-recorder.ts";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";

const ctx = { group: "a".repeat(64), exporter: "b".repeat(64) };

test("an audition is one validated note-on/note-off take the archival planner accepts", () => {
  const playback = auditionPlayback(ctx, legacyJamPatch("square"), 72, 250);
  assert.equal(playback.gridMs, 250);
  assert.equal(playback.take.call, AUDITION_CALL);
  assert.deepEqual(playback.take.parts, [ctx.exporter]);
  assert.deepEqual(playback.take.lanes, [{ src: 0, sn: AUDITION_LANE_NONCE }]);
  assert.equal(playback.take.patches.length, 1);
  assert.deepEqual(playback.take.events.map((e) => [e.ms, "on" in e ? e.on : "d"]), [[0, 1], [250, 0]]);
  const on = playback.take.events[0];
  assert.ok("w" in on && on.w === "square" && on.p === 0 && on.n === 72);
  // The same validator the jukebox and Score export use must accept it unchanged.
  const again = validateJamTake(playback.take);
  assert.ok(again.ok);
  // And the planner must not refuse it (it is what StudioAudioSession runs it through).
  const plan = planJamArchivalPlayback(playback.take);
  assert.equal(plan.take.events.length, 2);
});

test("the hold is exact and bounded like a saved marker", () => {
  assert.equal(auditionPlayback(ctx, legacyJamPatch("sine"), 60, 1).take.events[1].ms, 1);
  assert.equal(auditionPlayback(ctx, legacyJamPatch("sine"), 60, 600_000).gridMs, 600_000);
  for (const bad of [0, -1, 600_001, 1.5, Number.NaN]) assert.throws(() => auditionPlayback(ctx, legacyJamPatch("sine"), 60, bad), /hold/);
});

test("rejections mirror the marker contract: note range, recipe validity, provenance", () => {
  assert.throws(() => auditionPlayback(ctx, legacyJamPatch("sine"), 128, 250), /MIDI/);
  assert.throws(() => auditionPlayback(ctx, legacyJamPatch("sine"), -1, 250), /MIDI/);
  assert.throws(() => auditionPlayback(ctx, legacyJamPatch("sine"), 60.5, 250), /MIDI/);
  assert.throws(() => auditionPlayback(ctx, { ...legacyJamPatch("sine"), o: [] } as never, 60, 250));
  assert.throws(() => auditionPlayback({ group: "", exporter: ctx.exporter }, legacyJamPatch("sine"), 60, 250), /group/);
  assert.throws(() => auditionPlayback({ group: ctx.group, exporter: "" }, legacyJamPatch("sine"), 60, 250), /identity/);
});

test("the caller's patch object is not what the take carries", () => {
  const patch = legacyJamPatch("triangle");
  const playback = auditionPlayback(ctx, patch, 60, 100);
  patch.o[0].l = 1;
  assert.notEqual(playback.take.patches[0].o[0].l, 1, "a later mutation of the input must not reach the planned take");
});
