import assert from "node:assert/strict";
import test from "node:test";
import { TAKE_PLAYBACK_EVENTS_PER_TICK, type JamTake } from "./jam-contract.ts";
import { planJamArchivalPlayback, type JamArchivalAnimation, type JamArchivalPlan } from "./jam-archival-plan.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import type { JamArchivalPlayback, JamPlayResult } from "./jam-engine.ts";
import { StudioPlaybackController, type StudioPlaybackEngine, type StudioPlaybackScope } from "./studio-playback.ts";

const scope: StudioPlaybackScope = { generation: 1, server: 0, channel: "channel", membership: 1 };
function take(): JamTake {
  return { v: 1, group: "group", call: "call", met: { bpm: 120, bpb: 4 }, parts: ["author"],
    lanes: [{ src: 0, sn: "a".repeat(16) }], patches: [], events: [
      { ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 0 },
      { ms: 600_000, lane: 0, n: 60, on: 0, q: 1 },
    ] };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}
class FakeEngine implements StudioPlaybackEngine {
  resumes = 0;
  context: Pick<AudioContext, "state" | "resume"> = { state: "running", resume: async () => { this.resumes++; } };
  plan: JamArchivalPlan | null = null;
  capability: JamArchivalPlayback | null = null;
  prepared = 0;
  revoked: JamArchivalPlayback[] = [];
  calls: Array<{ index: number; age: number }> = [];
  preparing: Promise<unknown> | null = null;
  rendering: Promise<unknown> | null = null;
  /** What dispatch resolves to; `sync` returns it directly, as the engine does for note events. */
  outcome: JamPlayResult = { ok: true };
  sync = false;
  documents: boolean[] = [];
  async prepareArchivalPlayback(input: unknown, animation: JamArchivalAnimation | undefined, _current?: () => boolean, opts?: { document?: boolean }): Promise<JamArchivalPlayback> {
    this.prepared++; this.documents.push(opts?.document === true);
    const plan = planJamArchivalPlayback(input, animation);
    if (this.preparing) await this.preparing;
    this.plan = plan;
    return this.capability = {} as JamArchivalPlayback;
  }
  archivalPlaybackPlan(capability: JamArchivalPlayback): JamArchivalPlan | null { return capability === this.capability ? this.plan : null; }
  playArchivalEvent(_capability: JamArchivalPlayback, index: number, age = 0): JamPlayResult | Promise<JamPlayResult> {
    this.calls.push({ index, age });
    if (this.sync) return this.outcome;
    const rendering = this.rendering;
    return (async () => { if (rendering) await rendering; return this.outcome; })();
  }
  revokeArchivalPlayback(capability: JamArchivalPlayback): boolean {
    this.revoked.push(capability);
    if (this.capability !== capability) return false;
    this.capability = null; this.plan = null; return true;
  }
}
function setup() {
  const engine = new FakeEngine(), now = { value: 0 };
  const controller = new StudioPlaybackController(engine, () => now.value);
  controller.setScope(scope, false);
  return { engine, controller, now };
}

test("explicit local Play resumes immediately and late/seek attacks use remaining authored age", async () => {
  const { engine, controller, now } = setup();
  engine.context.state = "suspended";
  const start = controller.startTake(take(), 500_000);
  assert.equal(engine.resumes, 1, "resume runs in the original Play gesture turn");
  assert.equal(controller.state, "preparing");
  assert.equal(await start, true);
  now.value = 20_000;
  assert.equal(await controller.tick(), 1);
  assert.deepEqual(engine.calls, [{ index: 0, age: 520_000 }]);
  now.value = 100_000;
  assert.equal(await controller.tick(), 1);
  assert.deepEqual(engine.calls[1], { index: 1, age: 0 });
  assert.equal(controller.state, "playing", "release horizon is not a hard cut at the last event");
  now.value = 100_125;
  assert.equal(await controller.tick(), 0);
  assert.equal(controller.state, "stopped");
  assert.equal(controller.completed, true);
});

