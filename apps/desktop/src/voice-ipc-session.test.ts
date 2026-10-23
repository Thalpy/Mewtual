import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { webcrypto } from "node:crypto";
import { readFileSync } from "node:fs";
import {
  VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker,
  type VoiceIpcNative, type VoiceSelectedKeyNative, type VoiceCryptoChainBinding,
  type VoiceDerivationInput, type VoiceDerivationNative,
} from "./voice-crypto.ts";
import {
  decodeVoiceIpcAdmission, encodeVoiceIpcAdmission,
  voiceIpcNonce, type VoiceIpcHeader,
} from "./voice-ipc-frame.ts";
import { windowVoiceIpcNative } from "./voice-ipc-native.ts";
import { encodeVoiceIpcAction, type VoiceIpcPurpose } from "./voice-ipc-actions.ts";
import type { VoiceIpcActionReply } from "./voice-ipc-replies.ts";

const real = webcrypto.subtle as unknown as SubtleCrypto;
const utf8 = new TextEncoder();
const hash = (n: number) => n.toString(16).padStart(64, "0");
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const deadline = performance.now() + 15_000;
  while (!check()) { if (performance.now() >= deadline) throw new Error("condition did not settle"); await new Promise<void>(yes => setImmediate(yes)); }
}
const integer = (n: bigint, size: number) => Buffer.from(n.toString(16).padStart(size * 2, "0"), "hex");
const historyPurpose: VoiceIpcPurpose = Object.freeze({ peer: hash(7), join: hash(8), keyRef: hash(9), epoch: "9007199254740993" });
const historyVectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/replies/vectors.json", import.meta.url), "utf8")) as {
  statusCases: { name: string; plaintext: string; expected: VoiceIpcActionReply }[];
  refusedCases: { name: string; plaintext: string; expected: VoiceIpcActionReply }[];
};
const historyVector = (name: string) => {
  const found = [...historyVectors.statusCases, ...historyVectors.refusedCases].find(item => item.name === name);
  assert.ok(found, `missing shared history vector ${name}`); return found;
};

/** Independent actual-WebCrypto native endpoint fixture. It consumes only the published
 * selected point, never this broker's private key or its Session internals/counters. */
async function fixture(t: TestContext, sharedBroker?: VoiceCryptoBroker, identitySeed = 1) {
  const cleanups: Array<() => void> = [];
  let ordinary = 0, setups = 0, settlements = 0, loseSetup = false, loseSettle = false;
  let setupClosed = false, published: Uint8Array | null = null, executor = "";
  let collections = 0, loseCollection = false;
  let tamperSetup = false, badBody = false, failEncrypt = false;
  let forceSequence: bigint | null = null, nativeSequence = 0n;
  const requestSequences: bigint[] = [], engineSteps: string[] = [];
  const bodyReads: number[] = [];
  const operations: number[] = [];
  let historyBody = Buffer.from(historyVector("derived-drained").plaintext, "hex");
  let actionResponder: ((header: VoiceIpcHeader, opened: Uint8Array) => { kind: 0 | 1; body: Uint8Array }) | null = null;
  let responseChange: Partial<VoiceIpcHeader> | null = null;
  let holdAt = -1, held: ReturnType<typeof gate> | null = null, engineEnabled = false;
  let stalled = false, cancelEntered = false, cancelGate: ReturnType<typeof gate> | null = null;
  let observedStep = -1;
  let now = 0, timerSerial = 0, hostileHeaders = false, hangFetch = false, fetchAborted = false, fetchEntered = false;
  const timers = new Map<number, { at: number; run(): void }>();
  let timerPeak = 0, scheduleHook: (() => void) | null = null;
  let fetchCancel: ReturnType<typeof gate> | null = null;
  const engine = new Proxy(real, { get(target, name) {
    const value = Reflect.get(target, name);
    if (typeof value !== "function") return value;
    return async (...args: unknown[]) => {
      if (engineEnabled) {
        const index = engineSteps.length; engineSteps.push(String(name));
        if (index === holdAt) { observedStep = index; await held!.promise; }
        if (name === "encrypt" && failEncrypt) { failEncrypt = false; throw new Error("local provider failure"); }
      }
      return value.apply(target, args);
    };
  } });
  const owner = createVoiceCryptoOwner(identitySeed === 1 ? "native-owner" : `native-owner-${identitySeed}`, hash(identitySeed), () => true);
  const admission = { async admit() { ordinary++; return "ordinary"; }, async settle() {}, async commitInbound() {} };
  const broker = sharedBroker ?? new VoiceCryptoBroker(admission, engine, () => now, {
    now: () => now, schedule(run, ms) {
      const id = ++timerSerial; timers.set(id, { at: now + ms, run }); timerPeak = Math.max(timerPeak, timers.size);
      const hook = scheduleHook; scheduleHook = null; hook?.(); return id;
    },
    cancel(id) { timers.delete(id as number); },
  });
  const selectedNative: VoiceSelectedKeyNative = {
    async admit(owner, value) { executor = value; return { owner, executor, permit: "permit", join: hash(2), kind: "admitted" }; },
    async publish(owner, executor, permit, point) { published = point.slice(); return { owner, executor, permit, join: hash(2), keyRef: hash(3) }; },
    async settle() {},
  };
  const selected = await broker.selectJoinKey(owner, selectedNative).attempt();
  assert.equal(selected.kind, "accepted");
  if (selected.kind !== "accepted") throw new Error("selection failed");
  await until(() => broker.counts().active === 0);
  const nativeKey = await real.generateKey({ name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]);
  const nativePoint = new Uint8Array(await real.exportKey("raw", nativeKey.publicKey));
  const session = Buffer.alloc(16, 4);
  const transcript = Buffer.concat([
    utf8.encode("Mewtual native voice bootstrap\0"), Buffer.from([1, 1]), session, Buffer.from([1]),
    integer(1n, 8), integer(2n, 8), integer(3n, 8), integer(1n, 2), Buffer.from([7]),
    Buffer.from(owner.group, "hex"), Buffer.from([0]), integer(1n, 16), Buffer.alloc(32, 6),
    Buffer.from(hash(2), "hex"), integer(BigInt(Buffer.byteLength(owner.token)), 2), Buffer.from(owner.token),
    Buffer.from(executor, "hex"), published!, nativePoint,
  ]);
  const transcriptHash = new Uint8Array(await real.digest("SHA-256", transcript));
  const peer = await real.importKey("raw", published!, { name: "ECDH", namedCurve: "P-256" }, false, []);
  const shared = await real.deriveBits({ name: "ECDH", public: peer }, nativeKey.privateKey, 256);
  const base = await real.importKey("raw", shared, "HKDF", false, ["deriveBits"]);
  const seed = await real.deriveBits({ name: "HKDF", hash: "SHA-256", salt: transcriptHash, info: utf8.encode("Mewtual native voice bootstrap seed v1") }, base, 256);
  const material = await real.importKey("raw", seed, "HKDF", false, ["deriveKey"]);
  new Uint8Array(shared).fill(0); new Uint8Array(seed).fill(0);
  const parameters = (label: string) => ({ name: "HKDF", hash: "SHA-256", salt: utf8.encode("Mewtual native voice handoff v1"), info: utf8.encode(label) });
  const requestMac = await real.deriveKey(parameters("v1 c2n header HMAC-SHA256"), material, { name: "HMAC", hash: "SHA-256", length: 256 }, false, ["verify"]);
  const requestBody = await real.deriveKey(parameters("v1 c2n body AES-256-GCM"), material, { name: "AES-GCM", length: 256 }, false, ["decrypt"]);
  const responseMac = await real.deriveKey(parameters("v1 n2c header HMAC-SHA256"), material, { name: "HMAC", hash: "SHA-256", length: 256 }, false, ["sign"]);
  const responseBody = await real.deriveKey(parameters("v1 n2c body AES-256-GCM"), material, { name: "AES-GCM", length: 256 }, false, ["encrypt"]);
  const native: VoiceIpcNative = {
    async setup(value, execution) {
      setups++; assert.equal(value, owner.token); assert.equal(execution, executor);
      if (loseSetup) { loseSetup = false; throw new Error("native ready reply lost"); }
      if (setupClosed) return { owner: value, executor: execution, kind: "closed", nativeSetupDrained: true };
      return { owner: value, executor: tamperSetup ? hash(99) : execution, kind: "ready", port: 43210, transcript: transcript.toString("hex") };
    },
    async settleSetup(value, execution) {
      if (!setups && !owner.isCurrent()) throw new Error("unknown retired owner");
      settlements++; assert.equal(value, owner.token); assert.equal(execution, executor); setupClosed = true;
      if (loseSettle) { loseSettle = false; throw new Error("settlement reply lost"); }
    },
    async collectChannel(value, execution) {
      collections++; assert.equal(value, owner.token); assert.equal(execution, executor);
      assert.equal(setupClosed, true, "final collection follows exact setup closure");
      if (loseCollection) { loseCollection = false; throw new Error("collection reply lost"); }
    },
  };
  const transport: typeof fetch = async (input, init) => {
    assert.equal(input, "http://127.0.0.1:43210/voice/v2");
    assert.equal(init!.credentials, "omit"); assert.equal(init!.redirect, "error");
    assert.equal(init!.keepalive, false); assert.equal(init!.cache, "no-store");
    const headers = new Headers(init!.headers);
    const request = decodeVoiceIpcAdmission(headers.get("x-voice-admission")!);
    if (hangFetch) {
      fetchEntered = true;
      return await new Promise<Response>((_yes, no) => {
        init!.signal!.addEventListener("abort", () => {
          fetchAborted = true;
          void fetchCancel!.promise.then(() => no(new Error("fetch cancelled")));
        }, { once: true });
      });
    }
    assert.equal(await real.verify("HMAC", requestMac, request.tag, request.bytes), true);
    requestSequences.push(request.header.sequence);
    const opened = new Uint8Array(await real.decrypt({ name: "AES-GCM", iv: voiceIpcNonce(request.header), additionalData: request.bytes }, requestBody, init!.body as Uint8Array<ArrayBuffer>));
    operations.push(request.header.operation);
    let actionReply: { kind: 0 | 1; body: Uint8Array } | null = null;
    if (request.header.operation === 9) {
      assert.deepEqual(opened, new Uint8Array(Buffer.concat([Buffer.from([1]), session, transcriptHash])));
    } else if (request.header.operation === 8) {
      assert.equal(request.header.operation, 8); assert.equal(request.header.kind, 0);
      assert.equal(request.header.logical, "0".repeat(32)); assert.equal(request.header.length, 122);
      assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "begin", retry: false }).body);
    } else {
      assert.ok(actionResponder, "unexpected actual framed action");
      actionReply = actionResponder(request.header, opened);
    }
    opened.fill(0);
    const plaintext = actionReply?.body ?? (request.header.operation === 8 ? historyBody : new Uint8Array([1, setupClosed ? 2 : 1, 1]));
    const response: VoiceIpcHeader = Object.freeze({ ...request.header, direction: 1, kind: actionReply?.kind ?? 0, length: plaintext.length + 16,
      sequence: forceSequence ?? ++nativeSequence, ...responseChange });
    // Deliberately allow a valid native MAC over a wrongly bound response. The actual
    // browser must reject its binding before accepting/reading the body, even with a MAC.
    const aad = new Uint8Array(Buffer.concat([
      Buffer.from(decodeVoiceIpcAdmission(encodeVoiceIpcAdmission(response, new Uint8Array(32))).bytes), request.bytes,
    ]));
    const body = new Uint8Array(await real.encrypt({ name: "AES-GCM", iv: voiceIpcNonce(response), additionalData: aad }, responseBody, plaintext));
    if (badBody) body[0] ^= 1;
    const mac = new Uint8Array(await real.sign("HMAC", responseMac, aad));
    const wire = new Uint8Array(Buffer.concat([utf8.encode(encodeVoiceIpcAdmission(response, mac)), body]));
    let used = 0;
    const stream = new ReadableStream({ type: "bytes", pull(controller: ReadableByteStreamController) {
      const nextLength = controller.byobRequest!.view!.byteLength;
      if (nextLength !== 144) bodyReads.push(nextLength);
      if (stalled && used >= 144) return;
      const request = controller.byobRequest!, view = request.view!;
      const n = Math.min(view.byteLength, wire.length - used);
      new Uint8Array(view.buffer, view.byteOffset, n).set(wire.subarray(used, used + n)); used += n;
      request.respond(n); if (used === wire.length) controller.close();
    }, async cancel() { cancelEntered = true; if (cancelGate) await cancelGate.promise; } } as UnderlyingByteSource);
    const result = new Response(stream, { headers: { "content-length": String(wire.length), "x-voice-admission": "hostile".repeat(hostileHeaders ? 65536 : 1) } });
    if (hostileHeaders) result.headers.get = () => { throw new Error("must not materialize hostile header values"); };
    return result;
  };
  engineEnabled = true;
  t.after(async () => {
    for (const cleanup of cleanups) cleanup();
    held?.release(); cancelGate?.release(); fetchCancel?.release(); owner.retire(); broker.retrySelectedJoinKeys();
    await until(() => broker.counts().active === 0);
    assert.equal(broker.counts().bytes, 0);
  });
  return {
    owner, broker, native, transport, key: selected.key, admission,
    cleanup(work: () => void) { cleanups.push(work); },
    channel() { return broker.nativeChannel(owner, selected.key, native, transport); },
    counts: () => ({ ordinary, setups, settlements, collections, steps: [...engineSteps], tx: [...requestSequences], reads: bodyReads.length }),
    historyBytes(value: Uint8Array) { historyBody = Buffer.from(value); },
    actions(responder: typeof actionResponder) { actionResponder = responder; },
    historyReply(name: string) { historyBody = Buffer.from(historyVector(name).plaintext, "hex"); },
    responseChange(value: Partial<VoiceIpcHeader> | null) { responseChange = value; },
    operations: () => [...operations], bodyReads: () => [...bodyReads],
    hold(index: number) { holdAt = index; held = gate(); return { release: held.release, entered: () => observedStep === index }; },
    loseSetup() { loseSetup = true; }, loseSettle() { loseSettle = true; }, tamperSetup() { tamperSetup = true; },
    closeBeforeSetup() { setupClosed = true; },
    loseCollection() { loseCollection = true; },
    failEncrypt() { failEncrypt = true; }, badBody(value = true) { badBody = value; }, forceSequence(value: bigint | null) { forceSequence = value; },
    stallBody() { stalled = true; cancelGate = gate(); return { entered: () => cancelEntered, release: cancelGate.release }; },
    hostileHeaders() { hostileHeaders = true; },
    hangFetch() { hangFetch = true; fetchCancel = gate(); return { entered: () => fetchEntered, aborted: () => fetchAborted, release() { hangFetch = false; fetchCancel!.release(); } }; },
    timers: () => ({ pending: timers.size, peak: timerPeak, callbacks: [...timers.values()].map(timer => timer.run),
      delays: [...timers.values()].map(timer => timer.at - now) }),
    fireTimerEarly() {
      assert.equal(timers.size, 1); const [id, timer] = [...timers][0]; timers.delete(id); timer.run();
    },
    onSchedule(work: () => void) { scheduleHook = work; },
    advance(ms: number) { now += ms; for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.run(); } },
  };
}

