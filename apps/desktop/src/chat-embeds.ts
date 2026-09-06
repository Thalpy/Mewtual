/**
 * Third-party player cards in chat: which links get one, and when one is allowed to exist.
 *
 * Two providers, one set of rules. A Spotify or YouTube link that stands alone on its line can be
 * opened out into that service's own embedded player, and both are gated identically because the
 * cost is identical: the frame tells the service this device's address and what it is looking at,
 * and it keeps doing so for as long as it is mounted.
 *
 * Hence the two-part rule below. A card needs a click before it may ever load (the app makes no
 * third-party request for a message that merely scrolled past), and it stays mounted only while
 * somebody is actually looking at it. The second half is what makes the first half mean anything:
 * a grant that outlived the reader would leave frames talking to Google and Spotify from a tab
 * nobody has open, which is the thing the click was supposed to be consent for.
 */

import { spotifyPageUrl, spotifyRef, type SpotifyRef } from "./spotify.ts";
import { youtubePageUrl, youtubeRef, type YouTubeRef } from "./youtube.ts";

/** A link chat knows how to open out into a player. */
export type ChatEmbed =
  | { provider: "spotify"; ref: SpotifyRef }
  | { provider: "youtube"; ref: YouTubeRef };

export type EmbedProvider = ChatEmbed["provider"];

/**
 * The embed a link deserves, or `null` for an ordinary link.
 *
 * Ordering is not a judgement about the providers: the two host sets are disjoint, so at most one
 * can match and a link is never ambiguous.
 */
export function chatEmbedFor(url: string): ChatEmbed | null {
  const track = spotifyRef(url);
  if (track) return { provider: "spotify", ref: track };
  const video = youtubeRef(url);
  if (video) return { provider: "youtube", ref: video };
  return null;
}

/**
 * The entity a card is for, stable across mounting and unmounting.
 *
 * Used as the key for a session's granted cards and to rebuild a card from its chip, so it must
 * name the content and nothing situational: not the message it appeared in, not where it was on
 * screen. The same track linked twice in a conversation is one decision.
 */
export function embedKey(embed: ChatEmbed): string {
  return embed.provider === "spotify"
    ? `spotify:${embed.ref.kind}:${embed.ref.id}`
    : `youtube:${embed.ref.id}:${embed.ref.start}`;
}

/**
 * The link a card is rebuilt from, in the one spelling this module will parse back.
 *
 * A card is mounted and unmounted repeatedly as the reader scrolls, so each swap has to
 * reconstruct the other half from what the DOM is holding. Keeping a canonical link on the
 * element (rather than the member's original text) means the round trip goes through the same
 * validation as the first parse did: an attribute somebody managed to tamper with is re-checked
 * on the way back in, not trusted because it was already there.
 */
export function chatEmbedLink(embed: ChatEmbed): string {
  return embed.provider === "spotify" ? spotifyPageUrl(embed.ref) : youtubePageUrl(embed.ref);
}

/**
 * How far outside the viewport a card is still kept alive, in CSS pixels.
 *
 * Zero would be correct and unusable: a card would die on the scroll that nudged it one pixel
 * past the edge and the reader would watch it flicker. A screen's worth of slack means ordinary
 * reading never disturbs a card, while a genuine scroll away from it still unmounts it.
 */
export const EMBED_KEEPALIVE_MARGIN_PX = 600;

/** Everything that decides whether one card may be mounted right now. */
export type EmbedVisibility = {
  /** The member clicked this card's chip at some point in this session. */
  granted: boolean;
  /** The card's place in the document is within the keepalive margin of the viewport. */
  onScreen: boolean;
  /** The window itself is showing (not minimised, not a background tab). */
  windowVisible: boolean;
};

/**
 * Whether the live frame should exist.
 *
 * Written as one function rather than as conditions spread over an observer, a visibility handler
 * and a click handler, because the failure that matters is a frame that survives one of them: a
 * card left running in a pane the reader navigated away from is exactly the passive third-party
 * connection the click gate exists to prevent, and it is invisible when it happens.
 *
 * Note what is NOT here: trust policy. A card is click-only in every mode (see
 * `mayAutoLoadRemoteUrl`), so there is no policy under which `granted` could be implied rather
 * than clicked.
 */
export function embedMayRender(v: EmbedVisibility): boolean {
  return v.granted && v.onScreen && v.windowVisible;
}

/**
 * What the chip says before it is clicked.
 *
 * It names the service, because "load embed" does not tell anybody what is about to be contacted,
 * and that is the whole substance of the decision being asked for.
 */
export function embedChipLabel(embed: ChatEmbed): string {
  if (embed.provider === "youtube") return "Load YouTube video";
  const noun = embed.ref.kind === "show" ? "podcast" : embed.ref.kind;
  return `Load Spotify ${noun}`;
}

/** What clicking the chip will actually do, said plainly enough to be a decision. */
export function embedChipTitle(embed: ChatEmbed): string {
  return embed.provider === "youtube"
    ? "This loads YouTube's player, which discloses your address to Google and runs their script in the window. The card unloads again when you scroll away from it."
    : "This loads Spotify's player, which discloses your address to Spotify and runs their script in the window. Playback is usually a short preview unless this device is signed in to Spotify Premium. The card unloads again when you scroll away from it.";
}
