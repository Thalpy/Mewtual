import { VOICE_AUDIO_FIXTURE_SDP, VOICE_AUDIO_FIXTURE_MAX_SDP } from "../scripts/voice-audio-sdp-fixture.ts";
import { test } from "node:test";
import assert from "node:assert/strict";
import { VoiceSdpRtcOwner, type VoiceSdpRtcClock } from "./voice-sdp-rtc.ts";
import { VoiceRtcConfigurationOwner, voiceRtcConfigurationCurrent,
  type VoiceRtcConfigurationLease } from "./voice-rtc-configuration.ts";
const gate = () => { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; };
async function turn() { await new Promise<void>(yes => setImmediate(yes)); }
function fixture(low = true, configuration: VoiceRtcConfigurationLease | null = null) {
  const old = globalThis.RTCPeerConnection;
  let owner!: VoiceSdpRtcOwner, now = 1000, live = true, complete = true;
  let localHeld: ReturnType<typeof gate> | null = null, remoteHeld: ReturnType<typeof gate> | null = null;
  let createHook: (() => void) | null = null, currentHook: (() => void) | null = null;
  let registrationHook: (() => void) | null = null;
  let closeHook: (() => void) | null = null;
  let remoteFails = false, local = VOICE_AUDIO_FIXTURE_SDP;
  let answerDirection = "recvonly", transceiverCount = 1, directionHook: (() => void) | null = null;
  const transceiver = { receiver: { track: { kind: "audio" } },
    get direction() { return answerDirection; }, set direction(value: string) { directionHook?.(); answerDirection = value; } };
  const calls = { created: 0, local: 0, remote: 0, closed: 0, audio: 0 };
  const listeners = new Set<() => void>(), timers = new Map<object, { run: () => void; deadline: number }>();
  let incoming: RTCSessionDescriptionInit | null = null;
  const configurations: RTCConfiguration[] = [];
  class Peer {
    constructor(configuration: RTCConfiguration) { configurations.push(configuration); calls.created++; createHook?.(); }
    get iceGatheringState() { return complete ? "complete" : "gathering"; }
    get localDescription() { return { type: low ? "offer" : "answer", sdp: local }; }
    addTransceiver() { calls.audio++; }
    getTransceivers() { return Array.from({ length: transceiverCount }, () => transceiver); }
    async setLocalDescription() { calls.local++; if (localHeld) await localHeld.promise; }
    async setRemoteDescription(description: RTCSessionDescriptionInit) {
      calls.remote++; incoming = description; if (remoteHeld) await remoteHeld.promise;
      if (remoteFails) throw new Error("RTC syntax rejected");
    }
    addEventListener(_kind: string, callback: () => void) { listeners.add(callback); registrationHook?.(); }
    removeEventListener(_kind: string, callback: () => void) { listeners.delete(callback); }
    close() { calls.closed++; closeHook?.(); }
  }
  globalThis.RTCPeerConnection = Peer as unknown as typeof RTCPeerConnection;
  const clock: VoiceSdpRtcClock = {
    now: () => now,
    schedule(run, milliseconds) { const handle = {}; timers.set(handle, { run, deadline: now + milliseconds }); return handle; },
    cancel(handle) { timers.delete(handle as object); },
  };
  owner = new VoiceSdpRtcOwner(low, () => { currentHook?.(); return live; }, clock, configuration);
  return { owner, clock, calls, configurations, timers, listeners, incoming: () => incoming,
    transceiver, direction: () => answerDirection, transceivers(count: number) { transceiverCount = count; },
    onDirection(run: () => void) { directionHook = run; },
    restore() { globalThis.RTCPeerConnection = old; }, local(text: string) { local = text; }, time(value: number) { now = value; },
    onRegistration(run: () => void) { registrationHook = run; },
    onClose(run: () => void) { closeHook = run; },
    stop() { live = false; owner.close(); }, gathering() { complete = false; },
    gathered() { complete = true; for (const callback of [...listeners]) callback(); },
    holdLocal() { localHeld = gate(); return localHeld; }, holdRemote() { remoteHeld = gate(); return remoteHeld; },
    failRemote() { remoteFails = true; }, onCreate(run: () => void) { createHook = run; }, onCurrent(run: () => void) { currentHook = run; },
  };
}

