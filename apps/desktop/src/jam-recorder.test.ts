import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { JAM_SESSION_NONCE_HEX_CHARS, JAM_TAKE_COMMITMENT_DOMAIN, TAKE_ID_MAX_BYTES, TAKE_MAX_BYTES, type JamPatch } from "./jam-contract.ts";
import {
  applyJamRecorderConsent,
  captureJamRecorderLease,
  jamRecorderTimelineMs,
  jamParticipantCommitment,
  jamTakeId,
  JamTakeRecorder,
  parseJamTakeJson,
  recordLeasedJamDrum,
  recordLeasedJamNoteOff,
  recordLeasedJamNoteOn,
  startJamRecorderTimeline,
  validateJamTake,
} from "./jam-recorder.ts";

const alice = "alice-fingerprint";
const bob = "bob-fingerprint";
const aliceSn = "a".repeat(JAM_SESSION_NONCE_HEX_CHARS);
const bobSn = "b".repeat(JAM_SESSION_NONCE_HEX_CHARS);
const patch: JamPatch = {
  v: 1,
  o: [{ w: 3, t: 0, c: 0, l: 100 }],
  e: { a: 400, d: 800, s: 65, r: 2_000 },
  f: { m: 0, c: 2_200, q: 25, e: 20 },
  l: { r: 20, d: 10, t: 1 },
  x: { c: 12, d: 8, r: 30 },
};

function recorder() {
  return new JamTakeRecorder({ groupId: "group-1", callId: "general", bpm: 120, beatsPerBar: 4, participants: [alice, bob] });
}

function start(rec: JamTakeRecorder) {
  assert.equal(rec.setConsent(alice, true), true);
  assert.equal(rec.setConsent(bob, true), true);
  assert.equal(rec.start(), true);
}

test("recording starts only with every participant's honest-client consent", () => {
  const rec = recorder();
  assert.equal(rec.start(), false);
  rec.setConsent(alice, true);
  assert.equal(rec.start(), false);
  rec.setConsent(bob, true);
  assert.equal(rec.start(), true);
  rec.setConsent(bob, false);
  assert.equal(rec.state(), "arming");
});

test("local consent withdrawal immediately closes the recorder event gate", () => {
  const rec = recorder();
  start(rec);
  assert.equal(applyJamRecorderConsent(rec, alice, false), true);
  assert.equal(rec.state(), "arming");
  assert.deepEqual(
    rec.recordNoteOn({ source: alice, sessionNonce: aliceSn, ms: 1, sequence: 1, note: 60, wave: "sine" }),
    { ok: false, reason: "not-recording" },
  );
  assert.equal(applyJamRecorderConsent(rec, alice, true), true);
  assert.equal(rec.state(), "arming", "re-allow still requires the normal sync/start transition");
  assert.equal(rec.start(), true);
});

test("an async drum keeps receipt time and cannot cross into a replacement take", () => {
  const old = recorder();
  start(old);
  const lease = captureJamRecorderLease(old, 7.4);
  const replacement = recorder();
  start(replacement);
  assert.equal(recordLeasedJamDrum(replacement, lease, {
    source: alice, sessionNonce: aliceSn, sequence: 1, pad: 0,
  }), null);
  assert.equal(replacement.stop().events.length, 0);

  const result = recordLeasedJamDrum(old, lease, {
    source: alice, sessionNonce: aliceSn, sequence: 1, pad: 0,
  });
  assert.equal(result?.ok, true);
  if (result?.ok) assert.equal(result.event.ms, 7);
});

test("an event received before consent cannot cross the recorder start boundary", () => {
  const rec = recorder();
  const preConsentLease = captureJamRecorderLease(rec, 7.4);
  assert.equal(preConsentLease, null);
  start(rec);
  assert.equal(recordLeasedJamDrum(rec, preConsentLease, {
    source: alice, sessionNonce: aliceSn, sequence: 1, pad: 0,
  }), null);
  assert.equal(rec.snapshot().events.length, 0);
});

