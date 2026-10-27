import assert from "node:assert/strict";
import test from "node:test";
import { webcrypto } from "node:crypto";
import {
  VoiceCryptoBroker, createVoiceCryptoOwner, createVoiceJoinKey, deriveVoicePair, freezeVoicePacket,
  voicePacketIv, windowVoiceCryptoBroker,
} from "./voice-crypto.ts";
import type { VoiceEncryptedIncoming } from "./voice-crypto.ts";

const subtle = webcrypto.subtle as unknown as SubtleCrypto;
const encoder = new TextEncoder();
const text = (value: string) => encoder.encode(value);
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
function fixture(crypto: SubtleCrypto = subtle) {
  let current = true, serial = 0;
  const admitted: string[] = [], settled: string[] = [];
  const admission = {
    async admit() { const token = `crypto-${++serial}`; admitted.push(token); return token; },
    async settle(token: string) { settled.push(token); },
    async commitInbound(_owner: string, _token: string, _packet: VoiceEncryptedIncoming, _plaintext: Uint8Array) {},
  };
  const owner = createVoiceCryptoOwner("native-call-owner", "a".repeat(64), () => current);
  return { owner, admission, admitted, settled, broker: new VoiceCryptoBroker(admission, crypto), retire() { current = false; }, tryResurrect() { current = true; } };
}
async function pairs(crypto: SubtleCrypto = subtle) {
  const f = fixture(crypto);
  const a = await createVoiceJoinKey(f.broker, f.owner), b = await createVoiceJoinKey(f.broker, f.owner);
  const transcript = text("native canonical pair transcript");
  const left = await deriveVoicePair(f.broker, f.owner, a.privateKey, b.publicKey, transcript, true);
  const right = await deriveVoicePair(f.broker, f.owner, b.privateKey, a.publicKey, transcript, false);
  return { ...f, a, b, transcript, left, right };
}

test("voice keys are non-extractable and directional encryption opens only at the peer", async () => {
  const f = await pairs();
  assert.equal(f.a.privateKey.extractable, false);
  await assert.rejects(subtle.exportKey("pkcs8", f.a.privateKey));
  const reservation = { token: "reservation-1", sequence: 1n, aad: text("native exact header") };
  const packet = freezeVoicePacket(f.broker, f.owner, f.left, reservation, text("private SDP"));
  const sealed = await packet.encrypt();
  assert.deepEqual(await f.right.open(reservation.aad, 1n, sealed, "native-inbound"), text("private SDP"));
  await assert.rejects(f.left.open(reservation.aad, 1n, sealed, "native-inbound"), "opposite direction has another key");
  assert.equal(f.admitted.length, f.settled.length);
});

test("SDP accepts exactly 8 KiB and refuses the next byte before native admission or nonce use", async () => {
  const f = await pairs(), aad = text("native bounded SDP header");
  const reservation = { token: "bounded-sdp", sequence: 100n, aad };
  const admitted = f.admitted.length;
  assert.throws(() => freezeVoicePacket(f.broker, f.owner, f.left, reservation, new Uint8Array(8193)), /plaintext/);
  assert.equal(f.admitted.length, admitted, "oversized local input never reaches native engine admission");
  const maximum = new Uint8Array(8192).fill(7);
  const packet = freezeVoicePacket(f.broker, f.owner, f.left, reservation, maximum);
  const sealed = await packet.encrypt();
  assert.equal(sealed.byteLength, 8192 + 16);
  const beforeOpen = f.admitted.length;
  await assert.rejects(f.right.open(aad, 100n, new Uint8Array(8192 + 17), "oversized-sdp"), /ciphertext/);
  assert.equal(f.admitted.length, beforeOpen, "oversized peer input never reaches native engine admission");
  assert.deepEqual(await f.right.open(aad, 100n, sealed, "maximum-sdp"), maximum);
});

test("a relay's own legitimate key cannot open an endpoint pair's packet", async () => {
  const f = await pairs(), relay = await createVoiceJoinKey(f.broker, f.owner);
  const unrelated = await deriveVoicePair(f.broker, f.owner, relay.privateKey, f.a.publicKey, f.transcript, false);
  const aad = text("native header");
  const packet = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 2n, aad }, text("ICE addresses"));
  await assert.rejects(unrelated.open(aad, 2n, await packet.encrypt(), "native-inbound"));
});

