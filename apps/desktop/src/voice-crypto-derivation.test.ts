import { DEFAULT_VOICE_BROWSER_BUDGET } from "./voice-browser-budget.ts";
import assert from "node:assert/strict";
import test from "node:test";
import type { TestContext } from "node:test";
import { webcrypto } from "node:crypto";
import { VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker } from "./voice-crypto.ts";
import type { VoiceCryptoChainBinding, VoiceDerivationInput, VoiceDerivationNative } from "./voice-crypto.ts";

const real = webcrypto.subtle as unknown as SubtleCrypto;
const hash = (n: number) => n.toString(16).padStart(64, "0");
const emptyTag = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const deadline = performance.now() + 15_000;
  while (!check()) { if (performance.now() >= deadline) throw new Error("condition did not settle"); await new Promise<void>(yes => setImmediate(yes)); }
}
async function fixture(t: TestContext, transcript = new TextEncoder().encode("native selected transcript")) {
  let now = 100, derives = 0, imports = 0, hkdfs = 0, admits = 0, commits = 0, settlements = 0, transitions = 0;
  let loseAdmit = false, loseCommit = false, reject = false, historical = false;
  let hold: ReturnType<typeof gate> | null = null, settleHold: ReturnType<typeof gate> | null = null;
  let heldAt = "deriveBits";
  let failedAt = "";
  let engineEntered = false;
  const engine = new Proxy(real, { get(target, name) {
    const value = Reflect.get(target, name);
    if (typeof value !== "function") return value;
    return async (...args: unknown[]) => {
      if (name === "deriveBits") derives++;
      if (name === "importKey") imports++;
      if (name === "deriveKey") hkdfs++;
      if (name === failedAt) throw new Error("engine refused");
      const result = await value.apply(target, args);
      if (name === heldAt && hold) { engineEntered = true; await hold.promise; }
      return result;
    };
  } });
  const ordinary = { async admit() { return "ordinary"; }, async settle() {}, async commitInbound() {} };
  const owner = createVoiceCryptoOwner("owner", hash(1), () => true);
  const broker = new VoiceCryptoBroker(ordinary, engine, () => now);
  t.after(async () => { hold?.release(); settleHold?.release(); owner.retire(); await until(() => broker.counts().active === 0); });
  const key = await broker.createJoinKeyHandle(owner), peer = await broker.createJoinKeyHandle(owner);
  await until(() => broker.counts().active === 0);
  const binding: VoiceCryptoChainBinding = {
    chain: "9007199254740993", epoch: "0", localDevice: hash(2), localJoin: hash(3), localKey: hash(4),
    peerDevice: hash(5), peerJoin: hash(6), peerKey: hash(7), handshake: hash(8), generation: "1", nativeDeadline: "15000", localIsLow: true,
  };
  broker.bindJoinKey(owner, key, binding.localJoin, binding.localKey, key.publicKey());
  const setup = { localPublicKey: key.publicKey(), peerPublicKey: peer.publicKey(), transcript };
  const makePlan = () => broker.prepareChain(owner, key, binding, 15_000, setup);
  const input: VoiceDerivationInput = { owner: owner.token, action: "9007199254740995", version: "9007199254740997", inputTag: hash(9),
    binding, ...setup, engineBytes: 3 * setup.transcript.length + 4096 + 256 };
  let receipt: Awaited<ReturnType<VoiceDerivationNative["commitDerivation"]>> | null = null;
  const result = () => reject
    ? { ...identity(), kind: "rejected" as const, resultTag: emptyTag, receipt: "receipt", code: "invalid" as const }
    : { ...identity(), kind: "accepted" as const, resultTag: emptyTag, receipt: "receipt", derived: true as const };
  const identity = () => ({ owner: owner.token, action: input.action, version: input.version, inputTag: input.inputTag });
  const native: VoiceDerivationNative = {
    async admitAction(id, bytes) {
      admits++; assert.deepEqual(id, identity()); assert.equal(bytes, input.engineBytes);
      if (historical) return result();
      if (loseAdmit) { loseAdmit = false; throw new Error("admission reply lost"); }
      return { ...id, kind: "admitted", permit: "original-permit" };
    },
    async commitDerivation(id, tag) {
      commits++; assert.deepEqual(id, identity()); assert.equal(tag, emptyTag);
      if (!receipt) { transitions++; receipt = result(); }
      if (loseCommit) { loseCommit = false; throw new Error("terminal reply lost"); }
      return receipt;
    },
    async settleAction() { settlements++; if (settleHold) await settleHold.promise; },
  };
  return {
    owner, broker, key, binding, setup, input, native, ordinary, engine, makePlan,
    register(plan = makePlan(), packet = input) { return broker.registerProtectedDerivation(owner, plan, native, packet); },
    counts: () => ({ derives, imports, hkdfs, admits, commits, settlements, transitions }),
    loseAdmit: () => { loseAdmit = true; }, loseCommit: () => { loseCommit = true; },
    reject: () => { reject = true; }, historical: () => { historical = true; },
    advance: (value: number) => { now = value; broker.collectChains(); },
    holdEngine(name = "deriveBits") { heldAt = name; hold = gate(); return { release: hold.release, entered: () => engineEntered }; },
    holdSettle() { settleHold = gate(); return settleHold; },
    failEngine(name: string) { failedAt = name; },
  };
}