function configured() {
  const owner = new VoiceRtcConfigurationOwner();
  owner.capture(() => ({ stun: "stun:device.example.test", turn: "turns:relay.example.test?transport=tcp",
    username: "device-user", credential: "device-secret", policy: "relay" }), () => true);
  return owner;
}

test("HIGH answers with the same audio transceiver prepared for both negotiated directions", async t => {
  const f = fixture(false); t.after(f.restore);
  await f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "offer");
  assert.equal(f.direction(), "recvonly");
  await f.owner.prepareLocal(async () => 30_000, "answer");
  assert.equal(f.direction(), "sendrecv"); assert.equal(f.calls.audio, 0); assert.equal(f.calls.local, 1);
  f.owner.takeLocal().fill(0); await f.owner.drain();
});

for (const invalid of ["none", "extra", "video"] as const) test(`HIGH refuses unexpected actual SDK media before answering: ${invalid}`, async t => {
  const f = fixture(false); t.after(f.restore);
  await f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "offer");
  if (invalid === "video") f.transceiver.receiver.track.kind = "video";
  else f.transceivers(invalid === "none" ? 0 : 2);
  await assert.rejects(f.owner.prepareLocal(async () => 30_000, "answer"));
  assert.equal(f.calls.local, 0); assert.equal(f.calls.closed, 1); await f.owner.drain();
});

test("End in the HIGH direction setter closes the actual peer before description submission", async t => {
  const f = fixture(false); t.after(f.restore);
  await f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "offer");
  f.onDirection(() => f.stop());
  await assert.rejects(f.owner.prepareLocal(async () => 30_000, "answer"));
  assert.equal(f.calls.local, 0); assert.equal(f.calls.closed, 1); await f.owner.drain();
});

for (const boundary of ["transceivers", "receiver-kind", "native14"] as const) test(`HIGH refuses before direction mutation at ${boundary}`, async t => {
  const f = fixture(false); t.after(f.restore);
  await f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "offer");
  if (boundary === "transceivers") {
    const prototype = globalThis.RTCPeerConnection.prototype, read = prototype.getTransceivers;
    Object.defineProperty(prototype, "getTransceivers", { get() { f.stop(); return read; } });
  } else if (boundary === "receiver-kind") {
    Object.defineProperty(f.transceiver.receiver.track, "kind", { get() { f.stop(); return "audio"; } });
  }
  await assert.rejects(f.owner.prepareLocal(async () => { if (boundary === "native14") throw new Error("native admission refused"); return 30_000; }, "answer"));
  assert.equal(f.direction(), "recvonly"); assert.equal(f.calls.local, 0); await f.owner.drain();
});

test("configured private constructor still waits for native14 and retains the loan through held admission End", async t => {
  const settings = configured(), lease = settings.borrow(), f = fixture(true, lease); t.after(f.restore);
  const held = gate(), work = f.owner.prepareLocal(async () => { await held.promise; return 30_000; }, "offer");
  await turn(); assert.equal(f.calls.created, 0); f.stop();
  assert.throws(() => settings.reset()); let joined = false;
  const cleanup = f.owner.drain().then(() => { joined = true; });
  await turn(); assert.equal(joined, false); held.release(); await work.catch(() => {}); await cleanup;
  settings.reset(); assert.equal(f.calls.created, 0);
});

test("End inside the configured constructor closes its actual returned peer before returning configuration custody", async t => {
  const settings = configured(), f = fixture(true, settings.borrow()); t.after(f.restore);
  f.onCreate(() => { f.stop(); assert.throws(() => settings.reset()); });
  await assert.rejects(f.owner.prepareLocal(async () => 30_000, "offer"));
  assert.equal(f.calls.created, 1); assert.equal(f.calls.closed, 1); assert.equal(f.calls.local, 0);
  assert.deepEqual(f.configurations[0].iceServers, [{ urls: "turns:relay.example.test?transport=tcp", username: "device-user", credential: "device-secret" }]);
  await f.owner.drain(); settings.reset();
});

