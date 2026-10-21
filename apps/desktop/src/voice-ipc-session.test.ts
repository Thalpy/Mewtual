import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { webcrypto } from "node:crypto";
import {
  VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker,
  type VoiceIpcNative, type VoiceSelectedKeyNative,
} from "./voice-crypto.ts";
import {
  decodeVoiceIpcAdmission, encodeVoiceIpcAdmission,
  voiceIpcNonce, voiceIpcResponseAad, type VoiceIpcHeader,
} from "./voice-ipc-frame.ts";

const real = webcrypto.subtle as unknown as SubtleCrypto;
const utf8 = new TextEncoder();
const hash = (n: number) => n.toString(16).padStart(64, "0");
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const deadline = performance.now() + 15_000;
  while (!check()) { if (performance.now() >= deadline) throw new Error("condition did not settle"); await new Promise<void>(yes => setImmediate(yes)); }
}
const integer = (n: bigint, size: number) => Buffer.from(n.toString(16).padStart(size * 2, "0"), "hex");

/** Independent actual-WebCrypto native endpoint fixture. It consumes only the published
 * selected point, never this broker's private key or its Session internals/counters. */
async function fixture(t: TestContext) {
  let ordinary = 0, setups = 0, settlements = 0, loseSetup = false, loseSettle = false;
  let setupClosed = false, published: Uint8Array | null = null, executor = "";
  let tamperSetup = false, badBody = false, failEncrypt = false;
  let forceSequence: bigint | null = null, nativeSequence = 0n;
  const requestSequences: bigint[] = [], engineSteps: string[] = [];
  const bodyReads: number[] = [];
  let holdAt = -1, held: ReturnType<typeof gate> | null = null, engineEnabled = false;
  let stalled = false, cancelEntered = false, cancelGate: ReturnType<typeof gate> | null = null;
  let observedStep = -1;
  let now = 0, timerSerial = 0, hostileHeaders = false, hangFetch = false, fetchAborted = false, fetchEntered = false;
  const timers = new Map<number, { at: number; run(): void }>();
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
  const owner = createVoiceCryptoOwner("native-owner", hash(1), () => true);
  const admission = { async admit() { ordinary++; return "ordinary"; }, async settle() {}, async commitInbound() {} };
  const broker = new VoiceCryptoBroker(admission, engine, () => now, {
    now: () => now, schedule(run, ms) { const id = ++timerSerial; timers.set(id, { at: now + ms, run }); return id; },
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
    assert.deepEqual(opened, new Uint8Array(Buffer.concat([Buffer.from([1]), session, transcriptHash]))); opened.fill(0);
    const response: VoiceIpcHeader = Object.freeze({ ...request.header, direction: 1, length: 19,
      sequence: forceSequence ?? ++nativeSequence });
    const aad = voiceIpcResponseAad(response, request.header);
    const body = new Uint8Array(await real.encrypt({ name: "AES-GCM", iv: voiceIpcNonce(response), additionalData: aad }, responseBody, new Uint8Array([1, setupClosed ? 2 : 1, 1])));
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
    held?.release(); cancelGate?.release(); fetchCancel?.release(); owner.retire(); broker.retrySelectedJoinKeys();
    await until(() => broker.counts().active === 0);
    assert.equal(broker.counts().bytes, 0);
  });
  return {
    owner, broker, native, transport, key: selected.key, admission,
    channel() { return broker.nativeChannel(owner, selected.key, native, transport); },
    counts: () => ({ ordinary, setups, settlements, steps: [...engineSteps], tx: [...requestSequences], reads: bodyReads.length }),
    hold(index: number) { holdAt = index; held = gate(); return { release: held.release, entered: () => observedStep === index }; },
    loseSetup() { loseSetup = true; }, loseSettle() { loseSettle = true; }, tamperSetup() { tamperSetup = true; },
    closeBeforeSetup() { setupClosed = true; },
    failEncrypt() { failEncrypt = true; }, badBody(value = true) { badBody = value; }, forceSequence(value: bigint | null) { forceSequence = value; },
    stallBody() { stalled = true; cancelGate = gate(); return { entered: () => cancelEntered, release: cancelGate.release }; },
    hostileHeaders() { hostileHeaders = true; },
    hangFetch() { hangFetch = true; fetchCancel = gate(); return { entered: () => fetchEntered, aborted: () => fetchAborted, release() { hangFetch = false; fetchCancel!.release(); } }; },
    advance(ms: number) { now += ms; for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.run(); } },
  };
}

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

test("HMR refuses v7 retained pool rather than creating a second channel allocator", () => {
  const host = { [Symbol.for("mewtual.voice.crypto-broker.v2")]: { version: 7 } };
  const native = { async admit() { return ""; }, async settle() {}, async commitInbound() {} };
  assert.throws(() => windowVoiceCryptoBroker(native, host, real), /reload required/);
});
