/**
 * Driving a third-party player frame from the jukebox transport.
 *
 * The deck's shape does not change for any of these: nothing is sent between peers, the DJ says
 * what is playing and where it is, and every listener runs its own player against that clock. A
 * linked track is one more backend for that transport alongside the media element and the jam-take
 * synth, and it is the awkward one because the player lives in another origin's document. What can
 * be done to it is send it messages; what can be known about it is whatever it volunteers.
 *
 * ## Why the frames are driven directly rather than through the vendors' scripts
 *
 * All three services publish a JavaScript SDK, and using one would mean allowing third-party
 * script in the app's own document. That is a far larger concession than allowing a frame: script
 * here can read the window it runs in, and `script-src 'self'` is one of the few things holding
 * the untrusted-markup boundary up. So each frame is spoken to directly, in the same postMessage
 * protocol its SDK would have used on our behalf. The cost is that these protocols are not
 * formally specified, which is why every reply is treated as optional and untrusted, and why a
 * deck keeps time perfectly well when a player says nothing at all.
 *
 * ## What is trusted
 *
 * Nothing a frame says. It is a foreign document under somebody else's control, so a reply is
 * evidence and never an instruction: a reported position corrects drift, a reported state notices
 * a player that would not start. Neither can move the room's transport, which changes only when a
 * member presses something.
 */

/** The player states worth telling apart, normalised across providers that number them. */
export type DeckState = "unstarted" | "playing" | "paused" | "buffering" | "ended";

/** What a message from a player frame told us. Every field is optional: it volunteers what it likes. */
export type DeckReport = {
  state?: DeckState;
  /** Seconds into the media, when this message carried a position. */
  currentTime?: number;
  /** Total length in seconds, when known and non-zero. */
  duration?: number;
};

/**
 * One provider's half of the conversation: the wire format, and nothing else.
 *
 * Everything about WHEN to send a command lives in `deckTransportPlan` and the deck itself, so a
 * driver cannot introduce a provider-specific rule about playback. It translates.
 */
export type DeckDriver = {
  /** The provider slug this drives, matching `EmbedProvider.id` and the stored queue source. */
  id: string;
  /** The origin its frames are served from; commands go only here and replies only come from here. */
  origin: string;
  /**
   * Messages that make the frame start talking, sent repeatedly until it answers.
   *
   * Every one of these players ignores anything that arrives before its own player object is
   * constructed, and none of them announce when that was. Repeating a handshake is cheaper than a
   * protocol with no way to start.
   */
  hello(): string[];
  play(): string;
  pause(): string;
  /** Seek to `seconds`; each provider has its own unit, which is the point of this layer. */
  seek(seconds: number): string;
  /** Volume as 0..1, or `null` where the provider offers no control. */
  volume(level: number): string | null;
  mute(on: boolean): string | null;
  /** Read a message from the frame, or `null` if it is not one of its reports. */
  read(data: unknown): DeckReport | null;
  /**
   * Whether this provider's player shows a picture, and therefore wants one of the call surfaces.
   *
   * Not cosmetic: a deck surface is the focus view or the dock's screen, and handing one to a
   * SoundCloud strip would put a 166-pixel audio player in a 16:9 box and drop the faces out of
   * the call to make room for it. Audio plays perfectly well from a frame hidden in the body,
   * exactly as a shared audio file does.
   */
  picture: boolean;
};

/** A time in seconds that arithmetic can safely use: finite, not negative, not absurd. */
function realSeconds(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 86_400
    ? value
    : undefined;
}

/** Parse a frame message that may arrive as a JSON string or as an already-structured object. */
function body(data: unknown): Record<string, unknown> | null {
  let payload: unknown = data;
  if (typeof payload === "string") {
    try {
      payload = JSON.parse(payload);
    } catch {
      return null;
    }
  }
  return payload && typeof payload === "object" && !Array.isArray(payload)
    ? (payload as Record<string, unknown>)
    : null;
}

// --- YouTube -------------------------------------------------------------------------------------

/** YouTube numbers its states; these are the numbers, named so a condition can be read. */
const YT_STATES: Record<number, DeckState> = {
  [-1]: "unstarted",
  0: "ended",
  1: "playing",
  2: "paused",
  3: "buffering",
  5: "unstarted", // "cued": loaded and waiting, which is unstarted as far as the deck cares
};

