import { encodeVoiceMediaOriginal, encodeVoiceMediaExisting } from "./voice-ipc-media.ts";
import { encodeVoicePcIntent } from "./voice-ipc-pc.ts";
import { encodeVoiceSdpPhase } from "./voice-ipc-sdp.ts";
import { encodeVoiceSdkCustody } from "./voice-ipc-sdk.ts";
import { encodeVoiceSdpAction, encodeVoiceSdpBegin, encodeVoiceSdpResolve } from "./voice-ipc-sdp-actions.ts";
import { encodeVoiceKeySession } from "./voice-ipc-key-session.ts";
import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { webcrypto, createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import {
  VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker, voicePacketIv,
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
import { encodeVoiceTransition } from "./voice-ipc-transition.ts";

const real = webcrypto.subtle as unknown as SubtleCrypto;
const utf8 = new TextEncoder();
const hash = (n: number) => n.toString(16).padStart(64, "0");
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const deadline = performance.now() + 15_000;
  while (!check()) { if (performance.now() >= deadline) throw new Error("condition did not settle"); await new Promise<void>(yes => setImmediate(yes)); }
}
const flush = () => new Promise<void>(resolve => setImmediate(resolve));
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
  type ActionResponse = { kind: 0 | 1 | 2; body: Uint8Array };
  let actionResponder: ((header: VoiceIpcHeader, opened: Uint8Array) => ActionResponse | Promise<ActionResponse>) | null = null;
  let responseChange: Partial<VoiceIpcHeader> | null = null;
  let holdAt = -1, held: ReturnType<typeof gate> | null = null, engineEnabled = false;
  let holdWhen: ((name: string, args: unknown[]) => boolean) | null = null;
  let stalled = false, cancelEntered = false, cancelGate: ReturnType<typeof gate> | null = null;
  let observedStep = -1, sdpSdkCalls = 0, providerActive = 0, providerPeak = 0;
  let now = 0, timerSerial = 0, hostileHeaders = false, hangFetch = false, fetchAborted = false, fetchEntered = false;
  const timers = new Map<number, { at: number; run(): void }>();
  let timerPeak = 0, scheduleHook: (() => void) | null = null, cancelHook: (() => void) | null = null, nowHook: (() => void) | null = null;
  let fetchCancel: ReturnType<typeof gate> | null = null;
  const engine = new Proxy(real, { get(target, name) {
    const value = Reflect.get(target, name);
    if (typeof value !== "function") return value;
    return async (...args: unknown[]) => {
      const tracked = engineEnabled;
      if (tracked) { providerActive++; providerPeak = Math.max(providerPeak, providerActive); }
      try {
      if (engineEnabled) {
        const index = engineSteps.length; engineSteps.push(String(name));
        if ((name === "encrypt" || name === "decrypt") && (args[0] as AesGcmParams)?.additionalData?.byteLength === 579) sdpSdkCalls++;
        if (index === holdAt) { observedStep = index; await held!.promise; }
        if (holdWhen?.(String(name), args)) { holdWhen = null; observedStep = -2; await held!.promise; }
        if (name === "encrypt" && failEncrypt) { failEncrypt = false; throw new Error("local provider failure"); }
      }
      return await value.apply(target, args);
      } finally { if (tracked) providerActive--; }
    };
  } });
  let ownerHook: (() => void) | null = null;
  const owner = createVoiceCryptoOwner(`voice-${identitySeed.toString(16).padStart(16, "0")}`, hash(identitySeed), () => {
    const hook = ownerHook; ownerHook = null; hook?.(); return true;
  });
  const admission = { async admit() { ordinary++; return "ordinary"; }, async settle() {}, async commitInbound() {} };
  const broker = sharedBroker ?? new VoiceCryptoBroker(admission, engine, () => now, {
    now() { const hook = nowHook; nowHook = null; hook?.(); return now; }, schedule(run, ms) {
      const id = ++timerSerial; timers.set(id, { at: now + ms, run }); timerPeak = Math.max(timerPeak, timers.size);
      const hook = scheduleHook; scheduleHook = null; hook?.(); return id;
    },
    cancel(id) { timers.delete(id as number); const hook = cancelHook; cancelHook = null; hook?.(); },
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
    async readWork(value, execution, _fromSlot) {
      assert.equal(value, owner.token); assert.equal(execution, executor);
      return { kind: "snapshot", owner: value, executor: execution, nextSlot: 32, work: null };
    },
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
  let responseHook: ((header: VoiceIpcHeader, mode: number) => Promise<void>) | null = null;
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
    const requestMode=opened[1]; // Closed public request-mode metadata only, not body retention.
    let actionReply: ActionResponse | null = null;
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
    if (responseHook) await responseHook(request.header,requestMode);
    return result;
  };
  engineEnabled = true;
  t.after(async () => {
    for (const cleanup of cleanups) cleanup();
    held?.release(); cancelGate?.release(); fetchCancel?.release(); owner.retire(); broker.retrySelectedJoinKeys();
    await until(() => broker.counts().active === 0);
    // Entered terminal-handle dispatch can finish after the last IPC activity. Join
    // its queued disposal tail too; End alone is never proof of that stack's return.
    await flush();
    assert.equal(broker.counts().bytes, 0);
  });
  return {
    owner, broker, native, transport, key: selected.key, admission,
    onOwnerCheck(hook: () => void) { ownerHook = hook; },
    onResponse(hook: (header: VoiceIpcHeader, mode: number) => Promise<void>) { responseHook=hook; },
    cleanup(work: () => void) { cleanups.push(work); },
    channel() { return broker.nativeChannel(owner, selected.key, native, transport); },
    counts: () => ({ ordinary, setups, settlements, collections, steps: [...engineSteps], tx: [...requestSequences], reads: bodyReads.length }),
    historyBytes(value: Uint8Array) { historyBody = Buffer.from(value); },
    actions(responder: typeof actionResponder) { actionResponder = responder; },
    actionHandler() { return actionResponder; },
    historyReply(name: string) { historyBody = Buffer.from(historyVector(name).plaintext, "hex"); },
    responseChange(value: Partial<VoiceIpcHeader> | null) { responseChange = value; },
    operations: () => [...operations], bodyReads: () => [...bodyReads],
    sdpSdkCount: () => sdpSdkCalls,
    providers: () => ({ active: providerActive, peak: providerPeak }),
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
    onCancel(work: () => void) { cancelHook = work; },
    onNow(work: () => void) { nowHook = work; },
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
async function framedDerivation(f: Awaited<ReturnType<typeof fixture>>, localIsLow = true, serial = 1,
    selectedPurpose: VoiceIpcPurpose = historyPurpose) {
  const historyPurpose = selectedPurpose, extra = BigInt(serial - 1) * 1000n;
  const peer = await real.generateKey({ name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]);
  const point = new Uint8Array(await real.exportKey("raw", peer.publicKey));
  const transcript = utf8.encode("original native framed action" + (serial === 1 ? "" : ` peer ${serial}`));
  const row = "0".repeat(16) + serial.toString(16).padStart(16, "0");
  const identity = Object.freeze({ action: (9007199254741007n + extra).toString(), version: "1", inputTag: hash(11) });
  const engineBytes = 3 * transcript.length + 4096 + 256;
  const binding = { chain: String(serial), epoch: historyPurpose.epoch, localDevice: "06".repeat(32), localJoin: hash(2), localKey: hash(3),
    peerDevice: historyPurpose.peer, peerJoin: historyPurpose.join, peerKey: historyPurpose.keyRef, handshake: hash(10 + (serial - 1) * 100),
    generation: "9007199254741011", nativeDeadline: "500000", localIsLow };
  const data = Buffer.concat([Buffer.from([1, 5]), Buffer.from(row, "hex"), integer(BigInt(identity.action), 8), integer(1n, 8), Buffer.from(identity.inputTag, "hex"),
    integer(BigInt(serial), 8), Buffer.from(f.owner.group, "hex"), Buffer.from([0]), Buffer.alloc(32), integer(1n, 16), integer(BigInt(binding.epoch), 8),
    ...[binding.localDevice, binding.localJoin, binding.localKey, binding.peerDevice, binding.peerJoin, binding.peerKey, binding.handshake].map(s => Buffer.from(s, "hex")),
    integer(BigInt(binding.generation), 8), integer(BigInt(binding.nativeDeadline), 8), Buffer.from([Number(localIsLow)]), integer(BigInt(engineBytes), 4), Buffer.from([0]),
    f.key.publicKey(), point, integer(BigInt(transcript.length), 2), transcript]);
  assert.equal(data.length, 541 + transcript.length);
  let inputReads = 0, admissions = 0, commits = 0, acknowledgements = 0, settlements = 0;
  let derived = false, acknowledged = false, settled = false, historical = false, lostResult = false;
  let retireCalls = 0, finalCalls = 0, finalAcknowledged = false, finalPending = false;
  let loseIssuance = false, loseFinalAck = false, ordinaryReturned = false, loseReturn = false;
  let planCalls = 0, finalMode: 1 | 2 = 1, replyMode: 1 | 2 | null = null;
  let alter: ((data: Uint8Array) => Uint8Array) | null = null;
  let childResponder: Parameters<typeof f.actions>[0] = null;
  let transitionResponder: Parameters<typeof f.actions>[0] = null;
  let settleHold: ReturnType<typeof gate> | null = null;
  let returnHold: ReturnType<typeof gate> | null = null;
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
  f.cleanup(() => { finalPending = false; loseIssuance = false; loseFinalAck = false; settleHold?.release(); returnHold?.release(); f.native.settleSetup = nativeSettle; });
  f.actions(async (header, opened) => {
    if (header.operation >= 10) {
      assert.ok(transitionResponder, "unexpected custody operation");
      return transitionResponder(header, opened);
    }
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
          ordinaryReturned = true;
          if (returnHold) await returnHold.promise;
          if (loseReturn) { loseReturn = false; throw new Error("ordinary Return committed but reply lost"); }
          return status();
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
  // Independent peer derives its outgoing key from the public browser point and its own
  // private key. Never extract/access the retained browser Pair or selected private key.
  let incomingKey: CryptoKey | null = null;
  const incoming = async (plaintext: Uint8Array<ArrayBuffer>, aad: Uint8Array<ArrayBuffer>, sequence: bigint) => {
    if (!incomingKey) {
      const browserPoint = await real.importKey("raw", f.key.publicKey(), { name: "ECDH", namedCurve: "P-256" }, false, []);
      const secret = new Uint8Array(await real.deriveBits({ name: "ECDH", public: browserPoint }, peer.privateKey, 256));
      let material: CryptoKey;
      try { material = await real.importKey("raw", secret, "HKDF", false, ["deriveKey"]); } finally { secret.fill(0); }
      const context = (label: string) => Buffer.concat([utf8.encode(label + "\0"), transcript]);
      const salt = new Uint8Array(await real.digest("SHA-256", context("mewtual/voice/v2/transcript")));
      incomingKey = await real.deriveKey({ name: "HKDF", hash: "SHA-256", salt,
        info: context("mewtual/voice/v2/aes-gcm/" + (localIsLow ? "high-to-low" : "low-to-high")) }, material,
        { name: "AES-GCM", length: 256 }, false, ["encrypt"]);
    }
    return new Uint8Array(await real.encrypt({ name: "AES-GCM", iv: voicePacketIv(sequence), additionalData: aad }, incomingKey, plaintext));
  };
  return { row, identity, binding, data, engineBytes, incoming, counts: () => ({ inputReads, admissions, commits, acknowledgements, settlements }),
    historical() { historical = true; derived = true; settled = true; },
    loseResult() { lostResult = true; }, alter(work: typeof alter) { alter = work; },
    finalCounts: () => ({ retireCalls, finalCalls, finalAcknowledged }),
    planCalls: () => planCalls,
    finalReplyMode(value: typeof replyMode) { replyMode = value; },
    pendingFinal(value = true) { finalPending = value; },
    loseFinalReplies() { loseIssuance = true; loseFinalAck = true; },
    returned() { ordinaryReturned = true; historical = true; settled = true; },
    wasReturned: () => ordinaryReturned,
    loseReturn() { loseReturn = true; },
    holdReturn() { returnHold = gate(); return returnHold; },
    childReplies(responder: typeof childResponder) { childResponder = responder; },
    transitionReplies(responder: typeof transitionResponder) { transitionResponder = responder; },
    holdSettlement() { settleHold = gate(); return settleHold; },
    beforeFinal(check: () => void) { checkFinal = check; },
  };
}

/** Mock native signer/Root only; actual original-channel framing, P256/HKDF/AEAD, bounded
 * stream reads, broker ownership and native-action adapters execute unchanged. */
async function framedConfirmation(f: Awaited<ReturnType<typeof fixture>>, finish = false, localIsLow = true, serial = 1) {
  const extra = serial - 1;
  const selectedPurpose = extra ? { ...historyPurpose, peer: hash(7 + 100 * extra), join: hash(8 + 100 * extra), keyRef: hash(9 + 100 * extra) } : historyPurpose;
  const channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f, localIsLow, serial, selectedPurpose);
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, selectedPurpose, parent.row, parent.identity).attempt();
  assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original chain required");
  let row = "0000000000000001" + serial.toString(16).padStart(16, "0"), identity = { action: (9007199254741027n + BigInt(extra) * 1000n).toString(), version: "3", inputTag: hash(91) };
  let purpose: VoiceIpcConfirmationPurpose = { parent: selectedPurpose, parentRow: parent.row, request: { kind: "start", predecessor: null } };
  const aad = utf8.encode("native signed Hello header"), body = localIsLow ? utf8.encode("native closed Hello")
    : await parent.incoming(utf8.encode("native closed Hello"), aad, 1n);
  const engineBytes = 4 * (aad.length + body.length + (localIsLow ? 16 : 0)) + 3 * 512 + 2048;
  const b = parent.binding;
  const data = Buffer.concat([Buffer.from([1, 7]), Buffer.from(row, "hex"), integer(BigInt(identity.action), 8), integer(BigInt(identity.version), 8),
    Buffer.from(identity.inputTag, "hex"), Buffer.from(parent.row, "hex"), Buffer.from(f.owner.group, "hex"), Buffer.from([0]), Buffer.alloc(32), integer(1n, 16), integer(BigInt(b.epoch), 8),
    ...[b.localDevice, b.localJoin, b.localKey, b.peerDevice, b.peerJoin, b.peerKey, b.handshake].map(s => Buffer.from(s, "hex")),
    integer(BigInt(b.generation), 8), integer(BigInt(b.nativeDeadline), 8), Buffer.from([Number(localIsLow)]), integer(BigInt(engineBytes), 4), Buffer.from([localIsLow ? 1 : 2, finish ? 3 : 1]),
    integer(1n, 8), integer(BigInt(aad.length), 2), integer(BigInt(body.length), 2), aad, body]);
  assert.equal(data.length, 430 + aad.length + body.length);
  let inputs = 0, admissions = 0, results = 0, acknowledgements = 0, settlements = 0, returns = 0;
  let resultTag: string | null = null, output: Uint8Array | null = null, acknowledged = false, settled = false, returned = false;
  let lostInput = false, lostResult = false, lostReturn = false, historical = false, rejected = false;
  let nativeOnly = false;
  let alter: ((data: Uint8Array) => Uint8Array) | null = null, onInput: (() => void) | null = null;
  let holdSettle: ReturnType<typeof gate> | null = null;
  let beginMode: "attached" | "pending" | "absent" = "attached", lostBegin = false;
  let holdBegin: ReturnType<typeof gate> | null = null, beginAlter: ((data: Uint8Array) => Uint8Array) | null = null;
  let onBegin: (() => void) | null = null;
  let holdResult: ReturnType<typeof gate> | null = null;
  let holdReturned: ReturnType<typeof gate> | null = null;
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
  f.cleanup(() => { holdSettle?.release(); holdBegin?.release(); holdResult?.release(); holdReturned?.release(); f.clearBodyStall(); });
  parent.childReplies(async (header, opened) => {
    assert.equal(header.logical, header.operation === 8 ? "0".repeat(32) : row); assert.equal(opened[0], 2);
    assert.ok(parent.wasReturned(), "native child use follows actual parent Return");
    const exact = (request: Parameters<typeof encodeVoiceIpcConfirmationAction>[1]) => assert.deepEqual(opened, encodeVoiceIpcConfirmationAction(purpose, request).body);
    if (header.operation === 8) {
      const retry = opened.at(-1) === 1; exact({ kind: "begin", retry }); begins.push(retry);
      assert.deepEqual(f.rowBudget(), { reserved: retry ? 11 : nativeOnly ? 0 : 1, actions: 1 },
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
      if (holdResult) await holdResult.promise;
      if (lostResult) { lostResult = false; throw new Error("native result committed; reply lost"); }
      return status();
    }
    if (header.operation === 4) { acknowledgements++; exact({ kind: "ack", identity, digest: receipt }); acknowledged = true; return status(); }
    if (header.operation === 5) {
      settlements++; exact({ kind: "settle", identity }); if (holdSettle) await holdSettle.promise; settled = true; return status();
    }
    assert.equal(header.operation, 6); returns++; exact({ kind: "close", mode: "return" });
    assert.ok(settled && (resultTag === null || acknowledged)); returned = true;
    if (holdReturned) await holdReturned.promise;
    if (lostReturn) { lostReturn = false; throw new Error("native Return completed; reply lost"); }
    return status();
  });
  return { channel, parent, ready, purpose, row, identity,
    nativeOnly() { nativeOnly = true; },
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
    holdResult() { holdResult = gate(); return holdResult; },
    holdReturn() { holdReturned = gate(); return holdReturned; },
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

/** Independent mock native lifecycle; actual channel HMAC/AEAD, SDK/IO and broker cleanup
 * execute unchanged. Real Root/Finish/Session evidence is covered by the Rust TCP fixture. */
function custodyReplies(f: Awaited<ReturnType<typeof fixture>>, child: Awaited<ReturnType<typeof framedConfirmation>>) {
  const b = child.parent.binding;
  const historyPurpose: VoiceIpcPurpose = { peer: b.peerDevice, join: b.peerJoin, keyRef: b.peerKey, epoch: b.epoch };
  let sdpResponder: Parameters<typeof f.actions>[0] = null;
  let mediaResponder: typeof sdpResponder = null;
  let mediaReleased: (() => boolean) | null = null;
  let claims = 0, releases = 0, claimed = false, released = false, loseClaim = false, loseRelease = false, refuse = false;
  let releaseHold: ReturnType<typeof gate> | null = null;
  let keyAdoptions = 0, keyReleases = 0, keyAdopted = false, keyReleased = false, loseAdoption = false;
  let keyHold: ReturnType<typeof gate> | null = null, endOnKeyReply = false;
  const storageProof = hash(100), sdkProof = hash(101), pcProof = hash(102);
  let pcAdmits = 0, pcCloses = 0, pcPrepared = false, pcClosed = false;
  let losePc = false, losePcClose = false, refusePc = false, preparingPc = false;
  let pcHold: ReturnType<typeof gate> | null = null, pcCloseHold: ReturnType<typeof gate> | null = null;
  let storagePrepared = false, storageReleased = false, sdkAdmitted = false, sdkReleased = false;
  let storagePrepares = 0, storageReleases = 0, sdkAdmissions = 0, sdkReleases = 0;
  let loseStorage = false, loseSdk = false, loseSdkRelease = false;
  let sdkHold: ReturnType<typeof gate> | null = null;
  const effectOrder: string[] = [];
  const keyProof = hash(99);
  const proof = hash(96);
  f.cleanup(() => { releaseHold?.release(); keyHold?.release(); sdkHold?.release(); pcHold?.release(); pcCloseHold?.release(); });
  child.parent.beforeFinal(() => {
    assert.ok(!claimed || released, "parent final cannot overtake native Release");
    assert.ok(!keyAdopted || keyReleased, "parent final cannot overtake separate native key Release");
    assert.ok(!storagePrepared || storageReleased, "Final cannot overtake storage Release");
    assert.ok(!sdkAdmitted || sdkReleased, "Final cannot overtake SDK Release");
    assert.ok(!pcPrepared || pcClosed, "Final cannot overtake PC Close");
    assert.ok(!mediaReleased || mediaReleased(), "Final cannot overtake media Release");
  });
  child.parent.transitionReplies(async (header, opened) => {
    if (header.operation === 28) {
      assert.ok(mediaResponder, "unexpected private media operation");
      return mediaResponder(header, opened);
    }
    if (header.operation >= 15) {
      assert.ok(sdpResponder, "unexpected private SDP operation");
      return sdpResponder(header, opened);
    }
    const base = { parentRow: child.parent.row, identity: child.parent.identity, handshake: child.parent.binding.handshake, key: keyProof, sequence: "1" };
    const prefix = () => [Buffer.from(base.parentRow, "hex"), integer(BigInt(base.identity.action), 8), integer(BigInt(base.identity.version), 8),
      Buffer.from(base.identity.inputTag, "hex"), Buffer.from(base.handshake, "hex"), Buffer.from(base.key, "hex"), integer(1n, 8)];
    if (header.operation === 14) {
      assert.ok(storagePrepared && !storageReleased && sdkAdmitted && !sdkReleased);
      const closing = opened[193] === 1;
      assert.deepEqual(opened, encodeVoicePcIntent({ ...base, storage: storageProof, sdkGeneration: "1", sdkProof,
        request: closing ? "close" : "admit" }).body);
      if (closing) {
        pcCloses++; pcClosed = true; effectOrder.push("pc-close");
        if (pcCloseHold) await pcCloseHold.promise;
        if (losePcClose) { losePcClose = false; throw new Error("native PC Close reply lost"); }
      } else {
        pcAdmits++;
        if (!pcPrepared && !f.owner.isCurrent()) return { kind: 0, body: new Uint8Array([7, 2, 0]) };
        pcPrepared = true; effectOrder.push("pc-admit");
        if (pcHold) await pcHold.promise;
        if (losePc) { losePc = false; throw new Error("native PC Admit reply lost"); }
        // This models a failure AFTER the native Claim/Actor boundary. Refusal is not
        // evidence of absence and must retain the exact Close obligation.
        if (refusePc) return { kind: 0, body: new Uint8Array([1, 3, 5]) };
      }
      if (!pcPrepared) return { kind: 0, body: new Uint8Array([7, 2, 0]) };
      const state = pcClosed ? 2 : preparingPc ? 0 : 1;
      return { kind: 0, body: Buffer.concat([Buffer.from([7, state, 1]), integer(1n, 8), Buffer.from(pcProof, "hex"),
        ...(state === 1 ? [Buffer.from([1, child.parent.binding.localIsLow ? 1 : 0]), Buffer.from(hash(103), "hex"), integer(10_000n, 8)] : [])]) };
    }
    if (header.operation === 12) {
      assert.ok(keyAdopted && !keyReleased, "storage cleanup precedes KeyRelease");
      const releasing = opened[121] === 1;
      assert.deepEqual(opened, encodeVoiceSdpPhase({ ...base, request: releasing ? { kind: "release", proof: storageProof } : { kind: "prepare" } }).body);
      if (releasing) {
        assert.ok(!sdkAdmitted || sdkReleased, "storage Release cannot stand in for SDK drainage");
        storageReleases++; storageReleased = true; effectOrder.push("storage-release");
      } else {
        storagePrepares++;
        if (!storagePrepared && !f.owner.isCurrent()) return { kind: 0, body: new Uint8Array([1, 3, 5]) };
        storagePrepared = true; effectOrder.push("storage-prepare");
        if (loseStorage) { loseStorage = false; throw new Error("native storage reply lost"); }
      }
      return { kind: 0, body: Buffer.concat([Buffer.from([5, storageReleased ? 3 : f.owner.isCurrent() ? 1 : 2]), ...prefix(), Buffer.from(storageProof, "hex")]) };
    }
    if (header.operation === 13) {
      assert.ok(storagePrepared && !storageReleased, "original storage remains until SDK Release");
      const releasing = opened[153] === 1;
      assert.deepEqual(opened, encodeVoiceSdkCustody({ ...base, storage: storageProof,
        request: releasing ? { kind: "release", generation: "1", proof: sdkProof } : { kind: "admit" } }).body);
      if (releasing) {
        assert.ok(!pcPrepared || pcClosed, "PC Close must precede SDK Release");
        assert.ok(!mediaReleased || mediaReleased(), "media Release precedes old SDK Release");
        sdkReleases++; assert.ok(sdkAdmitted); sdkReleased = true; effectOrder.push("sdk-release");
        if (loseSdkRelease) { loseSdkRelease = false; throw new Error("native SDK Release reply lost"); }
      } else {
        sdkAdmissions++;
        if (!sdkAdmitted && !f.owner.isCurrent()) return { kind: 0, body: new Uint8Array([1, 3, 5]) };
        sdkAdmitted = true; effectOrder.push("sdk-admit");
        if (sdkHold) await sdkHold.promise;
        if (loseSdk) { loseSdk = false; throw new Error("native SDK Admit reply lost"); }
      }
      return { kind: 0, body: Buffer.concat([Buffer.from([6, sdkReleased ? 3 : f.owner.isCurrent() ? 1 : 2]), ...prefix(),
        Buffer.from(storageProof, "hex"), integer(1n, 8), Buffer.from(sdkProof, "hex")]) };
    }
    if (header.operation === 11) {
      const releasing = opened[185] === 1;
      assert.deepEqual(opened, encodeVoiceKeySession({ parentRow: child.parent.row, purpose: historyPurpose,
        identity: child.parent.identity, handshake: child.parent.binding.handshake, claim: proof,
        request: releasing ? { kind: "release", receipt: keyProof } : { kind: "adopt" } }).body);
      assert.ok(claimed, "existing original native Claim is required");
      if (releasing) {
        assert.ok(!storagePrepared || storageReleased); assert.ok(!sdkAdmitted || sdkReleased);
        keyReleases++; assert.ok(keyAdopted); keyReleased = true; effectOrder.push("key-release");
      }
      else {
        keyAdoptions++; keyAdopted = true;
        if (keyHold) await keyHold.promise;
        if (loseAdoption) { loseAdoption = false; throw new Error("key adoption committed; reply lost"); }
      }
      const state = keyReleased ? 3 : f.owner.isCurrent() ? 1 : 2;
      const bytes = Buffer.concat([Buffer.from([4, state]), Buffer.from(child.parent.row, "hex"),
        integer(BigInt(child.parent.identity.action), 8), integer(BigInt(child.parent.identity.version), 8),
        Buffer.from(child.parent.identity.inputTag, "hex"), Buffer.from(child.parent.binding.handshake, "hex"),
        integer(BigInt(historyPurpose.epoch), 8), Buffer.from(hash(97), "hex"), Buffer.from(hash(98), "hex"),
        Buffer.from(proof, "hex"), Buffer.from(keyProof, "hex")]);
      if (endOnKeyReply) { endOnKeyReply = false; f.owner.retire(); }
      assert.equal(bytes.length, 234); return { kind: 0, body: bytes };
    }
    const release = opened[185] === 1;
    assert.equal(header.logical, child.parent.row);
    assert.deepEqual(opened, encodeVoiceTransition({ parentRow: child.parent.row, purpose: historyPurpose,
      identity: child.parent.identity, handshake: child.parent.binding.handshake,
      request: release ? { kind: "release", receipt: proof } : { kind: "claim" } }).body);
    if (release) {
      releases++; assert.ok(claimed); released = true;
      if (releaseHold) await releaseHold.promise;
      if (loseRelease) { loseRelease = false; throw new Error("Release committed; reply lost"); }
    } else {
      claims++;
      if (refuse) return { kind: 0, body: new Uint8Array([1, 3, 5]) };
      claimed = true;
      if (loseClaim) { loseClaim = false; throw new Error("Claim committed; reply lost"); }
    }
    const status = released ? 3 : f.owner.isCurrent() ? 1 : 2;
    const bytes = Buffer.concat([Buffer.from([3, status]), Buffer.from(child.parent.row, "hex"),
      integer(BigInt(child.parent.identity.action), 8), integer(BigInt(child.parent.identity.version), 8),
      Buffer.from(child.parent.identity.inputTag, "hex"), Buffer.from(child.parent.binding.handshake, "hex"),
      integer(BigInt(historyPurpose.epoch), 8), Buffer.from(hash(97), "hex"), Buffer.from(hash(98), "hex"),
      integer(9007199254740993n, 8), integer(status === 1 ? 20_000n : 0n, 4), Buffer.from(proof, "hex")]);
    assert.equal(bytes.length, 214); return { kind: 0, body: bytes };
  });
  return { counts: () => ({ claims, releases, claimed, released }),
    sdpReplies(responder: typeof sdpResponder) { sdpResponder = responder; },
    mediaReplies(responder: typeof sdpResponder, released: () => boolean) { mediaResponder = responder; mediaReleased = released; },
    loseClaim() { loseClaim = true; }, loseRelease() { loseRelease = true; }, refuseClaim() { refuse = true; },
    holdRelease() { releaseHold = gate(); return releaseHold; },
    keyCounts: () => ({ adoptions: keyAdoptions, releases: keyReleases, adopted: keyAdopted, released: keyReleased }),
    loseAdoption() { loseAdoption = true; }, holdAdoption() { keyHold = gate(); return keyHold; },
    endOnKeyReply() { endOnKeyReply = true; },
    effectCounts: () => ({ storagePrepares, storageReleases, sdkAdmissions, sdkReleases }),
    effectOrder: () => [...effectOrder],
    loseStorage() { loseStorage = true; }, loseSdk() { loseSdk = true; }, loseSdkRelease() { loseSdkRelease = true; },
    holdSdk() { sdkHold = gate(); return sdkHold; },
    pcCounts: () => ({ pcAdmits, pcCloses, pcPrepared, pcClosed }),
    losePc() { losePc = true; }, losePcClose() { losePcClose = true; }, refusePc() { refusePc = true; },
    preparingPc() { preparingPc = true; },
    holdPc() { pcHold = gate(); return pcHold; }, holdPcClose() { pcCloseHold = gate(); return pcCloseHold; },
  };
}

test("native Finish accepted before a delayed result crosses expiry retains the same keys through Claim", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive();
  const hold = child.holdResult();
  const pair = child.ready.derivation.pair(); assert.ok(pair);
  const pending = child.handle().attempt();
  await until(() => child.counts().results === 1);
  f.advance(15_001); f.broker.collectChains();
  assert.equal(child.ready.chain.isCurrent(), true, "custody survives old admission expiry");
  assert.equal(child.ready.derivation.pair(), pair, "no second key or pair wrapper is created");
  hold.release(); await pending;
  await until(() => custody.counts().claims === 1 && f.broker.counts().active === 0);
  assert.equal(child.ready.derivation.pair(), pair);
  assert.equal(child.counts().admissions, 1); assert.equal(child.counts().results, 1);
  assert.throws(() => child.nextHistorical(), /original derived pair required/,
    "retained keys cannot admit another old confirmation after Claim/deadline");
  f.owner.retire(); await child.channel.close();
  assert.equal(custody.counts().releases, 1);
});

test("lost Claim followed by End recovers original Closing proof and releases before parent final", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive();
  custody.loseClaim();
  await child.handle().attempt();
  await until(() => custody.counts().claims === 1 && f.broker.counts().active === 0);
  f.owner.retire(); await child.channel.close();
  assert.deepEqual(custody.counts(), { claims: 2, releases: 1, claimed: true, released: true });
  assert.equal(child.parent.finalCounts().finalAcknowledged, true);
});

test("a held Finish Return crossing old expiry keeps keys and recovers that same Return before Claim", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive(); const pair = child.ready.derivation.pair(); assert.ok(pair);
  const hold = child.holdReturn(), pending = child.handle().attempt();
  await until(() => child.counts().returns === 1);
  f.advance(15_001); f.broker.collectChains();
  assert.equal(child.ready.derivation.pair(), pair); assert.equal(custody.counts().claims, 0);
  hold.release();
  await until(() => f.timers().delays.includes(1000));
  f.broker.retrySelectedJoinKeys(); await pending;
  await until(() => custody.counts().claims === 1 && f.broker.counts().active === 0);
  assert.equal(child.counts().returns, 2, "retry the original lost Return; do not relabel it");
  assert.equal(child.ready.derivation.pair(), pair);
  f.owner.retire(); await child.channel.close(); assert.equal(custody.counts().releases, 1);
});

test("custody never renews an old Finish action while its actual SDK call crosses expiry", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive(); custody.refuseClaim();
  const held = f.holdWhen((name, args) => name === "encrypt" &&
    (args[0] as AesGcmParams).iv instanceof Uint8Array && ((args[0] as AesGcmParams).iv as Uint8Array)[0] === 0);
  const funding = f.broker.counts().bytes, pending = child.handle().attempt();
  await until(held.entered);
  f.advance(15_001); f.broker.collectChains();
  assert.equal(child.ready.chain.isCurrent(), true);
  assert.equal(f.broker.counts().bytes, funding, "actual held SDK call retains its original envelope");
  assert.ok(f.broker.counts().active > 0);
  held.release(); await pending; await child.channel.close();
  assert.equal(child.counts().admissions, 1); assert.equal(child.counts().results, 0,
    "custody cannot turn a late SDK completion into a fresh old-action commit");
  assert.equal(custody.counts().releases, 0);
});

test("a delayed/lost Release reply keeps original channel custody and cannot advance parent final", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive();
  await child.handle().attempt();
  await until(() => custody.counts().claims === 1 && f.broker.counts().active === 0);
  const held = custody.holdRelease(); custody.loseRelease();
  f.owner.retire(); let closed = false;
  const closing = child.channel.close().then(() => { closed = true; });
  await until(() => custody.counts().releases === 1);
  assert.equal(closed, false); assert.equal(child.parent.planCalls(), 0);
  assert.ok(f.broker.counts().bytes > 0);
  held.release(); await until(() => f.timers().pending > 0);
  f.broker.retrySelectedJoinKeys(); await closing;
  assert.equal(custody.counts().releases, 2);
  assert.equal(child.parent.finalCounts().finalAcknowledged, true);
});

test("rejected Finish cannot gain key custody and still finishes original native cleanup", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive();
  child.reject(); custody.refuseClaim();
  assert.equal((await child.handle().attempt()).kind, "rejected");
  await until(() => custody.counts().claims === 1);
  await child.channel.close();
  assert.equal(child.ready.chain.isCurrent(), false);
  assert.equal(custody.counts().releases, 0);
  assert.equal(child.parent.finalCounts().finalAcknowledged, true);
});

