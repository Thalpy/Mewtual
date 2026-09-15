/**
 * Third-party player cards in chat: which links get one, and when one is allowed to exist.
 *
 * Several providers, one set of rules. A link to any of them that stands alone on its line can be
 * opened out into that service's own embedded player, and every one is gated identically because
 * the cost is identical: the frame tells the service this device's address and what it is looking
 * at, and it keeps doing so for as long as it is mounted.
 *
 * Hence the two-part rule below. A card needs permission before it may ever load, which is a click
 * unless the member has turned the standing preference on, and it stays mounted only while
 * somebody is actually looking at it. The second half is what makes the first half mean anything:
 * a grant that outlived the reader would leave frames talking to Google and Spotify from a tab
 * nobody has open, which is the thing the permission was supposed to be for.
 *
 * Providers themselves are deliberately dull; see `embed-provider.ts` for the contract and for the
 * one invariant each is responsible for. Adding one here must not be able to change any of the
 * above, which is why the registry is a list of parsers rather than a list of behaviours.
 */

import { SPOTIFY } from "./spotify.ts";
import { YOUTUBE } from "./youtube.ts";
import { APPLE_MUSIC, MIXCLOUD, SOUNDCLOUD } from "./providers-audio.ts";
import { BLUESKY, VIMEO } from "./providers-video.ts";
import type { EmbedProvider, EmbedRef } from "./embed-provider.ts";

/**
 * Every provider chat knows how to open out, in the order links are tested against them.
 *
 * Order carries no judgement and must not: the host sets are disjoint, so at most one provider
 * can claim any link and a link is never ambiguous. `chat-embeds.test.ts` asserts that disjointness
 * directly rather than trusting the reading, because it is the property that would quietly stop
 * holding when somebody adds a provider sharing a host with another.
 */
export const EMBED_PROVIDERS: readonly EmbedProvider[] = [
  SPOTIFY,
  YOUTUBE,
  SOUNDCLOUD,
  VIMEO,
  MIXCLOUD,
  APPLE_MUSIC,
  BLUESKY,
];

/** A link chat knows how to open out into a player, and what it points at. */
export type ChatEmbed = { provider: EmbedProvider; ref: EmbedRef };

/** The embed a link deserves, or `null` for an ordinary link. */
export function chatEmbedFor(url: string): ChatEmbed | null {
  for (const provider of EMBED_PROVIDERS) {
    const ref = provider.parse(url);
    if (ref) return { provider, ref };
  }
  return null;
}

/** Look a provider up by its stable slug, for rebuilding a card from stored state. */
export function embedProvider(id: string): EmbedProvider | null {
  return EMBED_PROVIDERS.find((provider) => provider.id === id) ?? null;
}

/**
 * The entity a card is for, stable across mounting and unmounting.
 *
 * Used as the key for a session's granted cards and to tell two cards apart, so it must name the
 * content and nothing situational: not the message it appeared in, not where it was on screen. The
 * same track linked twice in a conversation is one decision.
 *
 * Every field that distinguishes one thing from another goes in, because leaving one out silently
 * merges two entities into one grant. `start` is included for the same reason it is a separate
 * card: a video linked at a timestamp is a different thing to watch.
 */
export function embedKey(embed: ChatEmbed): string {
  const { id, kind, extra, start } = embed.ref;
  return [embed.provider.id, kind, id, extra, start].join(":");
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
  return embed.provider.pageUrl(embed.ref);
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
  clicked: boolean;
  /** The device-wide "load these automatically" preference is on (Settings, Chat & Media). */
  autoLoad: boolean;
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
 * connection the gate exists to prevent, and it is invisible when it happens.
 *
 * The two halves answer different questions and only one of them is a preference. `clicked` and
 * `autoLoad` decide **whether a card may load at all**, and either satisfies that. `onScreen` and
 * `windowVisible` decide **whether it may still be running**, and no preference relaxes those:
 * turning auto-load on is asking not to be interrupted by chips, not asking for frames talking to
 * a third party from a window nobody has open. Keeping the second half unconditional is what makes
 * the setting a convenience rather than a standing leak.
 *
 * Note what is still NOT here: per-server trust policy. An embed's host set is fixed by the CSP,
 * so unlike a remote image it cannot be pointed at loopback or a private LAN, and unlike a shared
 * file it has no author attestation a policy could act on. The decision is about disclosing this
 * device to a named company, which is the same decision whichever server the link was in.
 */
export function embedMayRender(v: EmbedVisibility): boolean {
  return (v.clicked || v.autoLoad) && v.onScreen && v.windowVisible;
}

/**
 * What the chip says before it is clicked.
 *
 * It names the service, because "load embed" does not tell anybody what is about to be contacted,
 * and that is the whole substance of the decision being asked for.
 */
export function embedChipLabel(embed: ChatEmbed): string {
  return `Load ${embed.provider.name} ${embed.provider.noun(embed.ref)}`;
}

/** What clicking the chip will actually do, said plainly enough to be a decision. */
export function embedChipTitle(embed: ChatEmbed): string {
  const who = embed.provider.name;
  const extra = embed.provider.id === "spotify"
    ? " Playback is usually a short preview unless this device is signed in to Spotify Premium."
    : "";
  return `This loads ${who}'s player, which discloses your address to them and runs their script in the window.${extra} The card unloads again when you scroll away from it.`;
}

/**
 * The frame origins every provider needs, for checking against the app's `frame-src`.
 *
 * A provider whose origin is missing from the policy produces a frame that is blocked with no
 * visible error, which is indistinguishable from a provider that is simply broken. The test that
 * consumes this reads the real `tauri.conf.json`, so the two lists cannot drift apart.
 */
export function embedFrameOrigins(): string[] {
  return [...new Set(EMBED_PROVIDERS.map((provider) => provider.origin))].sort();
}
