import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { EMBED_PROVIDERS, chatEmbedFor, embedFrameOrigins, embedKey } from "./chat-embeds.ts";
import { path, segment, webUrl } from "./embed-provider.ts";
import { APPLE_MUSIC, MIXCLOUD, SOUNDCLOUD } from "./providers-audio.ts";
import { BLUESKY, VIMEO } from "./providers-video.ts";

// --- the rules every provider obeys, checked against all of them at once -------------------------

test("no two providers can claim the same link", () => {
  // The registry returns the first parser that claims a link, so overlapping host sets would make
  // which card you get depend on array order. Asserting disjointness directly is the only way this
  // stays true when somebody adds a provider; reading the list and believing it is not.
  const seen = new Map<string, string>();
  for (const provider of EMBED_PROVIDERS) {
    for (const host of providerHosts(provider.id)) {
      const already = seen.get(host);
      assert.equal(already, undefined, `${host} is claimed by both ${already} and ${provider.id}`);
      seen.set(host, provider.id);
    }
  }
});

test("every provider's frame origin is in the app's frame-src, or its cards silently never load", () => {
  // A frame blocked by the CSP fails with no visible error, which looks exactly like a provider
  // that is broken. This reads the real config so the two lists cannot drift.
  const conf = JSON.parse(
    readFileSync(fileURLToPath(new URL("../src-tauri/tauri.conf.json", import.meta.url)), "utf8"),
  ) as { app: { security: { csp: string; devCsp: string } } };

  for (const policy of [conf.app.security.csp, conf.app.security.devCsp]) {
    const directive = policy
      .split(";")
      .map((part) => part.trim())
      .find((part) => part.startsWith("frame-src"));
    assert.ok(directive, "there must be a frame-src directive at all");
    const allowed = new Set(directive!.split(/\s+/).slice(1));
    for (const origin of embedFrameOrigins()) {
      assert.ok(allowed.has(origin), `${origin} is missing from frame-src`);
    }
    // And nothing beyond what the providers actually need: a leftover origin is a host that can
    // still frame us after the provider using it was removed.
    for (const origin of allowed) {
      assert.ok(
        embedFrameOrigins().includes(origin),
        `frame-src allows ${origin}, which no provider uses`,
      );
    }
  }
});

test("every provider builds addresses only on its own origin, whatever it is handed", () => {
  // The one invariant a provider owns: nothing a member wrote may steer where the frame points.
  const hostile = [
    "../../evil", "..%2f..%2fevil", "a/b", "a?b", "a#b", "a%00b", "javascript:alert(1)",
    "//evil.example", "\\evil", "a b", "'\"><script>", "	", "é".repeat(20),
  ];
  for (const provider of EMBED_PROVIDERS) {
    for (const raw of hostile) {
      // Feed the junk everywhere a provider reads from: as a whole link, and inside a plausible
      // one for that provider's own hosts.
      for (const candidate of [raw, ...linkShapes(provider.id, raw)]) {
        const ref = provider.parse(candidate);
        if (!ref) continue;
        for (const built of [provider.frameUrl(ref), provider.pageUrl(ref)]) {
          const url = new URL(built);
          assert.ok(
            `${url.protocol}//${url.host}` === provider.origin ||
              `${url.protocol}//${url.host}` === new URL(provider.pageUrl(ref)).origin,
            `${provider.id} built ${built} from ${candidate}`,
          );
          assert.doesNotMatch(url.pathname, /\/\.\.(\/|$)/, `${built} traverses`);
        }
      }
    }
  }
});

test("a card's key separates entities that are genuinely different things", () => {
  const keys = new Set<string>();
  for (const link of [
    "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
    "https://open.spotify.com/album/4cOdK2wGLETKBW3PvgPWqT", // same id, different kind
    "https://youtu.be/dQw4w9WgXcQ",
    "https://youtu.be/dQw4w9WgXcQ?t=90", // same video, different moment
    "https://soundcloud.com/artist/track",
    "https://soundcloud.com/artist/sets/track", // same last segment, different thing
    "https://vimeo.com/123456789",
    "https://vimeo.com/123456789/abcdef0123", // unlisted: a different address
    "https://www.mixcloud.com/user/show",
    "https://music.apple.com/us/album/name/123456789",
    "https://music.apple.com/us/album/name/123456789?i=987654321", // one song off the record
  ]) {
    const embed = chatEmbedFor(link);
    assert.ok(embed, `${link} should be claimed`);
    const key = embedKey(embed!);
    assert.ok(!keys.has(key), `${link} collides with another entity on ${key}`);
    keys.add(key);
  }
});

// --- the shared parsing helpers ------------------------------------------------------------------

