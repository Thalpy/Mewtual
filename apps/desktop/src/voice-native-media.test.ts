import { test, type TestContext } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { VoiceNativeMedia, VOICE_NATIVE_MEDIA_BYTES } from "./voice-native-media.ts";
import { VoiceSdpRtcOwner } from "./voice-sdp-rtc.ts";
import type { VoiceSdpOriginal } from "./voice-ipc-sdp-actions.ts";
import type { VoiceMediaReceipt } from "./voice-ipc-media.ts";
const original = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/media-vectors.json", import.meta.url), "utf8")).original as VoiceSdpOriginal;
const gate = () => { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; };
async function turn() { await new Promise<void>(yes => setImmediate(yes)); }
async function until(condition: () => boolean) { for (let n = 0; n < 200; n++) { if (condition()) return; await turn(); } assert.fail("fixture did not progress"); }

/** SDK/Root responses are controlled here. Real native framing is a separate acceptance
 * layer; these tests pin ownership, actual SDK calls and asynchronous driver ordering. */
async function fixture(t: TestContext) {
  const old = globalThis.RTCPeerConnection;
  let driver: VoiceNativeMedia | null = null, now = 1000, live = true, ready = true;
  let failClose = false, loseAdmit = false, loseRelease = false, loseDetach = false, loseHistory = false;
  let historyResponse: "ok" | "current" | "absent" | "refuse" = "ok";
  let inCurrent = false, expireInCurrent = false;
  let heldMode: number | null = null, held: ReturnType<typeof gate> | null = null;
  let onSchedule: (() => void) | null = null, onCancel: (() => void) | null = null, onNow: (() => void) | null = null;
  const timers = new Map<object, { due: number; work: () => void }>();
  const requests: number[] = [], counts = { created: 0, closed: 0, active: 0, refund: 0, paid: VOICE_NATIVE_MEDIA_BYTES, stops: 0 };
  let admitted = false, nativeClosed = false, released = false;
  const receipt = (current = false): VoiceMediaReceipt => Object.freeze({ proof: "17".repeat(32), admitted,
    closed: nativeClosed, released, current: current && admitted && !nativeClosed, retireInMs: current ? 0 : null });
  class Peer {
    constructor() { counts.created++; }
    iceGatheringState = "complete";
    get localDescription() { return { type: "offer", sdp: "v=0" }; }
    addTransceiver() {} addEventListener() {} removeEventListener() {}
    async setLocalDescription() {} async setRemoteDescription() {}
    close() { counts.closed++; if (failClose) throw new Error("SDK close failed"); }
  }
  globalThis.RTCPeerConnection = Peer as unknown as typeof RTCPeerConnection;
  t.after(() => { globalThis.RTCPeerConnection = old; held?.release(); });
  const clock = { now: () => { const hook = onNow; onNow = null; hook?.(); return now; },
    schedule(work: () => void, delay: number) { const handle = {}; timers.set(handle, { work, due: now + delay }); onSchedule?.(); return handle; },
    cancel(handle: unknown) { onCancel?.(); timers.delete(handle as object); },
  };
  const source = new VoiceSdpRtcOwner(true, () => {
    inCurrent = true;
    try { return driver?.transferCurrent(source) ?? ready; }
    finally { inCurrent = false; }
  }, clock);
  await source.prepareLocal(async () => 30_000, "offer"); source.takeLocal().fill(0);
  await source.applyRemote(new TextEncoder().encode("v=0"), "answer");
  driver = new VoiceNativeMedia(original, source, { clock, live: () => { if (inCurrent && expireInCurrent) now = 6000; return live; },
    begin: () => { counts.active++; }, end: () => { counts.active--; },
    refund: () => { counts.refund++; counts.paid = 0; },
    stopped: () => { counts.stops++; live = false; },
    request: async (request, cleanup) => {
      const mode = request.body[1]; requests.push(mode);
      assert.equal(cleanup, mode !== 0 && mode !== 4 && mode !== 7);
      if (mode === 0) admitted = true;
      if (mode === heldMode) await held!.promise; // deliberately ignores abort/Close
      if (mode === 0 && loseAdmit) { loseAdmit = false; throw new Error("lost Admit"); }
      if (mode === 7 && loseDetach) { loseDetach = false; throw new Error("lost Detach"); }
      if (mode === 8) {
        if (loseHistory) { loseHistory=false; throw new Error("lost positive native history result"); }
        if (historyResponse === "refuse") throw new Error("native history pending");
        if (historyResponse === "absent") return null;
        if (historyResponse === "current") return receipt(true);
      }
      if (mode === 2) nativeClosed = true;
      if (mode === 3) {
        assert.ok(nativeClosed); assert.ok(counts.closed > 0, "native Release follows actual SDK drain");
        released = true;
        if (loseRelease) { loseRelease = false; throw new Error("lost Release"); }
      }
      return mode === 1 && !admitted ? null : receipt(mode === 4 || mode === 7);
    },
  });
  return { driver, source, counts, requests, timers,
    hold(mode: number) { heldMode = mode; held = gate(); return held; },
    historyReply(value: typeof historyResponse) { historyResponse=value; },
    loseHistory() { loseHistory=true; },
    loseAdmit() { loseAdmit = true; }, loseRelease() { loseRelease = true; }, loseDetach() { loseDetach = true; },
    failClose(value: boolean) { failClose = value; },
    onSchedule(hook: () => void) { onSchedule = hook; }, onCancel(hook: () => void) { onCancel = hook; },
    onNow(hook: () => void) { onNow = hook; },
    ready(value: boolean) { ready = value; }, time(value: number) { now = value; },
    expireFromCurrentPredicate() { expireInCurrent = true; },
    advance(delta: number) {
      now += delta;
      for (const [handle, timer] of [...timers]) if (timer.due <= now) { timers.delete(handle); timer.work(); }
    },
  };
}

