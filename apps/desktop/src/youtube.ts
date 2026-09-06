/**
 * Recognising a YouTube link, and building the address a player frame loads.
 *
 * Two callers share this module and they want different things from it, which is why it is only
 * parsing and address arithmetic:
 *
 *   - chat unfurls a link that stands alone into a player card, on an explicit click;
 *   - the jukebox queues a video as a track, and every listener runs its own frame against the
 *     DJ's clock (see `youtube-deck.ts`, which drives one of these frames over postMessage).
 *
 * The privacy cost is real and is the same in both places: loading a frame tells Google this
 * device's address and what the room is watching, on an origin the app otherwise never touches.
 * `youtube-nocookie.com` is used throughout, which withholds the advertising cookie but is not
 * anonymity: the request still happens and the address is still disclosed. Nothing here decides
 * whether to load anything, on purpose; that is the caller's gate.
 */

/**
 * A video id, which is the only member-supplied text that ever reaches a URL path here.
 *
 * Eleven characters of the URL-safe base64 alphabet, exactly. The point is not the length, which
 * YouTube has never changed, but the alphabet: no slash, no dot, no `?`, no `#`, so an id cannot
 * leave the path segment it is written into and there is no escaping step to get wrong.
 */
const ID_RE = /^[A-Za-z0-9_-]{11}$/;

/** The hosts a YouTube link is actually served from. Anything else is somebody else's site. */
const HOSTS = new Set([
  "youtube.com",
  "www.youtube.com",
  "m.youtube.com",
  "music.youtube.com",
  "youtu.be",
  "www.youtu.be",
  "youtube-nocookie.com",
  "www.youtube-nocookie.com",
]);

/** Longer than this is not a share link; refusing early keeps `URL` off obvious junk. */
const MAX_URL_CHARS = 2048;

/**
 * The largest start offset worth believing, in seconds.
 *
 * A start offset is arithmetic somebody else supplied: it is handed to a player as a seek target
 * and, in the jukebox, added to a shared clock. A day is longer than any video and small enough
 * that nothing downstream has to think about overflow.
 */
export const MAX_START_S = 24 * 60 * 60;

/** A YouTube video, and where in it the link pointed. */
export type YouTubeRef = {
  id: string;
  /** Seconds into the video, 0 when the link named no time. */
  start: number;
};

/**
 * Read a video out of a pasted link, or `null` if this is not one.
 *
 * Every spelling that circulates is accepted, because a member pastes whatever their phone or
 * their browser gave them: `watch?v=`, the `youtu.be` short link, `/shorts/`, `/live/`, `/v/`,
 * an `/embed/` address copied out of an existing player, and the music and mobile subdomains.
 *
 * A playlist link with no video in it gets nothing. The embed can play a playlist, but a jukebox
 * entry has to be one track with one duration for the room's transport to mean anything, and
 * chat and the deck deliberately parse the same way so a link cannot behave differently in the
 * two places.
 */
export function youtubeRef(raw: string): YouTubeRef | null {
  const text = (raw ?? "").trim();
  if (!text || text.length > MAX_URL_CHARS) return null;

  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  if (!HOSTS.has(url.hostname.toLowerCase())) return null;

  const parts = url.pathname.split("/").filter(Boolean);
  const short = url.hostname.toLowerCase().endsWith("youtu.be");
  // On youtu.be the id IS the path. Everywhere else it is either the `v` parameter or the segment
  // after the one naming the surface it was shared from.
  const raw_id = short
    ? (parts[0] ?? "")
    : parts[0] === "watch"
      ? (url.searchParams.get("v") ?? "")
      : parts[0] === "embed" || parts[0] === "shorts" || parts[0] === "live" || parts[0] === "v"
        ? (parts[1] ?? "")
        : (url.searchParams.get("v") ?? "");
  if (!ID_RE.test(raw_id)) return null;

  return { id: raw_id, start: startSeconds(url) };
}

