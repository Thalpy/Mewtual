import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_BUFFERED_ICE_PER_PEER,
  VIDEO_BITRATE,
  bufferIce,
  directionIdle,
  directionSending,
  heartbeatRecovery,
  isCurrentVoiceRoom,
  mergePeerState,
  videoSlotPlan,
  type SlotDirection,
} from "./voice-signaling.ts";

test("voice signals are scoped by server as well as channel", () => {
  assert.equal(isCurrentVoiceRoom(7, "general", 7, "general"), true);
  assert.equal(isCurrentVoiceRoom(7, "general", 8, "general"), false);
  assert.equal(isCurrentVoiceRoom(7, "general", 7, "random"), false);
  assert.equal(isCurrentVoiceRoom(null, "", 7, "general"), false);
});

test("a heartbeat repairs a missed one-shot hello", () => {
  assert.equal(heartbeatRecovery({ currentRoom: true, hasPeer: false }), "create");
  assert.equal(heartbeatRecovery({ currentRoom: false, hasPeer: false }), null);
});

test("heartbeats retry SDP that can otherwise be lost forever", () => {
  assert.equal(
    heartbeatRecovery({
      currentRoom: true,
      hasPeer: true,
      connectionState: "new",
      signalingState: "have-local-offer",
      localDescriptionType: "offer",
    }),
    "resend-offer",
  );
  assert.equal(
    heartbeatRecovery({
      currentRoom: true,
      hasPeer: true,
      connectionState: "connecting",
      signalingState: "stable",
      localDescriptionType: "answer",
    }),
    "resend-answer",
  );
  assert.equal(
    heartbeatRecovery({
      currentRoom: true,
      hasPeer: true,
      connectionState: "failed",
      signalingState: "stable",
      localDescriptionType: "offer",
    }),
    "restart-ice",
  );
  assert.equal(
    heartbeatRecovery({
      currentRoom: true,
      hasPeer: true,
      connectionState: "connected",
      signalingState: "stable",
      localDescriptionType: "answer",
    }),
    null,
  );
});

test("a heartbeat that omits vid does not retract a live screen share", () => {
  // The share is announced once, on the data channel, and then five-second pings carry only the
  // mute states. Folding one of those in used to zero vid, which tore the picture down on every
  // viewer seconds after it started while the sender's own preview kept playing.
  const sharing = mergePeerState(undefined, { t: "s", mic: 0, inst: 0, vid: 2 });
  assert.deepEqual(sharing, { mic: false, inst: false, vid: 2, rx: 1080 });

  const afterPing = mergePeerState(sharing, { mic: 1, inst: 0 });
  assert.equal(afterPing.vid, 2, "an absent vid is not a claim that the video stopped");
  assert.equal(afterPing.mic, true, "the states the ping does carry still apply");
});

test("receiver resolution is coarse, validated and retained across partial heartbeats", () => {
  const first = mergePeerState(undefined, { mic: 0, inst: 0, rx: 2160 });
  assert.equal(first.rx, 2160);
  assert.equal(mergePeerState(first, { mic: 1 }).rx, 2160);
  assert.equal(mergePeerState(first, { mic: 1, rx: 901 }).rx, 2160);
  assert.equal(mergePeerState(undefined, { mic: 0, rx: "2160" }).rx, 1080);
});

test("stopping the video is believed, and nothing is invented for a peer we never heard from", () => {
  const sharing = mergePeerState(undefined, { mic: 0, inst: 0, vid: 2 });
  assert.equal(mergePeerState(sharing, { mic: 0, inst: 0, vid: 0 }).vid, 0);
  assert.equal(mergePeerState(sharing, { mic: 0, inst: 0, vid: 1 }).vid, 1); // screen -> camera
  assert.equal(mergePeerState(undefined, { mic: 0, inst: 0 }).vid, 0);
  assert.equal(mergePeerState(sharing, { mic: 0, inst: 0, vid: "2" }).vid, 2); // not a number: not a claim
});

test("parking a video slot drops the send half and leaves the receive half alone", () => {
  assert.equal(directionIdle("sendrecv"), "recvonly"); // still watching them, no longer sending
  assert.equal(directionIdle("sendonly"), "inactive"); // nothing either way
  assert.equal(directionIdle("recvonly"), null, "already not sending: nothing to change");
  assert.equal(directionIdle("inactive"), null);
  assert.equal(directionIdle("stopped"), null, "assigning a direction to a stopped one throws");
});

test("reviving a parked video slot restores exactly the half that was dropped", () => {
  assert.equal(directionSending("recvonly"), "sendrecv");
  assert.equal(directionSending("inactive"), "sendonly");
  assert.equal(directionSending("sendrecv"), null, "already sending: a cam/screen swap is silent");
  assert.equal(directionSending("sendonly"), null);
  assert.equal(directionSending("stopped"), null);
});

test("a stop/start cycle returns a video slot to where it began", () => {
  // The round trip is the whole point: if it did not land back on itself, the slot would drift a
  // direction per toggle and eventually stop carrying video without anything reporting a failure.
  for (const start of ["sendrecv", "sendonly"] as SlotDirection[]) {
    const parked = directionIdle(start);
    assert.ok(parked, `${start} should park`);
    assert.equal(directionSending(parked), start, `${start} did not survive the round trip`);
  }
});

test("a video slot is opened once per peer and reused for the rest of the call", () => {
  // The first video on an edge: nothing to reuse, so a transceiver gets added and negotiated.
  assert.deepEqual(
    videoSlotPlan({ hasSender: false, direction: "stopped" }),
    { action: "add", direction: null },
  );
  // Swapping camera for screen share on a live slot: same m-line, and no renegotiation at all.
  assert.deepEqual(
    videoSlotPlan({ hasSender: true, direction: "sendrecv" }),
    { action: "reuse", direction: null },
  );
  // The regression: a slot parked by a previous stop is revived, NOT abandoned for a second one.
  // Stopping used to removeTrack, which retires the transceiver, so every stop/start bolted
  // another video section onto the SDP and left a sender no "has a video track" test could find.
  assert.deepEqual(
    videoSlotPlan({ hasSender: true, direction: "recvonly" }),
    { action: "reuse", direction: "sendrecv" },
  );
  assert.deepEqual(
    videoSlotPlan({ hasSender: true, direction: "inactive" }),
    { action: "reuse", direction: "sendonly" },
  );
  // A transceiver that really is stopped cannot be revived, so this one honestly starts over.
  assert.deepEqual(
    videoSlotPlan({ hasSender: true, direction: "stopped" }),
    { action: "add", direction: null },
  );
});

test("a screen share is not held to the camera's bitrate", () => {
  // Cam and screen ride the same sender, so the cap has to be re-applied on every swap. A screen
  // share left on the camera's budget arrives as mush: text and edges need the headroom.
  assert.ok(VIDEO_BITRATE.screen > VIDEO_BITRATE.cam);
  assert.equal(VIDEO_BITRATE.cam, 500_000);
  assert.equal(VIDEO_BITRATE.screen, 1_200_000);
});

test("early ICE buffering is bounded and keeps the newest candidates", () => {
  assert.deepEqual(bufferIce(bufferIce([], "host"), "relay"), ["host", "relay"]);

  let queued: number[] = [];
  for (let i = 0; i < MAX_BUFFERED_ICE_PER_PEER + 9; i += 1) queued = bufferIce(queued, i);
  assert.equal(queued.length, MAX_BUFFERED_ICE_PER_PEER);
  assert.equal(queued[0], 9);
  assert.equal(queued.at(-1), MAX_BUFFERED_ICE_PER_PEER + 8);
});
