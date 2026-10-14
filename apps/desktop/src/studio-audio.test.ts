import assert from "node:assert/strict";
import test from "node:test";
import { StudioAudioSession } from "./studio-audio.ts";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";
import type { JamTake } from "./jam-contract.ts";
import type { JamArchivalPlayback } from "./jam-engine.ts";
import { deferred } from "./studio-testkit.ts";
const take: JamTake = { v: 1, group: "group", call: "call", met: { bpm: 120, bpb: 4 }, parts: ["author"], lanes: [{ src: 0, sn: "a".repeat(16) }], patches: [], events: [{ ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 0 }, { ms: 90_000, lane: 0, n: 60, on: 0, q: 1 }] };
function harness() {
  let now = 0, prepared = 0, resumes = 0, revoked = 0;
  const plans = new Map<JamArchivalPlayback, ReturnType<typeof planJamArchivalPlayback>>();
  const context = { state: "running" as AudioContextState, resume: async () => { resumes++; context.state = "running"; } };
  const audio = new StudioAudioSession({ context,
    prepareArchivalPlayback: async (value, animation) => { const cap = {} as JamArchivalPlayback; plans.set(cap, planJamArchivalPlayback(value, animation)); prepared++; return cap; },
    archivalPlaybackPlan: (cap) => plans.get(cap) ?? null, playArchivalEvent: () => ({ ok: true }),
    revokeArchivalPlayback: (cap) => { revoked++; return plans.delete(cap); },
  }, () => now);
  audio.setScope({ generation: 1, server: 1, channel: "channel", membership: 1 }, false);
  return { audio, context, now: (ms: number) => { now = ms; }, prepared: () => prepared, resumes: () => resumes, revoked: () => revoked };
}
test("Studio resumes in the gesture turn and stale preparation cannot return after Deafen ABA", async () => {
  const h = harness(), prep = deferred<JamTake>(); h.context.state = "suspended";
  const start = h.audio.start("score", () => prep.promise, false); assert.equal(h.resumes(), 1); assert.equal(h.audio.state.state, "preparing");
  h.audio.setScope({ generation: 1, server: 1, channel: "channel", membership: 1 }, true);
  h.audio.setScope({ generation: 1, server: 1, channel: "channel", membership: 1 }, false);
  prep.resolve(take); assert.equal(await start, false); assert.equal(h.prepared(), 0); assert.equal(h.audio.state.state, "stopped");
});
test("explicit loop waits for authored hold and release, while suspended audio never auto-resumes", async () => {
  const h = harness(); assert.equal(await h.audio.start("score", async () => take, true), true);
  await h.audio.tick(); h.now(90_000); await h.audio.tick(); assert.equal(h.prepared(), 1);
  h.now(90_125); await h.audio.tick(); assert.equal(h.prepared(), 2, "new loop starts only after fallback release horizon");
  h.context.state = "suspended"; await h.audio.tick(); assert.equal(h.audio.state.state, "stopped"); assert.equal(h.resumes(), 0);
});
test("navigation scope and Stop revoke owned sound and suppress loop restart", async () => {
  const h = harness(); await h.audio.start("score", async () => take, true);
  h.audio.setScope({ generation: 2, server: 1, channel: "channel", membership: 1 }, false);
  h.now(100_000); await h.audio.tick(); assert.equal(h.prepared(), 1); assert.equal(h.revoked(), 1); assert.equal(h.audio.state.state, "stopped");
});
