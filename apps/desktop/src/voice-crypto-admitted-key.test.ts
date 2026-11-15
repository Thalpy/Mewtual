import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { webcrypto } from "node:crypto";
import { VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker } from "./voice-crypto.ts";
import type { VoiceAdmittedSelectedKeyHandle, VoiceJoinKeyHandle } from "./voice-crypto.ts";
import { isVoiceAdmittedAdapterRegistry, voiceAdmittedJoinKeyNative } from "./voice-admitted-join-native.ts";
import { voiceJoinKeyNative } from "./voice-join-native.ts";
import { VOICE_ADMITTED_SELECTED_KEY_BYTES, voiceAdmittedKeyInventory } from "./voice-admitted-key-inventory.ts";

const real = webcrypto.subtle as unknown as SubtleCrypto;
const hash = (n: number) => n.toString(16).padStart(64, "0");
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const deadline = performance.now() + 15_000;
  while (!check()) { if (performance.now() >= deadline) throw new Error("admitted key did not settle"); await new Promise<void>(yes => setImmediate(yes)); }
}
function fixture(t: TestContext) {
  let generates = 0, exports = 0, admissions = 0, publications = 0, acknowledgements = 0, settlements = 0;
  let failAdmit = 0, failPublish = 0, failAck = 0, failSettle = 0, engineFails = false;
  let terminal: "accepted" | "rejected" | "closed" | "not-started" = "accepted";
  let sdk: ReturnType<typeof gate> | null = null, publish: ReturnType<typeof gate> | null = null;
  let ack: ReturnType<typeof gate> | null = null, settle: ReturnType<typeof gate> | null = null;
  let ackReturned = false, executor: string | null = null, point: string | null = null, privateKey: CryptoKey | null = null;
  let mutateReply: ((reply: unknown) => unknown) | null = null, mutateAdmission: ((reply: unknown) => unknown) | null = null;
  let currentHook: (() => void) | null = null;
  const engine = new Proxy(real, { get(target, name) {
    const value = Reflect.get(target, name);
    if (typeof value !== "function") return value;
    return async (...args: unknown[]) => {
      if (name === "generateKey") { generates++; if (sdk) await sdk.promise; if (engineFails) throw new Error("SDK failed"); }
      if (name === "exportKey") exports++;
      const result = await value.apply(target, args);
      if (name === "generateKey") privateKey = (result as CryptoKeyPair).privateKey;
      return result;
    };
  } });
  const ordinary = { async admit() { throw new Error("ordinary admission forbidden"); }, async settle() {}, async commitInbound() {} };
  const owner = createVoiceCryptoOwner("admitted-owner", hash(1), () => { currentHook?.(); return true; });
  const broker = new VoiceCryptoBroker(ordinary, engine);
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const native = voiceAdmittedJoinKeyNative(async <T>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    const identity = { owner: args.owner, executor: args.executor ?? args.executorId, permit: "native-permit", join: hash(2) };
    if (command === "admit_admitted_voice_join_key") {
      admissions++; executor ??= String(args.executor); assert.equal(args.executor, executor);
      if (failAdmit-- > 0) throw new Error("admission reply lost");
      const reply = { ...identity, kind: "admitted" };
      return (mutateAdmission ? mutateAdmission(reply) : reply) as T;
    }
    if (command === "publish_admitted_voice_join_key") {
      publications++; assert.equal(args.executor, executor);
      assert.equal(typeof args.publicKey, "string"); assert.equal(String(args.publicKey).length, 130);
      if (point === null) point = String(args.publicKey); else assert.equal(args.publicKey, point);
      if (publish) await publish.promise;
      if (failPublish-- > 0) throw new Error("unknown publication reply");
      const reply = terminal === "accepted" ? { ...identity, kind: terminal, keyRef: hash(3) } :
        { ...identity, kind: terminal, code: terminal === "rejected" ? "refused" : terminal === "closed" ? "unconfirmed" : "ended" };
      return (mutateReply ? mutateReply(reply) : reply) as T;
    }
    if (command === "acknowledge_admitted_voice_key_publication") {
      acknowledgements++; assert.equal(settlements, 0, "ACK precedes SDK settle to preserve lost-ACK recovery");
      if (ack) await ack.promise;
      if (failAck-- > 0) throw new Error("ACK reply lost");
      ackReturned = true; return undefined as T;
    }
    assert.equal(command, "settle_admitted_voice_join_key"); settlements++;
    if (publications && terminal !== "not-started") assert.equal(ackReturned, true);
    if (settle) await settle.promise;
    if (failSettle-- > 0) throw new Error("settlement reply lost");
    return undefined as T;
  });
  const selected: VoiceAdmittedSelectedKeyHandle[] = [];
  t.after(async () => {
    failAdmit = failPublish = failAck = failSettle = 0;
    mutateReply = mutateAdmission = null; currentHook = null;
    sdk?.release(); publish?.release(); ack?.release(); settle?.release();
    owner.retire(); broker.retrySelectedJoinKeys();
    for (const handle of selected) void handle.attempt();
    await until(() => broker.counts().active === 0);
  });
  return { broker, owner, native, ordinary, engine, calls,
    select() { const handle = broker.selectAdmittedJoinKey(owner, native); selected.push(handle); return handle; },
    counts: () => ({ generates, exports, admissions, publications, acknowledgements, settlements }),
    privateKey: () => privateKey,
    lose(phase: "admit" | "publish" | "ack" | "settle", count = 1) {
      if (phase === "admit") failAdmit = count; else if (phase === "publish") failPublish = count; else if (phase === "ack") failAck = count; else failSettle = count;
    },
    terminal(kind: typeof terminal) { terminal = kind; }, failEngine() { engineFails = true; },
    reply(mutator: ((reply: unknown) => unknown) | null) { mutateReply = mutator; },
    admission(mutator: ((reply: unknown) => unknown) | null) { mutateAdmission = mutator; },
    onCurrent(hook: () => void) { currentHook = hook; },
    hold(phase: "engine" | "publish" | "ack" | "settle") {
      const held = gate(); if (phase === "engine") sdk = held; else if (phase === "publish") publish = held; else if (phase === "ack") ack = held; else settle = held;
      return held;
    },
  };
}