test("protected derivation coalesces one actual ECDH/HKDF and publishes keys after native acceptance", async t => {
  const f = await fixture(t), plan = f.makePlan(), action = f.register(plan);
  assert.equal(f.register(plan), action);
  assert.equal(action.pair(), null);
  const a = action.attempt(), b = action.attempt(); assert.equal(a, b);
  assert.equal((await a).kind, "accepted");
  await until(() => f.broker.counts().active === 0);
  assert.ok(action.pair());
  assert.deepEqual(f.counts(), { derives: 1, imports: 2, hkdfs: 2, admits: 1, commits: 1, settlements: 1, transitions: 1 });
  assert.equal((await action.attempt()).kind, "accepted"); assert.equal(f.counts().derives, 1);
  const pair = action.pair()!; plan.close(); assert.equal(action.pair(), null);
  assert.throws(() => pair.encrypt(new Uint8Array(), 1n, new Uint8Array()), /retired/);
  assert.throws(() => f.makePlan(), /retired/);
});

for (const rejected of [false, true]) test(`lost ${rejected ? "rejected" : "accepted"} derivation reply never repeats the engine`, async t => {
  const f = await fixture(t); if (rejected) f.reject(); f.loseCommit();
  const action = f.register();
  assert.equal((await action.attempt()).kind, "uncertain"); assert.equal(action.pair(), null);
  assert.equal(f.counts().derives, 1); assert.equal(f.broker.counts().active, 1);
  assert.equal((await action.attempt()).kind, rejected ? "rejected" : "accepted");
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().derives, 1); assert.equal(f.counts().commits, 2); assert.equal(f.counts().transitions, 1);
  assert.equal(!!action.pair(), !rejected);
});

test("lost admission retries the native identity before a single engine computation", async t => {
  const f = await fixture(t); f.loseAdmit(); const action = f.register();
  assert.equal((await action.attempt()).kind, "uncertain"); assert.equal(f.counts().derives, 0);
  assert.equal((await action.attempt()).kind, "accepted"); assert.equal(f.counts().admits, 2); assert.equal(f.counts().derives, 1);
});

for (const rejected of [false, true]) test(`terminal native history without local keys is ${rejected ? "rejected" : "accepted history only"}`, async t => {
  const f = await fixture(t); f.historical(); if (rejected) f.reject(); const action = f.register();
  assert.equal((await action.attempt()).kind, rejected ? "rejected" : "accepted");
  assert.equal(action.pair(), null); assert.equal(f.counts().derives, 0); assert.equal(f.counts().commits, 0);
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().settlements, 0, "past history cannot settle an earlier browser's engine");
});