test("known child uses original derived keys through framed Input, permit, result and Return", async t => {
  const f = await fixture(t), child = await framedConfirmation(f);
  assert.equal((await child.derive()).kind, "accepted");
  const handle = child.handle(), baseline = f.broker.counts().bytes, pair = child.ready.derivation.pair();
  assert.equal(handle.row(), null, "a supplied child hint is not authenticated native metadata");
  assert.deepEqual(f.rowBudget(), { reserved: 11, actions: 1 });
  assert.equal(child.handle(), handle);
  const before = f.counts().steps.length, outcome = await handle.attempt();
  assert.equal(outcome.kind, "accepted");
  assert.equal(handle.row(), child.row, "exact authenticated Input adopts the original native row");
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
  assert.equal(handle.row(), null);
  assert.equal(child.begin(), handle); assert.deepEqual(f.rowBudget(), { reserved: 11, actions: 1 });
  const pending = handle.attempt(); assert.equal(handle.attempt(), pending);
  assert.equal((await pending).kind, "accepted"); assert.deepEqual(child.begins(), [true]);
  assert.equal(handle.row(), child.row);
  assert.equal(child.handle(), handle, "authenticated adoption retains the same descriptor/handle");
  assert.equal(f.broker.counts().bytes, baseline); assert.deepEqual(f.rowBudget(), { reserved: 10, actions: 2 });
});

test("lost Begin retries the same purpose without replacing its original pair or deadline", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive(); child.loseBegin();
  const pair = child.ready.derivation.pair(), handle = child.begin(), baseline = f.broker.counts().bytes;
  assert.equal((await handle.attempt()).kind, "uncertain"); assert.equal(child.counts().inputs, 0);
  assert.equal(handle.row(), null, "lost native reply cannot authenticate an unknown row");
  assert.equal((await handle.attempt()).kind, "accepted"); assert.deepEqual(child.begins(), [true, true]);
  assert.equal(handle.row(), child.row);
  assert.equal(child.ready.derivation.pair(), pair); assert.equal(f.broker.counts().bytes, baseline);
  assert.equal(child.counts().admissions, 1); assert.equal(child.counts().results, 1);
});

test("Pending Begin has no invented action and later adopts only its original authenticated row", async t => {
  const f = await fixture(t), child = await framedConfirmation(f); await child.derive(); child.beginMode("pending");
  const handle = child.begin(), pending = await handle.attempt();
  assert.equal(pending.kind, "status"); if (pending.kind !== "status") throw new Error("actual Pending required");
  assert.equal(pending.pending, "open"); assert.equal(pending.identity, null);
  assert.equal(handle.row(), child.row, "Pending authenticates only a row, never an action or accepted trigger");
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
  await flush(); // Retained cells join the retirement callback before fixed-history refund.
  assert.deepEqual(f.rowBudget(), { reserved: 0, actions: 0 }, "owner retirement after unlink collects every occupied record");
  assert.equal(f.broker.counts().bytes, 0);
});

test("End between actual intake transfer and ready publication still detaches the original cell", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f); parent.pendingFinal();
  const handle = f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity);
  type Intake = { transferred: boolean; outcome: { kind: string } | null; cell: { detached: boolean } };
  const state = f.broker as unknown as { intakes: Map<object, Map<string, Intake>> };
  const original = state.intakes.get(f.owner)!.get(parent.row)!;
  let ended = false;
  const beforeReady = () => {
    if (!ended && original.transferred && original.outcome?.kind !== "ready") {
      ended = true; f.owner.retire();
    } else if (!ended) f.onOwnerCheck(beforeReady);
  };
  f.onOwnerCheck(beforeReady);
  const outcome = await handle.attempt();
  assert.equal(ended, true, "actual transfer must precede the injected End");
  assert.equal(outcome.kind, "retired");
  await until(() => parent.finalCounts().retireCalls > 0);
  assert.ok(f.broker.counts().bytes > 0, "pending native final retains the original terminal cells");
  assert.equal(original.cell.detached, false);
  parent.pendingFinal(false); f.broker.retrySelectedJoinKeys(); await channel.close(); await flush();
  assert.equal(original.cell.detached, true); assert.equal(f.broker.counts().bytes, 0);
  const calls = f.operations(), first = handle.attempt();
  for (let n = 0; n < 2000; n++) assert.equal(handle.attempt(), first);
  assert.equal((await first).kind, "retired"); assert.deepEqual(f.operations(), calls);
});

test("End detaches retained public handles and erases the actual old adapter graph", async t => {
  const f = await fixture(t), child = await framedConfirmation(f);
  const derived = await child.derive(), intake = child.handle();
  const confirmed = await intake.attempt();
  assert.equal(derived.kind, "accepted"); assert.equal(confirmed.kind, "accepted");
  type Cell = { detached: boolean };
  type Row = { cell: Cell; native: unknown; chain: unknown; job: unknown; protectedInput: unknown };
  type Chain = { cell: Cell; records: Map<unknown, Row>; confirmations: Map<unknown, { cell: Cell; native: unknown; record: unknown }>;
    channel: unknown; custody: unknown; sdp: unknown; record: unknown };
  const state = f.broker as unknown as { chains: WeakMap<object, Chain> };
  const original = state.chains.get(child.ready.chain)!;
  const rows = [...original.records.values()], children = [...original.confirmations.values()];
  assert.ok(rows.length >= 2); assert.ok(children.length >= 1);
  await child.channel.close(); // Real native final/ACK and SDK cleanup; owner still retains fixed history.
  assert.ok(f.broker.counts().bytes > 0); assert.equal(original.cell.detached, false);
  f.owner.retire();
  // The retirement callback itself entered the cell. Its fee cannot leave while that
  // entered stack still needs the old record; the joined tail runs only after return.
  assert.ok(f.broker.counts().bytes > 0);
  assert.equal(child.ready.chain.isCurrent(), false);
  await flush();
  assert.equal(f.broker.counts().bytes, 0); assert.equal(state.chains.has(child.ready.chain), false);
  assert.equal(original.cell.detached, true); assert.equal(original.records.size, 0);
  assert.equal(original.confirmations.size, 0); assert.equal(original.channel, null);
  assert.equal(original.custody, null); assert.equal(original.sdp, null); assert.equal(original.record, null);
  for (const row of rows) {
    assert.equal(row.cell.detached, true); assert.equal(row.native, null); assert.equal(row.chain, null);
    assert.equal(row.job, null); assert.equal(row.protectedInput, null);
  }
  for (const item of children) {
    assert.equal(item.cell.detached, true); assert.equal(item.native, null); assert.equal(item.record, null);
  }
  const calls = f.operations();
  const first = intake.attempt();
  for (let n = 0; n < 2000; n++) {
    assert.equal(intake.attempt(), first); child.ready.chain.close(); child.ready.derivation.retire();
    assert.equal(child.ready.derivation.inspect(), "retired"); assert.equal(child.ready.derivation.pair(), null);
  }
  assert.equal((await first).kind, "retired"); assert.equal(intake.row(), null);
  assert.deepEqual(f.operations(), calls, "old handles start no native request after disposal");
  assert.equal(derived.kind, "accepted"); assert.equal(confirmed.kind, "accepted", "held receipts remain historical observations");
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
  const handle = child.handle();
  assert.equal((await handle.attempt()).kind, "uncertain");
  assert.equal(handle.row(), null, "invalid Input cannot authenticate a supplied row hint");
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

test("native continuation Return coalesces and waits for the actual Derive settlement", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f), held = parent.holdSettlement();
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity).attempt();
  assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original derivation required");
  assert.equal((await ready.derivation.attempt()).kind, "accepted");
  await until(() => parent.counts().settlements === 1);
  const returning = f.broker.returnNativeDerivation(f.owner, channel, ready.chain);
  assert.equal(f.broker.returnNativeDerivation(f.owner, channel, ready.chain), returning);
  await new Promise<void>(resolve => setImmediate(resolve));
  assert.equal(parent.wasReturned(), false, "a receipt cannot substitute actual settlement");
  assert.ok(f.broker.counts().active > 0);
  held.release(); await returning;
  assert.ok(parent.wasReturned()); assert.ok(ready.derivation.pair()); assert.ok(ready.chain.isCurrent());
  // Exact Return repeats the original idempotent native Settle before closing its Phase.
  assert.deepEqual(parent.counts(), { inputReads: 1, admissions: 1, commits: 1, acknowledgements: 1, settlements: 2 });
});

