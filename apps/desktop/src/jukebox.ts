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
