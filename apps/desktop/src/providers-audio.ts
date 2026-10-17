/**
 * The path-addressed audio providers: SoundCloud, Mixcloud, Apple Music.
 *
 * Grouped because they share the shape that makes them different from YouTube and Spotify: what
 * identifies the thing is not a token but a **path**, and two of the three take that path as a
 * query parameter rather than as part of the frame's own path. That changes where the escaping
 * has to happen, which is the only part of a provider worth being careful about, so they are
 * written next to each other where the difference is visible.
 *
 * SoundCloud is also the one provider here the jukebox can drive. Its widget answers play, pause,
 * `seekTo` and reports position, which is the whole requirement for keeping a room together, and
 * it does it over postMessage so no third-party script has to enter this document.
 */

import {
  path,
  ref,
  segment,
  startSeconds,
  webUrl,
  type EmbedProvider,
  type EmbedRef,
} from "./embed-provider.ts";

// --- SoundCloud ---------------------------------------------------------------------------------

const SOUNDCLOUD_HOSTS = new Set(["soundcloud.com", "www.soundcloud.com", "m.soundcloud.com"]);

/**
 * Paths that are a person or a listing rather than something playable.
 *
 * A bare `soundcloud.com/artist` is a profile page; the widget will render it as that artist's
 * whole catalogue, which is not what somebody who pasted a profile link was offering the room.
 * Two segments is a track, and `sets/<name>` is a playlist.
 */
const SOUNDCLOUD_NOT_MEDIA = new Set(["discover", "search", "you", "stream", "upload", "pages"]);

function soundcloudParse(raw: string): EmbedRef | null {
  const url = webUrl(raw, SOUNDCLOUD_HOSTS);
  if (!url) return null;
  const parts = url.pathname.split("/").filter(Boolean);
  if (parts.length < 2 || parts.length > 3) return null;
  if (SOUNDCLOUD_NOT_MEDIA.has(parts[0].toLowerCase())) return null;
  // `artist/track`, or `artist/sets/playlist`. A third segment that is not `sets` is a comment
  // permalink or a stray, and naming the track without it is the better answer.
  const kind = parts.length === 3 && parts[1].toLowerCase() === "sets" ? "sets" : "";
  const wanted = kind ? parts.slice(0, 3) : parts.slice(0, 2);
  const id = path(wanted);
  if (!id) return null;
  // `t=` on a SoundCloud link is `1h2m3s`-ish or plain seconds, same as everywhere else.
  return ref(id, { kind, start: clockParam(url.searchParams.get("t")) });
}

/**
 * The widget takes the track's own page address as a query parameter, so the escaping happens in
 * `encodeURIComponent` rather than in the path alphabet. Both are applied: the id was validated
 * segment by segment on the way in, and it is encoded again on the way out, because a value that
 * is safe in a path is not automatically safe in a query.
 */
function soundcloudFrame(r: EmbedRef): string {
  const target = encodeURIComponent(`https://soundcloud.com/${r.id}`);
  const params = "auto_play=false&hide_related=true&show_comments=false&show_teaser=false";
  return `https://w.soundcloud.com/player/?url=${target}&${params}`;
}

export const SOUNDCLOUD: EmbedProvider = {
  id: "soundcloud",
  name: "SoundCloud",
  origin: "https://w.soundcloud.com",
  parse: soundcloudParse,
  frameUrl: soundcloudFrame,
  pageUrl: (r) => `https://soundcloud.com/${r.id}`,
  // A single track gets the compact player; a set needs room for its list, the same reason a
  // Spotify playlist does.
  height: (r) => (r.kind === "sets" ? 400 : 166),
  noun: (r) => (r.kind === "sets" ? "playlist" : "track"),
  deck: true,
};

// --- Mixcloud -----------------------------------------------------------------------------------

const MIXCLOUD_HOSTS = new Set(["mixcloud.com", "www.mixcloud.com", "m.mixcloud.com"]);

function mixcloudParse(raw: string): EmbedRef | null {
  const url = webUrl(raw, MIXCLOUD_HOSTS);
  if (!url) return null;
  const parts = url.pathname.split("/").filter(Boolean);
  // `user/show`. A bare `/user` is a profile, and anything deeper is a listing rather than a mix.
  if (parts.length !== 2) return null;
  if (parts[0].toLowerCase() === "discover") return null;
  const id = path(parts);
  return id ? ref(id) : null;
}

export const MIXCLOUD: EmbedProvider = {
  id: "mixcloud",
  name: "Mixcloud",
  origin: "https://player.mixcloud.com",
  parse: mixcloudParse,
  // The widget wants the feed as a percent-encoded path WITH its surrounding slashes, which is
  // why this is built rather than interpolated: the slashes are part of the encoded value.
  frameUrl: (r) => `https://player.mixcloud.com/widget/iframe/?feed=${encodeURIComponent(`/${r.id}/`)}&hide_cover=1`,
  pageUrl: (r) => `https://www.mixcloud.com/${r.id}/`,
  height: () => 120,
  noun: () => "mix",
  deck: false,
};