test("preparation refuses both occupied positions and later protects its position from the ordinary queue", async t => {
  const f = await fixture(t), held = gate(); t.after(held.release);
  const running = [f.broker.run(f.owner, "derive", 1, () => held.promise), f.broker.run(f.owner, "derive", 1, () => held.promise)];
  assert.throws(f.makePlan, /capacity/);
  held.release(); await Promise.all(running); await until(() => f.broker.counts().active === 0);
  const plan = f.makePlan(), action = f.register(plan), ordinary = gate(); t.after(ordinary.release);
  const queued = Array.from({ length: 33 }, () => f.broker.run(f.owner, "derive", 1, () => ordinary.promise));
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().queued, 32);
  assert.equal((await action.attempt()).kind, "accepted"); assert.equal(f.counts().derives, 1);
  ordinary.release(); await Promise.all(queued);
});

test("20 ordinary identities plus twelve protected row claims exhaust the combined table", async t => {
  const f = await fixture(t);
  const aes = await real.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]) as CryptoKey;
  const pair = f.broker.createPair(f.owner, aes, aes, "fixture");
  const unused = { async admitAction() { throw new Error("not reached"); }, async commitConfirmation() { throw new Error("not reached"); }, async settleAction() {} };
  const input = (n: number) => ({ action: String(n), version: "1", inputTag: hash(n), handshake: hash(8), deliveryToken: String(n), kind: "hello" as const,
    sequence: String(n), aad: new Uint8Array(), ciphertext: new Uint8Array(16) });
  for (let n = 1; n <= 20; n++) f.broker.registerConfirmation(f.owner, pair, unused, input(n));
  const plan = f.makePlan(); f.register(plan);
  assert.throws(() => f.broker.registerConfirmation(f.owner, pair, unused, input(21)), /capacity/);
  plan.close();
  assert.doesNotThrow(() => f.broker.registerConfirmation(f.owner, pair, unused, input(21)));
  assert.throws(() => f.register(plan, { ...f.input, action: "777" }), /invalid/);
});

for (const field of ["chain", "epoch", "localDevice", "localJoin", "localKey", "peerDevice", "peerJoin", "peerKey", "handshake", "generation", "nativeDeadline", "localIsLow"] as const) {
  test(`changed ${field} refuses before native admission`, async t => {
    const f = await fixture(t), plan = f.makePlan();
    const value = field === "localIsLow" ? false : ["chain", "epoch", "generation", "nativeDeadline"].includes(field) ? "2" : hash(90);
    assert.throws(() => f.register(plan, { ...f.input, binding: { ...f.binding, [field]: value } }), /invalid/);
    assert.equal(f.counts().admits, 0); assert.equal(f.counts().derives, 0);
  });
}

test("native setup and engine byte count are immutable before and after publication", async t => {
  const f = await fixture(t), plan = f.makePlan();
  assert.throws(() => f.register(plan, { ...f.input, engineBytes: f.input.engineBytes + 1 }), /invalid/);
  assert.throws(() => f.register(plan, { ...f.input, transcript: new Uint8Array(f.input.transcript.length), engineBytes: f.input.engineBytes }), /invalid/);
  assert.throws(() => f.register(plan, { ...f.input, peerPublicKey: f.input.localPublicKey }), /invalid/);
  const action = f.register(plan);
  assert.throws(() => f.register(plan, { ...f.input, inputTag: hash(99) }), /identity changed/);
  assert.throws(() => f.register(plan, { ...f.input, action: "777" }), /already issued/);
  f.input.transcript.fill(0); // The published record uses its own immutable copy.
  assert.equal((await action.attempt()).kind, "accepted");
});

