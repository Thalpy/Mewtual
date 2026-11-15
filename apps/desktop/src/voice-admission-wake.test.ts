import assert from "node:assert/strict";
import test from "node:test";
import { VoiceAdmissionWake } from "./voice-admission-wake.ts";
import type { VoiceIpcClock } from "./voice-crypto.ts";

const flush = () => new Promise<void>(resolve => setImmediate(resolve));
function fixture() {
  let now = 0, serial = 0, peak = 0;
  const timers = new Map<number, () => void>();
  const clock: VoiceIpcClock = {
    now: () => now,
    schedule(work) { const id = ++serial; timers.set(id, work); peak = Math.max(peak, timers.size); return id; },
    cancel(id) { timers.delete(id as number); },
  };
  return { clock, timers, peak: () => peak,
    setNow(value: number) { now = value; },
    fire(id: number) { const work = timers.get(id)!; timers.delete(id); work(); },
  };
}
test("admission cadence is inert until publication and joins one exact timer", async () => {
  const f = fixture(), wait = new VoiceAdmissionWake(f.clock);
  assert.equal(f.timers.size, 0);
  wait.start(); wait.start(); assert.equal(f.timers.size, 1);
  let returned = false; void wait.join().then(() => { returned = true; });
  f.setNow(999); f.fire(1); await flush();
  assert.equal(returned, false); assert.equal(f.timers.size, 1);
  f.setNow(1_000); f.fire(2); assert.equal(returned, false, "callback frame has not returned");
  await wait.join(); assert.equal(returned, true); assert.equal(f.peak(), 1);
});
test("admission cadence schedule-then-throw is joined only by its exact callback", async () => {
  const f = fixture(); let callback!: () => void;
  const wait = new VoiceAdmissionWake({ ...f.clock, schedule(work) { callback = work; throw new Error("unknown registration"); } });
  wait.start(); wait.wake(); wait.start();
  let returned = false; void wait.join().then(() => { returned = true; }); await flush();
  assert.equal(returned, false);
  callback(); assert.equal(returned, false); await wait.join();
  assert.equal(returned, true); callback(); wait.wake();
});
test("admission cadence old callback cannot clear a later unknown registration", async () => {
  const f = fixture(); let old!: () => void, unknown!: () => void, calls = 0;
  const wait = new VoiceAdmissionWake({ ...f.clock, schedule(work, delay) {
    calls++; if (calls === 1) { old = work; return f.clock.schedule(work, delay); }
    unknown = work; throw new Error("second registration unknown");
  } });
  wait.start(); f.setNow(500); f.fire(1); wait.wake();
  let returned = false; void wait.join().then(() => { returned = true; });
  old(); await flush(); assert.equal(returned, false); assert.equal(calls, 2);
  unknown(); await wait.join(); assert.equal(returned, true);
});
for (const finish of ["cancel retry", "original callback"] as const)
test(`admission cadence cancellation failure retains original debt through ${finish}`, async () => {
  const f = fixture(); let refuse = true;
  const wait = new VoiceAdmissionWake({ ...f.clock, cancel(id) {
    if (refuse) throw new Error("cancel uncertain"); f.clock.cancel(id);
  } });
  wait.start(); wait.wake();
  let returned = false; void wait.join().then(() => { returned = true; }); await flush();
  assert.equal(returned, false); assert.equal(f.timers.size, 1);
  if (finish === "original callback") f.fire(1);
  else { refuse = false; wait.wake(); }
  await wait.join(); assert.equal(returned, true); assert.equal(f.timers.size, 0);
});
test("admission cadence synchronous callback and reentrant wake cannot reschedule", async () => {
  const f = fixture(); let wait!: VoiceAdmissionWake, calls = 0;
  wait = new VoiceAdmissionWake({ ...f.clock, schedule(work, delay) {
    calls++; work(); wait.wake(); return f.clock.schedule(work, delay);
  } });
  wait.start(); await wait.join(); assert.equal(calls, 1); assert.equal(f.timers.size, 0);
});
test("admission cadence wake during Clock.now fences registration", async () => {
  const f = fixture(); let wait!: VoiceAdmissionWake;
  wait = new VoiceAdmissionWake({ ...f.clock, now() { wait.wake(); return 0; } });
  wait.start(); await wait.join(); assert.equal(f.timers.size, 0);
});
test("admission cadence invalid or imprecise time waits for availability instead of spinning", async () => {
  for (const now of [Number.NaN, Infinity, Number.MAX_VALUE]) {
    const f = fixture(); f.setNow(now); const wait = new VoiceAdmissionWake(f.clock);
    wait.start(); let returned = false; void wait.join().then(() => { returned = true; });
    await flush(); assert.equal(returned, false); assert.equal(f.timers.size, 0);
    wait.wake(); await wait.join();
  }
});
test("admission deadline notifies a late fault separately from actual return", async () => {
  const f = fixture(); let faults = 0;
  const wait = new VoiceAdmissionWake(f.clock, 5_000, () => { faults++; });
  wait.start(); let returned = false; void wait.join().then(() => { returned = true; });
  f.setNow(Number.NaN); f.fire(1);
  assert.equal(faults, 1); assert.equal(wait.faulted, true); assert.equal(wait.elapsed, false);
  await flush(); assert.equal(returned, false);
  wait.wake(); await wait.join(); assert.equal(returned, true); assert.equal(faults, 1);
});
