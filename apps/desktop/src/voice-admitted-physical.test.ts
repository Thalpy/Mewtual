import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { webcrypto } from "node:crypto";
import { VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker } from "./voice-crypto.ts";
import { voiceAdmittedPhysicalNative } from "./voice-admitted-physical-native.ts";
import { voiceAdmittedJoinKeyNative } from "./voice-admitted-join-native.ts";
import { VOICE_ADMITTED_PHYSICAL_BYTES, VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES,
  voiceAdmittedPhysicalInventory } from "./voice-admitted-physical-inventory.ts";
import { VOICE_ADMITTED_SELECTED_KEY_BYTES } from "./voice-admitted-key-inventory.ts";
import type { VoiceIpcClock } from "./voice-crypto.ts";

const real = webcrypto.subtle as unknown as SubtleCrypto;
const hash = (n: number) => n.toString(16).padStart(64, "0");
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const deadline = performance.now() + 10_000;
  while (!check()) { if (performance.now() > deadline) throw new Error("physical fixture exceeded bound"); await new Promise<void>(yes => setImmediate(yes)); }
}
function clock() {
  let now = 0, serial = 0;
  const timers = new Map<number, { due: number; work(): void }>();
  const value: VoiceIpcClock = { now: () => now, schedule: (work, ms) => { const id = ++serial; timers.set(id, { due: now + ms, work }); return id; },
    cancel: id => { timers.delete(id as number); } };
  return { value, timers, advance(ms = 1000) { now += ms; for (const [id, timer] of [...timers]) if (timer.due <= now) { timers.delete(id); timer.work(); } } };
}
function u64(n: bigint) { const out = Buffer.alloc(8); out.writeBigUInt64BE(n); return out; }
function u128(n: bigint) { return Buffer.concat([u64(n >> 64n), u64(n & ((1n << 64n) - 1n))]); }
function short(value: Buffer) { const prefix = Buffer.alloc(2); prefix.writeUInt16BE(value.length); return Buffer.concat([prefix, value]); }

/** Browser-side contract fixture: real SDK operations and original-key ECDH,
 * mocked native metadata. It is NOT native Root/HTTP/RTC/physical acceptance. */