test("the SAME configuration lease transfers with the PC and failed media SDK close retains it", async t => {
  const settings = configured(), f = fixture(true, settings.borrow()); t.after(f.restore);
  await f.owner.prepareLocal(async () => 30_000, "offer"); f.owner.takeLocal().fill(0);
  await f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "answer");
  const peer = f.owner.prepareMediaDestination(); f.owner.transferInto(peer);
  assert.equal(peer.adopted, true); await f.owner.drain();
  let fail = true; f.onClose(() => { if (fail) throw new Error("held SDK close"); });
  assert.throws(() => peer.close(), /held SDK close/); assert.equal(peer.drained, false);
  assert.throws(() => settings.reset());
  fail = false; await peer.drain(); assert.equal(peer.drained, true); settings.reset();
});

test("one original peer failure returns only its configuration loan while the sibling remains usable", async t => {
  const settings = configured(), first = settings.borrow(), second = settings.borrow();
  const f = fixture(true, first); t.after(f.restore);
  const sibling = new VoiceSdpRtcOwner(true, () => true, f.clock, second);
  await f.owner.prepareLocal(async () => 30_000, "offer"); f.owner.takeLocal().fill(0);
  await sibling.prepareLocal(async () => 30_000, "offer"); sibling.takeLocal().fill(0);
  f.stop(); await f.owner.drain();
  assert.equal(voiceRtcConfigurationCurrent(first), false); assert.equal(voiceRtcConfigurationCurrent(second), true);
  await sibling.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "answer");
  const peer = sibling.prepareMediaDestination(); sibling.transferInto(peer); assert.equal(peer.adopted, true);
  await sibling.drain(); await peer.drain(); settings.reset();
  assert.equal(f.calls.created, 2); assert.equal(f.calls.closed, 2);
});

test("original RTC local preparation awaits fresh admission and real gathering; observers carry no description", async t => {
  const f = fixture(); t.after(f.restore); f.gathering(); f.local(VOICE_AUDIO_FIXTURE_MAX_SDP); const admitted = gate();
  const work = f.owner.prepareLocal(async () => { await admitted.promise; return 30_000; }, "offer");
  assert.throws(() => f.owner.takeLocal()); await turn(); assert.equal(f.calls.created, 0);
  admitted.release(); await turn(); assert.equal(f.calls.local, 1); assert.equal(f.calls.audio, 1);
  assert.throws(() => f.owner.takeLocal()); f.gathered();
  assert.equal(await work, undefined); const bytes = f.owner.takeLocal();
  assert.equal(bytes.length, 8192); assert.equal(new TextDecoder().decode(bytes), VOICE_AUDIO_FIXTURE_MAX_SDP);
  assert.throws(() => f.owner.takeLocal()); bytes.fill(0);
  assert.equal(f.listeners.size, 0); assert.equal(f.timers.size, 0); await f.owner.drain(); assert.equal(f.calls.closed, 1);
});

test("a failed pre-transfer SDK close retains the real peer until explicit cleanup succeeds", async t => {
  const f = fixture(false); t.after(f.restore); const held = f.holdRemote();
  const work = f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "offer"); await turn();
  let fails = true; f.onClose(() => { if (fails) throw new Error("SDK close failed"); });
  f.stop(); let completed = false;
  const closing = f.owner.drain().then(() => { completed = true; });
  const rejected = assert.rejects(closing, /close still running/);
  await turn(); assert.equal(completed, false); assert.equal(f.calls.created, 1);
  held.release(); await work; await rejected;
  assert.equal(completed, false); assert.throws(() => f.owner.prepareMediaDestination());
  fails = false; const before = f.calls.closed; await f.owner.drain();
  assert.equal(f.calls.closed, before + 1); await f.owner.drain(); assert.equal(f.calls.closed, before + 1);
});