test("receipt leases cannot cross consent withdrawal and restart on the same recorder", () => {
  const rec = recorder();
  start(rec);
  const beforeWithdrawal = captureJamRecorderLease(rec, 10);
  rec.setConsent(bob, false);
  const duringWithdrawal = captureJamRecorderLease(rec, 20);
  rec.setConsent(bob, true);
  assert.equal(rec.start(), true);

  assert.equal(recordLeasedJamNoteOn(rec, beforeWithdrawal, {
    source: alice, sessionNonce: aliceSn, sequence: 1, note: 60, wave: "sine",
  }), null);
  assert.equal(recordLeasedJamDrum(rec, duringWithdrawal, {
    source: alice, sessionNonce: aliceSn, sequence: 2, pad: 0,
  }), null);
  assert.equal(rec.snapshot().events.length, 0);

  const afterRestart = captureJamRecorderLease(rec, 30);
  assert.equal(recordLeasedJamNoteOn(rec, afterRestart, {
    source: alice, sessionNonce: aliceSn, sequence: 3, note: 60, wave: "sine",
  })?.ok, true);
  assert.equal(recordLeasedJamNoteOff(rec, afterRestart, {
    source: alice, sessionNonce: aliceSn, sequence: 4, note: 60,
  })?.ok, true);
  assert.deepEqual(rec.snapshot().events.map((event) => event.ms), [30, 30]);
});

