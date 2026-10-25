import { test } from "node:test";
import assert from "node:assert/strict";
import { VoiceSdpRtcOwner } from "./voice-sdp-rtc.ts";
const gate = () => { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; };
async function turn() { await new Promise<void>(yes => setImmediate(yes)); }
function fixture(low = true) {
  const old = globalThis.RTCPeerConnection;
  let owner!: VoiceSdpRtcOwner, now = 1000, live = true, complete = true;
  let localHeld: ReturnType<typeof gate> | null = null, remoteHeld: ReturnType<typeof gate> | null = null;
  let createHook: (() => void) | null = null, currentHook: (() => void) | null = null;
  let registrationHook: (() => void) | null = null;
  let remoteFails = false, local = "v=0\r\n";
  const calls = { created: 0, local: 0, remote: 0, closed: 0, audio: 0 };
  const listeners = new Set<() => void>(), timers = new Map<object, { run: () => void; deadline: number }>();
  let incoming: RTCSessionDescriptionInit | null = null;
  class Peer {
    constructor() { calls.created++; createHook?.(); }
    get iceGatheringState() { return complete ? "complete" : "gathering"; }
    get localDescription() { return { type: low ? "offer" : "answer", sdp: local }; }
    addTransceiver() { calls.audio++; }
    async setLocalDescription() { calls.local++; if (localHeld) await localHeld.promise; }
    async setRemoteDescription(description: RTCSessionDescriptionInit) {
      calls.remote++; incoming = description; if (remoteHeld) await remoteHeld.promise;
      if (remoteFails) throw new Error("RTC syntax rejected");
    }
    addEventListener(_kind: string, callback: () => void) { listeners.add(callback); registrationHook?.(); }
    removeEventListener(_kind: string, callback: () => void) { listeners.delete(callback); }
    close() { calls.closed++; }
  }
  globalThis.RTCPeerConnection = Peer as unknown as typeof RTCPeerConnection;
  owner = new VoiceSdpRtcOwner(low, () => { currentHook?.(); return live; }, {
    now: () => now,
    schedule(run, milliseconds) { const handle = {}; timers.set(handle, { run, deadline: now + milliseconds }); return handle; },
    cancel(handle) { timers.delete(handle as object); },
  });
  return { owner, calls, timers, listeners, incoming: () => incoming,
    restore() { globalThis.RTCPeerConnection = old; }, local(text: string) { local = text; }, time(value: number) { now = value; },
    onRegistration(run: () => void) { registrationHook = run; },
    stop() { live = false; owner.close(); }, gathering() { complete = false; },
    gathered() { complete = true; for (const callback of [...listeners]) callback(); },
    holdLocal() { localHeld = gate(); return localHeld; }, holdRemote() { remoteHeld = gate(); return remoteHeld; },
    failRemote() { remoteFails = true; }, onCreate(run: () => void) { createHook = run; }, onCurrent(run: () => void) { currentHook = run; },
  };
}

test("original RTC local preparation awaits fresh admission and real gathering; observers carry no description", async t => {
  const f = fixture(); t.after(f.restore); f.gathering(); f.local("x".repeat(8192)); const admitted = gate();
  const work = f.owner.prepareLocal(async () => { await admitted.promise; return 30_000; }, "offer");
  assert.throws(() => f.owner.takeLocal()); await turn(); assert.equal(f.calls.created, 0);
  admitted.release(); await turn(); assert.equal(f.calls.local, 1); assert.equal(f.calls.audio, 1);
  assert.throws(() => f.owner.takeLocal()); f.gathered();
  assert.equal(await work, undefined); const bytes = f.owner.takeLocal();
  assert.equal(bytes.length, 8192); assert.equal(bytes.every(value => value === 120), true);
  assert.throws(() => f.owner.takeLocal()); bytes.fill(0);
  assert.equal(f.listeners.size, 0); assert.equal(f.timers.size, 0); await f.owner.drain(); assert.equal(f.calls.closed, 1);
});

test("provider callbacks see published flight; constructor End closes the actual returned peer", async t => {
  const f = fixture(); t.after(f.restore); f.onCurrent(() => assert.ok((f.owner as unknown as { flight: unknown }).flight));
  f.onCreate(() => f.stop()); await assert.rejects(f.owner.prepareLocal(async () => 30_000, "offer"));
  assert.deepEqual(f.calls, { created: 1, local: 0, remote: 0, closed: 1, audio: 0 }); await f.owner.drain(); assert.equal(f.calls.closed, 1);
});

