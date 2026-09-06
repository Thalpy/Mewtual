/**
 * Driving a YouTube player frame from the jukebox transport.
 *
 * The deck's shape is unchanged by this: nothing is sent between peers, the DJ broadcasts what is
 * playing and where it is, and every listener runs its own player against that clock. A YouTube
 * track is a third backend for that same transport, alongside the media element and the jam-take
 * synth, and it is the awkward one because the player is in another origin's document. What can
 * be done to it is send it messages; what can be known about it is whatever it volunteers.
 *
 * ## Why the frame is driven directly rather than through Google's API script
 *
 * The documented way to control an embed is to load `youtube.com/iframe_api` and use the object
 * it defines. That would mean allowing third-party script in the app's own document, which is a
 * far larger concession than allowing a frame: script in this document can read the window it
 * runs in, and the CSP here (`script-src 'self'`) is one of the few things holding the untrusted
 * markup boundary up. So the frame is spoken to directly instead, in the same postMessage
 * protocol that script would have used on our behalf. The cost is that the protocol is not
 * formally specified, which is why everything read back is treated as untrusted and optional, and
 * why the deck can keep time with no replies at all (see `YouTubeClock`).
 *
 * ## What is trusted
 *
 * Nothing the frame says. It is a foreign document under somebody else's control, so a reply is
 * evidence and never an instruction: a reported position is used to correct drift, and a reported
 * state is used to notice the player refused to start. Neither can move the room's transport,
 * which only ever changes when a member presses something.
 */

/** The origin the frame is served from, and the only one commands are ever sent to. */
export const YT_ORIGIN = "https://www.youtube-nocookie.com";

/** The player states YouTube reports. Named because the numbers are unreadable in a condition. */
export const YT_UNSTARTED = -1;
export const YT_ENDED = 0;
export const YT_PLAYING = 1;
export const YT_PAUSED = 2;
export const YT_BUFFERING = 3;
export const YT_CUED = 5;

/**
 * The handshake that makes the frame talk back.
 *
 * An `enablejsapi` frame stays silent until something announces it is listening; after this it
 * volunteers state and position updates. Sent repeatedly while a track is loading, because the
 * frame ignores anything that arrives before its own player is constructed and there is no event
 * that says when that was.
 */
export function ytListen(): string {
  return JSON.stringify({ event: "listening", id: 1, channel: "widget" });
}

/** One player command, in the wire form the frame expects. */
export function ytCommand(func: string, args: readonly unknown[] = []): string {
  return JSON.stringify({ event: "command", func, args });
}

/** What a message from the frame told us. Every field is optional: it volunteers what it likes. */
export type YouTubeReport = {
  /** One of the `YT_*` states, when this message carried one. */
  state?: number;
  /** Seconds into the video, when this message carried one. */
  currentTime?: number;
  /** Total length in seconds, when known and non-zero. */
  duration?: number;
};

/**
 * Read a message from the player frame, or `null` if it is not one.
 *
 * Deliberately forgiving about shape and unforgiving about values. The protocol is not specified,
 * so a field being absent or renamed must degrade to "we learned nothing" rather than to a bad
 * number: a `currentTime` of `NaN` reaching the drift arithmetic would produce a seek to nowhere
 * on every listener at once, which is a worse failure than never correcting drift at all.
 *
 * The caller is responsible for having checked the message's origin first. This function cannot:
 * it sees the payload, not where it came from.
 */
export function readYouTubeMessage(data: unknown): YouTubeReport | null {
  let payload: unknown = data;
  if (typeof payload === "string") {
    try {
      payload = JSON.parse(payload);
    } catch {
      return null;
    }
  }
  if (!payload || typeof payload !== "object") return null;
  const body = payload as Record<string, unknown>;
  const event = typeof body.event === "string" ? body.event : "";
  if (event !== "onStateChange" && event !== "infoDelivery" && event !== "onReady") return null;

  const report: YouTubeReport = {};
  // `onStateChange` carries the state as the bare `info`; `infoDelivery` carries an object.
  const info = body.info;
  if (typeof info === "number") {
    if (Number.isInteger(info)) report.state = info;
  } else if (info && typeof info === "object") {
    const fields = info as Record<string, unknown>;
    if (typeof fields.playerState === "number" && Number.isInteger(fields.playerState)) {
      report.state = fields.playerState;
    }
    if (isRealSeconds(fields.currentTime)) report.currentTime = fields.currentTime as number;
    if (isRealSeconds(fields.duration) && (fields.duration as number) > 0) {
      report.duration = fields.duration as number;
    }
  }
  return report;
}