async function fixture(t: TestContext, token = "physical-owner", verifyOwnProvider = true) {
  const nativePair = await real.generateKey({ name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]) as CryptoKeyPair;
  const nativePoint = new Uint8Array(await real.exportKey("raw", nativePair.publicKey));
  const time = clock(), owner = createVoiceCryptoOwner(token, hash(1), () => true);
  let generates = 0, exports = 0, sdkReturned = 0, derives = 0, inflight = 0, admitted = false, ended = false, collected = false;
  let point = "", transcript: string | null = null, executor = "";
  let sdkHold: ReturnType<typeof gate> | null = null, deriveHold: ReturnType<typeof gate> | null = null;
  let ackHold: ReturnType<typeof gate> | null = null;
  let loseSetup = 0, loseParent = 0, loseFinal = 0, failFinal = false, loseObserve = 0;
  let admissionNeverEntered = false, refuseAdmission = false, observeMutator: ((value: unknown) => unknown) | null = null;
  let actualShared = "", actualSeed = "";
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const engine = new Proxy(real, { get(target, name) {
    const method = Reflect.get(target, name);
    if (typeof method !== "function") return method;
    return async (...args: unknown[]) => {
      inflight++;
      try {
      if (name === "generateKey") { generates++; assert.equal(args[1], false); if (sdkHold) await sdkHold.promise; }
      if (name === "exportKey") exports++;
      if (name === "deriveBits") { derives++; if (deriveHold) await deriveHold.promise; }
      const result = await method.apply(target, args);
      if (name === "deriveBits") {
        const algorithm = (args[0] as { name: string }).name;
        if (algorithm === "ECDH") actualShared = Buffer.from(result as ArrayBuffer).toString("hex");
        if (algorithm === "HKDF") actualSeed = Buffer.from(result as ArrayBuffer).toString("hex");
      }
      if (name === "deriveKey") sdkReturned++;
      return result;
      } finally { inflight--; }
    };
  } });
  const invoke = async <T>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    const identity = { owner: owner.token, executor: args.executor ?? args.executorId, permit: "physical-permit", join: hash(2) };
    if (command === "admit_admitted_physical_voice_join_key") {
      if (refuseAdmission) return { ...identity, kind: "refused", permit: "", join: "" } as T;
      if (admissionNeverEntered) throw new Error("request never entered");
      admitted = true; executor ||= String(args.executor); assert.equal(args.executor, executor);
      return { ...identity, kind: "admitted" } as T;
    }
    if (command === "publish_admitted_voice_join_key") {
      if (point) assert.equal(args.publicKey, point); else point = String(args.publicKey);
      return { ...identity, kind: "accepted", keyRef: hash(3) } as T;
    }
    if (command === "setup_admitted_voice_channel") {
      assert.equal(ended, false); assert.equal(admitted, true);
      if (!transcript) transcript = Buffer.concat([
        Buffer.from("Mewtual admitted native voice bootstrap\0"), Buffer.from([2, 1, 1]), u128(1n), Buffer.from([1]),
        u64(1n), u64(7n), u64(9n), short(Buffer.from("native raw group")), Buffer.from(owner.group, "hex"), Buffer.from([0]),
        u128(42n), Buffer.from(hash(4), "hex"), Buffer.from(hash(2), "hex"), short(Buffer.from(owner.token)),
        Buffer.from(executor, "hex"), Buffer.from(point, "hex"), Buffer.from(nativePoint),
      ]).toString("hex");
      if (loseSetup-- > 0) throw new Error("lost setup reply");
      return { kind: "ready", owner: owner.token, executor, port: 1428, transcript } as T;
    }
    if (command === "acknowledge_admitted_voice_activation") {
      if (verifyOwnProvider && !ended) assert.equal(sdkReturned, 4, "parent ACK follows all real construction SDK returns");
      if (loseParent-- > 0) throw new Error("parent ACK reply lost");
      return undefined as T;
    }
    if (command === "acknowledge_admitted_voice_key_publication" || command === "settle_admitted_voice_join_key") {
      if (verifyOwnProvider && !ended) assert.equal(sdkReturned, 4, "physical construction precedes key settlement");
      return undefined as T;
    }
    if (command === "end_voice_owner") { ended = true; return undefined as T; }
    if (command === "settle_admitted_voice_channel_setup") {
      if (verifyOwnProvider) assert.equal(inflight, 0, "negative settlement follows the actual held SDK");
      return undefined as T;
    }
    if (command === "collect_admitted_voice_channel") { if (transcript) collected = true; return undefined as T; }
    if (command === "observe_admitted_voice_channel") {
      assert.equal(ended, true);
      if (loseObserve-- > 0) throw new Error("unknown physical observation");
      const value = !admitted ? { kind: "no-physical-intent", owner: owner.token } : !transcript ?
        { kind: "not-started", owner: owner.token, executor, join: hash(2), receipt: hash(8) } : collected ?
        { kind: "collected", owner: owner.token, executor, join: hash(2), session: "0".repeat(31) + "1", receipt: hash(9) } :
        { kind: "pending", owner: owner.token, executor, join: hash(2) };
      return (observeMutator ? observeMutator(value) : value) as T;
    }
    assert.equal(command, "acknowledge_admitted_voice_channel");
    assert.equal(args.receipt, transcript ? hash(9) : hash(8));
    if (ackHold) await ackHold.promise;
    if (failFinal || loseFinal-- > 0) throw new Error("unknown final ACK (including NoProof)");
    return undefined as T;
  };
  const native = voiceAdmittedPhysicalNative(invoke);
  const ordinary = { async admit() { throw new Error("ordinary crypto forbidden"); }, async settle() {}, async commitInbound() {} };
  const broker = new VoiceCryptoBroker(ordinary, engine, time.value.now, time.value);
  t.after(async () => {
    sdkHold?.release(); deriveHold?.release(); ackHold?.release();
    loseSetup = loseParent = loseFinal = loseObserve = 0; failFinal = false; observeMutator = null;
    await broker.retireNativeOwner(owner);
    await until(() => broker.counts().active === 0);
    await broker.abandonAdmittedPhysicalReceipts();
  });
  return { owner, native, broker, calls, time, engine, ordinary, invoke,
    counts: () => ({ generates, exports, derives, sdkReturned }),
    hold(phase: "generate" | "derive" | "final") { const held = gate(); if (phase === "generate") sdkHold = held; else if (phase === "derive") deriveHold = held; else ackHold = held; return held; },
    lose(phase: "setup" | "parent" | "final" | "observe") { if (phase === "setup") loseSetup++; else if (phase === "parent") loseParent++; else if (phase === "final") loseFinal++; else loseObserve++; },
    failFinal(value: boolean) { failFinal = value; }, neverAdmit() { admissionNeverEntered = true; },
    refuse() { refuseAdmission = true; },
    async verifyOriginalSeed() {
      assert.ok(transcript && point);
      const peer = await real.importKey("raw", Buffer.from(point, "hex"), { name: "ECDH", namedCurve: "P-256" }, false, []);
      const shared = await real.deriveBits({ name: "ECDH", public: peer }, nativePair.privateKey, 256);
      assert.equal(Buffer.from(shared).toString("hex"), actualShared);
      const base = await real.importKey("raw", shared, "HKDF", false, ["deriveBits"]);
      const salt = await real.digest("SHA-256", Buffer.from(transcript!, "hex"));
      const seed = await real.deriveBits({ name: "HKDF", hash: "SHA-256", salt,
        info: new TextEncoder().encode("Mewtual admitted native voice bootstrap seed v2") }, base, 256);
      assert.equal(Buffer.from(seed).toString("hex"), actualSeed, "actual constructor uses SAME original key and new domain");
      new Uint8Array(shared).fill(0); new Uint8Array(seed).fill(0);
    },
    observation(mutator: ((value: unknown) => unknown) | null) { observeMutator = mutator; },
  };
}