test("admitted selection retains one original nonextractable key in its separate profile map", async t => {
  const f = fixture(t), selected = f.select(); assert.equal(f.select(), selected);
  const result = await selected.attempt(); assert.equal(result.kind, "accepted");
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().generates, 1); assert.equal(f.counts().acknowledgements, 1); assert.equal(f.counts().settlements, 1);
  assert.equal(f.privateKey()?.extractable, false);
  if (result.kind !== "accepted") return;
  assert.deepEqual(Object.keys(result.key), ["publicPoint"]);
  const point = result.key.publicPoint(); point.fill(0); assert.equal(result.key.publicPoint()[0], 4);
  const maps = f.broker as unknown as { joinKeys: WeakMap<object, unknown>; admittedKeys: WeakMap<object, unknown> };
  assert.equal(maps.joinKeys.has(result.key), false); assert.equal(maps.admittedKeys.has(result.key), true);
  assert.throws(() => f.broker.bindJoinKey(f.owner, result.key as unknown as VoiceJoinKeyHandle, hash(2), hash(3), result.key.publicPoint()));
  assert.throws(() => f.broker.nativeChannel(f.owner, result.key as unknown as VoiceJoinKeyHandle, {} as never), /selected voice key/);
  f.owner.retire(); assert.throws(() => result.key.publicPoint(), /retired/);
  assert.equal((await selected.attempt()).kind, "retired"); await f.broker.retireNativeOwner(f.owner);
  assert.equal(f.broker.counts().bytes, 0);
});

test("factory profile/owner/adapter substitutions refuse before payment or SDK entry", async t => {
  const f = fixture(t);
  assert.throws(() => f.broker.selectJoinKey(f.owner, f.native as never), /admitted/);
  assert.throws(() => f.broker.nativeAppCalls({} as never, f.native as never, {} as never), /admitted/);
  assert.throws(() => f.broker.selectAdmittedJoinKey(f.owner, voiceJoinKeyNative(async <T>() => undefined as T) as never), /original admitted/);
  assert.equal(f.broker.counts().bytes, 0);
  f.select(); const before = f.broker.counts();
  assert.throws(() => f.broker.selectAdmittedJoinKey(f.owner, { ...f.native }), /original admitted/);
  const replacement = createVoiceCryptoOwner(f.owner.token, f.owner.group, () => true);
  assert.throws(() => f.broker.selectAdmittedJoinKey(replacement, f.native), /selection changed/);
  assert.throws(() => f.broker.selectJoinKey(f.owner, voiceJoinKeyNative(async <T>() => undefined as T)), /selection changed/);
  assert.deepEqual(f.broker.counts(), before); assert.equal(f.counts().generates, 0); replacement.retire();
});