for (const boundary of ["importKey", "deriveBits", "digest", "deriveKey"]) test(`close during actual ${boundary} retains execution until settlement`, async t => {
  const f = await fixture(t), plan = f.makePlan(), action = f.register(plan), hold = f.holdEngine(boundary);
  const attempt = action.attempt(); await until(hold.entered);
  const bytes = f.broker.counts().bytes;
  plan.close(); assert.equal((await attempt).kind, "retired"); assert.equal(action.pair(), null);
  assert.equal(f.broker.counts().active, 1); assert.ok(f.broker.counts().bytes >= bytes - 22 * 1024);
  hold.release(); await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().commits, 0); assert.equal(f.counts().settlements, 1);
  assert.ok(f.broker.counts().bytes < 20 * 1024);
});

test("actual settle RPC holds the reserved position across End", async t => {
  const f = await fixture(t), plan = f.makePlan(), action = f.register(plan), held = f.holdSettle();
  assert.equal((await action.attempt()).kind, "accepted"); await until(() => f.counts().settlements === 1);
  f.owner.retire(); assert.equal(f.broker.counts().active, 1); assert.ok(f.broker.counts().bytes >= 96 * 1024);
  held.release(); await until(() => f.broker.counts().active === 0); assert.equal(f.broker.counts().bytes, 0);
});

test("protected keys refuse ordinary encrypt, decrypt and confirmation fallback before native or engine work", async t => {
  const f = await fixture(t), plan = f.makePlan(), action = f.register(plan);
  assert.equal((await action.attempt()).kind, "accepted"); await until(() => f.broker.counts().active === 0);
  const pair = action.pair()!, before = f.counts();
  assert.throws(() => pair.encrypt(new Uint8Array(), 1n, new Uint8Array([1])), /protected native/);
  await assert.rejects(pair.open(new Uint8Array(), 1n, new Uint8Array(16), "ordinary"), /protected native/);
  const unused = { async admitAction() { throw new Error("not reached"); }, async commitConfirmation() { throw new Error("not reached"); }, async settleAction() {} };
  assert.throws(() => f.broker.registerConfirmation(f.owner, pair, unused, {
    action: "1", version: "1", inputTag: hash(1), handshake: f.binding.handshake, deliveryToken: "1", kind: "hello", sequence: "1",
    aad: new Uint8Array(), ciphertext: new Uint8Array(16),
  }), /protected native/);
  assert.deepEqual(f.counts(), before);
});

test("original expiry closes unused positions and cannot be renewed by repull", async t => {
  const f = await fixture(t), before = f.broker.counts().bytes, plan = f.makePlan();
  assert.equal(f.makePlan(), plan);
  assert.throws(() => f.broker.prepareChain(f.owner, f.key, f.binding, 15_001, f.setup), /changed/);
  f.advance(15_000); assert.equal(plan.isCurrent(), false);
  assert.equal(f.broker.counts().bytes, before + 8 * 1024);
  assert.throws(f.makePlan, /invalid|retired/);
});

test("foreign handles and changed accepted native local points cannot adopt a private key", async t => {
  const f = await fixture(t);
  assert.throws(() => f.broker.bindJoinKey(f.owner, Object.freeze({ publicKey: () => f.key.publicKey() }), f.binding.localJoin, f.binding.localKey, f.key.publicKey()), /binding/);
  assert.throws(() => f.broker.bindJoinKey(f.owner, f.key, f.binding.localJoin, f.binding.localKey, f.setup.peerPublicKey), /binding/);
  assert.throws(() => f.broker.bindJoinKey(f.owner, f.key, hash(77), f.binding.localKey, f.key.publicKey()), /binding/);
  const other = createVoiceCryptoOwner("other", hash(1), () => true); t.after(() => other.retire());
  assert.throws(() => f.broker.prepareChain(other, f.key, f.binding, 15_000, f.setup), /binding/);
});