/** Real selected keys and provider work, with mock Root action replies. Status inside the
 * action hooks exercises same-channel RPC progress; these tests do NOT establish native
 * framed action ingress or native pair provenance, which need their own integration gate. */
async function linkedDerivation(f: Awaited<ReturnType<typeof fixture>>, channel = f.channel()) {
  const peer = await real.generateKey({ name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]);
  const setup = { localPublicKey: f.key.publicKey(), peerPublicKey: new Uint8Array(await real.exportKey("raw", peer.publicKey)), transcript: utf8.encode("original native handoff purpose") };
  const binding: VoiceCryptoChainBinding = {
    chain: "1", epoch: "0", localDevice: hash(6), localJoin: hash(2), localKey: hash(3),
    peerDevice: hash(7), peerJoin: hash(8), peerKey: hash(9), handshake: hash(10),
    generation: "1", nativeDeadline: "15000", localIsLow: true,
  };
  const input: VoiceDerivationInput = { owner: f.owner.token, action: "1", version: "1", inputTag: hash(11),
    binding, ...setup, engineBytes: 3 * setup.transcript.length + 4096 + 256 };
  const empty = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
  let admits = 0, commits = 0, settlements = 0, loseSettle = false;
  let settleHold: ReturnType<typeof gate> | null = null;
  let admitHold: ReturnType<typeof gate> | null = null, admitted = false;
  f.cleanup(() => { settleHold?.release(); admitHold?.release(); });
  const native: VoiceDerivationNative = {
    async admitAction(id, bytes) {
      admits++; assert.equal(id.owner, f.owner.token); assert.equal(bytes, input.engineBytes);
      assert.equal((await channel.status()).kind, "ready");
      admitted = true; if (admitHold) await admitHold.promise;
      return { ...id, kind: "admitted", permit: "original-permit" };
    },
    async commitDerivation(id, tag) {
      commits++; assert.equal(id.owner, f.owner.token); assert.equal(tag, empty);
      assert.equal((await channel.status()).kind, "ready");
      return { ...id, kind: "accepted", resultTag: tag, receipt: "exact-receipt", derived: true };
    },
    async settleAction(id) {
      settlements++; assert.equal(id.owner, f.owner.token);
      if (settleHold) await settleHold.promise;
      if (loseSettle) { loseSettle = false; throw new Error("original settlement reply lost"); }
    },
  };
  const plan = () => f.broker.prepareChainFromChannel(f.owner, channel, f.key, binding, 15000, setup);
  return { channel, setup, binding, input, native, plan,
    register() { return f.broker.registerProtectedDerivation(f.owner, plan(), native, input); },
    counts: () => ({ admits, commits, settlements }),
    holdSettle() { settleHold = gate(); return settleHold; },
    holdAdmit() { admitHold = gate(); return { release: admitHold.release, entered: () => admitted }; },
    loseSettle() { loseSettle = true; },
  };
}

/** Independent fixed native projection for browser transport tests. Unlike native integration
 * tests this fixture grants no Actor/MLS authority; its fake keyRef belongs to its mock signer. */
