import assert from "node:assert/strict";
import test from "node:test";
import { VoiceByteLedger, reserveVoiceBytePayment } from "./voice-byte-ledger.ts";
import { VoiceAudioFunding, voiceAudioFundingInventory } from "./voice-audio-funding.ts";
import { VoiceAudioCapturePool, VOICE_AUDIO_CAPTURE_POOL_BYTES as FIXED, VOICE_AUDIO_CAPTURE_BYTES as CAPSULE } from "./voice-audio-capture.ts";
import { VoiceMediaStatus } from "./voice-media-status.ts";

const A = "11".repeat(32), B = "22".repeat(32), C = "33".repeat(32);
function gate<T>() { let release!: (value: T) => void; const promise = new Promise<T>(yes => { release = yes; }); return { promise, release }; }
function setup(groupLimit = 256 * 1024) {
  const ledger = new VoiceByteLedger(2 * 1024 * 1024, groupLimit);
  const funding = VoiceAudioFunding.create(ledger);
  return { ledger, funding };
}
function authority() {
  const status = new VoiceMediaStatus("44".repeat(32));
  status.finish(status.begin(100), { proof: "44".repeat(32), admitted: true, current: true,
    closed: false, released: false, retireInMs: 0 }, 100);
  return { status, clock: { now: () => 100, schedule() { assert.fail(); }, cancel() { assert.fail(); } } };
}
function stream(failStop: () => boolean = () => false) {
  let stops = 0, enabled = true;
  const track = { kind: "audio", readyState: "live", get enabled() { return enabled; },
    set enabled(value: boolean) { enabled = value; }, stop() { if (failStop()) throw new Error("busy device"); stops++; } };
  return { stream: { getTracks: () => [track] } as unknown as MediaStream, stops: () => stops, enabled: () => enabled };
}

test("released capsule keeps original fixed group fee until its actual finalizer returns", async () => {
  const { ledger, funding } = setup(), old = funding.bind(A), task = gate<void>();
  const payment = old.budget.reserve(); payment.trackTask(task.promise); old.close();
  const next = funding.bind(B); payment.release();
  assert.equal(ledger.groups.get(A), FIXED); assert.equal(ledger.groups.get(B), FIXED);
  assert.equal(funding.inspect().pending, true); assert.equal(ledger.bytes, FIXED);
  task.release(); await task.promise;
  assert.equal(ledger.groups.has(A), false); assert.equal(funding.inspect().pending, false);
  next.close(); assert.equal(ledger.groups.size, 0); assert.equal(ledger.bytes, FIXED);
});

test("returned finalizer keeps capsule and group ownership until actual master release", async () => {
  const { ledger, funding } = setup(), old = funding.bind(A), task = gate<void>();
  const payment = old.budget.reserve(); payment.trackTask(task.promise); old.close();
  task.release(); await task.promise;
  assert.equal(ledger.groups.get(A), FIXED + CAPSULE); assert.equal(funding.inspect().pending, true);
  payment.release(); payment.release();
  assert.equal(ledger.groups.size, 0); assert.equal(funding.inspect().pending, false);
  assert.equal(ledger.bytes, FIXED);
});

test("explicit no-task refusal releases only after both local transitions", () => {
  for (const releaseFirst of [false, true]) {
    const { ledger, funding } = setup(), scope = funding.bind(A), payment = scope.budget.reserve();
    scope.close();
    if (releaseFirst) { payment.release(); assert.equal(ledger.groups.get(A), FIXED); payment.noTaskEntered(); }
    else { payment.noTaskEntered(); assert.equal(ledger.groups.get(A), FIXED + CAPSULE); payment.release(); }
    assert.equal(funding.inspect().pending, false); assert.equal(ledger.groups.size, 0);
    assert.throws(() => payment.trackTask(Promise.resolve()));
  }
});