test("retained HMR broker shares the continuing protected action and refuses older broker versions", async t => {
  const f = await fixture(t), host = { [Symbol.for("mewtual.voice.crypto-broker.v2")]: f.broker }, plan = f.makePlan(), action = f.register(plan);
  f.loseCommit(); assert.equal((await action.attempt()).kind, "uncertain");
  const url = new URL("./voice-crypto.ts", import.meta.url); url.search = "derive-hmr";
  const fresh = await import(url.href);
  const broker = fresh.windowVoiceCryptoBroker(f.ordinary, host, f.engine);
  assert.equal(broker, f.broker); assert.equal(broker.registerProtectedDerivation(f.owner, plan, f.native, f.input), action);
  assert.equal((await action.attempt()).kind, "accepted"); assert.equal(f.counts().derives, 1);
  assert.throws(() => windowVoiceCryptoBroker(f.ordinary, { [Symbol.for("mewtual.voice.crypto-broker.v2")]: { version: 4 } }, f.engine), /reload/);
});

test("reserved derivation succeeds with the ordinary group byte pool full", async t => {
  const f = await fixture(t), plan = f.makePlan(), action = f.register(plan), held = gate(); t.after(held.release);
  const free = DEFAULT_VOICE_BROWSER_BUDGET.groupBytes - f.broker.counts().bytes;
  const ordinary = Promise.all(Array.from({ length: Math.ceil(free / (256 * 1024)) }, (_, index) =>
    f.broker.run(f.owner, "derive", Math.min(256 * 1024, free - index * 256 * 1024), () => held.promise)));
  assert.equal(f.broker.counts().bytes, DEFAULT_VOICE_BROWSER_BUDGET.groupBytes);
  assert.equal((await action.attempt()).kind, "accepted"); assert.equal(f.counts().derives, 1);
  held.release(); await ordinary;
});

test("failed plan admission leaves no phantom claims, bytes or permanent stamp", async t => {
  const f = await fixture(t), held = gate(); t.after(held.release);
  const before = f.broker.counts().bytes;
  // Keep the original capacity-refusal contract at the selected local limit.
  // A historical200KiB filler no longer fills the larger default group budget.
  const pressure = DEFAULT_VOICE_BROWSER_BUDGET.groupBytes - before - 1;
  const ordinary = Promise.all(Array.from({ length: Math.ceil(pressure / (256 * 1024)) }, (_, index) =>
    f.broker.run(f.owner, "derive", Math.min(256 * 1024, pressure - index * 256 * 1024), () => held.promise)));
  assert.throws(f.makePlan, /capacity/); assert.equal(f.broker.counts().bytes, before + pressure);
  held.release(); await ordinary; await until(() => f.broker.counts().active === 0);
  assert.doesNotThrow(f.makePlan);
});

test("maximum transcript is frozen before admission and one byte beyond refuses without charging", async t => {
  const f = await fixture(t, new Uint8Array(987).fill(7));
  const before = f.broker.counts().bytes;
  assert.throws(() => f.broker.prepareChain(f.owner, f.key, f.binding, 15_000, { ...f.setup, transcript: new Uint8Array(988) }), /invalid/);
  assert.equal(f.broker.counts().bytes, before);
  const plan = f.makePlan(), action = f.register(plan);
  f.setup.transcript.fill(0);
  assert.equal((await action.attempt()).kind, "accepted"); assert.equal(f.counts().derives, 1);
  assert.equal(f.input.engineBytes, 7_313);
});

test("epoch must be an exact string before plan charge or publication", async t => {
  const f = await fixture(t), before = f.broker.counts().bytes;
  for (const epoch of [0, ["0"], { toString: () => "0", hidden: new Uint8Array(100_000) }]) {
    assert.throws(() => f.broker.prepareChain(f.owner, f.key, { ...f.binding, epoch } as unknown as VoiceCryptoChainBinding, 15_000, f.setup), /invalid/);
    assert.equal(f.broker.counts().bytes, before);
  }
  assert.doesNotThrow(f.makePlan);
});

