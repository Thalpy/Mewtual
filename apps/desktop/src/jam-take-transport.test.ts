import assert from "node:assert/strict";
import test from "node:test";
import { archivalDjPosition, archivalPlaybackNeedsStart, JamTakeTransport, type JamTakeTransportEngine } from "./jam-take-transport.ts";
import { pixaFrameAt } from "./pixa-media.ts";
import { JamLatestTaskQueue } from "./jam-publication.ts";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";
import type { JamArchivalPlayback, JamPlayResult } from "./jam-engine.ts";
import type { JamTake } from "./jam-contract.ts";
import { deferred } from "./studio-testkit.ts";
import { readFileSync } from "node:fs";
import { decodePixa } from "./pixa.ts";
const take: JamTake = { v: 1, group: "group", call: "call", met: { bpm: 120, bpb: 4 }, parts: ["author"], lanes: [{ src: 0, sn: "a".repeat(16) }], patches: [], events: [{ ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 0 }, { ms: 95_000, lane: 0, n: 60, on: 0, q: 1 }] };
const scope = { generation: 1, server: 1, channel: "channel", membership: 1 };

test("PIXA DJ pictures and announcements use the actual audio clock after delayed fetch and preparation", async () => {
  const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
  const p = decodePixa(Buffer.from(vector.pixaHex, "hex"));
  const animation = { ...p, fps: 2, frames: Array(6).fill(p.frames[0]), take: { ...p.take, patches: [vector.patch] }, sfx: [{ frame: 0, patch: 0, note: 72, durationMs: 5000 }] };
  const h = harness(), download = deferred<void>(), preparation = deferred<void>();
  h.digest(preparation.promise);
  assert.equal(archivalDjPosition(0, null), 0);
  const loading = (async () => { await download.promise; return h.transport.startPixa(animation, 0, scope, false, h.current); })();
  h.now(2000); download.resolve(); await Promise.resolve();
  assert.equal(archivalDjPosition(0, h.transport), 0, "the DJ holds its pressed frame during preparation");
  h.now(3000); preparation.resolve(); assert.equal(await loading, true); await h.transport.tick();
  assert.equal(h.transport.elapsedMs, 0); assert.equal(h.events[0].age, 0);
  assert.equal(pixaFrameAt(animation, archivalDjPosition(0, h.transport) * 1000), 0);
  h.now(3500); await h.transport.tick();
  assert.equal(archivalDjPosition(0, h.transport), .5);
  assert.equal(pixaFrameAt(animation, h.transport.elapsedMs), 1);
});

test("a listener samples its seek after preparation and reconstructs the actual held sound age", async () => {
  const h = harness(), preparation = deferred<void>(); h.digest(preparation.promise);
  let projectedMs = 2000;
  const pending = h.transport.start(take, projectedMs, scope, true, h.current, () => projectedMs);
  projectedMs = 3500; h.now(1500); preparation.resolve(); assert.equal(await pending, true);
  await h.transport.tick();
  assert.equal(h.transport.elapsedMs, 3500); assert.equal(h.events[0].age, 3500);
  assert.equal(h.resumes(), 0);
});