test("admitted physical factory reuses one original SDK key and exposes bootstrap only", async t => {
  const f = await fixture(t), channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal(f.broker.admittedPhysicalChannel(f.owner, f.native), channel);
  assert.deepEqual(Object.keys(channel), ["attempt", "close"]);
  assert.equal((await channel.attempt()).kind, "bootstrap-only");
  await until(() => f.broker.counts().active === 0);
  assert.deepEqual(f.counts(), { generates: 1, exports: 1, derives: 2, sdkReturned: 4 });
  await f.verifyOriginalSeed();
  assert.equal(f.calls.some(call => /read_voice|setup_voice_channel$/.test(call.command)), false);
  await channel.close();
  await until(() => f.broker.counts().bytes === 0);
  assert.equal((await channel.attempt()).kind, "retired");
});

test("physical and key-only factories refuse cross-profile adoption before payment", async t => {
  const f = await fixture(t), before = f.broker.counts();
  assert.throws(() => f.broker.selectJoinKey(f.owner, f.native as never), /admitted/);
  assert.throws(() => f.broker.selectAdmittedJoinKey(f.owner, f.native as never), /original admitted/);
  assert.throws(() => f.broker.admittedPhysicalChannel(f.owner, { ...f.native }), /original physical/);
  assert.throws(() => f.broker.admittedPhysicalChannel(f.owner, voiceAdmittedJoinKeyNative(async <T>() => undefined as T) as never), /original physical/);
  assert.deepEqual(f.broker.counts(), before);
  const channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  const paid = f.broker.counts();
  const replacement = createVoiceCryptoOwner(f.owner.token, f.owner.group, () => true);
  assert.throws(() => f.broker.admittedPhysicalChannel(replacement, f.native), /selection changed/);
  assert.throws(() => f.broker.selectAdmittedJoinKey(f.owner, voiceAdmittedJoinKeyNative(async <T>() => undefined as T)), /selection changed/);
  assert.deepEqual(f.broker.counts(), paid); replacement.retire();
  await channel.close(); await until(() => f.broker.counts().bytes === 0);
});

for (const phase of ["setup", "parent"] as const) test(`lost physical ${phase} reply recovers the SAME key and construction`, async t => {
  const f = await fixture(t); f.lose(phase);
  const channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal((await channel.attempt()).kind, "uncertain");
  assert.equal((await channel.attempt()).kind, "bootstrap-only");
  await until(() => f.broker.counts().active === 0);
  assert.deepEqual(f.counts(), { generates: 1, exports: 1, derives: 2, sdkReturned: 4 });
  await channel.close(); await until(() => f.broker.counts().bytes === 0);
});

