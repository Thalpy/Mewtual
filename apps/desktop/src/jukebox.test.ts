import assert from "node:assert/strict";
import test from "node:test";
import {
  DRIFT_HOLD_S,
  DRIFT_SEEK_S,
  driftAction,
  fetchPhase,
  mediaKind,
  mediaUrl,
  nudgeRate,
} from "./jukebox.ts";

const progress = (over: Partial<Parameters<typeof fetchPhase>[0]> = {}) => ({
  cid: "ab".repeat(32),
  done: 1,
  total: 4,
  bytes_done: 250,
  bytes_total: 1000,
  network_bytes_done: 0,
  provider: null,
  ...over,
});

test("a media url names the server and the content address", () => {
  assert.equal(mediaUrl(7, "abc123"), "catcoms-media://a/7/abc123");
});

test("a media url escapes anything that would change its path shape", () => {
  // The cid reaching this function is already validated, but the backend refuses anything that is
  // not exactly <server>/<64 hex>, so a smuggled separator must not silently become one.
  assert.equal(mediaUrl(1, "a/b"), "catcoms-media://a/1/a%2Fb");
  assert.equal(mediaUrl(1, "../x"), "catcoms-media://a/1/..%2Fx");
});

test("the declared mime decides a file's kind when it says anything useful", () => {
  assert.equal(mediaKind("clip.bin", "video/mp4"), "video");
  assert.equal(mediaKind("clip.bin", "audio/mpeg"), "audio");
  assert.equal(mediaKind("clip.bin", "VIDEO/WEBM"), "video");
});

test("a queue entry with no mime falls back to its extension", () => {
  // The load-bearing case: a queue entry carries no mime at all, so a listener reading the queue
  // has only the filename to tell a film from a song.
  assert.equal(mediaKind("holiday.mkv"), "video");
  assert.equal(mediaKind("holiday.MP4"), "video");
  assert.equal(mediaKind("song.flac"), "audio");
  assert.equal(mediaKind("song.opus"), "audio");
  assert.equal(mediaKind("notes.txt"), "other");
  assert.equal(mediaKind("noextension"), "other");
  assert.equal(mediaKind(""), "other");
});

test("a mime that is not media at all does not become media by its extension", () => {
  // A file the backend will serve as application/octet-stream must not be queued as playable
  // just because somebody named it .mp4.
  assert.equal(mediaKind("payload.mp4", "text/html"), "video");
});

test("video eases small drift out and snaps only large drift", () => {
  // A snap in a shared film is jarring for everyone already in sync, so the middle band nudges.
  assert.equal(driftAction(0.1, "video"), "hold");
  assert.equal(driftAction(DRIFT_HOLD_S - 0.01, "video"), "hold");
  assert.equal(driftAction(1, "video"), "nudge");
  assert.equal(driftAction(-1, "video"), "nudge");
  assert.equal(driftAction(DRIFT_SEEK_S + 0.01, "video"), "seek");
  assert.equal(driftAction(-5, "video"), "seek");
});

test("audio keeps snap-or-nothing, because a rate change is audible where a seek is not", () => {
  assert.equal(driftAction(1, "audio"), "hold");
  assert.equal(driftAction(-1, "audio"), "hold");
  assert.equal(driftAction(3, "audio"), "seek");
  assert.equal(driftAction(-3, "audio"), "seek");
});

test("the nudge rate chases the room in the right direction", () => {
  // Positive drift means the room is ahead of us, so we must play faster to catch it.
  assert.ok(nudgeRate(1) > 1, "behind the room plays faster");
  assert.ok(nudgeRate(-1) < 1, "ahead of the room plays slower");
  assert.equal(nudgeRate(0), 1, "in sync never changes rate");
  assert.equal(nudgeRate(DRIFT_HOLD_S - 0.01), 1, "inside the hold band never changes rate");
  // Gentle enough not to sound wrong, and a fixed step rather than proportional to the drift:
  // a rate that scaled with a large gap would sound like a fast-forward before the seek band
  // took over.
  assert.equal(nudgeRate(9), 1.05);
  assert.equal(nudgeRate(-9), 0.95);
});

test("progress separates reading this disk from pulling off a peer", () => {
  assert.equal(fetchPhase(progress()).source, "local");
  assert.equal(fetchPhase(progress({ network_bytes_done: 10 })).source, "network");
});

test("a partly held file does not flicker between local and network", () => {
  // provider names only the most recent chunk's source, so a half-held file would flip labels as
  // the loop walked its chunks. network_bytes_done is cumulative, so it does not.
  const held = fetchPhase(progress({ network_bytes_done: 128, provider: null }));
  assert.equal(held.source, "network", "one network chunk means the fetch was not purely local");
});

test("progress percent is derived and clamped", () => {
  assert.equal(fetchPhase(progress({ bytes_done: 0 })).percent, 0);
  assert.equal(fetchPhase(progress({ bytes_done: 500 })).percent, 50);
  assert.equal(fetchPhase(progress({ bytes_done: 1000 })).percent, 100);
  // A total that is not known yet must read as 0, never NaN, or the bar renders as width:NaN%.
  assert.equal(fetchPhase(progress({ bytes_total: 0, bytes_done: 0 })).percent, 0);
  // Defensive: an over-count must not produce a bar wider than its track.
  assert.equal(fetchPhase(progress({ bytes_done: 5000 })).percent, 100);
});

test("progress carries the serving peer through for attribution", () => {
  assert.equal(fetchPhase(progress({ provider: "d465c67a" })).provider, "d465c67a");
  assert.equal(fetchPhase(progress()).provider, "");
});