test("one native handoff transfers the same peer, monitors it and releases only after actual cleanup", async t => {
  const f = await fixture(t); const work = f.driver.start(); assert.equal(f.driver.start(), work); await work;
  assert.equal(f.driver.adopted, true); assert.deepEqual(f.requests, [0, 4]);
  await f.source.drain(); assert.equal(f.counts.closed, 0, "old owner cannot close transferred media");
  f.advance(1000); await until(() => f.requests.filter(mode => mode === 4).length === 2);
  await turn(); assert.equal(f.driver.closed, false); assert.equal(f.timers.size, 1);
  await f.driver.cleanup(); assert.equal(f.counts.closed, 1); assert.equal(f.counts.paid, 0);
  assert.equal(f.counts.refund, 1); assert.equal(f.counts.active, 0); assert.deepEqual(f.requests.slice(-2), [2, 3]);
  await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});
for (const mode of [0, 4]) test(`acquisition expiry closes the real peer while native mode${mode} ignores cancellation`, async t => {
  const f = await fixture(t), held = f.hold(mode), work = f.driver.start();
  await until(() => f.requests.includes(mode)); f.advance(5000);
  assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1); assert.equal(f.driver.adopted, false);
  let done = false; const cleanup = f.driver.cleanup().then(() => { done = true; }); await turn();
  assert.equal(done, false); assert.equal(f.counts.paid, VOICE_NATIVE_MEDIA_BYTES);
  held.release(); await work; await cleanup; assert.equal(done, true); assert.equal(f.counts.paid, 0);
});
test("stale deadline remains armed while an adopted peer's Current request hangs", async t => {
  const f = await fixture(t); await f.driver.start(); const held = f.hold(4);
  f.advance(1000); await until(() => f.requests.filter(mode => mode === 4).length === 2);
  f.advance(4000); assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1);
  const closing = f.driver.cleanup(); await turn(); assert.equal(f.counts.paid, VOICE_NATIVE_MEDIA_BYTES);
  held.release(); await closing; assert.equal(f.counts.refund, 1); assert.equal(f.counts.closed, 1);
});
test("lost Admit recovers the same native claim and transfers after old Ready expires", async t => {
  const f = await fixture(t); f.loseAdmit(); await f.driver.start();
  assert.equal(f.driver.adopted, false); f.ready(false); f.advance(1000);
  await until(() => f.driver.adopted); assert.deepEqual(f.requests.slice(0, 3), [0, 1, 4]);
  assert.equal(f.requests.filter(mode => mode === 0).length, 1);
  await f.driver.cleanup(); assert.equal(f.counts.closed, 1); assert.equal(f.counts.refund, 1);
});
test("a late first Current cannot extend acquisition when its timer was throttled", async t => {
  const f = await fixture(t), held = f.hold(4), work = f.driver.start();
  await until(() => f.requests.includes(4)); f.time(6000); held.release(); await work;
  assert.equal(f.driver.adopted, false); assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1);
  await f.driver.cleanup();
});
test("lost native Release retains payment and recovers exact release through original Close", async t => {
  const f = await fixture(t); await f.driver.start(); f.loseRelease();
  await assert.rejects(f.driver.cleanup(), /lost Release/); assert.equal(f.counts.paid, VOICE_NATIVE_MEDIA_BYTES);
  await f.driver.cleanup(); assert.deepEqual(f.requests.slice(-3), [2, 3, 2]); assert.equal(f.counts.refund, 1);
});
test("throwing SDK close cannot release native custody or refund the driver", async t => {
  const f = await fixture(t); await f.driver.start(); f.failClose(true);
  await assert.rejects(f.driver.cleanup(), /SDK close failed/);
  assert.equal(f.requests.includes(3), false); assert.equal(f.counts.refund, 0);
  f.failClose(false); await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});
