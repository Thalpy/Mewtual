import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_BUFFERED_ICE_PER_PEER,
  bufferIce,
  heartbeatRecovery,
  isCurrentVoiceRoom,
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

test("early ICE buffering is bounded and keeps the newest candidates", () => {
  assert.deepEqual(bufferIce(bufferIce([], "host"), "relay"), ["host", "relay"]);

  let queued: number[] = [];
  for (let i = 0; i < MAX_BUFFERED_ICE_PER_PEER + 9; i += 1) queued = bufferIce(queued, i);
  assert.equal(queued.length, MAX_BUFFERED_ICE_PER_PEER);
  assert.equal(queued[0], 9);
  assert.equal(queued.at(-1), MAX_BUFFERED_ICE_PER_PEER + 8);
});
