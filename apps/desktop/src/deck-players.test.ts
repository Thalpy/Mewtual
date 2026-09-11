import assert from "node:assert/strict";
import test from "node:test";

import {
  DECK_REPORT_FRESH_MS,
  SOUNDCLOUD_DECK,
  VIMEO_DECK,
  YOUTUBE_DECK,
  deckDriver,
  deckPlayerBlocked,
  deckPlayerPosition,
  deckReported,
  deckSources,
  deckTransportPlan,
  type DeckDriver,
} from "./deck-players.ts";

const ALL = [YOUTUBE_DECK, SOUNDCLOUD_DECK, VIMEO_DECK];

test("every driver is reachable by the slug a queue entry stores", () => {
  for (const driver of ALL) assert.equal(deckDriver(driver.id), driver);
  assert.deepEqual(deckSources().sort(), ["soundcloud", "vimeo", "youtube"]);
  // A source this build does not know is not a track it can play, and must not resolve to one.
  assert.equal(deckDriver("spotify"), null, "Spotify is deliberately not a deck source");
  assert.equal(deckDriver(""), null);
  assert.equal(deckDriver("YOUTUBE"), null, "slugs are exact");
});

test("a driver sends commands only on its own origin's protocol, and every command is JSON", () => {
  for (const driver of ALL) {
    assert.match(driver.origin, /^https:\/\//);
    const messages = [
      ...driver.hello(),
      driver.play(),
      driver.pause(),
      driver.seek(42),
      driver.volume(0.5),
      driver.mute(true),
    ].filter((m): m is string => m !== null);
    assert.ok(messages.length >= 4, `${driver.id} must speak`);
    for (const message of messages) {
      assert.doesNotThrow(() => JSON.parse(message), `${driver.id} sent non-JSON: ${message}`);
    }
  }
});

test("SoundCloud seeks in milliseconds, which is the reason this layer exists at all", () => {
  // A seek sent in seconds would land a thousandth of the way in, and a room would drift apart in
  // a way that reads as a sync bug rather than a unit bug.
  assert.deepEqual(JSON.parse(SOUNDCLOUD_DECK.seek(90)), { method: "seekTo", value: 90_000 });
  assert.deepEqual(JSON.parse(VIMEO_DECK.seek(90)), { method: "setCurrentTime", value: 90 });
  assert.deepEqual(JSON.parse(YOUTUBE_DECK.seek(90)), {
    event: "command", func: "seekTo", args: [90, true],
  });
  // And a negative target never leaves the app as one: the offset comes off the wire and is aged
  // on a local clock, so it can go slightly negative between a correction and a pause.
  assert.equal(JSON.parse(SOUNDCLOUD_DECK.seek(-5)).value, 0);
  assert.equal(JSON.parse(VIMEO_DECK.seek(-5)).value, 0);
  assert.deepEqual(JSON.parse(YOUTUBE_DECK.seek(-5)).args, [0, true]);
});

test("volume reaches each provider in the scale it actually uses", () => {
  assert.deepEqual(JSON.parse(YOUTUBE_DECK.volume(0.5)!).args, [50], "YouTube counts to a hundred");
  assert.equal(JSON.parse(SOUNDCLOUD_DECK.volume(0.5)!).value, 50);
  assert.equal(JSON.parse(VIMEO_DECK.volume(0.5)!).value, 0.5, "Vimeo counts to one");
  // Out-of-range and junk clamp rather than reaching a player as nonsense.
  assert.deepEqual(JSON.parse(YOUTUBE_DECK.volume(9)!).args, [100]);
  assert.equal(JSON.parse(SOUNDCLOUD_DECK.volume(-1)!).value, 0);
  assert.equal(JSON.parse(VIMEO_DECK.volume(NaN)!).value, 1);
});

test("every driver reads its own progress reports, and ignores everything else", () => {
  const cases: [DeckDriver, unknown, { state?: string; currentTime?: number }][] = [
    [YOUTUBE_DECK, JSON.stringify({ event: "onStateChange", info: 1 }), { state: "playing" }],
    [YOUTUBE_DECK, { event: "infoDelivery", info: { playerState: 2, currentTime: 12.5 } }, { state: "paused", currentTime: 12.5 }],
    [SOUNDCLOUD_DECK, { method: "playProgress", value: { currentPosition: 12_500 } }, { state: "playing", currentTime: 12.5 }],
    [SOUNDCLOUD_DECK, { method: "finish" }, { state: "ended" }],
    [VIMEO_DECK, { event: "timeupdate", data: { seconds: 12.5, duration: 210 } }, { state: "playing", currentTime: 12.5 }],
    [VIMEO_DECK, { event: "pause", data: {} }, { state: "paused" }],
  ];
  for (const [driver, message, expected] of cases) {
    const report = driver.read(message);
    assert.ok(report, `${driver.id} should read ${JSON.stringify(message)}`);
    for (const [key, value] of Object.entries(expected)) {
      assert.equal((report as Record<string, unknown>)[key], value, `${driver.id}.${key}`);
    }
  }
  // Vimeo's timeupdate is where a track's length comes from.
  assert.equal(VIMEO_DECK.read({ event: "timeupdate", data: { seconds: 1, duration: 210 } })?.duration, 210);

  for (const driver of ALL) {
    for (const junk of [null, undefined, 42, "not json", "{", [], {}, { event: "other" }, { method: "other" }]) {
      assert.equal(driver.read(junk), null, `${driver.id} must not read ${JSON.stringify(junk)}`);
    }
  }
});

test("a value arithmetic cannot use is dropped rather than believed", () => {
  // This is the failure that matters: a bad position reaching the drift arithmetic would seek
  // every listener in the room to nowhere at once, which is far worse than never correcting.
  //
  // "Absurd" has to be expressed in each provider's own unit, which is itself the point of this
  // layer: 999999 is a nonsense number of seconds and an ordinary sixteen minutes of SoundCloud's
  // milliseconds, and a bound applied in the wrong unit would reject real positions.
  const nonsense = [NaN, Infinity, -5, "12", null, {}];
  for (const bad of [...nonsense, 999_999]) {
    assert.equal(
      YOUTUBE_DECK.read({ event: "infoDelivery", info: { currentTime: bad } })?.currentTime,
      undefined,
      `YouTube currentTime ${String(bad)}`,
    );
    assert.equal(
      VIMEO_DECK.read({ event: "timeupdate", data: { seconds: bad } })?.currentTime,
      undefined,
      `Vimeo seconds ${String(bad)}`,
    );
  }
  for (const bad of [...nonsense, 999_999_999]) {
    assert.equal(
      SOUNDCLOUD_DECK.read({ method: "playProgress", value: { currentPosition: bad } })?.currentTime,
      undefined,
      `SoundCloud currentPosition ${String(bad)}`,
    );
  }
  // And the bound really is applied after the conversion, so a long mix is not mistaken for junk.
  assert.equal(
    SOUNDCLOUD_DECK.read({ method: "playProgress", value: { currentPosition: 999_999 } })?.currentTime,
    999.999,
    "sixteen minutes in is an ordinary position",
  );
  // A zero duration is "not known yet", not a zero-length track.
  assert.equal(VIMEO_DECK.read({ event: "timeupdate", data: { seconds: 1, duration: 0 } })?.duration, undefined);
});

test("not knowing where a player is stays distinguishable from it being where we expected", () => {
  // The drift check needs this difference. Comparing against a projection derived from the very
  // offset being checked would compare a number with itself, so a player that had quietly wandered
  // would read as perfectly in sync forever.
  assert.equal(deckReported(null, 5000), null);
  assert.equal(deckReported({ at: 5000 - DECK_REPORT_FRESH_MS - 1, currentTime: 90 }, 5000), null);
  assert.equal(deckReported({ at: 4000, currentTime: 90 }, 5000), 91);
  assert.equal(deckReported({ at: 9000, currentTime: 90 }, 5000), null, "a clock that went backwards");
  assert.equal(deckReported({ at: NaN, currentTime: 90 }, 5000), null);

  // Position falls back to the projection wherever there is no fresh reading, so a player that
  // never answers still keeps a room roughly together.
  assert.equal(deckPlayerPosition(100, null, 5000), 100);
  assert.equal(deckPlayerPosition(100, { at: 4000, currentTime: 90 }, 5000), 91);
  assert.equal(deckPlayerPosition(100, { at: 0, currentTime: 90 }, 5000), 100);
});

test("a player refusing to start reads as needing a gesture, not as a broken track", () => {
  assert.equal(deckPlayerBlocked("paused", true), true);
  assert.equal(deckPlayerBlocked("unstarted", true), true);
  assert.equal(deckPlayerBlocked("buffering", true), false, "loading is not refusing");
  assert.equal(deckPlayerBlocked("playing", true), false);
  assert.equal(deckPlayerBlocked("ended", true), false);
  // Nothing is being refused when the room is not asking for playback.
  for (const state of ["paused", "unstarted", "buffering", "playing"] as const) {
    assert.equal(deckPlayerBlocked(state, false), false);
  }
});

test("the transport plan seeks before it plays, for every provider", () => {
  // Playing first would make every listener audibly start in the wrong place and then jump.
  for (const driver of ALL) {
    const plan = deckTransportPlan({ driver, target: 60, at: 10, playing: true, state: "paused", seekAfter: 2 });
    assert.equal(plan.length, 2, driver.id);
    assert.equal(plan[0], driver.seek(60), `${driver.id} seeks first`);
    assert.equal(plan[1], driver.play(), `${driver.id} plays second`);
  }
});

test("a small gap is left alone, because a seek is the only correction on offer", () => {
  // None of these embeds exposes a usable playback-rate control, so the gentle easing a shared
  // local video gets is not available. Seeking for small gaps would make a room stutter in unison.
  for (const driver of ALL) {
    assert.deepEqual(
      deckTransportPlan({ driver, target: 61, at: 60, playing: true, state: "playing", seekAfter: 2 }),
      [],
      `${driver.id}: one second adrift and already playing needs nothing`,
    );
    assert.deepEqual(
      deckTransportPlan({ driver, target: 90, at: 60, playing: true, state: "playing", seekAfter: 2 }),
      [driver.seek(90)],
      `${driver.id}: a real gap is snapped`,
    );
    // A player that has not said where it is gets placed once, so a joiner lands with the room.
    assert.equal(
      deckTransportPlan({ driver, target: 300, at: null, playing: true, state: "unstarted", seekAfter: 2 })[0],
      driver.seek(300),
      `${driver.id}: not the top of the track`,
    );
  }
});

test("pause and resume follow the room, and neither is issued twice", () => {
  for (const driver of ALL) {
    assert.deepEqual(
      deckTransportPlan({ driver, target: 60, at: 60, playing: false, state: "playing", seekAfter: 2 }),
      [driver.pause()],
    );
    assert.deepEqual(
      deckTransportPlan({ driver, target: 60, at: 60, playing: false, state: "paused", seekAfter: 2 }),
      [],
      `${driver.id}: already paused`,
    );
    assert.deepEqual(
      deckTransportPlan({ driver, target: 60, at: 60, playing: true, state: "buffering", seekAfter: 2 }),
      [],
      `${driver.id}: on its way to playing`,
    );
  }
});