test("lost ordinary Return retries the same native Derive without recomputation or End", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f);
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity).attempt();
  assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original derivation required");
  assert.equal((await ready.derivation.attempt()).kind, "accepted");
  parent.loseReturn();
  await assert.rejects(f.broker.returnNativeDerivation(f.owner, channel, ready.chain));
  assert.ok(parent.wasReturned());
  const before = parent.counts();
  await f.broker.returnNativeDerivation(f.owner, channel, ready.chain);
  assert.deepEqual(parent.counts(), { ...before, settlements: before.settlements + 1 });
  assert.ok(ready.derivation.pair()); assert.ok(ready.chain.isCurrent());
  assert.equal(parent.finalCounts().retireCalls, 0); assert.equal(parent.planCalls(), 0);
});

test("continuation Return requires the exact original broker owner, channel and chain", async t => {
  const f = await fixture(t), foreign = await fixture(t), channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f);
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity).attempt();
  assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original derivation required");
  const before = f.operations();
  for (const [owner, handle, chain] of [
    [foreign.owner, channel, ready.chain], [f.owner, foreign.channel(), ready.chain],
    [f.owner, channel, Object.freeze({ close() {}, isCurrent: () => true })],
  ] as const) {
    assert.throws(() => f.broker.returnNativeDerivation(owner, handle, chain), /original native derivation/);
  }
  assert.deepEqual(f.operations(), before); assert.equal(parent.wasReturned(), false);
});

test("End cannot collect an ordinary Derive Return while its actual response is outstanding", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f), held = parent.holdReturn();
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity).attempt();
  assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original derivation required");
  assert.equal((await ready.derivation.attempt()).kind, "accepted");
  const returning = f.broker.returnNativeDerivation(f.owner, channel, ready.chain);
  const returned = returning.then(() => "returned", () => "cancelled");
  await until(parent.wasReturned);
  f.owner.retire(); let collected = false;
  const closed = channel.close().then(() => { collected = true; });
  await new Promise<void>(resolve => setImmediate(resolve));
  assert.equal(collected, false); assert.ok(f.broker.counts().bytes > 0);
  held.release(); await returned; await closed;
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.broker.counts().bytes, 0);
});

test("End waits for an admitted ordinary Return's earlier Settle RPC before final cleanup", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f);
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity).attempt();
  assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original derivation required");
  assert.equal((await ready.derivation.attempt()).kind, "accepted");
  await until(() => f.broker.counts().active === 0);
  const held = parent.holdSettlement();
  const returning = f.broker.returnNativeDerivation(f.owner, channel, ready.chain).then(() => "returned", () => "cancelled");
  await until(() => parent.counts().settlements === 2);
  f.owner.retire(); let closed = false;
  const closure = channel.close().then(() => { closed = true; });
  await new Promise<void>(resolve => setImmediate(resolve));
  assert.equal(closed, false); assert.equal(parent.finalCounts().retireCalls, 0);
  held.release(); await returning; await closure;
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.broker.counts().bytes, 0);
});

test("a final-cleanup callback cannot enqueue new ordinary Return after End", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f);
  const ready = await f.broker.nativeDerivationFromChannel(f.owner, channel, f.key, historyPurpose, parent.row, parent.identity).attempt();
  assert.equal(ready.kind, "ready"); if (ready.kind !== "ready") throw new Error("original derivation required");
  assert.equal((await ready.derivation.attempt()).kind, "accepted");
  let fenced = false;
  parent.beforeFinal(() => {
    assert.throws(() => f.broker.returnNativeDerivation(f.owner, channel, ready.chain), /channel closed/);
    fenced = true;
  });
  f.owner.retire(); await channel.close();
  await until(() => f.broker.counts().active === 0);
  assert.ok(fenced); assert.equal(f.broker.counts().bytes, 0);
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
    // The first call is the new close-first producer fence, before final cleanup. Refuse
    // the later actual closure too so the test still pins retained keys across that refusal.
    if (++attempts === 2) throw new Error("native preflight still owns a required original channel row");
    await settle(...args);
  };
  f.owner.retire(); const close = channel.close();
  await until(() => attempts === 2 && f.timers().delays.includes(1000));
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

test("same-Session status progresses while an actual protected SDK provider is held", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const linked = await linkedDerivation(f, channel);
  const provider = f.holdWhen(name => name === "deriveBits"), action = linked.register();
  const attempt = action.attempt(); await until(provider.entered);
  const before = f.counts(), status = channel.status();
  assert.equal(channel.status(), status, "logical status observers coalesce");
  assert.equal((await status).kind, "ready");
  assert.ok(f.counts().tx.length > before.tx.length, "the codec advances without returning the held SDK");
  assert.equal(f.broker.counts().active, 2, "one codec reservation and one actual SDK loan");
  provider.release(); assert.equal((await attempt).kind, "accepted"); await channel.close();
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

test("End joins independent actual SDK and channel cancellation before releasing their custody", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const linked = await linkedDerivation(f, channel), admission = linked.holdAdmit();
  const attempt = linked.register().attempt(); await until(admission.entered);
  const cancellation = f.stallBody(), reads = f.counts().reads;
  const status = channel.status(); await until(() => f.counts().reads === reads + 1);
  const provider = f.holdWhen(name => name === "deriveBits");
  admission.release(); await until(provider.entered);
  const steps = f.counts().steps.length;
  f.owner.retire(); assert.equal((await status).kind, "retired"); assert.equal((await attempt).kind, "retired");
  let closed = false; const close = channel.close().then(() => { closed = true; });
  await until(cancellation.entered);
  assert.equal(closed, false); assert.equal(linked.counts().settlements, 0);
  assert.equal(f.counts().settlements, 0); assert.equal(f.counts().collections, 0);
  cancellation.release(); await flush(); assert.equal(closed, false, "returning IO cannot settle the held provider");
  provider.release(); await close;
  assert.equal(f.counts().steps.length, steps, "End forbids the next SDK operation");
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
    if (command === "read_voice_channel_work") return await f.native.readWork(owner, executor, args.fromSlot as number) as T;
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
  assert.ok(before.active >= 1 && before.active <= 2);
  assert.equal(f.broker.counts().active, before.active); assert.equal(f.broker.counts().bytes, before.bytes);
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

for (const version of [7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 22, 23]) test(`HMR refuses v${version} retained pool rather than creating a second channel allocator`, () => {
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

test("same-Session history progresses during a held SDK without ordinary native admission", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const linked = await linkedDerivation(f, channel), action = linked.register();
  const held = f.holdWhen(name => name === "deriveBits"), attempt = action.attempt(); await until(held.entered);
  const before = f.counts(), request = f.broker.recoverChannelHistory(f.owner, channel, historyPurpose);
  assert.equal(f.broker.recoverChannelHistory(f.owner, channel, historyPurpose), request);
  assert.equal((await request).kind, "status");
  assert.ok(f.counts().tx.length > before.tx.length);
  assert.equal(f.broker.counts().active, 2);
  held.release(); assert.equal((await attempt).kind, "accepted");
  const providerSteps = f.counts().steps.filter(step => step === "deriveBits").length;
  assert.equal((await action.attempt()).kind, "accepted");
  assert.equal(f.counts().steps.filter(step => step === "deriveBits").length, providerSteps);
  assert.equal(f.counts().ordinary, 0); await channel.close();
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
  const sdkLoan = 128 + 3 * 128 + 2 * 128;
  const variable = framing + decodedAndAdopted + engine + 16 * 1024 + sdkLoan;
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
  // One coalesced ordinary-Return promise, fixed chain/record/channel references and
  // its continuation/finally wrappers can overlap the actual Control RPC. No byte copy
  // or new execution lane is created. End drains it before keys/history are unlinked.
  const ordinaryReturn = 512;
  const control = controlBuffers + descriptor + decoded + wrappers + ordinaryReturn;
  assert.ok(control <= 8 * 1024, `terminal Control payload/wrapper envelope ${control}/8192`);
  // Claim/Release fit the same buffers. Its one-flight promise/continuations live in
  // separate chain metadata (counted below), not in this nearly-full serial Control sum.
  const transitionBuffers = 2 * (218 + 16) + 2 * (214 + 16) + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const rawTransitionStrings = 2 * (8 + 32 + 20 + 20 + 64 + 64 + 20 + 64 + 64 + 20 + 64);
  const transitionDecoded = rawTransitionStrings + 512; // raw object/identity, expected/request references and codec wrappers
  assert.ok(transitionDecoded <= decoded, "complete transient custody decode fits existing fixed decoded peak");
  const keyBuffers = 2 * (250 + 16) + 2 * (234 + 16) + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const keyDecoded = 2 * (8 + 32 + 20 + 20 + 64 + 64 + 20 + 64 + 64 + 64 + 64) + 512;
  assert.ok(keyDecoded <= decoded, "key reply transient graph fits the existing decode bound");
  const keyControl = keyBuffers + descriptor + decoded + wrappers + ordinaryReturn;
  assert.ok(keyControl <= 8 * 1024, `key Control graph ${keyControl}/8192`);
  const transitionControl = transitionBuffers + descriptor + transitionDecoded + wrappers + ordinaryReturn;
  assert.ok(transitionControl <= 8 * 1024, `custody Control graph ${transitionControl}/8192`);
  // Opcode13 has the largest new SDK/storage Control request/reply. Its transient decoded
  // identity graph is explicit; its RETAINED phase/flight below has a separate64KiB payment.
  const sdkBuffers = 2 * (194 + 16) + 2 * (210 + 16) + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const sdkDecoded = 2 * (8 + 32 + 20 + 20 + 64 + 64 + 64 + 20 + 64 + 20 + 64) + 512;
  assert.ok(sdkDecoded <= decoded, `SDK custody transient graph ${sdkDecoded}/${decoded}`);
  const sdkControl = sdkBuffers + descriptor + sdkDecoded + wrappers + ordinaryReturn;
  assert.ok(sdkControl <= 8 * 1024, `SDK custody Control graph ${sdkControl}/8192`);
  // Control14 uses the same original194B obligations; its largest reply is85B.
  // The decoded identity+admission and compact retained PC records coexist, so count
  // both rather than moving new PC history into the old nearly-full8KiB chain budget.
  const pcBuffers = 2 * (194 + 16) + 2 * (85 + 16) + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const pcDecoded = 2 * ("admitted".length + 1 + 64 + "complete-sdp-v1".length + 64) + 512;
  assert.ok(pcDecoded <= decoded, `PC intent transient decode ${pcDecoded}/${decoded}`);
  const pcControl = pcBuffers + descriptor + pcDecoded + wrappers + ordinaryReturn;
  assert.ok(pcControl <= 8 * 1024, `PC intent Control graph ${pcControl}/8192`);
  const sdkCompactStrings = 2 * (7 + 64 + 8 + 20 + 64);
  const pcCompactStrings = 2 * (1 + 64 + "complete-sdp-v1".length + 64);
  // Class, immutable identity/admission wrappers, finite deadline and cleanup scalars.
  // startPc shares the existing sdpFlight/lane/finally, counted once in sdkPhase below.
  const pcFixed = pcCompactStrings + 256 + 128 + 128 + 64;
  const sdkPhase = sdkCompactStrings + 512 + 512 + 512 + 128 + pcFixed;
  assert.ok(sdkPhase <= 4096, `independently prepaid SDP/PC phase/flight graph ${sdkPhase}/4096`);
  // This fixed subtree is part of the new64KiB, never the nearly-full old chain8KiB.
  // Future SDK/SDP/UTF16/crypto owners still need a complete coexistence inventory.
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
  const ordinaryReturn = 512; // same one retained completion/closure graph, no data buffers
  // The retained custody reply shares parent/action/handshake/epoch with original chain
  // objects (transitionRpc explicitly canonicalizes them). Only these strings are new.
  const custodyStrings = 2 * (8 + 64 + 64 + 20 + 64); // state, Finish ID/hash, opaque until, receipt
  const custodyObjects = 256 + 64; // retained receipt + custody scalars/reference wrapper
  const transitionFlight = 512; // one coalesced promise, async frame/closures and channel/chain references
  const metadata = parentSelector + sharedBinding + 2 * 128 + 4096 + 2 * 128 + ordinaryReturn
    + custodyStrings + custodyObjects + transitionFlight
    // Compact key history owns only state/proof. Original parent/Finish/Claim identities
    // stay in the old receipt; the flag word fits its existing64B custody wrapper.
    + 2 * 64 + 64;
  assert.ok(metadata <= 8 * 1024, `chain metadata/maps/drainage inventory ${metadata}/8192`);

  // The current child can overlap frame ciphertext/opened body, decoder-owned input and
  // adopted immutable input. Count all four full-sized buffers even though the decoder
  // copies only AAD/body and registration zeros its temporary arrays before the engine.
  const transcript = 1024, retained = 2 * (130 + transcript) + 9 * (2048 - 430);
  const largestRequest = 154 + 82 + 528;
  const framing = 2 * (largestRequest + 16) + 4 * 2064 + 144 + 288 + 4 * 40 + 2 * 32 + 2 * 12 + 1;
  const sdkLoan = 128 + 3 * 128 + 2 * 128;
  const variable = retained + framing + 16 * 1024 + 16 * 1024 + sdkLoan;
  assert.ok(variable <= 96 * 1024, `child Data/engine/retained-input inventory ${variable}/98304`);
});

for (const { loseFinalAck, lateChild, alterAck } of [
  { loseFinalAck: false, lateChild: false, alterAck: null }, { loseFinalAck: true, lateChild: false, alterAck: null },
  { loseFinalAck: false, lateChild: true, alterAck: null }, { loseFinalAck: false, lateChild: false, alterAck: "proof" },
  { loseFinalAck: false, lateChild: false, alterAck: "pending" },
]) test(`End discovers unseen work without Input/SDK; lost ACK=${loseFinalAck}, late child=${lateChild}, altered ACK=${alterAck}`, async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const row = "00000000000000000000000000000001";
  const identity = { action: "42", version: "1", inputTag: hash(42) };
  const proof = hash(43);
  let settled = false, returned = false, final = false, acknowledged = false, lose = loseFinalAck;
  let reads = 0, closeAttempts = 0;
  let altered = false, planRequests = 0, finalRequests = 0;
  let childAppeared = false, childClosed = false;
  const childRow = "00000000000000010000000000000001";
  const childStatus = () => { const body = Buffer.alloc(242); body[0] = 1; Buffer.from(childRow, "hex").copy(body, 2);
    body[18] = childClosed ? 3 : 1; return { kind: 0 as const, body }; };
  const held = gate(); f.cleanup(held.release);
  const metadata = { row, purpose: { kind: "derive", ...historyPurpose }, state: "attached", actorHandoff: null, identity };
  f.native.readWork = async (owner, executor, fromSlot) => {
    reads++; if (reads === 1) await held.promise;
    const child = { row: childRow, state: "pending", actorHandoff: null, identity: null,
      purpose: { kind: "confirmation", parent: row, parentPurpose: metadata.purpose, request: "start", predecessor: null } };
    return { kind: "snapshot", owner, executor, nextSlot: fromSlot === 0 ? 1 : fromSlot === 1 && childAppeared ? 2 : 32,
      work: fromSlot === 0 ? metadata : fromSlot === 1 && childAppeared ? child : null };
  };
  const status = () => {
    const body = Buffer.alloc(242); body[0] = 1; Buffer.from(row, "hex").copy(body, 2); body[19] = 1;
    integer(42n, 8).copy(body, 20); integer(1n, 8).copy(body, 28); Buffer.from(identity.inputTag, "hex").copy(body, 36);
    body[68] = Number(settled); body[209] = Number(returned); f.historyBytes(body);
    return { kind: 0 as const, body };
  };
  status();
  const originalClose = f.native.settleSetup;
  f.native.settleSetup = async (...args) => {
    closeAttempts++;
    if (!acknowledged) throw new Error("original unseen parent still owns native work");
    await originalClose(...args);
  };
  f.actions(async (header, opened) => {
    if (opened[0] === 2) {
      assert.equal(header.logical, header.operation === 8 ? "0".repeat(32) : childRow);
      if (header.operation === 8) { assert.equal(opened.at(-1), 0); return childStatus(); }
      assert.equal(header.operation, 6); childClosed = true; return childStatus();
    }
    assert.equal(header.logical, row);
    assert.ok(closeAttempts > 0, "native close-first fence precedes unseen action cleanup");
    assert.ok(![1, 2, 3].includes(header.operation), "cleanup must never fetch Input, admit or compute");
    if (header.operation === 5) { settled = true; return status(); }
    if (header.operation === 6 && opened[105] === 0) { assert.ok(settled); returned = true; return status(); }
    if (header.operation === 6) {
      planRequests++;
      assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "close-plan" }).body);
      if (lateChild && !childAppeared) { childAppeared = true; return { kind: 0, body: new Uint8Array([1, 3, 7]) }; }
      if (lateChild) assert.ok(childClosed, "late child must close before the retained parent can retry finalization");
      assert.ok(returned); final = true;
    } else {
      finalRequests++;
      assert.equal(header.operation, 7); assert.ok(final);
      assert.deepEqual(opened, encodeVoiceIpcAction(historyPurpose, { kind: "final", proof }).body);
      acknowledged = true;
      if (lose) { lose = false; throw new Error("actual native final ACK reply lost"); }
      if (alterAck && !altered) {
        altered = true;
        return { kind: 0, body: Buffer.concat([Buffer.from([1, 4]), Buffer.from(row, "hex"), integer(42n, 8), integer(1n, 8),
          Buffer.from(identity.inputTag, "hex"), Buffer.from([2, alterAck === "pending" ? 0 : 2]),
          Buffer.from(alterAck === "pending" ? "0".repeat(64) : hash(44), "hex")]) };
      }
    }
    return { kind: 0, body: Buffer.concat([Buffer.from([1, 4]), Buffer.from(row, "hex"), integer(42n, 8), integer(1n, 8),
      Buffer.from(identity.inputTag, "hex"), Buffer.from([2, acknowledged ? 2 : 1]), Buffer.from(proof, "hex")]) };
  });
  const funding = f.broker.counts().bytes;
  const engineSteps = f.counts().steps.filter(step => step === "deriveBits").length;
  f.owner.retire(); let closed = false; const closing = channel.close().then(() => { closed = true; });
  await until(() => reads === 1);
  assert.equal(closed, false); assert.equal(f.counts().collections, 0);
  assert.equal(f.broker.counts().bytes, funding, "parked public metadata read retains original payment");
  held.release();
  if (lateChild) {
    await until(() => childAppeared && f.timers().delays.includes(1000));
    f.advance(1000);
  }
  if (loseFinalAck) {
    await until(() => acknowledged && f.timers().delays.includes(1000));
    assert.equal(f.counts().collections, 0); f.advance(1000);
  }
  if (alterAck) {
    await until(() => altered && f.timers().delays.includes(1000));
    assert.equal(f.counts().collections, 0, "changed proof/Pending cannot replace an already-issued final proof");
    f.advance(1000);
  }
  await closing;
  assert.ok(settled && returned && final && acknowledged);
  assert.equal(childClosed, lateChild);
  if (alterAck || loseFinalAck) {
    assert.equal(planRequests, 1, "issued proof survives retries without reissuance");
    assert.equal(finalRequests, 2, "retry only the exact original final ACK");
  }
  assert.equal(f.counts().steps.filter(step => step === "deriveBits").length, engineSteps);
  assert.equal(f.counts().collections, 1);
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.broker.counts().bytes, 0);
});

for (const attaches of [false, true]) test(`End closes an unseen child; Pending attaches before authenticated history=${attaches}`, async t => {
  const f = await fixture(t), child = await framedConfirmation(f);
  child.nativeOnly();
  assert.equal((await child.derive()).kind, "accepted");
  await f.broker.returnNativeDerivation(f.owner, child.channel, child.ready.chain);
  if (!attaches) child.beginMode("pending");
  const p = child.purpose;
  const request = p.request;
  assert.equal(request.kind, "start");
  if (request.kind !== "start") throw new Error("original Start required");
  const descriptor = { row: child.row, state: "pending", actorHandoff: null, identity: null,
    purpose: { kind: "confirmation", parent: p.parentRow, parentPurpose: { kind: "derive", ...p.parent },
      request: "start", predecessor: request.predecessor } };
  f.native.readWork = async (owner, executor, fromSlot) => ({ kind: "snapshot", owner, executor,
    nextSlot: fromSlot <= 1 ? 2 : 32, work: fromSlot <= 1 ? descriptor : null });
  child.parent.beforeFinal(() => {
    assert.equal(child.counts().returns, 1, "the actual unseen child closes before parent final issuance");
  });
  f.owner.retire(); await child.channel.close(); await until(() => f.broker.counts().active === 0);
  assert.deepEqual(child.begins(), [false], "metadata hints cannot request new native work");
  assert.deepEqual(child.counts(), { inputs: 0, admissions: 0, results: 0, acknowledgements: 0,
    settlements: Number(attaches), returns: 1 });
  assert.equal(f.broker.counts().bytes, 0);
});