for (const phase of ["admit", "publish", "ack", "settle"] as const) test(`lost admitted ${phase} reply retains the same SDK key and cleanup owner`, async t => {
  const f = fixture(t); f.lose(phase); const selected = f.select();
  const first = await selected.attempt();
  if (phase === "admit" || phase === "publish") assert.equal(first.kind, "uncertain");
  else assert.equal(first.kind, "accepted");
  assert.equal(f.broker.counts().active, 1);
  // Known accepted key observation precedes independent ACK/SDK cleanup. Wait
  // for the requested failing phase before providing its availability wake.
  if (phase === "ack" || phase === "settle") {
    await until(() => phase === "ack" ? f.counts().acknowledgements === 1 : f.counts().settlements === 1);
    await new Promise<void>(yes => setImmediate(yes));
  }
  const retry = await selected.attempt(); assert.equal(retry.kind, "accepted");
  if (first.kind === "accepted") assert.equal(retry, first, "known result never becomes a different observation during ACK cleanup");
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().generates, 1); assert.equal(f.counts().exports, 1);
  assert.equal(f.counts().acknowledgements, phase === "ack" ? 2 : 1);
});

for (const phase of ["engine", "publish", "ack", "settle"] as const) test(`End during admitted ${phase} joins actual custody before refund`, async t => {
  const f = fixture(t), held = f.hold(phase), selected = f.select(), attempt = selected.attempt();
  await until(() => phase === "engine" ? f.counts().generates === 1 : phase === "publish" ? f.counts().publications === 1 :
    phase === "ack" ? f.counts().acknowledgements === 1 : f.counts().settlements === 1);
  const before = f.broker.counts(); const closed = f.broker.retireNativeOwner(f.owner);
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, before.bytes);
  held.release(); await closed;
  assert.equal((await selected.attempt()).kind, "retired");
  const observed = await attempt;
  if (observed.kind === "accepted") assert.throws(() => observed.key.publicPoint(), /retired/);
  assert.equal(f.broker.counts().bytes, 0); assert.equal(f.counts().generates, 1);
});

for (const kind of ["rejected", "closed", "not-started"] as const) test(`exact ${kind} history after End is cleanup, never another SDK generation`, async t => {
  const f = fixture(t), held = f.hold("publish"); f.terminal(kind);
  const selected = f.select(), attempt = selected.attempt(); await until(() => f.counts().publications === 1);
  const closed = f.broker.retireNativeOwner(f.owner); held.release(); await closed;
  assert.equal((await attempt).kind, "retired"); assert.equal(f.counts().generates, 1);
  assert.equal(f.counts().acknowledgements, kind === "not-started" ? 0 : 1);
  assert.equal(f.counts().settlements, 1); assert.equal(f.broker.counts().bytes, 0);
});

test("failed ACK keeps the original known result and one bounded availability retry", async t => {
  const f = fixture(t); f.lose("ack", 3); const selected = f.select();
  const result = await selected.attempt(); assert.equal(result.kind, "accepted");
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(f.counts().acknowledgements, 1);
  f.owner.retire(); await until(() => f.counts().acknowledgements === 2);
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(f.counts().acknowledgements, 2);
  assert.equal(f.counts().settlements, 0); assert.ok(f.broker.counts().bytes > 0);
  f.broker.retrySelectedJoinKeys(); await until(() => f.counts().acknowledgements === 3);
  f.broker.retrySelectedJoinKeys(); await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().generates, 1); assert.equal(f.counts().settlements, 1);
});

test("original admitted flight and adapter survive broker module remount", async t => {
  const f = fixture(t), host = {}, held = f.hold("publish");
  const broker = windowVoiceCryptoBroker(f.ordinary, host, f.engine);
  const selected = broker.selectAdmittedJoinKey(f.owner, f.native), attempt = selected.attempt();
  t.after(async () => { held.release(); f.owner.retire(); broker.retrySelectedJoinKeys(); await until(() => broker.counts().active === 0); });
  await until(() => f.counts().publications === 1);
  const fresh = await import(`./voice-crypto.ts?admitted-remount=${Date.now()}`);
  assert.equal(fresh.windowVoiceCryptoBroker(f.ordinary, host, f.engine), broker);
  assert.equal(broker.selectAdmittedJoinKey(f.owner, f.native), selected);
  held.release(); assert.equal((await attempt).kind, "accepted"); await until(() => broker.counts().active === 0);
  assert.equal(f.counts().generates, 1);
});