async function framedDerivation(f: Awaited<ReturnType<typeof fixture>>) {
  const peer = await real.generateKey({ name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]);
  const point = new Uint8Array(await real.exportKey("raw", peer.publicKey));
  const transcript = utf8.encode("original native framed action");
  const row = "0".repeat(31) + "1";
  const identity = Object.freeze({ action: "9007199254741007", version: "1", inputTag: hash(11) });
  const engineBytes = 3 * transcript.length + 4096 + 256;
  const binding = { chain: "1", epoch: historyPurpose.epoch, localDevice: "06".repeat(32), localJoin: hash(2), localKey: hash(3),
    peerDevice: historyPurpose.peer, peerJoin: historyPurpose.join, peerKey: historyPurpose.keyRef, handshake: hash(10),
    generation: "9007199254741011", nativeDeadline: "500000", localIsLow: true };
  const data = Buffer.concat([Buffer.from([1, 5]), Buffer.from(row, "hex"), integer(BigInt(identity.action), 8), integer(1n, 8), Buffer.from(identity.inputTag, "hex"),
    integer(1n, 8), Buffer.from(f.owner.group, "hex"), Buffer.from([0]), Buffer.alloc(32), integer(1n, 16), integer(BigInt(binding.epoch), 8),
    ...[binding.localDevice, binding.localJoin, binding.localKey, binding.peerDevice, binding.peerJoin, binding.peerKey, binding.handshake].map(s => Buffer.from(s, "hex")),
    integer(BigInt(binding.generation), 8), integer(BigInt(binding.nativeDeadline), 8), Buffer.from([1]), integer(BigInt(engineBytes), 4), Buffer.from([0]),
    f.key.publicKey(), point, integer(BigInt(transcript.length), 2), transcript]);
  assert.equal(data.length, 541 + transcript.length);
  let inputReads = 0, admissions = 0, commits = 0, acknowledgements = 0, settlements = 0;
  let derived = false, acknowledged = false, settled = false, historical = false, lostResult = false;
  let retireCalls = 0, finalCalls = 0, finalAcknowledged = false, finalPending = false;
  let loseIssuance = false, loseFinalAck = false, ordinaryReturned = false;
  let planCalls = 0, finalMode: 1 | 2 = 1, replyMode: 1 | 2 | null = null;
  let alter: ((data: Uint8Array) => Uint8Array) | null = null;
  const emptyTag = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
  const receipt = hash(77);
  const proof = hash(78);
  const planProof = hash(79);
  const status = () => {
    const bytes = Buffer.alloc(242); bytes[0] = 1; Buffer.from(row, "hex").copy(bytes, 2); bytes[19] = 1;
    integer(BigInt(identity.action), 8).copy(bytes, 20); integer(1n, 8).copy(bytes, 28); Buffer.from(identity.inputTag, "hex").copy(bytes, 36);
    bytes[68] = Number(settled); bytes[69] = Number(derived); bytes[208] = Number(acknowledged);
    bytes[209] = Number(ordinaryReturned); bytes[207] = Number(ordinaryReturned && derived);
    if (derived) { integer(BigInt(identity.action), 8).copy(bytes, 70); Buffer.from(identity.inputTag, "hex").copy(bytes, 78);
      Buffer.from(emptyTag, "hex").copy(bytes, 110); Buffer.from(receipt, "hex").copy(bytes, 210); }
    return { kind: 0 as const, body: bytes };
  };
  const nativeSettle = f.native.settleSetup;
  f.native.settleSetup = async (...args) => {
    assert.ok(finalAcknowledged, "native setup cannot close ahead of the separate original final ACK");
    await nativeSettle(...args);
  };
  f.cleanup(() => { finalPending = false; loseIssuance = false; loseFinalAck = false; f.native.settleSetup = nativeSettle; });
  f.actions((header, opened) => {
    assert.equal(header.logical, row);
    if (header.operation === 1) {
      inputReads++;
      // This callback is reached only after actual request AEAD/fetch. The full claim MUST
      // already be visible, even though no chain/input has yet been materialized locally.
      assert.ok(f.broker.counts().bytes >= 128 * 1024);
      assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "input" }).body);
      if (historical) return status();
      return { kind: 1, body: alter ? alter(new Uint8Array(data)) : data };
    }
    if (header.operation === 2) {
      admissions++; assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "admit", identity, engineBytes }).body);
      const permit = Buffer.concat([Buffer.from([1, 6]), Buffer.from(row, "hex"), integer(BigInt(identity.action), 8), integer(1n, 8),
        Buffer.from(identity.inputTag, "hex"), integer(BigInt(identity.action), 8), integer(BigInt(engineBytes), 4)]);
      return { kind: 0, body: permit };
    }
    if (header.operation === 3) {
      commits++; assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "result", identity, tag: emptyTag, output: new Uint8Array() }).body);
      derived = true;
      if (lostResult) { lostResult = false; throw new Error("native committed but result reply lost"); }
      return status();
    }
    if (header.operation === 4) {
      acknowledgements++; assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "ack", identity, digest: receipt }).body);
      acknowledged = true; return status();
    }
    if (header.operation === 6 || header.operation === 7) {
      assert.ok(settled && (!derived || acknowledged), "native terminal close follows exact action settlement/receipt ACK");
      if (header.operation === 6) {
        if (opened[105] === 1) {
          retireCalls++; assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "close", mode: "retire" }).body);
          assert.equal(planCalls, 0, "Plan retries cannot revert to Phase mode after uncertain intent");
          if (ordinaryReturned) return { kind: 0, body: new Uint8Array([1, 3, 5]) };
        } else {
          planCalls++; assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "close-plan" }).body);
          assert.ok(ordinaryReturned, "Plan final is a distinct original-parent intent after Return");
          finalMode = 2;
        }
        if (loseIssuance) { loseIssuance = false; throw new Error("actual native final issuance reply lost"); }
      } else {
        finalCalls++; assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "final", proof: finalMode === 2 ? planProof : proof }).body);
        assert.equal(finalPending, false); finalAcknowledged = true;
        if (loseFinalAck) { loseFinalAck = false; throw new Error("actual native final ACK reply lost"); }
      }
      const body = Buffer.concat([Buffer.from([1, 4]), Buffer.from(row, "hex"), integer(BigInt(identity.action), 8), integer(1n, 8),
        Buffer.from(identity.inputTag, "hex"), Buffer.from([replyMode ?? finalMode, finalPending ? 0 : finalAcknowledged ? 2 : 1]),
        Buffer.from(finalPending ? "0".repeat(64) : finalMode === 2 ? planProof : proof, "hex")]);
      return { kind: 0, body };
    }
    assert.equal(header.operation, 5); settlements++; settled = true; return status();
  });
  return { row, identity, binding, data, engineBytes, counts: () => ({ inputReads, admissions, commits, acknowledgements, settlements }),
    historical() { historical = true; derived = true; settled = true; },
    loseResult() { lostResult = true; }, alter(work: typeof alter) { alter = work; },
    finalCounts: () => ({ retireCalls, finalCalls, finalAcknowledged }),
    planCalls: () => planCalls,
    finalReplyMode(value: typeof replyMode) { replyMode = value; },
    pendingFinal(value = true) { finalPending = value; },
    loseFinalReplies() { loseIssuance = true; loseFinalAck = true; },
    returned() { ordinaryReturned = true; historical = true; settled = true; },
  };
}

test("native Input prepays and transfers the same claim, then executes real framed admission/result/ACK/settlement", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f);
  const baseline = f.broker.counts().bytes;
  const intake = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  assert.equal(f.broker.counts().bytes, baseline + 128 * 1024);
  assert.equal(f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity), intake);
  const attempt = intake.attempt(); assert.equal(intake.attempt(), attempt);
  const ready = await attempt; assert.equal(ready.kind, "ready");
  if (ready.kind !== "ready") throw new Error("actual framed derivation input required");
  assert.equal(f.broker.counts().bytes, baseline + 128 * 1024, "adoption never refunds/recharges or adds another chain");
  assert.equal((await ready.derivation.attempt()).kind, "accepted");
  await until(() => source.counts().settlements === 1 && f.broker.counts().active === 0);
  assert.ok(ready.derivation.pair());
  assert.deepEqual(source.counts(), { inputReads: 1, admissions: 1, commits: 1, acknowledgements: 1, settlements: 1 });
  assert.deepEqual(f.operations(), [9, 1, 2, 3, 4, 5]);
});

test("historical Input drains prepaid intake without deriving keys or admitting an engine", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f); source.historical();
  const baseline = f.broker.counts().bytes;
  const intake = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  const answer = await intake.attempt(); assert.equal(answer.kind, "status");
  assert.equal((await intake.attempt()).kind, "status");
  assert.equal(f.broker.counts().bytes, baseline + 8 * 1024, "only paid fixed intake history remains");
  assert.deepEqual(source.counts(), { inputReads: 1, admissions: 0, commits: 0, acknowledgements: 0, settlements: 0 });
});