test("a path segment cannot escape the part of an address it is written into", () => {
  for (const good of ["track", "a-b_c.d", "123", "did.plc"]) assert.equal(segment(good), good);
  for (const bad of ["", "..", ".", "...", "a/b", "a?b", "a#b", "a%2Fb", "a b", "a\\b", "é"]) {
    assert.equal(segment(bad), "", `${JSON.stringify(bad)} must not pass`);
  }
  assert.equal(segment("x".repeat(129)), "", "bounded");
  assert.equal(path(["a", "b"]), "a/b");
  assert.equal(path(["a", ".."]), "", "one bad segment poisons the whole path");
  assert.equal(path([]), "");
});

test("the host check is exact, because every loose form of it hands a frame to somebody else", () => {
  const hosts = new Set(["example.com"]);
  assert.ok(webUrl("https://example.com/a", hosts));
  assert.ok(webUrl("HTTPS://EXAMPLE.COM/a", hosts), "case is not a different host");
  for (const bad of [
    "https://example.com.evil.invalid/a", // startsWith
    "https://notexample.com/a", // endsWith
    "https://evil.invalid/example.com", // includes
    "javascript:alert(1)",
    "file:///example.com",
    "",
  ]) {
    assert.equal(webUrl(bad, hosts), null, `${bad} must not pass`);
  }
});

// --- SoundCloud ----------------------------------------------------------------------------------

test("SoundCloud claims tracks and sets, and declines profiles and listings", () => {
  assert.deepEqual(SOUNDCLOUD.parse("https://soundcloud.com/artist/a-track"), {
    id: "artist/a-track", kind: "", extra: "", start: 0,
  });
  assert.equal(SOUNDCLOUD.parse("https://soundcloud.com/artist/sets/a-list")?.kind, "sets");
  assert.equal(SOUNDCLOUD.parse("https://m.soundcloud.com/artist/a-track")?.id, "artist/a-track");
  assert.equal(SOUNDCLOUD.parse("https://soundcloud.com/artist/a-track?t=90")?.start, 90);
  assert.equal(SOUNDCLOUD.parse("https://soundcloud.com/artist/a-track?t=1m30s")?.start, 90);

  // A profile link would render that artist's whole catalogue, which is not what was offered.
  assert.equal(SOUNDCLOUD.parse("https://soundcloud.com/artist"), null);
  assert.equal(SOUNDCLOUD.parse("https://soundcloud.com/discover/sets/x"), null);
  assert.equal(SOUNDCLOUD.parse("https://soundcloud.com/a/b/c/d"), null);
});

test("SoundCloud percent-encodes the track address it passes as a parameter", () => {
  const ref = SOUNDCLOUD.parse("https://soundcloud.com/artist/a-track")!;
  const url = new URL(SOUNDCLOUD.frameUrl(ref));
  assert.equal(url.origin, "https://w.soundcloud.com");
  // The widget takes the page address as a query parameter, so the escaping moves from the path
  // alphabet to the encoder. Both apply: the id was validated segment by segment on the way in.
  assert.equal(url.searchParams.get("url"), "https://soundcloud.com/artist/a-track");
  assert.ok(SOUNDCLOUD.frameUrl(ref).includes("%2F"), "the slashes are encoded, not literal");
  assert.equal(SOUNDCLOUD.deck, true, "its widget answers seek and reports position");
});

// --- Vimeo ---------------------------------------------------------------------------------------

test("Vimeo keeps the privacy hash an unlisted video cannot play without", () => {
  assert.equal(VIMEO.parse("https://vimeo.com/123456789")?.id, "123456789");
  assert.equal(VIMEO.parse("https://player.vimeo.com/video/123456789")?.id, "123456789");
  assert.equal(VIMEO.parse("https://vimeo.com/channels/staffpicks/123456789")?.id, "123456789");

  // Both spellings of the unlisted form have to reach the same ref, or the frame gets a refusal
  // instead of the video.
  const inPath = VIMEO.parse("https://vimeo.com/123456789/abcdef0123")!;
  const inQuery = VIMEO.parse("https://player.vimeo.com/video/123456789?h=abcdef0123")!;
  assert.equal(inPath.extra, "abcdef0123");
  assert.deepEqual(inPath, inQuery);
  assert.equal(new URL(VIMEO.frameUrl(inPath)).searchParams.get("h"), "abcdef0123");
  assert.equal(new URL(VIMEO.frameUrl(VIMEO.parse("https://vimeo.com/123456789")!)).searchParams.get("h"), null);

  assert.equal(VIMEO.parse("https://vimeo.com/user/settings"), null);
  assert.equal(VIMEO.parse("https://vimeo.com/12345"), null, "too short to be an id");
  assert.equal(VIMEO.deck, true);
});

// --- Mixcloud ------------------------------------------------------------------------------------

