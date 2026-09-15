import { test } from "node:test";
import assert from "node:assert/strict";

import {
  spotifyEmbedHeight,
  spotifyEmbedUrl,
  spotifyLabel,
  spotifyPageUrl,
  spotifyRef,
} from "./spotify.ts";

const ID = "4cOdK2wGLETKBW3PvgPWqT";

test("the shapes people actually paste are recognised", () => {
  assert.deepEqual(spotifyRef(`https://open.spotify.com/track/${ID}`), { kind: "track", id: ID });
  assert.deepEqual(spotifyRef(`spotify:track:${ID}`), { kind: "track", id: ID });
  assert.deepEqual(spotifyRef(`  https://open.spotify.com/album/${ID}  `), { kind: "album", id: ID });
  // The locale prefix on a share link and the `/embed/` form of an already-built card both reduce
  // to the same entity, so a member pasting either gets the same card.
  assert.deepEqual(spotifyRef(`https://open.spotify.com/intl-de/track/${ID}`), { kind: "track", id: ID });
  assert.deepEqual(spotifyRef(`https://open.spotify.com/intl-pt-br/playlist/${ID}`), { kind: "playlist", id: ID });
  assert.deepEqual(spotifyRef(`https://open.spotify.com/embed/episode/${ID}`), { kind: "episode", id: ID });
});

test("the share tracking token does not survive into the embed", () => {
  const ref = spotifyRef(`https://open.spotify.com/track/${ID}?si=8fd1a2b3c4d5e6f7&utm_source=copy-link`);
  assert.deepEqual(ref, { kind: "track", id: ID });
  // `si` identifies whose share was followed. The address is rebuilt from the kind and id alone,
  // so what Spotify is asked for is the music rather than one person's share of it.
  const url = spotifyEmbedUrl(ref!);
  assert.equal(url, `https://open.spotify.com/embed/track/${ID}`);
  assert.ok(!url.includes("si="), "no tracking token reaches the frame");
  assert.ok(!url.includes("utm_"), "no campaign token either");
});

test("only Spotify's own host is a Spotify link", () => {
  for (const bad of [
    `https://open.spotify.com.evil.example/track/${ID}`,
    `https://evil.example/open.spotify.com/track/${ID}`,
    `https://openspotify.com/track/${ID}`,
    `https://open.spotify.com.evil.example:443/track/${ID}`,
    `https://user:pass@evil.example/track/${ID}`,
  ]) {
    assert.equal(spotifyRef(bad), null, `${bad} must not be treated as Spotify`);
  }
});

test("nothing a member writes can leave the path segment it is written into", () => {
  // The id goes straight into a URL path. These are the ways out of a segment, and the base62
  // alphabet is what closes them: there is no escaping step to get wrong.
  for (const bad of [
    "https://open.spotify.com/track/../../../etc/passwd",
    `https://open.spotify.com/track/${ID}/../../artist/x`,
    "https://open.spotify.com/track/a%2Fb%2Fc%2Fd%2Fe%2Ff%2Fg%2Fh%2Fi",
    "https://open.spotify.com/track/id?x=1#frag",
    "https://open.spotify.com/track/id.with.dots.and.more.dots",
    "https://open.spotify.com/track/has-a-dash-in-it-which-is-not",
  ]) {
    const ref = spotifyRef(bad);
    if (ref) assert.match(ref.id, /^[A-Za-z0-9]+$/, `${bad} yielded a non-base62 id`);
  }
  // And the escaped-slash case specifically resolves to nothing rather than to a longer path.
  assert.equal(spotifyRef("https://open.spotify.com/track/a%2Fb"), null);
});

test("a scheme that is not the web is refused outright", () => {
  for (const bad of [
    `javascript:alert(1)//open.spotify.com/track/${ID}`,
    `data:text/html,<b>open.spotify.com/track/${ID}</b>`,
    `file:///open.spotify.com/track/${ID}`,
    `spotify:javascript:${ID}`,
  ]) {
    assert.equal(spotifyRef(bad), null, `${bad} must not be treated as Spotify`);
  }
});

test("an unknown entity kind gets no card, because the embed cannot render one", () => {
  assert.equal(spotifyRef(`https://open.spotify.com/user/${ID}`), null);
  assert.equal(spotifyRef(`https://open.spotify.com/search/${ID}`), null);
  assert.equal(spotifyRef("https://open.spotify.com/track"), null, "a kind with no id is not an entity");
  assert.equal(spotifyRef("https://open.spotify.com/"), null);
});

test("an id has to be a plausible length, and junk is not a link", () => {
  assert.equal(spotifyRef("https://open.spotify.com/track/short"), null);
  assert.equal(spotifyRef(`https://open.spotify.com/track/${"a".repeat(41)}`), null);
  assert.equal(spotifyRef(""), null);
  assert.equal(spotifyRef("   "), null);
  assert.equal(spotifyRef("not a url at all"), null);
  assert.equal(spotifyRef(`https://open.spotify.com/track/${ID}${"?pad=".padEnd(3000, "x")}`), null,
    "an absurdly long string is not a share link");
});

test("a collection is given room for its track list; a single track is not", () => {
  assert.equal(spotifyEmbedHeight("track"), 152);
  assert.equal(spotifyEmbedHeight("episode"), 152);
  assert.ok(spotifyEmbedHeight("playlist") > spotifyEmbedHeight("track"),
    "a playlist at compact height shows one row and reads as broken");
  assert.ok(spotifyEmbedHeight("album") > spotifyEmbedHeight("track"));
});

test("the chip and the outbound link name the same entity", () => {
  const ref = spotifyRef(`https://open.spotify.com/show/${ID}`)!;
  assert.equal(spotifyLabel(ref), "Load Spotify podcast", "\"show\" is not what anyone calls it");
  assert.equal(spotifyPageUrl(ref), `https://open.spotify.com/show/${ID}`);
  assert.equal(spotifyLabel({ kind: "track", id: ID }), "Load Spotify track");
});