test("historical Input cannot lose its original final cleanup descriptor to a second row", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f); source.historical();
  const original = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  assert.equal((await original.attempt()).kind, "status");
  const paid = f.broker.counts().bytes;
  assert.throws(() => f.broker.nativeDerivationFromChannel(f.owner, channel, f.key,
    { ...historyPurpose, peer: hash(19) }, "0".repeat(31) + "2", { ...source.identity, action: "9007199254741008" }), /ready voice channel/);
  assert.equal(f.broker.counts().bytes, paid, "a different plan refuses before any new charge");
  assert.equal(f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity), original);
  f.owner.retire(); await channel.close(); await until(() => f.broker.counts().bytes === 0);
  assert.deepEqual(source.finalCounts(), { retireCalls: 1, finalCalls: 1, finalAcknowledged: true });
  assert.equal(source.counts().inputReads, 1);
});

test("post-End final issuance and ACK recover lost replies using original framing keys without repeating derivation", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f);
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity).attempt();
  if (ready.kind !== "ready") throw new Error("framed native input required");
  assert.equal((await ready.derivation.attempt()).kind, "accepted");
  await until(() => source.counts().settlements === 1 && f.broker.counts().active === 0);
  const derives = f.counts().steps.filter(step => step === "deriveBits").length;
  source.loseFinalReplies(); f.owner.retire();
  const close = channel.close();
  await until(() => source.finalCounts().retireCalls === 1 && f.timers().delays.includes(1000));
  assert.equal(f.counts().settlements, 0, "native channel stays open after a lost issuance reply");
  f.advance(1000);
  await until(() => source.finalCounts().finalCalls === 1 && f.timers().delays.includes(1000));
  assert.equal(f.counts().settlements, 0, "native channel stays open after a lost final ACK reply");
  f.advance(1000); await close; await until(() => f.broker.counts().bytes === 0);
  assert.deepEqual(source.finalCounts(), { retireCalls: 2, finalCalls: 2, finalAcknowledged: true });
  assert.equal(source.planCalls(), 0, "lost Phase replies never select another intent");
  assert.equal(f.counts().steps.filter(step => step === "deriveBits").length, derives);
  assert.equal(f.counts().setups, 1); assert.equal(f.timers().pending, 0);
});

test("post-End Pending retains Control and retries on bounded Clock cadence until actual native retirement", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f); source.pendingFinal();
  f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  f.owner.retire(); let done = false;
  const close = channel.close().then(() => { done = true; });
  await until(() => source.finalCounts().retireCalls === 1 && f.timers().delays.includes(1000));
  const held = f.broker.counts().bytes;
  assert.ok(held > 0); assert.equal(done, false); assert.equal(f.counts().settlements, 0);
  f.advance(100_000);
  await until(() => source.finalCounts().retireCalls === 2 && f.timers().delays.includes(1000));
  assert.equal(f.broker.counts().bytes, held); assert.equal(source.finalCounts().finalCalls, 0);
  source.pendingFinal(false); f.advance(1000); await close;
  await until(() => f.broker.counts().bytes === 0);
  assert.equal(source.finalCounts().finalAcknowledged, true); assert.equal(f.timers().pending, 0);
  assert.equal(source.planCalls(), 0, "Pending Phase intent must remain Phase");
});

test("ordinary Return retains original Plan mode through lost intent and ACK replies", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f); source.returned();
  await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity).attempt();
  const steps = f.counts().steps.length;
  source.loseFinalReplies(); f.owner.retire(); const close = channel.close();
  await until(() => source.planCalls() === 1 && f.timers().delays.includes(1000));
  assert.equal(source.finalCounts().retireCalls, 1); assert.equal(f.counts().settlements, 0);
  f.advance(1000);
  await until(() => source.finalCounts().finalCalls === 1 && f.timers().delays.includes(1000));
  assert.equal(source.planCalls(), 2); assert.equal(source.finalCounts().retireCalls, 1);
  assert.equal(f.counts().settlements, 0); assert.ok(f.broker.counts().bytes > 0);
  f.advance(1000); await close; await until(() => f.broker.counts().bytes === 0);
  assert.deepEqual(source.finalCounts(), { retireCalls: 1, finalCalls: 2, finalAcknowledged: true });
  assert.equal(source.planCalls(), 2); assert.equal(f.counts().setups, 1);
  assert.equal(f.counts().steps.filter(s => s === "deriveBits").length,
    f.counts().steps.slice(0, steps).filter(s => s === "deriveBits").length, "cleanup never derives replacement keys");
});

for (const refusal of ["pending", "wrong-kind"] as const) test(`Plan ${refusal} retains keys and retries the same mode before setup closure`, async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f); source.returned();
  await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity).attempt();
  if (refusal === "pending") source.pendingFinal(); else source.finalReplyMode(1);
  f.owner.retire(); let done = false;
  const close = channel.close().then(() => { done = true; });
  await until(() => source.planCalls() === 1 && f.timers().delays.includes(1000));
  assert.equal(done, false); assert.equal(f.counts().settlements, 0); assert.equal(f.counts().collections, 0);
  assert.equal(source.finalCounts().finalCalls, 0); assert.ok(f.broker.counts().bytes > 0);
  source.pendingFinal(false); source.finalReplyMode(2); f.advance(1000); await close;
  await until(() => f.broker.counts().bytes === 0);
  assert.equal(source.finalCounts().retireCalls, 1); assert.equal(source.planCalls(), 2);
  assert.equal(source.finalCounts().finalAcknowledged, true); assert.equal(f.counts().setups, 1);
});

test("historical ordinary Return uses a separate original Plan-final intent and exact ACK", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f); source.returned();
  const reply = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity).attempt();
  assert.equal(reply.kind, "status");
  await channel.close(); await until(() => f.broker.counts().active === 0);
  assert.deepEqual(source.finalCounts(), { retireCalls: 1, finalCalls: 1, finalAcknowledged: true });
  assert.equal(source.planCalls(), 1);
  assert.equal(f.counts().settlements, 1, "native final ACK still precedes channel preflight");
});

test("ordinary history never bypasses native channel preflight or loses keys after its refusal", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f); source.returned();
  await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity).attempt();
  const settle = f.native.settleSetup; let attempts = 0;
  f.cleanup(() => { f.native.settleSetup = settle; });
  f.native.settleSetup = async (...args) => {
    if (++attempts === 1) throw new Error("native preflight still owns a required original channel row");
    await settle(...args);
  };
  f.owner.retire(); const close = channel.close();
  await until(() => attempts === 1 && f.timers().delays.includes(1000));
  assert.equal(f.counts().collections, 0); assert.equal(f.counts().settlements, 0);
  assert.ok(f.broker.counts().bytes > 0);
  f.advance(1000); await close; await until(() => f.broker.counts().bytes === 0);
  assert.deepEqual(source.finalCounts(), { retireCalls: 1, finalCalls: 1, finalAcknowledged: true });
  assert.equal(source.planCalls(), 1, "lost close reply never repeats acknowledged Plan intent");
  assert.equal(f.counts().setups, 1, "successful retry framing used original keys, never new setup");
  assert.equal(f.counts().collections, 1);
});

test("lost native result replies retry the same action/output automatically without a second derive", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f); source.loseResult();
  const intake = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  const ready = await intake.attempt(); if (ready.kind !== "ready") throw new Error("framed input required");
  assert.equal((await ready.derivation.attempt()).kind, "uncertain");
  const derives = f.counts().steps.filter(step => step === "deriveBits").length;
  f.advance(1000);
  await until(() => ready.derivation.inspect() === "accepted" && source.counts().settlements === 1 && f.broker.counts().active === 0);
  assert.equal(f.counts().steps.filter(step => step === "deriveBits").length, derives);
  assert.deepEqual(source.counts(), { inputReads: 1, admissions: 1, commits: 2, acknowledgements: 1, settlements: 1 });
});

test("lost Input and malformed bound metadata retain one claim and never renew its original deadline", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f);
  source.alter(data => { data[162] ^= 1; return data; }); // valid MAC; wrong MLS epoch
  const intake = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  const reserved = f.broker.counts().bytes;
  assert.equal((await intake.attempt()).kind, "uncertain");
  assert.equal(f.broker.counts().bytes, reserved);
  f.advance(14_999); source.alter(null);
  assert.equal((await intake.attempt()).kind, "ready");
  f.advance(1);
  const next = await intake.attempt();
  if (next.kind === "ready") assert.equal(next.chain.isCurrent(), false, "native deadline500000 does not renew browser deadline15000");
  assert.equal(source.counts().admissions, 0);
});

test("End while Input SDK is unresolved retains its intake/payment until actual completion", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f);
  const held = f.hold(f.counts().steps.length);
  const intake = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  const pending = intake.attempt(); await until(held.entered);
  const reserved = f.broker.counts().bytes; f.owner.retire();
  assert.equal(f.broker.counts().bytes, reserved);
  held.release(); assert.equal((await pending).kind, "retired");
  await channel.close(); await until(() => f.broker.counts().bytes === 0);
  assert.equal(source.counts().inputReads, 0);
});

test("explicit channel close retires an idle Input claim without waiting for its deadline", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f);
  const intake = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  await channel.close(); // no Clock advancement, owner is still current
  assert.equal(f.owner.isCurrent(), true);
  assert.equal((await intake.attempt()).kind, "retired");
  assert.equal(source.counts().inputReads, 0);
  assert.equal(f.timers().pending, 0);
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.broker.counts().active, 0);
});