test("admitted logical graph prepays its complete separate fee before SDK work", async t => {
  const inventory = voiceAdmittedKeyInventory(); assert.ok(inventory.total <= VOICE_ADMITTED_SELECTED_KEY_BYTES);
  assert.ok(inventory.pointAndEncoding >= 2 * 65 + 130 + 260);
  const f = fixture(t), held = f.hold("engine"), selected = f.select();
  assert.equal(f.broker.counts().bytes, VOICE_ADMITTED_SELECTED_KEY_BYTES);
  void selected.attempt(); await until(() => f.counts().generates === 1);
  assert.equal(f.broker.counts().bytes, VOICE_ADMITTED_SELECTED_KEY_BYTES + 128);
  f.owner.retire(); assert.equal(f.broker.counts().bytes, VOICE_ADMITTED_SELECTED_KEY_BYTES + 128);
  held.release(); await until(() => f.broker.counts().active === 0); assert.equal(f.broker.counts().bytes, 0);
});

test("admitted metadata uses the same aggregate group ledger and releases one exact slot", async () => {
  const ownerList = Array.from({ length: 65 }, (_, n) => createVoiceCryptoOwner("owner-" + n, hash(1), () => true));
  const broker = new VoiceCryptoBroker({ async admit() { throw new Error("unused"); }, async settle() {}, async commitInbound() {} }, real);
  const native = voiceAdmittedJoinKeyNative(async <T>() => { throw new Error("no invocation before attempt") as T; });
  try {
    for (const owner of ownerList.slice(0, 64)) broker.selectAdmittedJoinKey(owner, native);
    assert.equal(broker.counts().bytes, 512 * 1024);
    assert.throws(() => broker.selectAdmittedJoinKey(ownerList[64], native), /capacity/);
    await broker.retireNativeOwner(ownerList[0]); assert.equal(broker.counts().bytes, 512 * 1024 - VOICE_ADMITTED_SELECTED_KEY_BYTES);
    broker.selectAdmittedJoinKey(ownerList[64], native); assert.equal(broker.counts().bytes, 512 * 1024);
  } finally { await Promise.all(ownerList.map(owner => broker.retireNativeOwner(owner))); }
  assert.equal(broker.counts().bytes, 0);
});

for (const malformed of ["unknown-kind", "wrong-owner", "unknown-code", "accessor"] as const) test(`malformed ${malformed} publication cannot ACK or settle unknown work`, async t => {
  const f = fixture(t);
  f.reply(reply => {
    const data = { ...(reply as Record<string, unknown>) };
    if (malformed === "unknown-kind") data.kind = "disappeared";
    else if (malformed === "wrong-owner") data.owner = "replacement";
    else if (malformed === "unknown-code") { delete data.keyRef; data.kind = "closed"; data.code = "timeout-means-collected"; }
    else Object.defineProperty(data, "permit", { get() { throw new Error("not native JSON"); }, enumerable: true });
    return data;
  });
  const selected = f.select(); assert.equal((await selected.attempt()).kind, "uncertain");
  assert.equal(f.counts().acknowledgements, 0); assert.equal(f.counts().settlements, 0);
  f.owner.retire(); await until(() => f.counts().publications === 2);
  assert.equal(f.counts().acknowledgements, 0); assert.equal(f.counts().settlements, 0);
  f.reply(null); f.broker.retrySelectedJoinKeys(); await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().generates, 1);
});

test("reentrant publicPoint retirement detaches actual entered cells before metadata refund", async t => {
  const f = fixture(t), selected = f.select(), result = await selected.attempt();
  await until(() => f.broker.counts().active === 0); assert.equal(result.kind, "accepted");
  if (result.kind !== "accepted") return;
  const paid = f.broker.counts().bytes; let close: Promise<void> | null = null;
  f.onCurrent(() => {
    close ??= f.broker.retireNativeOwner(f.owner);
    assert.equal(f.broker.counts().bytes, paid, "entered public-point dispatch remains prepaid");
  });
  assert.throws(() => result.key.publicPoint(), { message: "voice crypto owner changed" });
  assert.notEqual(close, null); await close;
  assert.throws(() => result.key.publicPoint(), /retired/);
  assert.equal(f.broker.counts().bytes, 0); assert.equal((await selected.attempt()).kind, "retired");
});

