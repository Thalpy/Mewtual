import { test } from "node:test";
import assert from "node:assert/strict";
import { VoiceSdpRtcOwner } from "./voice-sdp-rtc.ts";
const gate = () => { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; };
async function turn() { await new Promise<void>(yes => setImmediate(yes)); }
function fixture(low = true) {
  const old = globalThis.RTCPeerConnection;
  let live = true, currentHook: (() => void) | null = null, clockHook: (() => void) | null = null;
  let closeHook: (() => void) | null = null, hold: ReturnType<typeof gate> | null = null;
  const calls = { created: 0, local: 0, remote: 0, closed: 0, clock: 0 };
  const transceiver = { direction: "recvonly", receiver: { track: { kind: "audio" } } };
  class Peer {
    constructor() { calls.created++; }
    get iceGatheringState() { return "complete"; }
    get localDescription() { return { type: low ? "offer" : "answer", sdp: "v=0\r\n" }; }
    addTransceiver() {}
    getTransceivers() { return [transceiver]; }
    async setLocalDescription() { calls.local++; }
    async setRemoteDescription() { calls.remote++; if (hold) await hold.promise; }
    addEventListener() {}
    removeEventListener() {}
    close() { calls.closed++; closeHook?.(); }
  }
  globalThis.RTCPeerConnection = Peer as unknown as typeof RTCPeerConnection;
  const source = new VoiceSdpRtcOwner(low, () => { currentHook?.(); return live; }, {
    now() { calls.clock++; clockHook?.(); return 1000; }, schedule() { return {}; }, cancel() {},
  });
  return { source, calls, restore() { globalThis.RTCPeerConnection = old; },
    stop() { live = false; source.close(); }, onCurrent(hook: () => void) { currentHook = hook; },
    onClock(hook: () => void) { clockHook = hook; }, onClose(hook: () => void) { closeHook = hook; },
    holdRemote() { hold = gate(); return hold; },
    async complete() {
      if (low) {
        await source.prepareLocal(async () => 30_000, "offer"); source.takeLocal().fill(0);
        await source.applyRemote(new TextEncoder().encode("v=0"), "answer");
      } else {
        await source.applyRemote(new TextEncoder().encode("v=0"), "offer");
        await source.prepareLocal(async () => 30_000, "answer"); source.takeLocal().fill(0);
        assert.equal(transceiver.direction, "sendrecv", "the transferred HIGH peer permits both negotiated audio directions");
      }
    },
  };
}

for (const low of [false, true]) test(`affine media leaf moves the same complete ${low ? "LOW" : "HIGH"} peer once and severs old currentness`, async t => {
  const f = fixture(low); t.after(f.restore);
  const target = f.source.prepareMediaDestination();
  assert.equal(f.source.prepareMediaDestination(), target); assert.equal(target.adopted, false);
  assert.equal(f.calls.created, 0, "destination construction is inert");
  await f.complete();
  const previousClock = f.calls.clock;
  f.onClock(() => { throw new Error("no Clock callback in transfer"); });
  f.source.transferInto(target);
  assert.equal(f.calls.clock, previousClock); assert.equal(target.adopted, true); assert.equal(target.closed, false);
  for (const name of ["peer", "current", "mediaDestination"]) assert.equal(Object.hasOwn(f.source, name), false, "ownership/callback fields are runtime-private");
  assert.equal(f.calls.created, 1); assert.equal(f.calls.closed, 0);
  f.source.close(); await f.source.drain();
  assert.equal(f.calls.closed, 0, "old cleanup cannot close the moved peer");
  assert.throws(() => f.source.transferInto(target)); assert.throws(() => f.source.prepareMediaDestination());
  target.close(); target.close(); assert.equal(target.drained, false, "close intent is not owner disposal");
  await target.drain(); await target.drain();
  assert.equal(target.drained, true); assert.equal(target.adopted, true); assert.equal(f.calls.closed, 1);
});

test("foreign and forged destinations cannot take the original peer", async t => {
  const a = fixture(); t.after(a.restore); await a.complete();
  const b = fixture(); t.after(b.restore); const foreign = b.source.prepareMediaDestination();
  const own = a.source.prepareMediaDestination();
  for (const wrong of [foreign, Object.create(Object.getPrototypeOf(own)), {}]) assert.throws(() => a.source.transferInto(wrong));
  assert.equal(a.calls.closed, 0); assert.equal(own.adopted, false); assert.equal(foreign.adopted, false);
  a.source.transferInto(own); await own.drain(); await b.source.drain(); assert.equal(a.calls.closed, 1);
});

