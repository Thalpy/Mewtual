import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { webcrypto, createHash } from "node:crypto";
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
import { encodeVoiceIpcAction, encodeVoiceIpcConfirmationAction, type VoiceIpcPurpose, type VoiceIpcConfirmationPurpose } from "./voice-ipc-actions.ts";
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
  type ActionResponse = { kind: 0 | 1; body: Uint8Array };
  let actionResponder: ((header: VoiceIpcHeader, opened: Uint8Array) => ActionResponse | Promise<ActionResponse>) | null = null;
  let responseChange: Partial<VoiceIpcHeader> | null = null;
  let holdAt = -1, held: ReturnType<typeof gate> | null = null, engineEnabled = false;
  let holdWhen: ((name: string, args: unknown[]) => boolean) | null = null;
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
        if (holdWhen?.(String(name), args)) { holdWhen = null; observedStep = -2; await held!.promise; }
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
    } else if (request.header.operation === 8 && opened[0] !== 2) {
      assert.equal(request.header.operation, 8); assert.equal(request.header.kind, 0);
      assert.equal(request.header.logical, "0".repeat(32)); assert.equal(request.header.length, 122);
      assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "begin", retry: false }).body);
    } else {
      assert.ok(actionResponder, "unexpected actual framed action");
      actionReply = await actionResponder(request.header, opened);
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
    rowBudget() {
      // Read-only unit observation of actual retained claims. Public counts intentionally
      // expose no capability/history internals to application callers.
      const state = broker as unknown as { reservedRows: number; actions: Map<unknown, unknown> };
      return { reserved: state.reservedRows, actions: state.actions.size };
    },
    hold(index: number) { holdAt = index; held = gate(); return { release: held.release, entered: () => observedStep === index }; },
    holdWhen(predicate: NonNullable<typeof holdWhen>) { holdWhen = predicate; held = gate(); return { release: held.release, entered: () => observedStep === -2 }; },
    loseSetup() { loseSetup = true; }, loseSettle() { loseSettle = true; }, tamperSetup() { tamperSetup = true; },
    closeBeforeSetup() { setupClosed = true; },
    loseCollection() { loseCollection = true; },
    failEncrypt() { failEncrypt = true; }, badBody(value = true) { badBody = value; }, forceSequence(value: bigint | null) { forceSequence = value; },
    stallBody() { stalled = true; cancelGate = gate(); return { entered: () => cancelEntered, release: cancelGate.release }; },
    clearBodyStall() { stalled = false; },
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
  let childResponder: Parameters<typeof f.actions>[0] = null;
  let settleHold: ReturnType<typeof gate> | null = null;
  let checkFinal: (() => void) | null = null;
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
  f.cleanup(() => { finalPending = false; loseIssuance = false; loseFinalAck = false; settleHold?.release(); f.native.settleSetup = nativeSettle; });
  f.actions(async (header, opened) => {
    if (opened[0] === 2) {
      assert.ok(childResponder, "unexpected child action");
      return childResponder(header, opened);
    }
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
        if (opened[105] === 0) {
          assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "close", mode: "return" }).body);
          ordinaryReturned = true; return status();
        }
        checkFinal?.();
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
        checkFinal?.();
        finalCalls++; assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "final", proof: finalMode === 2 ? planProof : proof }).body);
        assert.equal(finalPending, false); finalAcknowledged = true;
        if (loseFinalAck) { loseFinalAck = false; throw new Error("actual native final ACK reply lost"); }
      }
      const body = Buffer.concat([Buffer.from([1, 4]), Buffer.from(row, "hex"), integer(BigInt(identity.action), 8), integer(1n, 8),
        Buffer.from(identity.inputTag, "hex"), Buffer.from([replyMode ?? finalMode, finalPending ? 0 : finalAcknowledged ? 2 : 1]),
        Buffer.from(finalPending ? "0".repeat(64) : finalMode === 2 ? planProof : proof, "hex")]);
      return { kind: 0, body };
    }
    assert.equal(header.operation, 5); settlements++; if (settleHold) await settleHold.promise;
    settled = true; return status();
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
    wasReturned: () => ordinaryReturned,
    childReplies(responder: typeof childResponder) { childResponder = responder; },
    holdSettlement() { settleHold = gate(); return settleHold; },
    beforeFinal(check: () => void) { checkFinal = check; },
  };
}

/** Mock native signer/Root only; actual original-channel framing, P256/HKDF/AEAD, bounded
 * stream reads, broker ownership and native-action adapters execute unchanged. */
