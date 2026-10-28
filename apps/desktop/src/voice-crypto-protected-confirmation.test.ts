import { DEFAULT_VOICE_BROWSER_BUDGET } from "./voice-browser-budget.ts";
import assert from "node:assert/strict";
import test from "node:test";
import type { TestContext } from "node:test";
import { createHash, webcrypto } from "node:crypto";
import { VoiceCryptoBroker, createVoiceCryptoOwner } from "./voice-crypto.ts";
import type { VoiceCryptoChainBinding, VoiceDerivationNative, VoiceProtectedConfirmationInput,
  VoiceProtectedConfirmationNative, VoiceConfirmationIdentity } from "./voice-crypto.ts";

// Real P-256/HKDF/AES-GCM. The native adapter is explicitly a fault seam, not native IPC/RTC
// acceptance. Rust tests separately cover the original protected factories/token transitions.
const real = webcrypto.subtle as unknown as SubtleCrypto;
const hash = (n: number) => n.toString(16).padStart(64, "0");
const text = (s: string) => new TextEncoder().encode(s);
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const deadline = performance.now() + 15_000;
  while (!check()) { if (performance.now() >= deadline) throw new Error("condition did not settle"); await new Promise<void>(yes => setImmediate(yes)); }
}
const identity = (input: VoiceConfirmationIdentity) => ({ owner: input.owner, action: input.action, version: input.version, inputTag: input.inputTag });

async function fixture(t: TestContext) {
  const ordinary = { async admit() { return "ordinary"; }, async settle() {}, async commitInbound() { throw new Error("protected plaintext escaped"); } };
  let serial = 100;
  const endpoints = [true, false].map(low => {
    let encrypts = 0, decrypts = 0, admitCalls = 0, commits = 0, settlements = 0, transitions = 0;
    let loseAdmit = false, loseCommit = false, reject = false;
    let engineHold: ReturnType<typeof gate> | null = null, settleHold: ReturnType<typeof gate> | null = null;
    const engine = new Proxy(real, { get(target, name) {
      const value = Reflect.get(target, name);
      if (typeof value !== "function") return value;
      return async (...args: unknown[]) => {
        if (name === "encrypt" || name === "decrypt") {
          if (name === "encrypt") encrypts++; else decrypts++;
          if (engineHold) await engineHold.promise;
        }
        return value.apply(target, args);
      };
    } });
    const owner = createVoiceCryptoOwner(low ? "low" : "high", hash(1), () => true);
    const broker = new VoiceCryptoBroker(ordinary, engine, () => 100);
    const receipts = new Map<string, Awaited<ReturnType<VoiceProtectedConfirmationNative["commitProtectedConfirmation"]>>>();
    const outputs = new Map<string, Uint8Array>();
    const native: VoiceProtectedConfirmationNative = {
      async admitAction(id) {
        admitCalls++;
        if (loseAdmit) { loseAdmit = false; throw new Error("admitted; reply lost"); }
        return receipts.get(id.action) ?? { ...id, kind: "admitted", permit: "same-permit" };
      },
      async commitProtectedConfirmation(id, output, resultTag) {
        commits++;
        assert.equal(createHash("sha256").update(output).digest("hex"), resultTag);
        let receipt = receipts.get(id.action);
        if (!receipt) {
          transitions++; outputs.set(id.action, output.slice());
          receipt = reject
            ? { ...id, kind: "rejected", resultTag, receipt: "receipt-" + id.action, code: "invalid" }
            : { ...id, kind: "accepted", resultTag, receipt: "receipt-" + id.action, confirmation: "continue" };
          receipts.set(id.action, receipt);
        } else assert.equal(receipt.resultTag, resultTag);
        if (loseCommit) { loseCommit = false; throw new Error("committed; reply lost"); }
        return receipt;
      },
      async settleAction() { settlements++; if (settleHold) await settleHold.promise; },
    };
    t.after(async () => { engineHold?.release(); settleHold?.release(); owner.retire(); await until(() => broker.counts().active === 0); });
    return { low, owner, broker, native, outputs, receipts,
      counts: () => ({ encrypts, decrypts, admitCalls, commits, settlements, transitions }),
      loseAdmission: () => { loseAdmit = true; }, loseResult: () => { loseCommit = true; }, reject: () => { reject = true; },
      holdEngine: () => engineHold = gate(), holdSettlement: () => settleHold = gate(),
    };
  });
  const keys = await Promise.all(endpoints.map(end => end.broker.createJoinKeyHandle(end.owner)));
  const transcript = text("actual native-selected protected pair transcript");
  const prepared = await Promise.all(endpoints.map(async (end, index) => {
    const own = end.low ? 2 : 5, peer = end.low ? 5 : 2;
    const binding: VoiceCryptoChainBinding = {
      chain: "1", epoch: "0", localDevice: hash(own), localJoin: hash(own + 1), localKey: hash(own + 2),
      peerDevice: hash(peer), peerJoin: hash(peer + 1), peerKey: hash(peer + 2), handshake: hash(8), generation: "1", nativeDeadline: "15000", localIsLow: end.low,
    };
    end.broker.bindJoinKey(end.owner, keys[index], binding.localJoin, binding.localKey, keys[index].publicKey());
    const setup = { localPublicKey: keys[index].publicKey(), peerPublicKey: keys[1 - index].publicKey(), transcript };
    const plan = end.broker.prepareChain(end.owner, keys[index], binding, 15_000, setup);
    const deriveNative: VoiceDerivationNative = {
      async admitAction(id) { return { ...id, kind: "admitted", permit: "derive" }; },
      async commitDerivation(id, resultTag) { return { ...id, kind: "accepted", resultTag, receipt: "derived", derived: true }; },
      async settleAction() {},
    };
    const derive = end.broker.registerProtectedDerivation(end.owner, plan, deriveNative, {
      owner: end.owner.token, action: String(index + 1), version: "1", inputTag: hash(9), binding, ...setup,
      engineBytes: 3 * transcript.length + 4096 + 256,
    });
    assert.equal((await derive.attempt()).kind, "accepted");
    await until(() => end.broker.counts().active === 0);
    return { ...end, binding, plan, derive,
      input(row: number, body: Uint8Array, sequence = String(row), aad = text("native authenticated header")): VoiceProtectedConfirmationInput {
        const step = (row - 1) % 3, operation = (end.low ? step !== 1 : step === 1) ? "seal" : "open";
        return { owner: end.owner.token, action: String(++serial), version: "1", inputTag: hash(serial), binding,
          row, operation, kind: (["hello", "proof", "finish"] as const)[step], sequence, aad, body,
          engineBytes: 4 * (aad.length + body.length + (operation === "seal" ? 16 : 0)) + 3 * 512 + 2048 };
      },
      register(input: VoiceProtectedConfirmationInput) { return end.broker.registerProtectedConfirmation(end.owner, plan, end.native, input); },
    };
  }));
  return { low: prepared[0], high: prepared[1] };
}