test("only a start the caller marks as the Studio's own document asks the engine for the document voice cap", async () => {
  const { engine, controller } = setup();
  assert.equal(await controller.startTake(take()), true);
  controller.document = true;
  assert.equal(await controller.startTake(take()), true);
  controller.document = false;
  assert.equal(await controller.startTake(take()), true);
  assert.deepEqual(engine.documents, [false, true, false]);
});

test("only natural completion marks EOF; manual, revoked and render-failed ownership never does", async () => {
  const h = setup(); await h.controller.startTake(take()); h.controller.stop(); assert.equal(h.controller.completed, false);
  await h.controller.startTake(take()); h.engine.capability = null; await h.controller.tick(); assert.equal(h.controller.completed, false);
  await h.controller.startTake(take()); h.engine.rendering = Promise.reject(new Error("renderer failed"));
  await h.controller.tick(); assert.equal(h.controller.state, "stopped"); assert.equal(h.controller.completed, false);
});

test("a resolved audio refusal, sync or async, stops without EOF and is not resumed by later ticks", async () => {
  for (const sync of [true, false]) {
    const h = setup(); h.engine.sync = sync;
    assert.equal(await h.controller.startTake(take()), true);
    h.engine.outcome = { ok: false, reason: "audio" };
    assert.equal(await h.controller.tick(), 1, `sync=${sync}`);
    assert.equal(h.controller.state, "stopped", `sync=${sync}`);
    assert.equal(h.controller.completed, false, `sync=${sync}: an unrendered take is not natural EOF`);
    assert.equal(h.engine.revoked.length, 1, `sync=${sync}: the failed capability is released`);
    h.engine.outcome = { ok: true }; h.now.value = 600_125;
    assert.equal(await h.controller.tick(), 0, `sync=${sync}`);
    assert.equal(h.engine.calls.length, 1, `sync=${sync}: the later release is never dispatched`);
    assert.equal(h.controller.completed, false, `sync=${sync}`);
  }
});

test("voice-budget drops, sync or async, keep the take playing through natural EOF", async () => {
  for (const sync of [true, false]) for (const reason of ["room-held", "source-held"] as const) {
    const h = setup(); h.engine.sync = sync;
    assert.equal(await h.controller.startTake(take()), true);
    h.engine.outcome = { ok: false, reason };
    assert.equal(await h.controller.tick(), 1);
    assert.equal(h.controller.state, "playing", `${reason} sync=${sync}: one dropped voice is not a failed take`);
    h.engine.outcome = { ok: true };
    h.now.value = 600_000; assert.equal(await h.controller.tick(), 1);
    h.now.value = 600_125; await h.controller.tick();
    assert.equal(h.controller.state, "stopped");
    assert.equal(h.controller.completed, true, `${reason} sync=${sync}`);
  }
});

test("an authored horizon past the last release keeps playing through the silence; invalid horizons refuse", async () => {
  const h = setup(), input = take(); input.events = [{ ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 0 }, { ms: 124, lane: 0, n: 60, on: 0, q: 1 }];
  assert.equal(await h.controller.startTake(input, 0, 2_000), true);
  await h.controller.tick(); h.now.value = 1_999; await h.controller.tick();
  assert.equal(h.controller.state, "playing", "trailing rests are part of the authored length");
  assert.equal(h.controller.elapsedMs, 1_999);
  h.now.value = 2_000; await h.controller.tick();
  assert.equal(h.controller.state, "stopped"); assert.equal(h.controller.completed, true);
  assert.equal(await h.controller.startTake(input, 1_000, 2_000), true, "a seek inside the trailing rests still plays to the grid end");
  assert.equal(h.controller.state, "playing");
  assert.equal(await h.controller.startTake(input, 2_000, 2_000), true);
  assert.equal(h.controller.state, "stopped"); assert.equal(h.controller.completed, true, "a seek at the grid end is complete");
  for (const bad of [-1, 1.5, NaN, Infinity, 600_001]) assert.equal(await h.controller.startTake(input, 0, bad), false, String(bad));
});

