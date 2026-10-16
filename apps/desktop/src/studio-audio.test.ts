import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { StudioAudioSession } from "./studio-audio.ts";
import { archivalReleaseTailMs, planJamArchivalPlayback } from "./jam-archival-plan.ts";
import type { JamTake } from "./jam-contract.ts";
import type { JamArchivalPlayback } from "./jam-engine.ts";
import { exportScorePlayback, type ScoreExportInput, type ScorePlayback } from "./studio-export.ts";
import { deferred } from "./studio-testkit.ts";
const take: JamTake = { v: 1, group: "group", call: "call", met: { bpm: 120, bpb: 4 }, parts: ["author"], lanes: [{ src: 0, sn: "a".repeat(16) }], patches: [], events: [{ ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 0 }, { ms: 90_000, lane: 0, n: 60, on: 0, q: 1 }] };
/** A generic take has no Score grid; zero keeps the take-derived horizon. */
const playback = (value: JamTake): ScorePlayback => ({ take: value, gridMs: 0 });
const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
/** A real resolved Score at 120 BPM (125 ms per sixteenth) using the fixture's 120-ms-release recipe. */
function score(steps: number, cells: ScoreExportInput["cells"]): Promise<ScorePlayback> {
  return exportScorePlayback({ id: vector.object, bpm: 120, bpb: 4, steps, patches: { [vector.patchId]: structuredClone(vector.patch) }, cells },
    { group: vector.group, exporter: vector.exporter });
}
function releaseEnd(value: ScorePlayback): number {
  const plan = planJamArchivalPlayback(value.take);
  return plan.endMs + archivalReleaseTailMs(plan) + 5;
}
function harness() {
  let now = 0, prepared = 0, resumes = 0, revoked = 0, fail = false;
  const plans = new Map<JamArchivalPlayback, ReturnType<typeof planJamArchivalPlayback>>();
  const context = { state: "running" as AudioContextState, resume: async () => { resumes++; context.state = "running"; } };
  const audio = new StudioAudioSession({ context,
    prepareArchivalPlayback: async (value, animation) => { const cap = {} as JamArchivalPlayback; plans.set(cap, planJamArchivalPlayback(value, animation)); prepared++; return cap; },
    archivalPlaybackPlan: (cap) => plans.get(cap) ?? null, playArchivalEvent: () => fail ? Promise.reject(new Error("renderer failed")) : { ok: true },
    revokeArchivalPlayback: (cap) => { revoked++; return plans.delete(cap); },
  }, () => now);
  audio.setScope({ generation: 1, server: 1, channel: "channel", membership: 1 }, false);
  return { audio, context, now: (ms: number) => { now = ms; }, prepared: () => prepared, resumes: () => resumes, revoked: () => revoked, fail: () => { fail = true; } };
}
test("Studio resumes in the gesture turn and stale preparation cannot return after Deafen ABA", async () => {
  const h = harness(), prep = deferred<ScorePlayback>(); h.context.state = "suspended";
  const start = h.audio.start("score", () => prep.promise, false); assert.equal(h.resumes(), 1); assert.equal(h.audio.state.state, "preparing");
  h.audio.setScope({ generation: 1, server: 1, channel: "channel", membership: 1 }, true);
  h.audio.setScope({ generation: 1, server: 1, channel: "channel", membership: 1 }, false);
  prep.resolve(playback(take)); assert.equal(await start, false); assert.equal(h.prepared(), 0); assert.equal(h.audio.state.state, "stopped");
});
test("explicit loop waits for authored hold and release, while suspended audio never auto-resumes", async () => {
  const h = harness(); assert.equal(await h.audio.start("score", async () => playback(take), true), true);
  await h.audio.tick(); h.now(90_000); await h.audio.tick(); assert.equal(h.prepared(), 1);
  h.now(90_125); await h.audio.tick(); assert.equal(h.prepared(), 2, "new loop starts only after fallback release horizon");
  h.context.state = "suspended"; await h.audio.tick(); assert.equal(h.audio.state.state, "stopped"); assert.equal(h.resumes(), 0);
});
test("navigation scope and Stop revoke owned sound and suppress loop restart", async () => {
  const h = harness(); await h.audio.start("score", async () => playback(take), true);
  h.audio.setScope({ generation: 2, server: 1, channel: "channel", membership: 1 }, false);
  h.now(100_000); await h.audio.tick(); assert.equal(h.prepared(), 1); assert.equal(h.revoked(), 1); assert.equal(h.audio.state.state, "stopped");
});

test("renderer failure and empty scores never spin an automatic loop", async () => {
  const h = harness(); await h.audio.start("score", async () => playback(take), true); h.fail(); await h.audio.tick(); await h.audio.tick();
  assert.equal(h.audio.state.state, "stopped"); assert.equal(h.prepared(), 1, "failed rendering is not natural completion");
  const empty = harness(); await empty.audio.start("score", async () => playback({ ...take, events: [] }), true);
  empty.now(10); await empty.audio.tick(); empty.now(50); await empty.audio.tick(); assert.equal(empty.prepared(), 1, "an empty score does not hash/restart every timer turn");
});

test("a looped Score keeps its trailing rests and empty bars before restarting", async () => {
  const cases: Array<{ name: string; steps: number; cells: ScoreExportInput["cells"] }> = [
    { name: "first-step note then rests", steps: 16, cells: { "0:0": { k: "n", p: vector.patchId } } },
    { name: "three trailing empty bars", steps: 64, cells: { "0:2": { k: "d", pad: 3 }, "12:0": { k: "n", p: vector.patchId, len: 2 } } },
  ];
  for (const { name, steps, cells } of cases) {
    const value = await score(steps, cells), grid = steps * 125;
    assert.equal(value.gridMs, grid, name);
    assert.ok(releaseEnd(value) < grid, `${name}: the fixture's last release ends inside the grid`);
    const h = harness(); assert.equal(await h.audio.start("score", async () => value, true), true);
    for (const at of [0, releaseEnd(value), grid - 1]) {
      h.now(at); await h.audio.tick();
      assert.equal(h.audio.state.state, "playing", `${name} at ${at} ms: the rest of the grid is still the score`);
      assert.equal(h.prepared(), 1, `${name} at ${at} ms: no early restart`);
    }
    h.now(grid); await h.audio.tick();
    assert.equal(h.prepared(), 2, `${name}: the loop restarts at the authored grid end`);
  }
});

test("a Score whose final release outlasts its grid loops after that release, not at the grid end", async () => {
  const value = await score(16, { "15:0": { k: "n", p: vector.patchId } }), end = releaseEnd(value);
  assert.equal(value.gridMs, 2_000);
  assert.ok(end > 2_000, "the final note's release crosses the grid boundary");
  const h = harness(); assert.equal(await h.audio.start("score", async () => value, true), true);
  for (const at of [0, 1_875, 2_000, end - 1]) {
    h.now(at); await h.audio.tick();
    assert.equal(h.audio.state.state, "playing", `at ${at} ms`);
    assert.equal(h.prepared(), 1, `at ${at} ms: the release is not cut by a restart`);
  }
  h.now(end); await h.audio.tick();
  assert.equal(h.prepared(), 2);
});