test("scheduler End reentry cancels the actual returned registration before refund", async t => {
  const f = await fixture(t); f.onSchedule(() => f.driver.close());
  await f.driver.start(); assert.equal(f.driver.closed, true); assert.equal(f.requests.length, 0);
  assert.equal(f.counts.closed, 1); await f.driver.cleanup(); assert.equal(f.timers.size, 0); assert.equal(f.counts.refund, 1);
});
test("cancel reentry coalesces cleanup and cannot refund an entered provider frame", async t => {
  const f = await fixture(t); await f.driver.start(); let closing: Promise<void> | null = null;
  f.onCancel(() => { closing = f.driver.cleanup(); f.driver.close(); assert.equal(f.counts.refund, 0); });
  f.driver.close(); await closing!; assert.equal(f.counts.refund, 1); assert.equal(f.counts.active, 0);
});


test("an early one-shot keeps monitoring the adopted peer through hung refresh expiry", async t => {
  const f = await fixture(t); await f.driver.start();
  const [handle, timer] = [...f.timers][0]; f.timers.delete(handle); timer.work();
  assert.equal(f.timers.size, 1); assert.equal(f.driver.closed, false);
  const held = f.hold(4); f.advance(1000);
  await until(() => f.requests.filter(mode => mode === 4).length === 2);
  f.advance(4000); assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1);
  held.release(); await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});

for (const value of [NaN, 999]) test(`invalid timer clock ${value} closes peer during hung Current`, async t => {
  const f = await fixture(t); await f.driver.start(); const held = f.hold(4);
  f.advance(1000); await until(() => f.requests.filter(mode => mode === 4).length === 2);
  const [handle, timer] = [...f.timers][0]; f.timers.delete(handle); f.time(value); timer.work();
  assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1);
  held.release(); await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});


test("an old notification inside cancel cannot strand an overwritten timer handle", async t => {
  const f = await fixture(t); await f.driver.start();
  const oldNotification = [...f.timers.values()][0].work;
  let once = true;
  f.onCancel(() => { if (once) { once = false; oldNotification(); } });
  await f.driver.start();
  assert.equal(f.timers.size, 1); assert.equal(f.driver.closed, false);
  await f.driver.cleanup(); assert.equal(f.timers.size, 0); assert.equal(f.counts.refund, 1);
});


test("expired media at transfer cannot fall back to still-valid old Ready", async t => {
  const f = await fixture(t); f.expireFromCurrentPredicate(); await f.driver.start();
  assert.deepEqual(f.requests, [0, 4]); assert.equal(f.driver.adopted, false);
  assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1);
  await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});


test("schedule registering then throwing retains unknown timer debt until its actual callback", async t => {
  const f = await fixture(t); f.onSchedule(() => { throw new Error("schedule failed after registration"); });
  await f.driver.start(); assert.equal(f.driver.closed, true); assert.equal(f.timers.size, 1);
  assert.equal(f.requests.length, 0); assert.equal(f.counts.closed, 1);
  await assert.rejects(f.driver.cleanup(), /unavailable/); assert.equal(f.counts.refund, 0);
  f.advance(5000); await f.driver.cleanup(); assert.equal(f.timers.size, 0); assert.equal(f.counts.refund, 1);
});