async function framedConfirmation(f: Awaited<ReturnType<typeof fixture>>) {
  const channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f);
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity).attempt();
  assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original chain required");
  let row = "00000000000000010000000000000001", identity = { action: "9007199254741027", version: "3", inputTag: hash(91) };
  let purpose: VoiceIpcConfirmationPurpose = { parent: historyPurpose, parentRow: parent.row, request: { kind: "start", predecessor: null } };
  const aad = utf8.encode("native signed Hello header"), body = utf8.encode("native closed Hello");
  const engineBytes = 4 * (aad.length + body.length + 16) + 3 * 512 + 2048;
  const b = parent.binding;
  const data = Buffer.concat([Buffer.from([1, 7]), Buffer.from(row, "hex"), integer(BigInt(identity.action), 8), integer(BigInt(identity.version), 8),
    Buffer.from(identity.inputTag, "hex"), Buffer.from(parent.row, "hex"), Buffer.from(f.owner.group, "hex"), Buffer.from([0]), Buffer.alloc(32), integer(1n, 16), integer(BigInt(b.epoch), 8),
    ...[b.localDevice, b.localJoin, b.localKey, b.peerDevice, b.peerJoin, b.peerKey, b.handshake].map(s => Buffer.from(s, "hex")),
    integer(BigInt(b.generation), 8), integer(BigInt(b.nativeDeadline), 8), Buffer.from([1]), integer(BigInt(engineBytes), 4), Buffer.from([1, 1]),
    integer(1n, 8), integer(BigInt(aad.length), 2), integer(BigInt(body.length), 2), aad, body]);
  assert.equal(data.length, 430 + aad.length + body.length);
  let inputs = 0, admissions = 0, results = 0, acknowledgements = 0, settlements = 0, returns = 0;
  let resultTag: string | null = null, output: Uint8Array | null = null, acknowledged = false, settled = false, returned = false;
  let lostInput = false, lostResult = false, lostReturn = false, historical = false, rejected = false;
  let alter: ((data: Uint8Array) => Uint8Array) | null = null, onInput: (() => void) | null = null;
  let holdSettle: ReturnType<typeof gate> | null = null;
  let beginMode: "attached" | "pending" | "absent" = "attached", lostBegin = false;
  let holdBegin: ReturnType<typeof gate> | null = null, beginAlter: ((data: Uint8Array) => Uint8Array) | null = null;
  let onBegin: (() => void) | null = null;
  const begins: boolean[] = [];
  const receipt = hash(92);
  parent.beforeFinal(() => assert.ok(returned || beginMode === "absent", "parent final cannot overtake original child Return"));
  const status = () => {
    const bytes = Buffer.alloc(242); bytes[0] = 1; Buffer.from(row, "hex").copy(bytes, 2); bytes[19] = 1;
    integer(BigInt(identity.action), 8).copy(bytes, 20); integer(BigInt(identity.version), 8).copy(bytes, 28); Buffer.from(identity.inputTag, "hex").copy(bytes, 36);
    bytes[68] = Number(settled); bytes[69] = Number(resultTag !== null); bytes[208] = Number(acknowledged); bytes[209] = Number(returned);
    if (resultTag !== null) {
      integer(BigInt(identity.action), 8).copy(bytes, 70); Buffer.from(identity.inputTag, "hex").copy(bytes, 78); Buffer.from(resultTag, "hex").copy(bytes, 110);
      bytes[142] = rejected ? 3 : 1;
      if (rejected) bytes[143] = 1;
      else { Buffer.from(hash(93), "hex").copy(bytes, 143); Buffer.from(hash(94), "hex").copy(bytes, 175); }
      bytes[207] = Number(returned); Buffer.from(receipt, "hex").copy(bytes, 210);
    }
    return { kind: 0 as const, body: bytes };
  };
  const pendingStatus = () => {
    const bytes = Buffer.alloc(242); bytes[0] = 1; Buffer.from(row, "hex").copy(bytes, 2); bytes[18] = returned ? 3 : 1;
    return { kind: 0 as const, body: bytes };
  };
  f.cleanup(() => { holdSettle?.release(); holdBegin?.release(); f.clearBodyStall(); });
  parent.childReplies(async (header, opened) => {
    assert.equal(header.logical, header.operation === 8 ? "0".repeat(32) : row); assert.equal(opened[0], 2);
    assert.ok(parent.wasReturned(), "native child use follows actual parent Return");
    const exact = (request: Parameters<typeof encodeVoiceIpcConfirmationAction>[1]) => assert.deepEqual(opened, encodeVoiceIpcConfirmationAction(purpose, request).body);
    if (header.operation === 8) {
      const retry = opened.at(-1) === 1; exact({ kind: "begin", retry }); begins.push(retry);
      assert.deepEqual(f.rowBudget(), { reserved: retry ? 11 : 1, actions: 1 },
        "original descriptor claim precedes Begin; End refunds only unused rows");
      if (holdBegin) await holdBegin.promise;
      onBegin?.();
      if (lostBegin) { lostBegin = false; throw new Error("native Begin admitted; reply lost"); }
      if (beginMode === "absent") return { kind: 0, body: new Uint8Array([1, 3, 5]) };
      const reply = beginMode === "pending" ? pendingStatus() : status();
      return beginAlter ? { ...reply, body: beginAlter(reply.body) } : reply;
    }
    if (beginMode === "pending") {
      assert.equal(header.operation, 6, "Pending has no action to admit, compute or settle");
      exact({ kind: "close", mode: "return" }); returns++; returned = true; return pendingStatus();
    }
    if (header.operation === 1) {
      inputs++; exact({ kind: "input" }); onInput?.();
      assert.ok(f.broker.counts().bytes >= 128 * 1024, "same original envelope paid before Data read");
      if (lostInput) { lostInput = false; throw new Error("native Input reply lost"); }
      return historical ? status() : { kind: 1, body: alter ? alter(new Uint8Array(data)) : data };
    }
    if (header.operation === 2) {
      admissions++; exact({ kind: "admit", identity, engineBytes });
      return { kind: 0, body: Buffer.concat([Buffer.from([1, 6]), Buffer.from(row, "hex"), integer(BigInt(identity.action), 8),
        integer(BigInt(identity.version), 8), Buffer.from(identity.inputTag, "hex"), integer(BigInt(identity.action), 8), integer(BigInt(engineBytes), 4)]) };
    }
    if (header.operation === 3) {
      results++;
      // Start(None) has the122-byte selector; result suffix is identity48+tag32+len2.
      const next = opened.slice(204), tag = Buffer.from(opened.subarray(170, 202)).toString("hex");
      assert.equal(createHash("sha256").update(next).digest("hex"), tag);
      exact({ kind: "result", identity, tag, output: next });
      if (output) assert.deepEqual(next, output, "lost replies retry the same provider output");
      else { output = next; resultTag = tag; }
      if (lostResult) { lostResult = false; throw new Error("native result committed; reply lost"); }
      return status();
    }
    if (header.operation === 4) { acknowledgements++; exact({ kind: "ack", identity, digest: receipt }); acknowledged = true; return status(); }
    if (header.operation === 5) {
      settlements++; exact({ kind: "settle", identity }); if (holdSettle) await holdSettle.promise; settled = true; return status();
    }
    assert.equal(header.operation, 6); returns++; exact({ kind: "close", mode: "return" });
    assert.ok(settled && (resultTag === null || acknowledged)); returned = true;
    if (lostReturn) { lostReturn = false; throw new Error("native Return completed; reply lost"); }
    return status();
  });
  return { channel, parent, ready, purpose, row, identity,
    handle: () => f.broker.nativeConfirmationFromChannel(f.owner, channel, ready.chain, purpose, row, identity),
    begin: () => f.broker.nativeConfirmationFromChannel(f.owner, channel, ready.chain, purpose),
    begins: () => [...begins],
    beginMode(value: typeof beginMode) { beginMode = value; },
    alterBegin(value: typeof beginAlter) { beginAlter = value; },
    onBegin(work: typeof onBegin) { onBegin = work; },
    loseBegin() { lostBegin = true; },
    holdBegin() { holdBegin = gate(); return holdBegin; },
    derive: () => ready.derivation.attempt(),
    counts: () => ({ inputs, admissions, results, acknowledgements, settlements, returns }),
    output: () => output,
    loseInput() { lostInput = true; }, loseResult() { lostResult = true; }, loseReturn() { lostReturn = true; },
    reject() { rejected = true; },
    historical() { historical = true; resultTag = hash(95); },
    alter(work: typeof alter) { alter = work; }, onInput(work: typeof onInput) { onInput = work; },
    holdSettlement() { holdSettle = gate(); return holdSettle; },
    nextHistorical() {
      assert.ok(returned); const next = BigInt(identity.action) + 1n;
      row = (BigInt("0x" + row.slice(0, 16)) + 1n).toString(16).padStart(16, "0") + "0000000000000001";
      identity = { action: next.toString(), version: "3", inputTag: hash(Number(next % 1000n)) };
      purpose = { ...purpose, request: { kind: "start", predecessor: hash(Number(next % 1000n)) } };
      returned = false; settled = false; acknowledged = false; historical = true; resultTag = hash(95);
      return f.broker.nativeConfirmationFromChannel(f.owner, channel, ready.chain, purpose, row, identity);
    },
  };
}

