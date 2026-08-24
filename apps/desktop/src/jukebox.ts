/**
 * Jukebox deck logic that does not need a DOM: what a shared file is, where its bytes come from,
 * and what to do when this listener's clock has drifted off the DJ's.
 *
 * The deck itself never moves media between peers. It broadcasts transport state (which entry,
 * what offset, playing or paused) and every listener plays its own local copy, so all of the
 * below is about reconciling one local element against a shared clock.
 */

/** What a queued file is, as far as the deck cares. */
export type MediaKind = "audio" | "video" | "other";

/** The custom scheme the webview plays shared media through. */
export const MEDIA_SCHEME = "catcoms-media";

/** The user agent, where there is one to read (a unit test runs without a webview). */
function ua(): string {
  return typeof navigator === "undefined" ? "" : navigator.userAgent;
}

/**
 * The origin a shared file is served from, which is not the same string on every platform.
 *
 * macOS and Linux register `catcoms-media:` in the webview as a real scheme, so a URL in that
 * scheme reaches the handler. Windows (and Android) cannot: WebView2 has no custom schemes, so
 * the toolkit instead intercepts `http://<scheme>.localhost/...` and nothing else. A
 * `catcoms-media://` request there is an unknown scheme, so it never reaches the backend at all:
 * the element fails to load, the deck reads that as a track nobody will serve, and every track in
 * the queue fails the same way the moment it is pressed. That is the whole of "the jukebox is
 * broken" on Windows, and no amount of backend correctness could have shown through it.
 *
 * The host segment is a constant placeholder either way: the backend routes on the path alone,
 * because the Windows form makes the host `catcoms-media.localhost` and it stops being ours to
 * choose.
 */
export function mediaOrigin(userAgent: string = ua()): string {
  return /Windows|Android/i.test(userAgent)
    ? `http://${MEDIA_SCHEME}.localhost`
    : `${MEDIA_SCHEME}://a`;
}

/**
 * The URL a media element loads a shared file from.
 *
 * This mirrors what Tauri's own `convertFileSrc` does per platform. It is not simply called
 * because that helper percent-encodes the whole path into one segment, and the backend routes on
 * `/<server>/<cid>` as two.
 */
export function mediaUrl(server: number, cid: string, userAgent: string = ua()): string {
  return `${mediaOrigin(userAgent)}/${server}/${encodeURIComponent(cid)}`;
}

const VIDEO_EXT = new Set(["mp4", "m4v", "webm", "mkv", "mov", "avi", "ogv"]);
const AUDIO_EXT = new Set(["mp3", "m4a", "aac", "ogg", "oga", "opus", "flac", "wav", "wma"]);

/**
 * Whether a queued file is audio or video.
 *
 * The declared MIME wins when it says something useful, because it is what the backend will
 * actually serve the bytes as. The extension is the fallback, and it is load-bearing rather than
 * belt-and-braces: a queue entry carries only `{id, cid, name, author, added_ms}`, with no MIME
 * at all, so a listener reading the queue has nothing else to go on.
 */
export function mediaKind(name: string, mime = ""): MediaKind {
  const type = mime.trim().toLowerCase().split("/")[0];
  if (type === "video") return "video";
  if (type === "audio") return "audio";
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  if (VIDEO_EXT.has(ext)) return "video";
  if (AUDIO_EXT.has(ext)) return "audio";
  return "other";
}

/** What the "add from share" picker is currently listing. */
export type MediaFilter = "all" | "audio" | "video";

/** The least a file has to say for the picker to place it. */
export type MediaChoice = { cid: string; name: string; mime: string };

/**
 * The share's media, as the picker offers it: the kinds asked for, each piece of content once.
 *
 * The de-duplication is the load-bearing half. A file index is an append-only list and `add_file`
 * re-lists content it already holds rather than storing it twice, so one piece of content can
 * appear several times over: in two folders, or twice in one folder after a concurrent double
 * add. The deck plays a content address, so those listings are all the same track to it, and a
 * keyed list that assumed otherwise crashed the whole app with `each_key_duplicate` the moment a
 * share held one. First listing wins, so the order the share reports is preserved.
 */
export function mediaChoices<T extends MediaChoice>(
  files: readonly T[],
  filter: MediaFilter = "all",
): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const f of files) {
    const kind = mediaKind(f.name, f.mime);
    if (kind === "other") continue;
    if (filter !== "all" && kind !== filter) continue;
    if (seen.has(f.cid)) continue;
    seen.add(f.cid);
    out.push(f);
  }
  return out;
}

