import assert from "node:assert/strict";
import test from "node:test";
import { webcrypto } from "node:crypto";
import { VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker } from "./voice-crypto.ts";
import { DEFAULT_VOICE_BROWSER_BUDGET, VOICE_BROWSER_PROCESS_BYTES,
  copyVoiceBrowserBudget, voiceBrowserBudget } from "./voice-browser-budget.ts";

const native = { async admit() { return "ordinary"; }, async settle() {}, async commitInbound() {} };
const real = webcrypto.subtle as unknown as SubtleCrypto;
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }

test("local voice budgets use a512KiB group default and unchanged2MiB process aggregate", () => {
  assert.deepEqual(DEFAULT_VOICE_BROWSER_BUDGET, { groupBytes: 512 * 1024, processBytes: 2 * 1024 * 1024 });
  for (const kib of [256, 257, 512, 1024]) {
    const value = voiceBrowserBudget(kib);
    assert.ok(Object.isFrozen(value)); assert.equal(value.groupBytes, kib * 1024);
    assert.equal(value.processBytes, VOICE_BROWSER_PROCESS_BYTES);
  }
});

test("malformed, fractional and out-of-range local settings refuse without coercion", () => {
  for (const value of [undefined, null, "512", 0, -1, 255, 1025, 511.5, NaN, Infinity, {}, 1n]) {
    if (value === undefined) continue; // Omitted setting explicitly selects the default.
    assert.throws(() => voiceBrowserBudget(value as number), /invalid voice browser memory budget/);
  }
  for (const value of [null, undefined, {}, { groupBytes: 512 * 1024 },
    { groupBytes: 512 * 1024, processBytes: 4 * 1024 * 1024 },
    { groupBytes: 512 * 1024 + 1, processBytes: VOICE_BROWSER_PROCESS_BYTES }])
    assert.throws(() => copyVoiceBrowserBudget(value as never), /invalid voice browser memory budget/);
});

test("budget fields are read once and later caller mutation cannot change the captured policy", () => {
  let groupReads = 0, processReads = 0;
  const input = { get groupBytes() { groupReads++; return 512 * 1024; },
    get processBytes() { processReads++; return VOICE_BROWSER_PROCESS_BYTES; } };
  const policy = copyVoiceBrowserBudget(input);
  assert.equal(groupReads, 1); assert.equal(processReads, 1); assert.notEqual(policy, input);
  assert.ok(Object.isFrozen(policy));
  const mutable = { groupBytes: 256 * 1024, processBytes: VOICE_BROWSER_PROCESS_BYTES };
  const lower = copyVoiceBrowserBudget(mutable); mutable.groupBytes = 1024 * 1024;
  assert.equal(lower.groupBytes, 256 * 1024);
});

test("the original broker ledger enforces each selected group bound and the same process aggregate", async () => {
  for (const kib of [256, 512, 1024]) {
    const policy = voiceBrowserBudget(kib), broker = new VoiceCryptoBroker(native, real, undefined, undefined, policy);
    const held = gate(), owners = Array.from({ length: policy.processBytes / policy.groupBytes }, (_, index) =>
      createVoiceCryptoOwner("voice-budget-" + index, index.toString(16).padStart(64, "0"), () => true));
    const jobs = owners.flatMap(owner => Array.from({ length: policy.groupBytes / (256 * 1024) }, () =>
      broker.run(owner, "generate", 256 * 1024, () => held.promise)));
    assert.equal(broker.counts().bytes, policy.processBytes);
    await assert.rejects(broker.run(owners[0], "generate", 1, async () => {}), /capacity reached/);
    const extra = createVoiceCryptoOwner("voice-budget-extra", "ff".repeat(32), () => true);
    await assert.rejects(broker.run(extra, "generate", 1, async () => {}), /capacity reached/);
    held.release(); await Promise.all(jobs);
    await new Promise<void>(yes => setImmediate(yes));
    assert.equal(broker.counts().bytes, 0);
    assert.equal(broker.memoryBudget.groupBytes, kib * 1024);
  }
});

