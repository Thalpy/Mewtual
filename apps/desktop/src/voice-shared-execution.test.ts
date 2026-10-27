import assert from "node:assert/strict";
import test from "node:test";
import { VoiceSharedExecution, VOICE_SHARED_WAITERS } from "./voice-shared-execution.ts";
const gate = () => { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return {promise, release}; };
const flush = () => new Promise<void>(yes => setImmediate(yes));

test("shared execution is bounded FIFO with one actual physical owner", async () => {
  let active = 0, peak = 0, starts = 0, ends = 0;
  const lane = new VoiceSharedExecution({begin: () => {starts++; peak = Math.max(peak, ++active);}, end: () => {ends++; active--;}});
  const held = gate(), order: number[] = [];
  const first = lane.run(async () => {order.push(-1); await held.promise;});
  const pending = Array.from({length: VOICE_SHARED_WAITERS}, (_, n) => lane.run(async () => {order.push(n);}));
  await assert.rejects(lane.run(async () => {throw new Error("overflow work must never start");}), /execution capacity/);
  assert.deepEqual(order, [-1]); assert.equal(starts, 1); assert.equal(ends, 0); assert.equal(active, 1);
  held.release(); await first; await Promise.all(pending);
  assert.deepEqual(order, [-1, ...Array.from({length: VOICE_SHARED_WAITERS}, (_, n) => n)]);
  assert.equal(peak, 1); assert.equal(active, 0); assert.equal(starts, ends);
  assert.equal(await lane.run(async () => 7), 7);
});

test("losing a queued observer does not forget its actual execution owner", async () => {
  const lane = new VoiceSharedExecution({begin() {}, end() {}}, 2), held = gate(), lost = gate();
  let second = false, third = false;
  const first = lane.run(() => held.promise);
  void lane.run(async () => {second = true; await lost.promise;});
  const last = lane.run(async () => {third = true;});
  held.release(); await first; await flush();
  assert.equal(second, true); assert.equal(third, false);
  lost.release(); await last; assert.equal(third, true);
});

test("reentrant accounting cannot let a new owner overtake an admitted waiter", async () => {
  const held = gate(), order: string[] = []; let reenter = false, extra: Promise<void> | null = null;
  const lane = new VoiceSharedExecution({begin() {}, end() {
    if (reenter) {reenter = false; extra = lane.run(async () => {order.push("reentrant");});}
  }}, 3);
  const first = lane.run(() => held.promise), next = lane.run(async () => {order.push("waiting");});
  reenter = true; held.release(); await first; await next; await extra;
  assert.deepEqual(order, ["waiting", "reentrant"]);
});

test("actual work and accounting failures release the lane without starting refused work", async () => {
  let failBegin = true, failEnd = true, started = 0;
  const lane = new VoiceSharedExecution({begin() {if (failBegin) {failBegin = false; throw new Error("begin");}},
    end() {if (failEnd) {failEnd = false; throw new Error("end");}}});
  await assert.rejects(lane.run(async () => {started++;}), /begin/); assert.equal(started, 0);
  await assert.rejects(lane.run(async () => {started++;}), /end/);
  await assert.rejects(lane.run(async () => {throw new Error("work");}), /work/);
  assert.equal(await lane.run(async () => 9), 9);
});

test("execution bounds cannot enlarge the finite inventory", () => {
  for (const limit of [-1, 0.5, NaN, Infinity, VOICE_SHARED_WAITERS + 1])
    assert.throws(() => new VoiceSharedExecution({begin() {}, end() {}}, limit), /invalid voice execution bound/);
});


for (const failure of ["work", "end"]) test(`${failure} failure transfers ownership to already queued callers`, async () => {
  const held = gate(), order: string[] = []; let endFails = failure === "end";
  const lane = new VoiceSharedExecution({begin() {}, end() {if (endFails) {endFails = false; throw new Error("end");}}});
  const first = lane.run(async () => {await held.promise; if (failure === "work") throw new Error("work");});
  const rejection = assert.rejects(first, new RegExp(failure));
  const second = lane.run(async () => {order.push("second");});
  const third = lane.run(async () => {order.push("third");});
  held.release(); await rejection; await Promise.all([second, third]);
  assert.deepEqual(order, ["second", "third"]);
});

test("begin failure still hands an actual reentrantly queued owner its turn", async () => {
  let first = true, next: Promise<number> | null = null;
  const lane = new VoiceSharedExecution({begin() {
    if (first) {first = false; next = lane.run(async () => 5); throw new Error("begin");}
  }, end() {}});
  await assert.rejects(lane.run(async () => {throw new Error("must not run");}), /begin/);
  assert.equal(await next, 5);
});

test("zero waiter bound refuses only queued work and recovers after actual return", async () => {
  const held = gate(), lane = new VoiceSharedExecution({begin() {}, end() {}}, 0);
  const first = lane.run(() => held.promise);
  await assert.rejects(lane.run(async () => {throw new Error("must not run");}), /execution capacity/);
  held.release(); await first; assert.equal(await lane.run(async () => 11), 11);
});