/** Which surface holds the deck element: see [`deckSurface`]. */
export type DeckSurface = "focus" | "dock" | "none";

/**
 * Where a playing track's picture goes.
 *
 * There is exactly one deck element and the surfaces adopt it by re-parenting, so at most one of
 * them may claim it at a time: two claims means one surface shows an empty box and the other
 * steals the frame back on the next render. Hence one function rather than a condition written
 * twice.
 *
 * Audio never claims a surface (there is nothing to see, and the element is hidden in the body
 * where the room can still hear it), the focus view outranks the dock because it is the surface
 * asked for by name, and a folded dock is one line by definition: the picture is what the focus
 * view is for.
 */
export function deckSurface(
  kind: MediaKind,
  playing: boolean,
  focusOpen: boolean,
  dockOpen: boolean,
): DeckSurface {
  if (!playing || kind !== "video") return "none";
  if (focusOpen) return "focus";
  return dockOpen ? "dock" : "none";
}

/** Below this, a listener is close enough to the DJ that correcting would be worse than drifting. */
export const DRIFT_HOLD_S = 0.4;
/** Above this, easing back would take longer than the drift is old; snap instead. */
export const DRIFT_SEEK_S = 2;

/** What to do about the gap between this element and where the DJ says the room is. */
export type DriftAction = "hold" | "nudge" | "seek";

/**
 * Audio can hide a seek; video cannot. A snap in a shared film is jarring for everyone who was
 * already in sync, so a small video drift is eased out by playing slightly fast or slow instead,
 * and only a gap too large to ease is snapped. Audio keeps the old snap-or-nothing behaviour,
 * because a 300ms rate change is audible as a pitch bend where a 300ms seek is not.
 */
export function driftAction(drift: number, kind: MediaKind): DriftAction {
  const gap = Math.abs(drift);
  if (kind === "video") {
    if (gap < DRIFT_HOLD_S) return "hold";
    return gap > DRIFT_SEEK_S ? "seek" : "nudge";
  }
  return gap > DRIFT_SEEK_S ? "seek" : "hold";
}

/**
 * The playback rate that eases a drift out. Positive drift means the room is ahead of us, so we
 * play faster to catch up. Deliberately gentle: 5% is below the threshold where speech starts
 * sounding wrong, and the drift window it serves is at most two seconds.
 */
export function nudgeRate(drift: number): number {
  if (Math.abs(drift) < DRIFT_HOLD_S) return 1;
  return drift > 0 ? 1.05 : 0.95;
}

/** One entry in a channel's shared queue, as the backend hands it over. */
export type JukeEntry = {
  id: string;
  cid: string;
  name: string;
  author: string;
  added_ms: number;
};

/**
 * A short digest of a queue's contents, for noticing when a refresh changed nothing.
 *
 * The failure this exists to make visible: a `channel-updated` event arrives saying the jukebox
 * moved, the UI dutifully re-reads the queue, and the queue is identical. That means the event and
 * the document disagree, and until now it looked exactly like a queue that had legitimately not
 * changed since the last look. Comparing digests across a refresh separates them.
 *
 * Order matters, because a reorder is a change. Only the entry ids go in: names are user content
 * and have no business in a value that ends up in a diagnostic record, and the ids already
 * determine the queue completely.
 */
export function queueDigest(entries: readonly JukeEntry[]): string {
  if (!entries.length) return "empty";
  // FNV-1a over the ids in order. Not a cryptographic claim, just a cheap stable fingerprint that
  // is the same on both sides of a comparison and short enough to sit in a log line.
  let hash = 0x811c9dc5;
  for (const entry of entries) {
    for (let at = 0; at < entry.id.length; at += 1) {
      hash ^= entry.id.charCodeAt(at);
      hash = Math.imul(hash, 0x01000193) >>> 0;
    }
    hash ^= 0x2c; // a separator, so ["ab","c"] and ["a","bc"] differ
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return `${entries.length}:${hash.toString(16).padStart(8, "0")}`;
}

/**
 * The queue in the order the deck will play it: when it was added, with the id as the tiebreak so
 * two machines that received the same two adds in different orders still agree. Tracks nobody
 * would serve are dropped, so the deck does not stop on one.
 */
export function playableQueue(
  entries: readonly JukeEntry[],
  failed: ReadonlySet<string>,
): JukeEntry[] {
  return [...entries]
    .sort((a, b) => a.added_ms - b.added_ms || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0))
    .filter((e) => !failed.has(e.cid));
}