test("pre-transfer SDK close reentry cannot report drainage from the entered close callback", async t => {
  const f = fixture(); t.after(f.restore); await f.owner.prepareLocal(async () => 30_000, "offer");
  f.owner.takeLocal().fill(0); let nested: Promise<void> | null = null;
  f.onClose(() => { nested = f.owner.drain(); });
  f.owner.close(); await assert.rejects(nested!, /close still running/);
  await f.owner.drain(); assert.equal(f.calls.closed, 1);
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
  const f = fixture(false); t.after(f.restore); const held = f.holdRemote(); const bytes = new TextEncoder().encode(VOICE_AUDIO_FIXTURE_MAX_SDP);
  const work = f.owner.applyRemote(bytes, "offer"); await turn(); assert.equal(f.calls.remote, 1); assert.equal(f.incoming()?.sdp?.length, 8192);
  f.stop(); let drained = false; const close = f.owner.drain().then(() => { drained = true; }); await turn();
  assert.equal(drained, false); assert.equal(f.calls.closed, 1); assert.equal(f.incoming()?.sdp?.length, 8192);
  assert.throws(() => f.owner.applyRemote(bytes, "offer")); held.release(); assert.equal(await work, undefined); await close;
  assert.equal(f.incoming()?.sdp, ""); assert.equal(f.owner.applied, true, "actual SDK success is separate from current native Applied permission");
  assert.equal(Object.hasOwn(f.owner, "peer"), false, "actual peer custody is runtime-private");
  assert.throws(() => f.owner.prepareMediaDestination(), "drained SDK owner cannot revive");
});

test("RTC parser rejection never becomes AEAD failure or invented Applied", async t => {
  const f = fixture(false); t.after(f.restore); f.failRemote();
  await assert.rejects(f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "offer"), /RTC syntax rejected/);
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
  await assert.rejects(f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "offer"));
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

// These are Source-boundary regressions, not just parser unit tests: a remote offer
// must fail before even HIGH's original SDK constructor can allocate receivers/SCTP.
for (const [name, invalid] of [
  ["empty", ""],
  ["extra audio", VOICE_AUDIO_FIXTURE_SDP + "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"],
  ["video", VOICE_AUDIO_FIXTURE_SDP + "m=video 9 UDP/TLS/RTP/SAVPF 96\r\n"],
  ["SCTP", VOICE_AUDIO_FIXTURE_SDP + "m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n"],
  ["rejected SCTP", VOICE_AUDIO_FIXTURE_SDP + "m=application 0 UDP/DTLS/SCTP webrtc-datachannel\r\n"],
  ["folded media", VOICE_AUDIO_FIXTURE_SDP + " m=video 9 UDP/TLS/RTP/SAVPF 96\r\n"],
  ["missing MID", VOICE_AUDIO_FIXTURE_SDP.replace("a=mid:0\r\n", "")],
  ["unsupported RTP", VOICE_AUDIO_FIXTURE_SDP.replace("UDP/TLS/RTP/SAVPF", "TCP/DTLS/RTP/SAVPF")],
] as const) test(`HIGH rejects ${name} before original PC construction`, async t => {
  const f = fixture(false); t.after(f.restore);
  await assert.rejects(f.owner.applyRemote(new TextEncoder().encode(invalid), "offer"));
  assert.equal(f.calls.created, 0); assert.equal(f.calls.remote, 0); assert.equal(f.owner.applied, false);
  await f.owner.drain(); assert.equal(f.calls.closed, 0);
});

for (const low of [true, false]) test(`original ${low ? "LOW remote" : "HIGH local"} answer cannot replace the offer MID`, async t => {
  const f = fixture(low); t.after(f.restore);
  const different = VOICE_AUDIO_FIXTURE_SDP.replace("BUNDLE 0", "BUNDLE 1").replace("mid:0", "mid:1");
  if (low) {
    await f.owner.prepareLocal(async () => 30_000, "offer"); f.owner.takeLocal().fill(0);
    await assert.rejects(f.owner.applyRemote(new TextEncoder().encode(different), "answer"));
    assert.equal(f.calls.remote, 0); assert.equal(f.owner.applied, false);
  } else {
    await f.owner.applyRemote(new TextEncoder().encode(VOICE_AUDIO_FIXTURE_SDP), "offer");
    f.local(different); await assert.rejects(f.owner.prepareLocal(async () => 30_000, "answer"));
    assert.throws(() => f.owner.takeLocal());
  }
  assert.equal(f.calls.closed, 1); assert.equal(Object.hasOwn(f.owner, "offerShape"), false);
  await f.owner.drain();
});

test("gathered local extra media is rejected before plaintext can leave the Source", async t => {
  const f = fixture(); t.after(f.restore);
  f.local(VOICE_AUDIO_FIXTURE_SDP + "m=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\n");
  await assert.rejects(f.owner.prepareLocal(async () => 30_000, "offer"));
  assert.throws(() => f.owner.takeLocal()); assert.equal(f.calls.closed, 1); await f.owner.drain();
});