test("an old queued notification cannot join a newer unknown registration", async t => {
  const f = await fixture(t); await f.driver.start(); const old = [...f.timers.values()][0].work;
  f.onSchedule(() => { throw new Error("schedule failed after registration"); });
  await f.driver.start(); assert.equal(f.driver.closed, true); assert.equal(f.timers.size, 1);
  old(); await assert.rejects(f.driver.cleanup(), /unavailable/); assert.equal(f.counts.refund, 0);
  f.advance(5000); await f.driver.cleanup(); assert.equal(f.timers.size, 0); assert.equal(f.counts.refund, 1);
});


test("ordinary fractional acquisition time does not close a valid original peer", async t => {
  const f = await fixture(t); f.time(3192.004); await f.driver.start();
  assert.equal(f.driver.adopted, true); assert.equal(f.driver.closed, false);
  await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});

test("lost Admit recovery at a fractional Current start preserves the same valid claim", async t => {
  const f = await fixture(t); f.loseAdmit(); await f.driver.start();
  f.time(3192.004); f.ready(false); await f.driver.start();
  assert.deepEqual(f.requests, [0, 1, 4]); assert.equal(f.driver.adopted, true);
  await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});


test("same native successor is requested only after actual peer adoption", async t => {
  const f = await fixture(t); assert.throws(() => f.driver.requestDetach(), /unavailable/);
  await f.driver.start(); await f.driver.requestDetach(); assert.equal(f.driver.detached, true);
  assert.deepEqual(f.requests, [0,4,4,7]); assert.equal(f.counts.created, 1); assert.equal(f.counts.closed, 0);
  await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});

test("lost detach observation repeats the same indexed successor without another admission", async t => {
  const f = await fixture(t); await f.driver.start(); f.loseDetach(); await f.driver.requestDetach();
  assert.equal(f.driver.detached, false); await f.driver.start(); assert.equal(f.driver.detached, true);
  assert.deepEqual(f.requests, [0,4,4,7,4,7]); await f.driver.cleanup();
});

test("a refreshed deadline stays monitored while the same flight waits on detach", async t => {
  const f = await fixture(t); await f.driver.start(); f.time(2000); const held=f.hold(7), request=f.driver.requestDetach();
  await until(() => f.requests.includes(7)); f.advance(4000);
  assert.equal(f.driver.closed, false); assert.equal(f.timers.size, 1);
  f.advance(1000); assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1);
  const cleanup=f.driver.cleanup(); await turn(); assert.equal(f.counts.refund, 0);
  held.release(); await request; await cleanup; assert.equal(f.counts.refund, 1);
});

test("native history collection needs exact detachment and keeps browser payment and peer", async t => {
  const f=await fixture(t); assert.throws(() => f.driver.collectNativeHistory(), /unavailable/);
  await f.driver.start(); assert.throws(() => f.driver.collectNativeHistory(), /unavailable/);
  await f.driver.requestDetach(); assert.equal(await f.driver.collectNativeHistory(),true);
  assert.deepEqual(f.requests,[0,4,4,7,4,8]);
  assert.equal(f.counts.paid,VOICE_NATIVE_MEDIA_BYTES); assert.equal(f.counts.closed,0);
  const calls=f.requests.length; assert.equal(await f.driver.collectNativeHistory(),true);
  assert.equal(f.requests.length,calls,"completed history needs no replacement request");
  await f.driver.cleanup(); assert.equal(f.counts.refund,1);
});
for (const response of ["absent","refuse","current"] as const) test(`native history ${response} cannot count as completed removal`, async t => {
  const f=await fixture(t); await f.driver.start(); await f.driver.requestDetach(); f.historyReply(response);
  assert.equal(await f.driver.collectNativeHistory(),false);
  assert.equal(f.counts.closed,0); assert.equal(f.counts.paid,VOICE_NATIVE_MEDIA_BYTES);
  f.historyReply("ok"); assert.equal(await f.driver.collectNativeHistory(),true);
  await f.driver.cleanup();
});
test("lost positive history response recovers the same claim without admission or refund", async t => {
  const f=await fixture(t); await f.driver.start(); await f.driver.requestDetach(); f.loseHistory();
  assert.equal(await f.driver.collectNativeHistory(),false); assert.equal(await f.driver.collectNativeHistory(),true);
  assert.equal(f.requests.filter(mode=>mode===0).length,1); assert.equal(f.requests.filter(mode=>mode===8).length,2);
  assert.equal(f.counts.created,1); assert.equal(f.counts.closed,0); assert.equal(f.counts.refund,0);
  await f.driver.cleanup();
});
test("history IO remains paid and monitored through expiry and actual return", async t => {
  const f=await fixture(t); await f.driver.start(); await f.driver.requestDetach();
  const held=f.hold(8), work=f.driver.collectNativeHistory(); await until(()=>f.requests.includes(8));
  f.advance(5000); assert.equal(f.counts.closed,1); assert.equal(f.driver.closed,true);
  let done=false; const cleanup=f.driver.cleanup().then(()=>{done=true;}); await turn();
  assert.equal(done,false); assert.equal(f.counts.refund,0); held.release(); await work; await cleanup;
  assert.equal(f.counts.refund,1); assert.equal(f.counts.closed,1);
});