/**
 * What the deck does when it leaves a track: which entry plays next, and which entry comes off
 * the shared queue.
 *
 * The queue is a playlist, not a library. A track the room has heard is spent, so it is dropped
 * rather than left sitting above the play head where it can never be reached again (the transport
 * only ever moves forwards). `played` is false for a track the deck gave up on: nobody heard it,
 * and whoever holds the file may come back, so it stays queued for a later retry.
 *
 * `queue` is already the playable order. A `currentId` that is not in it (nothing playing, or a
 * track that just failed out of it) starts from the top, which is what makes pressing play on an
 * idle deck work.
 */
export function deckAdvance(
  queue: readonly JukeEntry[],
  currentId: string,
  played: boolean,
): { next: JukeEntry | null; drop: string } {
  const i = queue.findIndex((e) => e.id === currentId);
  return { next: queue[i + 1] ?? null, drop: played && i >= 0 ? currentId : "" };
}

/**
 * The largest transport revision the protocol will carry, well inside the range JavaScript can
 * still count in whole numbers.
 *
 * The revision decides who owns the deck: a press adopts `max(mine, heard) + 1`. Any peer-supplied
 * value that arithmetic cannot move past therefore freezes control wherever it happens to be. A
 * `Number.isInteger` check is not enough, because `1e308` passes it, survives JSON, and satisfies
 * `1e308 + 1 === 1e308`. Two to the fortieth is a trillion presses: unreachable by use, and a
 * decade of headroom below `Number.MAX_SAFE_INTEGER` for the increment to stay exact.
 *
 * This is only the arithmetic bound, and on its own it is not a defence: a peer can simply name
 * it. [`JUKE_SEQ_LEAD`] is the rule that decides whether a revision is believable at all, and
 * [`jukeSeqSpent`] is what happens if one gets here anyway.
 */
export const MAX_JUKE_SEQ = 2 ** 40;

/**
 * How far past the transport it is already following a deck will believe a peer.
 *
 * A revision is an assertion by whoever sent it, not a fact, and the deck has no way to check one.
 * Taking the highest number in the room on trust meant a single frame naming a huge revision owned
 * the deck for everybody, permanently: nobody could count past it and it outlived the sender
 * leaving the room. Bounding the jump keeps the counter near what this deck has actually watched
 * happen, so the worst a poisoned frame can do is take the deck once and the next honest press
 * takes it straight back.
 *
 * The bound is deliberately loose, because being wrong is not symmetric. Too tight and a deck that
 * missed a burst of presses (a partition, a laptop that slept) refuses the room it came back to
 * and follows nothing until the user leaves and rejoins. Sixty-five thousand presses is hours of
 * somebody leaning on skip, and still under a millionth of [`MAX_JUKE_SEQ`].
 */
export const JUKE_SEQ_LEAD = 2 ** 16;

/**
 * The highest revision a deck that is following nothing yet will believe.
 *
 * Somebody who has just walked in has no observation to measure a claim against, so the only
 * question left is whether a room could plausibly have counted this high. A million presses is
 * years of unbroken playback in a room that never once emptied; past that the number is somebody's
 * invention rather than a history.
 *
 * It has to be its own, larger bound: measuring an arrival against [`JUKE_SEQ_LEAD`] would lock a
 * long-running room out to everyone who did not happen to be there at the start.
 */
export const JUKE_SEQ_COLD_CEILING = 2 ** 20;

/** Is a peer-supplied transport revision one this deck can safely order and increment past? */
export function validJukeSeq(seq: unknown): seq is number {
  return typeof seq === "number" && Number.isSafeInteger(seq) && seq >= 0 && seq <= MAX_JUKE_SEQ;
}

/**
 * Is this a revision no later press could ever outrank?
 *
 * The deadlock the bounds above exist to prevent, written as a question the code can ask. If a
 * deck does end up holding a revision at the ceiling anyway (an older build adopted one, or a
 * bound here turns out to be wrong), the safe answer is not to clamp the next press to the same
 * number: that hands the deck to whoever put it there and never gives it back. A spent revision is
 * one nothing defends, so the press starts a fresh generation and every deck sheds the same claim.
 */
export function jukeSeqSpent(seq: number): boolean {
  return seq >= MAX_JUKE_SEQ;
}

