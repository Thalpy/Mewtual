import assert from "node:assert/strict";
import test from "node:test";
import { createHash, webcrypto } from "node:crypto";
import {
  VoiceCryptoBroker, createVoiceCryptoOwner, createVoiceJoinKey, deriveVoicePair,
  freezeVoicePacket, windowVoiceCryptoBroker,
} from "./voice-crypto.ts";
import type {
  VoiceConfirmationIdentity, VoiceConfirmationInput, VoiceConfirmationNative, VoiceConfirmationReceipt,
} from "./voice-crypto.ts";

// Native replies below are an explicit IPC fault seam, not evidence of a native action queue.
// Actual P-256/HKDF/AES-GCM computations use Node's real WebCrypto implementation.
const subtle = webcrypto.subtle as unknown as SubtleCrypto;
const text = (value: string) => new TextEncoder().encode(value);
const flush = () => new Promise<void>(resolve => setImmediate(resolve));
function signal<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
async function fixture() {
  let decrypts = 0, digests = 0, engineFailure = "", sequence = 0n;
  let engineGate: ReturnType<typeof signal<void>> | null = null;
  let engineEntered = signal<void>();
  let engineResult: Uint8Array | null = null;
  const engine = new Proxy(subtle, { get(target, name) {
    if (name === "decrypt") return async (...args: Parameters<SubtleCrypto["decrypt"]>) => {
      decrypts++; engineEntered.resolve();
      if (engineFailure === "decrypt") throw new Error("real operation refused");
      const result = await target.decrypt(...args); engineResult = new Uint8Array(result);
      if (engineGate) await engineGate.promise;
      return result;
    };
    if (name === "digest") return async (...args: Parameters<SubtleCrypto["digest"]>) => {
      digests++; if (engineFailure === "digest") throw new Error("digest refused");
      return target.digest(...args);
    };
    const value = Reflect.get(target, name); return typeof value === "function" ? value.bind(target) : value;
  } });
  const ordinary = {
    async admit() { return "ordinary"; }, async settle() {},
    async commitInbound() { throw new Error("confirmation must not use general inbound delivery"); },
  };
  const owner = createVoiceCryptoOwner("native-owner", "a".repeat(64), () => true);
  const window = {};
  const broker = windowVoiceCryptoBroker(ordinary, window, engine);
  const a = await createVoiceJoinKey(broker, owner), b = await createVoiceJoinKey(broker, owner);
  const transcript = text("native confirmation transcript");
  const left = await deriveVoicePair(broker, owner, a.privateKey, b.publicKey, transcript, true);
  const right = await deriveVoicePair(broker, owner, b.privateKey, a.publicKey, transcript, false);
  await flush();
  const receipts = new Map<string, object>(), admitted = new Set<string>(), settled = new Set<string>();
  let admitCalls = 0, commitCalls = 0, transitions = 0, settleCalls = 0, loseReply = false, reject = false;
  const native: VoiceConfirmationNative = {
    async admitAction(identity) {
      admitCalls++;
      if (settled.has(identity.action)) return { ...identity, kind: "rejected", receipt: "settled", code: "retired" };
      admitted.add(identity.action);
      return { ...identity, kind: "admitted", permit: "permit-" + identity.action };
    },
    async commitConfirmation(identity, packet, plaintext, resultTag) {
      commitCalls++;
      assert.equal(packet.sequence, BigInt(packet.sequence).toString());
      assert.equal(createHash("sha256").update(plaintext).digest("hex"), resultTag);
      let receipt = receipts.get(identity.action);
      if (!receipt) {
        transitions++;
        receipt = reject
          ? { ...identity, kind: "rejected", resultTag, receipt: "receipt-" + identity.action, code: "invalid" }
          : { ...identity, kind: "accepted", resultTag, receipt: "receipt-" + identity.action, confirmation: "continue" };
        receipts.set(identity.action, receipt);
      }
      if (loseReply) { loseReply = false; throw new Error("native committed, reply lost"); }
      return receipt as Awaited<ReturnType<VoiceConfirmationNative["commitConfirmation"]>>;
    },
    async settleAction(identity) { settleCalls++; settled.add(identity.action); admitted.delete(identity.action); },
  };
  async function input(action = "9007199254740993", aad = text("native exact AAD")): Promise<VoiceConfirmationInput> {
    sequence++;
    const ciphertext = await freezeVoicePacket(broker, owner, left,
      { token: "reserved-" + sequence, sequence, aad }, text("closed native confirmation")).encrypt();
    await flush();
    return { action, version: "9007199254740995", inputTag: "b".repeat(64), handshake: "c".repeat(64),
      deliveryToken: "9007199254740997", kind: "hello", sequence: sequence.toString(), aad, ciphertext };
  }
  return {
    broker, owner, left, right, native, input, window, ordinary, engine, a, b, transcript,
    register(input: VoiceConfirmationInput) { return broker.registerConfirmation(owner, right, native, input); },
    counts() { return { decrypts, digests, admitCalls, commitCalls, transitions, settleCalls, unsettled: admitted.size }; },
    lose() { loseReply = true; }, reject() { reject = true; },
    fail(kind: string) { engineFailure = kind; },
    holdEngine() { engineGate = signal<void>(); engineEntered = signal<void>(); return { entered: engineEntered.promise, release: () => engineGate!.resolve() }; },
    result() { return engineResult; },
  };
}

