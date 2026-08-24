/**
 * Unread state, derived from state rather than remembered from events.
 *
 * The old model answered "did some event happen while I was listening", which is not the question
 * the indicator asks. It cannot survive a lock or a restart, it counts a reaction as an arrival,
 * and it treats a selected channel as a read one even when the log is nowhere on screen. Every
 * helper here answers the real question instead: is there a message this person has not seen?
 *
 * All pure, so the two-client behaviour is testable without a webview.
 */

/** Which part of a channel document moved, as the native `channel-updated` event reports it. */
export type ChannelChange = {
  /** A message id appeared that was not there before. The ONLY flag that may create unread state. */
  messagesAppended: boolean;
  /** The log re-rendered without an arrival: an edit, a delete, a reaction or a pin. */
  messagesChanged: boolean;
  topic: boolean;
  jukebox: boolean;
};

/**
 * Read the change flags off a `channel-updated` payload.
 *
 * A payload with no flags at all is read as an arrival. Native and webview ship together so that
 * should not happen, but if the contract ever drifts the failure is a badge too many rather than a
 * message that never announces itself, and only one of those is a lost message.
 */
export function readChannelChange(payload: unknown): ChannelChange {
  const p = (payload ?? {}) as Record<string, unknown>;
  const known =
    typeof p.messages_appended === "boolean" ||
    typeof p.messages_changed === "boolean" ||
    typeof p.topic === "boolean" ||
    typeof p.jukebox === "boolean";
  if (!known) {
    return { messagesAppended: true, messagesChanged: true, topic: true, jukebox: true };
  }
  return {
    messagesAppended: p.messages_appended === true,
    messagesChanged: p.messages_changed === true,
    topic: p.topic === true,
    jukebox: p.jukebox === true,
  };
}

/**
 * How far ahead of this machine's clock a sender's timestamp may be and still be believed.
 * Ordinary skew between two desktops is seconds; five minutes is generous for that and still
 * nowhere near a value that could hide a day's messages.
 */
export const CLOCK_SKEW_GRACE_MS = 5 * 60_000;

/**
 * The newest timestamp in a channel that is worth trusting as a read boundary.
 *
 * A message timestamp is the SENDER's clock, injected by the sender. Used raw as a cursor, one
 * device with a broken clock (or one modified member choosing the value) moves the read mark years
 * into the future, and every legitimate message after it silently fails the "newer than the mark"
 * test: the indicator goes quiet exactly when it matters.
 *
 * The ceiling is the newest plausible timestamp actually present. Anything above it is pulled down
 * to it by `effectiveTs`, so an implausible message can never push the cursor past the real
 * conversation, and it settles as read rather than sticking as a permanent unread row.
 */
export function readCeiling(timestamps: number[], now: number, grace = CLOCK_SKEW_GRACE_MS): number {
  const limit = now + grace;
  let ceiling = 0;
  for (const ts of timestamps) {
    if (Number.isFinite(ts) && ts <= limit && ts > ceiling) ceiling = ts;
  }
  return ceiling;
}

/** A message's timestamp as read state may use it: never past the channel's plausible ceiling. */
export function effectiveTs(ts: number, ceiling: number): number {
  if (!Number.isFinite(ts)) return 0;
  return ts > ceiling ? ceiling : ts;
}

/**
 * Everything that has to be true for a conversation to count as SEEN.
 *
 * Selecting a channel is not reading it, and neither is loading its messages. The app has a dozen
 * surfaces that cover the log completely (files, wiki, settings, the inbox, the orbit view, a call
 * taking the window), and a message that arrives behind any of them was never observed.
 */
export type ChatObservation = {
  locked: boolean;
  /** The content column's active surface; only "chat" shows the message log. */
  view: string;
  /** The cross-server inbox has taken the main pane. */
  inboxView: boolean;
  /** The DM home placeholder is showing instead of a conversation. */
  dmPlaceholder: boolean;
  /** The orbit/server-space overlay is up. */
  spaceOpen: boolean;
  /** The call focus surface has taken the window. */
  callFocusOpen: boolean;
  /** A modal overlay is covering the log (settings, lightbox, wiki review, …). */
  overlayOpen: boolean;
  windowFocused: boolean;
  /** `document.visibilityState === "visible"`: minimised or on another virtual desktop is not. */
  documentVisible: boolean;
  /** The log is pinned to its newest row; reading history further up is not seeing the arrival. */
  atBottom: boolean;
};

