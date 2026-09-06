/**
 * Recognising a Spotify share link in a message, and turning it into an embed address.
 *
 * The app never talks to Spotify on its own. This module is pure string work: it decides whether
 * a link a member pasted is one the Spotify embed player could show, and builds the address for
 * it. Whether that address is ever fetched is the caller's decision, and in chat it is always a
 * click (see `resolveRemoteMedia` in App.svelte): loading the frame discloses this device's
 * address to Spotify and puts a third party's script in the window, so it is never done for a
 * message that merely scrolled past.
 *
 * What the embed actually plays is worth knowing before reading the rest. Full tracks need a
 * Spotify Premium session signed in to `open.spotify.com` in this webview AND a working Widevine
 * decoder, and a fresh webview has neither. In practice the card plays a preview of about thirty
 * seconds, and sometimes refuses with "Spotify was not able to play encrypted media". That is why
 * this is a chat embed and not a jukebox source: a room cannot listen together to a thirty-second
 * clip, and the embed offers no playback-rate control to correct drift with even if it could.
 */

/** The entity kinds `open.spotify.com/embed/<kind>/<id>` will render. */
export type SpotifyKind = "track" | "album" | "playlist" | "artist" | "episode" | "show";

const KINDS = new Set<SpotifyKind>(["track", "album", "playlist", "artist", "episode", "show"]);

/** A Spotify entity, split into the two parts the embed address is built from. */
export type SpotifyRef = { kind: SpotifyKind; id: string };

/**
 * The id shapes accepted, which is the real boundary here.
 *
 * Every id goes into a URL path, so the only thing that matters is that it cannot leave the
 * segment it is written into: no slash, no `?`, no `#`, no dot, nothing percent-encoded. Base62
 * and nothing else gives that outright, which is why this is an allow-list of characters rather
 * than an escape step.
 *
 * The length range is deliberately looser than reality. Spotify ids are twenty-two characters
 * today, but pinning that exactly would turn a change at their end into "links silently stop
 * being recognised", and the length is not what makes this safe: the alphabet is.
 */
const ID_RE = /^[A-Za-z0-9]{15,40}$/;

/** Links longer than this are not share links; refusing early keeps `URL` off obvious junk. */
const MAX_URL_CHARS = 2048;

/**
 * A locale segment Spotify puts in front of the kind on shared links (`/intl-de/track/...`).
 * It carries no meaning for the embed, which takes the bare kind, so it is dropped.
 */
const LOCALE_RE = /^intl-[a-z]{2}(?:-[a-z]{2})?$/i;

/**
 * Read a Spotify entity out of a pasted link, or `null` if this is not one.
 *
 * Both spellings people actually paste are accepted: the web link, and the `spotify:track:ID`
 * URI the desktop client's "Copy Spotify URI" produces.
 *
 * Note what is thrown away. A shared link carries `?si=`, which is a per-share tracking token
 * identifying who sent it; rebuilding the address from the kind and id alone means the embed is
 * asked for a piece of music rather than for one person's share of it.
 */
export function spotifyRef(raw: string): SpotifyRef | null {
  const text = (raw ?? "").trim();
  if (!text || text.length > MAX_URL_CHARS) return null;

  const uri = /^spotify:([a-z]+):([A-Za-z0-9]+)$/i.exec(text);
  if (uri) return refOf(uri[1], uri[2]);

  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  if (url.hostname !== "open.spotify.com") return null;

  // `/track/ID`, `/intl-de/track/ID`, and the `/embed/track/ID` form somebody may have copied
  // out of an existing embed, all reduce to the same two parts.
  const parts = url.pathname.split("/").filter(Boolean);
  if (parts.length && LOCALE_RE.test(parts[0])) parts.shift();
  if (parts[0] === "embed") parts.shift();
  if (parts.length < 2) return null;
  return refOf(parts[0], parts[1]);
}

function refOf(kind: string, id: string): SpotifyRef | null {
  const k = kind.toLowerCase();
  if (!KINDS.has(k as SpotifyKind) || !ID_RE.test(id)) return null;
  return { kind: k as SpotifyKind, id };
}

/**
 * The address the embed frame loads.
 *
 * Built from the parsed parts rather than from the pasted string, so nothing a member wrote
 * survives into the URL except a kind this module named and an id that is bare base62.
 */
export function spotifyEmbedUrl(ref: SpotifyRef): string {
  return `https://open.spotify.com/embed/${ref.kind}/${ref.id}`;
}

/** The canonical page for a ref, for the "open in Spotify" affordance next to a loaded card. */
export function spotifyPageUrl(ref: SpotifyRef): string {
  return `https://open.spotify.com/${ref.kind}/${ref.id}`;
}

/**
 * How tall the card should be.
 *
 * A single track or episode gets Spotify's compact player; a collection gets the taller one with
 * a scrollable track list, because at compact height a playlist shows one row and reads as
 * broken. These are the sizes Spotify's own share dialog offers.
 */
export function spotifyEmbedHeight(kind: SpotifyKind): number {
  return kind === "track" || kind === "episode" ? 152 : 352;
}

/** What the load chip says, so a member knows what the click will reach for. */
export function spotifyLabel(ref: SpotifyRef): string {
  const noun = ref.kind === "show" ? "podcast" : ref.kind;
  return `Load Spotify ${noun}`;
}
