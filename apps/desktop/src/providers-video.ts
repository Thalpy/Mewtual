/**
 * Vimeo, and Bluesky posts.
 *
 * Vimeo is the second provider the jukebox can drive: its player answers `play`, `pause` and
 * `setCurrentTime` over postMessage and reports position through a `playProgress` event, which is
 * the whole requirement for keeping a room together on something. Like YouTube it is spoken to
 * directly rather than through the vendor's SDK, so no third-party script enters this document.
 *
 * Bluesky is here because it is the one social-post embed with a fixed host, and therefore the
 * only one that can live behind this app's `frame-src` allow-list at all. It comes with a real
 * limitation, stated at `blueskyParse`: the frame takes a DID and refuses a handle, and most
 * shared links carry a handle.
 */

import {
  ref,
  segment,
  startSeconds,
  webUrl,
  type EmbedProvider,
  type EmbedRef,
} from "./embed-provider.ts";

// --- Vimeo --------------------------------------------------------------------------------------

const VIMEO_HOSTS = new Set(["vimeo.com", "www.vimeo.com", "player.vimeo.com"]);

/** A video id is digits. An unlisted video's privacy hash is lowercase hex. */
const VIMEO_ID = /^\d{6,12}$/;
const VIMEO_HASH = /^[0-9a-f]{6,20}$/i;

/**
 * Read a Vimeo link.
 *
 * The unlisted form is the one worth care. `vimeo.com/123456789/abcdef0123` carries a privacy
 * hash as a second path segment, and `player.vimeo.com/video/123456789?h=abcdef0123` carries the
 * same thing as a parameter. A frame built without it gets a refusal rather than the video, so
 * both spellings have to reach the same ref.
 */
function vimeoParse(raw: string): EmbedRef | null {
  const url = webUrl(raw, VIMEO_HOSTS);
  if (!url) return null;
  const parts = url.pathname.split("/").filter(Boolean);
  // `/video/<id>` on the player host, `/channels/<name>/<id>` and `/groups/<name>/videos/<id>` on
  // the site, and a bare `/<id>` on a share link. The id is the last all-digit segment either way.
  const at = parts.findIndex((part) => VIMEO_ID.test(part));
  if (at < 0) return null;
  const id = parts[at];
  const after = parts[at + 1] ?? "";
  const hash = VIMEO_HASH.test(after) ? after : segment(url.searchParams.get("h") ?? "", 20);
  return ref(id, {
    extra: VIMEO_HASH.test(hash) ? hash.toLowerCase() : "",
    start: clockParam(url.hash.replace(/^#/, "") || url.searchParams.get("t")),
  });
}

/** How a frame is going to be used: only the deck drives one, so only the deck arms one. */
export type VimeoFrameOptions = { controlled?: boolean; start?: number };

export function vimeoFrameUrl(r: EmbedRef, opts: VimeoFrameOptions = {}): string {
  const params = new URLSearchParams({ byline: "0", portrait: "0", dnt: "1" });
  if (r.extra) params.set("h", r.extra);
  const start = startSeconds(opts.start ?? r.start);
  if (start > 0) params.set("#t", String(start));
  if (opts.controlled) {
    // `api=1` is what makes the player answer postMessage at all; without it a frame silently
    // ignores every command, which looks exactly like a deck that is not working.
    params.set("api", "1");
    params.set("autoplay", "1");
    params.set("controls", "0");
  }
  // `dnt=1` asks Vimeo not to track the session. It is a request to a third party rather than a
  // guarantee, and it does not stop them seeing the address that asked.
  return `https://player.vimeo.com/video/${r.id}?${params.toString()}`;
}

export const VIMEO: EmbedProvider = {
  id: "vimeo",
  name: "Vimeo",
  origin: "https://player.vimeo.com",
  parse: vimeoParse,
  frameUrl: (r) => vimeoFrameUrl(r),
  pageUrl: (r) => `https://vimeo.com/${r.id}${r.extra ? `/${r.extra}` : ""}`,
  height: () => 0, // 16:9, sized by the stylesheet like any other picture
  noun: () => "video",
  deck: true,
};

// --- Bluesky ------------------------------------------------------------------------------------

const BLUESKY_HOSTS = new Set(["bsky.app", "www.bsky.app", "staging.bsky.app"]);

/** A `did:plc:` identifier: the method is fixed, the suffix is base32-ish and bounded. */
const BLUESKY_DID = /^did:plc:[a-z0-9]{16,32}$/;
/** A record key: the TID format Bluesky mints, lowercase base32-sortable. */
const BLUESKY_RKEY = /^[a-z0-9]{8,16}$/;

/**
 * Read a Bluesky post link, when it is one this can embed without asking anybody.
 *
 * The frame takes a DID and rejects a handle, and a link somebody copies out of the app carries
 * whichever the author is currently using, which is usually a handle. Turning a handle into a DID
 * is a network request to Bluesky, and making one here would be this device contacting them about
 * a link that merely scrolled past, which is the exact thing the click gate exists to prevent.
 *
 * So a handle link is not claimed at all: it stays an ordinary link rather than becoming a card
 * that cannot load. That is a real gap and it is the honest shape of it; resolving handles belongs
 * to the send-time unfurl path, where one person's client does the lookup once and the result
 * becomes ordinary shared content.
 */
function blueskyParse(raw: string): EmbedRef | null {
  const url = webUrl(raw, BLUESKY_HOSTS);
  if (!url) return null;
  const parts = url.pathname.split("/").filter(Boolean);
  if (parts.length !== 4 || parts[0] !== "profile" || parts[2] !== "post") return null;
  const did = parts[1].toLowerCase();
  const rkey = parts[3].toLowerCase();
  if (!BLUESKY_DID.test(did) || !BLUESKY_RKEY.test(rkey)) return null;
  return ref(rkey, { extra: did });
}

export const BLUESKY: EmbedProvider = {
  id: "bluesky",
  name: "Bluesky",
  origin: "https://embed.bsky.app",
  parse: blueskyParse,
  // The DID contains colons, which are legal in a path segment and must NOT be encoded away:
  // `encodeURIComponent` would turn `did:plc:x` into `did%3Aplc%3Ax` and the embed would not find
  // the record. It is safe unencoded because the shape was pinned by the regex above.
  frameUrl: (r) => `https://embed.bsky.app/embed/${r.extra}/app.bsky.feed.post/${r.id}`,
  pageUrl: (r) => `https://bsky.app/profile/${r.extra}/post/${r.id}`,
  height: () => 300,
  noun: () => "post",
  deck: false,
};

/** Shared with the audio providers: `t=` in plain seconds or the `1h2m3s` duration form. */
function clockParam(raw: string | null): number {
  if (!raw) return 0;
  if (/^\d{1,7}s?$/.test(raw)) return startSeconds(Number(raw.replace(/s$/, "")));
  const parts = /^(?:(\d{1,3})h)?(?:(\d{1,4})m)?(?:(\d{1,5})s)?$/.exec(raw);
  if (!parts || !(parts[1] || parts[2] || parts[3])) return 0;
  return startSeconds(Number(parts[1] ?? 0) * 3600 + Number(parts[2] ?? 0) * 60 + Number(parts[3] ?? 0));
}