test("known child uses original derived keys through framed Input, permit, result and Return", async t => {
  const f = await fixture(t), child = await framedConfirmation(f);
  assert.equal((await child.derive()).kind, "accepted");
  const handle = child.handle(), baseline = f.broker.counts().bytes, pair = child.ready.derivation.pair();
  assert.deepEqual(f.rowBudget(), { reserved: 11, actions: 1 });
  assert.equal(child.handle(), handle);
  const before = f.counts().steps.length, outcome = await handle.attempt();
  assert.equal(outcome.kind, "accepted");
  assert.equal(f.broker.counts().bytes, baseline, "no new96KiB envelope or row reservation");
  assert.equal(child.ready.derivation.pair(), pair, "ordinary Return preserves the same pair");
  assert.deepEqual(f.rowBudget(), { reserved: 10, actions: 2 }, "one original descriptor claim transfers into one actual record");
  assert.ok(child.output());
  assert.deepEqual(child.counts(), { inputs: 1, admissions: 1, results: 1, acknowledgements: 1, settlements: 2, returns: 1 });
  assert.equal(f.counts().steps.slice(before).filter(name => name === "deriveBits" || name === "deriveKey").length, 0);
  const tx = f.counts().tx; assert.ok(tx.every((value, index) => index === 0 || value > tx[index - 1]));
  assert.throws(() => f.broker.nativeConfirmationFromChannel(f.owner, child.channel, child.ready.chain, child.purpose, child.row,
    { ...child.identity, version: "4" }), /identity changed/);
});

