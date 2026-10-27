import assert from "node:assert/strict";
import test from "node:test";
import { VoiceAudioCapturePool, VOICE_AUDIO_CAPTURE_BYTES,
  type VoiceAudioCaptureBudget } from "./voice-audio-capture.ts";
import { VoiceMediaStatus } from "./voice-media-status.ts";

function gate<T>() { let release!: (value: T) => void; const promise = new Promise<T>(yes => { release = yes; }); return { promise, release }; }
function authority() {
  let now = 100;
  const proof = "12".repeat(32), status = new VoiceMediaStatus(proof);
  status.finish(status.begin(now), { proof, admitted: true, current: true, closed: false, released: false, retireInMs: 0 }, now);
  return { status, clock: { now: () => now, schedule() { assert.fail(); }, cancel() { assert.fail(); } }, at(at: number) { now = at; } };
}
function ledger(hook: () => void = () => {}, released: () => void = () => {}) {
  let paid = 0, reserves = 0, returns = 0;
  const budget: VoiceAudioCaptureBudget = { reserve() {
    hook(); paid += VOICE_AUDIO_CAPTURE_BYTES; reserves++;
    let returned = false;
    return { release() { assert.equal(returned, false); returned = true; returns++; paid -= VOICE_AUDIO_CAPTURE_BYTES; released(); } };
  } };
  return { budget, paid: () => paid, reserves: () => reserves, returns: () => returns };
}
function stream(options: { kind?: string; count?: number; onTracks?(): void; onEnable?(): void; onStop?(): void } = {}) {
  let enabled = true, stopped = 0;
  const track = { kind: options.kind ?? "audio", readyState: "live",
    get enabled() { return enabled; }, set enabled(value: boolean) { options.onEnable?.(); enabled = value; },
    stop() { options.onStop?.(); stopped++; },
  };
  const actual = { getTracks() { options.onTracks?.(); return Array.from({ length: options.count ?? 1 }, () => track); } } as unknown as MediaStream;
  return { actual, enabled: () => enabled, stopped: () => stopped };
}