test("late seek beyond the entire authored horizon is silent natural completion, not final-event replay", async () => {
  const h = setup(); assert.equal(await h.controller.startTake(take(), 600_125), true);
  assert.equal(h.controller.state, "stopped"); assert.equal(h.controller.completed, true); assert.equal(h.engine.prepared, 0); assert.equal(h.engine.calls.length, 0);
});

test("Deafen, leave, scope and membership changes synchronously revoke playback; reopening never auto-plays", async () => {
  for (const update of [
    { scope: null, deafened: false },
    { scope, deafened: true },
    { scope: { ...scope, membership: 2 }, deafened: false },
    { scope: { ...scope, generation: 2 }, deafened: false },
    { scope: { ...scope, channel: "other" }, deafened: false },
    { scope: { ...scope, server: 1 }, deafened: false },
  ]) {
    const { engine, controller } = setup();
    assert.equal(await controller.startTake(take()), true);
    await controller.tick();
    controller.setScope(update.scope, update.deafened);
    assert.equal(controller.state, "stopped");
    assert.equal(engine.revoked.length, 1);
    controller.setScope(scope, false);
    assert.equal(await controller.tick(), 0);
    assert.equal(engine.calls.length, 1);
  }
});

test("pre-Deafen async preparation cannot return after Deafen ABA, even with a permissive fake engine", async () => {
  const { engine, controller } = setup();
  const gate = deferred<void>(); engine.preparing = gate.promise;
  const start = controller.startTake(take());
  await Promise.resolve(); await Promise.resolve();
  assert.equal(engine.prepared, 1);
  controller.setScope(scope, true); controller.setScope(scope, false);
  gate.resolve();
  assert.equal(await start, false);
  assert.equal(controller.state, "stopped");
  assert.equal(engine.revoked.length, 1);
  assert.equal(await controller.tick(), 0);
});

test("preparation keeps only one running and one latest request", async () => {
  const { engine, controller } = setup();
  const gate = deferred<void>(); engine.preparing = gate.promise;
  const first = controller.startTake(take());
  await Promise.resolve(); await Promise.resolve();
  const second = controller.startTake(take());
  const third = controller.startTake(take());
  assert.equal(await second, false);
  assert.equal(engine.prepared, 1);
  engine.preparing = null; gate.resolve();
  assert.equal(await first, false);
  assert.equal(await third, true);
  assert.equal(engine.prepared, 2);
  assert.equal(controller.state, "playing");
});

test("each tick and a concurrent tick use one shared 128-event work budget", async () => {
  const { engine, controller } = setup();
  const input = take(); input.events = Array.from({ length: 200 }, (_, q) => ({ ms: 0, lane: 0, n: q % 10, d: 1, q }));
  assert.equal(await controller.startTake(input), true);
  const gate = deferred<void>(); engine.rendering = gate.promise;
  const tick = controller.tick();
  assert.equal(await controller.tick(), 0);
  gate.resolve();
  assert.equal(await tick, TAKE_PLAYBACK_EVENTS_PER_TICK);
  assert.equal(engine.calls.length, TAKE_PLAYBACK_EVENTS_PER_TICK);
  assert.equal(await controller.tick(), 200 - TAKE_PLAYBACK_EVENTS_PER_TICK);
});

test("scope revocation during drum hashing cannot dispatch the next scheduled event", async () => {
  const { engine, controller } = setup();
  const input = take(); input.events.unshift({ ms: 0, lane: 0, n: 9, d: 1, q: 0 });
  input.events[1].q = 1; input.events[2].q = 2;
  assert.equal(await controller.startTake(input), true);
  const gate = deferred<void>(); engine.rendering = gate.promise;
  const tick = controller.tick();
  controller.setScope(null, false); gate.resolve();
  assert.equal(await tick, 1);
  assert.equal(engine.calls.length, 1);
  assert.equal(controller.state, "stopped");
});