test("first child Begin publishes one original claim and coalesces through authenticated adoption", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  const baseline = f.broker.counts().bytes, handle = child.begin();
  assert.equal(child.begin(), handle); assert.deepEqual(f.rowBudget(), { reserved: 11, actions: 1 });
  const pending = handle.attempt(); assert.equal(handle.attempt(), pending);
  assert.equal((await pending).kind, "accepted"); assert.deepEqual(child.begins(), [true]);
  assert.equal(child.handle(), handle, "authenticated adoption retains the same descriptor/handle");
  assert.equal(f.broker.counts().bytes, baseline); assert.deepEqual(f.rowBudget(), { reserved: 10, actions: 2 });
});

test("lost Begin retries the same purpose without replacing its original pair or deadline", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive(); child.loseBegin();
  const pair = child.ready.derivation.pair(), handle = child.begin(), baseline = f.broker.counts().bytes;
  assert.equal((await handle.attempt()).kind, "uncertain"); assert.equal(child.counts().inputs, 0);
  assert.equal((await handle.attempt()).kind, "accepted"); assert.deepEqual(child.begins(), [true, true]);
  assert.equal(child.ready.derivation.pair(), pair); assert.equal(f.broker.counts().bytes, baseline);
  assert.equal(child.counts().admissions, 1); assert.equal(child.counts().results, 1);
});

test("Pending Begin has no invented action and later adopts only its original authenticated row", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive(); child.beginMode("pending");
  const handle = child.begin(), pending = await handle.attempt();
  assert.equal(pending.kind, "status"); if (pending.kind !== "status") throw new Error("actual Pending required");
  assert.equal(pending.pending, "open"); assert.equal(pending.identity, null);
  assert.equal(child.counts().inputs, 0); assert.equal(child.counts().admissions, 0);
  child.beginMode("attached"); assert.equal((await handle.attempt()).kind, "accepted");
  assert.deepEqual(child.begins(), [true, true]); assert.deepEqual(f.rowBudget(), { reserved: 10, actions: 2 });
});

for (const mode of ["attached", "pending"] as const) test(`lost Begin ${mode} after End uses history-only recovery and exact original Return`, async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive(); child.beginMode(mode); child.loseBegin();
  const handle = child.begin(); assert.equal((await handle.attempt()).kind, "uncertain");
  child.parent.pendingFinal(); f.owner.retire(); let closed = false;
  const closure = child.channel.close().then(() => { closed = true; });
  await until(() => child.parent.planCalls() === 1 && f.timers().delays.includes(1000));
  assert.equal(closed, false); assert.deepEqual(child.begins(), [true, false]);
  assert.equal(child.counts().inputs, 0); assert.equal(child.counts().admissions, 0); assert.equal(child.counts().returns, 1);
  assert.equal(child.counts().settlements, mode === "attached" ? 1 : 0);
  assert.deepEqual(f.rowBudget(), { reserved: 1, actions: 1 }, "unknown intake cannot refund its fixed slot before parent final/unlink");
  child.parent.pendingFinal(false); f.advance(1000); await closure;
  await until(() => f.broker.counts().bytes === 0); assert.deepEqual(f.rowBudget(), { reserved: 0, actions: 0 });
});

test("unknown absent Begin retains its claim through refused lookup, Pending Plan-final and lost final ACK", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive(); child.beginMode("absent");
  const baseline = f.broker.counts().bytes; child.begin(); // End BEFORE any Begin(true)
  child.parent.pendingFinal(); f.owner.retire(); let closed = false;
  const closure = child.channel.close().then(() => { closed = true; });
  await until(() => child.parent.planCalls() === 1 && f.timers().delays.includes(1000));
  const held = baseline - 96 * 1024 - 10 * 2048;
  assert.equal(closed, false); assert.equal(f.broker.counts().bytes, held);
  assert.deepEqual(child.begins(), [false]); assert.deepEqual(f.rowBudget(), { reserved: 1, actions: 1 });
  assert.equal(child.counts().settlements, 0); assert.equal(child.counts().returns, 0);
  f.advance(1000); await until(() => child.parent.planCalls() === 2 && f.timers().delays.includes(1000));
  assert.equal(f.broker.counts().bytes, held); assert.deepEqual(child.begins(), [false, false]);
  child.parent.pendingFinal(false); child.parent.loseFinalReplies(); f.advance(1000);
  await until(() => child.parent.planCalls() === 3 && f.timers().delays.includes(1000));
  f.advance(1000); await until(() => child.parent.finalCounts().finalCalls === 1 && f.timers().delays.includes(1000));
  assert.equal(closed, false); assert.equal(f.broker.counts().bytes, held);
  assert.deepEqual(f.rowBudget(), { reserved: 1, actions: 1 });
  f.advance(1000); await closure; await until(() => f.broker.counts().bytes === 0);
  assert.ok(child.begins().every(retry => !retry), "End never starts/adopts new work");
});