test("altered header, sequence, ciphertext or pair transcript cannot authenticate", async () => {
  const f = await pairs(), aad = text("header");
  const packet = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 3n, aad }, text("offer"));
  const sealed = await packet.encrypt();
  await assert.rejects(f.right.open(text("changed header"), 3n, sealed, "native-inbound"));
  await assert.rejects(f.right.open(aad, 4n, sealed, "native-inbound"));
  const changed = sealed.slice(); changed[0] ^= 1;
  await assert.rejects(f.right.open(aad, 3n, changed, "native-inbound"));
  const wrongPair = await deriveVoicePair(f.broker, f.owner, f.b.privateKey, f.a.publicKey, text("other group/room/joins"), false);
  await assert.rejects(wrongPair.open(aad, 3n, sealed, "native-inbound"));
});

test("encryption freezes native AAD and plaintext and caches exact ciphertext once", async () => {
  const f = await pairs(), aad = text("original header"), plaintext = text("original offer");
  const packet = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 5n, aad }, plaintext);
  aad.fill(0); plaintext.fill(0);
  const before = f.admitted.length;
  const [first, retry] = await Promise.all([packet.encrypt(), packet.encrypt()]);
  assert.equal(f.admitted.length - before, 1, "concurrent exact retries encrypt only once");
  assert.deepEqual(first, retry);
  first.fill(0);
  const cached = await packet.encrypt(); assert.deepEqual(cached, retry, "caller mutation cannot change cached bytes");
  assert.deepEqual(await f.right.open(text("original header"), 5n, cached, "native-inbound"), text("original offer"));
});

test("failed encryption burns its reservation and does not run crypto again", async () => {
  const f = await pairs(); f.retire();
  const packet = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 6n, aad: text("header") }, text("offer"));
  const before = f.admitted.length;
  await assert.rejects(packet.encrypt()); await assert.rejects(packet.encrypt());
  assert.equal(f.admitted.length, before, "retired work is never admitted");
});

test("separate wrappers and re-derived live pairs cannot reuse a burned nonce", async () => {
  const f = await pairs(), aad = text("header");
  await freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 7n, aad }, text("first")).encrypt();
  const second = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 7n, aad }, text("different"));
  await assert.rejects(second.encrypt(), /already burned/);
  const derivedAgain = await deriveVoicePair(f.broker, f.owner, f.a.privateKey, f.b.publicKey, f.transcript, true);
  await assert.rejects(freezeVoicePacket(f.broker, f.owner, derivedAgain, { token: "r", sequence: 7n, aad }, text("re-derived")).encrypt(), /already burned/);
});

test("reordered native admission permits both distinct outstanding counters", async () => {
  const f = await pairs(), firstAdmission = deferred<string>();
  let encryptAdmissions = 0;
  f.admission.admit = async () => ++encryptAdmissions === 1 ? firstAdmission.promise : "second-admission";
  const aad = text("header");
  const first = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r1", sequence: 10n, aad }, text("offer")).encrypt();
  const second = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r2", sequence: 11n, aad }, text("ICE")).encrypt();
  assert.ok((await second).length > 16);
  firstAdmission.resolve("first-admission"); assert.ok((await first).length > 16);
});

test("pruning the nonce window never permits a previously burned or too-old counter", () => {
  const f = fixture();
  f.broker.burnSequence(f.owner, "native direction", 1n);
  f.broker.burnSequence(f.owner, "native direction", 130n);
  assert.throws(() => f.broker.burnSequence(f.owner, "native direction", 1n), /already burned/);
  assert.throws(() => f.broker.burnSequence(f.owner, "native direction", 2n), /already burned/);
  f.broker.burnSequence(f.owner, "native direction", 3n);
  assert.throws(() => f.broker.burnSequence(f.owner, "native direction", 3n), /already burned/);
});

test("an actual engine encryption failure burns the nonce through retry and re-derivation", async () => {
  let seals = 0;
  const failing = new Proxy(subtle, { get(target, key) {
    if (key === "encrypt") return async () => { seals++; throw new Error("engine refused encryption"); };
    const value = Reflect.get(target, key); return typeof value === "function" ? value.bind(target) : value;
  } });
  const f = await pairs(failing), aad = text("header");
  const packet = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 12n, aad }, text("offer"));
  await assert.rejects(packet.encrypt(), /engine refused/); await assert.rejects(packet.encrypt(), /engine refused/);
  const again = await deriveVoicePair(f.broker, f.owner, f.a.privateKey, f.b.publicKey, f.transcript, true);
  await assert.rejects(freezeVoicePacket(f.broker, f.owner, again, { token: "r", sequence: 12n, aad }, text("changed")).encrypt(), /already burned/);
  assert.equal(seals, 1);
});