test("protected real WebCrypto completes all three flights without exposing confirmation plaintext", async t => {
  const { low, high } = await fixture(t);
  for (const [sender, receiver, row, sequence] of [[low, high, 1, "1"], [high, low, 2, "1"], [low, high, 3, "2"]] as const) {
    const body = text("closed native " + row), input = sender.input(row, body, sequence);
    const seal = sender.register(input);
    assert.equal(sender.register({ ...input, aad: input.aad.slice(), body: input.body.slice() }), seal);
    assert.equal((await seal.attempt()).kind, "accepted");
    await until(() => sender.broker.counts().active === 0);
    const ciphertext = sender.outputs.get(input.action)!;
    assert.equal(ciphertext.length, body.length + 16);
    const incoming = receiver.input(row, ciphertext, sequence), open = receiver.register(incoming);
    const result = await open.attempt();
    assert.equal(result.kind, "accepted");
    assert.deepEqual(Object.keys(result), ["kind", "receipt"]);
    await until(() => receiver.broker.counts().active === 0);
    assert.deepEqual(receiver.outputs.get(incoming.action), body);
  }
  assert.equal(low.counts().encrypts, 2); assert.equal(low.counts().decrypts, 1);
  assert.equal(high.counts().encrypts, 1); assert.equal(high.counts().decrypts, 2);
  assert.throws(() => low.derive.pair()!.encrypt(new Uint8Array(), 10n, text("bypass")), /protected/);
});