test("End during actual Begin transport waits for its real drainage before history recovery", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  const hold = child.holdBegin(), pending = child.begin().attempt(); await until(() => child.begins().length === 1);
  f.owner.retire(); let closed = false; const closure = child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.deepEqual(child.begins(), [true]); assert.ok(f.broker.counts().bytes >= 96 * 1024);
  hold.release(); assert.equal((await pending).kind, "retired"); await closure;
  assert.deepEqual(child.begins(), [true, false]); assert.equal(child.counts().admissions, 0); assert.equal(child.counts().returns, 1);
});

test("End during Begin SDK retains its original envelope until the actual provider settles", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive(); child.beginMode("absent");
  const held = f.holdWhen((name, args) => name === "encrypt" && new Uint8Array((args[0] as AesGcmParams).additionalData as ArrayBuffer)[6] === 8);
  const pending = child.begin().attempt(); await until(held.entered);
  f.owner.retire(); let closed = false; const closure = child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.deepEqual(child.begins(), []); assert.ok(f.broker.counts().bytes >= 96 * 1024);
  held.release(); assert.equal((await pending).kind, "retired"); await closure;
  assert.deepEqual(child.begins(), [false]); assert.equal(child.counts().admissions, 0); assert.equal(child.counts().returns, 0);
});

test("End during unknown Begin read waits for actual reader cancellation before native recovery", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  let cancel: ReturnType<typeof f.stallBody> | null = null;
  child.onBegin(() => { if (cancel === null) cancel = f.stallBody(); });
  const pending = child.begin().attempt(); await until(() => child.begins().length === 1);
  f.owner.retire(); let closed = false; const closure = child.channel.close().then(() => { closed = true; });
  await until(() => cancel !== null && cancel.entered());
  assert.equal(closed, false); assert.deepEqual(child.begins(), [true]); assert.ok(f.broker.counts().bytes >= 96 * 1024);
  f.clearBodyStall(); cancel!.release(); assert.equal((await pending).kind, "retired"); await closure;
  assert.deepEqual(child.begins(), [true, false]); assert.equal(child.counts().inputs, 0); assert.equal(child.counts().returns, 1);
});

test("Pending Begin cannot change its authenticated row on later adoption", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive(); child.beginMode("pending");
  const handle = child.begin(); assert.equal((await handle.attempt()).kind, "status");
  child.beginMode("attached"); child.alterBegin(bytes => { bytes[2] ^= 1; return bytes; });
  assert.equal((await handle.attempt()).kind, "uncertain"); assert.equal(child.counts().inputs, 0); assert.equal(child.counts().admissions, 0);
  child.alterBegin(null); assert.equal((await handle.attempt()).kind, "accepted");
  assert.deepEqual(child.begins(), [true, true, true]);
});

test("known child waits for actual derivation settlement before lending its phase", async t => {
  const f = await fixture(t), child = await framedConfirmation(f), hold = child.parent.holdSettlement();
  assert.equal((await child.derive()).kind, "accepted");
  await until(() => child.parent.counts().settlements === 1);
  const pending = child.handle().attempt();
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(child.parent.wasReturned(), false); assert.equal(child.counts().inputs, 0);
  hold.release(); assert.equal((await pending).kind, "accepted");
});

test("known child historical result is observed and returned without AEAD or engine admission", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  child.historical(); const result = await child.handle().attempt();
  assert.equal(result.kind, "status"); if (result.kind === "status") assert.equal(result.localDrained, true);
  assert.equal(child.counts().admissions, 0); assert.equal(child.counts().results, 0);
});

test("known child lost Input retries the same descriptor and does not reconstruct the pair", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  child.loseInput(); const handle = child.handle(), pair = child.ready.derivation.pair();
  assert.equal((await handle.attempt()).kind, "uncertain"); assert.equal(child.counts().admissions, 0);
  assert.equal((await handle.attempt()).kind, "accepted");
  assert.equal(child.ready.derivation.pair(), pair); assert.equal(child.counts().inputs, 2);
});

for (const rejected of [false, true]) test(`known child lost ${rejected ? "rejected" : "accepted"} result/Return preserves original output and retries cleanup`, async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  if (rejected) child.reject(); child.loseResult(); child.loseReturn();
  const handle = child.handle(); assert.equal((await handle.attempt()).kind, "uncertain");
  const returning = handle.attempt();
  await until(() => child.counts().returns === 1); // lost Return remains owned
  assert.equal((await returning).kind, "uncertain");
  const final = handle.attempt();
  f.advance(1000);
  assert.equal((await final).kind, rejected ? "rejected" : "accepted");
  assert.equal(child.counts().admissions, 1); assert.equal(child.counts().inputs, 1); assert.equal(child.counts().results, 2);
});

test("known child End waits for actual Input read cancellation and original child Return", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  let cancel: ReturnType<typeof f.stallBody> | null = null;
  child.onInput(() => { cancel = f.stallBody(); });
  const pending = child.handle().attempt(); await until(() => child.counts().inputs === 1);
  let closed = false; const closure = child.channel.close().then(() => { closed = true; });
  await until(() => cancel !== null && cancel.entered());
  assert.equal(closed, false); assert.ok(f.broker.counts().bytes >= 96 * 1024);
  assert.equal(child.counts().settlements, 0); assert.equal(child.counts().returns, 0);
  f.clearBodyStall(); cancel!.release();
  assert.equal((await pending).kind, "retired"); await closure;
  assert.equal(child.counts().admissions, 0); assert.equal(child.counts().returns, 1);
});