test("explicit channel close retains a blocked Input read until actual cancellation drains", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const source = await framedDerivation(f), blocked = f.hangFetch();
  const intake = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, source.row, source.identity);
  const attempt = intake.attempt(); await until(blocked.entered);
  const paid = f.broker.counts().bytes;
  let closed = false; const close = channel.close().then(() => { closed = true; });
  await until(blocked.aborted);
  assert.equal(closed, false);
  assert.equal(f.broker.counts().bytes, paid, "abort is not actual read drainage");
  blocked.release();
  assert.equal((await attempt).kind, "retired");
  await close; // same Clock instant, no 15-second intake expiry
  assert.equal(f.owner.isCurrent(), true);
  assert.equal(f.timers().pending, 0);
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.broker.counts().active, 0);
  assert.equal(source.counts().inputReads, 0);
});

test("two retained channels lend their existing two positions without a third reservation or nested RPC deadlock", async t => {
  const f = await fixture(t), a = f.channel(); await a.status();
  const second = await fixture(t, f.broker, 2), b = second.channel(); await b.status();
  const linked = await linkedDerivation(f, a);
  const linkedSecond = await linkedDerivation(second, b);
  assert.throws(() => f.broker.prepareChain(f.owner, f.key, linked.binding, 15000, linked.setup), /capacity/);
  const plan = linked.plan(); assert.equal(linked.plan(), plan);
  const action = linked.register(), secondAction = linkedSecond.register();
  assert.deepEqual((await Promise.all([action.attempt(), secondAction.attempt()])).map(outcome => outcome.kind), ["accepted", "accepted"]);
  assert.ok(action.pair()); assert.ok(secondAction.pair());
  assert.throws(() => f.broker.registerProtectedDerivation(f.owner, plan, { ...linked.native }, linked.input), /identity changed/);
  assert.equal(linked.counts().admits, 1); assert.equal(linked.counts().commits, 1);
  await a.close(); second.owner.retire(); await b.close();
  assert.equal(linked.counts().settlements, 1);
  assert.deepEqual(linkedSecond.counts(), { admits: 1, commits: 1, settlements: 1 });
});

test("a lent provider and same-channel status are mutually exclusive through actual provider settlement", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const linked = await linkedDerivation(f, channel);
  // Admit's four Status SDK calls precede row0's first import. Hold its real ECDH after import.
  const provider = f.hold(f.counts().steps.length + 5), action = linked.register();
  const attempt = action.attempt(); await until(provider.entered);
  const before = f.counts(), status = channel.status();
  assert.equal(channel.status(), status, "logical status observers still coalesce");
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.counts().steps.length, before.steps.length, "queued channel cannot enter SDK behind a running provider");
  assert.deepEqual(f.counts().tx, before.tx);
  assert.equal(f.broker.counts().active, 1, "one physical position, not two logical reservations");
  provider.release(); assert.equal((await status).kind, "ready");
  assert.equal((await attempt).kind, "accepted");
  await channel.close();
});

test("End with a provider and queued channel retains cleanup transport until exact action settlement", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const linked = await linkedDerivation(f, channel), settlement = linked.holdSettle();
  const provider = f.hold(f.counts().steps.length + 5), action = linked.register();
  const attempt = action.attempt(); await until(provider.entered);
  const status = channel.status(), charged = f.broker.counts().bytes;
  f.owner.retire();
  assert.equal((await attempt).kind, "retired"); assert.equal((await status).kind, "retired");
  let closed = false; const close = channel.close().then(() => { closed = true; });
  assert.equal(f.counts().settlements, 0);
  // End legitimately returns the eleven unused fixed row claims. Actual row0,96KiB
  // variable copies and original key/control custody must still remain until their owners drain.
  assert.equal(f.broker.counts().bytes, charged - 11 * 2048);
  f.advance(2_000); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.timers().pending, 0, "cleanup cadence cannot precede actual linked provider drainage");
  assert.equal(f.counts().settlements, 0);
  provider.release(); await until(() => linked.counts().settlements === 1);
  assert.equal(closed, false); assert.equal(f.counts().settlements, 0); assert.equal(f.counts().collections, 0);
  assert.ok(f.broker.counts().bytes > 0, "original key/control/chain custody stays retained");
  settlement.release(); await close;
  assert.equal(f.counts().settlements, 1); assert.equal(f.counts().collections, 1);
});

test("lost linked settlement after End retries only on an availability wake and keeps original cleanup custody", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const linked = await linkedDerivation(f, channel); linked.loseSettle();
  const provider = f.hold(f.counts().steps.length + 5), action = linked.register();
  const attempt = action.attempt(); await until(provider.entered);
  f.owner.retire(); assert.equal((await attempt).kind, "retired");
  let closed = false; const close = channel.close().then(() => { closed = true; });
  provider.release();
  await until(() => linked.counts().settlements === 1);
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.equal(f.counts().settlements, 0);
  assert.equal(linked.counts().settlements, 1, "no retry timer/failure loop");
  f.broker.retrySelectedJoinKeys(); await close;
  assert.equal(linked.counts().settlements, 2);
  assert.equal(linked.counts().admits, 1); assert.equal(linked.counts().commits, 0);
});

test("End while real channel cancellation holds the lane retires an already-admitted queued provider without new SDK", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const linked = await linkedDerivation(f, channel), admission = linked.holdAdmit();
  const attempt = linked.register().attempt(); await until(admission.entered);
  const cancellation = f.stallBody(), reads = f.counts().reads;
  const status = channel.status(); await until(() => f.counts().reads === reads + 1);
  const steps = f.counts().steps.length;
  admission.release(); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.counts().steps.length, steps, "already-admitted provider queues behind actual channel intake");
  f.owner.retire(); assert.equal((await status).kind, "retired"); assert.equal((await attempt).kind, "retired");
  let closed = false; const close = channel.close().then(() => { closed = true; });
  await until(cancellation.entered);
  assert.equal(closed, false); assert.equal(linked.counts().settlements, 0);
  assert.equal(f.counts().settlements, 0); assert.equal(f.counts().collections, 0);
  cancellation.release(); await close;
  assert.equal(f.counts().steps.length, steps, "queued provider performs only its post-acquisition retirement check");
  assert.equal(linked.counts().settlements, 1);
  assert.equal(f.counts().settlements, 1); assert.equal(f.counts().collections, 1);
});

test("channel-linked preparation rejects unready, copied and foreign retained handles before reserving a chain", async t => {
  const f = await fixture(t), channel = f.channel(), linked = await linkedDerivation(f, channel);
  const before = f.broker.counts().bytes;
  assert.throws(linked.plan, /ready voice channel/); assert.equal(f.broker.counts().bytes, before);
  await channel.status();
  const copied = Object.freeze({ ...channel });
  assert.throws(() => f.broker.prepareChainFromChannel(f.owner, copied, f.key, linked.binding, 15000, linked.setup), /ready voice channel/);
  const generic = await f.broker.createJoinKeyHandle(f.owner);
  assert.throws(() => f.broker.prepareChainFromChannel(f.owner, channel, generic, linked.binding, 15000, linked.setup), /ready voice channel/);
  const foreign = createVoiceCryptoOwner(f.owner.token, f.owner.group, () => true);
  assert.throws(() => f.broker.prepareChainFromChannel(foreign, channel, f.key, linked.binding, 15000, linked.setup), /ready voice channel/);
  foreign.retire();
  await channel.close(); assert.throws(linked.plan, /ready voice channel/);
});

test("retained channel derives from actual selected key once, coalesces and advances original counters", async t => {
  const f = await fixture(t), channel = f.channel();
  assert.equal(f.channel(), channel);
  const first = channel.status(); assert.equal(channel.status(), first);
  assert.deepEqual(await first, { kind: "ready", nativeSetupDrained: true });
  const firstSteps = f.counts().steps.length;
  assert.deepEqual(await channel.status(), { kind: "ready", nativeSetupDrained: true });
  assert.equal(f.counts().steps.length - firstSteps, 4, "second status only seals/MACs/verifies/opens");
  assert.deepEqual(f.counts().tx, [1n, 2n]); assert.equal(f.counts().setups, 1); assert.equal(f.counts().ordinary, 0);
  await channel.close(); assert.equal((await channel.status()).kind, "retired");
  assert.equal(f.channel(), channel, "close history cannot reconstruct a Session for this selection");
});

test("production native adapter remount and lost setup reply retain one selected channel and original counters", async t => {
  const f = await fixture(t), host = {};
  const invoke = async <T>(command: string, args: Record<string, unknown>): Promise<T> => {
    assert.deepEqual(Object.keys(args).sort(), ["executor", "owner"]);
    const owner = args.owner as string, executor = args.executor as string;
    if (command === "setup_voice_channel") return await f.native.setup(owner, executor) as T;
    if (command === "settle_voice_channel_setup") return await f.native.settleSetup(owner, executor) as T;
    if (command === "collect_voice_channel") return await f.native.collectChannel(owner, executor) as T;
    throw new Error("unexpected native command");
  };
  const native = windowVoiceIpcNative(invoke, host);
  const channel = f.broker.nativeChannel(f.owner, f.key, native, f.transport);
  f.loseSetup(); assert.equal((await channel.status()).kind, "uncertain");
  assert.deepEqual(f.counts().steps, []);
  const remounted = await import("./voice-ipc-native.ts?remount=session");
  const retained = remounted.windowVoiceIpcNative(invoke, host);
  assert.equal(retained, native);
  assert.equal(f.broker.nativeChannel(f.owner, f.key, retained, f.transport), channel);
  assert.equal((await channel.status()).kind, "ready");
  assert.equal((await channel.status()).kind, "ready");
  assert.deepEqual(f.counts().tx, [1n, 2n]);
  assert.equal(f.counts().setups, 2);
  await channel.close();
  assert.equal(f.counts().settlements, 1); assert.equal(f.counts().collections, 1);
});