test("deferred parent, current cleanup descriptor and serial projection fit the separate16KiB prepayment", () => {
  // Concrete payload bounds plus conservative wrappers. Native caps JSON to2304B; allow
  // its UTF16 string, raw parsed graph AND the decoded graph, original purpose copies,
  // proof/row strings, two asynchronous frames and map/closure/Promise scratch. Encrypted
  // request/response buffers retain their existing separate8KiB Control payment.
  const largest = { kind: "snapshot", owner: "voice-ffffffffffffffff", executor: hash(1), nextSlot: 32,
    work: { row: "000000000000001fffffffffffffffff", state: "attached", actorHandoff: null,
      purpose: { kind: "confirmation", parent: "000000000000001effffffffffffffff",
        parentPurpose: { kind: "derive", peer: hash(2), join: hash(3), keyRef: hash(4), epoch: "18446744073709551615" },
        request: "start", predecessor: hash(5) }, identity: { action: "18446744073709551615", version: "18446744073709551615", inputTag: hash(6) } } };
  const strings = (value: unknown): number => typeof value === "string" ? value.length * 2
    : value && typeof value === "object" ? Object.entries(value).reduce((n, [key, item]) => n + key.length * 2 + strings(item), 0) : 8;
  const json = 2304, stringAndGraphs = json * 2 + 3 * strings(largest);
  const wrappers = 4096, originalPurposeAndProof = (3 * 64 + 20 + 32 + 64) * 2;
  assert.ok(JSON.stringify(largest).length <= json);
  assert.ok(stringAndGraphs + wrappers + originalPurposeAndProof <= 16 * 1024);
  // Live pumping and cleanup do not overlap. Live scan can retain one decoded parent
  // while reading raw+decoded child metadata; opaque handles share the paid chain graph.
  const liveParent = { ...largest.work, purpose: largest.work.purpose.parentPurpose };
  const livePump = json * 2 + 2 * strings(largest) + strings(liveParent) + wrappers + 1024;
  assert.ok(livePump <= 16 * 1024, `serial live metadata/parent/handle inventory ${livePump}/16384`);
  assert.ok(json < stringAndGraphs);
  const source = readFileSync(new URL("./voice-crypto.ts", import.meta.url), "utf8");
  assert.match(source, /const IPC_WORK_BYTES = 16 \* 1024;/);
});

test("native work observation requires exact selected key and authenticated Ready and coalesces irreversibly", async t => {
  const f = await fixture(t), channel = f.channel();
  assert.throws(() => f.broker.observeNativeWork(f.owner, channel, f.key), /authenticated ready/);
  await channel.status();
  assert.throws(() => f.broker.observeNativeWork(f.owner, channel, { publicKey: () => f.key.publicKey() }), /original selected/);
  assert.throws(() => f.broker.observeNativeWork(f.owner, { ...channel }, f.key), /original selected/);
  const observer = f.broker.observeNativeWork(f.owner, channel, f.key);
  assert.equal(f.broker.observeNativeWork(f.owner, channel, f.key), observer);
  await observer.close(); const reads = f.bodyReads().length;
  assert.equal(f.broker.observeNativeWork(f.owner, channel, f.key), observer);
  f.advance(5000); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(f.bodyReads().length, reads); assert.equal(f.timers().pending, 0);
});

test("actual channel pump consumes native Derive without caller row/purpose and preserves original Return", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const parent = await framedDerivation(f);
  const before = f.counts().steps.filter(step => step === "deriveBits").length;
  f.native.readWork = async (owner, executor, fromSlot) => ({ kind: "snapshot", owner, executor,
    nextSlot: fromSlot === 0 ? 1 : 32, work: fromSlot === 0 ? {
      row: parent.row, purpose: { kind: "derive", ...historyPurpose }, state: "attached", actorHandoff: null, identity: parent.identity,
    } : null });
  const observer = f.broker.observeNativeWork(f.owner, channel, f.key);
  await until(() => f.timers().delays.includes(250)); f.advance(250);
  await until(parent.wasReturned); await observer.close();
  assert.deepEqual(parent.counts(), { inputReads: 1, admissions: 1, commits: 1, acknowledgements: 1, settlements: 2 });
  assert.equal(f.counts().steps.filter(step => step === "deriveBits").length, before + 1, "exactly one original pair derivation");
  f.native.readWork = async (owner, executor) => ({ kind: "snapshot", owner, executor, nextSlot: 32, work: null });
  f.owner.retire(); await channel.close();
});

test("channel End waits for a held pump metadata read before starting cleanup or collecting", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const held = gate(); f.cleanup(held.release); let reads = 0, closed = false;
  f.native.readWork = async (owner, executor) => { reads++; if (reads === 1) await held.promise;
    return { kind: "snapshot", owner, executor, nextSlot: 32, work: null }; };
  f.broker.observeNativeWork(f.owner, channel, f.key); await until(() => reads === 1);
  const funding = f.broker.counts().bytes;
  f.owner.retire(); const closing = channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.equal(f.counts().settlements, 0); assert.equal(f.counts().collections, 0);
  assert.equal(f.broker.counts().bytes, funding);
  held.release(); await closing; await until(() => f.broker.counts().active === 0);
  assert.equal(f.broker.counts().bytes, 0);
});

for (const end of [false, true]) test(`pump admission rechecks owner reentry; End=${end}`, async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  let nested: ReturnType<typeof f.broker.observeNativeWork> | null = null, reads = 0;
  const original = f.native.readWork;
  f.native.readWork = async (...args) => { reads++; return original(...args); };
  // Two predicates are sampled during admission. Reenter the LAST one to pin the
  // check-then-publish boundary rather than merely exercising an earlier Ready check.
  f.onOwnerCheck(() => f.onOwnerCheck(() => {
    if (end) void channel.close();
    else nested = f.broker.observeNativeWork(f.owner, channel, f.key);
  }));
  if (end) {
    assert.throws(() => f.broker.observeNativeWork(f.owner, channel, f.key), /authenticated ready/);
    f.owner.retire(); await channel.close(); await until(() => f.broker.counts().active === 0);
    assert.equal(reads, 0);
  } else {
    const outer = f.broker.observeNativeWork(f.owner, channel, f.key);
    assert.equal(outer, nested);
    await until(() => reads > 0); assert.equal(reads, 1);
    await outer.close(); assert.equal(f.timers().pending, 0);
  }
});

test("pump observes A's exact Return after a remounted caller has adopted B", async t => {
  const f = await fixture(t), child = await framedConfirmation(f);
  assert.equal((await child.derive()).kind, "accepted");
  let reads = 0;
  const parentWork = { row: child.parent.row, purpose: { kind: "derive", ...historyPurpose },
    state: "attached", actorHandoff: null, identity: child.parent.identity };
  const childWork = { row: child.row, purpose: { kind: "confirmation", parent: child.parent.row,
    parentPurpose: parentWork.purpose, request: "start", predecessor: null },
    state: "attached", actorHandoff: null, identity: child.identity };
  f.native.readWork = async (owner, executor, fromSlot) => { reads++;
    return { kind: "snapshot", owner, executor, nextSlot: fromSlot < 2 ? fromSlot + 1 : 32,
      work: fromSlot === 0 ? parentWork : fromSlot === 1 ? childWork : null }; };
  const observer = f.broker.observeNativeWork(f.owner, child.channel, f.key);
  await until(() => f.timers().delays.includes(250)); f.advance(250);
  // Read-only observation places the other caller exactly after pump adoption but
  // before its next attempt. The original production handles perform all actual work.
  const internals = f.broker as unknown as { ipcChannels: Map<string, { negotiation: { workPump: { child: { handle: unknown } | null } } }> };
  await until(() => internals.ipcChannels.get(f.owner.token)?.negotiation.workPump.child !== null &&
    f.timers().delays.includes(250));
  const a = child.handle(); assert.equal((await a.attempt()).kind, "accepted");
  const b = child.nextHistorical(); assert.notEqual(a, b);
  const before = reads; f.advance(250);
  await until(() => reads > before);
  assert.equal(internals.ipcChannels.get(f.owner.token)?.negotiation.workPump.child, null,
    "latest B cannot hide the retained A completion");
  await observer.close();
  f.native.readWork = async (owner, executor) => ({ kind: "snapshot", owner, executor, nextSlot: 32, work: null });
  f.owner.retire(); await child.channel.close();
});

test("End during an automatically consumed child waits for its actual held settlement", async t => {
  const f = await fixture(t), child = await framedConfirmation(f);
  assert.equal((await child.derive()).kind, "accepted");
  const held = child.holdSettlement();
  const parentWork = { row: child.parent.row, purpose: { kind: "derive", ...historyPurpose },
    state: "attached", actorHandoff: null, identity: child.parent.identity };
  const childWork = { row: child.row, purpose: { kind: "confirmation", parent: child.parent.row,
    parentPurpose: parentWork.purpose, request: "start", predecessor: null },
    state: "attached", actorHandoff: null, identity: child.identity };
  f.native.readWork = async (owner, executor, fromSlot) => ({ kind: "snapshot", owner, executor,
    // Known-child drainage must work without discovery supplying another descriptor.
    // The separate unseen-work tests cover post-End snapshots and late attachments.
    nextSlot: f.owner.isCurrent() && fromSlot < 2 ? fromSlot + 1 : 32,
    work: !f.owner.isCurrent() ? null : fromSlot === 0 ? parentWork : fromSlot === 1 ? childWork : null });
  f.broker.observeNativeWork(f.owner, child.channel, f.key);
  await until(() => f.timers().delays.includes(250)); f.advance(250);
  await until(() => child.parent.wasReturned() && f.timers().delays.includes(250)); f.advance(250);
  await until(() => child.counts().settlements > 0);
  const payment = f.broker.counts().bytes, unusedRows = f.rowBudget().reserved; let closed = false;
  f.owner.retire(); const closing = child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.equal(f.counts().settlements, 0); assert.equal(f.counts().collections, 0);
  // End may refund never-used row claims. The actual running child, chain variable
  // envelope and original channel/control owners must remain charged through settlement.
  assert.equal(f.broker.counts().bytes, payment - unusedRows * 2048);
  held.release(); await closing; await until(() => f.broker.counts().active === 0);
  assert.equal(f.broker.counts().bytes, 0);
});


test("uncertain key adoption beyond old expiry preserves the identical pair and drains old progress", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive(); const pair = child.ready.derivation.pair(); assert.ok(pair);
  custody.loseAdoption(); await child.handle().attempt();
  await until(() => custody.keyCounts().adoptions === 1 && f.broker.counts().active === 0);
  const retained = f.broker.counts().bytes;
  assert.ok(retained < 96 * 1024, "old variable envelope actually returned while key/history custody remains paid");
  f.advance(60_000); f.broker.collectChains();
  assert.equal(child.ready.derivation.pair(), pair, "unknown adoption cannot reconstruct or retire the original key domain");
  assert.throws(() => child.nextHistorical(), /original derived pair required/, "retention grants no old action");
  f.owner.retire(); await child.channel.close();
  assert.deepEqual(custody.keyCounts(), { adoptions: 2, releases: 1, adopted: true, released: true });
  assert.equal(child.ready.derivation.pair(), null); assert.equal(child.parent.finalCounts().finalAcknowledged, true);
});

test("held adoption reply past old deadline keeps existing keys but actual Control remains occupied", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive(); const pair = child.ready.derivation.pair(); assert.ok(pair);
  const held = custody.holdAdoption(); await child.handle().attempt();
  await until(() => custody.keyCounts().adoptions === 1);
  f.advance(60_000); f.broker.collectChains();
  assert.equal(child.ready.derivation.pair(), pair);
  assert.ok(f.broker.counts().active > 0, "an unresolved real IO promise cannot be refunded by expiry");
  f.owner.retire(); const closing = child.channel.close();
  assert.equal(child.ready.derivation.pair(), null, "End fences keys before the delayed Current reply");
  held.release(); await closing;
  assert.equal(custody.keyCounts().releases, 1); assert.equal(child.ready.derivation.pair(), null);
});

test("End during authenticated Current key reply cannot revive keys and still releases both obligations", async t => {
  const f = await fixture(t), child = await framedConfirmation(f, true), custody = custodyReplies(f, child);
  await child.derive(); custody.endOnKeyReply(); await child.handle().attempt();
  await until(() => custody.keyCounts().adoptions === 1);
  await child.channel.close();
  assert.equal(child.ready.derivation.pair(), null);
  assert.equal(custody.keyCounts().releases, 1); assert.equal(custody.counts().releases, 1);
  assert.equal(child.parent.finalCounts().finalAcknowledged, true);
});


async function establishedSdpFixture(t: TestContext, localIsLow = true,
    original: Awaited<ReturnType<typeof fixture>> | null = null, serial = 1) {
  const f = original ?? await fixture(t), child = await framedConfirmation(f, true, localIsLow, serial), custody = custodyReplies(f, child);
  await child.derive(); await child.handle().attempt();
  await until(() => custody.keyCounts().adopted && f.broker.counts().active === 0);
  const prepare = () => f.broker.prepareNativeSdpHandoff(f.owner, child.channel, child.ready.chain);
  return { f, child, custody, prepare };
}

test("browser SDP subtree is paid once before admission and uses independent SDK/storage releases", async t => {
  const { f, child, custody, prepare } = await establishedSdpFixture(t);
  const before = f.broker.counts().bytes;
  await prepare(); assert.equal(f.broker.counts().bytes, before + 64 * 1024);
  await prepare(); assert.deepEqual(custody.effectCounts(), { storagePrepares: 1, storageReleases: 0, sdkAdmissions: 1, sdkReleases: 0 });
  f.owner.retire(); await child.channel.close();
  assert.deepEqual(custody.effectOrder(), ["storage-prepare", "sdk-admit", "sdk-release", "storage-release", "key-release"]);
  assert.equal(f.broker.counts().bytes, 0);
});

for (const lost of ["storage", "sdk"] as const) test(`lost ${lost} admission retains original paid intent and recovers after End`, async t => {
  const { f, child, custody, prepare } = await establishedSdpFixture(t);
  const before = f.broker.counts().bytes;
  if (lost === "storage") custody.loseStorage(); else custody.loseSdk();
  await prepare(); assert.equal(f.broker.counts().bytes, before + 64 * 1024);
  f.owner.retire(); await child.channel.close();
  assert.deepEqual(custody.effectCounts(), { storagePrepares: lost === "storage" ? 2 : 1, storageReleases: 1,
    sdkAdmissions: lost === "sdk" ? 2 : 0, sdkReleases: lost === "sdk" ? 1 : 0 });
  assert.equal(f.broker.counts().bytes, 0);
});

test("End cannot refund held SDK admission IO or release native custody before its actual finally", async t => {
  const { f, child, custody, prepare } = await establishedSdpFixture(t);
  const hold = custody.holdSdk(), pending = prepare();
  await until(() => custody.effectCounts().sdkAdmissions === 1);
  const payment = f.broker.counts().bytes; let closed = false;
  f.owner.retire(); const closing = child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.equal(f.broker.counts().bytes, payment);
  assert.equal(custody.effectCounts().sdkReleases, 0); assert.equal(f.counts().collections, 0);
  hold.release(); await pending; await closing;
  assert.equal(custody.effectCounts().sdkReleases, 1); assert.equal(f.broker.counts().bytes, 0);
});

test("lost SDK Release preserves payment and exact proof before storage/key release", async t => {
  const { f, child, custody, prepare } = await establishedSdpFixture(t);
  await prepare(); custody.loseSdkRelease();
  f.owner.retire(); const closing = child.channel.close();
  await until(() => custody.effectCounts().sdkReleases === 1 && f.timers().delays.includes(1000));
  assert.equal(custody.effectCounts().storageReleases, 0); assert.equal(custody.keyCounts().releases, 0);
  assert.ok(f.broker.counts().bytes >= 64 * 1024);
  f.advance(1000); await closing;
  assert.equal(custody.effectCounts().sdkReleases, 2); assert.equal(f.broker.counts().bytes, 0);
});

test("HMR refuses fully shaped funded broker24 without waking or replacing its outstanding pool", async t => {
  const { f, child, prepare } = await establishedSdpFixture(t); await prepare();
  const previous = Object.create(f.broker); Object.defineProperty(previous, "version", { value: 24 });
  let wakes = 0; previous.retrySelectedJoinKeys = () => { wakes++; };
  const key = Symbol.for("mewtual.voice.crypto-broker.v2"), host = { [key]: previous }, bytes = f.broker.counts().bytes;
  assert.throws(() => windowVoiceCryptoBroker({ async admit() { return ""; }, async settle() {}, async commitInbound() {} }, host, real), /reload required/);
  assert.equal(host[key], previous); assert.equal(wakes, 0); assert.equal(f.broker.counts().bytes, bytes);
  f.owner.retire(); await child.channel.close();
});


async function establishedPcFixture(t: TestContext, localIsLow = true,
    original: Awaited<ReturnType<typeof fixture>> | null = null, serial = 1) {
  const fixture = await establishedSdpFixture(t, localIsLow, original, serial); await fixture.prepare();
  const { f, child } = fixture;
  return { ...fixture, admitPc: () => f.broker.prepareNativePcIntent(f.owner, child.channel, child.ready.chain) };
}

/** Original broker and real channel/SDK crypto; native authority is still an independent
 * mock. The actual Rust TCP gates establish Root/native behavior separately, not a WebView
 * or an interoperating browser/native process. This fixture never grants RTC delivery. */