/** The transport currently being followed, as far as deciding who wins is concerned. */
export type JukeClaim = { seq: number; fromFp: string };

/**
 * Should an incoming transport frame be adopted?
 *
 * "newer" is a higher revision, or the same revision from a higher fingerprint so two people
 * pressing at the same moment resolve identically on every machine. A frame that is not newer is
 * still adopted when it comes from the DJ already being followed: that is the five-second
 * re-announce, which keeps the deck alive and corrects drift.
 *
 * Newer is not enough on its own. The revision is a claim by a peer this deck has no reason to
 * trust, so it also has to be a number this deck could believe: within [`JUKE_SEQ_LEAD`] of the
 * transport already being followed, or under [`JUKE_SEQ_COLD_CEILING`] when nothing is.
 */
export function jukeClaimWins(current: JukeClaim | null, incoming: JukeClaim): boolean {
  if (!validJukeSeq(incoming.seq)) return false;
  // Following nothing, so there is no observation to measure against and the only question left is
  // whether a room could have counted this far. This is also how a late arrival picks up a room
  // that has been playing for hours, so the ceiling has to be generous.
  if (!current) return incoming.seq <= JUKE_SEQ_COLD_CEILING;
  // A claim nothing could outrank is not worth defending, because defending it is the deadlock.
  // Anything countable beats it, which is how a deck that reached the ceiling gets out again. Two
  // spent claims still settle by fingerprint, so every machine drops the same one rather than the
  // pair of them swapping DJ back and forth.
  const spent = jukeSeqSpent(current.seq);
  if (spent && !jukeSeqSpent(incoming.seq)) return true;
  if (!spent && incoming.seq > current.seq + JUKE_SEQ_LEAD) return false;
  if (incoming.seq > current.seq) return true;
  if (incoming.seq < current.seq) return false;
  return incoming.fromFp >= current.fromFp;
}

/**
 * The revision a press claims: one past the highest thing this deck can see, which is what makes a
 * press outrank everything it has heard.
 *
 * It does not clamp, and that is the point. Clamping to [`MAX_JUKE_SEQ`] meant that at the ceiling
 * every press produced the same number, the fingerprint tiebreak decided the room for good, and no
 * lower peer could reclaim the deck however long it waited. A floor with nowhere left to go is
 * discarded instead and the counting starts again from one, which [`jukeClaimWins`] takes over a
 * spent claim for exactly this reason.
 */
export function nextJukeSeq(mine: number, adopted: number | null): number {
  const floor = Math.max(validJukeSeq(mine) ? mine : 0, validJukeSeq(adopted) ? adopted : 0);
  return jukeSeqSpent(floor) ? 1 : floor + 1;
}

/** Everything the deck's position depends on. `element` is null unless it holds the live track. */
export type DeckClock = {
  /** Whether the transport being followed is my own press. */
  isDj: boolean;
  paused: boolean;
  /** The DJ went quiet: the deck is frozen where it got to. */
  stale: boolean;
  /** The offset the followed transport named. */
  off: number;
  /** Milliseconds measured locally since that offset was adopted. */
  since: number;
  element: { currentTime: number; readyState: number } | null;
};

/**
 * Where the deck is in the current track.
 *
 * The DJ's own element **is** the room's clock. Projecting a wall clock for the DJ as well was
 * what broke playback: the projection ran on while the element sat waiting for bytes, so the DJ
 * seeked itself forward to a place it had never played, announced that place to the room, and did
 * it again at the next ping. A track that took a moment to arrive could never get going, and
 * every pause/resume jumped forward by the accumulated gap. While the element has no track loaded
 * the DJ's position is simply the offset it pressed: a slow load must not eat the head of a track.
 *
 * A listener has no such element to read, because its element is chasing the DJ rather than
 * leading. For a listener the offset is somebody else's and ageing it locally is the only honest
 * way to use it, which is the whole reason nobody has to agree on the time of day.
 */
export function deckPosition(c: DeckClock): number {
  if (c.isDj) return c.element && c.element.readyState > 0 ? c.element.currentTime : c.off;
  if (c.paused || c.stale) return c.off;
  return c.off + c.since / 1000;
}

/** A `download-progress` event, as the backend emits it. */
export type ProgressEvent = {
  cid: string;
  done: number;
  total: number;
  bytes_done: number;
  bytes_total: number;
  network_bytes_done: number;
  provider?: string | null;
};