test("one observed task cannot be replaced or gain another completion observer", async () => {
  const { ledger, funding } = setup(), scope = funding.bind(A), task = gate<void>(), foreign = gate<void>();
  const payment = scope.budget.reserve(); payment.trackTask(task.promise); scope.close(); payment.release();
  assert.throws(() => payment.trackTask(foreign.promise)); assert.throws(() => payment.noTaskEntered());
  foreign.release(); await foreign.promise;
  assert.equal(ledger.groups.get(A), FIXED); assert.equal(funding.inspect().pending, true);
  task.release(); await task.promise;
  assert.equal(ledger.groups.size, 0); assert.equal(funding.inspect().pending, false);
});

test("same-group replacement shares the fixed fee while distinct replacements stay bounded", async () => {
  const { ledger, funding } = setup(), original = funding.bind(A), task = gate<void>();
  const payment = original.budget.reserve(); payment.trackTask(task.promise); original.close();
  const same = funding.bind(A);
  assert.equal(ledger.groups.get(A), FIXED + CAPSULE); assert.equal(funding.inspect().groups, 1);
  assert.throws(() => funding.bind(B)); same.close();
  for (let at = 0; at < 40; at++) {
    const replacement = funding.bind(at % 2 ? B : C);
    assert.equal(funding.inspect().groups, 2);
    assert.throws(() => replacement.budget.reserve());
    replacement.close(); replacement.close();
    assert.equal(funding.inspect().groups, 1);
  }
  payment.release(); task.release(); await task.promise;
  assert.equal(ledger.groups.size, 0); assert.equal(ledger.bytes, FIXED);
  assert.throws(() => original.budget.reserve());
});

test("ordinary pressure and refused active bindings do not strand extra leases", () => {
  const { ledger, funding } = setup(16 * 1024), scope = funding.bind(A);
  const pressure = reserveVoiceBytePayment(ledger, A, CAPSULE);
  const before = ledger.bytes;
  assert.throws(() => scope.budget.reserve()); assert.equal(ledger.bytes, before);
  assert.equal(funding.inspect().pending, false); assert.equal(funding.inspect().groups, 1);
  assert.throws(() => funding.bind(B));
  pressure.release();
  const payment = scope.budget.reserve(); payment.noTaskEntered(); payment.release(); scope.close();
  assert.equal(ledger.groups.size, 0); assert.equal(ledger.bytes, FIXED);
});

test("late original closure and callbacks cannot clear a replacement capsule", async () => {
  const { ledger, funding } = setup(), old = funding.bind(A), task = gate<void>();
  const payment = old.budget.reserve(); payment.trackTask(task.promise); old.close(); payment.release();
  task.release(); await task.promise;
  const next = funding.bind(A), nextTask = gate<void>(), nextPayment = next.budget.reserve();
  nextPayment.trackTask(nextTask.promise);
  old.close(); payment.release(); task.release(); await Promise.resolve();
  assert.equal(funding.inspect().pending, true); assert.equal(ledger.groups.get(A), FIXED + CAPSULE);
  next.close(); nextPayment.release(); nextTask.release(); await nextTask.promise;
  assert.equal(ledger.groups.size, 0); assert.equal(ledger.bytes, FIXED);
});

test("actual capture End permits a new group before the old OS promise returns", async () => {
  const { ledger, funding } = setup(), pool = new VoiceAudioCapturePool(), scope = funding.bind(A);
  const old = pool.session(scope.budget), held = gate<MediaStream>(), entered = gate<void>();
  assert.equal(old.request(authority(), "", { getUserMedia() { entered.release(); return held.promise; } }), true);
  const actualFinalizer = pool.settlement(); await entered.promise;
  old.close(); scope.close();
  const nextScope = funding.bind(B), next = pool.session(nextScope.budget);
  assert.equal(next.request(authority(), "", { async getUserMedia() { assert.fail("old OS position remains owned"); } }), false);
  assert.equal(ledger.groups.get(A), FIXED + CAPSULE); assert.equal(ledger.groups.get(B), FIXED);
  const late = stream(); held.release(late.stream); await actualFinalizer;
  assert.equal(late.stops(), 1); assert.equal(next.available, false); assert.equal(ledger.groups.has(A), false);
  const fresh = stream();
  assert.equal(next.request(authority(), "", { async getUserMedia() { return fresh.stream; } }), true);
  await pool.settlement(); assert.equal(next.available, true); assert.equal(fresh.enabled(), false);
  next.close(); nextScope.close();
  assert.equal(ledger.groups.size, 0); assert.equal(ledger.bytes, FIXED);
});