async function framedSdpOffer(t: TestContext, localIsLow = true, plaintextLength = 8192, authenticationFails = false,
    shared: Awaited<ReturnType<typeof establishedPcFixture>> | null = null, answer = false) {
  const f = shared ?? await establishedPcFixture(t, localIsLow); if (!shared) await f.admitPc();
  const localSeal = answer ? !localIsLow : localIsLow;
  const { child, custody } = f;
  const parentRow = child.parent.row, row = (answer ? "0000000000000003" : "0000000000000002") + parentRow.slice(16);
  const identity = { action: ((answer ? 9007199254741102n : 9007199254741101n) + (BigInt("0x" + parentRow.slice(16)) - 1n) * 1000n).toString(), version: "1", inputTag: hash(110) };
  const original = { parentRow, identity: child.parent.identity, handshake: child.parent.binding.handshake,
    key: hash(99), sequence: "1", storage: hash(100), sdkGeneration: "1", sdkProof: hash(101), pcProof: hash(102) };
  const selector = { parentRow, row, purpose: localSeal ? "seal" as const : "open" as const };
  const plaintext = new Uint8Array(plaintextLength).fill(115), aad = new Uint8Array(579).fill(4);
  const incoming = localSeal ? plaintext : await child.parent.incoming(plaintext, aad, (answer ? 43n : 42n));
  if (authenticationFails) incoming[0] ^= 1;
  const engineBytes = 4 * (aad.length + incoming.length + (localSeal ? 16 : 0)) + 3 * 8192 + 2048;
  const input = Buffer.alloc(277 + aad.length + incoming.length);
  input.set([8, 3]); Buffer.from(row, "hex").copy(input, 2); Buffer.from(parentRow, "hex").copy(input, 18);
  integer(BigInt(identity.action), 8).copy(input, 34); integer(1n, 8).copy(input, 42); Buffer.from(identity.inputTag, "hex").copy(input, 50);
  integer(1n, 8).copy(input, 82); Buffer.from(hash(103), "hex").copy(input, 90); input[122] = 1;
  Buffer.from(original.key, "hex").copy(input, 123); integer(1n, 8).copy(input, 155); Buffer.from(original.sdkProof, "hex").copy(input, 163);
  Buffer.from(original.storage, "hex").copy(input, 195); Buffer.from(original.pcProof, "hex").copy(input, 227);
  integer((answer ? 43n : 42n), 8).copy(input, 259); integer(BigInt(engineBytes), 4).copy(input, 267); input[271] = Number(!localSeal); input[272] = Number(answer);
  integer(BigInt(aad.length), 2).copy(input, 273); integer(BigInt(incoming.length), 2).copy(input, 275); input.set(aad, 277); input.set(incoming, 277 + aad.length);
  const counts = { begin: 0, resolve: 0, input: 0, admit: 0, result: 0, settle: 0, ack: 0, returned: 0, deliveryAdmit: 0, deliveryApplied: 0 };
  let began = false, settled = false, acknowledged = false, returned = false, pendingBegin = false, attached = false;
  let output: Uint8Array | null = null, outputTag: string | null = null, resultRecorded = false;
  let deliveryState = 1, busyDelivery = false;
  let lose: keyof typeof counts | null = null, hold: keyof typeof counts | null = null, held = gate();
  let onInput: (() => void) | null = null, onReturn: (() => void) | null = null;
  let badInput = false, busyBegin = false;
  f.f.cleanup(() => held.release());
  const status = () => {
    const bytes = Buffer.alloc(220); bytes.set([8, 0]); Buffer.from(row, "hex").copy(bytes, 2); Buffer.from(parentRow, "hex").copy(bytes, 18); bytes[34] = Number(!localSeal);
    bytes[35] = attached ? 0 : 1; bytes[36] = Number(returned); bytes[37] = Number(attached);
    if (attached) { integer(BigInt(identity.action), 8).copy(bytes, 38); integer(1n, 8).copy(bytes, 46); Buffer.from(identity.inputTag, "hex").copy(bytes, 54); }
    bytes[86] = Number(settled); bytes[87] = Number(resultRecorded);
    if (resultRecorded) {
      Buffer.from(outputTag!, "hex").copy(bytes, 88);
      if (authenticationFails) { bytes[120] = 3; bytes[121] = 0; } // Root's closed rejection is Malformed.
      else if (localSeal) { bytes[120] = 1; Buffer.from(hash(111), "hex").copy(bytes, 121); Buffer.from(hash(112), "hex").copy(bytes, 153); }
      else { bytes[120] = 2; bytes[219] = deliveryState; }
      bytes[185] = Number(settled && acknowledged); Buffer.from(hash(113), "hex").copy(bytes, 187);
    }
    bytes[186] = Number(acknowledged); return { kind: 0 as const, body: bytes };
  };
  const step = async (name: keyof typeof counts) => {
    counts[name]++; if (hold === name) await held.promise;
    if (lose === name) { lose = null; throw new Error("native reply lost after effect"); }
  };
  const respond: NonNullable<Parameters<typeof custody.sdpReplies>[0]> = async (header, opened) => {
    if (header.operation === 15 || header.operation === 16) {
      assert.deepEqual(f.f.rowBudget(), { reserved: 2, actions: 2 }, "both global rows precede unknown Begin");
      assert.deepEqual(opened, encodeVoiceSdpBegin(original, localSeal ? plaintext : null).body);
      if (busyBegin) { counts.begin++; return { kind: 0, body: new Uint8Array([8, 2, 2, 7]) }; }
      began = true; attached = !pendingBegin; await step("begin"); return status();
    }
    if (header.operation === 27) {
      const close = opened[226] === 1;
      assert.deepEqual(opened, encodeVoiceSdpResolve(original, selector.purpose, close).body); await step("resolve");
      if (!began) return { kind: 0, body: Buffer.concat([Buffer.from([8, 1]), Buffer.from(parentRow, "hex"), Buffer.from([Number(!localSeal), Number(close)])]) };
      return status();
    }
    assert.equal(header.logical, row);
    if (header.operation === 17) {
      assert.deepEqual(opened, encodeVoiceSdpAction(selector, { kind: "input", identity }).body);
      onInput?.();
      await step("input"); const body = Buffer.from(input); if (badInput) body[227] ^= 1; return { kind: 2, body };
    }
    if (header.operation === 18) {
      assert.deepEqual(opened, encodeVoiceSdpAction(selector, { kind: "admit", identity, engineBytes }).body);
      await step("admit"); return { kind: 0, body: Buffer.concat([Buffer.from([8, 4]), Buffer.from(row, "hex"),
        integer(BigInt(identity.action), 8), integer(1n, 8), Buffer.from(identity.inputTag, "hex"), integer(BigInt(identity.action), 8), integer(BigInt(engineBytes), 4)]) };
    }
    if (header.operation === 19 || header.operation === 20) {
      const bytes = opened.subarray(100), tag = Buffer.from(opened.subarray(66, 98)).toString("hex");
      assert.deepEqual(opened, encodeVoiceSdpAction(selector, { kind: localSeal ? "sealed" : "opened", identity, bytes, tag }).body);
      assert.equal(tag, createHash("sha256").update(bytes).digest("hex")); assert.equal(bytes.length, plaintextLength + (localSeal ? 16 : 0));
      if (!localSeal) assert.deepEqual(bytes, plaintext);
      if (output) assert.deepEqual(bytes, output); else { output = bytes.slice(); outputTag = tag; }
      resultRecorded = true;
      await step("result"); return status();
    }
    if (header.operation === 21) {
      assert.equal(authenticationFails, true); assert.deepEqual(opened, encodeVoiceSdpAction(selector, { kind: "authentication-failed", identity }).body);
      resultRecorded = true; outputTag = createHash("sha256").update("mewtual/voice/native-sdp-aead-failure/v1\0").update(Buffer.from(identity.inputTag, "hex")).digest("hex");
      await step("result"); return status();
    }
    if (header.operation === 22) {
      assert.deepEqual(opened, encodeVoiceSdpAction(selector, { kind: "ack", identity, digest: hash(113) }).body);
      acknowledged = true; await step("ack"); return status();
    }
    if (header.operation === 23) {
      assert.deepEqual(opened, encodeVoiceSdpAction(selector, { kind: "settle", identity }).body);
      settled = true; await step("settle"); return status();
    }
    if (header.operation === 26) {
      assert.ok(!localSeal && resultRecorded && !authenticationFails);
      if (busyDelivery) return { kind: 0, body: new Uint8Array([8, 2, 2, 7]) };
      const applied = opened.at(-1) === 1;
      assert.deepEqual(opened, encodeVoiceSdpAction(selector, { kind: "delivery", identity, applied }).body);
      if (applied) { assert.ok(deliveryState >= 2); deliveryState = 3; await step("deliveryApplied"); }
      else { deliveryState = Math.max(deliveryState, 2); await step("deliveryAdmit"); }
      return status();
    }
    assert.equal(header.operation, 24); assert.ok(settled && (!resultRecorded || acknowledged));
    assert.deepEqual(opened, encodeVoiceSdpAction(selector, { kind: "return" }).body);
    returned = true; onReturn?.(); await step("returned"); return status();
  };
  custody.sdpReplies(respond);
  const start = (text: string | null = null) => f.f.broker.prepareNativeSdpCrypto(f.f.owner, child.channel, child.ready.chain, text);
  const rtc = () => f.f.broker.prepareNativeSdpRtc(f.f.owner, child.channel, child.ready.chain);
  return { ...f, counts, start, rtc, respond, busyDelivery(value: boolean) { busyDelivery = value; }, lose(name: keyof typeof counts) { lose = name; },
    pendingBegin(value: boolean) { pendingBegin = value; },
    onInput(work: () => void) { onInput = work; }, onReturn(work: () => void) { onReturn = work; },
    malformedInput(value: boolean) { badInput = value; }, busyBegin(value: boolean) { busyBegin = value; },
    historical() { attached = true; settled = true; acknowledged = true; resultRecorded = true; outputTag = hash(114); deliveryState = 3; },
    hold(name: keyof typeof counts) { hold = name; return held; },
    cryptoCount: () => f.f.counts().steps.filter(name => name === "encrypt").length - f.f.operations().length,
  };
}

test("maximum browser SDP runs through original authenticated IPC and retries Result without another SDK", async t => {
  const f = await framedSdpOffer(t); const before = f.cryptoCount(); f.lose("result");
  const first = f.start("s".repeat(8192)); assert.equal(f.start(), first); assert.equal(await first, undefined);
  assert.equal(f.counts.result, 1); await f.start(); assert.equal(f.counts.result, 2);
  assert.equal(f.cryptoCount() - before, 1); assert.equal(f.counts.input, 1); assert.equal(f.counts.admit, 1);
  assert.ok(f.f.bodyReads().includes(277 + 579 + 8192 + 16), "dedicated op17 reader consumes maximum Seal Input");
  assert.throws(() => f.start("different"), /already consumed/);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.counts.returned, 1); assert.equal(f.f.rowBudget().reserved, 0);
});

for (const sample of [{ name: "maximum", length: 8192, bad: false }, { name: "tag-only", length: 0, bad: false },
  { name: "AEAD failure", length: 0, bad: true }]) test(`HIGH ${sample.name} Open uses the original retained Pair RX and exact Result retry`, async t => {
  const f = await framedSdpOffer(t, false, sample.length, sample.bad);
  assert.throws(() => f.start("answer before Applied"), /first SDP Offer role/);
  const before = f.f.sdpSdkCount(); f.lose("result"); await f.start();
  assert.equal(f.counts.result, 1); await f.start(); assert.equal(f.counts.result, 2);
  assert.equal(f.f.sdpSdkCount() - before, 1); assert.equal(f.counts.input, 1); assert.equal(f.counts.admit, 1);
  assert.ok(f.f.operations().includes(sample.bad ? 21 : 20));
  if (sample.length === 8192) assert.ok(f.f.bodyReads().includes(277 + 579 + 8208 + 16));
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.counts.returned, 1); assert.equal(f.f.broker.counts().bytes, 0);
});

test("End during retained HIGH RX SDK is retirement rather than a forged AEAD failure", async t => {
  const f = await framedSdpOffer(t, false), held = f.f.holdWhen((name, args) => name === "decrypt" &&
    (args[0] as AesGcmParams)?.additionalData?.byteLength === 579);
  const pending = f.start(); await until(held.entered); const payment = f.f.broker.counts().bytes;
  let closed = false; f.f.owner.retire(); const closing = f.child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(closed, false); assert.equal(f.counts.settle, 0);
  assert.equal(f.f.broker.counts().bytes, payment); held.release(); await pending; await closing;
  assert.equal(f.counts.result, 0); assert.equal(f.counts.settle, 1); assert.equal(f.counts.returned, 1);
});

test("lost browser SDP Begin recovers its exact row before any SDK work", async t => {
  const f = await framedSdpOffer(t); f.lose("begin"); await f.start("s".repeat(8192));
  assert.equal(f.counts.begin, 1); assert.equal(f.counts.input, 0);
  await f.start(); assert.equal(f.counts.resolve, 1); assert.equal(f.counts.begin, 1); assert.equal(f.counts.result, 1);
  f.f.owner.retire(); await f.child.channel.close();
});

test("End with a held browser SDP Result keeps funding and joins native cleanup outside the lane", async t => {
  const f = await framedSdpOffer(t), held = f.hold("result"), pending = f.start("s".repeat(8192));
  await until(() => f.counts.result === 1); const payment = f.f.broker.counts().bytes;
  let closed = false; f.f.owner.retire(); const closing = f.child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(closed, false); assert.equal(f.counts.settle, 0);
  assert.equal(f.f.broker.counts().bytes, payment); assert.equal(f.custody.effectCounts().sdkReleases, 0);
  held.release(); await pending; await closing; assert.equal(f.counts.settle, 1); assert.equal(f.counts.returned, 1);
  assert.equal(f.f.broker.counts().bytes, 0);
});

test("End in Input timer finally refuses the loan before engine admission", async t => {
  const f = await framedSdpOffer(t);
  f.onInput(() => f.f.onCancel(() => f.f.owner.retire()));
  await f.start("s".repeat(8192)); await f.child.channel.close();
  assert.equal(f.counts.input, 1); assert.equal(f.counts.admit, 0); assert.equal(f.counts.result, 0);
  assert.equal(f.counts.settle, 1); assert.equal(f.counts.returned, 1); assert.equal(f.f.broker.counts().bytes, 0);
});

test("End joins actual held SDP WebCrypto before releasing its original input/payment", async t => {
  const f = await framedSdpOffer(t);
  const held = f.f.holdWhen((name, args) => name === "encrypt" &&
    (args[0] as AesGcmParams)?.additionalData?.byteLength === 579);
  const pending = f.start("s".repeat(8192)); await until(held.entered);
  let closed = false; const payment = f.f.broker.counts().bytes;
  f.f.owner.retire(); const closing = f.child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(closed, false); assert.equal(f.counts.settle, 0);
  assert.equal(f.f.broker.counts().bytes, payment); assert.equal(f.custody.effectCounts().sdkReleases, 0);
  held.release(); await pending; await closing; assert.equal(f.counts.result, 0); assert.equal(f.counts.settle, 1);
  assert.equal(f.f.broker.counts().bytes, 0);
});

test("SDP reserves both rows before unknown admission and refuses a retained broker25", async t => {
  const { f, child, prepare } = await establishedSdpFixture(t), bytes = f.broker.counts().bytes;
  const state = f.broker as unknown as { reservedRows: number };
  const reserved = state.reservedRows; state.reservedRows = 29;
  try { assert.throws(prepare, /history capacity/); assert.equal(f.broker.counts().bytes, bytes); }
  finally { state.reservedRows = reserved; }
  await prepare(); assert.equal(f.rowBudget().reserved, 2);
  const previous = Object.create(f.broker); Object.defineProperty(previous, "version", { value: 25 });
  const key = Symbol.for("mewtual.voice.crypto-broker.v2"), host = { [key]: previous };
  assert.throws(() => windowVoiceCryptoBroker(f.admission, host, real), /reload required/); assert.equal(host[key], previous);
  f.owner.retire(); await child.channel.close(); assert.equal(f.rowBudget().reserved, 0);
});

test("Pending native SDP retries the same Begin bytes until Action attachment", async t => {
  const f = await framedSdpOffer(t); f.pendingBegin(true); await f.start("s".repeat(8192));
  assert.equal(f.counts.begin, 2); assert.equal(f.counts.input, 0);
  f.pendingBegin(false); await f.start(); assert.equal(f.counts.begin, 3); assert.equal(f.counts.input, 1); assert.equal(f.counts.result, 1);
  f.f.owner.retire(); await f.child.channel.close();
});

test("reentrant SDP creation clock cannot install a second owner", async t => {
  const f = await framedSdpOffer(t);
  f.f.onNow(() => assert.throws(() => f.start("s".repeat(8192)), /creation is already in progress/));
  await f.start("s".repeat(8192)); assert.equal(f.counts.begin, 1); assert.equal(f.counts.result, 1);
  f.f.owner.retire(); await f.child.channel.close();
});

test("an engine permit held beyond the original PC deadline cannot start SDP WebCrypto", async t => {
  const f = await framedSdpOffer(t), before = f.cryptoCount(), held = f.hold("admit");
  const pending = f.start("s".repeat(8192)); await until(() => f.counts.admit === 1);
  f.f.advance(10_000); held.release(); await pending;
  assert.equal(f.cryptoCount() - before, 0); assert.equal(f.counts.result, 0);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.counts.settle, 1); assert.equal(f.counts.returned, 1);
});

test("PC admission retains the original paid subtree and Close precedes SDK/storage/key release", async t => {
  const { f, child, custody, admitPc } = await establishedPcFixture(t);
  const funding = f.broker.counts().bytes;
  const first = admitPc(); assert.equal(admitPc(), first); await first; await admitPc();
  assert.equal(f.broker.counts().bytes, funding, "fixed PC graph uses the existing64KiB subtree");
  assert.deepEqual(custody.pcCounts(), { pcAdmits: 2, pcCloses: 0, pcPrepared: true, pcClosed: false });
  f.owner.retire(); await child.channel.close();
  assert.deepEqual(custody.effectOrder(), ["storage-prepare", "sdk-admit", "pc-admit", "pc-admit", "pc-close", "sdk-release", "storage-release", "key-release"]);
  assert.equal(f.broker.counts().bytes, 0);
});

for (const outcome of ["lost", "refused", "preparing"] as const) test(`PC ${outcome} admission requires exact Close before SDK release`, async t => {
  const { f, child, custody, admitPc } = await establishedPcFixture(t);
  if (outcome === "lost") custody.losePc();
  else if (outcome === "refused") custody.refusePc();
  else custody.preparingPc();
  await admitPc(); assert.equal(custody.pcCounts().pcAdmits, 1);
  f.owner.retire(); await child.channel.close();
  assert.deepEqual(custody.pcCounts(), { pcAdmits: 1, pcCloses: 1, pcPrepared: true, pcClosed: true });
  assert.equal(f.broker.counts().bytes, 0);
});

test("End during real PC admission IO waits for finally before exact Close and refunds", async t => {
  const { f, child, custody, admitPc } = await establishedPcFixture(t);
  const held = custody.holdPc(), pending = admitPc();
  await until(() => custody.pcCounts().pcAdmits === 1);
  const payment = f.broker.counts().bytes; let closed = false;
  f.owner.retire(); const closing = child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.equal(custody.pcCounts().pcCloses, 0);
  assert.equal(custody.effectCounts().sdkReleases, 0); assert.equal(f.broker.counts().bytes, payment);
  held.release(); await pending; await closing;
  assert.equal(custody.pcCounts().pcCloses, 1); assert.equal(f.broker.counts().bytes, 0);
});

test("End inside PC request encryption closes unknown intent without submitting fresh admission", async t => {
  const { f, child, custody, admitPc } = await establishedPcFixture(t);
  const held = f.holdWhen((name, args) => name === "encrypt" && new Uint8Array((args[0] as AesGcmParams).additionalData as ArrayBuffer)[6] === 14);
  const pending = admitPc(); await until(held.entered);
  const payment = f.broker.counts().bytes; f.owner.retire(); const closing = child.channel.close();
  assert.equal(custody.pcCounts().pcAdmits, 0); assert.equal(f.broker.counts().bytes, payment);
  held.release(); await pending; await closing;
  assert.deepEqual(custody.pcCounts(), { pcAdmits: 0, pcCloses: 1, pcPrepared: false, pcClosed: true });
  assert.equal(f.broker.counts().bytes, 0);
});

test("lost PC Close preserves its original payment and prevents later releases until retry", async t => {
  const { f, child, custody, admitPc } = await establishedPcFixture(t);
  await admitPc(); custody.losePcClose();
  f.owner.retire(); const closing = child.channel.close();
  await until(() => custody.pcCounts().pcCloses === 1 && f.timers().delays.includes(1000));
  assert.equal(custody.effectCounts().sdkReleases, 0); assert.equal(custody.effectCounts().storageReleases, 0);
  assert.ok(f.broker.counts().bytes >= 64 * 1024);
  f.advance(1000); await closing;
  assert.equal(custody.pcCounts().pcCloses, 2); assert.equal(f.broker.counts().bytes, 0);
});

test("PC custody never starts on copied handles, foreign chains or a closed original channel", async t => {
  const { f, child, custody, admitPc } = await establishedPcFixture(t);
  assert.throws(() => f.broker.prepareNativePcIntent(f.owner, { ...child.channel }, child.ready.chain), /original native PC custody/);
  assert.throws(() => f.broker.prepareNativePcIntent(f.owner, child.channel, { ...child.ready.chain }), /original native PC custody/);
  f.owner.retire(); await admitPc(); await child.channel.close();
  assert.equal(custody.pcCounts().pcAdmits, 0); assert.equal(custody.pcCounts().pcCloses, 0);
  assert.equal(f.broker.counts().bytes, 0);
});


/** Concrete retained-channel caller with a controlled RTC provider. Native MAC/AEAD and
 * original broker SDK are real; Root authority and WebRTC implementation are still mocked.
 * This does not claim WebView/native interoperability, connected media or physical audio. */
function sdpRtcProvider(t: TestContext, localIsLow: boolean, onLocal: () => void = () => {}, onClose: () => void = () => {}, onDescription: () => void = () => {}) {
  const old = globalThis.RTCPeerConnection;
  const calls = { created: 0, local: 0, remote: 0, closed: 0 };
  let held: ReturnType<typeof gate> | null = null, localHeld: ReturnType<typeof gate> | null = null;
  let descriptor: RTCSessionDescriptionInit | null = null;
  class Peer {
    constructor() { calls.created++; }
    iceGatheringState = "complete";
    get localDescription() { onDescription(); return { type: localIsLow ? "offer" : "answer", sdp: "s".repeat(8192) }; }
    addTransceiver() {}
    async setLocalDescription() { calls.local++; onLocal(); if (localHeld) await localHeld.promise; }
    async setRemoteDescription(value: RTCSessionDescriptionInit) {
      calls.remote++; descriptor = value; assert.equal(value.type, localIsLow ? "answer" : "offer"); assert.equal(value.sdp, "s".repeat(8192));
      if (held) await held.promise;
    }
    addEventListener() {} removeEventListener() {}
    close() { calls.closed++; onClose(); }
  }
  globalThis.RTCPeerConnection = Peer as unknown as typeof RTCPeerConnection;
  t.after(() => { globalThis.RTCPeerConnection = old; held?.release(); localHeld?.release(); });
  return { calls, descriptor: () => descriptor, hold() { held = gate(); return held; }, holdLocal() { localHeld = gate(); return localHeld; } };
}

test("LOW concrete RTC local preparation gets a fresh14 and retires getter text before maximum crypto", async t => {
  const f = await framedSdpOffer(t); const before = f.custody.effectOrder().filter(step => step === "pc-admit").length;
  const sdk = sdpRtcProvider(t, true, () => assert.equal(f.custody.effectOrder().filter(step => step === "pc-admit").length, before + 1));
  const pending = f.rtc(); assert.equal(f.rtc(), pending); assert.equal(await pending, undefined);
  assert.equal(sdk.calls.local, 1); assert.equal(f.counts.begin, 1); assert.equal(f.counts.result, 1);
  assert.equal(f.counts.deliveryAdmit, 0); assert.equal(f.counts.deliveryApplied, 0);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(sdk.calls.closed, 1); assert.equal(f.f.broker.counts().bytes, 0);
});