test("Mixcloud addresses a show by its feed path", () => {
  const ref = MIXCLOUD.parse("https://www.mixcloud.com/someone/a-show/")!;
  assert.equal(ref.id, "someone/a-show");
  const url = new URL(MIXCLOUD.frameUrl(ref));
  assert.equal(url.origin, "https://player.mixcloud.com");
  // The feed wants its surrounding slashes, encoded as part of the value.
  assert.equal(url.searchParams.get("feed"), "/someone/a-show/");
  assert.equal(MIXCLOUD.parse("https://www.mixcloud.com/someone"), null, "a profile is not a mix");
  assert.equal(MIXCLOUD.parse("https://www.mixcloud.com/discover/x"), null);
});

// --- Apple Music ---------------------------------------------------------------------------------

test("Apple Music keeps the storefront, and tells a song apart from the record it is on", () => {
  const album = APPLE_MUSIC.parse("https://music.apple.com/gb/album/some-record/123456789")!;
  assert.equal(album.id, "123456789");
  assert.equal(album.kind, "album");
  assert.equal(new URL(APPLE_MUSIC.frameUrl(album)).pathname, "/gb/album/some-record/123456789");

  // `?i=` is the difference between "this track" and "the record it is on", so it has to survive
  // into the frame or the card plays the wrong thing.
  const song = APPLE_MUSIC.parse("https://music.apple.com/us/album/some-record/123456789?i=987654321")!;
  assert.equal(new URL(APPLE_MUSIC.frameUrl(song)).searchParams.get("i"), "987654321");
  assert.notEqual(embedKey({ provider: APPLE_MUSIC, ref: album }), embedKey({ provider: APPLE_MUSIC, ref: song }));
  assert.ok(APPLE_MUSIC.height(song) < APPLE_MUSIC.height(album), "a single song is the compact player");

  assert.equal(APPLE_MUSIC.parse("https://music.apple.com/browse"), null);
  assert.equal(APPLE_MUSIC.parse("https://music.apple.com/gb/nonsense/x/1"), null);
  assert.equal(APPLE_MUSIC.parse("https://music.apple.com/toolong/album/x/1"), null, "a storefront is two letters");
});

// --- Bluesky -------------------------------------------------------------------------------------

test("Bluesky claims a DID link and deliberately leaves a handle link alone", () => {
  const did = "did:plc:u5cwb2mwiv2bfq53cjufe6yn";
  const ref = BLUESKY.parse(`https://bsky.app/profile/${did}/post/3k4duaz5vfs2b`)!;
  assert.equal(ref.extra, did);
  assert.equal(ref.id, "3k4duaz5vfs2b");
  assert.equal(
    BLUESKY.frameUrl(ref),
    `https://embed.bsky.app/embed/${did}/app.bsky.feed.post/3k4duaz5vfs2b`,
  );
  // The colons in a DID are legal in a path segment and must NOT be encoded, or the embed cannot
  // find the record. Safe unencoded because the shape was pinned before it got here.
  assert.ok(!BLUESKY.frameUrl(ref).includes("%3A"));

  // The frame takes a DID and rejects a handle, and turning a handle into one is a network request
  // this must not make for a link that merely scrolled past. So a handle link stays a plain link
  // rather than becoming a card that cannot load.
  assert.equal(BLUESKY.parse("https://bsky.app/profile/alice.bsky.social/post/3k4duaz5vfs2b"), null);
  assert.equal(BLUESKY.parse("https://bsky.app/profile/did:web:example.com/post/3k4duaz5vfs2b"), null);
  assert.equal(BLUESKY.parse(`https://bsky.app/profile/${did}`), null, "a profile is not a post");
});

/** The hosts a provider actually claims, discovered by asking it rather than by a second list. */
function providerHosts(id: string): string[] {
  const by: Record<string, string[]> = {
    spotify: ["open.spotify.com"],
    youtube: ["youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com", "youtu.be", "youtube-nocookie.com"],
    soundcloud: ["soundcloud.com", "www.soundcloud.com", "m.soundcloud.com"],
    vimeo: ["vimeo.com", "www.vimeo.com", "player.vimeo.com"],
    mixcloud: ["mixcloud.com", "www.mixcloud.com", "m.mixcloud.com"],
    applemusic: ["music.apple.com", "embed.music.apple.com", "geo.music.apple.com"],
    bluesky: ["bsky.app", "www.bsky.app", "staging.bsky.app"],
  };
  return by[id] ?? [];
}

/** Plausible links for one provider with `raw` spliced into each position it reads from. */
function linkShapes(id: string, raw: string): string[] {
  const host = providerHosts(id)[0] ?? "example.com";
  return [
    `https://${host}/${raw}`,
    `https://${host}/${raw}/${raw}`,
    `https://${host}/album/${raw}/${raw}`,
    `https://${host}/watch?v=${raw}`,
    `https://${host}/profile/${raw}/post/${raw}`,
    `https://${host}/video/${raw}?h=${raw}`,
  ];
}