test("coercible tags cannot retain undeclared objects or consume a protected identity", async t => {
  const f = await fixture(t), plan = f.makePlan(), before = f.broker.counts().bytes;
  for (const inputTag of [[hash(9)], { toString: () => hash(9), hidden: new Uint8Array(100_000) }]) {
    assert.throws(() => f.register(plan, { ...f.input, inputTag } as unknown as VoiceDerivationInput), /invalid/);
    assert.equal(f.broker.counts().bytes, before);
  }
  assert.equal((await f.register(plan).attempt()).kind, "accepted");
});

test("opaque join key handles have a finite per-owner inventory", async t => {
  const f = await fixture(t);
  for (let n = 2; n < 16; n++) await f.broker.createJoinKeyHandle(f.owner);
  await assert.rejects(f.broker.createJoinKeyHandle(f.owner), /capacity/);
});

for (const field of ["owner", "resultTag", "derived", "extra"]) test(`malformed derivation ${field} reply stays uncertain without exposing keys`, async t => {
  const f = await fixture(t), commit = f.native.commitDerivation;
  let corrupt = true;
  f.native.commitDerivation = async (...args) => {
    const answer = await commit(...args);
    if (!corrupt) return answer;
    corrupt = false;
    return { ...answer, [field]: field === "derived" ? false : "wrong" } as Awaited<ReturnType<typeof commit>>;
  };
  const action = f.register();
  assert.equal((await action.attempt()).kind, "uncertain"); assert.equal(action.pair(), null);
  assert.equal((await action.attempt()).kind, "accepted"); assert.ok(action.pair());
  assert.equal(f.counts().derives, 1); assert.equal(f.counts().transitions, 1);
});

test("engine failure is permanent for a published native derive identity", async t => {
  const f = await fixture(t); f.failEngine("deriveBits"); const plan = f.makePlan(), action = f.register(plan);
  assert.equal((await action.attempt()).kind, "failed");
  assert.equal((await f.register(plan).attempt()).kind, "failed");
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().derives, 1); assert.equal(f.counts().commits, 0); assert.equal(action.pair(), null);
});

for (const operation of ["encrypt", "decrypt"] as const) test(`${operation} final wrapper refuses retirement in the work-to-broker microtask gap`, async () => {
  let live = true, checks = 0;
  let decrypted: Uint8Array | null = null;
  const subtle = new Proxy(real, { get(target, name) {
    if (name === "decrypt") return async (...args: Parameters<SubtleCrypto["decrypt"]>) => {
      const result = await target.decrypt(...args); decrypted = new Uint8Array(result); return result;
    };
    const value = Reflect.get(target, name); return typeof value === "function" ? value.bind(target) : value;
  } });
  const ordinary = { async admit() { return "actual"; }, async settle() {}, async commitInbound() {} };
  const owner = createVoiceCryptoOwner("gap-owner", hash(1), () => true);
  const broker = new VoiceCryptoBroker(ordinary, subtle);
  const key = await real.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]) as CryptoKey;
  const pair = broker.createPair(owner, key, key, "gap", () => {
    if (++checks === (operation === "encrypt" ? 3 : 4)) queueMicrotask(() => { live = false; });
    return live;
  });
  if (operation === "encrypt") await assert.rejects(pair.encrypt(new Uint8Array(), 1n, new Uint8Array([1, 2, 3])), /retired/);
  else {
    const iv = new Uint8Array(12); iv[11] = 1;
    const ciphertext = new Uint8Array(await real.encrypt({ name: "AES-GCM", iv }, key, new Uint8Array([1, 2, 3])));
    await assert.rejects(pair.open(new Uint8Array(), 1n, ciphertext, "native"), /retired/);
    assert.deepEqual(decrypted, new Uint8Array(3), "the withheld late plaintext is cleared");
  }
  await until(() => broker.counts().active === 0); owner.retire();
});