test("partial round, retained local bytes and pending SDK do not lose the original owner", async t => {
  const f = fixture(); t.after(f.restore); const target = f.source.prepareMediaDestination();
  assert.throws(() => f.source.transferInto(target));
  await f.source.prepareLocal(async () => 30_000, "offer");
  assert.throws(() => f.source.transferInto(target));
  const held = f.holdRemote(); const remote = f.source.applyRemote(new TextEncoder().encode("v=0"), "answer");
  await turn(); assert.equal(f.calls.remote, 1); assert.throws(() => f.source.transferInto(target));
  held.release(); await remote;
  assert.throws(() => f.source.transferInto(target), "local UTF8 must be consumed before detaching its envelope");
  f.source.takeLocal().fill(0); f.source.transferInto(target); await target.drain(); assert.equal(f.calls.closed, 1);
});

test("a closed destination never becomes reusable and does not close its source", async t => {
  const f = fixture(); t.after(f.restore); const target = f.source.prepareMediaDestination();
  await target.drain(); await f.complete();
  assert.equal(f.source.prepareMediaDestination(), target); assert.equal(target.drained, true);
  assert.throws(() => f.source.transferInto(target)); assert.equal(target.adopted, false); assert.equal(f.calls.closed, 0);
  await f.source.drain(); assert.equal(f.calls.closed, 1);
});

for (const closesTarget of [false, true]) test(`reentrant currentness ${closesTarget ? "destination Close" : "source End"} wins before ownership mutation`, async t => {
  const f = fixture(); t.after(f.restore); await f.complete(); const target = f.source.prepareMediaDestination();
  f.onCurrent(() => { if (closesTarget) target.close(); else f.stop(); });
  assert.throws(() => f.source.transferInto(target)); assert.equal(target.adopted, false);
  assert.equal(f.calls.closed, closesTarget ? 0 : 1); await f.source.drain(); await target.drain();
  assert.equal(f.calls.closed, 1);
});

test("a reentrant transfer refuses without creating two owners", async t => {
  const f = fixture(); t.after(f.restore); await f.complete(); const target = f.source.prepareMediaDestination();
  f.onCurrent(() => assert.throws(() => f.source.transferInto(target)));
  f.source.transferInto(target); assert.equal(target.adopted, true); await target.drain(); assert.equal(f.calls.closed, 1);
});

test("a late bookkeeping getter closing the empty destination cannot steal an unclosed peer", async t => {
  const f = fixture(); t.after(f.restore); await f.complete(); const target = f.source.prepareMediaDestination();
  Object.defineProperty(f.source, "localDone", { get() { target.close(); return true; } });
  assert.throws(() => f.source.transferInto(target));
  assert.equal(target.closed, true); assert.equal(target.adopted, false); assert.equal(f.calls.closed, 0);
  await target.drain(); assert.equal(target.drained, true); assert.equal(f.calls.closed, 0);
  await f.source.drain(); assert.equal(f.calls.closed, 1, "actual peer never left its paid original owner");
});

test("late bookkeeping reentry cannot make an inner transfer win invisibly", async t => {
  const f = fixture(); t.after(f.restore); await f.complete(); const target = f.source.prepareMediaDestination();
  Object.defineProperty(f.source, "localDone", { get() { assert.throws(() => f.source.transferInto(target)); return true; } });
  f.source.transferInto(target); assert.equal(target.adopted, true);
  await f.source.drain(); assert.equal(f.calls.closed, 0); await target.drain(); assert.equal(f.calls.closed, 1);
});

test("transfer calls no overridable destination method", async t => {
  const f = fixture(); t.after(f.restore); await f.complete(); const target = f.source.prepareMediaDestination();
  Object.defineProperty(target, "close", { configurable: true, get() { throw new Error("unexpected destination getter"); } });
  f.source.transferInto(target); assert.equal(target.adopted, true); assert.equal(f.calls.closed, 0);
  delete (target as unknown as { close?: unknown }).close;
  await target.drain(); assert.equal(f.calls.closed, 1);
});

test("throwing media SDK close retains custody and supports explicit idempotent cleanup", async t => {
  const f = fixture(); t.after(f.restore); await f.complete(); const target = f.source.prepareMediaDestination();
  f.source.transferInto(target); let fails = true;
  f.onClose(() => { if (fails) throw new Error("close provider failed"); });
  await assert.rejects(target.drain(), /close provider failed/);
  assert.equal(target.closed, true); assert.equal(target.adopted, true); assert.equal(target.drained, false);
  await f.source.drain(); assert.equal(f.calls.closed, 1);
  fails = false; await target.drain(); assert.equal(target.drained, true); assert.equal(f.calls.closed, 2);
});

test("reentrant media drain during actual close cannot report completion early", async t => {
  const f = fixture(); t.after(f.restore); await f.complete(); const target = f.source.prepareMediaDestination();
  f.source.transferInto(target); let nested: Promise<void> | null = null;
  f.onClose(() => { nested = target.drain(); assert.equal(target.drained, false); });
  target.close(); await assert.rejects(nested!, /close still running/);
  assert.equal(target.drained, false); await target.drain(); assert.equal(target.drained, true); assert.equal(f.calls.closed, 1);
});