test("native owner fields cannot rebind deferred work or revive after retirement", async () => {
  const f = fixture(), gate = deferred<number>();
  const pending = f.broker.run(f.owner, "derive", 64, () => gate.promise);
  assert.equal(Reflect.set(f.owner, "token", "new-owner"), false);
  assert.equal(Reflect.set(f.owner, "group", "b".repeat(64)), false);
  f.retire(); assert.equal(f.owner.isCurrent(), false); f.tryResurrect();
  assert.equal(f.owner.isCurrent(), false);
  gate.resolve(1); await assert.rejects(pending);
  assert.equal(f.owner.token, "native-call-owner"); assert.equal(f.broker.counts().bytes, 0);
});

test("native inbound refusal after successful decrypt withholds plaintext", async () => {
  const f = await pairs(), aad = text("header");
  const packet = freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 8n, aad }, text("offer"));
  const sealed = await packet.encrypt();
  f.admission.commitInbound = async () => { throw new Error("member removed"); };
  await assert.rejects(f.right.open(aad, 8n, sealed, "native-inbound"), /member removed/);
});

test("decrypted results stay charged and private until exact native confirmation commits", async () => {
  const f = await pairs(), aad = text("native confirmation header"), body = text("native confirmation nonces");
  const sealed = await freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 12n, aad }, body).encrypt();
  await new Promise<void>(resolve => setImmediate(resolve));
  const entered = deferred<void>(), commit = deferred<void>();
  let nativeCopy: Uint8Array | null = null;
  f.admission.commitInbound = async (owner, token, packet, plaintext) => {
    assert.equal(owner, f.owner.token); assert.equal(token, "in-confirm");
    assert.equal(packet.sequence, 12n); assert.deepEqual(packet.aad, aad); assert.deepEqual(packet.ciphertext, sealed);
    assert.deepEqual(plaintext, body); nativeCopy = plaintext;
    entered.resolve(); await commit.promise;
  };
  let released = false;
  const opened = f.right.open(aad, 12n, sealed, "in-confirm").then(value => { released = true; return value; });
  await entered.promise;
  assert.equal(released, false); assert.equal(f.broker.counts().active, 1);
  assert.equal(f.broker.counts().bytes, aad.length + sealed.length);
  const settledBefore = f.settled.length;
  await Promise.resolve(); assert.equal(f.settled.length, settledBefore);
  commit.resolve(); assert.deepEqual(await opened, body);
  assert.ok(nativeCopy); assert.ok((nativeCopy as Uint8Array).every(value => value === 0));
  await new Promise<void>(resolve => setImmediate(resolve)); assert.equal(f.broker.counts().bytes, 0);
});

for (const version of [2, 3, 39, 40, 41, 42, 43, 44, 45]) test("incompatible retained broker v" + version + " refuses without replacing its unresolved pool", async () => {
  const f = fixture(), window = {}, gate = deferred<number>();
  const broker = windowVoiceCryptoBroker(f.admission, window, subtle);
  const pending = broker.run(f.owner, "derive", 64, () => gate.promise);
  await Promise.resolve(); await Promise.resolve();
  Object.defineProperty(broker, "version", { value: version });
  assert.throws(() => windowVoiceCryptoBroker(f.admission, window, subtle), /reload required/);
  assert.equal((window as Record<symbol, unknown>)[Symbol.for("mewtual.voice.crypto-broker.v2")], broker);
  assert.equal(broker.counts().active, 1); assert.equal(broker.counts().bytes, 64);
  gate.resolve(1); assert.equal(await pending, 1);
  await new Promise<void>(resolve => setImmediate(resolve)); assert.equal(broker.counts().bytes, 0);
});

test("retained broker must implement the explicit configured factory before it can serve a new facade", async () => {
  const f = fixture(), host = {}, broker = windowVoiceCryptoBroker(f.admission, host, subtle), held = deferred<number>();
  const running = broker.run(f.owner, "generate", 64, () => held.promise);
  await new Promise<void>(yes => setImmediate(yes));
  Object.defineProperty(broker, "nativeConfiguredAppCalls", { value: undefined });
  assert.throws(() => windowVoiceCryptoBroker(f.admission, host, subtle), /reload required/);
  assert.equal((host as Record<symbol, unknown>)[Symbol.for("mewtual.voice.crypto-broker.v2")], broker);
  assert.equal(broker.counts().bytes, 64); held.resolve(1); assert.equal(await running, 1);
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(broker.counts().bytes, 0);
});