test("lost public setup reply retries the original identity before any browser computation", async t => {
  const f = await fixture(t); f.loseSetup(); const channel = f.channel();
  assert.equal((await channel.status()).kind, "uncertain"); assert.deepEqual(f.counts().steps, []);
  assert.equal((await channel.status()).kind, "ready");
  assert.equal(f.counts().setups, 2); assert.deepEqual(f.counts().tx, [1n]);
});

test("End before first status drains proven-unstarted custody without settling an unknown native purpose", async t => {
  const f = await fixture(t), channel = f.channel();
  f.owner.retire(); await channel.close();
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().setups, 0); assert.equal(f.counts().settlements, 0);
  assert.equal(f.counts().collections, 0);
  assert.deepEqual(f.counts().steps, []); assert.equal(f.broker.counts().bytes, 0);
});

test("substituted executor is refused before crypto and exact cleanup fences the original purpose", async t => {
  const f = await fixture(t); f.tamperSetup();
  assert.equal((await f.channel().status()).kind, "failed");
  assert.deepEqual(f.counts().steps, []); assert.equal(f.counts().settlements, 1);
});

test("exact native closed history stays closed metadata and never reconstructs browser keys", async t => {
  const f = await fixture(t); f.closeBeforeSetup(); const channel = f.channel();
  assert.deepEqual(await channel.status(), { kind: "closed", nativeSetupDrained: true });
  assert.deepEqual(f.counts().steps, []); assert.equal(f.counts().settlements, 1);
  assert.equal((await channel.status()).kind, "retired");
});

test("generic same-owner join key cannot construct a browser Session", async t => {
  const f = await fixture(t), generic = await f.broker.createJoinKeyHandle(f.owner);
  assert.throws(() => f.broker.nativeChannel(f.owner, generic, f.native, f.transport), /selected voice key/);
  assert.equal(f.counts().setups, 0);
});

test("failed outgoing encryption burns TX without reconstructing selected keys", async t => {
  const f = await fixture(t); f.failEncrypt(); const channel = f.channel();
  assert.equal((await channel.status()).kind, "uncertain");
  assert.equal((await channel.status()).kind, "ready");
  assert.deepEqual(f.counts().tx, [2n]); assert.equal(f.counts().setups, 1);
  assert.equal(f.counts().steps.filter(step => step === "deriveBits").length, 2);
});

test("authenticated bad body spends RX before intake; rebinding that RX to a fresh request is refused", async t => {
  const f = await fixture(t); f.badBody(); const channel = f.channel();
  assert.equal((await channel.status()).kind, "uncertain"); assert.equal(f.counts().reads, 1);
  f.badBody(false); f.forceSequence(1n);
  assert.equal((await channel.status()).kind, "uncertain");
  assert.equal(f.counts().reads, 1, "replayed authenticated header never reaches body reader");
  f.forceSequence(null); assert.equal((await channel.status()).kind, "ready");
  assert.deepEqual(f.counts().tx, [1n, 2n, 3n]);
});

for (let step = 0; step < 14; step++) test(`End during actual SDK step ${step + 1} retains funding/private key until it settles`, async t => {
  const f = await fixture(t), hold = f.hold(step), channel = f.channel(), work = channel.status();
  await until(hold.entered);
  const before = f.broker.counts(); f.owner.retire();
  assert.equal((await work).kind, "retired");
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, before.bytes);
  assert.equal(f.counts().settlements, 0);
  hold.release(); await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().steps.length, step + 1, "retirement cannot launch another SDK step");
  assert.equal(f.counts().settlements, 1); assert.equal(f.broker.counts().bytes, 0);
});

test("End waits for actual body cancellation before exact native cleanup or local refunds", async t => {
  const f = await fixture(t), cancellation = f.stallBody(), channel = f.channel();
  const pending = channel.status(); await until(() => f.counts().reads === 1);
  const before = f.broker.counts(); f.owner.retire();
  assert.equal((await pending).kind, "retired"); await until(cancellation.entered);
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, before.bytes);
  assert.equal(f.counts().settlements, 0);
  f.advance(2_000); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.counts().settlements, 0, "cleanup cadence cannot run ahead of actual read cancellation");
  assert.equal(f.timers().pending, 1, "only the original five-second IO deadline remains");
  cancellation.release(); await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().settlements, 1);
});

test("lost cleanup reply retains position and funding until availability wake, without new crypto", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status(); f.loseSettle();
  const steps = f.counts().steps.length, before = f.broker.counts().bytes;
  let done = false; const close = channel.close().then(() => { done = true; });
  await until(() => f.counts().settlements === 1); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(done, false); assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, before);
  f.broker.retrySelectedJoinKeys(); await close;
  assert.equal(f.counts().steps.length, steps); assert.equal(f.counts().settlements, 2);
});

test("lost final collection reply retains original custody and retries only that exact final operation", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status(); f.loseCollection();
  const before = f.broker.counts().bytes, steps = f.counts().steps.length;
  let done = false; const closed = channel.close().then(() => { done = true; });
  await until(() => f.counts().collections === 1); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(done, false); assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, before);
  f.broker.retrySelectedJoinKeys(); await closed;
  assert.equal(f.counts().collections, 2); assert.equal(f.counts().settlements, 1);
  assert.equal(f.counts().steps.length, steps, "lost final reply never re-derives, seals or reopens");
});

test("cleanup cadence retries a lost setup-close reply without availability or another SDK operation", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status(); f.loseSettle();
  const before = f.counts(), paid = f.broker.counts().bytes;
  let closed = false; const close = channel.close().then(() => { closed = true; });
  await until(() => f.timers().pending === 1);
  assert.equal(f.counts().settlements, 1); assert.equal(f.counts().collections, 0);
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, paid);
  f.advance(999); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.equal(f.counts().settlements, 1);
  f.advance(1); await close; await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().settlements, 2); assert.equal(f.counts().collections, 1);
  assert.deepEqual(f.counts().steps, before.steps); assert.deepEqual(f.counts().tx, before.tx);
  assert.equal(f.counts().setups, before.setups); assert.equal(f.timers().pending, 0);
  assert.equal(f.timers().peak, 1);
  f.owner.retire(); assert.equal(f.broker.counts().bytes, 0);
});

test("cleanup cadence retries only final collection after its reply is lost", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status(); f.loseCollection();
  const before = f.counts(), paid = f.broker.counts().bytes;
  const close = channel.close(); await until(() => f.timers().pending === 1);
  assert.equal(f.counts().settlements, 1); assert.equal(f.counts().collections, 1);
  assert.equal(f.broker.counts().bytes, paid);
  f.advance(1_000); await close; await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().settlements, 1); assert.equal(f.counts().collections, 2);
  assert.deepEqual(f.counts().steps, before.steps); assert.deepEqual(f.counts().tx, before.tx);
  assert.equal(f.timers().pending, 0); assert.equal(f.timers().peak, 1);
});

test("cleanup early timer firing rearms only the remainder of the original interval", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status(); f.loseSettle();
  const close = channel.close(); await until(() => f.timers().pending === 1);
  f.advance(400); f.fireTimerEarly(); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.counts().settlements, 1); assert.equal(f.timers().pending, 1);
  assert.equal(f.timers().peak, 1);
  f.advance(599); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.counts().settlements, 1);
  f.advance(1); await close; await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().settlements, 2); assert.equal(f.timers().pending, 0);
});

test("cleanup availability and stale cancelled timer callbacks cannot duplicate or wake a later retry", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status(); f.loseSettle();
  const close = channel.close(); await until(() => f.timers().pending === 1);
  const stale = f.timers().callbacks[0];
  f.loseSettle(); f.broker.retrySelectedJoinKeys();
  await until(() => f.counts().settlements === 2 && f.timers().pending === 1);
  stale(); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.counts().settlements, 2); assert.equal(f.timers().pending, 1);
  f.advance(999); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.counts().settlements, 2);
  f.advance(1); await close; await until(() => f.broker.counts().active === 0);
  stale(); assert.equal(f.counts().settlements, 3); assert.equal(f.timers().pending, 0);
  assert.equal(f.timers().peak, 1);
});

test("cleanup consumes availability during a refused RPC and during timer installation", async t => {
  for (const duringSchedule of [false, true]) {
    const f = await fixture(t), channel = f.channel(); await channel.status();
    const original = f.native.settleSetup; let calls = 0;
    f.cleanup(() => { f.native.settleSetup = original; });
    f.native.settleSetup = async (...args) => {
      if (++calls === 1) {
        if (duringSchedule) f.onSchedule(() => f.broker.retrySelectedJoinKeys());
        else f.broker.retrySelectedJoinKeys();
        throw new Error("original native state temporarily busy");
      }
      await original(...args);
    };
    await channel.close(); await until(() => f.broker.counts().active === 0);
    assert.equal(calls, 2); assert.equal(f.timers().pending, 0);
    assert.equal(f.timers().peak, 1);
  }
});

