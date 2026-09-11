/**
 * What every third-party player provider has to supply, and the rules they all obey.
 *
 * There are several of these now (Spotify, YouTube, SoundCloud, Vimeo, Mixcloud, Apple Music) and
 * they differ only in string arithmetic: which links they claim, and what address a frame loads.
 * Everything that actually matters is the same for all of them and is decided once, in
 * `chat-embeds.ts`: a card is inert until it is permitted, it exists only while somebody can see
 * it, and the frame is built in code rather than from a member's text. A provider is therefore
 * deliberately not allowed to be interesting. It is a parser and two URL builders.
 *
 * ## The one invariant a provider is responsible for
 *
 * Whatever it puts in `EmbedRef` ends up in a URL. A provider must guarantee that every field it
 * fills cannot escape the part of the address it is written into: a path component may hold no
 * slash, dot, `?` or `#`, and anything going into a query string must be percent-encoded by the
 * builder. `segment` and `path` below are the two ways to satisfy that, and providers use them
 * rather than inventing a check each, because this is the rule a new provider is most likely to
 * get subtly wrong and the one with the worst consequence when it does.
 */

/**
 * One thing a provider can show.
 *
 * Four fields rather than a per-provider type, because the registry stores and compares these
 * without knowing which provider they came from. What each field means is the provider's business;
 * what is guaranteed about all of them is that they are validated, bounded, and safe to place in
 * an address.
 */
export type EmbedRef = {
  /**
   * The provider's primary id. A single token for most, and a slash-joined path for the providers
   * whose address is a path (a SoundCloud track, a Mixcloud show); in that case every segment has
   * been validated separately, so the only slashes present are ones this module put there.
   */
  id: string;
  /** A sub-kind where the provider has several (a Spotify album vs track), `""` when it has one. */
  kind: string;
  /** A second component some addresses need: an unlisted-video hash, a storefront, a track id. */
  extra: string;
  /** Seconds into the media the link pointed at, `0` when it named no time. */
  start: number;
};

/** A provider with nothing filled in, so a parser can name only the fields it uses. */
export function ref(id: string, over: Partial<EmbedRef> = {}): EmbedRef {
  return { id, kind: "", extra: "", start: 0, ...over };
}

export type EmbedProvider = {
  /** Stable slug. Appears in stored keys and class names, so it never changes once shipped. */
  id: string;
  /** What to call the service in front of a person. */
  name: string;
  /**
   * The origin frames are served from. Must also be in the app's `frame-src`, or the frame is
   * blocked with no visible error; `chat-embeds.test.ts` checks the two lists against each other,
   * because a provider added to one and not the other looks exactly like a provider that is
   * simply broken.
   */
  origin: string;
  /** Read a link, or `null` if this provider does not claim it. */
  parse(raw: string): EmbedRef | null;
  /** The address a frame loads. Built from `ref`, never from the member's original text. */
  frameUrl(ref: EmbedRef): string;
  /** The canonical page, for "open on ..." next to a loaded card. */
  pageUrl(ref: EmbedRef): string;
  /** Frame height in CSS pixels, or `0` for a 16:9 picture box the stylesheet sizes. */
  height(ref: EmbedRef): number;
  /** What the chip calls this one: "track", "video", "album", "mix". */
  noun(ref: EmbedRef): string;
  /**
   * Whether the jukebox deck can drive this provider's player: it must expose play, pause, a seek
   * and some way of reporting where it has got to, or a room cannot be kept together on it. Only
   * YouTube, SoundCloud and Vimeo do; Spotify deliberately does not (see `spotify.ts`).
   */
  deck: boolean;
};

/** Links longer than this are not share links; refusing early keeps `URL` off obvious junk. */
export const MAX_URL_CHARS = 2048;

/**
 * The largest start offset worth believing, in seconds.
 *
 * A start offset is arithmetic somebody else supplied: it is handed to a player as a seek target
 * and, in the jukebox, added to a shared clock. A day is longer than any media and small enough
 * that nothing downstream has to think about overflow.
 */
export const MAX_START_S = 24 * 60 * 60;

/** Clamp a parsed offset to something the rest of the app can safely do arithmetic with. */
export function startSeconds(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  return Math.min(MAX_START_S, Math.floor(value));
}

/**
 * Parse a link to a URL, or `null` if it is not an ordinary web address on one of `hosts`.
 *
 * The host check is an exact set membership on purpose. A `startsWith` or an `includes` would
 * accept `open.spotify.com.example.invalid`, and a `endsWith` would accept
 * `notsoundcloud.com`; both are the standard way this check is got wrong, and both hand a frame
 * to somebody else's server.
 */
export function webUrl(raw: string, hosts: ReadonlySet<string>): URL | null {
  const text = (raw ?? "").trim();
  if (!text || text.length > MAX_URL_CHARS) return null;
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  return hosts.has(url.hostname.toLowerCase()) ? url : null;
}

/**
 * One path segment, or `""` if it is not one this may safely place in an address.
 *
 * The alphabet is the whole point, and it is deliberately narrower than what a URL path permits:
 * letters, digits, `_`, `-`, `.` are allowed, but a segment that IS a dot run (`.`, `..`) is not,
 * because those traverse. Nothing percent-encoded survives either, since `%2F` is a slash wearing
 * a hat.
 */
export function segment(raw: string, max = 128): string {
  const value = raw ?? "";
  if (!value || value.length > max) return "";
  if (/^\.+$/.test(value)) return "";
  return /^[A-Za-z0-9_.-]+$/.test(value) ? value : "";
}

/**
 * Several validated segments joined back into a path, or `""` if any of them is not one.
 *
 * Providers whose address is a path (SoundCloud, Mixcloud) keep their id in this form. Validating
 * each piece and re-joining means the only slashes in the result are the ones written here, so the
 * shape of the stored id is decided by this function rather than by whatever the member pasted.
 */
export function path(parts: readonly string[], max = 8): string {
  if (!parts.length || parts.length > max) return "";
  const clean: string[] = [];
  for (const part of parts) {
    const one = segment(part);
    if (!one) return "";
    clean.push(one);
  }
  return clean.join("/");
}