test("End during real physical ECDH cannot settle or refund before the same provider returns", async t => {
  const f = await fixture(t), held = f.hold("derive"), channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  const attempt = channel.attempt();
  await until(() => f.counts().derives === 1);
  const before = f.broker.counts().bytes;
  const closed = f.broker.retireNativeOwner(f.owner);
  assert.equal((await attempt).kind, "retired");
  assert.equal(f.broker.counts().bytes, before);
  assert.equal(f.calls.some(call => call.command === "settle_admitted_voice_channel_setup"), false);
  held.release();
  await closed; await until(() => f.broker.counts().bytes === 0);
  assert.equal(f.counts().generates, 1);
});

test("unentered physical admission followed by End uses exact no-intent, never SDK or fake join", async t => {
  const f = await fixture(t); f.neverAdmit();
  const channel = f.broker.admittedPhysicalChannel(f.owner, f.native), attempt = channel.attempt();
  await until(() => f.calls.some(call => call.command === "admit_admitted_physical_voice_join_key"));
  const close = f.broker.retireNativeOwner(f.owner);
  assert.equal((await attempt).kind, "retired");
  await close; await until(() => f.broker.counts().bytes === 0);
  assert.equal(f.counts().generates, 0);
  assert.equal(f.calls.some(call => call.command === "acknowledge_admitted_voice_channel"), false);
});

test("lost final ACK leaves only the SAME bounded negative descriptor and never claims delivery", async t => {
  const f = await fixture(t), channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal((await channel.attempt()).kind, "bootstrap-only");
  await until(() => f.broker.counts().active === 0);
  f.failFinal(true);
  await channel.close();
  await until(() => f.broker.counts().bytes === VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES);
  const calls = f.calls.filter(call => call.command === "acknowledge_admitted_voice_channel");
  assert.equal(calls.length, 1);
  for (let i = 0; i < 3; i++) { f.time.advance(); await new Promise<void>(yes => setImmediate(yes)); }
  const retried = f.calls.filter(call => call.command === "acknowledge_admitted_voice_channel");
  assert.equal(retried.length, 4);
  assert.ok(retried.every(call => JSON.stringify(call.args) === JSON.stringify(calls[0].args)));
  assert.equal(f.counts().generates, 1);
  assert.equal(f.broker.counts().bytes, VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES);
  f.failFinal(false); f.broker.retrySelectedJoinKeys();
  await until(() => f.broker.counts().bytes === 0);
});

test("physical full graph is prepaid in the original aggregate before admission", async t => {
  const f = await fixture(t);
  assert.ok(voiceAdmittedPhysicalInventory().total <= VOICE_ADMITTED_PHYSICAL_BYTES);
  f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal(f.broker.counts().bytes, VOICE_ADMITTED_SELECTED_KEY_BYTES + VOICE_ADMITTED_PHYSICAL_BYTES);
  assert.equal(f.calls.length, 0);
});

test("malformed physical terminal cannot release custody or evaluate a receipt getter", async t => {
  const f = await fixture(t), channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal((await channel.attempt()).kind, "bootstrap-only");
  await until(() => f.broker.counts().active === 0);
  let reads = 0;
  f.observation(value => {
    const result = { ...(value as Record<string, unknown>) };
    Object.defineProperty(result, "receipt", { enumerable: true, get() { reads++; return hash(9); } });
    return result;
  });
  const before = f.broker.counts().bytes;
  const closed = channel.close();
  await until(() => f.calls.some(call => call.command === "observe_admitted_voice_channel"));
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(reads, 0);
  assert.equal(f.broker.counts().bytes, before, "unknown terminal retains heavy original cleanup");
  assert.equal(f.calls.some(call => call.command === "acknowledge_admitted_voice_channel"), false);
  f.observation(null); f.broker.retrySelectedJoinKeys();
  await closed; await until(() => f.broker.counts().bytes === 0);
});