test("fixed admitted adapter sends only bounded public metadata and exact command names", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const native = voiceAdmittedJoinKeyNative(async <T>(command: string, args: Record<string, unknown>) => { calls.push({ command, args }); return undefined as T; });
  const point = new Uint8Array(65); point[0] = 4;
  await native.admit("owner", hash(1)); await native.publish("owner", hash(1), "permit", point);
  await native.acknowledge("owner", hash(1)); await native.settle("owner", hash(1));
  assert.deepEqual(calls.map(call => call.command), ["admit_admitted_voice_join_key", "publish_admitted_voice_join_key", "acknowledge_admitted_voice_key_publication", "settle_admitted_voice_join_key"]);
  assert.equal(calls[1].args.publicKey, "04" + "00".repeat(64));
  assert.throws(() => native.publish("owner", hash(1), "permit", new Uint8Array(66)), /point/);
  assert.throws(() => native.admit("owner", "different-executor"), /identity/);
  assert.equal(calls.length, 4);
});

test("admitted SDK failure is terminal without manufacturing publication ACK", async t => {
  const f = fixture(t); f.failEngine(); const selected = f.select();
  assert.equal((await selected.attempt()).kind, "failed");
  await until(() => f.broker.counts().active === 0);
  assert.equal((await selected.attempt()).kind, "failed");
  assert.equal(f.counts().generates, 1); assert.equal(f.counts().publications, 0);
  assert.equal(f.counts().acknowledgements, 0); assert.equal(f.counts().settlements, 1);
});

test("admitted selections share the original two provider positions and bounded32-job queue", async t => {
  const held = gate(); let generates = 0;
  const engine = new Proxy(real, { get(target, name) {
    const value = Reflect.get(target, name);
    if (typeof value !== "function") return value;
    return async (...args: unknown[]) => { if (name === "generateKey") { generates++; await held.promise; } return value.apply(target, args); };
  } });
  const broker = new VoiceCryptoBroker({ async admit() { throw new Error("ordinary admission forbidden"); }, async settle() {}, async commitInbound() {} }, engine);
  const native = voiceAdmittedJoinKeyNative(async <T>(command: string, args: Record<string, unknown>) => {
    const identity = { owner: args.owner, executor: args.executor ?? args.executorId, permit: "permit", join: hash(2) };
    if (command === "admit_admitted_voice_join_key") return { ...identity, kind: "admitted" } as T;
    if (command === "publish_admitted_voice_join_key") return { ...identity, kind: "accepted", keyRef: hash(3) } as T;
    return undefined as T;
  });
  const owners = Array.from({ length: 35 }, (_, n) => createVoiceCryptoOwner("queued-owner-" + n, hash(1), () => true));
  t.after(async () => { held.release(); for (const owner of owners) owner.retire(); broker.retrySelectedJoinKeys(); await until(() => broker.counts().active === 0); });
  const selected = owners.map(owner => broker.selectAdmittedJoinKey(owner, native));
  const attempts = selected.map(handle => handle.attempt());
  await until(() => generates === 2);
  assert.equal(broker.counts().active, 2); assert.equal(broker.counts().queued, 32);
  assert.equal((await attempts[34]).kind, "busy");
  held.release();
  const results = await Promise.all(attempts.slice(0, 34)); assert.ok(results.every(result => result.kind === "accepted"));
  await until(() => broker.counts().active === 0);
  assert.equal((await selected[34].attempt()).kind, "accepted");
  await until(() => broker.counts().active === 0); assert.equal(generates, 35);
});

for (const field of ["kind", "permit"] as const) test(`admitted ${field} accessor remains unknown and recovers the original generation`, async t => {
  const f = fixture(t); let reads = 0;
  f.admission(reply => Object.defineProperty({ ...(reply as object) }, field, {
    enumerable: true, get() { reads++; return field === "kind" ? "closed" : reads === 1 ? "native-permit" : "changed-permit"; },
  }));
  const selected = f.select(); assert.equal((await selected.attempt()).kind, "uncertain");
  assert.equal(reads, 0); assert.equal(f.counts().generates, 0); assert.equal(f.counts().settlements, 0);
  f.admission(null); assert.equal((await selected.attempt()).kind, "accepted");
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().generates, 1); assert.equal(f.counts().admissions, 2); assert.equal(f.counts().settlements, 1);
});