test("known child End before its first attempt still returns the original native row", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  const handle = child.handle(); await child.channel.close();
  assert.equal((await handle.attempt()).kind, "retired");
  assert.equal(child.counts().inputs, 0); assert.equal(child.counts().admissions, 0);
  assert.equal(child.counts().settlements, 1); assert.equal(child.counts().returns, 1);
});

test("known child does not report completed Return while actual native settlement is blocked", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  const hold = child.holdSettlement(); let completed = false;
  const pending = child.handle().attempt().then(outcome => { completed = true; return outcome; });
  await until(() => child.counts().settlements === 1);
  assert.equal(child.counts().results, 1); assert.equal(child.counts().acknowledgements, 1);
  assert.equal(completed, false); assert.equal(child.counts().returns, 0);
  hold.release(); assert.equal((await pending).kind, "accepted"); assert.equal(child.counts().returns, 1);
});

test("known child queued history and End release the history gate before framed settlement", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  const held = f.holdWhen((name, args) => name === "encrypt" && new TextDecoder().decode((args[0] as AesGcmParams).additionalData as ArrayBuffer) === "native signed Hello header");
  const action = child.handle().attempt(); await until(held.entered);
  const history = f.broker.recoverChannelHistory(f.owner, child.channel, historyPurpose);
  let closed = false; const close = child.channel.close().then(() => { closed = true; });
  assert.equal(closed, false); assert.equal(child.counts().settlements, 0);
  held.release(); assert.equal((await action).kind, "retired"); await close;
  assert.equal((await history).kind, "retired"); assert.equal(child.counts().returns, 1);
});

test("known child in-flight history cancellation and End release the gate before framed Return", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  await until(() => f.broker.counts().active === 0);
  child.handle();
  const cancel = f.stallBody();
  const history = f.broker.recoverChannelHistory(f.owner, child.channel, historyPurpose);
  await until(() => f.operations().at(-1) === 8);
  let closed = false; const close = child.channel.close().then(() => { closed = true; });
  await until(cancel.entered); assert.equal(closed, false); assert.equal(child.counts().returns, 0);
  assert.ok(f.broker.counts().bytes >= 96 * 1024);
  f.clearBodyStall(); cancel.release(); await close;
  assert.equal((await history).kind, "retired"); assert.equal(child.counts().returns, 1);
});

test("known child rejects unknown purpose discriminants instead of treating them as incoming", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  assert.throws(() => f.broker.nativeConfirmationFromChannel(f.owner, child.channel, child.ready.chain,
    { ...child.purpose, request: { kind: "other", role: 1 } as unknown as VoiceIpcConfirmationPurpose["request"] }, child.row, child.identity), /invalid voice confirmation purpose/);
  assert.equal(child.counts().inputs, 0);
  child.handle(); // retain the real known child for End; the rejected hint created none
});

for (const mode of ["idle", "historical", "failed-input"] as const) test(`known child ${mode} retains its exact fixed row after End without an ActionRecord`, async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  const baseline = f.broker.counts().bytes, handle = child.handle();
  assert.deepEqual(f.rowBudget(), { reserved: 11, actions: 1 }, "descriptor retains the original global preclaim");
  if (mode === "historical") { child.historical(); assert.equal((await handle.attempt()).kind, "status"); }
  if (mode === "failed-input") { child.loseInput(); assert.equal((await handle.attempt()).kind, "uncertain"); }
  assert.deepEqual(f.rowBudget(), { reserved: 11, actions: 1 });
  await child.channel.close();
  assert.equal(f.broker.counts().bytes, baseline - 96 * 1024 - 10 * 2048,
    "only ten unused rows and the actually drained variable envelope may be refunded");
  assert.deepEqual(f.rowBudget(), { reserved: 1, actions: 1 }, "known history still occupies its global logical slot");
});

test("several historical children retain separate fixed claims without manufacturing ActionRecords", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  const baseline = f.broker.counts().bytes;
  child.historical(); assert.equal((await child.handle().attempt()).kind, "status");
  for (let n = 0; n < 2; n++) assert.equal((await child.nextHistorical().attempt()).kind, "status");
  assert.deepEqual(f.rowBudget(), { reserved: 11, actions: 1 });
  await child.channel.close();
  assert.equal(f.broker.counts().bytes, baseline - 96 * 1024 - 8 * 2048);
  assert.deepEqual(f.rowBudget(), { reserved: 3, actions: 1 });
});

