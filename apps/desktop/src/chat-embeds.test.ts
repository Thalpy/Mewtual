import { test } from "node:test";
import assert from "node:assert/strict";

import {
  EMBED_KEEPALIVE_MARGIN_PX,
  chatEmbedFor,
  chatEmbedLink,
  embedChipLabel,
  embedChipTitle,
  embedKey,
  embedMayRender,
} from "./chat-embeds.ts";

const VIDEO = "dQw4w9WgXcQ";
const TRACK = "4cOdK2wGLETKBW3PvgPWqT";

test("each provider claims only its own links, and nothing else gets a card", () => {
  assert.equal(chatEmbedFor(`https://open.spotify.com/track/${TRACK}`)?.provider, "spotify");
  assert.equal(chatEmbedFor(`https://youtu.be/${VIDEO}`)?.provider, "youtube");
  assert.equal(chatEmbedFor("https://example.com/a-page"), null);
  assert.equal(chatEmbedFor("https://open.spotify.com.evil.example/track/x"), null);
  assert.equal(chatEmbedFor(""), null);
});

test("a card's key names the content, not the message it appeared in", () => {
  // The same track linked twice in a conversation is one decision, so the key must not carry
  // anything situational.
  const first = chatEmbedFor(`https://open.spotify.com/track/${TRACK}?si=abc123def456`)!;
  const second = chatEmbedFor(`spotify:track:${TRACK}`)!;
  assert.equal(embedKey(first), embedKey(second));
  assert.equal(embedKey(first), `spotify:track:${TRACK}`);

  // A video linked at a different timestamp is a different thing to watch, so it is its own key.
  const plain = chatEmbedFor(`https://youtu.be/${VIDEO}`)!;
  const timed = chatEmbedFor(`https://youtu.be/${VIDEO}?t=90`)!;
  assert.notEqual(embedKey(plain), embedKey(timed));
  assert.equal(embedKey(timed), `youtube:${VIDEO}:90`);

  // Two providers can never collide on a key.
  assert.notEqual(embedKey(plain), embedKey(first));
});

test("a frame exists only while it is permitted AND somebody can see it", () => {
  const live = { clicked: true, autoLoad: false, onScreen: true, windowVisible: true };
  assert.equal(embedMayRender(live), true);

  // Each condition on its own is enough to unmount, and that is the point: a card that survived
  // any one of them would be a third-party connection running where nobody can see it.
  assert.equal(embedMayRender({ ...live, clicked: false }), false, "never loaded without a click");
  assert.equal(embedMayRender({ ...live, onScreen: false }), false, "scrolled away, or a pane that is not selected");
  assert.equal(embedMayRender({ ...live, windowVisible: false }), false, "minimised or a background window");
  assert.equal(embedMayRender({ clicked: false, autoLoad: false, onScreen: false, windowVisible: false }), false);
});

test("auto-load replaces the click and nothing else", () => {
  const seen = { onScreen: true, windowVisible: true };
  // Either permission is enough to load, which is the whole of what the setting does.
  assert.equal(embedMayRender({ clicked: false, autoLoad: true, ...seen }), true);
  assert.equal(embedMayRender({ clicked: true, autoLoad: false, ...seen }), true);
  assert.equal(embedMayRender({ clicked: false, autoLoad: false, ...seen }), false);

  // And this is the line that matters: turning it on is asking not to be interrupted by chips,
  // not asking for frames that talk to Google from a window nobody has open. A preference must
  // never buy its way past the second half of the rule.
  for (const unseen of [
    { onScreen: false, windowVisible: true },
    { onScreen: true, windowVisible: false },
    { onScreen: false, windowVisible: false },
  ]) {
    assert.equal(
      embedMayRender({ clicked: true, autoLoad: true, ...unseen }),
      false,
      "auto-load must not keep an unseen card mounted",
    );
  }
});

test("a card survives the round trip through the DOM it is stored in", () => {
  // A card is unmounted and rebuilt every time the reader scrolls past it, so this round trip
  // runs constantly. It has to land on the same entity every time, and it has to go back through
  // the parser rather than trusting an attribute because it was already there.
  for (const link of [
    `https://open.spotify.com/track/${TRACK}?si=abc123def456`,
    `https://open.spotify.com/intl-de/playlist/${TRACK}`,
    `spotify:episode:${TRACK}`,
    `https://youtu.be/${VIDEO}?t=1m30s`,
    `https://www.youtube.com/shorts/${VIDEO}`,
    `https://music.youtube.com/watch?v=${VIDEO}`,
  ]) {
    const first = chatEmbedFor(link)!;
    assert.ok(first, link);
    const rebuilt = chatEmbedFor(chatEmbedLink(first));
    assert.ok(rebuilt, `${link} must survive being written to the DOM and read back`);
    assert.equal(embedKey(rebuilt!), embedKey(first));
    // And the stored form is a second fixed point, so repeated scrolling cannot drift.
    assert.equal(chatEmbedLink(rebuilt!), chatEmbedLink(first));
  }
});

test("a tampered stored link is re-validated rather than trusted", () => {
  // The round trip is a parse, not a cast: an attribute rewritten to point somewhere else yields
  // no card at all instead of a frame pointed at an arbitrary host.
  assert.equal(chatEmbedFor("https://evil.example/watch?v=dQw4w9WgXcQ"), null);
  assert.equal(chatEmbedFor("javascript:alert(1)"), null);
  assert.equal(chatEmbedFor("https://open.spotify.com/track/../../../evil"), null);
});

test("the keepalive margin is slack for reading, not a way to stay mounted off screen", () => {
  assert.ok(EMBED_KEEPALIVE_MARGIN_PX > 0, "zero would make a card flicker on any nudge of the scroll");
  assert.ok(EMBED_KEEPALIVE_MARGIN_PX <= 1000, "a card must not outlive a genuine scroll away from it");
});

test("the chip names the service, because that is what is being consented to", () => {
  const video = chatEmbedFor(`https://youtu.be/${VIDEO}`)!;
  const track = chatEmbedFor(`https://open.spotify.com/track/${TRACK}`)!;
  const show = chatEmbedFor(`https://open.spotify.com/show/${TRACK}`)!;

  assert.equal(embedChipLabel(video), "Load YouTube video");
  assert.equal(embedChipLabel(track), "Load Spotify track");
  assert.equal(embedChipLabel(show), "Load Spotify podcast", "\"show\" is not what anyone calls it");

  // "Load embed" would not tell anybody what is about to be contacted, which is the substance of
  // the decision.
  for (const embed of [video, track]) {
    const label = embedChipLabel(embed);
    const service = embed.provider === "youtube" ? "YouTube" : "Spotify";
    assert.ok(label.includes(service), `${label} must name the service`);
    assert.ok(embedChipTitle(embed).includes("discloses your address"), "the cost is stated, not implied");
    assert.ok(embedChipTitle(embed).includes("scroll away"), "so is the unloading behaviour");
  }
  assert.ok(embedChipTitle(track).includes("preview"), "Spotify's usual outcome is not full playback");
});