/** What the deck should say while a track is being made ready. */
export type FetchPhase = {
  /** `local` read every byte from this device; `network` needed at least one from a peer. */
  source: "local" | "network";
  /** 0..100, clamped; 0 when the total is not known yet. */
  percent: number;
  /** The peer serving it, when one is. */
  provider: string;
};

/**
 * Classify a progress event for the UI.
 *
 * The distinction is worth drawing because the two cases feel completely different to a user:
 * reading a held file off this disk is a progress bar that flies, and pulling one off a peer is a
 * progress bar that may not. `network_bytes_done` is the honest signal for that, since the
 * backend reads held chunks locally and only requests the ones it is missing. `provider` alone is
 * not enough: it names only the most recent chunk's source, so a file that was half held would
 * flicker between the two labels as the loop walked its chunks.
 */
export function fetchPhase(e: ProgressEvent): FetchPhase {
  const percent =
    e.bytes_total > 0
      ? Math.max(0, Math.min(100, Math.round((e.bytes_done / e.bytes_total) * 100)))
      : 0;
  return {
    source: e.network_bytes_done > 0 ? "network" : "local",
    percent,
    provider: e.provider ?? "",
  };
}

/**
 * Whether a media element that just fired `waiting` is genuinely stalled.
 *
 * `waiting` is not a problem report: it fires at every chunk boundary and every seek while
 * streaming, and clears again in milliseconds. Announcing it raw made an ordinary playing track
 * claim it had run dry. A stall is only worth saying when it has lasted long enough for a person
 * to notice AND the element still has nothing to play.
 *
 * `readyState` is the element's own verdict: HAVE_FUTURE_DATA (3) or better means it can keep
 * going, whatever the event said a moment ago.
 */
export const STALL_ANNOUNCE_MS = 1200;
export const HAVE_FUTURE_DATA = 3;

export function isStalled(state: { readyState: number; paused: boolean }): boolean {
  return !state.paused && state.readyState < HAVE_FUTURE_DATA;
}

/**
 * What moved the deck's buffering chip.
 *
 * `waiting` is the element saying it has run out for the moment, `progress` is any evidence to
 * the contrary (it started playing, it can play, its time moved), and `deadline` is
 * [`STALL_ANNOUNCE_MS`] having passed since the last `waiting` with nothing since.
 */
export type StallSignal = "waiting" | "progress" | "deadline";

/**
 * Whether the deck should be claiming it has run dry, after one signal.
 *
 * Worth stating as a rule rather than as three event handlers, because the interesting case is
 * the one that reads wrong: a track streaming perfectly well fires `waiting` at every chunk
 * boundary and every seek. Announcing that raw made an ordinary playing track claim it had
 * stalled, so the chip waits out the deadline and then asks the element, whose `readyState` is
 * the only witness that matters. Anything that proves progress clears the chip immediately,
 * whatever the last `waiting` claimed.
 *
 * Note what this cannot tell you: **local is not the same as instant.** The deck streams out of
 * the vault a window at a time, and every window is a sealed chunk that has to be opened by the
 * single-threaded server actor before a byte of it can be served. A file entirely on this disk
 * can still make a player wait, so a stall is never by itself proof that a peer is missing.
 */
export function stallChip(
  announced: boolean,
  signal: StallSignal,
  el: { readyState: number; paused: boolean },
): boolean {
  if (signal === "progress") return false;
  if (signal === "deadline") return isStalled(el);
  return announced; // `waiting` only starts the clock; the deadline is what speaks
}

/**
 * Resolve a display name for a call surface.
 *
 * The room's server wins, always. `profiles` is scoped to the server being *viewed* and is
 * replaced wholesale on every switch, so resolving a call participant through it renamed
 * everyone (and re-badged you) the moment you clicked another server in the rail, making the dock
 * read as though the call had moved with you. The viewed map is only a fallback for a peer the
 * room's map has not heard of yet, and a fingerprint is a better answer than a wrong name.
 */
export function resolveCallName(
  fp: string,
  callProfiles: Record<string, { name?: string } | undefined>,
  profiles: Record<string, { name?: string } | undefined>,
  deviceOrigin?: (fp: string) => string | undefined,
): string {
  const origin = deviceOrigin?.(fp);
  const found =
    callProfiles[fp] ?? (origin ? callProfiles[origin] : undefined) ?? profiles[fp];
  return found?.name?.trim() || fp;
}
