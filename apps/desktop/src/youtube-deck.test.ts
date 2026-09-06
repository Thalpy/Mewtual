import { test } from "node:test";
import assert from "node:assert/strict";

import {
  YT_BUFFERING,
  YT_CUED,
  YT_ENDED,
  YT_ORIGIN,
  YT_PAUSED,
  YT_PLAYING,
  YT_REPORT_FRESH_MS,
  YT_UNSTARTED,
  readYouTubeMessage,
  youtubeBlocked,
  youtubePosition,
  youtubeReported,
  youtubeTransportPlan,
  ytCommand,
  ytListen,
} from "./youtube-deck.ts";

test("commands go out in the form the frame expects, on the no-cookie origin", () => {
  assert.equal(YT_ORIGIN, "https://www.youtube-nocookie.com");
  assert.deepEqual(JSON.parse(ytListen()), { event: "listening", id: 1, channel: "widget" });
  assert.deepEqual(JSON.parse(ytCommand("playVideo")), { event: "command", func: "playVideo", args: [] });
  assert.deepEqual(JSON.parse(ytCommand("seekTo", [42, true])), {
    event: "command", func: "seekTo", args: [42, true],
  });
});

test("what the frame volunteers is read, in both shapes it sends", () => {
  // `onStateChange` carries the state bare; `infoDelivery` carries an object.
  assert.deepEqual(readYouTubeMessage(JSON.stringify({ event: "onStateChange", info: YT_PLAYING })), {
    state: YT_PLAYING,
  });
  assert.deepEqual(
    readYouTubeMessage(JSON.stringify({
      event: "infoDelivery",
      info: { playerState: YT_PAUSED, currentTime: 12.5, duration: 210 },
    })),
    { state: YT_PAUSED, currentTime: 12.5, duration: 210 },
  );
  // Already-parsed objects arrive too, depending on how the frame posted them.
  assert.deepEqual(readYouTubeMessage({ event: "infoDelivery", info: { currentTime: 3 } }), { currentTime: 3 });
});

test("a message that is not the player's, or not a message at all, teaches us nothing", () => {
  for (const junk of [
    null, undefined, 42, "not json", "{", [], {},
    { event: "somethingElse", info: { currentTime: 5 } },
    JSON.stringify({ info: { currentTime: 5 } }),
  ]) {
    assert.equal(readYouTubeMessage(junk), null, `${JSON.stringify(junk)} must not read as a report`);
  }
});

test("a value arithmetic cannot use is dropped rather than believed", () => {
  // This is the failure that matters: a bad currentTime reaching the drift arithmetic would seek
  // every listener in the room to nowhere at once, which is far worse than never correcting.
  for (const bad of [NaN, Infinity, -Infinity, -5, 999_999, "12", null, {}]) {
    const report = readYouTubeMessage({ event: "infoDelivery", info: { currentTime: bad } });
    assert.ok(report, "the message is still a message");
    assert.equal(report!.currentTime, undefined, `currentTime ${String(bad)} must be dropped`);
  }
  // A zero duration is "not known yet", not a zero-length video.
  assert.equal(readYouTubeMessage({ event: "infoDelivery", info: { duration: 0 } })!.duration, undefined);
  // A non-integer state is not one of the documented states.
  assert.equal(readYouTubeMessage({ event: "infoDelivery", info: { playerState: 1.5 } })!.state, undefined);
});

test("position prefers the player's own answer, but never depends on getting one", () => {
  const projected = 100;
  // No report at all: a frame that never answers still keeps the room roughly together.
  assert.equal(youtubePosition(projected, null, 5000), projected);
  // A fresh report wins, aged by how long ago it arrived.
  assert.equal(youtubePosition(projected, { at: 4000, currentTime: 90 }, 5000), 91);
  // A stale one does not: past the freshness window the projection at least advances.
  assert.equal(youtubePosition(projected, { at: 5000 - YT_REPORT_FRESH_MS - 1, currentTime: 90 }, 5000), projected);
  // Clock going backwards (a suspended machine, a reset performance clock) is not a fresh report.
  assert.equal(youtubePosition(projected, { at: 9000, currentTime: 90 }, 5000), projected);
  assert.equal(youtubePosition(projected, { at: NaN, currentTime: 90 }, 5000), projected);
});