// --- Apple Music --------------------------------------------------------------------------------

const APPLE_HOSTS = new Set(["music.apple.com", "embed.music.apple.com", "geo.music.apple.com"]);

/** The entity kinds the embed will render. Anything else is a page, not a thing to play. */
const APPLE_KINDS = new Set(["album", "playlist", "song", "music-video", "artist"]);

/** A storefront is a two-letter country code; it decides which catalogue the embed shows. */
const APPLE_STOREFRONT = /^[a-z]{2}$/i;

function appleParse(raw: string): EmbedRef | null {
  const url = webUrl(raw, APPLE_HOSTS);
  if (!url) return null;
  const parts = url.pathname.split("/").filter(Boolean);
  // `/<storefront>/<kind>/<slug>/<id>`, with the slug sometimes absent on a short share link.
  if (parts.length < 3 || parts.length > 4) return null;
  const store = APPLE_STOREFRONT.test(parts[0]) ? parts[0].toLowerCase() : "";
  const kind = parts[1].toLowerCase();
  if (!store || !APPLE_KINDS.has(kind)) return null;
  const id = segment(parts[parts.length - 1], 64);
  const slug = parts.length === 4 ? segment(parts[2], 128) : "";
  if (!id || (parts.length === 4 && !slug)) return null;
  // `?i=` selects one song inside an album, and is the difference between "this track" and "the
  // record it is on". It is a bare id like the others.
  const track = segment(url.searchParams.get("i") ?? "", 64);
  return { id, kind, extra: [store, slug, track].join("|"), start: 0 };
}

/** Unpack what `appleParse` packed, so the two halves cannot disagree about the order. */
function appleParts(r: EmbedRef): { store: string; slug: string; track: string } {
  const [store = "", slug = "", track = ""] = r.extra.split("|");
  return { store, slug, track };
}

function applePath(r: EmbedRef): string {
  const { store, slug } = appleParts(r);
  return slug ? `${store}/${r.kind}/${slug}/${r.id}` : `${store}/${r.kind}/${r.id}`;
}

export const APPLE_MUSIC: EmbedProvider = {
  id: "applemusic",
  name: "Apple Music",
  origin: "https://embed.music.apple.com",
  parse: appleParse,
  frameUrl: (r) => {
    const { track } = appleParts(r);
    const query = track ? `?i=${encodeURIComponent(track)}` : "";
    return `https://embed.music.apple.com/${applePath(r)}${query}`;
  },
  pageUrl: (r) => {
    const { track } = appleParts(r);
    const query = track ? `?i=${encodeURIComponent(track)}` : "";
    return `https://music.apple.com/${applePath(r)}${query}`;
  },
  // A single song is the compact player; a record or a playlist shows its track list. Apple's own
  // share dialog offers these two sizes.
  height: (r) => (r.kind === "song" || appleParts(r).track ? 175 : 450),
  noun: (r) => (r.kind === "music-video" ? "video" : r.kind === "song" ? "song" : r.kind),
  deck: false,
};

/**
 * A `t=`-style offset in the two spellings that circulate: plain seconds, and the `1h2m3s`
 * duration form. Anything else is no offset, never a throw: a bad timestamp should cost the
 * listener the timestamp and not the track.
 */
function clockParam(raw: string | null): number {
  if (!raw) return 0;
  if (/^\d{1,7}s?$/.test(raw)) return startSeconds(Number(raw.replace(/s$/, "")));
  const parts = /^(?:(\d{1,3})h)?(?:(\d{1,4})m)?(?:(\d{1,5})s)?$/.exec(raw);
  if (!parts || !(parts[1] || parts[2] || parts[3])) return 0;
  return startSeconds(Number(parts[1] ?? 0) * 3600 + Number(parts[2] ?? 0) * 60 + Number(parts[3] ?? 0));
}

/**
 * The address a **driveable** SoundCloud frame loads.
 *
 * Only the deck builds one of these. The widget answers postMessage commands whatever the address
 * says, so what the extra parameters buy is the absence of surprises: no auto-play before the
 * transport asks for one, no related-track continuation at the end of a queued track (which would
 * have one listener wander off into somebody else's catalogue while the room moved on), and none
 * of the social chrome a room is not using.
 *
 * `start` rides in the fragment, which is where this widget takes an offset; a listener joining
 * part-way through therefore begins level with the room rather than at the top.
 */
export function soundcloudDeckUrl(link: string, start = 0): string {
  const target = encodeURIComponent(`https://soundcloud.com/${link}`);
  const params = [
    "auto_play=true",
    "hide_related=true",
    "show_comments=false",
    "show_user=false",
    "show_teaser=false",
    "continuous_play=false",
    "visual=true",
  ].join("&");
  const at = startSeconds(start);
  return `https://w.soundcloud.com/player/?url=${target}&${params}${at > 0 ? `#t=${at}` : ""}`;
}
