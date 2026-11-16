import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { VoiceSessionLifecycle, VOICE_SESSION_LIFECYCLE_BYTES, type VoiceLifecycleObservation, type VoiceSessionLifecycleActions } from "./voice-session-lifecycle.ts";
import type { VoiceIpcClock } from "./voice-crypto.ts";

const flush = async () => { for (let n = 0; n < 12; n++) await Promise.resolve(); };
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(yes => { resolve = yes; });
  return { promise, resolve };
}
function fixture() {
  let now = 0, serial = 0;
  const timers = new Map<number, { work: () => void; at: number }>();
  const clock: VoiceIpcClock = {
    now: () => now,
    schedule(work, delay) { const token = ++serial; timers.set(token, { work, at: now + delay }); return token; },
    cancel(token) { timers.delete(token as number); },
  };
  return { clock, timers, get now() { return now; },
    async advance(ms: number) {
      now += ms;
      const ready = [...timers].filter(([, timer]) => timer.at <= now);
      for (const [token, timer] of ready) { timers.delete(token); timer.work(); }
      await flush();
    },
  };
}
const waiting = (): VoiceLifecycleObservation => Object.freeze({ generation: {}, plan: null, stage: "waiting" });

test("lifecycle task, callbacks, timer and observer fit the separately prepaid metadata inventory", () => {
  // Declared application graph, not engine heap/RSS. Native/SDK/codec and compacted-peer
  // objects have their own fees; this graph retains only original opaque references.
  const record = 256, actions = 384, observer = 128, loopAndStep = 256, cadence = 192;
  const cell = 64, timerIdentity = 64, timerClosure = 96, snapshots = 192;
  const completion = 192, sessionLinks = 128, reserve = 96;
  assert.equal(record + actions + observer + loopAndStep + cadence + cell + timerIdentity +
    timerClosure + snapshots + completion + sessionLinks + reserve, VOICE_SESSION_LIFECYCLE_BYTES);
  const source = readFileSync(new URL("./voice-session-lifecycle.ts", import.meta.url), "utf8");
  const privateSlots = source.match(/^\s+(?:readonly )?#[a-zA-Z]\w*(?::| =)/gm)?.length ?? 0;
  assert.ok((privateSlots + 1) * 8 + 64 <= record, "review record growth against the unchanged fee");
});

test("lifecycle retries actual observed stages, never treats a void attempt as completion", async () => {
  const f = fixture(), generation = {}, seen: string[] = [];
  let stage: VoiceLifecycleObservation["stage"] = "handoff";
  const driver = new VoiceSessionLifecycle({ live: () => true,
    observe: () => Object.freeze({ generation, plan: null, stage }),
    async step(observation) { seen.push(observation.stage); },
  }, f.clock);
  driver.start(); driver.start(); await flush();
  assert.deepEqual(seen, ["handoff"]);
  await f.advance(249); assert.equal(seen.length, 1);
  await f.advance(1); assert.deepEqual(seen, ["handoff", "handoff"]);
  stage = "pc"; await f.advance(250);
  assert.deepEqual(seen, ["handoff", "handoff", "pc"]);
  await driver.handle.close(); assert.equal(f.timers.size, 0);
});

test("coalesced close joins an actually held leaf rather than reporting cancellation as return", async () => {
  const f = fixture(), held = deferred(); let calls = 0;
  const driver = new VoiceSessionLifecycle({ live: () => true,
    observe: () => ({ generation: {}, plan: null, stage: "rtc" }),
    async step() { calls++; await held.promise; },
  }, f.clock);
  driver.start(); await flush();
  const first = driver.handle.close(), second = driver.handle.close();
  assert.equal(first, second);
  let returned = false; void first.then(() => { returned = true; });
  await flush(); assert.equal(returned, false); assert.equal(calls, 1);
  held.resolve(); await first; assert.equal(f.timers.size, 0);
});

test("retirement in final observation property getters submits no leaf", async () => {
  const f = fixture(); let calls = 0;
  let driver!: VoiceSessionLifecycle;
  const observation = { generation: {}, plan: null,
    get stage() { void driver.handle.close(); return "handoff" as const; },
  };
  driver = new VoiceSessionLifecycle({ live: () => true, observe: () => observation,
    async step() { calls++; },
  }, f.clock);
  driver.start(); await flush(); await driver.handle.close();
  assert.equal(calls, 0); assert.equal(f.timers.size, 0);
});

test("retirement in owner callback cannot restore a positive scheduling result", async () => {
  const f = fixture(); let calls = 0, driver!: VoiceSessionLifecycle;
  driver = new VoiceSessionLifecycle({ live() { void driver.handle.close(); return true; },
    observe: waiting, async step() { calls++; },
  }, f.clock);
  driver.start(); await flush(); await driver.handle.close();
  assert.equal(calls, 0); assert.equal(f.timers.size, 0);
});

test("End in final Clock callback never installs another registration", async () => {
  const f = fixture(); let driver!: VoiceSessionLifecycle, schedules = 0;
  const clock: VoiceIpcClock = { ...f.clock,
    now() { void driver.handle.close(); return 0; },
    schedule(work, delay) { schedules++; return f.clock.schedule(work, delay); },
  };
  driver = new VoiceSessionLifecycle({ live: () => true, observe: waiting, async step() {} }, clock);
  driver.start(); await flush(); await driver.handle.close(); assert.equal(schedules, 0);
});

test("registered-then-thrown timer stays joined until its SAME actual callback", async () => {
  const f = fixture(); let callback!: () => void;
  const clock: VoiceIpcClock = { ...f.clock, schedule(work) { callback = work; throw new Error("lost registration reply"); } };
  const driver = new VoiceSessionLifecycle({ live: () => true, observe: waiting, async step() {} }, clock);
  driver.start(); await flush();
  const closing = driver.handle.close(); let returned = false;
  void closing.then(() => { returned = true; }); await flush(); assert.equal(returned, false);
  callback(); await closing; assert.equal(returned, true);
});

test("early callback cannot shorten cadence or clear a newer unknown registration", async () => {
  const f = fixture(); let schedules = 0, old!: () => void, current!: () => void, calls = 0;
  const clock: VoiceIpcClock = { ...f.clock, schedule(work, delay) {
    schedules++;
    if (schedules === 1) { old = work; return f.clock.schedule(work, delay); }
    current = work; throw new Error("second registration uncertain");
  } };
  const driver = new VoiceSessionLifecycle({ live: () => true,
    observe: () => ({ generation: {}, plan: null, stage: "pc" }), async step() { calls++; },
  }, clock);
  driver.start(); await flush(); old(); await flush(); assert.equal(calls, 1);
  const closing = driver.handle.close(); let returned = false;
  void closing.then(() => { returned = true; }); old(); await flush(); assert.equal(returned, false);
  current(); await closing; assert.equal(returned, true); assert.equal(calls, 1);
});

test("inline scheduling stops without recursive polling and cancellation returns before completion", async () => {
  const f = fixture(); let schedules = 0, cancelled = 0;
  const clock: VoiceIpcClock = { ...f.clock,
    schedule(work) { schedules++; work(); return 9; }, cancel() { cancelled++; },
  };
  const driver = new VoiceSessionLifecycle({ live: () => true, observe: waiting, async step() {} }, clock);
  driver.start(); await flush(); await driver.handle.close();
  assert.equal(schedules, 1); assert.equal(cancelled, 1);
});

test("unstarted close is irreversible and invokes no owner, clock or leaf", async () => {
  const actions: VoiceSessionLifecycleActions = { live() { assert.fail("owner"); }, observe() { assert.fail("observe"); }, async step() { assert.fail("step"); } };
  const clock: VoiceIpcClock = { now() { assert.fail("clock"); }, schedule() { assert.fail("schedule"); }, cancel() { assert.fail("cancel"); } };
  const driver = new VoiceSessionLifecycle(actions, clock);
  await driver.handle.close(); driver.start(); await flush(); await driver.handle.close();
});

test("a finite clock whose cadence rounds to now cannot busy-poll", async () => {
  for (const now of [2 ** 65, Number.MAX_VALUE]) {
    let liveCalls = 0, observations = 0;
    const clock: VoiceIpcClock = { now: () => now,
      schedule() { assert.fail("unrepresentable cadence"); }, cancel() {},
    };
    const driver = new VoiceSessionLifecycle({ live: () => ++liveCalls < 20,
      observe() { observations++; return waiting(); }, async step() { assert.fail("waiting"); },
    }, clock);
    driver.start(); await flush(); await driver.handle.close();
    assert.equal(observations, 1, "invalid local time must stop, not turn into a zero-delay loop");
  }
});

test("ordinary fractional clocks keep their original cadence without rounding up or early polling", async () => {
  const f = fixture(); await f.advance(0.125);
  let observations = 0;
  const driver = new VoiceSessionLifecycle({ live: () => true,
    observe() { observations++; return waiting(); }, async step() { assert.fail("waiting"); },
  }, f.clock);
  driver.start(); await flush(); assert.equal(observations, 1);
  await f.advance(249.875); assert.equal(observations, 1);
  await f.advance(0.125); assert.equal(observations, 2);
  await driver.handle.close(); assert.equal(f.timers.size, 0);
});

test("NaN or clock rollback in an entered timer stops without another observed attempt", async () => {
  for (const invalid of [Number.NaN, 9]) {
    const f = fixture(); await f.advance(10);
    let bad = false, observations = 0;
    const clock: VoiceIpcClock = { ...f.clock, now: () => bad ? invalid : f.now };
    const driver = new VoiceSessionLifecycle({ live: () => true,
      observe() { observations++; return waiting(); }, async step() { assert.fail("waiting"); },
    }, clock);
    driver.start(); await flush(); bad = true;
    await f.advance(250); await driver.handle.close();
    assert.equal(observations, 1); assert.equal(f.timers.size, 0);
  }
});

test("failed cancellation retains the same known handle and close until actual retry returns", async () => {
  const f = fixture(); let refuse = true;
  const clock: VoiceIpcClock = { ...f.clock,
    cancel(token) { if (refuse) throw new Error("cancel uncertain"); f.clock.cancel(token); },
  };
  const driver = new VoiceSessionLifecycle({ live: () => true, observe: waiting, async step() {} }, clock);
  driver.start(); await flush(); const first = driver.handle.close();
  let returned = false; void first.then(() => { returned = true; });
  await flush(); assert.equal(returned, false); assert.equal(f.timers.size, 1);
  refuse = false; assert.equal(driver.handle.close(), first); await first;
  assert.equal(returned, true); assert.equal(f.timers.size, 0);
});

test("actual callback can join a known timer after cancellation failed", async () => {
  const f = fixture();
  const clock: VoiceIpcClock = { ...f.clock, cancel() { throw new Error("cancel refused"); } };
  const driver = new VoiceSessionLifecycle({ live: () => true, observe: waiting, async step() {} }, clock);
  driver.start(); await flush(); const closed = driver.handle.close();
  let returned = false; void closed.then(() => { returned = true; }); await flush(); assert.equal(returned, false);
  await f.advance(250); await closed; assert.equal(returned, true); assert.equal(f.timers.size, 0);
});