test("all three native candidate lanes retain original rows and accept maximum confirmation bodies", async t => {
  const { low, high } = await fixture(t);
  const retained: Array<{ end: typeof low; input: VoiceProtectedConfirmationInput; action: ReturnType<typeof low.register> }> = [];
  let lowSequence = 0, highSequence = 0;
  for (let candidate = 0; candidate < 3; candidate++) {
    for (let step = 0; step < 3; step++) {
      const sender = step === 1 ? high : low, receiver = step === 1 ? low : high;
      const row = candidate * 3 + step + 1, sequence = String(step === 1 ? ++highSequence : ++lowSequence);
      const body = new Uint8Array(512).fill(row), aad = new Uint8Array(4096).fill(row);
      const input = sender.input(row, body, sequence, aad), action = sender.register(input);
      assert.equal((await action.attempt()).kind, "accepted");
      await until(() => sender.broker.counts().active === 0);
      retained.push({ end: sender, input, action });
      const incoming = receiver.input(row, sender.outputs.get(input.action)!, sequence, aad);
      const open = receiver.register(incoming);
      assert.equal((await open.attempt()).kind, "accepted");
      await until(() => receiver.broker.counts().active === 0);
      retained.push({ end: receiver, input: incoming, action: open });
      assert.deepEqual(receiver.outputs.get(incoming.action), body);
    }
  }
  for (const { end, input, action } of retained) assert.equal(end.register(input), action);
  assert.throws(() => low.register(low.input(10, text("fourth Hello"), "7")), /invalid/);
  assert.equal(low.counts().encrypts, 6); assert.equal(low.counts().decrypts, 3);
  assert.equal(high.counts().encrypts, 3); assert.equal(high.counts().decrypts, 6);
});

for (const rejected of [false, true]) test(`protected lost ${rejected ? "rejection" : "success"} retries immutable completed output without AEAD rerun`, async t => {
  const { low } = await fixture(t), input = low.input(1, text("Hello")), action = low.register(input);
  if (rejected) low.reject();
  low.loseResult();
  assert.equal((await action.attempt()).kind, "uncertain");
  assert.equal(low.counts().encrypts, 1); assert.equal(low.counts().settlements, 0);
  const first = action.attempt(), second = action.attempt(); assert.equal(first, second);
  assert.equal((await first).kind, rejected ? "rejected" : "accepted");
  await until(() => low.broker.counts().active === 0);
  assert.equal(low.counts().encrypts, 1); assert.equal(low.counts().transitions, 1); assert.equal(low.counts().commits, 2);
  assert.equal(low.register(input), action);
});

test("protected admission lost reply retries before computing, and immutable row cannot branch", async t => {
  const { low } = await fixture(t), input = low.input(1, text("Hello")), action = low.register(input);
  low.loseAdmission();
  assert.equal((await action.attempt()).kind, "uncertain"); assert.equal(low.counts().encrypts, 0);
  assert.equal((await action.attempt()).kind, "accepted");
  await until(() => low.broker.counts().active === 0);
  assert.equal(low.counts().encrypts, 1);
  assert.throws(() => low.register(low.input(1, input.body)), /unavailable/);
  for (const change of [{ body: text("HellX") }, { sequence: "7" }, { aad: new Uint8Array(input.aad).fill(1) }, { row: 4 }]) {
    assert.throws(() => low.register({ ...input, ...change }), /changed/);
  }
});

test("protected projections own buffers even when a Uint8Array subclass slice aliases its caller", async t => {
  const { low, high } = await fixture(t);
  const expected = text("Hello"), header = text("native authenticated header");
  // Node Buffer passes instanceof Uint8Array but its slice() shares the original allocation.
  const body = Buffer.from(expected), aad = Buffer.from(header);
  const input = low.input(1, body, "1", aad), action = low.register(input);
  body.fill(0); aad.fill(0);
  assert.equal((await action.attempt()).kind, "accepted");
  await until(() => low.broker.counts().active === 0);
  const ciphertext = low.outputs.get(input.action)!;
  const incoming = high.input(1, ciphertext, "1", header);
  assert.equal((await high.register(incoming).attempt()).kind, "accepted");
  assert.deepEqual(high.outputs.get(incoming.action), expected);
});

test("protected rows serialize through actual prior settlement and keep their reserved position", async t => {
  const { low } = await fixture(t), hold = low.holdSettlement();
  const first = low.register(low.input(1, text("Hello"), "1"));
  assert.equal((await first.attempt()).kind, "accepted");
  await until(() => low.counts().settlements === 1);
  const next = low.register(low.input(4, text("new Hello"), "2"));
  const pending = next.attempt();
  assert.equal(low.counts().admitCalls, 1); assert.equal(low.counts().encrypts, 1);
  assert.equal(low.broker.counts().active, 1);
  hold.release();
  assert.equal((await pending).kind, "accepted");
  await until(() => low.broker.counts().active === 0);
  assert.equal(low.counts().encrypts, 2);
});