test("not knowing where the player is stays distinguishable from it being where we expected", () => {
  // The drift check needs this difference. Comparing against a projection derived from the very
  // offset being checked would compare a number with itself: the gap is always zero, so a player
  // that had quietly wandered would be reported as perfectly in sync forever.
  assert.equal(youtubeReported(null, 5000), null);
  assert.equal(youtubeReported({ at: 5000 - YT_REPORT_FRESH_MS - 1, currentTime: 90 }, 5000), null);
  assert.equal(youtubeReported({ at: 4000, currentTime: 90 }, 5000), 91);
  // And the two agree wherever a reading exists at all.
  assert.equal(youtubePosition(7, { at: 4000, currentTime: 90 }, 5000), youtubeReported({ at: 4000, currentTime: 90 }, 5000));
});

test("a player refusing to start reads as needing a gesture, not as a broken track", () => {
  // The webview will not start video without a gesture, and somebody who joined mid-track has
  // made none. That has to be distinguishable from a track that is simply still loading.
  assert.equal(youtubeBlocked(YT_PAUSED, true), true);
  assert.equal(youtubeBlocked(YT_UNSTARTED, true), true);
  assert.equal(youtubeBlocked(YT_CUED, true), true);
  assert.equal(youtubeBlocked(YT_BUFFERING, true), false, "loading is not refusing");
  assert.equal(youtubeBlocked(YT_PLAYING, true), false);
  assert.equal(youtubeBlocked(YT_ENDED, true), false);
  // Nothing is being refused when the room is not asking for playback.
  for (const state of [YT_PAUSED, YT_UNSTARTED, YT_CUED, YT_PLAYING]) {
    assert.equal(youtubeBlocked(state, false), false);
  }
});

test("the transport plan seeks before it plays", () => {
  // Playing first would make every listener audibly start in the wrong place and then jump.
  const plan = youtubeTransportPlan({ target: 60, at: 10, playing: true, state: YT_PAUSED, seekAfter: 2 });
  assert.equal(plan.length, 2);
  assert.equal(JSON.parse(plan[0]).func, "seekTo");
  assert.deepEqual(JSON.parse(plan[0]).args, [60, true]);
  assert.equal(JSON.parse(plan[1]).func, "playVideo");
});

test("a small gap is left alone, because a seek is the only correction available", () => {
  // The embed has no playback-rate control, so the gentle easing a shared video normally gets is
  // not on offer. Seeking for small gaps would make a room stutter in unison.
  assert.deepEqual(
    youtubeTransportPlan({ target: 61, at: 60, playing: true, state: YT_PLAYING, seekAfter: 2 }),
    [],
    "one second adrift and already playing needs nothing",
  );
  const corrected = youtubeTransportPlan({ target: 90, at: 60, playing: true, state: YT_PLAYING, seekAfter: 2 });
  assert.equal(corrected.length, 1);
  assert.equal(JSON.parse(corrected[0]).func, "seekTo");
});

test("a player that has not said where it is gets placed once, so a joiner lands with the room", () => {
  const plan = youtubeTransportPlan({ target: 300, at: null, playing: true, state: YT_UNSTARTED, seekAfter: 2 });
  assert.equal(JSON.parse(plan[0]).func, "seekTo");
  assert.deepEqual(JSON.parse(plan[0]).args, [300, true], "not the top of the video");
});

test("pause and resume follow the room, and neither is issued twice", () => {
  assert.deepEqual(
    youtubeTransportPlan({ target: 60, at: 60, playing: false, state: YT_PLAYING, seekAfter: 2 }),
    [ytCommand("pauseVideo")],
  );
  assert.deepEqual(
    youtubeTransportPlan({ target: 60, at: 60, playing: false, state: YT_PAUSED, seekAfter: 2 }),
    [],
    "already paused",
  );
  assert.deepEqual(
    youtubeTransportPlan({ target: 60, at: 60, playing: true, state: YT_BUFFERING, seekAfter: 2 }),
    [],
    "on its way to playing",
  );
});

test("a negative target never leaves the app as one", () => {
  // The offset comes off the wire and is aged on a local clock, so it can go slightly negative
  // between a correction and a pause. A player asked to seek before zero is being asked nonsense.
  const plan = youtubeTransportPlan({ target: -5, at: 60, playing: true, state: YT_PLAYING, seekAfter: 2 });
  assert.deepEqual(JSON.parse(plan[0]).args, [0, true]);
});