test("explicit receipt shutdown joins a genuinely held final invoke before ceding retries", async t => {
  const f = await fixture(t), channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal((await channel.attempt()).kind, "bootstrap-only");
  await until(() => f.broker.counts().active === 0);
  const held = f.hold("final"); f.failFinal(true);
  await channel.close();
  await until(() => f.calls.some(call => call.command === "acknowledge_admitted_voice_channel"));
  assert.equal(f.broker.counts().bytes, VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES);
  let returned = false;
  const abandoned = f.broker.abandonAdmittedPhysicalReceipts().then(() => { returned = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(returned, false, "no fake timer/invoke return on shutdown");
  assert.equal(f.broker.counts().bytes, VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES);
  held.release(); await abandoned;
  await until(() => f.broker.counts().bytes === 0);
});

test("old retained broker recognizes a real physical factory from another module without adopting work", async t => {
  const f = await fixture(t);
  const replacementModule = await import(new URL("./voice-admitted-physical-native.ts?physical-remount", import.meta.url).href) as typeof import("./voice-admitted-physical-native.ts");
  const fresh = replacementModule.voiceAdmittedPhysicalNative(f.invoke);
  assert.notEqual(fresh, f.native);
  const channel = f.broker.admittedPhysicalChannel(f.owner, fresh);
  assert.equal((await channel.attempt()).kind, "bootstrap-only");
  assert.throws(() => f.broker.admittedPhysicalChannel(f.owner, f.native), /selection changed/);
  assert.throws(() => f.broker.admittedPhysicalChannel(f.owner, { ...fresh }), /original physical/);
  await channel.close(); await until(() => f.broker.counts().bytes === 0);
});

test("physical setup transfers the same two positions and a queued third waits actual release", async t => {
  // All three use f's ONE actual provider; the other native fixtures' SDK
  // counters intentionally aren't presented as their own provider evidence.
  const f = await fixture(t, "physical-a", false), second = await fixture(t, "physical-b", false), third = await fixture(t, "physical-c", false);
  const a = f.broker.admittedPhysicalChannel(f.owner, f.native);
  const b = f.broker.admittedPhysicalChannel(second.owner, second.native);
  assert.equal((await a.attempt()).kind, "bootstrap-only");
  assert.equal((await b.attempt()).kind, "bootstrap-only");
  await until(() => f.broker.counts().active === 0);
  const c = f.broker.admittedPhysicalChannel(third.owner, third.native), waiting = c.attempt();
  assert.equal(f.broker.counts().queued, 1);
  assert.equal(f.counts().generates, 2);
  await a.close();
  assert.equal((await waiting).kind, "bootstrap-only");
  assert.equal(f.counts().generates, 3);
  await b.close(); await c.close();
  await until(() => f.broker.counts().active === 0 && f.broker.counts().bytes === 0);
});

test("known physical purpose refusal creates no SDK, End, ACK or settlement against the other family", async t => {
  const f = await fixture(t); f.refuse();
  const channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal((await channel.attempt()).kind, "retired");
  await channel.close(); await until(() => f.broker.counts().bytes === 0);
  assert.equal(f.counts().generates, 0);
  assert.deepEqual(f.calls.map(call => call.command), ["admit_admitted_physical_voice_join_key"]);
});

test("lost actual collection observation keeps heavy ownership until the same terminal is recovered", async t => {
  const f = await fixture(t), channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal((await channel.attempt()).kind, "bootstrap-only");
  await until(() => f.broker.counts().active === 0);
  f.lose("observe");
  const before = f.broker.counts().bytes, closed = channel.close();
  await until(() => f.calls.some(call => call.command === "observe_admitted_voice_channel"));
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.broker.counts().bytes, before);
  f.broker.retrySelectedJoinKeys();
  await closed; await until(() => f.broker.counts().bytes === 0);
  assert.equal(f.counts().generates, 1);
  assert.equal(f.calls.filter(call => call.command === "acknowledge_admitted_voice_channel").length, 1);
});

test("physical metadata snapshots the same bounded own-key list exactly once", async t => {
  const f = await fixture(t), channel = f.broker.admittedPhysicalChannel(f.owner, f.native);
  assert.equal((await channel.attempt()).kind, "bootstrap-only");
  await until(() => f.broker.counts().active === 0);
  let keyReads = 0, fields = 0;
  f.observation(value => new Proxy(value as Record<string, unknown>, {
    ownKeys(target) {
      keyReads++;
      return keyReads === 1 ? Reflect.ownKeys(target) : [...Reflect.ownKeys(target), ...Array.from({ length: 1000 }, (_, i) => `extra${i}`)];
    },
    getOwnPropertyDescriptor(target, key) {
      fields++;
      return Reflect.getOwnPropertyDescriptor(target, key) ?? { configurable: true, enumerable: true, writable: true, value: 0 };
    },
  }));
  await channel.close(); await until(() => f.broker.counts().bytes === 0);
  assert.equal(keyReads, 1);
  assert.equal(fields, 6);
});