test("cleanup repeated refusal owns one timer and a large clock jump admits only one retry", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const before = f.counts(), paid = f.broker.counts().bytes;
  const original = f.native.settleSetup; let calls = 0;
  f.cleanup(() => { f.native.settleSetup = original; });
  f.native.settleSetup = async (...args) => { if (++calls <= 4) throw new Error("native capacity"); await original(...args); };
  const close = channel.close(); await until(() => f.timers().pending === 1);
  for (let expected = 2; expected <= 4; expected++) {
    f.advance(100_000); await until(() => calls === expected && f.timers().pending === 1);
    await new Promise<void>(yes => setImmediate(yes));
    assert.equal(calls, expected, "no catch-up loop for missed intervals");
    assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, paid);
    assert.equal(f.timers().pending, 1); assert.equal(f.timers().peak, 1);
  }
  f.advance(999); await new Promise<void>(yes => setImmediate(yes)); assert.equal(calls, 4);
  f.advance(1); await close; await until(() => f.broker.counts().active === 0);
  assert.equal(calls, 5); assert.equal(f.timers().pending, 0);
  assert.deepEqual(f.counts().steps, before.steps); assert.deepEqual(f.counts().tx, before.tx);
});

test("cleanup never arms its cadence or overlaps a still-pending native RPC", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const held = gate(), original = f.native.settleSetup; let calls = 0;
  f.cleanup(() => { held.release(); f.native.settleSetup = original; });
  f.native.settleSetup = async (...args) => {
    if (++calls === 1) { await held.promise; throw new Error("native reply lost after delayed completion"); }
    await original(...args);
  };
  let done = false; const close = channel.close().then(() => { done = true; }); await until(() => calls === 1);
  f.advance(100_000); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(calls, 1); assert.equal(f.timers().pending, 0); assert.equal(done, false);
  held.release(); await until(() => f.timers().pending === 1);
  f.advance(1_000); await close; await until(() => f.broker.counts().active === 0);
  assert.equal(calls, 2); assert.equal(f.timers().pending, 0);
});

test("a channel waiting for cleanup cadence does not prevent a different group channel progressing", async t => {
  const first = await fixture(t), firstChannel = first.channel(); await firstChannel.status();
  // Create the second selected key before cleanup owns its reserved first position.
  const second = await fixture(t, first.broker, 2), secondChannel = second.channel();
  first.loseSettle(); const close = firstChannel.close(); await until(() => first.timers().pending === 1);
  assert.equal(first.timers().peak, 1, "the first channel owns only its one cleanup timer");
  assert.equal((await secondChannel.status()).kind, "ready");
  first.advance(1_000); await close; await secondChannel.close();
  await until(() => first.broker.counts().active === 0);
  assert.equal(first.timers().pending, 0);
  assert.equal(first.timers().peak, 2, "one cleanup timer and the other channel's original IO deadline");
  first.owner.retire(); second.owner.retire();
  assert.equal(first.broker.counts().bytes, 0);
});

test("occupied browser positions reject channel before native setup, then the original handle can proceed", async t => {
  const f = await fixture(t), hold = gate();
  let started = 0;
  const a = f.broker.run(f.owner, "derive", 1, async () => { started++; await hold.promise; });
  const b = f.broker.run(f.owner, "derive", 1, async () => { started++; await hold.promise; });
  await until(() => started === 2);
  assert.throws(() => f.channel(), /capacity/); assert.equal(f.counts().setups, 0);
  hold.release(); await Promise.all([a, b]); await until(() => f.broker.counts().active === 0);
  assert.equal((await f.channel().status()).kind, "ready");
});

test("response reads no untrusted HTTP header values before fixed BYOB prefix authentication", async t => {
  const f = await fixture(t); f.hostileHeaders();
  assert.equal((await f.channel().status()).kind, "ready");
});

test("absolute IO deadline aborts stale-port fetch but retains custody until actual cancellation settles", async t => {
  const f = await fixture(t), hung = f.hangFetch(), channel = f.channel();
  const work = channel.status(); await until(hung.entered);
  const charged = f.broker.counts().bytes;
  f.advance(4999); assert.equal(hung.aborted(), false);
  f.advance(1); assert.equal(hung.aborted(), true);
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, charged);
  hung.release(); assert.equal((await work).kind, "uncertain");
  assert.equal((await channel.status()).kind, "ready"); assert.deepEqual(f.counts().tx, [2n]);
});

for (const version of [7, 8, 9, 10, 11, 12, 13]) test(`HMR refuses v${version} retained pool rather than creating a second channel allocator`, () => {
  const host = { [Symbol.for("mewtual.voice.crypto-broker.v2")]: { version } };
  const native = { async admit() { return ""; }, async settle() {}, async commitInbound() {} };
  assert.throws(() => windowVoiceCryptoBroker(native, host, real), /reload required/);
});

test("history cannot construct a session and rejects substituted owner/handle before owner callbacks", async t => {
  const f = await fixture(t), channel = f.channel(), before = f.counts();
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), { kind: "busy" });
  assert.deepEqual(f.counts(), before);
  let callbacks = 0;
  const foreign = createVoiceCryptoOwner(f.owner.token, f.owner.group, () => { callbacks++; return true; });
  assert.throws(() => f.broker.recoverChannelHistory(foreign, channel, historyPurpose), /identity changed/);
  assert.throws(() => f.broker.recoverChannelHistory(f.owner, { ...channel }, historyPurpose), /identity changed/);
  assert.equal(callbacks, 0); assert.deepEqual(f.counts(), before);
  assert.equal((await channel.status()).kind, "ready");
  assert.equal((await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose)).kind, "status");
  assert.equal(f.counts().setups, 1);
});

test("history uses original keys/counters, coalesces the same promise and never queues a competing read", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const before = f.counts(), charged = f.broker.counts().bytes;
  const held = f.hold(before.steps.length), mutable = { ...historyPurpose };
  const request = f.broker.recoverChannelHistory(f.owner, channel, mutable);
  mutable.peer = hash(55);
  await until(held.entered);
  assert.equal(f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), request);
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, mutable), { kind: "busy" });
  const row = historyVector("derived-drained").expected;
  assert.equal(row.kind, "status"); if (row.kind !== "status") throw new Error("fixture");
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose, row.row), { kind: "busy" });
  assert.deepEqual(await channel.status(), { kind: "uncertain" });
  assert.equal(f.counts().steps.length, before.steps.length + 1);
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, charged);
  held.release(); assert.deepEqual(await request, row);
  assert.equal(f.counts().steps.length - before.steps.length, 4);
  assert.deepEqual(f.counts().steps.slice(before.steps.length), ["encrypt", "sign", "verify", "decrypt"]);
  assert.deepEqual(f.operations(), [9, 8]); assert.deepEqual(f.counts().tx, [1n, 2n]);
  assert.deepEqual(f.bodyReads(), [19, 258]); assert.equal(f.counts().ordinary, 0);
  assert.equal(f.counts().setups, 1); assert.equal(f.broker.counts().bytes, charged);
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose, row.row), row);
  assert.deepEqual(f.counts().tx, [1n, 2n, 3n]);
});

test("a pending status leaves history busy without reserving a waiter or starting another driver", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const hold = f.hold(f.counts().steps.length), status = channel.status(); await until(hold.entered);
  const before = f.counts(), charged = f.broker.counts().bytes;
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), { kind: "busy" });
  assert.deepEqual(f.counts(), before); assert.equal(f.broker.counts().bytes, charged);
  hold.release(); assert.equal((await status).kind, "ready");
  assert.deepEqual(f.operations(), [9, 9]);
});

for (const step of [1, 3]) test(`history coalesces after plaintext zeroing while SDK step ${step + 1} is parked`, async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const held = f.hold(f.counts().steps.length + step);
  const first = f.broker.recoverChannelHistory(f.owner, channel, historyPurpose); await until(held.entered);
  // Encrypt has already consumed and zeroed query.body. Coalescing must use only the
  // retained canonical purpose, not the now-zero bytes or a newly allocated request.
  const before = f.counts();
  assert.equal(f.broker.recoverChannelHistory(f.owner, channel, { ...historyPurpose }), first);
  assert.deepEqual(f.counts(), before); assert.equal(f.broker.counts().active, 1);
  held.release(); assert.deepEqual(await first, historyVector("derived-drained").expected);
});

for (const item of historyVectors.refusedCases) test(`authenticated history refusal ${item.name} cannot close or refund the channel`, async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const charged = f.broker.counts().bytes; f.historyReply(item.name);
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), item.expected);
  assert.equal(f.broker.counts().bytes, charged); assert.equal(f.counts().settlements, 0);
  assert.equal(f.counts().collections, 0); assert.equal((await channel.status()).kind, "ready");
  assert.deepEqual(f.operations(), [9, 8, 9]); assert.equal(f.counts().setups, 1);
});

