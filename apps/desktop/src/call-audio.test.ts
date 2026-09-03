import { test } from "node:test";
import assert from "node:assert/strict";

import {
  DEFAULT_PEER_LEVEL,
  MAX_PEER_LEVEL,
  RemoteAudioRouter,
  effectivePeerGain,
  normalizePeerLevel,
  peerGain,
} from "./call-audio.ts";

test("a peer can be turned up past unity, and never past the bound", () => {
  // The bug this replaces: the level drove HTMLAudioElement.volume, which the browser caps at 1,
  // so a friend with a quiet microphone could be turned down but never up.
  assert.ok(MAX_PEER_LEVEL > 100, "there has to be headroom above unity for this to be worth it");
  assert.equal(normalizePeerLevel(250), 250);
  assert.equal(peerGain(250), 2.5);
  assert.equal(peerGain(DEFAULT_PEER_LEVEL), 1, "unity is what they sent");
  assert.equal(peerGain(0), 0);

  assert.equal(normalizePeerLevel(MAX_PEER_LEVEL + 1), MAX_PEER_LEVEL, "the bound holds");
  assert.equal(normalizePeerLevel(-40), 0);
  assert.equal(normalizePeerLevel(137.6), 138, "a stored fractional level lands on a whole one");
  // Nothing that is not a number may reach the graph: a NaN gain silences a node permanently and
  // there is no UI that would explain why.
  assert.equal(normalizePeerLevel(Number.NaN), DEFAULT_PEER_LEVEL);
  // These three are the ones that matter, because Number(null) and Number("") are both 0: a level
  // that was never stored must come back as unity, not as silence.
  assert.equal(normalizePeerLevel(null), DEFAULT_PEER_LEVEL);
  assert.equal(normalizePeerLevel(undefined), DEFAULT_PEER_LEVEL);
  assert.equal(normalizePeerLevel(""), DEFAULT_PEER_LEVEL);
  assert.equal(normalizePeerLevel("   "), DEFAULT_PEER_LEVEL);
  assert.equal(normalizePeerLevel(false), DEFAULT_PEER_LEVEL);
  assert.equal(normalizePeerLevel("nonsense"), DEFAULT_PEER_LEVEL);
  assert.equal(normalizePeerLevel("175"), 175, "a level read back out of storage is a string");
  assert.equal(normalizePeerLevel(Number.NaN, 0), 0, "a caller may say what absent means");
});

test("mute and deafen reach the gain, not just the element", () => {
  assert.equal(effectivePeerGain(250, false, false), 2.5);
  assert.equal(effectivePeerGain(250, true, false), 0, "muted for me alone");
  assert.equal(effectivePeerGain(250, false, true), 0, "deafened outranks a level");
  assert.equal(effectivePeerGain(250, true, true), 0);
  // The level survives being silenced: lifting the mute restores what was set, it does not reset.
  assert.equal(effectivePeerGain(250, false, false), 2.5);
});

test("a peer's share is told apart from their voice by the stream their video rides", () => {
  const router = new RemoteAudioRouter();
  // Ordinary case: someone in a call talking, with no share at all.
  assert.equal(router.classify("alice", "mic-stream"), "voice");

  // They start sharing. The video arrives first, so its audio is known on arrival.
  assert.equal(router.noteVideo("alice", "screen-stream"), false, "nothing to revise yet");
  assert.equal(router.classify("alice", "screen-stream"), "share");
  assert.equal(router.classify("alice", "mic-stream"), "voice", "their voice is still their voice");
});

test("audio arriving before the video it belongs to is moved rather than left wrong", () => {
  const router = new RemoteAudioRouter();
  // One negotiation carries both tracks and the order is not ours to choose. Answering "voice"
  // here is unavoidable; failing to revise it is what would put a shared game on the voice slider.
  assert.equal(router.classify("bob", "screen-stream"), "voice");
  assert.equal(router.noteVideo("bob", "screen-stream"), true, "that answer has to be revised");
  assert.equal(router.classify("bob", "screen-stream"), "share");
  // Being told the same thing again is not a revision: a re-offer must not rebuild a correct graph.
  assert.equal(router.noteVideo("bob", "screen-stream"), false);

  // A peer who stops sharing and shares something else moves the share to the new stream.
  assert.equal(router.noteVideo("bob", "second-screen"), false, "unheard of stream, nothing to move");
  assert.equal(router.classify("bob", "second-screen"), "share");
  assert.equal(router.classify("bob", "screen-stream"), "voice", "the old one is no longer a share");
});

test("peers are forgotten when they leave, so a reused stream id is not misread", () => {
  const router = new RemoteAudioRouter();
  router.noteVideo("carol", "screen-stream");
  assert.equal(router.classify("carol", "screen-stream"), "share");
  router.forget("carol");
  assert.equal(router.classify("carol", "screen-stream"), "voice", "a rejoin starts from nothing");
  // One peer's knowledge is never another's.
  router.noteVideo("dan", "shared");
  assert.equal(router.classify("erin", "shared"), "voice");
  router.clear();
  assert.equal(router.classify("dan", "shared"), "voice");
});