test("retained policy mismatch cannot resize, replace or erase an occupied original ledger", async () => {
  const host = {}, policy = voiceBrowserBudget(256), owner = createVoiceCryptoOwner("voice-budget", "11".repeat(32), () => true);
  const broker = windowVoiceCryptoBroker(native, host, real, undefined, policy), held = gate();
  const running = broker.run(owner, "generate", 64, () => held.promise);
  assert.equal(windowVoiceCryptoBroker(native, host, real, broker, voiceBrowserBudget(256)), broker);
  for (const next of [512, 1024])
    assert.throws(() => windowVoiceCryptoBroker(native, host, real, undefined, voiceBrowserBudget(next)), /budget changed; reload required/);
  assert.equal(Reflect.set(broker, "memoryBudget", voiceBrowserBudget(1024)), false);
  assert.equal(broker.counts().bytes, 64); assert.equal(broker.memoryBudget.groupBytes, 256 * 1024);
  held.release(); await running; await new Promise<void>(yes => setImmediate(yes));
  assert.equal(broker.counts().bytes, 0);
});

test("ABI49 refusal leaves the original old broker cleanup callable", async () => {
  const host = {}, broker = windowVoiceCryptoBroker(native, host, real), held = gate();
  const owner = createVoiceCryptoOwner("voice-old-budget", "11".repeat(32), () => true);
  const running = broker.run(owner, "generate", 64, () => held.promise);
  Object.defineProperty(broker, "version", { value: 49 });
  assert.throws(() => windowVoiceCryptoBroker(native, host, real), /version changed; reload required/);
  assert.equal(broker.counts().bytes, 64);
  held.release(); await running; await new Promise<void>(yes => setImmediate(yes));
  assert.equal(broker.counts().bytes, 0);
});

test("constructor consumes caller policy fields once and owns immutable primitive limits", () => {
  let groupReads = 0, processReads = 0;
  const broker = new VoiceCryptoBroker(native, real, undefined, undefined,
    { get groupBytes() { groupReads++; return 512 * 1024; },
      get processBytes() { processReads++; return VOICE_BROWSER_PROCESS_BYTES; } });
  assert.equal(groupReads, 1); assert.equal(processReads, 1); assert.ok(Object.isFrozen(broker.memoryBudget));
  assert.equal(broker.counts().bytes, 0);
});

test("a lower requested limit requires reload and does not strand the old occupied policy", async () => {
  const host = {}, broker = windowVoiceCryptoBroker(native, host, real), held = gate();
  const owner = createVoiceCryptoOwner("voice-lower-budget", "11".repeat(32), () => true);
  const work = Promise.all([broker.run(owner, "generate", 256 * 1024, () => held.promise),
    broker.run(owner, "generate", 256 * 1024, () => held.promise)]);
  assert.throws(() => windowVoiceCryptoBroker(native, host, real, broker, voiceBrowserBudget(256)), /budget changed; reload required/);
  assert.equal(broker.counts().bytes, 512 * 1024); assert.equal(broker.memoryBudget.groupBytes, 512 * 1024);
  held.release(); await work; await new Promise<void>(yes => setImmediate(yes));
  assert.equal(broker.counts().bytes, 0);
  assert.throws(() => windowVoiceCryptoBroker(native, host, real, broker, voiceBrowserBudget(256)), /budget changed; reload required/);
});

test("reentrant caller policy capture cannot allocate another broker beside its published original", () => {
  const host = {}; let original: VoiceCryptoBroker | null = null, reads = 0;
  const requested = { get groupBytes() {
    reads++; original = windowVoiceCryptoBroker(native, host, real, undefined, voiceBrowserBudget(256));
    return 512 * 1024;
  }, processBytes: VOICE_BROWSER_PROCESS_BYTES };
  assert.throws(() => windowVoiceCryptoBroker(native, host, real, undefined, requested), /budget changed; reload required/);
  assert.equal(reads, 1);
  assert.equal(windowVoiceCryptoBroker(native, host, real, undefined, voiceBrowserBudget(256)), original);
});

test("larger local aggregates never enlarge the original256KiB single-job boundary", async () => {
  for (const kib of [512, 1024]) {
    let providers = 0, admissions = 0;
    const countedNative = { ...native, async admit() { admissions++; return "ordinary"; } };
    const broker = new VoiceCryptoBroker(countedNative, real, undefined, undefined, voiceBrowserBudget(kib));
    const owner = createVoiceCryptoOwner("voice-job-bound", "11".repeat(32), () => true);
    await assert.rejects(broker.run(owner, "generate", 256 * 1024 + 1, async () => { providers++; }), /capacity reached/);
    assert.equal(broker.counts().bytes, 0); assert.equal(providers, 0); assert.equal(admissions, 0);
    await broker.run(owner, "generate", 256 * 1024, async () => { providers++; });
    // The public result is earlier than the original job's actual finally/refund.
    await new Promise<void>(yes => setImmediate(yes));
    assert.equal(providers, 1); assert.equal(admissions, 1); assert.equal(broker.counts().bytes, 0);
  }
});