/** A time in seconds that arithmetic can safely use: finite, not negative, not absurd. */
function isRealSeconds(value: unknown): boolean {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 86_400;
}

/**
 * How long a position report stays worth using, in milliseconds.
 *
 * A report is a fact about when it arrived, not a standing truth. Past this the frame has gone
 * quiet (it is loading, it was never listening, the handshake was missed) and the projected clock
 * is the better answer, because it at least advances.
 */
export const YT_REPORT_FRESH_MS = 2000;

/**
 * Where a YouTube track is, given what the frame last said and when it said it.
 *
 * This exists because the position question has two answers of different quality and the deck has
 * to prefer the good one WITHOUT depending on it. The frame's own reported time, aged by however
 * long ago it arrived, is the accurate answer. The transport offset aged on the local clock is
 * the always-available one. A player that never answers therefore still keeps a room roughly
 * together, and one that does answer keeps it exactly together.
 *
 * `projected` is what `deckPosition` already computes for a track with no readable element, so
 * this is only ever choosing between that and something better.
 */
export function youtubePosition(
  projected: number,
  report: { at: number; currentTime: number } | null,
  nowMs: number,
): number {
  return youtubeReported(report, nowMs) ?? projected;
}

/**
 * The player's own reading, aged to now, or `null` when there is not a fresh one.
 *
 * Separate from [`youtubePosition`] because the drift check needs the difference that function
 * deliberately hides. Correcting toward a projection derived from the very offset being checked
 * would compare a number against itself: the gap is always zero, so a listener whose player had
 * quietly wandered would be told it was perfectly in sync. "We do not know where the player is"
 * has to stay distinguishable from "the player is where we expected".
 */
export function youtubeReported(
  report: { at: number; currentTime: number } | null,
  nowMs: number,
): number | null {
  if (!report) return null;
  const age = nowMs - report.at;
  if (!Number.isFinite(age) || age < 0 || age > YT_REPORT_FRESH_MS) return null;
  return report.currentTime + age / 1000;
}

/**
 * Whether the player is refusing to play something the room says is playing.
 *
 * The webview will not start a video without a gesture, exactly as it will not start audio, and a
 * listener who joined a room mid-track has made no gesture. That has to read as "press play to
 * join in" rather than as a broken track, which is what this distinguishes: a player that is
 * paused or unstarted while the transport says playing has been refused, whereas one that is
 * buffering is simply not ready yet and needs no explanation.
 */
export function youtubeBlocked(state: number, transportPlaying: boolean): boolean {
  if (!transportPlaying) return false;
  return state === YT_PAUSED || state === YT_UNSTARTED || state === YT_CUED;
}

/**
 * The commands that put a player where the transport says the room is.
 *
 * Returned as a list rather than sent, so the decision is testable on its own and the order is
 * fixed in one place: seek before play. Seeking a paused player leaves it paused and showing the
 * right frame, whereas playing first makes every listener audibly start in the wrong place and
 * then jump.
 *
 * `drift` is only corrected past the threshold the shared deck logic already uses for audio. The
 * embed offers no playback-rate control, so the gentle easing a shared video gets (see
 * `driftAction`) is not available here and a correction is always a seek: doing that for small
 * gaps would make a room of listeners visibly stutter in unison, which is worse than being a
 * fraction of a second apart.
 */
export function youtubeTransportPlan(opts: {
  /** Where the room says we should be, in seconds. */
  target: number;
  /** Where the player says it is, or null when it has not said. */
  at: number | null;
  /** Whether the room's transport is playing. */
  playing: boolean;
  /** The player's last reported state. */
  state: number;
  /** Past this many seconds of gap, seek. */
  seekAfter: number;
}): string[] {
  const out: string[] = [];
  const adrift =
    opts.at !== null && Number.isFinite(opts.at) && Math.abs(opts.target - opts.at) > opts.seekAfter;
  // A player that has not said where it is gets seeked once on the way in, so a listener joining
  // mid-track lands with the room rather than at the top of the video.
  if (adrift || opts.at === null) out.push(ytCommand("seekTo", [Math.max(0, opts.target), true]));
  if (opts.playing) {
    if (opts.state !== YT_PLAYING && opts.state !== YT_BUFFERING) out.push(ytCommand("playVideo"));
  } else if (opts.state === YT_PLAYING || opts.state === YT_BUFFERING) {
    out.push(ytCommand("pauseVideo"));
  }
  return out;
}
