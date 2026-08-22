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

/**
 * The URL a media element loads a shared file from.
 *
 * The host segment is a constant placeholder: the backend routes on the path alone, because
 * Windows rewrites the whole thing to `http://catcoms-media.localhost/...` and the host stops
 * being ours to choose.
 */
export function mediaUrl(server: number, cid: string): string {
  return `${MEDIA_SCHEME}://a/${server}/${encodeURIComponent(cid)}`;
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