test("explicit owner retirement wakes every observer once and cannot be resurrected", () => {
  const f = fixture();
  let observed = 0;
  const remove = f.owner.onRetired(() => { throw new Error("one observer failed"); });
  f.owner.onRetired(() => { throw new Error("remaining observers must still retire"); });
  f.owner.onRetired(() => { observed++; assert.equal(f.owner.isCurrent(), false); });
  remove();
  f.owner.retire(); f.owner.retire(); f.tryResurrect();
  assert.equal(observed, 1); assert.equal(f.owner.isCurrent(), false);
  f.owner.onRetired(() => { observed++; });
  assert.equal(observed, 2, "late observers see the same latched retirement");
});

test("observing a false owner predicate publishes retirement and bounds listeners", () => {
  const f = fixture();
  let observed = 0;
  for (let i = 0; i < 64; i++) f.owner.onRetired(() => { observed++; });
  assert.throws(() => f.owner.onRetired(() => {}), /capacity/);
  f.retire(); assert.equal(f.owner.isCurrent(), false); f.tryResurrect();
  assert.equal(observed, 64); assert.equal(f.owner.isCurrent(), false);
});

test("retirement at the final broker fence clears withheld decrypted bytes", async () => {
  let engineResult: Uint8Array | null = null;
  const crypto = new Proxy(subtle, { get(target, key) {
    if (key === "decrypt") return async (...args: Parameters<SubtleCrypto["decrypt"]>) => {
      const result = await target.decrypt(...args); engineResult = new Uint8Array(result); return result;
    };
    const value = Reflect.get(target, key); return typeof value === "function" ? value.bind(target) : value;
  } });
  const f = await pairs(crypto), aad = text("native header"), entered = deferred<void>(), commit = deferred<void>();
  const sealed = await freezeVoicePacket(f.broker, f.owner, f.left, { token: "r", sequence: 13n, aad }, text("withheld private offer")).encrypt();
  f.admission.commitInbound = async () => {
    entered.resolve(); await commit.promise;
    // The native hook succeeds; retirement runs after its native/owner check, before the outer
    // broker's final check. No test depends on a brittle count of isCurrent invocations.
    queueMicrotask(() => queueMicrotask(() => f.retire()));
  };
  const pending = f.right.open(aad, 13n, sealed, "in-late"); await entered.promise; commit.resolve();
  await assert.rejects(pending, /owner changed/);
  assert.ok(engineResult); assert.ok((engineResult as Uint8Array).every(value => value === 0));
});

test("a current peer's incoming token cannot authorize another peer's decrypted packet", async () => {
  const f = await pairs(), peerB = await createVoiceJoinKey(f.broker, f.owner);
  const transcriptB = text("native pair transcript for B and C");
  const senderB = await deriveVoicePair(f.broker, f.owner, peerB.privateKey, f.b.publicKey, transcriptB, true);
  const receiverB = await deriveVoicePair(f.broker, f.owner, f.b.privateKey, peerB.publicKey, transcriptB, false);
  const aadA = text("native A-C header"), aadB = text("native B-C header");
  const a = await freezeVoicePacket(f.broker, f.owner, f.left, { token: "out-A", sequence: 9n, aad: aadA }, text("retired A offer")).encrypt();
  const b = await freezeVoicePacket(f.broker, f.owner, senderB, { token: "out-B", sequence: 1n, aad: aadB }, text("current B offer")).encrypt();
  const incoming = new Map<string, VoiceEncryptedIncoming>([
    ["in-A", { aad: aadA, sequence: 9n, ciphertext: a }],
    ["in-B", { aad: aadB, sequence: 1n, ciphertext: b }],
  ]);
  f.admission.commitInbound = async (_owner, token, packet) => {
    const expected = incoming.get(token);
    if (!expected || expected.sequence !== packet.sequence ||
        Buffer.compare(expected.aad, packet.aad) !== 0 || Buffer.compare(expected.ciphertext, packet.ciphertext) !== 0) {
      throw new Error("incoming token packet mismatch");
    }
    if (token === "in-A") throw new Error("original peer A was removed");
  };
  await assert.rejects(f.right.open(aadA, 9n, a, "in-B"), /packet mismatch/);
  await assert.rejects(f.right.open(aadA, 9n, a, "in-A"), /peer A was removed/);
  assert.deepEqual(await receiverB.open(aadB, 1n, b, "in-B"), text("current B offer"));
});

test("invalid public points and exhausted/nonpositive sequence counters refuse", async () => {
  const f = fixture(), own = await createVoiceJoinKey(f.broker, f.owner);
  for (const malformed of [new Uint8Array(64), new Uint8Array(65), Uint8Array.from([4, ...new Array(64).fill(0)])]) {
    await assert.rejects(deriveVoicePair(f.broker, f.owner, own.privateKey, malformed, text("scope"), true));
  }
  assert.deepEqual(voicePacketIv(1n), Uint8Array.from([...new Array(11).fill(0), 1]));
  assert.throws(() => voicePacketIv(0n)); assert.throws(() => voicePacketIv(-1n)); assert.throws(() => voicePacketIv(1n << 64n));
});