test("real pre-SDK copy refusal uses the explicit no-task transition", () => {
  const { ledger, funding } = setup(), pool = new VoiceAudioCapturePool(), scope = funding.bind(A);
  const session = pool.session(scope.budget), original = String.fromCharCode;
  String.fromCharCode = () => { throw new Error("copy refused before SDK"); };
  try { assert.equal(session.request(authority(), "device", { async getUserMedia() { assert.fail("no SDK task"); } }), false); }
  finally { String.fromCharCode = original; }
  assert.equal(pool.pending, false); assert.equal(funding.inspect().pending, false);
  assert.equal(ledger.groups.get(A), FIXED); session.close(); scope.close();
  assert.equal(ledger.groups.size, 0); assert.equal(ledger.bytes, FIXED);
});

test("failed actual track stop retains old-group debt through replacement and cleanup retry", async () => {
  const { ledger, funding } = setup(), pool = new VoiceAudioCapturePool(), scope = funding.bind(A);
  const old = pool.session(scope.budget); let fail = true; const master = stream(() => fail);
  old.request(authority(), "", { async getUserMedia() { return master.stream; } }); await pool.settlement();
  old.close(); scope.close();
  const nextScope = funding.bind(B), next = pool.session(nextScope.budget);
  assert.equal(ledger.groups.get(A), FIXED + CAPSULE); assert.equal(funding.inspect().pending, true);
  next.close(); nextScope.close();
  fail = false; pool.retry(); await Promise.resolve();
  assert.equal(master.stops(), 1); assert.equal(ledger.groups.size, 0); assert.equal(ledger.bytes, FIXED);
});

test("bounded funding and serial ownership copies fit the unchanged fixed pool", () => {
  assert.ok(voiceAudioFundingInventory() <= FIXED);
  const { ledger, funding } = setup();
  const group = ("parent".repeat(100_000) + A).slice(-64), scope = funding.bind(group);
  assert.equal(ledger.groups.get(A), FIXED); scope.close();
  assert.equal(ledger.groups.size, 0); assert.equal(ledger.bytes, FIXED);
});

test("group binding cannot call a mutable string-copy hook into old quarantine cleanup", async () => {
  const { ledger, funding } = setup(), pool = new VoiceAudioCapturePool(), scope = funding.bind(A);
  const session = pool.session(scope.budget); let fail = true; const master = stream(() => fail);
  session.request(authority(), "", { async getUserMedia() { return master.stream; } }); await pool.settlement();
  session.close(); scope.close();
  assert.equal(ledger.groups.get(A), FIXED + CAPSULE);
  const original = String.fromCharCode; let entered = 0;
  String.fromCharCode = (...units: number[]) => { entered++; fail = false; pool.retry(); return original(...units); };
  let next: ReturnType<VoiceAudioFunding["bind"]>;
  try { next = funding.bind(B); }
  finally { String.fromCharCode = original; }
  assert.equal(entered, 0, "canonical group materialization has no mutable SDK callout");
  assert.equal(ledger.groups.get(A), FIXED + CAPSULE);
  fail = false; pool.retry(); await Promise.resolve();
  assert.equal(ledger.groups.has(A), false); assert.equal(ledger.groups.get(B), FIXED);
  next!.close(); assert.equal(ledger.groups.size, 0);
});
