import assert from "node:assert/strict";
import test from "node:test";
import {
  DRIFT_HOLD_S,
  DRIFT_SEEK_S,
  deckAdvance,
  deckPosition,
  deckSurface,
  driftAction,
  mediaChoices,
  fetchPhase,
  mediaKind,
  mediaOrigin,
  mediaUrl,
  nudgeRate,
  isStalled,
  playableQueue,
  queueDigest,
  resolveCallName,
  stallChip,
  jukeClaimWins,
  nextJukeSeq,
  validJukeSeq,
  HAVE_FUTURE_DATA,
  MAX_JUKE_SEQ,
  STALL_ANNOUNCE_MS,
  type JukeEntry,
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

const WINDOWS_UA =
  "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36 Edg/140.0.0.0";
const MAC_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
const LINUX_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const ANDROID_UA =
  "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36";

test("a media url names the server and the content address", () => {
  assert.equal(mediaUrl(7, "abc123", MAC_UA), "catcoms-media://a/7/abc123");
});

test("a media url escapes anything that would change its path shape", () => {
  // The cid reaching this function is already validated, but the backend refuses anything that is
  // not exactly <server>/<64 hex>, so a smuggled separator must not silently become one.
  assert.equal(mediaUrl(1, "a/b", MAC_UA), "catcoms-media://a/1/a%2Fb");
  assert.equal(mediaUrl(1, "../x", MAC_UA), "catcoms-media://a/1/..%2Fx");
  assert.equal(mediaUrl(1, "a/b", WINDOWS_UA), "http://catcoms-media.localhost/1/a%2Fb");
});

// --- The platform the deck plays on ------------------------------------------------------------

test("Windows plays through the http host, because it has no custom scheme to play through", () => {
  // The regression that broke the whole jukebox on Windows: WebView2 has no custom schemes, so
  // the toolkit intercepts `http://<scheme>.localhost/...` and NOTHING else. A catcoms-media://
  // request there reaches no handler at all, the element errors, and the deck reads every track
  // in the queue as one nobody will serve.
  assert.equal(mediaUrl(7, "abc123", WINDOWS_UA), "http://catcoms-media.localhost/7/abc123");
  assert.ok(
    !mediaUrl(7, "abc123", WINDOWS_UA).startsWith("catcoms-media:"),
    "the scheme form never reaches a Windows webview",
  );
});

test("Android is Windows-shaped here, even though its user agent says Linux", () => {
  // Android has no custom-scheme API either and uses the same work-around, and its UA contains
  // "Linux": matching on Linux first would send it to a scheme that does not exist there.
  assert.equal(mediaOrigin(ANDROID_UA), "http://catcoms-media.localhost");
});

test("macOS and Linux keep the real scheme, which is what their webviews register", () => {
  assert.equal(mediaOrigin(MAC_UA), "catcoms-media://a");
  assert.equal(mediaOrigin(LINUX_UA), "catcoms-media://a");
});

test("every platform's url keeps the two path segments the backend routes on", () => {
  // The backend parses `/<server>/<cid>` and ignores the host, so the split has to survive both
  // forms. This is also why Tauri's own convertFileSrc cannot be used: it would encode the whole
  // path as one segment.
  for (const ua of [WINDOWS_UA, MAC_UA, LINUX_UA, ANDROID_UA]) {
    const cid = "de".repeat(32);
    const path = mediaUrl(12, cid, ua).slice(mediaOrigin(ua).length);
    assert.equal(path, `/12/${cid}`, ua);
  }
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

test("a non-media mime still falls through to the extension, and that is safe", () => {
  // The name of this test used to claim the opposite of what it asserted. The behaviour is that
  // a mime naming neither audio nor video is treated as "no useful mime" and the extension
  // decides, which is what makes queue entries work at all (they carry no mime).
  assert.equal(mediaKind("payload.mp4", "text/html"), "video");
  // It is safe because kind only decides which surface to draw. The backend independently
  // refuses to serve a non-media declared type as one (safe_media_mime hands back
  // application/octet-stream), so such a file fails to decode and the deck skips it rather than
  // the webview being handed something it might treat as script.
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

// --- The deck's own clock ----------------------------------------------------------------------

const clock = (over: Partial<Parameters<typeof deckPosition>[0]> = {}) => ({
  isDj: false,
  paused: false,
  stale: false,
  off: 0,
  since: 0,
  element: null,
  ...over,
});

test("the DJ reads its own element, never a wall clock", () => {
  // The bug this exists for: the DJ projected `off + elapsed` like a listener, so five seconds
  // spent waiting for the first bytes counted as five seconds of playback. The deck then seeked
  // itself to a place it had never played, told the room to go there, and did it again at the
  // next ping, so a track that took a moment to arrive never got going at all.
  const pos = deckPosition(
    clock({ isDj: true, off: 0, since: 5000, element: { currentTime: 1.2, readyState: 4 } }),
  );
  assert.equal(pos, 1.2);
});

test("the DJ never corrects itself, whatever the stall did to the clock", () => {
  // The same thing said as the player says it: drift against your own element is always zero, so
  // the action is always "hold". A DJ that could reach "seek" here is a DJ that seeks every ping.
  const element = { currentTime: 8, readyState: 4 };
  const drift =
    deckPosition(clock({ isDj: true, off: 0, since: 30_000, element })) - element.currentTime;
  assert.equal(drift, 0);
  assert.equal(driftAction(drift, "audio"), "hold");
  assert.equal(driftAction(drift, "video"), "hold");
});

test("a DJ whose element has nothing loaded holds at the offset it pressed", () => {
  // Pressing play on a track that takes three seconds to arrive must still start it at the top:
  // an aged offset here is how the deck used to eat the head of every track it had to fetch.
  assert.equal(deckPosition(clock({ isDj: true, off: 0, since: 3000, element: null })), 0);
  assert.equal(
    deckPosition(clock({ isDj: true, off: 0, since: 3000, element: { currentTime: 0, readyState: 0 } })),
    0,
    "metadata-less is as good as absent: currentTime means nothing yet",
  );
  // And a resume names where it resumed from, not zero.
  assert.equal(deckPosition(clock({ isDj: true, off: 42, since: 3000, element: null })), 42);
});

test("the DJ's paused position is still the element's", () => {
  assert.equal(
    deckPosition(clock({ isDj: true, paused: true, off: 0, element: { currentTime: 12.5, readyState: 3 } })),
    12.5,
  );
});

test("a listener ages the DJ's offset on its own clock", () => {
  // Nobody has to agree on the time of day: the offset is the DJ's, the elapsed time is local.
  assert.equal(deckPosition(clock({ off: 10, since: 2500 })), 12.5);
  assert.equal(
    deckPosition(clock({ off: 10, since: 2500, element: { currentTime: 3, readyState: 4 } })),
    12.5,
    "a listener's element is chasing the room, so it is not the thing to read",
  );
});

test("a paused or stale deck is frozen where it got to", () => {
  assert.equal(deckPosition(clock({ off: 10, since: 9000, paused: true })), 10);
  assert.equal(deckPosition(clock({ off: 10, since: 9000, stale: true })), 10);
});

// --- The queue is a playlist, not a library ------------------------------------------------------

const entry = (over: Partial<JukeEntry> = {}): JukeEntry => ({
  id: "e1",
  cid: "c1",
  name: "Track",
  author: "a1",
  added_ms: 1,
  ...over,
});

const queue = [
  entry({ id: "b", cid: "c2", added_ms: 2 }),
  entry({ id: "a", cid: "c1", added_ms: 1 }),
  entry({ id: "c", cid: "c3", added_ms: 3 }),
];

test("the queue plays in the order it was built, with the id as the tiebreak", () => {
  assert.deepEqual(playableQueue(queue, new Set()).map((e) => e.id), ["a", "b", "c"]);
  const tied = [entry({ id: "z", added_ms: 5 }), entry({ id: "y", added_ms: 5 })];
  assert.deepEqual(
    playableQueue(tied, new Set()).map((e) => e.id),
    ["y", "z"],
    "every machine has to reach the same order or the room fans out",
  );
});

test("the playable order drops what nobody would serve and leaves the queue alone", () => {
  const before = queue.map((e) => e.id);
  assert.deepEqual(playableQueue(queue, new Set(["c2"])).map((e) => e.id), ["a", "c"]);
  assert.deepEqual(queue.map((e) => e.id), before, "sorting must not reorder the caller's array");
});

test("a track the room has heard comes off the queue", () => {
  // The queue is consumed as it plays: the transport only moves forwards, so an entry left behind
  // the play head can never be reached again.
  const list = playableQueue(queue, new Set());
  const { next, drop } = deckAdvance(list, "a", true);
  assert.equal(next?.id, "b");
  assert.equal(drop, "a");
});

test("the last track empties the queue and stops the room", () => {
  const list = playableQueue(queue, new Set());
  const { next, drop } = deckAdvance(list, "c", true);
  assert.equal(next, null, "nothing follows the last one: the deck goes idle");
  assert.equal(drop, "c", "and the last one is still spent");
});

test("playing the queue through drains it exactly once", () => {
  let list = playableQueue(queue, new Set());
  let current = list[0]!.id;
  const heard: string[] = [];
  for (let guard = 0; guard < 10 && current; guard += 1) {
    const { next, drop } = deckAdvance(list, current, true);
    heard.push(drop);
    list = list.filter((e) => e.id !== drop);
    current = next?.id ?? "";
  }
  assert.deepEqual(heard, ["a", "b", "c"]);
  assert.deepEqual(list, [], "an empty deck at the end, with nothing left to replay");
});

test("a track the deck gave up on stays queued", () => {
  // Nobody heard it, and whoever holds the file may come back, so it is not spent: only the
  // session-local blacklist keeps the deck off it.
  const list = playableQueue(queue, new Set());
  const { next, drop } = deckAdvance(list, "b", false);
  assert.equal(next?.id, "c");
  assert.equal(drop, "", "an unplayable track is not a played track");
});

test("an idle deck starts at the top of the queue and drops nothing", () => {
  const list = playableQueue(queue, new Set());
  assert.deepEqual(deckAdvance(list, "", true), { next: list[0], drop: "" });
  assert.deepEqual(
    deckAdvance(list, "gone", true),
    { next: list[0], drop: "" },
    "a current entry that is no longer in the list cannot be dropped twice",
  );
});

test("an empty queue advances to nothing rather than throwing", () => {
  assert.deepEqual(deckAdvance([], "", true), { next: null, drop: "" });
});

// --- Video: what the picker offers, and where the picture goes -----------------------------------

const share = [
  { cid: "c1", name: "Opening Theme.flac", mime: "audio/flac", path: "" },
  { cid: "c2", name: "Holiday.mkv", mime: "", path: "clips" },
  { cid: "c1", name: "Opening Theme.flac", mime: "audio/flac", path: "wiki/Lmao!" },
  { cid: "c3", name: "notes.txt", mime: "text/plain", path: "" },
  { cid: "c2", name: "Holiday.mkv", mime: "", path: "wiki/Lmao!" },
];

test("one piece of content is offered once, however many times the share lists it", () => {
  // The regression, and it was fatal rather than cosmetic: a file index is append-only and
  // add_file re-lists content it already holds, so the same cid turns up under two folders (or
  // twice under one after a concurrent add). A keyed list that assumed cid+path was unique took
  // the whole app down with each_key_duplicate as soon as a share held a double listing.
  const offered = mediaChoices(share);
  assert.deepEqual(offered.map((f) => f.cid), ["c1", "c2"]);
  assert.equal(new Set(offered.map((f) => f.cid)).size, offered.length, "cid is a usable key");
});

test("the picker offers only what the deck can play", () => {
  assert.ok(!mediaChoices(share).some((f) => f.name.endsWith(".txt")));
});

test("the kind toggle narrows to one kind, extension-only entries included", () => {
  // A queue entry carries no mime and neither does a plain share listing, so the video here is
  // recognised by its extension alone: filtering must go through the same rule playback does.
  assert.deepEqual(mediaChoices(share, "audio").map((f) => f.cid), ["c1"]);
  assert.deepEqual(mediaChoices(share, "video").map((f) => f.cid), ["c2"]);
  assert.deepEqual(mediaChoices(share, "all").map((f) => f.cid), ["c1", "c2"]);
  assert.deepEqual(mediaChoices([], "video"), []);
});

test("the picker keeps the share's own order", () => {
  const flipped = [share[1]!, share[0]!];
  assert.deepEqual(mediaChoices(flipped).map((f) => f.cid), ["c2", "c1"]);
});

test("a film has exactly one home, and audio has none", () => {
  // One element, adopted by re-parenting: two claims would leave one surface showing a black box
  // and the other losing the frame on the next render.
  assert.equal(deckSurface("video", true, true, true), "focus", "focus outranks the dock");
  assert.equal(deckSurface("video", true, true, false), "focus", "even with the dock folded");
  assert.equal(deckSurface("video", true, false, true), "dock", "otherwise the deck's own screen");
  assert.equal(deckSurface("video", true, false, false), "none", "folded is one line by definition");
  // Audio is heard from the hidden element in the body: a surface for it would be a black box.
  assert.equal(deckSurface("audio", true, true, true), "none");
  assert.equal(deckSurface("other", true, true, true), "none");
  // And nothing playing claims nothing.
  assert.equal(deckSurface("video", false, true, true), "none");
});

test("no pair of states ever hands the film to two surfaces at once", () => {
  for (const kind of ["audio", "video", "other"] as const) {
    for (const playing of [true, false]) {
      for (const focus of [true, false]) {
        for (const dock of [true, false]) {
          const where = deckSurface(kind, playing, focus, dock);
          assert.ok(
            where === "focus" || where === "dock" || where === "none",
            `${kind}/${playing}/${focus}/${dock}`,
          );
        }
      }
    }
  }
});

test("a video track is transported exactly like an audio one", () => {
  // The picture is the only difference. Kind changes how drift is corrected (a snap is visible
  // where it is inaudible) and where the element is hosted, and nothing else: same queue, same
  // clock, same advance.
  const list = playableQueue(
    [
      entry({ id: "v1", cid: "c2", name: "Holiday.mkv", added_ms: 1 }),
      entry({ id: "a1", cid: "c1", name: "Opening Theme.flac", added_ms: 2 }),
    ],
    new Set(),
  );
  assert.deepEqual(deckAdvance(list, "v1", true), { next: list[1], drop: "v1" });
  assert.equal(
    deckPosition(clock({ isDj: true, off: 0, since: 4000, element: { currentTime: 2, readyState: 4 } })),
    2,
  );
  // Video eases rather than snaps, which is the one place the kind reaches the transport.
  assert.equal(driftAction(1, "video"), "nudge");
  assert.equal(driftAction(1, "audio"), "hold");
});

// --- Regressions ------------------------------------------------------------------------------

// --- Buffering: what the chip is allowed to say --------------------------------------------------

test("a chunk boundary never raises the chip, however long the deadline waits", () => {
  // `waiting` fires at every chunk boundary and every seek of a perfectly healthy stream. The
  // element's readyState is the arbiter, so a deck that can keep playing never claims otherwise.
  const fine = { readyState: HAVE_FUTURE_DATA, paused: false };
  assert.equal(stallChip(false, "waiting", fine), false);
  assert.equal(stallChip(false, "deadline", fine), false);
});

test("only the deadline speaks, and only about a starved element", () => {
  const starved = { readyState: 1, paused: false };
  assert.equal(stallChip(false, "waiting", starved), false, "the wait alone says nothing");
  assert.equal(stallChip(false, "deadline", starved), true, "a real starve is announced");
});

test("any progress clears the chip immediately", () => {
  // Whatever the last `waiting` claimed: playing, canplay, a moved currentTime and a landed seek
  // are all proof the deck is not dry.
  const starved = { readyState: 0, paused: false };
  assert.equal(stallChip(true, "progress", starved), false);
  assert.equal(stallChip(true, "progress", { readyState: 4, paused: false }), false);
});

test("an announced stall stays up while it is still starved", () => {
  const starved = { readyState: 1, paused: false };
  assert.equal(stallChip(true, "waiting", starved), true, "a second waiting does not clear it");
  assert.equal(stallChip(true, "deadline", starved), true);
});

test("pausing takes the chip down rather than leaving it up forever", () => {
  // A deck paused on an unbuffered track is not waiting for anything.
  assert.equal(stallChip(true, "deadline", { readyState: 0, paused: true }), false);
});

test("buffering is possible on a file this device holds in full", () => {
  // Worth pinning as a fact, because it reads as a contradiction: the deck streams out of the
  // vault a window at a time and every window is a sealed chunk the single-threaded actor has to
  // open first. "Local" is not "instant", so the chip's rules must depend on the element's own
  // readyState and never on where the bytes were going to come from.
  const openingAChunk = { readyState: 1, paused: false };
  assert.equal(stallChip(false, "deadline", openingAChunk), true);
  assert.equal(stallChip(true, "progress", openingAChunk), false, "and it clears on the bytes");
});

test("a chunk boundary is not a stall", () => {
  // `waiting` fires constantly while streaming. Announcing it raw made a playing track claim it
  // had run dry; the element's own readyState is the arbiter.
  assert.equal(isStalled({ readyState: HAVE_FUTURE_DATA, paused: false }), false);
  assert.equal(isStalled({ readyState: 4, paused: false }), false);
});

test("a genuine stall is still reported", () => {
  assert.equal(isStalled({ readyState: 0, paused: false }), true);
  assert.equal(isStalled({ readyState: 2, paused: false }), true);
});

test("a paused deck is not buffering, however little it has loaded", () => {
  // Otherwise pausing on an unbuffered track sits there claiming to be stalled forever.
  assert.equal(isStalled({ readyState: 0, paused: true }), false);
});

test("the stall announcement waits long enough for a person to notice", () => {
  assert.ok(STALL_ANNOUNCE_MS >= 1000, "shorter than this and ordinary streaming trips it");
});

test("a call name resolves against the room's server, not the one being viewed", () => {
  // The regression: switching servers mid-call renamed every participant, including you.
  const inRoom = { a1: { name: "Sillycats" } };
  const inView = { a1: { name: "someone else entirely" } };
  assert.equal(resolveCallName("a1", inRoom, inView), "Sillycats");
});

test("the viewed server is a fallback only for a peer the room does not know", () => {
  assert.equal(resolveCallName("b2", {}, { b2: { name: "Known Elsewhere" } }), "Known Elsewhere");
});

test("an unknown peer renders as its fingerprint rather than a wrong name", () => {
  assert.equal(resolveCallName("c3", {}, {}), "c3");
  // A blank or whitespace-only profile name must not render as an empty label.
  assert.equal(resolveCallName("c3", { c3: { name: "   " } }, {}), "c3");
});

test("a companion device renders under its origin's name from the room's server", () => {
  const inRoom = { origin1: { name: "Bam" } };
  assert.equal(
    resolveCallName("dev9", inRoom, {}, (fp) => (fp === "dev9" ? "origin1" : undefined)),
    "Bam",
  );
});

test("a transport revision the deck cannot count past is refused", () => {
  assert.equal(validJukeSeq(0), true);
  assert.equal(validJukeSeq(7), true);
  assert.equal(validJukeSeq(MAX_JUKE_SEQ), true);
  // The regression: `1e308` passes `Number.isInteger`, survives JSON, and satisfies
  // `1e308 + 1 === 1e308`, so adopting it left no press able to outrank it.
  assert.equal(validJukeSeq(1e308), false);
  assert.equal(1e308 + 1 === 1e308, true); // why the bound has to exist at all
  assert.equal(validJukeSeq(Number.MAX_SAFE_INTEGER), false);
  assert.equal(validJukeSeq(Number.MAX_SAFE_INTEGER + 1), false);
  assert.equal(validJukeSeq(MAX_JUKE_SEQ + 1), false);
  assert.equal(validJukeSeq(-1), false);
  assert.equal(validJukeSeq(1.5), false);
  assert.equal(validJukeSeq(Number.NaN), false);
  assert.equal(validJukeSeq(Number.POSITIVE_INFINITY), false);
  assert.equal(validJukeSeq("3"), false);
  assert.equal(validJukeSeq(undefined), false);
});

test("a press always outranks what it has heard, and the bound holds", () => {
  assert.equal(nextJukeSeq(0, null), 1);
  assert.equal(nextJukeSeq(3, 9), 10); // someone else's press is what I have to beat
  assert.equal(nextJukeSeq(9, 3), 10);
  // An unusable value on either side is treated as nothing heard rather than as a ceiling.
  assert.equal(nextJukeSeq(1e308, null), 1);
  assert.equal(nextJukeSeq(0, 1e308), 1);
  assert.equal(nextJukeSeq(MAX_JUKE_SEQ, MAX_JUKE_SEQ), MAX_JUKE_SEQ);
});

test("the deck goes to the newest press, with a stable tiebreak", () => {
  assert.equal(jukeClaimWins(null, { seq: 0, fromFp: "a" }), true);
  assert.equal(jukeClaimWins({ seq: 4, fromFp: "b" }, { seq: 5, fromFp: "a" }), true);
  assert.equal(jukeClaimWins({ seq: 5, fromFp: "b" }, { seq: 4, fromFp: "a" }), false);
  // Two people pressing at the same moment must resolve identically on every machine.
  assert.equal(jukeClaimWins({ seq: 5, fromFp: "b" }, { seq: 5, fromFp: "c" }), true);
  assert.equal(jukeClaimWins({ seq: 5, fromFp: "c" }, { seq: 5, fromFp: "b" }), false);
  // The DJ's own five-second re-announce is not newer, but it keeps the deck alive.
  assert.equal(jukeClaimWins({ seq: 5, fromFp: "b" }, { seq: 5, fromFp: "b" }), true);
  // And a frame nobody can order is never adopted, whatever it claims.
  assert.equal(jukeClaimWins(null, { seq: 1e308, fromFp: "z" }), false);
  assert.equal(jukeClaimWins({ seq: 2, fromFp: "a" }, { seq: 1e308, fromFp: "z" }), false);
});

// --- queue digests -----------------------------------------------------------------------------
//
// The failure this exists to make visible: a channel-updated event says the jukebox moved, the UI
// re-reads the queue, and the queue is identical. The event and the document disagree, and that
// used to look exactly like a queue that legitimately had not changed since the last look.

const queued = (id: string, over: Partial<JukeEntry> = {}): JukeEntry => ({
  id,
  cid: `cid-${id}`,
  name: `Track ${id}`,
  author: "741af9ff",
  added_ms: 1000,
  ...over,
});

test("the same queue digests the same, so 'nothing changed' is detectable", () => {
  const queue = [queued("a"), queued("b"), queued("c")];
  assert.equal(queueDigest(queue), queueDigest([queued("a"), queued("b"), queued("c")]));
});

test("adding, removing or reordering all change the digest", () => {
  const base = queueDigest([queued("a"), queued("b")]);
  assert.notEqual(queueDigest([queued("a"), queued("b"), queued("c")]), base, "added");
  assert.notEqual(queueDigest([queued("a")]), base, "removed");
  // A reorder is a change: whoever is next to play is different.
  assert.notEqual(queueDigest([queued("b"), queued("a")]), base, "reordered");
});

test("an empty queue reads as empty rather than as a hash nobody can interpret", () => {
  assert.equal(queueDigest([]), "empty");
});

/**
 * Names are user content and have no business in a value that ends up in a diagnostic record. The
 * ids already determine the queue completely.
 */
test("a digest ignores everything except the ids, in order", () => {
  const plain = queueDigest([queued("a"), queued("b")]);
  const renamed = queueDigest([
    queued("a", { name: "something private", cid: "different", author: "someone" }),
    queued("b", { name: "also private", added_ms: 99 }),
  ]);
  assert.equal(renamed, plain);
});

test("ids that concatenate to the same string still differ", () => {
  // Without a separator ["ab","c"] and ["a","bc"] would collide, and a reorder-shaped bug would
  // be invisible exactly when the ids happen to line up.
  assert.notEqual(queueDigest([queued("ab"), queued("c")]), queueDigest([queued("a"), queued("bc")]));
});

test("a digest is short enough to sit in a log line", () => {
  const long = Array.from({ length: 64 }, (_, n) => queued(`entry-number-${n}`));
  assert.ok(queueDigest(long).length <= 16, queueDigest(long));
});