test("heartbeats preserve slow preparation and a fresh press can replace it", async () => {
  const h = harness(), gate = deferred<void>(); h.digest(gate.promise);
  let roomMs = 0;
  const pending = h.transport.start(take, 0, scope, true, h.current, () => roomMs);
  for (const at of [5000, 10000, 15000]) {
    roomMs = at; h.now(at);
    assert.equal(archivalPlaybackNeedsStart(true, null, roomMs), false);
    assert.equal(h.transport.state, "preparing"); assert.equal(h.revoked(), 0);
  }
  gate.resolve(); assert.equal(await pending, true); await h.transport.tick();
  assert.equal(h.events[0].age, 15000);
  assert.equal(archivalPlaybackNeedsStart(false, { elapsedMs: h.transport.elapsedMs, done: false }, roomMs), false);
  h.transport.stop(); // App retires ownership on a fresh sequence, including a same-CID seek.
  assert.equal(archivalPlaybackNeedsStart(false, null, 500), true);
  assert.equal(await h.transport.start(take, 500, scope, true, h.current), true);
  assert.equal(h.transport.elapsedMs, 500);
});
function harness() {
  let now = 0, resumes = 0, live = true, revoked = 0;
  const plans = new Map<JamArchivalPlayback, ReturnType<typeof planJamArchivalPlayback>>(), events: { index: number; age: number }[] = [], levels: number[] = [];
  const context = { state: "running" as AudioContextState, resume: async () => { resumes++; context.state = "running"; } };
  let digest: Promise<unknown> | null = null;
  let outcome: JamPlayResult = { ok: true };
  const engine: JamTakeTransportEngine = { context,
    prepareArchivalPlayback: async (take, animation, current) => { const plan = planJamArchivalPlayback(take, animation); if (digest) await digest; if (!current()) return null; const cap = {} as JamArchivalPlayback; plans.set(cap, plan); return cap; },
    archivalPlaybackPlan: cap => plans.get(cap) ?? null,
    playArchivalEvent: (_cap, index, age = 0) => { events.push({ index, age }); return outcome; },
    revokeArchivalPlayback: cap => { revoked++; return plans.delete(cap); }, setArchivalPlaybackLevel: (_cap, level) => { levels.push(level); return true; },
  };
  const queue = new JamLatestTaskQueue<JamArchivalPlayback | null>(), transport = new JamTakeTransport(engine, queue, () => now);
  return { transport, engine, queue, context, events, levels, now: (ms: number) => { now = ms; }, resumes: () => resumes, current: () => live,
    retire: () => { live = false; }, revoked: () => revoked, digest: (promise: Promise<unknown>) => { digest = promise; },
    outcome: (next: JamPlayResult) => { outcome = next; } };
}
test("shared take never resumes a suspended context; local gesture does so before digest", async () => {
  const h = harness(); h.context.state = "suspended";
  assert.equal(await h.transport.start(take, 0, scope, true, h.current), false); assert.equal(h.resumes(), 0);
  const gate = deferred<void>(); h.digest(gate.promise);
  const pending = h.transport.start(take, 0, scope, false, h.current); assert.equal(h.resumes(), 1);
  gate.resolve(); assert.equal(await pending, true);
});
test("shared/local take seek dispatches owned remaining-age event instead of raw 30s noteOn", async () => {
  const h = harness(); h.transport.setLevel(.4);
  assert.equal(await h.transport.start(take, 80_000, scope, true, h.current), true);
  await h.transport.tick(); assert.deepEqual(h.events, [{ index: 0, age: 80_000 }]); assert.deepEqual(h.levels, [.4]);
  h.now(15_000); await h.transport.tick(); assert.equal(h.transport.state, "playing", "authored release tail still owned");
  h.now(15_125); await h.transport.tick(); assert.equal(h.transport.state, "stopped"); assert.equal(h.revoked(), 1);
  assert.equal(h.transport.completed, true);
});
test("external call/deck lease retirement during digest or playing revokes stale ownership", async () => {
  const h = harness(), gate = deferred<void>(); h.digest(gate.promise);
  const pending = h.transport.start(take, 0, scope, true, h.current); h.retire(); gate.resolve(); assert.equal(await pending, false);
  const h2 = harness(); await h2.transport.start(take, 0, scope, true, h2.current); h2.retire(); await h2.transport.tick();
  assert.equal(h2.transport.state, "stopped"); assert.equal(h2.events.length, 0);
  assert.equal(h2.transport.completed, false);
});

test("a resolved render failure on a deck take stops without completion, so the DJ cannot advance", async () => {
  const h = harness(); assert.equal(await h.transport.start(take, 0, scope, true, h.current), true);
  h.outcome({ ok: false, reason: "audio" }); await h.transport.tick();
  assert.equal(h.transport.state, "stopped"); assert.equal(h.transport.completed, false); assert.equal(h.revoked(), 1);
  h.outcome({ ok: true }); h.now(95_125); await h.transport.tick();
  assert.equal(h.events.length, 1, "the failed take is not resumed"); assert.equal(h.transport.completed, false);
});

test("a suspended in-progress context cancels ownership without claiming completion or resuming", async () => {
  const h = harness(); await h.transport.start(take, 0, scope, true, h.current);
  h.context.state = "suspended"; await h.transport.tick(); assert.equal(h.transport.completed, false); assert.equal(h.revoked(), 1); assert.equal(h.resumes(), 0);
});
test("volume changes are capability-scoped and malformed values cannot reach the engine", async () => {
  const h = harness(); await h.transport.start(take, 0, scope, true, h.current);
  assert.equal(h.transport.setLevel(.2), true); for (const bad of [-1, 2, NaN, Infinity]) assert.equal(h.transport.setLevel(bad), false);
  assert.deepEqual(h.levels, [1, .2]);
  h.retire(); assert.equal(h.transport.setLevel(.8), false); assert.deepEqual(h.levels, [1, .2], "retired App lease cannot control old owned gains before the next tick");
});

test("shared PIXA preserves visual horizon/SFX seek age, remote resume gate and retirement", async () => {
  const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
  const p = decodePixa(Buffer.from(vector.pixaHex, "hex"));
  const animation = { ...p, fps: 2, frames: [p.frames[0], p.frames[0]], take: { ...p.take, patches: [vector.patch] }, sfx: [{ frame: 1, patch: 0, note: 72, durationMs: 1000 }] };
  const h = harness(); h.context.state = "suspended";
  assert.equal(await h.transport.startPixa(animation, 750, scope, true, h.current), false); assert.equal(h.resumes(), 0);
  h.context.state = "running";
  assert.equal(await h.transport.startPixa(animation, 750, scope, true, h.current), true);
  await h.transport.tick(); assert.equal(h.events[0].age, 250, "late join reconstructs authored held SFX, not a fresh attack");
  h.now(250); await h.transport.tick(); assert.equal(h.transport.state, "playing", "visual EOF cannot truncate longer sound");
  h.retire(); await h.transport.tick(); assert.equal(h.transport.state, "stopped"); assert.equal(h.transport.completed, false);
  const silent = harness(); await silent.transport.startPixa(p, 0, scope, true, silent.current);
  assert.equal(silent.transport.state, "playing", "empty music still owns the animation horizon");
  silent.now(100); await silent.transport.tick(); assert.equal(silent.transport.completed, true);
});