test("retirement keeps an unresolved crypto admission charged until actual settlement", async () => {
  const f = fixture(), gate = deferred<number>();
  const pending = f.broker.run(f.owner, "derive", 64, () => gate.promise);
  await Promise.resolve(); await Promise.resolve();
  assert.equal(f.admitted.length, 1); f.retire();
  assert.equal(f.settled.length, 0);
  assert.equal(f.broker.counts().active, 1);
  gate.resolve(42); await assert.rejects(pending);
  assert.deepEqual(f.settled, f.admitted); assert.equal(f.broker.counts().active, 0);
});

test("window broker survives component remount and refuses excessive queued work", async () => {
  const f = fixture(), window = {};
  const broker = windowVoiceCryptoBroker(f.admission, window, subtle);
  assert.equal(windowVoiceCryptoBroker(f.admission, window, subtle), broker);
  const gates = Array.from({ length: 34 }, () => deferred<number>());
  const work = gates.map(gate => broker.run(f.owner, "decrypt", 1, () => gate.promise));
  await Promise.resolve(); await Promise.resolve();
  await assert.rejects(broker.run(f.owner, "decrypt", 1, async () => 0), /capacity/);
  assert.equal(broker.counts().active, 2); assert.equal(broker.counts().queued, 32);
  f.retire(); gates.forEach(gate => gate.resolve(1));
  assert.ok((await Promise.allSettled(work)).every(result => result.status === "rejected"));
  assert.equal(broker.counts().bytes, 0);
});

test("a fresh module's owner works with the retained broker without refunding old work", async () => {
  const f = fixture(), window = {}, gate = deferred<number>();
  const broker = windowVoiceCryptoBroker(f.admission, window, subtle);
  const old = broker.run(f.owner, "derive", 64, () => gate.promise);
  await Promise.resolve(); await Promise.resolve();
  f.retire();
  const moduleUrl = new URL("./voice-crypto.ts", import.meta.url);
  moduleUrl.search = "hmr-owner-regression";
  const reloaded = await import(moduleUrl.href) as typeof import("./voice-crypto.ts");
  const retained = reloaded.windowVoiceCryptoBroker(f.admission, window, subtle);
  assert.equal(retained, broker);
  const freshOwner = reloaded.createVoiceCryptoOwner("new-native-owner", "a".repeat(64), () => true);
  assert.equal(await retained.run(freshOwner, "generate", 1, async () => 7), 7);
  // run resolves the value before its asynchronous settlement/finally releases the slot.
  await new Promise<void>(resolve => setImmediate(resolve));
  assert.equal(f.settled.length, 1, "only the new operation settled");
  assert.equal(broker.counts().active, 1, "old engine operation still holds its slot");
  assert.equal(broker.counts().bytes, 64);
  gate.resolve(1); await assert.rejects(old);
  await new Promise<void>(resolve => setImmediate(resolve));
  assert.equal(broker.counts().active, 0);
  assert.equal(broker.counts().bytes, 0);
});


test("HMR rejects the complete Broker22 API and preserves its unresolved pool until actual cleanup", async () => {
  const f = fixture(), host = {}, gate = deferred<number>();
  const old = windowVoiceCryptoBroker(f.admission, host, subtle);
  const running = old.run(f.owner, "derive", 64, () => gate.promise);
  await Promise.resolve(); await Promise.resolve();
  // Broker22 exposed the same public methods. A sparse fake object would also fail the
  // method checks and could miss an accidentally unchanged retained version entirely.
  Object.defineProperty(old, "version", { value: 22 });
  let retries = 0; const retry = old.retrySelectedJoinKeys.bind(old);
  old.retrySelectedJoinKeys = () => { retries++; retry(); };
  assert.throws(() => windowVoiceCryptoBroker(f.admission, host, subtle), /reload required/);
  assert.equal(retries, 0, "refusal cannot wake or start the old native setup/action drivers");
  assert.equal(f.admitted.length, 1); assert.equal(f.settled.length, 0);
  assert.equal((host as Record<symbol, unknown>)[Symbol.for("mewtual.voice.crypto-broker.v2")], old);
  assert.equal(old.counts().active, 1); assert.equal(old.counts().bytes, 64);
  f.retire(); gate.resolve(1); await assert.rejects(running);
  await new Promise<void>(resolve => setImmediate(resolve));
  assert.equal(old.counts().active, 0); assert.equal(old.counts().bytes, 0);
});