test("local admission refusal retries before SDK but never repeats an already-started SDK", async t => {
  const f = fixture(); t.after(f.restore);
  await assert.rejects(f.owner.prepareLocal(async () => { throw new Error("fresh14 uncertain"); }, "offer"));
  assert.equal(f.calls.local, 0); await f.owner.prepareLocal(async () => 30_000, "offer");
  await f.owner.prepareLocal(async () => { throw new Error("must not renew SDK"); }, "offer");
  assert.equal(f.calls.local, 1); f.owner.takeLocal().fill(0); await f.owner.drain();
});

test("gathering deadline stays the original admission deadline instead of renewing after SDK work", async t => {
  const f = fixture(); t.after(f.restore); f.gathering(); const held = f.holdLocal();
  const work = f.owner.prepareLocal(async () => 3000, "offer"); await turn(); f.time(2000); held.release(); await turn();
  assert.equal([...f.timers.values()][0].deadline, 3000); const rejected = assert.rejects(work);
  f.time(3000); [...f.timers.values()][0].run(); await rejected;
  assert.equal(f.calls.closed, 1); assert.equal(f.listeners.size, 0); assert.equal(f.timers.size, 0); await f.owner.drain();
});

test("End retains the original peer and real remote SDK promise until settlement", async t => {
  const f = fixture(false); t.after(f.restore); const held = f.holdRemote(); const bytes = new TextEncoder().encode("x".repeat(8192));
  const work = f.owner.applyRemote(bytes, "offer"); await turn(); assert.equal(f.calls.remote, 1); assert.equal(f.incoming()?.sdp?.length, 8192);
  f.stop(); let drained = false; const close = f.owner.drain().then(() => { drained = true; }); await turn();
  assert.equal(drained, false); assert.equal(f.calls.closed, 1); assert.equal(f.incoming()?.sdp?.length, 8192);
  assert.throws(() => f.owner.applyRemote(bytes, "offer")); held.release(); assert.equal(await work, undefined); await close;
  assert.equal(f.incoming()?.sdp, ""); assert.equal(f.owner.applied, true, "actual SDK success is separate from current native Applied permission");
  assert.equal((f.owner as unknown as { peer: unknown }).peer, null);
});

test("RTC parser rejection never becomes AEAD failure or invented Applied", async t => {
  const f = fixture(false); t.after(f.restore); f.failRemote();
  await assert.rejects(f.owner.applyRemote(new Uint8Array(0), "offer"), /RTC syntax rejected/);
  assert.equal(f.calls.remote, 1); assert.equal(f.owner.applied, false); assert.equal(f.incoming()?.sdp, ""); await f.owner.drain();
});

test("malformed UTF8 cannot reach the RTC SDK", async t => {
  const f = fixture(false); t.after(f.restore); await assert.rejects(f.owner.applyRemote(new Uint8Array([255]), "offer"));
  assert.equal(f.calls.remote, 0); assert.equal(f.owner.applied, false); await f.owner.drain();
});

test("bounded local UTF8 extraction rejects multibyte overflow and clears its private slot on End", async t => {
  const f = fixture(); t.after(f.restore); f.local("\u20ac".repeat(8192));
  await assert.rejects(f.owner.prepareLocal(async () => 30_000, "offer")); assert.throws(() => f.owner.takeLocal());
  await f.owner.drain(); assert.equal((f.owner as unknown as { localBytes: unknown }).localBytes, null);
});


test("End from a remote SDK method getter wins before that method is invoked", async t => {
  const f = fixture(false); t.after(f.restore); const prototype = globalThis.RTCPeerConnection.prototype;
  const original = prototype.setRemoteDescription;
  Object.defineProperty(prototype, "setRemoteDescription", { get() { f.stop(); return original; } });
  await assert.rejects(f.owner.applyRemote(new TextEncoder().encode("v=0"), "offer"));
  assert.equal(f.calls.remote, 0); assert.equal(f.calls.closed, 1); await f.owner.drain();
});


for (const time of [2500, 4000]) test(`registration advancing Clock to ${time} never renews gathering deadline`, async t => {
  const f = fixture(); t.after(f.restore); f.gathering(); f.onRegistration(() => f.time(time));
  const work = f.owner.prepareLocal(async () => 3000, "offer");
  if (time < 3000) {
    await turn(); assert.equal([...f.timers.values()][0].deadline, 3000);
    f.gathered(); await work; f.owner.takeLocal().fill(0);
  } else {
    await assert.rejects(work); assert.equal(f.timers.size, 0); assert.equal(f.listeners.size, 0);
  }
  await f.owner.drain();
});