test("protected work progresses at the full ordinary byte and queue limits", async t => {
  const { low } = await fixture(t), held = gate(); t.after(held.release);
  const free = DEFAULT_VOICE_BROWSER_BUDGET.groupBytes - low.broker.counts().bytes;
  const ordinary = Promise.all(Array.from({ length: Math.ceil(free / (256 * 1024)) }, (_, index) =>
    low.broker.run(low.owner, "derive", Math.min(256 * 1024, free - index * 256 * 1024), () => held.promise)));
  // One bounded pressure job can already occupy a queue position. Fill the SAME
  // original32-position queue, not32 additional positions beside that owned job.
  const queued = Array.from({ length: 32 - low.broker.counts().queued }, () => low.broker.run(low.owner, "derive", 0, () => held.promise));
  assert.equal(low.broker.counts().bytes, DEFAULT_VOICE_BROWSER_BUDGET.groupBytes); assert.equal(low.broker.counts().queued, 32);
  const action = low.register(low.input(1, text("Hello")));
  assert.equal((await action.attempt()).kind, "accepted");
  held.release(); await Promise.all([ordinary, ...queued]);
});

test("closing during real crypto retains custody until the promise and settlement drain", async t => {
  const { low } = await fixture(t), hold = low.holdEngine();
  const input = low.input(1, text("Hello")), action = low.register(input), pending = action.attempt();
  await until(() => low.counts().encrypts === 1);
  low.owner.retire();
  assert.equal((await pending).kind, "retired");
  assert.equal(low.broker.counts().active, 1); assert.ok(low.broker.counts().bytes >= 96 * 1024);
  hold.release(); await until(() => low.broker.counts().active === 0);
  assert.equal(low.counts().commits, 0); assert.equal(low.counts().settlements, 1);
  assert.equal(low.broker.counts().bytes, 0);
});

test("duplicate direction counters and corrupted ciphertext fail without native commits", async t => {
  const { low, high } = await fixture(t);
  const firstInput = low.input(1, text("Hello"), "1");
  assert.equal((await low.register(firstInput).attempt()).kind, "accepted");
  await until(() => low.broker.counts().active === 0);
  assert.equal((await low.register(low.input(4, text("Hello again"), "1")).attempt()).kind, "failed");
  assert.equal(low.counts().encrypts, 1);
  const corrupted = low.outputs.get(firstInput.action)!.slice(); corrupted[0] ^= 1;
  assert.equal((await high.register(high.input(1, corrupted, "1")).attempt()).kind, "failed");
  await until(() => high.broker.counts().active === 0);
  assert.equal(high.counts().commits, 0);
});

for (const rejected of [false, true]) test(`protected historical ${rejected ? "rejection" : "success"} cannot recompute or settle another browser`, async t => {
  const { low } = await fixture(t), input = low.input(1, text("Hello"));
  const receipt = rejected
    ? { ...identity(input), kind: "rejected" as const, resultTag: hash(8), receipt: "past", code: "invalid" as const }
    : { ...identity(input), kind: "accepted" as const, resultTag: hash(8), receipt: "past", confirmation: "continue" as const };
  low.receipts.set(input.action, receipt);
  assert.equal((await low.register(input).attempt()).kind, rejected ? "rejected" : "accepted");
  await until(() => low.broker.counts().active === 0);
  assert.equal(low.counts().encrypts, 0); assert.equal(low.counts().commits, 0); assert.equal(low.counts().settlements, 0);
});

test("protected tagless terminal admission refusal cannot settle another browser's retained engine", async t => {
  const { low } = await fixture(t), input = low.input(1, text("Hello"));
  low.native.admitAction = async id => ({ ...id, kind: "rejected", receipt: "past-refusal", code: "retired" });
  assert.equal((await low.register(input).attempt()).kind, "rejected");
  await until(() => low.broker.counts().active === 0);
  assert.equal(low.counts().encrypts, 0); assert.equal(low.counts().commits, 0); assert.equal(low.counts().settlements, 0);
});

test("role, row, binding and exact engine charge reject before native admission", async t => {
  const { low } = await fixture(t), input = low.input(1, text("Hello"));
  const before = low.broker.counts().bytes;
  for (const change of [{ row: 0 }, { row: 10 }, { operation: "open" as const }, { kind: "proof" as const },
    { engineBytes: input.engineBytes + 1 }, { binding: { ...input.binding, epoch: "1" } }, { owner: "high" },
    { sequence: "01" }, { sequence: "18446744073709551616" }, { aad: new Uint8Array(4097) }, { body: new Uint8Array(513) }]) {
    assert.throws(() => low.register({ ...input, ...change }), /invalid/);
  }
  assert.equal(low.counts().admitCalls, 0); assert.equal(low.broker.counts().bytes, before);
});
