import assert from "node:assert/strict";
import test from "node:test";
import { TAKE_MAX_DURATION_MS, TAKE_PLAYBACK_EVENTS_PER_TICK, type JamTake } from "./jam-contract.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { archivalReleaseTailMs, planJamArchivalPlayback, planJamArchivalSeek } from "./jam-archival-plan.ts";

function take(): JamTake {
  return { v: 1, group: "group", call: "call", met: { bpm: 120, bpb: 4 }, parts: ["author"],
    lanes: [{ src: 0, sn: "a".repeat(16) }], patches: [legacyJamPatch("sine")], events: [
      { ms: 0, lane: 0, n: 60, on: 1, w: "sine", p: 0, q: 0 },
      { ms: TAKE_MAX_DURATION_MS, lane: 0, n: 60, on: 0, q: 1 },
    ] };
}

test("archival score holds and seek retain the authored ten-minute note", () => {
  const plan = planJamArchivalPlayback(take());
  assert.equal(plan.events[0].holdMs, TAKE_MAX_DURATION_MS);
  assert.equal(plan.events[0].offMs, TAKE_MAX_DURATION_MS);
  assert.deepEqual(planJamArchivalSeek(plan, 500_000), { next: 1, sounding: [0] });
  assert.deepEqual(planJamArchivalSeek(plan, 600_000), { next: 1, sounding: [] });
  assert.equal(archivalReleaseTailMs(plan), 120);
});

test("frame SFX use exact rational frame start and independent same-pitch lanes", () => {
  const input = take(); input.events = [];
  const plan = planJamArchivalPlayback(input, { fps: 24, frameCount: 999, sfx: [
    { frame: 1, patch: 0, note: 60, durationMs: 600_000 },
    { frame: 1, patch: 0, note: 60, durationMs: 30 },
  ] });
  assert.deepEqual(plan.events.filter((event) => event.kind === "on").map((event) => [event.ms, event.lane, event.holdMs]), [[41, 1, 600_000], [41, 2, 30]]);
  assert.equal(plan.endMs, 600_041);
  assert.deepEqual(planJamArchivalSeek(plan, 100).sounding, [0]);
  assert.equal(plan.events.at(-1)?.ms, 600_166);
});

test("duplicate held attacks cannot move the first voice's key-up", () => {
  const input = take();
  input.events.splice(1, 0, { ms: 1, lane: 0, n: 60, on: 1, w: "sine", p: 0, q: 1 });
  input.events[2].q = 2;
  const plan = planJamArchivalPlayback(input);
  assert.equal(plan.events.length, 2);
  assert.equal(plan.events[0].holdMs, 600_000);
});

test("unmatched held notes release at end of take rather than an invented fixed hold", () => {
  const input = take();
  input.events[1] = { ms: 45_000, lane: 0, d: 1, n: 9, q: 1 };
  const plan = planJamArchivalPlayback(input);
  assert.equal(plan.events[0].offMs, 45_000);
  assert.equal(archivalReleaseTailMs(plan), 3_000);
});

test("archival input is copied/frozen once and accessor content is refused without executing it", () => {
  const input = take();
  const animation = { fps: 24, frameCount: 1, sfx: [{ frame: 0, patch: 0, note: 64, durationMs: 70_000 }] };
  const plan = planJamArchivalPlayback(input, animation);
  input.events[0].n = 80; input.patches[0].e.r = 2; animation.sfx[0].durationMs = 1;
  assert.equal(plan.events[0].note, 60);
  assert.equal(plan.animation?.sfx[0].durationMs, 70_000);
  assert.equal(Object.isFrozen(plan.take.patches[0].e), true);
  let calls = 0;
  const hostile = take(); Object.defineProperty(hostile.events[0], "n", { enumerable: true, get: () => { calls++; return 60; } });
  assert.throws(() => planJamArchivalPlayback(hostile), /accessor/);
  assert.equal(calls, 0);
});

test("maximal SFX expansion is bounded and malformed timing never reaches scheduling", () => {
  const input = take(); input.events = [];
  const sfx = Array.from({ length: 4096 }, () => ({ frame: 0, patch: 0, note: 60, durationMs: 1 }));
  const plan = planJamArchivalPlayback(input, { fps: 1, frameCount: 1, sfx });
  assert.equal(plan.events.length, 4096 * 3);
  assert.ok(plan.events.length > TAKE_PLAYBACK_EVENTS_PER_TICK);
  assert.throws(() => planJamArchivalPlayback(input, { fps: 1, frameCount: 1, sfx: [...sfx, sfx[0]] }), /animation/);
  for (const durationMs of [0, -1, NaN, 600_001, 0.5]) {
    assert.throws(() => planJamArchivalPlayback(input, { fps: 1, frameCount: 1, sfx: [{ ...sfx[0], durationMs }] }), /SFX/);
  }
});

test("custom zero-release notes reserve the legacy fallback tail for both SFX cleanup and completion", () => {
  const input = take(); input.patches[0].e.r = 0;
  const plan = planJamArchivalPlayback(input, { fps: 24, frameCount: 1,
    sfx: [{ frame: 0, patch: 0, note: 60, durationMs: 1 }] });
  assert.equal(archivalReleaseTailMs(plan), 120);
  assert.equal(plan.events.find((event) => event.kind === "cleanup")?.ms, 126);
});

test("snapshot text budget charges property names before downstream key sorting or joining", () => {
  const hostile = { ...take(), ["x".repeat(524_289)]: 0 };
  assert.throws(() => planJamArchivalPlayback(hostile), /archival input text exceeds its bound/);
  const repeated = take();
  repeated.events = Array.from({ length: 20_000 }, (_, q) => ({ ms: 0, lane: 0, n: 60, on: 1, w: "sine", q, ["x".repeat(32)]: 0 }));
  assert.throws(() => planJamArchivalPlayback(repeated), /archival input text exceeds its bound/);
});