for (const lost of ["deliveryAdmit", "deliveryApplied"] as const) test(`HIGH original RTC applies once across lost ${lost} reply`, async t => {
  const f = await framedSdpOffer(t, false); const sdk = sdpRtcProvider(t, false);
  f.lose(lost); await f.rtc();
  assert.equal(sdk.calls.remote, lost === "deliveryAdmit" ? 0 : 1);
  await f.rtc(); assert.equal(sdk.calls.remote, 1); assert.equal(f.counts.result, 1);
  assert.equal(f.counts.input, 1); assert.equal(f.counts.admit, 1); assert.equal(f.counts.settle, 1);
  assert.equal(f.counts.deliveryAdmit, lost === "deliveryAdmit" ? 2 : 1);
  assert.equal(f.counts.deliveryApplied, lost === "deliveryApplied" ? 2 : 1);
  assert.equal(sdk.descriptor()?.sdp, "", "application description frame clears before Applied RPC");
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(sdk.calls.closed, 1); assert.equal(f.f.broker.counts().bytes, 0);
});

test("End during actual remote RTC work joins it before Action Return and native SDK/storage release", async t => {
  const f = await framedSdpOffer(t, false); const sdk = sdpRtcProvider(t, false), held = sdk.hold();
  const work = f.rtc(); await until(() => sdk.calls.remote === 1);
  const paid = f.f.broker.counts().bytes; f.f.owner.retire(); let closed = false;
  const closing = f.child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(closed, false); assert.equal(sdk.calls.closed, 1); assert.equal(f.counts.returned, 0);
  assert.equal(f.custody.effectCounts().sdkReleases, 0); assert.equal(f.f.broker.counts().bytes, paid);
  held.release(); await work; await closing;
  assert.equal(f.counts.returned, 1); assert.equal(f.custody.effectCounts().sdkReleases, 1); assert.equal(f.f.broker.counts().bytes, 0);
});

test("HMR refuses funded broker26 before waking its old RTC-less shape", async t => {
  const f = await framedSdpOffer(t); const previous = Object.create(f.f.broker);
  Object.defineProperty(previous, "version", { value: 26 }); let wakes = 0;
  previous.retrySelectedJoinKeys = () => { wakes++; };
  const key = Symbol.for("mewtual.voice.crypto-broker.v2"), host = { [key]: previous };
  assert.throws(() => windowVoiceCryptoBroker({ async admit() { return ""; }, async settle() {}, async commitInbound() {} }, host, real), /reload required/);
  assert.equal(wakes, 0); assert.equal(host[key], previous); f.f.owner.retire(); await f.child.channel.close();
});