test("maximum supported remote offer keeps the actual byte bound before HIGH construction", async t => {
  const f = fixture(false); t.after(f.restore);
  const bytes = new TextEncoder().encode(VOICE_AUDIO_FIXTURE_MAX_SDP);
  assert.equal(bytes.byteLength, 8192);
  await f.owner.applyRemote(bytes, "offer"); assert.equal(f.calls.created, 1); assert.equal(f.calls.remote, 1);
  assert.equal(f.incoming()?.sdp, "", "actual SDK text wrapper is cleared after return");
  await f.owner.prepareLocal(async () => 30_000, "answer"); f.owner.takeLocal().fill(0); await f.owner.drain();
});

test("oversized remote bytes cannot enter the decoder, constructor or SDK", async t => {
  const f = fixture(false); t.after(f.restore);
  assert.throws(() => f.owner.applyRemote(new Uint8Array(8193), "offer"));
  assert.equal(f.calls.created, 0); assert.equal(f.calls.remote, 0); await f.owner.drain();
});

function maximumMidDescription(letter = "m"): string {
  const mid = letter.repeat(64), prefix = VOICE_AUDIO_FIXTURE_SDP.replace("BUNDLE 0", "BUNDLE " + mid).replace("mid:0", "mid:" + mid);
  const attribute = "a=x-fixture:";
  return prefix + attribute + "s".repeat(8192 - prefix.length - attribute.length - 2) + "\r\n";
}
for (const low of [true, false]) test(`MID64 in an actual8192-byte ${low ? "LOW" : "HIGH"} round survives transfer with no cached SDP text`, async t => {
  const f = fixture(low); t.after(f.restore); const input = maximumMidDescription();
  assert.equal(new TextEncoder().encode(input).byteLength, 8192); f.local(input);
  if (low) {
    await f.owner.prepareLocal(async () => 30_000, "offer");
    const bytes = f.owner.takeLocal(); assert.equal(new TextDecoder().decode(bytes), input); bytes.fill(0);
    await f.owner.applyRemote(new TextEncoder().encode(input), "answer");
  } else {
    await f.owner.applyRemote(new TextEncoder().encode(input), "offer");
    await f.owner.prepareLocal(async () => 30_000, "answer"); f.owner.takeLocal().fill(0);
  }
  const destination = f.owner.prepareMediaDestination(); f.owner.transferInto(destination);
  await f.owner.drain(); assert.equal(f.calls.closed, 0); assert.equal(destination.adopted, true);
  await destination.drain(); assert.equal(f.calls.closed, 1);
});
for (const low of [true, false]) test(`original MID64 rejects different8192-byte ${low ? "remote" : "local"} answer and closes once`, async t => {
  const f = fixture(low); t.after(f.restore); const input = maximumMidDescription(), changed = maximumMidDescription("n");
  f.local(input);
  if (low) {
    await f.owner.prepareLocal(async () => 30_000, "offer"); f.owner.takeLocal().fill(0);
    await assert.rejects(f.owner.applyRemote(new TextEncoder().encode(changed), "answer"));
    assert.equal(f.calls.remote, 0);
  } else {
    await f.owner.applyRemote(new TextEncoder().encode(input), "offer"); f.local(changed);
    await assert.rejects(f.owner.prepareLocal(async () => 30_000, "answer"));
    assert.throws(() => f.owner.takeLocal());
  }
  await f.owner.drain(); assert.equal(f.calls.closed, 1); assert.throws(() => f.owner.prepareMediaDestination());
});
test("End during bounded MID materialization still prevents HIGH construction", async t => {
  const f = fixture(false); t.after(f.restore); const actual = String.fromCharCode;
  try {
    String.fromCharCode = (...codes: number[]) => { f.stop(); return actual(...codes); };
    await assert.rejects(f.owner.applyRemote(new TextEncoder().encode(maximumMidDescription()), "offer"));
    assert.equal(f.calls.created, 0); assert.equal(f.calls.remote, 0); await f.owner.drain();
  } finally { String.fromCharCode = actual; }
});