export const YOUTUBE_DECK: DeckDriver = {
  id: "youtube",
  origin: "https://www.youtube-nocookie.com",
  picture: true,
  hello: () => [JSON.stringify({ event: "listening", id: 1, channel: "widget" })],
  play: () => ytCommand("playVideo"),
  pause: () => ytCommand("pauseVideo"),
  seek: (seconds) => ytCommand("seekTo", [Math.max(0, seconds), true]),
  volume: (level) => ytCommand("setVolume", [Math.round(clamp01(level) * 100)]),
  mute: (on) => ytCommand(on ? "mute" : "unMute"),
  read(data) {
    const message = body(data);
    if (!message) return null;
    const event = typeof message.event === "string" ? message.event : "";
    if (event !== "onStateChange" && event !== "infoDelivery" && event !== "onReady") return null;
    const report: DeckReport = {};
    const info = message.info;
    // `onStateChange` carries the state as the bare `info`; `infoDelivery` carries an object.
    if (typeof info === "number") {
      if (Number.isInteger(info) && info in YT_STATES) report.state = YT_STATES[info];
    } else if (info && typeof info === "object") {
      const fields = info as Record<string, unknown>;
      const state = fields.playerState;
      if (typeof state === "number" && Number.isInteger(state) && state in YT_STATES) {
        report.state = YT_STATES[state];
      }
      report.currentTime = realSeconds(fields.currentTime);
      const duration = realSeconds(fields.duration);
      if (duration) report.duration = duration;
    }
    return report;
  },
};

export function ytCommand(func: string, args: readonly unknown[] = []): string {
  return JSON.stringify({ event: "command", func, args });
}

// --- SoundCloud ----------------------------------------------------------------------------------

/**
 * SoundCloud's widget speaks `{method, value}` and answers with the same shape.
 *
 * Its positions are **milliseconds**, which is the whole reason this layer exists: a seek sent in
 * seconds silently lands a thousandth of the way in, and a room would drift apart in a way that
 * looks like a sync bug rather than a unit bug.
 */
export const SOUNDCLOUD_DECK: DeckDriver = {
  id: "soundcloud",
  origin: "https://w.soundcloud.com",
  picture: false,
  hello: () => [
    scCommand("addEventListener", "playProgress"),
    scCommand("addEventListener", "play"),
    scCommand("addEventListener", "pause"),
    scCommand("addEventListener", "finish"),
    scCommand("addEventListener", "ready"),
  ],
  play: () => scCommand("play"),
  pause: () => scCommand("pause"),
  seek: (seconds) => scCommand("seekTo", Math.max(0, Math.round(seconds * 1000))),
  volume: (level) => scCommand("setVolume", Math.round(clamp01(level) * 100)),
  // The widget has no mute of its own, so silence is volume zero. The deck restores the slider's
  // value on unmute, so nothing is lost by this.
  mute: (on) => (on ? scCommand("setVolume", 0) : null),
  read(data) {
    const message = body(data);
    if (!message) return null;
    const method = typeof message.method === "string" ? message.method : "";
    const value = message.value;
    switch (method) {
      case "playProgress": {
        const fields = value && typeof value === "object" ? (value as Record<string, unknown>) : {};
        const ms = realSeconds(
          typeof fields.currentPosition === "number" ? fields.currentPosition / 1000 : undefined,
        );
        // A progress tick is also the only proof the widget is actually running.
        return { state: "playing", currentTime: ms };
      }
      case "play":
        return { state: "playing" };
      case "pause":
        return { state: "paused" };
      case "finish":
        return { state: "ended" };
      case "ready":
        return { state: "unstarted" };
      default:
        return null;
    }
  },
};

function scCommand(method: string, value?: unknown): string {
  return JSON.stringify(value === undefined ? { method } : { method, value });
}

// --- Vimeo ---------------------------------------------------------------------------------------

/**
 * Vimeo speaks `{method, value}` outbound and `{event, data}` inbound, in seconds throughout.
 *
 * Its progress event has to be subscribed to by name like SoundCloud's, and its `timeupdate`
 * carries both the position and the duration, which is where the deck learns how long a track is.
 */
export const VIMEO_DECK: DeckDriver = {
  id: "vimeo",
  origin: "https://player.vimeo.com",
  picture: true,
  hello: () => [
    vimeoCommand("addEventListener", "timeupdate"),
    vimeoCommand("addEventListener", "play"),
    vimeoCommand("addEventListener", "pause"),
    vimeoCommand("addEventListener", "ended"),
    vimeoCommand("addEventListener", "bufferstart"),
  ],
  play: () => vimeoCommand("play"),
  pause: () => vimeoCommand("pause"),
  seek: (seconds) => vimeoCommand("setCurrentTime", Math.max(0, seconds)),
  volume: (level) => vimeoCommand("setVolume", clamp01(level)),
  mute: (on) => vimeoCommand("setVolume", on ? 0 : 1),
  read(data) {
    const message = body(data);
    if (!message) return null;
    const event = typeof message.event === "string" ? message.event : "";
    const fields = message.data && typeof message.data === "object"
      ? (message.data as Record<string, unknown>)
      : {};
    switch (event) {
      case "timeupdate": {
        const duration = realSeconds(fields.duration);
        return {
          state: "playing",
          currentTime: realSeconds(fields.seconds),
          ...(duration ? { duration } : {}),
        };
      }
      case "play":
        return { state: "playing" };
      case "pause":
        return { state: "paused" };
      case "ended":
        return { state: "ended" };
      case "bufferstart":
        return { state: "buffering" };
      case "ready":
        return { state: "unstarted" };
      default:
        return null;
    }
  },
};