test("HIGH delivery capacity keeps its same inbox and retries without repeating crypto or RTC", async t => {
  const f = await framedSdpOffer(t, false); const sdk = sdpRtcProvider(t, false);
  f.busyDelivery(true); await f.rtc(); assert.equal(f.counts.result, 1); assert.equal(sdk.calls.remote, 0);
  f.busyDelivery(false); await f.rtc(); assert.equal(sdk.calls.remote, 1); assert.equal(f.counts.result, 1);
  assert.equal(f.counts.input, 1); assert.equal(f.counts.admit, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

test("LOW delayed fresh14 after End never starts an RTC constructor or local SDK", async t => {
  const f = await framedSdpOffer(t); const sdk = sdpRtcProvider(t, true), held = f.custody.holdPc();
  const work = f.rtc(); await until(() => f.custody.pcCounts().pcAdmits === 2);
  const paid = f.f.broker.counts().bytes; f.f.owner.retire(); let closed = false;
  const closing = f.child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(closed, false); assert.equal(f.f.broker.counts().bytes, paid);
  held.release(); await work; await closing;
  assert.equal(sdk.calls.created, 0); assert.equal(sdk.calls.local, 0); assert.equal(f.counts.begin, 0);
  assert.equal(f.f.broker.counts().bytes, 0);
});

test("HIGH delayed fresh26 after End never reaches RTC even when native recorded admission", async t => {
  const f = await framedSdpOffer(t, false); const sdk = sdpRtcProvider(t, false), held = f.hold("deliveryAdmit");
  const work = f.rtc(); await until(() => f.counts.deliveryAdmit === 1);
  f.f.owner.retire(); const closing = f.child.channel.close(); held.release(); await work; await closing;
  assert.equal(sdk.calls.remote, 0); assert.equal(sdk.calls.created, 0); assert.equal(f.counts.deliveryApplied, 0);
  assert.equal(f.f.broker.counts().bytes, 0);
});


test("LOW RTC transfer excludes reentrant status until its original UTF8 slot is adopted", async t => {
  const f = await framedSdpOffer(t);
  const statuses: Promise<unknown>[] = []; let observations = 0;
  const probe = () => {
    if (f.counts.begin !== 0) return;
    observations++;
    statuses.push(f.child.channel.status());
    f.f.onOwnerCheck(probe);
  };
  const sdk = sdpRtcProvider(t, true, () => f.f.onOwnerCheck(probe));
  await f.rtc();
  assert.ok(observations > 3, "exercise callbacks during preparation and crypto adoption");
  assert.equal(f.counts.begin, 1, "status cannot strand the sole SDK description before Begin");
  assert.equal(f.counts.result, 1);
  assert.equal(sdk.calls.local, 1);
  assert.ok((await Promise.all(statuses)).every(value => (value as { kind: string }).kind === "uncertain"));
  await f.rtc(); assert.equal(sdk.calls.local, 1); assert.equal(f.counts.begin, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});


async function framedSdpRound(t: TestContext, low: boolean, badOffer = false,
    prepared: Awaited<ReturnType<typeof establishedPcFixture>> | null = null) {
  if (prepared) await prepared.admitPc();
  const offer = await framedSdpOffer(t, low, 8192, badOffer, prepared);
  const answer = await framedSdpOffer(t, low, 8192, false, offer, true);
  let onAnswer: (() => void) | null = null;
  offer.custody.sdpReplies(async (header, opened) => {
    const seal = header.operation === 15 ? true : header.operation === 16 ? false :
      (header.operation === 27 ? opened[225] : opened[17]) === 0;
    const next = seal !== low;
    if (next) {
      assert.equal(offer.counts.returned > 0, true, "first action compacts before Answer admission");
      onAnswer?.();
    }
    return (next ? answer : offer).respond(header, opened);
  });
  return { ...offer, offer, answer,
    round: () => offer.f.broker.completeNativeSdpRtcRound(offer.f.owner, offer.child.channel, offer.child.ready.chain),
    onAnswer(work: () => void) { onAnswer = work; },
  };
}

for (const low of [true, false]) test(`complete original RTC round compacts Offer before Answer; LOW=${low}`, async t => {
  const f = await framedSdpRound(t, low), sdk = sdpRtcProvider(t, low);
  const paid = f.f.broker.counts().bytes;
  await f.round();
  assert.equal(f.offer.counts.result, 1); assert.equal(f.answer.counts.result, 1);
  assert.equal(f.offer.counts.returned, 1); assert.equal(f.answer.counts.returned, 1);
  assert.equal(sdk.calls.created, 1); assert.equal(sdk.calls.local, 1); assert.equal(sdk.calls.remote, 1);
  assert.equal(sdk.calls.closed, 0, "live action-local compaction keeps the same PC alive");
  assert.equal(f.f.broker.counts().bytes, paid); assert.equal(f.f.rowBudget().reserved, 2);
  await f.round(); assert.equal(sdk.calls.local, 1); assert.equal(sdk.calls.remote, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(sdk.calls.closed, 1); assert.equal(f.f.broker.counts().bytes, 0);
});


for (const low of [true, false]) test(`lost Offer Return retains same owner before Answer; LOW=${low}`, async t => {
  const f = await framedSdpRound(t, low), sdk = sdpRtcProvider(t, low);
  f.offer.lose("returned"); await f.round();
  assert.equal(f.answer.counts.begin, 0); assert.equal(f.offer.counts.returned, 1);
  assert.equal(sdk.calls.local, Number(low)); assert.equal(sdk.calls.remote, Number(!low));
  await f.round(); assert.equal(f.offer.counts.returned, 2); assert.equal(f.answer.counts.result, 1);
  assert.equal(sdk.calls.local, 1); assert.equal(sdk.calls.remote, 1); assert.equal(sdk.calls.created, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

for (const [low, lost] of [[true, "result"], [false, "result"], [true, "deliveryApplied"],
  [true, "returned"], [false, "returned"]] as const) test(`lost Answer ${lost} retries original work; LOW=${low}`, async t => {
  const f = await framedSdpRound(t, low), sdk = sdpRtcProvider(t, low); f.answer.lose(lost);
  await f.round(); await f.round();
  assert.equal(f.offer.counts.result, 1); assert.equal(f.answer.counts.result, lost === "result" ? 2 : 1);
  assert.equal(f.answer.counts.input, 1); assert.equal(f.answer.counts.admit, 1);
  assert.equal(sdk.calls.created, 1); assert.equal(sdk.calls.local, 1); assert.equal(sdk.calls.remote, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

for (const low of [true, false]) test(`End during Offer compaction never admits Answer; LOW=${low}`, async t => {
  const f = await framedSdpRound(t, low), sdk = sdpRtcProvider(t, low), held = f.offer.hold("returned");
  const pending = f.round(); await until(() => f.offer.counts.returned === 1);
  const paid = f.f.broker.counts().bytes; f.f.owner.retire(); let closed = false;
  const closing = f.child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(closed, false);
  assert.equal(f.f.broker.counts().bytes, paid); assert.equal(f.answer.counts.begin, 0);
  held.release(); await pending; await closing; assert.equal(f.answer.counts.begin, 0);
  assert.equal(sdk.calls.local, Number(low)); assert.equal(sdk.calls.remote, Number(!low));
  assert.equal(sdk.calls.closed, 1); assert.equal(f.f.broker.counts().bytes, 0);
});

for (const low of [true, false]) test(`End joins held Answer SDK before Return/release; LOW=${low}`, async t => {
  const f = await framedSdpRound(t, low), sdk = sdpRtcProvider(t, low);
  const held = low ? sdk.hold() : sdk.holdLocal();
  const pending = f.round(); await until(() => sdk.calls.local === 1 && sdk.calls.remote === 1);
  const paid = f.f.broker.counts().bytes; f.f.owner.retire(); let closed = false;
  const closing = f.child.channel.close().then(() => { closed = true; });
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(closed, false);
  assert.equal(f.f.broker.counts().bytes, paid); assert.equal(f.answer.counts.returned, 0);
  assert.equal(f.custody.effectCounts().sdkReleases, 0); held.release(); await pending; await closing;
  assert.equal(sdk.calls.closed, 1); assert.equal(f.f.broker.counts().bytes, 0);
});

for (const low of [true, false]) test(`historical-only Offer cannot authorize Answer; LOW=${low}`, async t => {
  const f = await framedSdpRound(t, low), sdk = sdpRtcProvider(t, low); f.offer.historical();
  await f.round(); assert.equal(f.offer.counts.input, 0); assert.equal(f.answer.counts.begin, 0);
  assert.equal(sdk.calls.remote, 0); f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

test("rejected incoming Offer cannot authorize an Answer SDK", async t => {
  const f = await framedSdpRound(t, false, true), sdk = sdpRtcProvider(t, false);
  await f.round(); assert.equal(f.offer.counts.result, 1); assert.equal(f.answer.counts.begin, 0);
  assert.equal(sdk.calls.local, 0); assert.equal(sdk.calls.remote, 0);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

for (const low of [true, false]) test(`malformed Answer Input retries same native Action before crypto; LOW=${low}`, async t => {
  const f = await framedSdpRound(t, low), sdk = sdpRtcProvider(t, low); f.answer.malformedInput(true);
  await f.round(); assert.equal(f.answer.counts.begin, 1); assert.equal(f.answer.counts.result, 0);
  assert.equal(f.answer.counts.admit, 0); f.answer.malformedInput(false); await f.round();
  assert.equal(f.answer.counts.begin, 1); assert.equal(f.answer.counts.input, 2); assert.equal(f.answer.counts.result, 1);
  assert.equal(sdk.calls.local, 1); assert.equal(sdk.calls.remote, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

for (const low of [true, false]) test(`unavailable Answer Begin recovers original purpose without new SDK; LOW=${low}`, async t => {
  const f = await framedSdpRound(t, low), sdk = sdpRtcProvider(t, low); f.answer.busyBegin(true);
  await f.round(); assert.equal(f.answer.counts.result, 0); f.answer.busyBegin(false); await f.round();
  assert.equal(f.answer.counts.resolve, 1); assert.equal(f.answer.counts.begin, 2); assert.equal(f.answer.counts.result, 1);
  assert.equal(sdk.calls.local, 1); assert.equal(sdk.calls.remote, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

test("HIGH delayed Answer14 cannot renew the original deadline or start local SDK", async t => {
  const f = await framedSdpRound(t, false), sdk = sdpRtcProvider(t, false), held = f.custody.holdPc();
  const pending = f.round(); await until(() => f.custody.pcCounts().pcAdmits === 2);
  f.f.advance(10_000); held.release(); await pending;
  assert.equal(sdk.calls.remote, 1); assert.equal(sdk.calls.local, 0); assert.equal(f.answer.counts.begin, 0);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});


test("reentrant status during Offer Return and Answer adoption stays outside the composite flight", async t => {
  const f = await framedSdpRound(t, false), sdk = sdpRtcProvider(t, false);
  const statuses: Promise<unknown>[] = []; let probes = 0;
  const probe = () => {
    if (f.answer.counts.begin > 0) return;
    probes++; statuses.push(f.child.channel.status()); f.f.onOwnerCheck(probe);
  };
  f.offer.onReturn(() => f.f.onOwnerCheck(probe));
  const pending = f.round(); assert.equal(f.round(), pending); await pending;
  assert.ok(probes > 3); assert.ok((await Promise.all(statuses)).every(value => (value as { kind: string }).kind === "uncertain"));
  assert.equal(f.answer.counts.result, 1); assert.equal(sdk.calls.created, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

test("round composer rejects copied channel/chain and broker27 without waking old custody", async t => {
  const f = await framedSdpRound(t, true), sdk = sdpRtcProvider(t, true);
  assert.throws(() => f.f.broker.completeNativeSdpRtcRound(f.f.owner, { ...f.child.channel }, f.child.ready.chain), /original native RTC/);
  assert.throws(() => f.f.broker.completeNativeSdpRtcRound(f.f.owner, f.child.channel, { ...f.child.ready.chain }), /original native RTC/);
  const previous = Object.create(f.f.broker); Object.defineProperty(previous, "version", { value: 27 });
  const key = Symbol.for("mewtual.voice.crypto-broker.v2"), host = { [key]: previous };
  let wakes = 0; previous.retrySelectedJoinKeys = () => { wakes++; };
  assert.throws(() => windowVoiceCryptoBroker(f.f.admission, host, real), /reload required/);
  assert.equal(wakes, 0); assert.equal(host[key], previous); assert.equal(sdk.calls.created, 0);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});

test("failed original RTC close retains broker payment and withholds native SDK release until retry", async t => {
  const f = await framedSdpRound(t, true); let fails = true;
  const sdk = sdpRtcProvider(t, true, () => {}, () => { if (fails) throw new Error("SDK close failed"); });
  await f.round(); assert.equal(sdk.calls.created, 1);
  f.f.owner.retire(); let done = false;
  const closing = f.child.channel.close().then(() => { done = true; });
  try {
    await until(() => f.f.timers().delays.includes(1000));
    assert.equal(done, false); assert.ok(f.f.broker.counts().bytes >= 64 * 1024);
    assert.equal(f.custody.effectCounts().sdkReleases, 0);
    assert.equal(f.custody.effectCounts().storageReleases, 0);
  } finally { fails = false; f.f.advance(1000); }
  await closing; assert.equal(done, true); assert.equal(f.f.broker.counts().bytes, 0);
  assert.equal(f.custody.effectCounts().sdkReleases, 1);
});


/** Real framing/AEAD with independent native stand-in; Rust TCP tests separately
 * establish actual native admission. Assert exact original bindings and release order. */
async function framedMedia(t: TestContext, low = true,
    prepared: Awaited<ReturnType<typeof establishedPcFixture>> | null = null) {
  const f = await framedSdpRound(t, low, false, prepared), sdk = sdpRtcProvider(t, low); await f.round();
  const original = { parentRow: f.child.parent.row, identity: f.child.parent.identity,
    handshake: f.child.parent.binding.handshake, key: hash(99), sequence: "1",
    storage: hash(100), sdkGeneration: "1", sdkProof: hash(101), pcProof: hash(102) };
  const proof = hash(121), modes: number[] = [];
  let admitted = false, closed = false, released = false, indexed=false, remaining=1000, busy=-1, lost = -1, hold = -1, held = gate();
  f.f.cleanup(() => held.release());
  f.custody.mediaReplies(async (header, opened) => {
    const mode = opened[1]; modes.push(mode); assert.equal(header.logical, original.parentRow);
    const expected = mode <= 3 ? encodeVoiceMediaOriginal(original, (["admit", "resolve", "close", "release"] as const)[mode])
      : encodeVoiceMediaExisting(original.parentRow, original.identity, proof, (["current", "close", "release", "detach", "retirehistory"] as const)[mode - 4]);
    assert.deepEqual(opened, expected.body);
    if (mode === busy) return {kind:0,body:Uint8Array.of(9,2,2,7)};
    if (mode === 0) admitted = true;
    if (mode === 7) { assert.equal(remaining,0); indexed=true; }
    if (mode === 2 || mode === 5) closed = true;
    if (mode === 3 || mode === 6) { assert.ok(closed); assert.equal(sdk.calls.closed, 1); released = true; }
    if (mode === hold) await held.promise;
    if (mode === lost) { lost = -1; throw new Error("exact native media reply lost"); }
    if (!admitted) return { kind: 0, body: new Uint8Array([9, 1]) };
    const current = (mode === 4 || mode === 7) && !closed && f.f.owner.isCurrent();
    const bytes = Buffer.alloc(39); bytes.set([9, 0, 1 | (closed ? 2 : 0) | (released ? 4 : 0) | (current ? 8 : 0)]);
    Buffer.from(proof, "hex").copy(bytes, 3); bytes.writeUInt32BE(current ? remaining : 0xffffffff, 35);
    return { kind: 0, body: bytes };
  }, () => !admitted || released || indexed);
  return { ...f, sdk, modes, media: () => f.f.broker.completeNativeMediaHandoff(f.f.owner, f.child.channel, f.child.ready.chain),
    remaining(value: number) { remaining=value; }, busy(mode: number) { busy=mode; },
    collectHistory: () => f.f.broker.collectNativeNegotiationHistory(f.f.owner,f.child.channel,f.child.ready.chain),
    retireNegotiation: () => f.f.broker.retireNativeNegotiation(f.f.owner,f.child.channel,f.child.ready.chain),
    lose(mode: number) { lost = mode; }, hold(mode: number) { hold = mode; held = gate(); return held; } };
}
for (const low of [true, false]) test(`framed media adopts original peer once and releases before old SDK/Final; LOW=${low}`, async t => {
  const f = await framedMedia(t, low), before = f.f.broker.counts().bytes;
  const first = f.media(); assert.equal(f.media(), first); await first;
  assert.equal(f.f.broker.counts().bytes, before + 8192); assert.deepEqual(f.modes, [0, 4]);
  assert.equal(f.sdk.calls.created, 1); assert.equal(f.sdk.calls.closed, 0);
  await f.media(); assert.deepEqual(f.modes, [0, 4, 4]);
  f.f.owner.retire(); assert.equal(f.sdk.calls.closed, 1);
  await f.child.channel.close(); assert.deepEqual(f.modes.slice(-2), [2, 3]);
  assert.equal(f.f.broker.counts().bytes, 0); assert.equal(f.child.parent.finalCounts().finalAcknowledged, true);
});
for (const mode of [0, 4]) test(`framed End closes actual peer before held media mode${mode} returns`, async t => {
  const f = await framedMedia(t), held = f.hold(mode), pending = f.media();
  await until(() => f.modes.includes(mode)); const paid = f.f.broker.counts().bytes; let finished = false;
  f.f.owner.retire(); assert.equal(f.sdk.calls.closed, 1);
  const closing = f.child.channel.close().then(() => { finished = true; });
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(finished, false); assert.equal(f.f.broker.counts().bytes, paid); assert.equal(f.custody.effectCounts().sdkReleases, 0);
  held.release(); await pending; await closing;
  assert.equal(f.f.broker.counts().bytes, 0); assert.deepEqual(f.modes.slice(-2), [2, 3]);
});
test("framed lost media admission recovers original claim without a second peer or payment", async t => {
  const f = await framedMedia(t); f.lose(0); const paid = f.f.broker.counts().bytes;
  await f.media(); assert.deepEqual(f.modes, [0]);
  await f.media(); assert.deepEqual(f.modes, [0, 1, 4]);
  assert.equal(f.sdk.calls.created, 1); assert.equal(f.sdk.calls.closed, 0); assert.equal(f.f.broker.counts().bytes, paid + 8192);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});
for (const version of [28,29,30,31,32,33,34,35]) test(`HMR refuses funded broker${version} without waking incompatible cleanup`, async t => {
  const f = await framedMedia(t), previous = Object.create(f.f.broker);
  Object.defineProperty(previous, "version", { value: version }); let wakes = 0; previous.retrySelectedJoinKeys = () => { wakes++; };
  const key = Symbol.for("mewtual.voice.crypto-broker.v2"), host = { [key]: previous };
  assert.throws(() => windowVoiceCryptoBroker({ async admit() { return ""; }, async settle() {}, async commitInbound() {} }, host, real), /reload required/);
  assert.equal(wakes, 0); assert.equal(host[key], previous); f.f.owner.retire(); await f.child.channel.close();
});


test("framed expired media closes actual peer while Current IO stays entered", async t => {
  const f = await framedMedia(t); await f.media(); const held = f.hold(4);
  f.f.advance(1000); await until(() => f.modes.filter(mode => mode === 4).length === 2);
  f.f.advance(4000); assert.equal(f.sdk.calls.closed, 1);
  f.f.owner.retire(); // Release the separate selected-join key too.
  const paid = f.f.broker.counts().bytes; let finished = false;
  const closing = f.child.channel.close().then(() => { finished = true; });
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(finished, false); assert.equal(f.f.broker.counts().bytes, paid);
  held.release(); await closing; assert.equal(f.sdk.calls.closed, 1); assert.equal(f.f.broker.counts().bytes, 0);
});

test("framed lost media Release keeps old SDK/storage until exact released recovery", async t => {
  const f = await framedMedia(t); await f.media(); f.lose(3); f.f.owner.retire(); const closing = f.child.channel.close();
  await until(() => f.modes.includes(3) && f.f.timers().delays.includes(1000));
  assert.equal(f.custody.effectCounts().sdkReleases, 0); assert.equal(f.custody.effectCounts().storageReleases, 0);
  assert.ok(f.f.broker.counts().bytes >= 64 * 1024 + 8192);
  f.f.advance(1000); await closing; assert.deepEqual(f.modes.slice(-3), [2, 3, 2]); assert.equal(f.f.broker.counts().bytes, 0);
});

test("media prepayment refuses before native or SDK callbacks under full group pressure", async t => {
  const f = await framedMedia(t), before = f.f.broker.counts().bytes;
  // Model unrelated prepaid work by using the actual broker charge/refund seam. No
  // capacity constants or admission rules are changed; the fee is retained until released.
  const pool = f.f.broker as unknown as { charge(owner: typeof f.f.owner, bytes: number): void; refund(owner: typeof f.f.owner, bytes: number): void };
  let paid = 0;
  try {
    for (;;) { try { pool.charge(f.f.owner, 8192); paid += 8192; } catch { break; } }
    assert.ok(paid > 0); assert.throws(f.media, /capacity/);
    assert.deepEqual(f.modes, []); assert.equal(f.sdk.calls.closed, 0); assert.equal(f.sdk.calls.created, 1);
  } finally { pool.refund(f.f.owner, paid); }
  assert.equal(f.f.broker.counts().bytes, before); await f.media(); assert.deepEqual(f.modes, [0, 4]);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes, 0);
});


test("indexed media keeps the same peer and Session while original negotiation retires", async t => {
  const f=await framedMedia(t); await f.media(); f.remaining(0);
  const before=f.f.broker.counts().bytes, settlements=f.f.counts().settlements;
  const pending=f.retireNegotiation(); assert.equal(f.retireNegotiation(),pending); await pending;
  assert.ok(f.modes.includes(7)); assert.equal(f.sdk.calls.closed,0); assert.equal(f.sdk.calls.created,1);
  assert.equal(f.custody.effectCounts().sdkReleases,1); assert.equal(f.child.parent.finalCounts().finalAcknowledged,true);
  assert.equal(f.f.broker.counts().bytes,before-64*1024);
  assert.equal(f.f.counts().settlements,settlements,"shared Session is not settled/closed");
  assert.equal(f.child.ready.chain.isCurrent(),false); assert.equal(f.child.ready.derivation.pair(),null);
  await f.retireNegotiation(); assert.equal(f.custody.effectCounts().sdkReleases,1);
  f.f.owner.retire(); assert.equal(f.sdk.calls.closed,1); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes,0);
});

test("End during original detach closes peer before joining index and negotiation IO", async t => {
  const f=await framedMedia(t); await f.media(); f.remaining(0); const held=f.hold(7), work=f.retireNegotiation();
  await until(() => f.modes.includes(7)); const paid=f.f.broker.counts().bytes;
  f.f.owner.retire(); assert.equal(f.sdk.calls.closed,1); let finished=false;
  const end=f.child.channel.close().then(() => {finished=true;}); await new Promise<void>(yes => setImmediate(yes));
  assert.equal(finished,false); assert.equal(f.f.broker.counts().bytes,paid);
  held.release(); await work; await end; assert.equal(f.f.broker.counts().bytes,0); assert.equal(f.sdk.calls.closed,1);
});


test("negotiation retirement coalesces owner-predicate reentry before a second flight is published", async t => {
  const f=await framedMedia(t); await f.media(); f.remaining(0); let nested:Promise<void>|null=null;
  f.f.onOwnerCheck(() => { nested=f.retireNegotiation(); });
  const work=f.retireNegotiation(); assert.equal(work,nested); await work;
  assert.equal(f.modes.filter(mode=>mode===7).length,1); assert.equal(f.custody.effectCounts().sdkReleases,1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes,0);
});

test("End reentry before retirement publication cannot submit an index request", async t => {
  const f=await framedMedia(t); await f.media(); f.remaining(0);
  f.f.onOwnerCheck(() => f.f.owner.retire()); assert.throws(f.retireNegotiation,/ended/);
  assert.equal(f.modes.includes(7),false); assert.equal(f.sdk.calls.closed,1);
  await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes,0);
});


test("nonzero native Source remaining cannot submit detach or refund negotiation", async t => {
  const f=await framedMedia(t); await f.media(); const paid=f.f.broker.counts().bytes;
  const work=f.retireNegotiation(); await until(() => f.modes.filter(mode=>mode===4).length>=2 && f.f.timers().delays.includes(1000));
  assert.equal(f.modes.includes(7),false); assert.equal(f.custody.effectCounts().sdkReleases,0);
  assert.equal(f.f.broker.counts().bytes,paid); assert.equal(f.sdk.calls.closed,0);
  f.remaining(0); f.f.advance(1000); await work;
  assert.equal(f.custody.effectCounts().sdkReleases,1); assert.equal(f.f.broker.counts().bytes,paid-64*1024);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes,0);
});

test("a lost original detach reply retains negotiation until the same indexed claim is recovered", async t => {
  const f=await framedMedia(t); await f.media(); f.remaining(0); f.lose(7);
  const paid=f.f.broker.counts().bytes, work=f.retireNegotiation();
  await until(() => f.modes.includes(7) && f.f.timers().delays.includes(1000));
  assert.equal(f.custody.effectCounts().sdkReleases,0); assert.equal(f.f.broker.counts().bytes,paid); assert.equal(f.sdk.calls.closed,0);
  f.f.advance(1000); await work; assert.equal(f.modes.filter(mode=>mode===7).length,2);
  assert.equal(f.f.broker.counts().bytes,paid-64*1024); assert.equal(f.sdk.calls.created,1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes,0);
});


test("refused detach keeps the original negotiation paid until actual index admission", async t => {
  const f=await framedMedia(t); await f.media(); f.remaining(0); f.busy(7);
  const paid=f.f.broker.counts().bytes, work=f.retireNegotiation();
  await until(() => f.modes.includes(7) && f.f.timers().delays.includes(1000));
  assert.equal(f.custody.effectCounts().sdkReleases,0); assert.equal(f.f.broker.counts().bytes,paid);
  assert.equal(f.child.parent.finalCounts().finalAcknowledged,false); assert.equal(f.sdk.calls.closed,0);
  f.busy(-1); f.f.advance(1000); await work;
  assert.equal(f.custody.effectCounts().sdkReleases,1); assert.equal(f.f.broker.counts().bytes,paid-64*1024);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes,0);
});

test("framed exact native history collection retains fixed browser graph and original peer", async t=>{
  const f=await framedMedia(t); await f.media(); assert.throws(()=>f.collectHistory(),/retired original/);
  f.remaining(0); await f.retireNegotiation(); const paid=f.f.broker.counts().bytes;
  assert.equal(await f.collectHistory(),true); assert.equal(f.modes.filter(mode=>mode===8).length,1);
  assert.equal(f.f.broker.counts().bytes,paid); assert.equal(f.sdk.calls.closed,0);
  assert.throws(()=>f.f.broker.collectNativeNegotiationHistory(f.f.owner,{...f.child.channel},f.child.ready.chain),/original native history/);
  assert.throws(()=>f.f.broker.collectNativeNegotiationHistory(f.f.owner,f.child.channel,{...f.child.ready.chain}),/original native history/);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes,0);
});
test("framed lost history reply recovers original tombstone and keeps fixed fees", async t=>{
  const f=await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation(); f.lose(8);
  const paid=f.f.broker.counts().bytes; assert.equal(await f.collectHistory(),false);
  assert.equal(await f.collectHistory(),true); assert.equal(f.modes.filter(mode=>mode===8).length,2);
  assert.equal(f.f.broker.counts().bytes,paid); f.f.owner.retire(); await f.child.channel.close();
});
test("End closes actual peer before joining retained history IO", async t=>{
  const f=await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const held=f.hold(8), work=f.collectHistory(); await until(()=>f.modes.includes(8));
  const paid=f.f.broker.counts().bytes; f.f.owner.retire(); assert.equal(f.sdk.calls.closed,1);
  let done=false; const cleanup=f.child.channel.close().then(()=>{done=true;}); await new Promise<void>(yes=>setImmediate(yes));
  assert.equal(done,false); assert.equal(f.f.broker.counts().bytes,paid);
  held.release(); await work; await cleanup; assert.equal(f.f.broker.counts().bytes,0);
});

test("public native history observers coalesce before and during actual held framing", async t=>{
  const f=await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const held=f.hold(8), first=f.collectHistory();
  for(let n=0;n<1000;n++) assert.equal(f.collectHistory(),first);
  await until(()=>f.modes.includes(8));
  for(let n=0;n<1000;n++) assert.equal(f.collectHistory(),first);
  const paid=f.f.broker.counts().bytes; f.f.owner.retire(); assert.equal(f.sdk.calls.closed,1);
  let done=false; const cleanup=f.child.channel.close().then(()=>{done=true;});
  await new Promise<void>(yes=>setImmediate(yes)); assert.equal(done,false); assert.equal(f.f.broker.counts().bytes,paid);
  held.release(); await first; await cleanup; assert.equal(f.f.broker.counts().bytes,0);
});

test("returning retirement RPCs yield the physical lane to original media Current", async t=>{
  const f=await framedMedia(t); await f.media(); f.remaining(0);
  const before=f.f.broker.counts().bytes, trace:string[]=[]; let elapsed=0, active=true;
  f.f.onResponse(async(header,mode)=>{
    if(!active || !f.modes.includes(7)) return;
    if(header.operation===28){trace.push("media"+mode);return;}
    trace.push("cleanup"+header.operation); elapsed+=1200; f.f.advance(1200);
    await new Promise<void>(yes=>setImmediate(yes));
  });
  try {
    await f.retireNegotiation();
    assert.ok(elapsed>5000,"the batch must span more than one original freshness window");
    assert.equal(f.sdk.calls.closed,0,"returning individual RPCs must allow timely Current");
    assert.equal(f.child.parent.finalCounts().finalAcknowledged,true);
    assert.equal(f.f.broker.counts().bytes,before-65536);
    const first=trace.findIndex(value=>value.startsWith("cleanup"));
    const refresh=trace.findIndex((value,index)=>index>first && value==="media4");
    assert.ok(refresh>first && trace.slice(refresh+1).some(value=>value.startsWith("cleanup")),
      "authenticated Current must physically run between old cleanup requests");
  } finally {active=false;f.f.owner.retire();await f.child.channel.close();}
});
test("one actually held retirement RPC still expires media and retains payment until return", async t=>{
  const f=await framedMedia(t); await f.media(); f.remaining(0);
  const held=gate(); f.f.cleanup(held.release); let entered=false, done=false;
  f.f.onResponse(async(header)=>{if(header.operation===14 && f.modes.includes(7)){entered=true;await held.promise;}});
  const paid=f.f.broker.counts().bytes, retirement=f.retireNegotiation().then(()=>{done=true;});
  await until(()=>entered); f.f.advance(5000);
  assert.equal(f.sdk.calls.closed,1); assert.equal(done,false);
  assert.equal(f.f.broker.counts().bytes,paid);
  held.release();await retirement;f.f.owner.retire();await f.child.channel.close();
  assert.equal(f.f.broker.counts().bytes,0);
});

test("authenticated detachment fences old negotiation before held work-pump close", async t=>{
  const f=await framedMedia(t); await f.media();
  const held=gate(); f.f.cleanup(held.release); let entered=false;
  f.f.native.readWork=async(owner,executor)=>{entered=true;await held.promise;
    return {kind:"snapshot",owner,executor,nextSlot:32,work:null};};
  const observer=f.f.broker.observeNativeWork(f.f.owner,f.child.channel,f.f.key);
  await until(()=>entered); f.remaining(0); const paid=f.f.broker.counts().bytes;
  let done=false; const retirement=f.retireNegotiation().then(()=>{done=true;});
  try {
    await until(()=>f.modes.includes(7)); await f.media();
    assert.equal(f.child.ready.chain.isCurrent(),false,
      "physically joined native detachment must fence old browser negotiation before metadata joins");
    for(let n=0;n<35;n++){
      f.f.advance(1000); await f.media(); f.f.broker.collectChains();
      assert.equal(f.sdk.calls.closed,0,"indexed peer must remain independently current during old metadata cleanup");
    }
    assert.equal(done,false,"actual metadata owner must still keep retirement pending");
    assert.equal(f.f.broker.counts().bytes,paid,"old graph remains paid through held pump return");
  } finally {
    held.release();await retirement;await observer.close();f.f.owner.retire();await f.child.channel.close();
  }
});


test("live history compaction keeps the same peer and Session with only paid inert history", async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const broker = f.f.broker, plan = f.child.ready.chain;
  type Chain = { fixedBytes: number; records: Map<unknown, {native: unknown; chain: unknown}>; channel: unknown; origin: unknown };
  type Session = { keys: unknown; bootstrap: unknown; tx: bigint; negotiation: { linked: unknown; terminalAction: unknown; finalProof: unknown } };
  const state = broker as unknown as { chains: WeakMap<object, Chain>; ipcChannels: Map<string, Session> };
  const old = state.chains.get(plan)!, rows = [...old.records.values()], oldFixed = old.fixedBytes;
  const session = state.ipcChannels.get(f.f.owner.token)!, keys = session.keys, bootstrap = session.bootstrap, tx = session.tx;
  const paid = broker.counts().bytes;
  const compact = () => broker.compactNativeNegotiation(f.f.owner, f.child.channel, plan);
  const first = compact(); for (let n = 0; n < 2000; n++) assert.equal(compact(), first);
  assert.equal(await first, true);
  assert.equal(broker.counts().bytes, paid - oldFixed + 2048);
  assert.equal(state.chains.has(plan), false); assert.equal(old.records.size, 0);
  assert.equal(old.channel, null); assert.equal(old.origin, null);
  for (const row of rows) { assert.equal(row.native, null); assert.equal(row.chain, null); }
  assert.equal(session.negotiation.linked, null); assert.equal(session.negotiation.terminalAction, null); assert.equal(session.negotiation.finalProof, null);
  assert.equal(session.keys, keys); assert.equal(session.bootstrap, bootstrap); assert.ok(session.tx > tx);
  assert.equal(f.sdk.calls.created, 1); assert.equal(f.sdk.calls.closed, 0);
  assert.equal(plan.isCurrent(), false); assert.equal(f.child.ready.derivation.pair(), null);
  assert.equal((await f.child.ready.derivation.attempt()).kind, "retired");
  const after = broker.counts().bytes; assert.equal(await compact(), true); assert.equal(broker.counts().bytes, after);
  assert.throws(() => broker.compactNativeNegotiation(f.f.owner, f.child.channel, {...plan}), /original browser history/);
  assert.throws(() => broker.completeNativeMediaHandoff(f.f.owner, f.child.channel, {...plan}), /original native media/);
  await f.media(); assert.equal(f.sdk.calls.closed, 0); assert.equal(session.keys, keys);
  assert.throws(() => broker.nativeDerivationFromChannel(f.f.owner, f.child.channel, f.f.key, historyPurpose, f.child.parent.row, f.child.parent.identity), /ready voice channel|capacity/);
  f.f.owner.retire(); assert.equal(f.sdk.calls.closed, 1); await f.child.channel.close();
  assert.equal(broker.counts().bytes, 0);
});


for (const fault of ["capacity", "lost"] as const) test(`live compaction retains the same graph/payment after ${fault} native history`, async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const broker = f.f.broker, plan = f.child.ready.chain;
  const state = broker as unknown as {chains: WeakMap<object, {fixedBytes: number; cell: {detached: boolean}}>};
  const original = state.chains.get(plan)!, paid = broker.counts().bytes, fixed = original.fixedBytes;
  if (fault === "capacity") f.busy(8); else f.lose(8);
  assert.equal(await broker.compactNativeNegotiation(f.f.owner, f.child.channel, plan), false);
  assert.equal(broker.counts().bytes, paid); assert.equal(state.chains.get(plan), original);
  assert.equal(original.cell.detached, false); assert.equal(f.sdk.calls.closed, 0);
  f.busy(-1);
  assert.equal(await broker.compactNativeNegotiation(f.f.owner, f.child.channel, plan), true);
  assert.equal(broker.counts().bytes, paid - fixed + 2048);
  assert.equal(f.modes.filter(mode => mode === 8).length, 2, "the same native claim recovers after refusal/loss");
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(broker.counts().bytes, 0);
});

test("End closes the actual peer before a live compaction history request returns", async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const broker = f.f.broker, held = f.hold(8);
  const pending = broker.compactNativeNegotiation(f.f.owner, f.child.channel, f.child.ready.chain);
  await until(() => f.modes.includes(8)); const paid = broker.counts().bytes;
  f.f.owner.retire(); assert.equal(f.sdk.calls.closed, 1);
  let finished = false; const closing = f.child.channel.close().then(() => { finished = true; });
  await flush(); assert.equal(finished, false); assert.equal(broker.counts().bytes, paid);
  held.release(); assert.equal(await pending, false); await closing; await flush();
  assert.equal(broker.counts().bytes, 0);
});

test("End at the final live compaction owner check returns ownership to ordinary cleanup", async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const broker = f.f.broker, plan = f.child.ready.chain;
  const state = broker as unknown as {chains: WeakMap<object, {historyDisposing: boolean; cell: {detached: boolean}}>};
  const original = state.chains.get(plan)!; let ended = false;
  const atPrepared = () => {
    if (!ended && original.historyDisposing && original.cell.detached) { ended = true; f.f.owner.retire(); }
    else if (!ended) f.f.onOwnerCheck(atPrepared);
  };
  f.f.onOwnerCheck(atPrepared);
  assert.equal(await broker.compactNativeNegotiation(f.f.owner, f.child.channel, plan), false);
  assert.equal(ended, true); assert.equal(f.sdk.calls.closed, 1);
  await f.child.channel.close(); await flush(); assert.equal(broker.counts().bytes, 0);
});


test("unset runtime plans never match an empty compacted-history slot", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const calls = f.operations();
  for (const value of [null, undefined]) {
    const plan = value as unknown as Parameters<typeof f.broker.compactNativeNegotiation>[2];
    assert.throws(() => f.broker.compactNativeNegotiation(f.owner, channel, plan), /original browser history/);
    assert.throws(() => f.broker.collectNativeNegotiationHistory(f.owner, channel, plan), /original native history/);
    assert.throws(() => f.broker.retireNativeNegotiation(f.owner, channel, plan), /original native negotiation/);
    assert.throws(() => f.broker.completeNativeMediaHandoff(f.owner, channel, plan), /original native media/);
  }
  assert.deepEqual(f.operations(), calls, "unset plans neither collect history nor submit native work");
});


test("live compaction retains the full fixed fee until its heavy commit frame returns", async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const broker = f.f.broker, plan = f.child.ready.chain, paid = broker.counts().bytes;
  type Prepared = { commit(): number; abort(): void };
  type Custody = { prepareCompaction(chain: unknown): Promise<Prepared>; compactionReturned(bytes: number): void };
  const state = broker as unknown as { ipcChannels: Map<string, {custody: Custody}> };
  const custody = state.ipcChannels.get(f.f.owner.token)!.custody;
  const prepare = custody.prepareCompaction, returned = custody.compactionReturned;
  let commitReturned = false, refunds = 0;
  custody.prepareCompaction = async chain => {
    const prepared = await prepare(chain);
    return { abort: () => prepared.abort(), commit: () => {
      const refund = prepared.commit();
      assert.equal(broker.counts().bytes, paid, "actual entered commit/worker metadata still owns the original fixed fee");
      commitReturned = true; return refund;
    } };
  };
  custody.compactionReturned = bytes => {
    assert.equal(commitReturned, true, "small finalizer runs after heavy commit returned");
    refunds++; returned(bytes);
  };
  assert.equal(await broker.compactNativeNegotiation(f.f.owner, f.child.channel, plan), true);
  assert.equal(refunds, 1); assert.ok(broker.counts().bytes < paid);
  assert.equal(await broker.compactNativeNegotiation(f.f.owner, f.child.channel, plan), true); assert.equal(refunds, 1);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(broker.counts().bytes, 0);
});


test("a delayed old RTC callback cannot read SDK text or clear a replacement negotiation", async t => {
  const f = await framedSdpOffer(t); let reads = 0;
  const sdk = sdpRtcProvider(t,true,()=>{},()=>{},()=>{reads++;}), held = sdk.holdLocal();
  const pending = f.rtc(); await until(()=>sdk.calls.local===1);
  type Pair = {constructor: new () => Pair; rtcFlight: Promise<void>|null; sdpFlight: Promise<void>|null; actionBusy:boolean};
  const state = f.f.broker as unknown as {ipcChannels:Map<string,{negotiation:Pair}>};
  const channel = state.ipcChannels.get(f.f.owner.token)!, original = channel.negotiation;
  // Deliberately force a replacement earlier than production may publish it. This
  // negative regression pins callback ownership independently of the admission barrier.
  const replacement = new original.constructor(), sentinel = gate();
  replacement.sdpFlight = sentinel.promise; replacement.actionBusy = true;
  channel.negotiation = replacement;
  try {
    held.release(); await pending;
    assert.equal(reads,0,"an old SDK completion must recheck the actual negotiation before reading text");
    assert.equal(f.counts.begin,0); assert.equal(original.rtcFlight,null);
    assert.equal(replacement.sdpFlight,sentinel.promise); assert.equal(replacement.actionBusy,true);
  } finally { channel.negotiation = original; sentinel.release(); }
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(f.f.broker.counts().bytes,0);
});


for (const seam of ["owner", "clock"] as const) test(`fresh RTC rechecks negotiation identity after the ${seam} callback`, async t => {
  const f = await framedSdpOffer(t), sdk = sdpRtcProvider(t,true);
  type Pair = {constructor:new()=>Pair; actionBusy:boolean; rtcFlight:Promise<void>|null};
  const state = f.f.broker as unknown as {ipcChannels:Map<string,{negotiation:Pair}>};
  const channel=state.ipcChannels.get(f.f.owner.token)!, original=channel.negotiation, replacement=new original.constructor();
  replacement.actionBusy=true; let changed=false;
  const replace=()=>{changed=true;channel.negotiation=replacement;};
  if(seam==="owner")f.f.onOwnerCheck(replace);else f.f.onNow(replace);
  const pcAdmits=f.custody.pcCounts().pcAdmits;
  try {
    await f.rtc();assert.equal(changed,true);assert.equal(sdk.calls.created,0);
    assert.equal(f.custody.pcCounts().pcAdmits,pcAdmits);assert.equal(f.counts.begin,0);
    assert.equal(original.rtcFlight,null);assert.equal(replacement.actionBusy,true);
  }finally{channel.negotiation=original;}
  f.f.owner.retire();await f.child.channel.close();assert.equal(f.f.broker.counts().bytes,0);
});


test("PC Admit rechecks its captured negotiation after its own request clock", async t => {
  const {f,child,custody,admitPc}=await establishedPcFixture(t);
  type Pair={constructor:new()=>Pair;actionBusy:boolean;sdpFlight:Promise<void>|null};
  const state=f.broker as unknown as {ipcChannels:Map<string,{negotiation:Pair}>};
  const channel=state.ipcChannels.get(f.owner.token)!, original=channel.negotiation, replacement=new original.constructor();
  replacement.actionBusy=true;const baseline=custody.pcCounts().pcAdmits;
  f.onNow(()=>{channel.negotiation=replacement;});
  try {await admitPc();assert.equal(custody.pcCounts().pcAdmits,baseline);assert.equal(replacement.actionBusy,true);assert.equal(original.sdpFlight,null);}
  finally {channel.negotiation=original;}
  f.owner.retire();await child.channel.close();assert.equal(f.broker.counts().bytes,0);
});

test("a channel-only close inside its current owner predicate cannot report fresh negotiation permission", async t => {
  const f=await fixture(t), handle=f.channel();await handle.status();
  const state=f.broker as unknown as {ipcChannels:Map<string,{negotiation:object;negotiationCurrent(owner:object):boolean}>};
  const channel=state.ipcChannels.get(f.owner.token)!;let closing:Promise<void>|null=null;
  f.onOwnerCheck(()=>{closing=handle.close();});
  assert.equal(channel.negotiationCurrent(channel.negotiation),false);
  assert.equal(f.owner.isCurrent(),true,"closing this channel did not retire the external owner");
  await closing;f.owner.retire();await flush();assert.equal(f.broker.counts().bytes,0);
});

test("an old pump cannot reserve an intake when the broker owner predicate replaces its negotiation", async t => {
  const f=await fixture(t), handle=f.channel();await handle.status();const parent=await framedDerivation(f);
  const observer=f.broker.observeNativeWork(f.owner,handle,f.key);
  type Pair={constructor:new()=>Pair;workPump:{actions:{derive(work:unknown):unknown}}};
  const state=f.broker as unknown as {ipcChannels:Map<string,{negotiation:Pair}>};
  const channel=state.ipcChannels.get(f.owner.token)!, original=channel.negotiation, replacement=new original.constructor();
  const before=f.broker.counts(), rows=f.rowBudget();let checks=0;
  const replaceInBroker=()=>{if(++checks===2)channel.negotiation=replacement;else f.onOwnerCheck(replaceInBroker);};
  f.onOwnerCheck(replaceInBroker);
  try {
    assert.throws(()=>original.workPump.actions.derive({kind:"attached",row:parent.row,identity:parent.identity,
      purpose:{kind:"derive",...historyPurpose}}),/old voice intake generation/);
    assert.equal(checks,2);assert.deepEqual(f.broker.counts(),before);assert.deepEqual(f.rowBudget(),rows);
  }finally{channel.negotiation=original;}
  await observer.close();
  // Complete the same real fixture parent normally so the rejection oracle cannot hide
  // unfinished native work or skip the actual post-End zero-payment gate.
  const ready=await f.broker.nativeDerivationFromChannel(f.owner,handle,f.key,historyPurpose,parent.row,parent.identity).attempt();
  assert.equal(ready.kind,"ready");if(ready.kind!=="ready")throw new Error("original input required");
  assert.equal((await ready.derivation.attempt()).kind,"accepted");
  f.owner.retire();await handle.close();await flush();assert.equal(f.broker.counts().bytes,0);
});


test("indexed media keeps refreshing while another Session's actual bootstrap provider is held", async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const first = f.f, second = await fixture(t, first.broker, 2), channel = second.channel();
  const retained = first.broker as unknown as { ipcChannels: Map<string, { keys: unknown; bootstrap: unknown; tx: bigint }> };
  const original = retained.ipcChannels.get(first.owner.token)!, keys = original.keys, bootstrap = original.bootstrap;
  const before = original.tx, held = first.holdWhen(name => name === "deriveBits");
  const setup = channel.status(); await until(held.entered);
  for (let n = 0; n < 7; n++) {
    first.advance(1000); await f.media();
    assert.equal(f.sdk.calls.closed, 0, "hung bootstrap cannot monopolize original media Current");
  }
  assert.equal(original.keys, keys); assert.equal(original.bootstrap, bootstrap); assert.ok(original.tx > before);
  assert.equal(first.providers().peak, 2, "actual held SDK and serialized codec overlap within two providers");
  let ordinary = 0;
  const waiting = first.broker.run(second.owner, "generate", 128, async () => { ordinary++; });
  assert.equal(ordinary, 0, "two reserved channels plus SDK never admit a third provider");
  first.owner.retire(); assert.equal(f.sdk.calls.closed, 1); await f.child.channel.close();
  await flush(); assert.equal(ordinary, 0, "remaining channel plus granted SDK still consume both positions");
  held.release(); assert.equal((await setup).kind, "ready"); await waiting; assert.equal(ordinary, 1);
  assert.equal((await channel.status()).kind, "ready", "shared codec survives original channel collection");
  second.owner.retire(); await channel.close(); assert.equal(first.broker.counts().bytes, 0);
});

test("indexed media survives more than one freshness window during another actual protected derivation", async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const first = f.f, second = await fixture(t, first.broker, 2), channel = second.channel(); await channel.status();
  const linked = await linkedDerivation(second, channel);
  const held = first.holdWhen(name => name === "deriveBits"), action = linked.register(), attempt = action.attempt();
  await until(held.entered); assert.equal(linked.counts().admits, 1);
  for (let n = 0; n < 7; n++) {
    first.advance(1000); await f.media(); assert.equal(f.sdk.calls.closed, 0);
    assert.equal(linked.counts().commits, 0, "only actual held SDK return permits result publication");
  }
  assert.equal(first.providers().peak, 2);
  held.release(); assert.equal((await attempt).kind, "accepted");
  assert.deepEqual(linked.counts(), { admits: 1, commits: 1, settlements: 1 });
  first.owner.retire(); await f.child.channel.close(); second.owner.retire(); await channel.close();
  assert.equal(first.broker.counts().bytes, 0);
});


test("ordinary admission reserves its position before a reentrant owner predicate requests SDK", async t => {
  const f = await fixture(t), channel = f.channel(); await channel.status();
  const state = f.broker as unknown as { ipcChannels: Map<string, { reserveEngine(): Promise<(() => void) | null> }>; executionPositions(): number };
  const session = state.ipcChannels.get(f.owner.token)!, firstHeld = gate(), secondHeld = gate();
  let firstEntered = false, secondEntered = false, sdkGranted = false, used = 0;
  let sdk: Promise<(() => void) | null> | null = null;
  f.cleanup(() => { firstHeld.release(); secondHeld.release(); void sdk?.then(release => release?.()); });
  const first = f.broker.run(f.owner, "generate", 128, async () => { firstEntered = true; await firstHeld.promise; });
  await until(() => firstEntered);
  const second = f.broker.run(f.owner, "generate", 128, async () => {
    secondEntered = true;
    used = state.executionPositions();
    await secondHeld.promise;
  });
  const settle = f.admission.settle;
  f.admission.settle = async () => {
    f.admission.settle = settle;
    f.onOwnerCheck(() => { sdk = session.reserveEngine().then(release => { sdkGranted = release !== null; return release; }); });
  };
  firstHeld.release(); await first; await until(() => secondEntered);
  assert.ok(used <= 2, "owner callback cannot oversubscribe the spare position");
  assert.ok(sdk); assert.equal(sdkGranted, false, "queued SDK cannot preempt an already-reserved ordinary position");
  secondHeld.release(); await second; const release = await sdk; assert.ok(release); release();
  f.owner.retire(); await channel.close(); assert.equal(f.broker.counts().bytes, 0);
});


test("End cancels another channel's unstarted SDK borrow while the first actual provider stays held", async t => {
  const first = await fixture(t), a = first.channel(); await a.status();
  const second = await fixture(t, first.broker, 2), b = second.channel(); await b.status();
  const linked = await linkedDerivation(first, a), other = await linkedDerivation(second, b);
  const held = first.holdWhen(name => name === "deriveBits"), action = linked.register().attempt();
  await until(held.entered);
  const queued = other.register().attempt();
  const state = first.broker as unknown as { sdkPositions: { waiting: unknown[] }; executionPositions(): number };
  await until(() => state.sdkPositions.waiting.length === 1);
  assert.equal(other.counts().admits, 0, "queued borrower has no native engine admission");
  second.owner.retire(); assert.equal((await queued).kind, "retired"); await b.close();
  assert.equal(state.sdkPositions.waiting.length, 0);
  assert.equal(first.providers().active, 1, "other channel's End cannot cancel this actual SDK");
  assert.equal(state.executionPositions(), 2, "one channel plus its granted SDK still consume both positions");
  assert.equal(linked.counts().commits, 0); assert.equal(other.counts().admits, 0);
  let ordinary = false;
  const pending = first.broker.run(first.owner, "generate", 128, async () => { ordinary = true; });
  await flush(); assert.equal(ordinary, false);
  held.release(); assert.equal((await action).kind, "accepted"); await pending;
  assert.equal(first.providers().peak, 1, "unstarted queued borrower never invokes a second provider"); first.owner.retire(); await a.close();
  assert.equal(first.broker.counts().bytes, 0);
});


test("bootstrap transient provider and position metadata fit their existing prepayments", () => {
  // Parsed Session strings/keys belong to the independently retained8KiB Control payment.
  // Setup owns bounded IPC UTF16, parsed backing, provider transient inputs and continuation
  // wrappers. Its queued position uses the otherwise-idle original16KiB metadata workspace.
  const nativeReplyText = 2 * 2048, transcriptBacking = 1024, nativePoint = 65;
  const hashAndSecrets = 32 + 2 * 32, providerParameters = 256;
  const keyWrappers = 8 * 128, setupFrames = 2 * 256;
  const sdkLoan = 128 + 3 * 128 + 2 * 128;
  assert.ok(nativeReplyText + transcriptBacking + nativePoint + hashAndSecrets + providerParameters + keyWrappers + setupFrames <= 8192);
  const codecCoordinator = 256 + 2 * 128, sdkCoordinator = 256 + 2 * 128;
  assert.ok(sdkLoan + codecCoordinator + sdkCoordinator <= 16 * 1024);
});


test("a queued SDK grant cannot publish a loan into a replacement negotiation", async t => {
  const f = await fixture(t), handle = f.channel(); await handle.status();
  type Pair = { constructor: new () => Pair; sdkPosition: unknown };
  const state = f.broker as unknown as { ipcChannels: Map<string, { negotiation: Pair; reserveEngine(): Promise<(() => void) | null> }> };
  const channel = state.ipcChannels.get(f.owner.token)!, original = channel.negotiation;
  const held = gate(); f.cleanup(held.release);
  let entered = false;
  const ordinary = f.broker.run(f.owner, "generate", 128, async () => { entered = true; await held.promise; });
  await until(() => entered);
  const pending = channel.reserveEngine();
  const replacement = new original.constructor(); channel.negotiation = replacement;
  let release: (() => void) | null = null;
  try {
    held.release(); await ordinary; release = await pending;
    assert.equal(release, null, "old owner's granted position must return before publication");
    assert.equal(original.sdkPosition, null); assert.equal(replacement.sdkPosition, null);
  } finally { release?.(); channel.negotiation = original; }
  f.owner.retire(); await handle.close(); assert.equal(f.broker.counts().bytes, 0);
});


test("two sequential framed RTC peers share one Session and close independently after compaction", async t => {
  const originalRtc = globalThis.RTCPeerConnection;
  const first = await framedMedia(t), f = first.f;
  // Restore the real initial constructor after both providers' individual fixture hooks.
  t.after(() => { globalThis.RTCPeerConnection = originalRtc; });
  await first.media(); first.remaining(0); await first.retireNegotiation();
  const firstPlan = first.child.ready.chain;
  const state = f.broker as unknown as { ipcChannels: Map<string, { keys: unknown; bootstrap: unknown; tx: bigint; negotiation: object }> };
  const session = state.ipcChannels.get(f.owner.token)!, keys = session.keys, bootstrap = session.bootstrap, old = session.negotiation;
  assert.equal(f.broker.advanceNativeNegotiation(f.owner, first.child.channel, firstPlan), false);
  assert.equal(await f.broker.compactNativeNegotiation(f.owner, first.child.channel, firstPlan), true);
  const firstResponder = f.actionHandler()!;
  assert.equal(f.broker.advanceNativeNegotiation(f.owner, first.child.channel, firstPlan), true);
  assert.notEqual(session.negotiation, old); const newOwner = session.negotiation;
  assert.equal(f.broker.advanceNativeNegotiation(f.owner, first.child.channel, firstPlan), true);
  assert.equal(session.negotiation, newOwner, "old repeated marker cannot advance again");
  const tx = session.tx;
  const prepared = await establishedPcFixture(t, true, f, 2);
  const second = await framedMedia(t, true, prepared), secondResponder = f.actionHandler()!;
  f.actions((header, body) => header.logical === first.child.parent.row ? firstResponder(header, body) : secondResponder(header, body));
  await second.media(); second.remaining(0); await second.retireNegotiation();
  assert.equal(await f.broker.compactNativeNegotiation(f.owner, second.child.channel, second.child.ready.chain), true);
  assert.equal(session.keys, keys); assert.equal(session.bootstrap, bootstrap); assert.ok(session.tx > tx);
  assert.notEqual(first.child.parent.row, second.child.parent.row, "full serial differs when native slot is reused");
  assert.equal(first.sdk.calls.created, 1); assert.equal(second.sdk.calls.created, 1);
  for (let n = 0; n < 7; n++) { f.advance(1000); await first.media(); await second.media(); }
  assert.equal(first.sdk.calls.closed, 0); assert.equal(second.sdk.calls.closed, 0);
  first.busy(4);
  for (let n = 0; n < 5; n++) { f.advance(1000); await second.media(); await flush(); }
  assert.equal(first.sdk.calls.closed, 1, "expired first peer closes synchronously");
  assert.equal(second.sdk.calls.closed, 0, "first peer failure cannot close another indexed peer");
  assert.equal(f.owner.isCurrent(), true); await second.media();
  f.owner.retire(); assert.equal(second.sdk.calls.closed, 1); await first.child.channel.close(); assert.equal(f.broker.counts().bytes, 0);
});


test("next negotiation waits for actual compaction accounting return and rejects copied markers", async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const broker = f.f.broker, plan = f.child.ready.chain;
  type Channel = { custody: { compactionReturned(bytes: number): void }; negotiation: object };
  const state = broker as unknown as { ipcChannels: Map<string, Channel> };
  const channel = state.ipcChannels.get(f.f.owner.token)!, old = channel.negotiation;
  const returned = channel.custody.compactionReturned; let callbacks = 0;
  channel.custody.compactionReturned = bytes => {
    callbacks++; assert.equal(broker.advanceNativeNegotiation(f.f.owner, f.child.channel, plan), false,
      "accounting reentry is earlier than original compaction/finally return");
    assert.equal(channel.negotiation, old); returned(bytes);
  };
  assert.equal(await broker.compactNativeNegotiation(f.f.owner, f.child.channel, plan), true);
  assert.equal(callbacks, 1);
  assert.equal(broker.advanceNativeNegotiation(f.f.owner, f.child.channel, { ...plan }), false);
  assert.equal(channel.negotiation, old);
  assert.equal(broker.advanceNativeNegotiation(f.f.owner, f.child.channel, plan), true);
  assert.notEqual(channel.negotiation, old);
  f.f.owner.retire(); await f.child.channel.close(); assert.equal(broker.counts().bytes, 0);
});

test("End in the final next-owner predicate closes the peer and cannot publish another negotiation", async t => {
  const f = await framedMedia(t); await f.media(); f.remaining(0); await f.retireNegotiation();
  const broker = f.f.broker, plan = f.child.ready.chain;
  await broker.compactNativeNegotiation(f.f.owner, f.child.channel, plan);
  const state = broker as unknown as { ipcChannels: Map<string, { negotiation: object }> };
  const channel = state.ipcChannels.get(f.f.owner.token)!, old = channel.negotiation;
  f.f.onOwnerCheck(() => f.f.owner.retire());
  assert.equal(broker.advanceNativeNegotiation(f.f.owner, f.child.channel, plan), false);
  assert.equal(channel.negotiation, old); assert.equal(f.sdk.calls.closed, 1);
  await f.child.channel.close(); assert.equal(broker.counts().bytes, 0);
});

test("compacted peer record and cleanup wrappers fit the transferred2KiB marker", () => {
  const cell = 128, closures = 256, completedDispatch = 128, compactionReturn = 128;
  const record = 256, cleanup = 128, generation = 64, feeReturn = 256, mapEntry = 64, reserve = 640;
  assert.equal(cell + closures + completedDispatch + compactionReturn + record + cleanup + generation + feeReturn + mapEntry + reserve, 2048);
});


test("cleanup waiters share one timer owner and cannot strand each other on a joining scheduler fault", async t => {
  const f = await fixture(t), handle = f.channel(); await handle.status();
  type Signal = { resolve(): void };
  type Channel = { retry: Signal | null; wakeToken(): object; waitCleanupWake(token: object): Promise<void> };
  const state = f.broker as unknown as { ipcChannels: Map<string, Channel> }, channel = state.ipcChannels.get(f.owner.token)!;
  const token = channel.wakeToken(), a = channel.waitCleanupWake(token); await flush();
  const original = channel.retry!; let joiningSchedule = false;
  f.cleanup(() => { f.onSchedule(() => {}); original.resolve(); });
  f.onSchedule(() => { joiningSchedule = true; throw new Error("joining timer refused"); });
  const b = channel.waitCleanupWake(token); void b.catch(() => {}); await flush();
  assert.equal(joiningSchedule, false, "joining another cleanup must not become a second timer owner");
  assert.equal(a, b); f.onSchedule(() => {}); f.advance(1000); await a; await b;
  f.owner.retire(); await handle.close(); assert.equal(f.broker.counts().bytes, 0);
});