test("held history has one observer for repeated callers and joins before refund", async t=>{
  const f=await fixture(t); await f.driver.start(); await f.driver.requestDetach();
  const held=f.hold(8), first=f.driver.collectNativeHistory();
  for(let n=0;n<1000;n++) assert.equal(f.driver.collectNativeHistory(),first);
  await until(()=>f.requests.includes(8));
  for(let n=0;n<1000;n++) assert.equal(f.driver.collectNativeHistory(),first);
  assert.equal(f.requests.filter(mode=>mode===8).length,1);
  f.driver.close(); const cleanup=f.driver.cleanup(); assert.equal(f.counts.closed,1); await turn();
  assert.equal(f.counts.refund,0); held.release(); await first; await cleanup;
  assert.equal(f.counts.refund,1); assert.equal(f.counts.active,0);
});

test("End before the published history observer starts cannot send native history work", async t=>{
  const f=await fixture(t); await f.driver.start(); await f.driver.requestDetach();
  const observer=f.driver.collectNativeHistory(); f.driver.close(); const cleanup=f.driver.cleanup();
  assert.equal(f.counts.closed,1); assert.equal(await observer,false); await cleanup;
  assert.equal(f.requests.includes(8),false); assert.equal(f.counts.refund,1);
});


test("an early refresh wake crossing its cadence during rearm does not close a current peer", async t => {
  const f = await fixture(t); await f.driver.start();
  const [handle, timer] = [...f.timers][0]; f.timers.delete(handle);
  f.time(1999.9); let crossed = false;
  f.onCancel(() => { if (!crossed) { crossed = true; f.time(2000.1); } });
  timer.work();
  assert.equal(crossed, true);
  assert.equal(f.driver.closed, false, "refresh cadence is not the authenticated freshness deadline");
  assert.equal(f.counts.closed, 0); assert.equal(f.timers.size, 1);
  f.onCancel(() => {}); const held = f.hold(4); f.advance(1);
  await until(() => f.requests.filter(mode => mode === 4).length === 2);
  // The newly STARTED request has not returned. Rearming a refresh must not extend the
  // original1000+5000 observation, even if the new request could later authorize more.
  f.time(6000); f.advance(0); assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1);
  const cleanup = f.driver.cleanup(); await turn(); assert.equal(f.counts.refund, 0);
  held.release(); await cleanup; assert.equal(f.counts.refund, 1);
});


test("crossing the actual native freshness deadline during early rearm closes without another Current", async t => {
  const f = await fixture(t); await f.driver.start();
  const [handle, timer] = [...f.timers][0]; f.timers.delete(handle); f.time(1999.9);
  f.onCancel(() => f.time(6000)); timer.work();
  assert.equal(f.driver.closed, true); assert.equal(f.counts.closed, 1);
  assert.deepEqual(f.requests, [0, 4]); assert.equal(f.timers.size, 0);
  f.onCancel(() => {}); await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});

test("End inside the rearm Clock prevents any new timer registration", async t => {
  const f = await fixture(t); await f.driver.start();
  const [handle, timer] = [...f.timers][0]; f.timers.delete(handle); f.time(1999.9);
  let schedules = 0;
  f.onSchedule(() => { schedules++; });
  f.onCancel(() => f.onNow(() => f.driver.close())); timer.work();
  assert.equal(f.driver.closed, true); assert.equal(schedules, 0); assert.equal(f.timers.size, 0);
  f.onCancel(() => {}); await f.driver.cleanup(); assert.equal(f.counts.refund, 1);
});