function vimeoCommand(method: string, value?: unknown): string {
  return JSON.stringify(value === undefined ? { method } : { method, value });
}

function clamp01(value: number): number {
  return Number.isFinite(value) ? Math.min(1, Math.max(0, value)) : 1;
}

// --- the registry, and the rules every driver shares ---------------------------------------------

const DRIVERS: readonly DeckDriver[] = [YOUTUBE_DECK, SOUNDCLOUD_DECK, VIMEO_DECK];

/** The driver for a stored queue source, or `null` for one this build cannot play. */
export function deckDriver(source: string): DeckDriver | null {
  return DRIVERS.find((driver) => driver.id === source) ?? null;
}

/** Every provider slug the deck can play, for validating a queue entry or a transport frame. */
export function deckSources(): string[] {
  return DRIVERS.map((driver) => driver.id);
}

/**
 * How long a position report stays worth using, in milliseconds.
 *
 * A report is a fact about when it arrived, not a standing truth. Past this the frame has gone
 * quiet (it is loading, it was never listening, a handshake was missed) and the projected clock is
 * the better answer, because it at least advances.
 */
export const DECK_REPORT_FRESH_MS = 2000;

/**
 * The player's own reading, aged to now, or `null` when there is not a fresh one.
 *
 * Kept separate from [`deckPlayerPosition`] because the drift check needs the difference that
 * function deliberately hides. Correcting toward a projection derived from the very offset being
 * checked would compare a number against itself: the gap is always zero, so a listener whose
 * player had quietly wandered would be told it was perfectly in sync forever. "We do not know
 * where the player is" has to stay distinguishable from "the player is where we expected".
 */
export function deckReported(
  report: { at: number; currentTime: number } | null,
  nowMs: number,
): number | null {
  if (!report) return null;
  const age = nowMs - report.at;
  if (!Number.isFinite(age) || age < 0 || age > DECK_REPORT_FRESH_MS) return null;
  return report.currentTime + age / 1000;
}

/**
 * Where a linked track is, preferring the player's own answer without depending on it.
 *
 * `projected` is what the shared deck logic computes for a track with no readable element, so this
 * only ever chooses between that and something better. A player that never answers still keeps a
 * room roughly together; one that answers keeps it exactly together.
 */
export function deckPlayerPosition(
  projected: number,
  report: { at: number; currentTime: number } | null,
  nowMs: number,
): number {
  return deckReported(report, nowMs) ?? projected;
}

/**
 * Whether the player is refusing to play something the room says is playing.
 *
 * The webview will not start media without a gesture, and a listener who joined mid-track has made
 * none. That has to read as "press play to join in" rather than as a broken track, which is what
 * this distinguishes: a player paused or unstarted while the transport says playing has been
 * refused, whereas one that is buffering is simply not ready yet and needs no explanation.
 */
export function deckPlayerBlocked(state: DeckState, transportPlaying: boolean): boolean {
  if (!transportPlaying) return false;
  return state === "paused" || state === "unstarted";
}

/**
 * The commands that put a player where the transport says the room is.
 *
 * Returned as a list rather than sent, so the decision is testable on its own and the order is
 * fixed in one place: seek before play. Seeking a paused player leaves it paused and showing the
 * right frame, whereas playing first makes every listener audibly start in the wrong place and
 * then jump.
 *
 * Drift is corrected only past the threshold, and always by seeking. None of these embeds offers a
 * usable playback-rate control, so the gentle easing a shared local video gets is not available;
 * seeking for small gaps would make a room of listeners stutter in unison, which is worse than
 * being a fraction of a second apart.
 */
export function deckTransportPlan(opts: {
  driver: DeckDriver;
  /** Where the room says we should be, in seconds. */
  target: number;
  /** Where the player says it is, or null when it has not said. */
  at: number | null;
  /** Whether the room's transport is playing. */
  playing: boolean;
  /** The player's last reported state. */
  state: DeckState;
  /** Past this many seconds of gap, seek. */
  seekAfter: number;
}): string[] {
  const { driver } = opts;
  const out: string[] = [];
  const adrift =
    opts.at !== null && Number.isFinite(opts.at) && Math.abs(opts.target - opts.at) > opts.seekAfter;
  // A player that has not said where it is gets placed once on the way in, so a listener joining
  // mid-track lands with the room rather than at the top of the track.
  if (adrift || opts.at === null) out.push(driver.seek(Math.max(0, opts.target)));
  if (opts.playing) {
    if (opts.state !== "playing" && opts.state !== "buffering") out.push(driver.play());
  } else if (opts.state === "playing" || opts.state === "buffering") {
    out.push(driver.pause());
  }
  return out;
}