test("lost native success retries one computed confirmation and one native transition", async () => {
  const f = await fixture(), input = await f.input(), handle = f.register(input);
  f.lose();
  assert.equal((await handle.attempt()).kind, "uncertain");
  assert.equal(f.counts().decrypts, 1); assert.equal(f.counts().transitions, 1);
  assert.equal(f.counts().settleCalls, 0); assert.equal(f.broker.counts().active, 1);
  const charged = f.broker.counts().bytes;
  assert.ok(charged > input.aad.length + input.ciphertext.length);
  assert.equal(f.register({ ...input, aad: input.aad.slice(), ciphertext: input.ciphertext.slice() }), handle);
  const first = handle.attempt(), second = handle.attempt();
  assert.equal(first, second);
  const result = await first;
  assert.equal(result.kind, "accepted");
  assert.deepEqual(Object.keys(result), ["kind", "receipt"], "confirmation plaintext is never exported");
  assert.equal(f.counts().decrypts, 1); assert.equal(f.counts().commitCalls, 2); assert.equal(f.counts().transitions, 1);
  await flush(); assert.equal(f.broker.counts().active, 0);
  assert.equal(f.broker.counts().bytes, charged, "terminal receipt retains the same conservative charge");
  assert.ok(f.result()!.every(value => value === 0));
  f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("lost authoritative rejection is recovered without a second decrypt or transition", async () => {
  const f = await fixture(), handle = f.register(await f.input());
  f.reject(); f.lose();
  assert.equal((await handle.attempt()).kind, "uncertain");
  const outcome = await handle.attempt();
  assert.equal(outcome.kind, "rejected");
  assert.equal(f.counts().decrypts, 1); assert.equal(f.counts().transitions, 1);
  assert.deepEqual(await handle.attempt(), outcome);
  await flush(); f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("lost admission reply retries the same identity without starting the engine twice", async () => {
  const f = await fixture(), input = await f.input(), admit = f.native.admitAction;
  let lose = true;
  f.native.admitAction = async (...args) => {
    const response = await admit(...args);
    if (lose) { lose = false; throw new Error("reply lost after idempotent native admission"); }
    return response;
  };
  const handle = f.register(input);
  assert.equal((await handle.attempt()).kind, "uncertain");
  assert.equal(f.counts().decrypts, 0); assert.equal(f.counts().unsettled, 1);
  assert.equal(f.broker.counts().active, 1);
  assert.equal((await handle.attempt()).kind, "accepted");
  assert.equal(f.counts().admitCalls, 2); assert.equal(f.counts().decrypts, 1);
  assert.equal(f.counts().transitions, 1); await flush(); f.owner.retire();
  assert.equal(f.broker.counts().bytes, 0);
});

test("hanging commit coalesces observers and action retirement retains actual RPC custody", async () => {
  const f = await fixture(), input = await f.input(), gate = signal<void>(), entered = signal<void>();
  const commit = f.native.commitConfirmation;
  let hooks = 0, copy: Uint8Array | null = null;
  f.native.commitConfirmation = async (...args) => { hooks++; copy = args[2]; entered.resolve(); await gate.promise; return commit(...args); };
  const handle = f.register(input), first = handle.attempt();
  await entered.promise;
  for (let i = 0; i < 20; i++) assert.equal(handle.attempt(), first);
  assert.equal(hooks, 1);
  assert.equal(await Promise.race([first, Promise.resolve("caller stopped waiting")]), "caller stopped waiting");
  const charged = f.broker.counts().bytes;
  handle.retire(); assert.equal((await first).kind, "retired");
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, charged);
  assert.equal(f.counts().settleCalls, 0);
  gate.resolve(); await flush();
  assert.equal(f.broker.counts().active, 0); assert.equal(f.broker.counts().bytes, charged);
  assert.ok(copy!.every(value => value === 0));
  assert.equal((await handle.attempt()).kind, "retired"); assert.equal(hooks, 1);
  f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("retiring held admission wakes callers but settlement waits for the actual reply", async () => {
  const f = await fixture(), input = await f.input(), gate = signal<void>(), entered = signal<void>();
  const admit = f.native.admitAction;
  f.native.admitAction = async (...args) => { const reply = await admit(...args); entered.resolve(); await gate.promise; return reply; };
  const handle = f.register(input), attempt = handle.attempt(); await entered.promise;
  f.owner.retire(); assert.equal((await attempt).kind, "retired");
  assert.equal(f.broker.counts().active, 1); assert.ok(f.broker.counts().bytes > 0);
  assert.equal(f.counts().settleCalls, 0); assert.equal(f.counts().unsettled, 1);
  gate.resolve(); await flush();
  assert.equal(f.counts().decrypts, 0); assert.equal(f.counts().settleCalls, 1);
  assert.equal(f.broker.counts().bytes, 0);
  // This fake native adapter explicitly models the required admit-after-settle fence. Actual
  // native enforcement must receive its own tests when the action queue is implemented.
  const identity: VoiceConfirmationIdentity = { owner: f.owner.token, action: input.action, version: input.version, inputTag: input.inputTag };
  assert.equal((await f.native.admitAction(identity, 1)).kind, "rejected");
});

test("retirement during an actual engine promise clears the late result without early refund", { timeout: 30_000 }, async () => {
  const f = await fixture(), handle = f.register(await f.input()), engine = f.holdEngine();
  const settleEntered = signal<void>(), settleGate = signal<void>(), settled = signal<void>();
  const settle = f.native.settleAction;
  f.native.settleAction = async (...args) => {
    settleEntered.resolve(); await settleGate.promise;
    await settle(...args); settled.resolve();
  };
  const attempt = handle.attempt(); await engine.entered;
  const charged = f.broker.counts().bytes;
  f.owner.retire(); assert.equal((await attempt).kind, "retired");
  assert.equal(f.broker.counts().active, 1); assert.equal(f.counts().settleCalls, 0);
  assert.equal(f.broker.counts().bytes, charged);
  // engineEntered precedes real WebCrypto settlement. Fixed setImmediate turns cannot prove
  // that external work finished, especially under load. Await the actual native settlement
  // seam and also prove its pending RPC retains the original byte and active-work charge.
  engine.release(); await settleEntered.promise;
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, charged);
  assert.equal(f.counts().settleCalls, 0);
  settleGate.resolve(); await settled.promise; await flush();
  assert.equal(f.counts().settleCalls, 1); assert.equal(f.broker.counts().active, 0);
  assert.equal(f.counts().commitCalls, 0); assert.equal(f.broker.counts().bytes, 0);
  assert.ok(f.result()!.every(value => value === 0));
});

test("action-only retirement ends uncertainty without admitting or decrypting again", async () => {
  const f = await fixture(), input = await f.input(), handle = f.register(input);
  f.lose(); assert.equal((await handle.attempt()).kind, "uncertain");
  handle.retire(); await flush();
  assert.equal(f.owner.isCurrent(), true); assert.equal(f.broker.counts().active, 0);
  assert.ok(f.broker.counts().bytes > 0, "retired identity retains the combined slot/receipt budget");
  assert.equal(f.register(input), handle);
  assert.equal((await handle.attempt()).kind, "retired"); assert.equal(f.counts().decrypts, 1);
  assert.throws(() => f.register({ ...input, version: "9007199254740996" }), /identity changed/);
  f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

for (const operation of ["decrypt", "digest"]) test("genuine " + operation + " failure remains failed under retry", async () => {
  const f = await fixture(), handle = f.register(await f.input()); f.fail(operation);
  assert.equal((await handle.attempt()).kind, "failed");
  assert.equal((await handle.attempt()).kind, "failed");
  assert.equal(f.counts().decrypts, 1); assert.equal(f.counts().admitCalls, 1);
  assert.equal(f.counts().commitCalls, 0);
  await flush(); f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("full combined terminal capacity refuses before another engine or native transition", async () => {
  const f = await fixture();
  let last!: ReturnType<typeof f.register>;
  for (let i = 1; i <= 32; i++) {
    last = f.register(await f.input(String(i)));
    assert.equal((await last.attempt()).kind, "accepted");
  }
  await flush();
  const next = await f.input("33"), before = f.counts();
  assert.throws(() => f.register(next), /capacity/);
  assert.deepEqual(f.counts(), before);
  assert.equal((await last.attempt()).kind, "accepted", "full cache cannot evict a lost receipt");
  f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("large native inputs share the ordinary broker group byte ceiling before admission", async () => {
  const f = await fixture(), input = await f.input("1", new Uint8Array(4096));
  const held = signal<void>();
  const ordinary = f.broker.run(f.owner, "derive", 100_000, () => held.promise);
  await Promise.resolve(); await Promise.resolve();
  let registered = 0;
  for (let i = 1; i <= 32; i++) {
    try { f.register({ ...input, action: String(i) }); registered++; }
    catch (error) { assert.match(String(error), /capacity/); break; }
  }
  assert.ok(registered > 0 && registered < 32, "bytes constrain the combined registry before its row cap");
  assert.ok(f.broker.counts().bytes <= 256 * 1024); assert.equal(f.counts().admitCalls, 0);
  f.owner.retire(); assert.equal(f.broker.counts().bytes, 100_000, "unstarted records free, actual old engine remains");
  held.resolve(); await assert.rejects(ordinary); await flush();
  assert.equal(f.broker.counts().bytes, 0);
});

test("owner retirement removes queued records while held actual admissions keep two slots", async () => {
  const f = await fixture(), inputs = [];
  for (let i = 1; i <= 6; i++) inputs.push(await f.input(String(i)));
  const gate = signal<void>(), entered = signal<void>(), admit = f.native.admitAction;
  let calls = 0;
  f.native.admitAction = async (...args) => {
    const reply = await admit(...args); calls++; if (calls === 2) entered.resolve();
    await gate.promise; return reply;
  };
  const handles = inputs.map(input => f.register(input)), attempts = handles.map(handle => handle.attempt());
  await entered.promise;
  assert.equal(f.broker.counts().active, 2); assert.equal(f.broker.counts().queued, 4);
  f.owner.retire();
  assert.ok((await Promise.all(attempts)).every(outcome => outcome.kind === "retired"));
  assert.equal(f.broker.counts().active, 2); assert.equal(f.broker.counts().queued, 0);
  assert.ok(f.broker.counts().bytes > 0); assert.equal(f.counts().settleCalls, 0);
  gate.resolve(); await flush();
  assert.equal(calls, 2); assert.equal(f.counts().decrypts, 0);
  assert.equal(f.broker.counts().active, 0); assert.equal(f.broker.counts().bytes, 0);
});

test("temporary ordinary queue saturation leaves an untouched confirmation retryable", async () => {
  const f = await fixture(), handle = f.register(await f.input());
  const gate = signal<void>();
  const ordinary = Array.from({ length: 34 }, () => f.broker.run(f.owner, "derive", 1, () => gate.promise));
  await Promise.resolve(); await Promise.resolve();
  assert.equal(f.broker.counts().active, 2); assert.equal(f.broker.counts().queued, 32);
  assert.equal((await handle.attempt()).kind, "busy");
  assert.equal(f.counts().admitCalls, 0); assert.equal(f.counts().decrypts, 0);
  gate.resolve(); await Promise.all(ordinary); await flush();
  assert.equal((await handle.attempt()).kind, "accepted");
  assert.equal(f.counts().admitCalls, 1); assert.equal(f.counts().decrypts, 1);
  await flush(); f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("registration rejects undeclared mutable data before retaining bytes or starting work", async () => {
  const f = await fixture(), input = await f.input();
  const before = f.broker.counts();
  const extra = { ...input, unknown: new Uint8Array(1024 * 1024) };
  assert.throws(() => f.register(extra), /invalid native confirmation input/);
  const symbolic = { ...input, [Symbol("unknown")]: new Uint8Array(1024 * 1024) };
  assert.throws(() => f.register(symbolic), /invalid native confirmation input/);
  assert.deepEqual(f.broker.counts(), before); assert.equal(f.counts().admitCalls, 0);
  const handle = f.register(input); assert.equal((await handle.attempt()).kind, "accepted");
  await flush(); f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("retirement after a busy action retries preserves the newly running operation charge", async () => {
  const f = await fixture(), handle = f.register(await f.input()), ordinaryGate = signal<void>();
  const ordinary = Array.from({ length: 34 }, () => f.broker.run(f.owner, "derive", 1, () => ordinaryGate.promise));
  await Promise.resolve(); await Promise.resolve();
  assert.equal((await handle.attempt()).kind, "busy");
  ordinaryGate.resolve(); await Promise.all(ordinary); await flush();
  const entered = signal<void>(), gate = signal<void>(), commit = f.native.commitConfirmation;
  f.native.commitConfirmation = async (...args) => { entered.resolve(); await gate.promise; return commit(...args); };
  const attempt = handle.attempt(); await entered.promise;
  f.owner.retire(); assert.equal((await attempt).kind, "retired");
  assert.equal(f.broker.counts().active, 1); assert.ok(f.broker.counts().bytes > 0);
  assert.equal(f.counts().settleCalls, 0);
  gate.resolve(); await flush();
  assert.equal(f.broker.counts().bytes, 0); assert.equal(f.counts().decrypts, 1);
});

test("busy retry reports queued when one queue slot reopens behind two held workers", async () => {
  const f = await fixture(), handle = f.register(await f.input());
  const gates = Array.from({ length: 34 }, () => signal<void>());
  const ordinary = gates.map(gate => f.broker.run(f.owner, "derive", 1, () => gate.promise));
  await Promise.resolve(); await Promise.resolve();
  assert.equal((await handle.attempt()).kind, "busy");
  gates[0].resolve(); await ordinary[0]; await flush();
  assert.equal(f.broker.counts().active, 2); assert.equal(f.broker.counts().queued, 31);
  const retry = handle.attempt();
  assert.equal(handle.inspect(), "queued"); assert.equal(f.counts().admitCalls, 0);
  gates.forEach(gate => gate.resolve()); await Promise.all(ordinary);
  assert.equal((await retry).kind, "accepted");
  await flush(); f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("changed full input, owner, pair or unsafe numeric identity refuses before admission", async () => {
  const f = await fixture(), input = await f.input();
  f.register(input);
  const changed = input.ciphertext.slice(); changed[0] ^= 1;
  for (const patch of [
    { aad: text("changed") }, { ciphertext: changed }, { sequence: "2" }, { version: "2" },
    { deliveryToken: "2" }, { handshake: "d".repeat(64) }, { inputTag: "d".repeat(64) }, { kind: "proof" as const },
  ]) assert.throws(() => f.register({ ...input, ...patch }), /identity changed/);
  for (const action of ["01", "0", "18446744073709551616", 9007199254740992 as unknown as string]) {
    assert.throws(() => f.register({ ...input, action }), /invalid/);
  }
  const copiedOwner = createVoiceCryptoOwner(f.owner.token, f.owner.group, () => true);
  const copiedPair = await deriveVoicePair(f.broker, copiedOwner, f.b.privateKey, f.a.publicKey, f.transcript, false);
  assert.throws(() => f.broker.registerConfirmation(copiedOwner, copiedPair, f.native, input), /identity changed/);
  assert.equal(f.counts().admitCalls, 0); assert.equal(f.counts().decrypts, 0);
  f.owner.retire(); copiedOwner.retire(); await flush(); assert.equal(f.broker.counts().bytes, 0);
});

for (const fault of ["owner", "action", "resultTag", "confirmation", "extra"]) test("malformed " + fault + " receipt stays uncertain until an exact retry", async () => {
  const f = await fixture(), handle = f.register(await f.input()), commit = f.native.commitConfirmation;
  let corrupt = true;
  f.native.commitConfirmation = async (...args) => {
    const receipt = await commit(...args) as VoiceConfirmationReceipt;
    if (!corrupt) return receipt;
    if (fault === "extra") return { ...receipt, unknown: true };
    return { ...receipt, [fault]: fault === "resultTag" ? "d".repeat(64) : "wrong" };
  };
  assert.equal((await handle.attempt()).kind, "uncertain");
  assert.equal(f.counts().settleCalls, 0); assert.equal(f.broker.counts().active, 1);
  corrupt = false;
  assert.equal((await handle.attempt()).kind, "accepted");
  assert.equal(f.counts().decrypts, 1); assert.equal(f.counts().transitions, 1);
  await flush(); f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("retained HMR broker shares the exact action record only with its original native adapter", async () => {
  const f = await fixture(), input = await f.input(), handle = f.register(input);
  f.lose(); assert.equal((await handle.attempt()).kind, "uncertain");
  const url = new URL("./voice-crypto.ts", import.meta.url); url.search = "confirmation-hmr";
  const fresh = await import(url.href) as typeof import("./voice-crypto.ts");
  const retained = fresh.windowVoiceCryptoBroker(f.ordinary, f.window, f.engine);
  const replacement: VoiceConfirmationNative = {
    async admitAction() { throw new Error("mount callback must not replace the original adapter"); },
    async commitConfirmation() { throw new Error("wrong adapter"); }, async settleAction() {},
  };
  assert.equal(retained, f.broker);
  // v10 binds adapters per original row. A remount must reuse that retained adapter rather
  // than offer new callbacks that could target another channel; it cannot replace or silently
  // adopt the first channel's adapter. The original uncertain action still recovers unchanged.
  assert.throws(() => retained.registerConfirmation(f.owner, f.right, replacement, input), /identity changed/);
  const pulled = retained.registerConfirmation(f.owner, f.right, f.native, input);
  assert.equal(pulled, handle); assert.equal((await pulled.attempt()).kind, "accepted");
  assert.equal(f.counts().decrypts, 1); await flush(); f.owner.retire();
});

test("fresh HMR owner and newly derived pairs use the retained broker's computation seam", async () => {
  const f = await fixture();
  f.owner.retire();
  const url = new URL("./voice-crypto.ts", import.meta.url); url.search = "fresh-confirmation-hmr";
  const fresh = await import(url.href) as typeof import("./voice-crypto.ts");
  const broker = fresh.windowVoiceCryptoBroker(f.ordinary, f.window, f.engine);
  const owner = fresh.createVoiceCryptoOwner("fresh-native-owner", "a".repeat(64), () => true);
  const a = await fresh.createVoiceJoinKey(broker, owner), b = await fresh.createVoiceJoinKey(broker, owner);
  const left = await fresh.deriveVoicePair(broker, owner, a.privateKey, b.publicKey, f.transcript, true);
  const right = await fresh.deriveVoicePair(broker, owner, b.privateKey, a.publicKey, f.transcript, false);
  const aad = text("new native join after HMR");
  const ciphertext = await fresh.freezeVoicePacket(broker, owner, left,
    { token: "fresh-reservation", sequence: 1n, aad }, text("closed confirmation")).encrypt();
  const input: VoiceConfirmationInput = { action: "1", version: "1", inputTag: "e".repeat(64),
    handshake: "f".repeat(64), deliveryToken: "1", kind: "hello", sequence: "1", aad, ciphertext };
  const handle = broker.registerConfirmation(owner, right, f.native, input);
  assert.equal((await handle.attempt()).kind, "accepted");
  assert.equal(f.counts().decrypts, 1);
  await flush(); owner.retire(); assert.equal(broker.counts().bytes, 0);
});

test("two occupied broker slots finish native acknowledgements before follow-up crypto", async () => {
  const f = await fixture(), inputs = [await f.input("1"), await f.input("2"), await f.input("3")];
  const gates = [signal<void>(), signal<void>()], entered = signal<void>(), followups: string[] = [];
  const commit = f.native.commitConfirmation;
  let hooks = 0;
  f.native.commitConfirmation = async (...args) => {
    const index = hooks++;
    followups.push(args[0].action); // Native enqueues; it never awaits those later engine actions.
    if (hooks === 2) entered.resolve();
    if (index < 2) await gates[index].promise;
    return commit(...args);
  };
  const handles = inputs.map(input => f.register(input)), attempts = handles.map(handle => handle.attempt());
  await entered.promise;
  assert.equal(f.broker.counts().active, 2); assert.equal(f.broker.counts().queued, 1);
  assert.deepEqual(new Set(followups), new Set(["1", "2"]), "independent engine completions may reorder");
  gates.forEach(gate => gate.resolve());
  assert.ok((await Promise.all(attempts)).every(outcome => outcome.kind === "accepted"));
  await flush(); assert.equal(f.broker.counts().active, 0);
  assert.deepEqual([...followups].sort(), ["1", "2", "3"]);
  f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});