test("timer/hash delay skips already-ended attacks and recalculates age after each await", async () => {
  const { engine, controller, now } = setup();
  const input = take(); input.events.unshift({ ms: 0, lane: 0, n: 9, d: 1, q: 0 });
  input.events[1].q = 1; input.events[2].q = 2;
  assert.equal(await controller.startTake(input), true);
  const gate = deferred<void>(); engine.rendering = gate.promise;
  const tick = controller.tick();
  now.value = 600_001; gate.resolve();
  assert.equal(await tick, 3);
  assert.deepEqual(engine.calls.map((call) => call.index), [0, 2], "an ended attack is not a catch-up burst");
});

test("backgrounded ticks consume expired drum work without replaying an old-hit burst", async () => {
  const { engine, controller, now } = setup();
  const input = take();
  input.events = Array.from({ length: 200 }, (_, q) => ({ ms: 0, lane: 0, n: 4, d: 1, q }));
  assert.equal(await controller.startTake(input), true);
  now.value = 300; // jam-kit:v1 hat's exact closing boundary.
  assert.equal(await controller.tick(), TAKE_PLAYBACK_EVENTS_PER_TICK);
  assert.equal(await controller.tick(), 200 - TAKE_PLAYBACK_EVENTS_PER_TICK);
  assert.deepEqual(engine.calls, [], "expired one-shots never become fresh attacks");
});

test("an eligible delayed drum carries its actual age into the engine deadline", async () => {
  const { engine, controller, now } = setup();
  const input = take(); input.events = [{ ms: 0, lane: 0, n: 4, d: 1, q: 0 }];
  assert.equal(await controller.startTake(input), true);
  now.value = 100;
  await controller.tick();
  assert.deepEqual(engine.calls, [{ index: 0, age: 100 }]);
});

test("invalid package/take/scope and denied gesture resume fail without synthesis", async () => {
  const { engine, controller } = setup();
  assert.equal(await controller.startPixa(new Uint8Array([1, 2, 3])), false);
  assert.equal(await controller.startTake({ ...take(), parts: [] }), false);
  assert.equal(await controller.startTake(take(), NaN), false);
  assert.throws(() => controller.setScope({ ...scope, membership: -1 }, false), /scope/);
  engine.context.state = "suspended";
  engine.context.resume = () => Promise.reject(new Error("gesture denied"));
  assert.equal(await controller.startTake(take()), false);
  assert.equal(controller.state, "stopped");
  assert.equal(engine.prepared, 0);
  assert.equal(engine.calls.length, 0);
});

test("caller mutation while preparation is queued cannot change admitted content", async () => {
  const { engine, controller } = setup();
  const gate = deferred<void>(); engine.preparing = gate.promise;
  const input = take();
  const start = controller.startTake(input);
  input.events[0].n = 84; input.events[1].ms = 1;
  gate.resolve();
  assert.equal(await start, true);
  assert.equal(engine.plan?.events[0].note, 60);
  assert.equal(engine.plan?.events[0].holdMs, 600_000);
});

test("completion reserves a zero-release custom note's possible legacy fallback tail", async () => {
  const { controller, now } = setup();
  const input = take();
  input.patches = [legacyJamPatch("sine")]; input.patches[0].e.r = 0;
  input.events[0] = { ms: 0, lane: 0, n: 60, on: 1, w: "sine", p: 0, q: 0 };
  input.events[1].ms = 1;
  assert.equal(await controller.startTake(input), true);
  await controller.tick(); now.value = 13; await controller.tick();
  assert.equal(controller.state, "playing", "the fallback is still releasing at the old teardown time");
  now.value = 125; await controller.tick(); assert.equal(controller.state, "playing");
  now.value = 126; await controller.tick(); assert.equal(controller.state, "stopped");
});