test("remote musical frames capture gate and recorder admission before the outer causal queue", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("function handleJamFrame(");
  const process = source.indexOf("async function processJamFrame(", start);
  assert.ok(start >= 0 && process > start);
  const admission = source.indexOf("const admission: JamQueuedAdmission", start);
  const enqueue = source.indexOf("queue.enqueue(() => processJamFrame", start);
  assert.ok(admission >= 0 && admission < enqueue && enqueue < process,
    "receipt authority must be captured before an async queue can cross a gate change");
  const body = source.slice(process, source.indexOf("function jamBroadcastFrame", process));
  assert.match(body, /admission\.roomGeneration === jamRoomRenderGeneration/);
  assert.match(body, /recordLeasedJamNoteOn\(jamRec, admission\.recorderLease/);
  assert.match(body, /recordLeasedJamNoteOff\(jamRec, admission\.recorderLease/);
  assert.match(body, /recordLeasedJamDrum\(jamRec, admission\.recorderLease/);
});

test("a disconnected participant needs fresh consent before recording resumes", () => {
  const rec = recorder();
  start(rec);

  assert.equal(applyJamRecorderConsent(rec, bob, false), true);
  rec.membershipChanged([alice]);
  rec.membershipChanged([alice, bob]);
  assert.equal(rec.state(), "arming");
  assert.equal(rec.start(), false, "restoring membership cannot revive the disconnected edge's consent");

  assert.equal(applyJamRecorderConsent(rec, bob, true), true);
  assert.equal(rec.start(), true, "a fresh consent announcement admits the replacement edge");
});

test("consent pause and resume retain a monotonic recorder timeline", () => {
  const rec = recorder();
  start(rec);
  let timeline = startJamRecorderTimeline({ startMs: null }, 1_000);
  assert.equal(rec.recordDrum({
    source: alice, sessionNonce: aliceSn, ms: jamRecorderTimelineMs(timeline, 6_000), sequence: 1, pad: 0,
  }).ok, true);

  rec.setConsent(bob, false);
  rec.setConsent(bob, true);
  assert.equal(rec.start(), true);
  timeline = startJamRecorderTimeline(timeline, 6_100);
  assert.equal(timeline.startMs, 1_000, "resume must not reset the original recorder epoch");
  assert.equal(rec.recordDrum({
    source: alice, sessionNonce: aliceSn, ms: jamRecorderTimelineMs(timeline, 6_100), sequence: 2, pad: 1,
  }).ok, true, "the first post-resume event remains monotonic and is admitted");
});

test("take identities are independently byte-bounded before per-event drum hashing", () => {
  const tooLong = "x".repeat(TAKE_ID_MAX_BYTES + 1);
  assert.throws(
    () => new JamTakeRecorder({ groupId: "g", callId: tooLong, bpm: 120, beatsPerBar: 4, participants: [alice] }),
    TypeError,
  );
  const take = recorder().stop();
  assert.equal(validateJamTake({ ...take, call: tooLong }).ok, false);
  assert.equal(validateJamTake({ ...take, parts: [tooLong, bob] }).ok, false);
});

test("authenticated source/session lanes retain patches, fallback waves, drums and gaps", () => {
  const rec = recorder();
  start(rec);
  const on = rec.recordNoteOn({
    source: alice,
    sessionNonce: aliceSn,
    ms: 0,
    sequence: 10,
    note: 60,
    wave: "triangle",
    patch,
  });
  assert.equal(on.ok, true);
  const off = rec.recordNoteOff({ source: alice, sessionNonce: aliceSn, ms: 500, sequence: 12, note: 60 });
  assert.deepEqual(off.ok && off.gap, { from: 11, to: 11 });
  const drum = rec.recordDrum({ source: bob, sessionNonce: bobSn, ms: 250, sequence: 1, pad: 4 });
  assert.equal(drum.ok, true);

  const take = rec.stop();
  assert.deepEqual(take.parts, [alice, bob]);
  assert.deepEqual(take.lanes, [{ src: 0, sn: aliceSn }, { src: 1, sn: bobSn }]);
  assert.equal(take.patches.length, 1);
  assert.deepEqual(take.events.map((event) => event.ms), [0, 250, 500], "grid time, not arrival grouping, orders playback");
  assert.deepEqual(take.events[0], { ms: 0, lane: 0, n: 60, on: 1, w: "triangle", p: 0, q: 10 });
  assert.deepEqual(take.events[1], { ms: 250, lane: 1, n: 4, d: 1, q: 1 });
});

test("a sender cannot allocate lanes or patches with rejected events", () => {
  const rec = recorder();
  start(rec);
  assert.equal(rec.recordNoteOn({
    source: alice,
    sessionNonce: aliceSn,
    ms: 10,
    sequence: 2,
    note: 60,
    wave: "sine",
  }).ok, true);
  const before = rec.snapshot();
  const duplicateWithNewPatch = rec.recordNoteOn({
    source: alice,
    sessionNonce: aliceSn,
    ms: 20,
    sequence: 2,
    note: 61,
    wave: "sine",
    patch,
  });
  assert.deepEqual(duplicateWithNewPatch, { ok: false, reason: "duplicate" });
  assert.equal(rec.snapshot().patches.length, before.patches.length);

  const backwardNewLane = rec.recordDrum({
    source: bob,
    sessionNonce: bobSn,
    ms: -1,
    sequence: 1,
    pad: 0,
  });
  assert.deepEqual(backwardNewLane, { ok: false, reason: "invalid" });
  assert.equal(rec.snapshot().lanes.length, before.lanes.length);
});

test("membership changes pause an active take but do not auto-start an armed one", () => {
  const armed = recorder();
  armed.membershipChanged([alice, bob]);
  assert.equal(armed.state(), "arming");

  armed.membershipChanged([alice, bob, "carol"]);
  armed.setConsent(alice, true);
  armed.setConsent(bob, true);
  assert.equal(armed.state(), "paused-membership");
  assert.equal(armed.start(), false, "a join before consent must prevent recording the stale set");
  armed.membershipChanged([alice, bob]);
  assert.equal(armed.state(), "arming", "restoring an armed set still requires an explicit start");

  const active = recorder();
  start(active);
  active.membershipChanged([alice, bob, "newcomer"]);
  assert.equal(active.state(), "paused-membership");
  assert.deepEqual(active.recordDrum({ source: alice, sessionNonce: aliceSn, ms: 0, sequence: 1, pad: 0 }), {
    ok: false,
    reason: "not-recording",
  });
  active.membershipChanged([bob, alice]);
  assert.equal(active.state(), "recording");
  active.membershipChanged([alice, alice]);
  assert.equal(active.state(), "paused-membership", "a duplicate must not impersonate the exact participant set");
});

test("the recorder owns an immutable copy of its validated header", () => {
  const participants = [alice, bob];
  const config = { groupId: "group-1", callId: "general", bpm: 120, beatsPerBar: 4, participants };
  const rec = new JamTakeRecorder(config);
  participants.push("mallory");
  config.callId = "other-call";
  config.bpm = 240;
  assert.deepEqual(rec.snapshot().parts, [alice, bob]);
  assert.equal(rec.snapshot().call, "general");
  assert.equal(rec.snapshot().group, "group-1");
  assert.equal(rec.snapshot().met.bpm, 120);
});

test("take validation rejects parser tricks, unknown fields and dishonest lane order", () => {
  const circular: Record<string, unknown> = {};
  circular.self = circular;
  assert.equal(validateJamTake(circular).ok, false);

  const rec = recorder();
  start(rec);
  rec.recordNoteOn({ source: alice, sessionNonce: aliceSn, ms: 10, sequence: 1, note: 60, wave: "sine" });
  rec.recordNoteOff({ source: alice, sessionNonce: aliceSn, ms: 20, sequence: 2, note: 60 });
  const take = rec.stop();
  assert.equal(validateJamTake(take).ok, true);
  assert.equal(validateJamTake({ ...take, html: "<audio autoplay>" }).ok, false);
  assert.equal(validateJamTake({ ...take, events: [take.events[1], take.events[0]] }).ok, false);
  assert.equal(validateJamTake({ ...take, lanes: [...take.lanes, take.lanes[0]] }).ok, false);

  const imported = structuredClone(take);
  const validated = validateJamTake(imported);
  assert.equal(validated.ok, true);
  imported.events[0].n = 127;
  assert.equal(validated.ok && validated.take.events[0].n, 60, "validated events must not retain caller-owned objects");

  const inherited = Object.create(take) as typeof take;
  assert.equal(validateJamTake(inherited).ok, false);
  let getterReads = 0;
  const accessor = { ...take } as Record<string, unknown>;
  Object.defineProperty(accessor, "events", { enumerable: true, get: () => { getterReads += 1; return take.events; } });
  assert.equal(validateJamTake(accessor).ok, false);
  assert.equal(getterReads, 0);
  assert.equal(parseJamTakeJson("x".repeat(TAKE_MAX_BYTES + 1)).ok, false);
  assert.equal(parseJamTakeJson(JSON.stringify(take)).ok, true);
});

test("participant commitments bind group, take, call, device and every reconnect lane", async () => {
  const rec = recorder();
  start(rec);
  rec.recordDrum({ source: alice, sessionNonce: aliceSn, ms: 0, sequence: 1, pad: 0 });
  rec.recordDrum({ source: alice, sessionNonce: "c".repeat(JAM_SESSION_NONCE_HEX_CHARS), ms: 5, sequence: 1, pad: 1 });
  const take = rec.stop();
  const id = await jamTakeId(take);
  const commitment = await jamParticipantCommitment(id, take, 0);
  assert.equal(commitment.domain, JAM_TAKE_COMMITMENT_DOMAIN);
  assert.equal(commitment.takeId, id);
  assert.equal(commitment.groupId, "group-1");
  assert.equal(commitment.callId, "general");
  assert.equal(commitment.device, alice);
  assert.match(commitment.laneEventLogHash, /^[0-9a-f]{64}$/);
  await assert.rejects(() => jamParticipantCommitment("0".repeat(64), take, 0), /take id does not match/);
});