for (const mode of ["idle", "accepted", "historical", "failed-input"] as const) test(`known child ${mode} keeps fixed custody through Pending parent final after owner End`, async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  const baseline = f.broker.counts().bytes, handle = child.handle();
  if (mode === "accepted") assert.equal((await handle.attempt()).kind, "accepted");
  if (mode === "historical") { child.historical(); assert.equal((await handle.attempt()).kind, "status"); }
  if (mode === "failed-input") { child.loseInput(); assert.equal((await handle.attempt()).kind, "uncertain"); }
  child.parent.pendingFinal(); f.owner.retire(); let closed = false;
  const closure = child.channel.close().then(() => { closed = true; });
  await until(() => child.parent.planCalls() === 1 && f.timers().delays.includes(1000));
  assert.equal(closed, false); assert.equal(child.counts().returns, 1);
  assert.equal(f.broker.counts().bytes, baseline - 96 * 1024 - 10 * 2048,
    "SDK drainage releases variable buffers and unused rows, while the linked history stays paid");
  assert.deepEqual(f.rowBudget(), mode === "accepted" ? { reserved: 0, actions: 2 } : { reserved: 1, actions: 1 },
    "both descriptor and adopted ActionRecord claims survive owner End until actual unlink");
  f.advance(1000); await until(() => child.parent.planCalls() === 2 && f.timers().delays.includes(1000));
  assert.equal(child.parent.finalCounts().finalCalls, 0);
  child.parent.pendingFinal(false); f.advance(1000); await closure;
  assert.equal(child.parent.finalCounts().finalAcknowledged, true);
  await until(() => f.broker.counts().bytes === 0);
  assert.equal(f.broker.counts().bytes, 0); assert.deepEqual(f.rowBudget(), { reserved: 0, actions: 0 });
});

test("nine retained child histories count toward pressure until final ACK and actual channel unlink", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  child.historical(); assert.equal((await child.handle().attempt()).kind, "status");
  for (let n = 0; n < 8; n++) assert.equal((await child.nextHistorical().attempt()).kind, "status");
  const baseline = f.broker.counts().bytes;
  child.parent.pendingFinal(); f.owner.retire(); const closure = child.channel.close();
  await until(() => child.parent.planCalls() === 1 && f.timers().delays.includes(1000));
  const held = baseline - 96 * 1024 - 2 * 2048;
  assert.equal(f.broker.counts().bytes, held);
  assert.deepEqual(f.rowBudget(), { reserved: 9, actions: 1 });
  const pressureOwner = createVoiceCryptoOwner("pressure-owner", hash(40), () => true);
  t.after(() => pressureOwner.retire());
  const key = await real.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]);
  const pair = f.broker.createPair(pressureOwner, key, key, "pressure-domain");
  const native = { async admitAction() { throw new Error("unused pressure admission"); },
    async commitConfirmation() { throw new Error("unused pressure commit"); }, async settleAction() {} };
  const input = (n: number) => ({ action: String(n), version: "1", inputTag: hash(n), handshake: hash(41),
    deliveryToken: String(n), kind: "hello" as const, sequence: "1", aad: new Uint8Array(), ciphertext: new Uint8Array(16) });
  for (let n = 1; n <= 22; n++) f.broker.registerConfirmation(pressureOwner, pair, native, input(n));
  assert.throws(() => f.broker.registerConfirmation(pressureOwner, pair, native, input(23)), /capacity/,
    "ten retained chain slots leave only22 of the original32 global positions");
  f.loseCollection(); child.parent.pendingFinal(false); f.advance(1000);
  await until(() => child.parent.finalCounts().finalAcknowledged && f.counts().collections === 1 && f.timers().delays.includes(1000));
  assert.deepEqual(f.rowBudget(), { reserved: 9, actions: 23 }, "final proof alone does not unlink channel-held metadata");
  assert.throws(() => f.broker.registerConfirmation(pressureOwner, pair, native, input(23)), /capacity/);
  f.advance(1000); await closure;
  assert.deepEqual(f.rowBudget(), { reserved: 0, actions: 22 });
  f.broker.registerConfirmation(pressureOwner, pair, native, input(23));
  pressureOwner.retire(); await until(() => f.broker.counts().bytes === 0);
});

for (const withChild of [false, true]) test(`${withChild ? "accepted child" : "Derive-only"} collection before owner End releases the same global records afterward`, async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  if (withChild) {
    const child = await framedConfirmation(f); await child.derive();
    assert.equal((await child.handle().attempt()).kind, "accepted");
  } else {
    const parent = await framedDerivation(f);
    const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity).attempt();
    assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original derivation required");
    assert.equal((await ready.derivation.attempt()).kind, "accepted");
  }
  await channel.close(); await until(() => f.broker.counts().active === 0);
  assert.ok(f.owner.isCurrent());
  assert.deepEqual(f.rowBudget(), { reserved: 0, actions: withChild ? 2 : 1 }, "live owner retains completed identities");
  f.owner.retire();
  assert.deepEqual(f.rowBudget(), { reserved: 0, actions: 0 }, "owner retirement after unlink collects every occupied record");
  assert.equal(f.broker.counts().bytes, 0);
});

test("known child completion cannot renew the original browser deadline or encryption allocator", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  f.advance(10_000); const handle = child.handle(); assert.equal((await handle.attempt()).kind, "accepted");
  const before = f.counts(); f.advance(5_000); f.broker.collectChains();
  assert.equal(child.ready.chain.isCurrent(), false); assert.equal(child.ready.derivation.pair(), null);
  assert.throws(() => f.broker.nativeConfirmationFromChannel(f.owner, child.channel, child.ready.chain,
    { ...child.purpose, request: { kind: "incoming", role: 2 } }, "00000000000000020000000000000001", { ...child.identity, action: "9007199254741028" }), /derived pair/);
  assert.deepEqual(f.counts().tx, before.tx); assert.deepEqual(f.counts().steps, before.steps);
  assert.equal((await handle.attempt()).kind, "accepted", "retained past metadata is not a fresh engine or pair");
});