/**
 * The start offset a link carries, as seconds.
 *
 * `t` is the one people share and it has two spellings: plain seconds (`t=90`), and the duration
 * form the share dialog produces (`t=1m30s`, and `1h2m3s` on a long video). `start` is the
 * parameter the embed itself takes, so an `/embed/` address copied out of a player keeps its
 * offset too. Anything unparseable is simply no offset, never a throw: a bad `t` should cost the
 * viewer the timestamp, not the video.
 */
function startSeconds(url: URL): number {
  const raw = url.searchParams.get("t") ?? url.searchParams.get("start") ?? "";
  if (!raw) return 0;
  const plain = /^\d{1,7}s?$/.exec(raw);
  if (plain) return clampStart(Number(raw.replace(/s$/, "")));
  const parts = /^(?:(\d{1,3})h)?(?:(\d{1,4})m)?(?:(\d{1,5})s)?$/.exec(raw);
  if (!parts || !(parts[1] || parts[2] || parts[3])) return 0;
  const hours = Number(parts[1] ?? 0);
  const minutes = Number(parts[2] ?? 0);
  const seconds = Number(parts[3] ?? 0);
  return clampStart(hours * 3600 + minutes * 60 + seconds);
}

function clampStart(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  return Math.min(MAX_START_S, Math.floor(value));
}

/** How a frame is going to be used, which is the only thing that changes its address. */
export type YouTubeFrameOptions = {
  /**
   * Whether the app intends to drive this frame (play, pause, seek) rather than leave it to its
   * own controls. Only the jukebox does: a chat card is somebody watching a video by themselves,
   * and a frame that answers commands it never receives is a capability with no purpose.
   */
  controlled?: boolean;
  /**
   * The window origin, which YouTube requires alongside `enablejsapi` and checks against the
   * sender of every command. Omitted for an uncontrolled frame, which sends none.
   */
  origin?: string;
  /** Seconds to begin at; the deck sets this so a joiner's frame starts level with the room. */
  start?: number;
};

/**
 * The address a player frame loads.
 *
 * Built from a validated id and numbers this module produced, so nothing a member typed reaches
 * it as text. `rel=0` keeps the end-card recommendations inside the same channel rather than
 * turning the end of a queued track into an advert for something else, and `modestbranding`
 * keeps the chrome quiet.
 */
export function youtubeEmbedUrl(ref: YouTubeRef, opts: YouTubeFrameOptions = {}): string {
  const params = new URLSearchParams({ rel: "0", modestbranding: "1", playsinline: "1" });
  const start = clampStart(opts.start ?? ref.start);
  if (start > 0) params.set("start", String(start));
  if (opts.controlled) {
    params.set("enablejsapi", "1");
    // Autoplay is asked for because the deck's own transport decides what plays; the frame still
    // obeys the webview's gesture policy, which is what the deck's "blocked" state reports.
    params.set("autoplay", "1");
    params.set("controls", "0");
    params.set("disablekb", "1");
    if (webOrigin(opts.origin)) params.set("origin", opts.origin as string);
  }
  return `https://www.youtube-nocookie.com/embed/${ref.id}?${params.toString()}`;
}

/**
 * Whether an origin is one the player will accept being told about.
 *
 * The `origin` parameter is checked by YouTube against where commands come from, and it expects a
 * web origin. This app's window is not always served from one: Tauri gives it
 * `http://tauri.localhost` on Windows and Android, but the custom scheme `tauri://localhost` on
 * macOS and Linux. Passing that through produced a player that refused to configure itself at all
 * ("Error 153") on exactly those platforms, which is a worse outcome than not naming an origin:
 * without the parameter the player still works, and commands are still posted to a fixed target
 * origin and replies still checked against it, which is where the actual protection lives.
 */
function webOrigin(origin: string | undefined): boolean {
  return !!origin && /^https?:\/\/[^\s/]+$/.test(origin);
}

/** The canonical page, for an "open on YouTube" affordance next to a card. */
export function youtubePageUrl(ref: YouTubeRef): string {
  const time = ref.start > 0 ? `&t=${ref.start}` : "";
  return `https://www.youtube.com/watch?v=${ref.id}${time}`;
}

/** A stable, bounded display name for a video nobody has fetched a title for. */
export function youtubeLabel(ref: YouTubeRef): string {
  return `YouTube video ${ref.id}`;
}