test("End after malformed closed admission still settles the possibly admitted original", async t => {
  const f = fixture(t);
  f.admission(reply => Object.defineProperty({ ...(reply as object) }, "kind", { enumerable: true, get() { return "closed"; } }));
  const selected = f.select(); assert.equal((await selected.attempt()).kind, "uncertain");
  await f.broker.retireNativeOwner(f.owner);
  assert.equal(f.counts().generates, 0); assert.equal(f.counts().acknowledgements, 0); assert.equal(f.counts().settlements, 1);
  assert.equal(f.broker.counts().bytes, 0);
});

test("fixed admitted encoder ignores shadowed length and verifies actual typed-array width", async () => {
  let calls = 0, reads = 0;
  const native = voiceAdmittedJoinKeyNative(async <T>(_command: string, args: Record<string, unknown>) => {
    calls++; assert.equal(String(args.publicKey).length, 130); return undefined as T;
  });
  class Point extends Uint8Array {}
  const point = new Point(65); point[0] = 4;
  Object.defineProperty(point, "length", { get() { reads++; return Number.MAX_SAFE_INTEGER; } });
  Object.defineProperty(point, "byteLength", { get() { reads++; return 0; } });
  await native.publish("owner", hash(1), "permit", point);
  assert.equal(reads, 0); assert.equal(calls, 1);
  const oversized = new Point(66); oversized[0] = 4;
  Object.defineProperty(oversized, "byteLength", { value: 65 });
  assert.throws(() => native.publish("owner", hash(1), "permit", oversized), /point/);
  assert.throws(() => native.publish("owner", hash(1), "permit", new Proxy(point, {})), /point/);
  assert.equal(calls, 1);
});

test("retained broker recognizes a real factory from another adapter module instance without adopting an old selection", async t => {
  const f = fixture(t); f.select(); // old original owner is bound to its old actual adapter
  const fresh = await import(`./voice-admitted-join-native.ts?adapter-remount=${Date.now()}`);
  const calls: string[] = [];
  const native = fresh.voiceAdmittedJoinKeyNative(async <T>(command: string, args: Record<string, unknown>) => {
    calls.push(command);
    const identity = { owner: args.owner, executor: args.executor ?? args.executorId, permit: "new-permit", join: hash(4) };
    if (command === "admit_admitted_voice_join_key") return { ...identity, kind: "admitted" } as T;
    if (command === "publish_admitted_voice_join_key") return { ...identity, kind: "accepted", keyRef: hash(5) } as T;
    return undefined as T;
  });
  const owner = createVoiceCryptoOwner("new-owner-after-remount", hash(1), () => true);
  t.after(async () => { await f.broker.retireNativeOwner(owner); });
  assert.throws(() => f.broker.selectAdmittedJoinKey(f.owner, native), /selection changed/);
  assert.throws(() => f.broker.selectAdmittedJoinKey(owner, { ...native }), /original admitted/);
  const selected = f.broker.selectAdmittedJoinKey(owner, native);
  assert.equal((await selected.attempt()).kind, "accepted");
  await until(() => f.broker.counts().active === 0);
  assert.deepEqual(calls, ["admit_admitted_voice_join_key", "publish_admitted_voice_join_key", "acknowledge_admitted_voice_key_publication", "settle_admitted_voice_join_key"]);
});

test("retained adapter registry storage refuses unknown and prototype-shaped owners", () => {
  assert.equal(isVoiceAdmittedAdapterRegistry(new WeakSet()), true);
  const revoked = Proxy.revocable(new WeakSet(), {}); revoked.revoke();
  for (const invalid of [null, undefined, {}, Object.create(WeakSet.prototype), new Proxy(new WeakSet(), {}), revoked.proxy])
    assert.equal(isVoiceAdmittedAdapterRegistry(invalid), false);
  const changed = new WeakSet(); Object.defineProperty(changed, "owner", { value: "unexpected" });
  assert.equal(isVoiceAdmittedAdapterRegistry(changed), false);
});