/** Is the message log actually in front of a person right now? */
export function chatIsObserved(o: ChatObservation): boolean {
  return observationBlocker(o) === null;
}

/**
 * The first reason the log is *not* in front of a person, or `null` when it is.
 *
 * `chatIsObserved` answers yes or no, which is all the product needs and none of what a diagnostic
 * needs. When an unread badge fails to appear, the question is never "was it observed" but "what
 * did the app think was covering the log", and that is ten different conditions producing one
 * boolean. Ordered from most to least likely to be the real explanation, so the first hit is the
 * one worth reporting.
 *
 * Stable identifiers rather than prose: these end up in a diagnostic record, get counted, and are
 * compared across reports.
 */
export function observationBlocker(o: ChatObservation): string | null {
  if (o.locked) return "locked";
  if (o.view !== "chat") return "another_view";
  if (o.inboxView) return "inbox_open";
  if (o.dmPlaceholder) return "dm_placeholder";
  if (o.spaceOpen) return "space_open";
  if (o.callFocusOpen) return "call_focus";
  if (o.overlayOpen) return "overlay_open";
  if (!o.windowFocused) return "window_unfocused";
  if (!o.documentVisible) return "window_hidden";
  if (!o.atBottom) return "scrolled_up";
  return null;
}

/** What happened to one channel's unread state, and why. */
export type UnreadDecision = {
  /** `mark_unread`, `seen`, or `not_an_arrival`. */
  decision: "mark_unread" | "seen" | "not_an_arrival";
  /** A stable identifier for the reason. Empty when the decision needs no explanation. */
  reason: string;
};

/**
 * Decide what an update to a channel means for its unread state, and say why.
 *
 * The review's highest-value invariant is that an append either produces an unread transition or a
 * valid explanation of why it was already seen. Both halves of that used to be spread across
 * branches of an event handler, so a badge that failed to appear left no record of which branch
 * decided it should not: "reacted to an old message", "the window was behind something", and "the
 * user was looking straight at it" are three very different answers and they all produced silence.
 */
export function unreadDecision(
  change: { messagesAppended: boolean },
  observation: ChatObservation,
  isActiveChannel: boolean,
): UnreadDecision {
  // A reaction, an edit, a topic rename or a queued track is not somebody talking, and treating
  // any of them as an arrival is what made unread badges untrustworthy in the first place.
  if (!change.messagesAppended) return { decision: "not_an_arrival", reason: "no_messages_appended" };
  if (!isActiveChannel) return { decision: "mark_unread", reason: "not_the_open_channel" };
  const blocker = observationBlocker(observation);
  if (blocker) return { decision: "mark_unread", reason: blocker };
  return { decision: "seen", reason: "chat_on_screen" };
}

/** One channel's newest activity, with no message text: what `get_channel_heads` returns. */
export type ChannelHead = {
  channel: string;
  count: number;
  latest_ts: number;
  /** Newest timestamp among messages this device did not write (`0` if there are none). */
  latest_incoming_ts: number;
  latest_incoming_id: string;
};

/**
 * Rebuild a server's unread channel list by comparing activity heads with durable read marks.
 *
 * This is the path that survives an explicit lock, a restart and an offline catch-up, none of
 * which the live event stream covers: while locked the native bridge deliberately drops actor
 * notifications, and a restart begins with no event history at all. Without this the badges for
 * everything that arrived in the meantime are simply gone.
 *
 * `known` filters to channels the UI actually lists, so a head for a channel that was removed
 * from the directory cannot raise a badge nothing can open.
 */
export function unreadFromHeads(
  heads: ChannelHead[],
  readMarkOf: (channel: string) => number,
  now: number,
  known?: (channel: string) => boolean,
  grace = CLOCK_SKEW_GRACE_MS,
): string[] {
  const limit = now + grace;
  const out: string[] = [];
  for (const head of heads) {
    if (known && !known(head.channel)) continue;
    const ts = head.latest_incoming_ts;
    if (!Number.isFinite(ts) || ts <= 0) continue;
    // A head above the plausible limit cannot be measured against a read mark honestly, and
    // treating it as unread would hand anyone with a wrong clock a badge that never clears.
    if (ts > limit) continue;
    if (ts > readMarkOf(head.channel)) out.push(head.channel);
  }
  return out;
}