for (const offset of [81, 417, 425]) test(`known child bound Input mutation at${offset} cannot reach engine admission`, async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive();
  child.alter(bytes => { bytes[offset] ^= 1; return bytes; });
  assert.equal((await child.handle().attempt()).kind, "uncertain");
  assert.equal(child.counts().admissions, 0); assert.equal(child.counts().results, 0);
});

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

for (const version of [7, 8, 9, 10, 11, 12, 13, 14, 15, 16]) test(`HMR refuses v${version} retained pool rather than creating a second channel allocator`, () => {
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
  // Start(previous) has the largest child selector: its154-byte prefix plus80-byte ACK.
  // The same paid child descriptor/RPC closure is borrowed, never copied into Control.
  const childAck = encodeVoiceIpcConfirmationAction({ parent: historyPurpose, parentRow: "0".repeat(31) + "1",
    request: { kind: "start", predecessor: hash(10) } }, { kind: "ack", identity: { action: "1", version: "1", inputTag: hash(11) }, digest: hash(12) });
  assert.equal(childAck.body.length, 234); childAck.body.fill(0);
  const controlBuffers = 2 * (234 + 16) + 2 * (242 + 16) + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const descriptor = 2 * (3 * 64 + 20 + 32 + 2 * 20 + 64 + 64);
  const decoded = 2 * 1024; // bounded fixed status/final graph, including exact-u64 strings
  const wrappers = 3 * 1024 + 2 * "retire-plan".length; // retained fixed final-mode discriminator
  const control = controlBuffers + descriptor + decoded + wrappers;
  assert.ok(control <= 8 * 1024, `terminal Control payload/wrapper envelope ${control}/8192`);
});

test("nine child descriptors and serial Data adoption fit unchanged chain envelopes", () => {
  // Structural application-owned payload/wrapper accounting, not a JS engine/SDK/GC/RSS
  // bound. The immutable parent selector and chain binding are SHARED across every child.
  // Each occupied2KiB history row covers its descriptor, cached adapter, ActionRecord,
  // receipt and one-shot physical-drain Signal. Nine children plus Derive occupy10/12 rows.
  const childHints = 2 * (32 + 64 + 32 + 20 + 20 + 64);
  const decodedInputIdentity = 2 * (20 + 20 + 64);
  const terminalIdentityAndTags = 2 * (20 + 20 + 64 + 64 + 64);
  const rowWrappers = 768; // bounded adapter/functions, records, Signal and scalar fields
  const row = childHints + decodedInputIdentity + terminalIdentityAndTags + rowWrappers;
  assert.ok(row <= 2048, `child row payload/wrapper inventory ${row}/2048`);
  assert.ok(10 * row <= 24 * 1024, `retained child/Derive history ${10 * row}/24576`);
  // Historical Input has no ActionRecord, but retains the FULL native Status instead of
  // the compact accepted CommitReply. Use the shared Sealed vector (largest receipt), with
  // only row/identity/receipt.action/inputTag shared with the original descriptor as code
  // explicitly normalizes them. Do not silently substitute the smaller live-result graph.
  const sealed = historyVector("sealed").expected;
  assert.ok(sealed.kind === "status" && sealed.receipt?.outcome.kind === "sealed");
  const stringPayload = (value: unknown): number => typeof value === "string" ? 2 * value.length :
    value && typeof value === "object" ? Object.values(value).reduce<number>((sum, field) => sum + stringPayload(field), 0) : 0;
  const historicalPayload = stringPayload({ kind: sealed.kind, pending: sealed.pending,
    receipt: { outputTag: sealed.receipt.outputTag, digest: sealed.receipt.digest, outcome: sealed.receipt.outcome } });
  const historicalRow = childHints + historicalPayload + rowWrappers;
  assert.ok(historicalRow <= 2048, `full historical Sealed row inventory ${historicalRow}/2048`);
  const parentSelector = 2 * (3 * 64 + 20);
  const sharedBinding = 2 * (7 * 64 + 4 * 20) + 8;
  const metadata = parentSelector + sharedBinding + 2 * 128 + 4096 + 2 * 128;
  assert.ok(metadata <= 8 * 1024, `chain metadata/maps/drainage inventory ${metadata}/8192`);

  // The current child can overlap frame ciphertext/opened body, decoder-owned input and
  // adopted immutable input. Count all four full-sized buffers even though the decoder
  // copies only AAD/body and registration zeros its temporary arrays before the engine.
  const transcript = 1024, retained = 2 * (130 + transcript) + 9 * (2048 - 430);
  const largestRequest = 154 + 82 + 528;
  const framing = 2 * (largestRequest + 16) + 4 * 2064 + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const variable = retained + framing + 16 * 1024 + 16 * 1024;
  assert.ok(variable <= 96 * 1024, `child Data/engine/retained-input inventory ${variable}/98304`);
});