test("malformed authenticated history spends RX; a fresh retry never reconstructs the session", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const malformed = Buffer.from(historyVector("derived-drained").plaintext, "hex"); malformed[209] = 2;
  f.historyBytes(malformed);
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), { kind: "uncertain" });
  assert.deepEqual(f.bodyReads(), [19, 258]);
  f.historyReply("derived-drained"); f.forceSequence(2n);
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), { kind: "uncertain" });
  assert.deepEqual(f.bodyReads(), [19, 258], "authenticated replay is refused before body intake");
  f.forceSequence(null);
  const outcome = historyVector("derived-drained").expected;
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), outcome);
  assert.equal(f.counts().setups, 1); assert.deepEqual(f.counts().tx, [1n, 2n, 3n, 4n]);
  assert.equal(f.counts().steps.filter(step => step === "deriveBits").length, 2);
});

test("an authenticated history from another native row is rejected without becoming current history", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const row = historyVector("derived-drained").expected;
  assert.equal(row.kind, "status"); if (row.kind !== "status") throw new Error("fixture");
  const wrongRow = "00000000000000010000000000000001";
  assert.notEqual(wrongRow, row.row);
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose, wrongRow), { kind: "uncertain" });
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose, row.row), row);
  assert.equal(f.counts().setups, 1);
});

for (const [name, change] of Object.entries({
  operation: { operation: 9 }, logical: { logical: "1".repeat(32) }, slot: { slot: 2 },
  direction: { direction: 0 }, length: { length: 257 },
} satisfies Record<string, Partial<VoiceIpcHeader>>)) test(`wrong history response ${name} is refused before body intake`, async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  f.responseChange(change);
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), { kind: "uncertain" });
  assert.deepEqual(f.bodyReads(), [19]);
  f.responseChange(null);
  assert.equal((await f.broker.recoverChannelHistory(f.owner, channel, historyPurpose)).kind, "status");
});

for (let index = 0; index < 4; index++) test(`End during history SDK step ${index + 1} retains the original flight until actual completion`, async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const before = f.counts().steps.length, held = f.hold(before + index);
  const request = f.broker.recoverChannelHistory(f.owner, channel, historyPurpose); await until(held.entered);
  const charged = f.broker.counts().bytes; f.owner.retire();
  assert.deepEqual(await request, { kind: "retired" });
  let closed = false; const close = channel.close().then(() => { closed = true; });
  assert.equal(closed, false); assert.equal(f.broker.counts().bytes, charged);
  assert.equal(f.broker.counts().active, 1); assert.equal(f.counts().settlements, 0);
  held.release(); await close;
  assert.equal(f.counts().steps.length, before + index + 1);
  assert.equal(f.counts().setups, 1); assert.equal(f.counts().settlements, 1);
  assert.equal(f.counts().collections, 1);
  // close observes native cleanup; the enclosing original driver's finally still owns
  // the paid Control position until it actually returns. Never equate that to a refund.
  await until(() => f.broker.counts().active === 0); assert.equal(f.broker.counts().bytes, 0);
  assert.throws(() => f.channel(), /owner changed/, "retired selection cannot create/recover a session");
});

test("End cannot refund history while actual BYOB cancellation still owns its buffers", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const cancellation = f.stallBody(), before = f.counts().reads;
  const request = f.broker.recoverChannelHistory(f.owner, channel, historyPurpose);
  await until(() => f.counts().reads === before + 1);
  const charged = f.broker.counts().bytes; f.owner.retire();
  assert.deepEqual(await request, { kind: "retired" }); await until(cancellation.entered);
  assert.equal(f.broker.counts().bytes, charged); assert.equal(f.broker.counts().active, 1);
  assert.equal(f.counts().settlements, 0);
  cancellation.release(); await channel.close();
  await until(() => f.broker.counts().active === 0); assert.equal(f.broker.counts().bytes, 0);
});

test("history shares a lent provider's one physical lane without ordinary admission", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const linked = await linkedDerivation(f, channel), action = linked.register();
  const held = f.hold(f.counts().steps.length + 5), attempt = action.attempt(); await until(held.entered);
  const before = f.counts(), request = f.broker.recoverChannelHistory(f.owner, channel, historyPurpose);
  assert.equal(f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), request);
  await new Promise<void>(yes => setImmediate(yes));
  assert.deepEqual(f.counts(), before); assert.equal(f.broker.counts().active, 1);
  held.release(); assert.equal((await request).kind, "status");
  // The fixture's commit RPC asks for Status. It deliberately collides with the queued
  // history flight and is uncertain; retry reuses the original cached provider output.
  assert.equal((await attempt).kind, "uncertain");
  const providerSteps = f.counts().steps.filter(step => step === "deriveBits").length;
  assert.equal((await action.attempt()).kind, "accepted");
  assert.equal(f.counts().steps.filter(step => step === "deriveBits").length, providerSteps);
  assert.equal(f.counts().ordinary, 0);
});

test("a reentrant purpose getter that retires the owner cannot admit history", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const before = f.counts();
  const purpose = { ...historyPurpose, get peer() { f.owner.retire(); return historyPurpose.peer; } };
  assert.deepEqual(await f.broker.recoverChannelHistory(f.owner, channel, purpose), { kind: "retired" });
  await channel.close();
  assert.deepEqual(f.counts().steps, before.steps); assert.deepEqual(f.counts().tx, before.tx);
  assert.equal(f.counts().setups, 1);
});

test("the bounded history graph fits the original 8KiB Control payment with its existing wrappers", () => {
  // Structural payload accounting, NOT an assertion about arbitrary JS/SDK heap sizes or
  // caller-retained replies. One paid flight owns at most these concrete bounded copies.
  // Reserving half the 8KiB for promises/closures/records/typed-array wrappers deliberately
  // leaves slack; changes to the buffers/grammar/retained fields must update this inventory.
  const strings = (value: unknown): number => {
    if (typeof value === "string") return value.length * 2;
    if (value && typeof value === "object") return Object.entries(value).reduce((n, [key, field]) => n + key.length * 2 + strings(field), 0);
    return 8;
  };
  const request = encodeVoiceIpcAction(historyPurpose, { kind: "begin", retry: false });
  const largestReply = Math.max(...historyVectors.statusCases.map(item => strings(item.expected)));
  // Request body+cipher, header, MAC, nonce; response prefix+admission+decoded header/tag,
  // response cipher+opened body, EOF byte, response AAD and nonce. Include purpose/row,
  // native metadata and one immutable request+response header record with string fields.
  const buffers = request.body.length + request.body.length + 16 + 40 + 32 + 12 +
    144 + 288 + 40 + 32 + 258 + 242 + 1 + 80 + 12;
  // The original driver's cleanup wait owns one timer handle/callback/deadline and ONE
  // existing Signal, never Promise.race waiters. Add a structural wrapper allowance; this
  // remains no claim about implementation-specific SDK/timer/Promise heap allocation.
  const cleanupWait = 256;
  const graph = buffers + strings(historyPurpose) + 64 + largestReply + 2 * (64 + 10 * 8) + 4096 + cleanupWait;
  assert.ok(graph <= 8192, `history payload/wrapper envelope ${graph}/8192`);
  // One query slot in the original driver, not a new map or a fresh ordinary permit.
  const source = readFileSync(new URL("./voice-crypto.ts", import.meta.url), "utf8");
  assert.match(source, /const IPC_CONTROL_BYTES = 8 \* 1024;/);
  assert.match(source, /private historyRequest: IpcHistoryRequest \| null = null;/);
  assert.equal(request.body.length, 106);
});

test("Derive intake and terminal Control payload graphs fit their separate original prepayments", () => {
  // Structural bounds on application-owned buffers/wrappers, not arbitrary SDK/GC heaps.
  // Fixed chain/row metadata is paid separately. Input/compute are serial on ONE lane;
  // this sum is deliberately more conservative than their nonoverlapping live payloads.
  const transcript = 1024 - utf8.encode("mewtual/voice/v2/aes-gcm/high-to-low\0").length;
  const input = 541 + transcript;
  const request = 187; // largest Derive request: exact empty result, no extra output copy
  const framing = 2 * (request + 16) + 2 * (input + 16) + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const decodedAndAdopted = 3 * (transcript + 2 * 65);
  const engine = 3 * transcript + 4096 + 256;
  const variable = framing + decodedAndAdopted + engine + 16 * 1024;
  assert.ok(variable <= 96 * 1024, `Derive data/engine/wrapper envelope ${variable}/98304`);
  // Original purpose/row/action strings, one stored final proof, native bootstrap, decoded
  // fixed status/final metadata and wrappers. No history query is live during terminal
  // cleanup; the decoded action graph ends before reading the separate final proof.
  const controlBuffers = 2 * (185 + 16) + 2 * (242 + 16) + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const descriptor = 2 * (3 * 64 + 20 + 32 + 2 * 20 + 64 + 64);
  const decoded = 2 * 1024; // bounded fixed status/final graph, including exact-u64 strings
  const wrappers = 3 * 1024 + 2 * "retire-plan".length; // retained fixed final-mode discriminator
  const control = controlBuffers + descriptor + decoded + wrappers;
  assert.ok(control <= 8 * 1024, `terminal Control payload/wrapper envelope ${control}/8192`);
});