test("one paid OS request is coalesced; the same capsule owns the disabled master until actual stop", async () => {
  const pool = new VoiceAudioCapturePool(), fees = ledger(), session = pool.session(fees.budget), live = authority();
  const held = gate<MediaStream>(), entered = gate<void>(); let requests = 0;
  const media = stream({ onStop() { assert.equal(fees.paid(), VOICE_AUDIO_CAPTURE_BYTES); } });
  const devices = { getUserMedia(constraints: MediaStreamConstraints) {
    requests++; assert.equal(fees.paid(), VOICE_AUDIO_CAPTURE_BYTES);
    assert.deepEqual(constraints, { video: false, audio: { deviceId: { exact: "device-a" },
      echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
    entered.release(); return held.promise;
  } };
  assert.equal(session.request(live, "device-a", devices), true); await entered.promise;
  assert.equal(session.request(live, "device-b", devices), false);
  assert.throws(() => pool.session(fees.budget));
  held.release(media.actual); await pool.settlement();
  assert.equal(requests, 1); assert.equal(session.available, true); assert.equal(media.enabled(), false);
  assert.equal(fees.paid(), VOICE_AUDIO_CAPTURE_BYTES); assert.equal(pool.pending, false);
  session.close(); assert.equal(media.stopped(), 1); assert.equal(fees.paid(), 0); assert.equal(fees.returns(), 1);
  session.close(); assert.equal(media.stopped(), 1);
});

test("old pending permission keeps its original fee; new listen-only session never adopts the late stream", async () => {
  const pool = new VoiceAudioCapturePool(), oldFees = ledger(), newFees = ledger(), live = authority();
  const old = pool.session(oldFees.budget), held = gate<MediaStream>(), entered = gate<void>();
  const devices = { getUserMedia() { entered.release(); return held.promise; } };
  old.request(live, "", devices); await entered.promise; old.close();
  const next = pool.session(newFees.budget);
  assert.equal(next.request(live, "", devices), false);
  assert.equal(newFees.reserves(), 0); assert.equal(oldFees.paid(), VOICE_AUDIO_CAPTURE_BYTES);
  const late = stream(); held.release(late.actual); await pool.settlement();
  assert.equal(late.stopped(), 1); assert.equal(late.enabled(), false);
  assert.equal(old.available, false); assert.equal(next.available, false); assert.equal(oldFees.paid(), 0);
  const fresh = stream(); assert.equal(next.request(live, "", { async getUserMedia() { return fresh.actual; } }), true);
  await pool.settlement(); assert.equal(next.available, true); assert.equal(fresh.stopped(), 0);
  next.close(); assert.equal(fresh.stopped(), 1); assert.equal(newFees.paid(), 0);
});

for (const negative of ["expired", "closed", "provider", "bad-track", "extra-track"] as const) {
  test(`capture refusal is listen-only with actual cleanup: ${negative}`, async () => {
    const pool = new VoiceAudioCapturePool(), fees = ledger(), session = pool.session(fees.budget), live = authority();
    const held = gate<MediaStream>(), entered = gate<void>(), media = stream({ kind: negative === "bad-track" ? "video" : "audio", count: negative === "extra-track" ? 2 : 1 });
    const devices = { async getUserMedia() { entered.release(); if (negative === "provider") throw new Error("device refused with private details"); return held.promise; } };
    assert.equal(session.request(live, "", devices), true); await entered.promise;
    if (negative === "expired") live.at(5100);
    if (negative === "closed") live.status.close();
    held.release(media.actual); await pool.settlement();
    assert.equal(session.available, false); assert.equal(fees.paid(), 0); assert.equal(pool.pending, false);
    if (negative !== "provider") assert.ok(media.stopped() > 0);
    session.close();
  });
}

test("non-current media, oversized device setting and group refusal never enter the SDK", async () => {
  const pool = new VoiceAudioCapturePool(), fees = ledger(), live = authority(), session = pool.session(fees.budget);
  const devices = { async getUserMedia(): Promise<MediaStream> { assert.fail("SDK cannot run"); } };
  assert.equal(session.request(live, "x".repeat(1025), devices), false);
  live.status.close(); assert.equal(session.request(live, "", devices), false); assert.equal(fees.reserves(), 0); session.close();
  const refusing = pool.session({ reserve() { throw new Error("group capacity"); } });
  assert.equal(refusing.request(authority(), "", devices), false); assert.equal(pool.pending, false); refusing.close();
});

test("reserve reentry cannot overwrite the original capsule or start a replacement OS request", async () => {
  const pool = new VoiceAudioCapturePool(), live = authority(), newFees = ledger();
  let next: ReturnType<VoiceAudioCapturePool["session"]> | null = null;
  const devices = { async getUserMedia(): Promise<MediaStream> { assert.fail("End precedes SDK entry"); } };
  const oldFees = ledger(() => {
    old.close(); next = pool.session(newFees.budget);
    assert.equal(next.request(live, "", devices), false);
  });
  const old = pool.session(oldFees.budget);
  assert.equal(old.request(live, "", devices), true); await pool.settlement();
  assert.equal(oldFees.paid(), 0); assert.equal(newFees.reserves(), 0); assert.equal(pool.pending, false);
  (next as ReturnType<VoiceAudioCapturePool["session"]> | null)?.close();
});

for (const hook of ["method", "tracks", "enabled"] as const) {
  test(`End at the last ${hook} getter cannot publish a live master`, async () => {
    const pool = new VoiceAudioCapturePool(), fees = ledger(), session = pool.session(fees.budget), live = authority();
    const media = stream({ onTracks() { if (hook === "tracks") session.close(); }, onEnable() { if (hook === "enabled") session.close(); } });
    let calls = 0;
    const devices = { get getUserMedia() {
      if (hook === "method") session.close();
      return async () => { calls++; return media.actual; };
    } };
    session.request(live, "", devices); await pool.settlement();
    assert.equal(session.available, false); assert.equal(fees.paid(), 0);
    if (hook === "method") assert.equal(calls, 0); else assert.equal(media.stopped(), 1);
  });
}

test("actual failed stop stays quarantined and paid; reentrant replacement capture is refused", async () => {
  const pool = new VoiceAudioCapturePool(), fees = ledger(), live = authority(), session = pool.session(fees.budget), nextFees = ledger();
  let fail = true, next: ReturnType<VoiceAudioCapturePool["session"]> | null = null;
  const devices = { async getUserMedia(): Promise<MediaStream> { assert.fail("replacement remains listen-only"); } };
  const media = stream({ onStop() {
    assert.equal(fees.paid(), VOICE_AUDIO_CAPTURE_BYTES);
    if (!next) { next = pool.session(nextFees.budget); assert.equal(next.request(live, "", devices), false); }
    if (fail) throw new Error("SDK close failed");
  } });
  session.request(live, "", { async getUserMedia() { return media.actual; } }); await pool.settlement();
  session.close(); assert.equal(fees.paid(), VOICE_AUDIO_CAPTURE_BYTES); assert.equal(pool.pending, true); assert.equal(nextFees.reserves(), 0);
  fail = false; session.close(); assert.equal(media.stopped(), 1); assert.equal(fees.paid(), 0); assert.equal(pool.pending, false);
  (next as ReturnType<VoiceAudioCapturePool["session"]> | null)?.close();
});

test("End between master adoption and actual SDK worker return retains the SAME pending capsule", async () => {
  const pool = new VoiceAudioCapturePool(), fees = ledger(), live = authority(), session = pool.session(fees.budget);
  let queued = false;
  const media = stream({ onEnable() {
    if (queued) return; queued = true;
    queueMicrotask(() => {
      session.close();
      assert.equal(fees.paid(), VOICE_AUDIO_CAPTURE_BYTES, "worker has not returned yet");
      assert.equal(media.stopped(), 0);
    });
  } });
  session.request(live, "", { async getUserMedia() { return media.actual; } }); await pool.settlement();
  assert.equal(session.available, false); assert.equal(media.stopped(), 1); assert.equal(fees.paid(), 0);
});

test("pending capture keeps only the original passive status and clock, never a mutable caller wrapper", async () => {
  const pool = new VoiceAudioCapturePool(), fees = ledger(), live = authority(), session = pool.session(fees.budget);
  const held = gate<MediaStream>(), entered = gate<void>();
  const wrapper = { status: live.status, clock: live.clock, get unexpected(): never { return assert.fail("unrelated graph must not be read"); } };
  session.request(wrapper, "", { getUserMedia() { entered.release(); return held.promise; } }); await entered.promise;
  wrapper.status = authority().status; live.status.close();
  const late = stream(); held.release(late.actual); await pool.settlement();
  assert.equal(session.available, false); assert.equal(late.stopped(), 1); assert.equal(fees.paid(), 0); session.close();
});

test("maximum-device permission debt returns after SDK/stop frames; refund reentry cannot revive the old session", async () => {
  const pool = new VoiceAudioCapturePool(), live = authority(), held = gate<MediaStream>(), entered = gate<void>();
  let sdkReturned = false, stopDepth = 0, next: ReturnType<VoiceAudioCapturePool["session"]> | null = null;
  const newFees = ledger(), fresh = stream();
  const oldFees = ledger(() => {}, () => {
    assert.equal(sdkReturned, true); assert.equal(stopDepth, 0);
    assert.equal(old.closed, true); assert.equal(old.available, false);
    assert.ok(next); assert.equal(next.request(live, "", { async getUserMedia() { return fresh.actual; } }), true);
  });
  const old = pool.session(oldFees.budget);
  const lateTrack = { kind: "audio", readyState: "live", enabled: true,
    stop() { stopDepth++; try { assert.equal(oldFees.paid(), VOICE_AUDIO_CAPTURE_BYTES); }
      finally { stopDepth--; } },
  };
  const late = { getTracks() { return [lateTrack]; } } as unknown as MediaStream;
  old.request(live, "x".repeat(1024), { async getUserMedia(input) {
    assert.equal((input!.audio as MediaTrackConstraints).deviceId &&
      ((input!.audio as MediaTrackConstraints).deviceId as ConstrainDOMStringParameters).exact?.length, 1024);
    entered.release(); const result = await held.promise; sdkReturned = true; return result;
  } });
  await entered.promise; old.close(); next = pool.session(newFees.budget);
  assert.equal(next.request(live, "", { async getUserMedia() { assert.fail(); } }), false);
  held.release(late); await pool.settlement(); await pool.settlement();
  assert.equal(oldFees.paid(), 0); assert.equal(next.available, true); assert.equal(lateTrack.enabled, false);
  next.close(); assert.equal(newFees.paid(), 0); assert.equal(fresh.stopped(), 1);
});
