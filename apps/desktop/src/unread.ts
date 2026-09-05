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
  /** The ids that actually arrived, in the order they now read. Empty when nothing did, or when
   * the payload predates this field. Rows are ordered by their sender timestamps, so an arrival
   * is not always the last row and anything describing it has to be told which rows to use. */
  arrivals: string[];
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
    return { messagesAppended: true, messagesChanged: true, topic: true, jukebox: true, arrivals: [] };
  }
  return {
    messagesAppended: p.messages_appended === true,
    messagesChanged: p.messages_changed === true,
    topic: p.topic === true,
    jukebox: p.jukebox === true,
    arrivals: Array.isArray(p.arrivals)
      ? p.arrivals.filter((id): id is string => typeof id === "string" && id.length > 0)
      : [],
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
 * How far this device has read in one channel.
 *
 * `id` is the cursor and `ts` is for display. That split is the whole point: a message id is
 * minted with the message and means the same thing on every device, while `ts` is the SENDER's
 * clock and means only what that machine believed at the time. Deciding "is there something new
 * here" from a clock is what produced the reports: two messages in the same millisecond collapse,
 * a sender whose clock runs backwards writes a newer message with an older stamp, and every sender
 * in a channel is running an unrelated clock anyway.
 *
 * `ts` survives because the "new messages" divider needs somewhere to sit in a list ordered by
 * time, and being a few rows out there is a cosmetic problem rather than a lost message.
 */
export type ReadMark = {
  /** Effective timestamp of the newest row seen. Positions the divider; never decides unread. */
  ts: number;
  /** Stable id of the newest message from somebody else that this device has seen. The cursor. */
  id: string;
};

/** A read mark for a channel that has never been read. */
export const NO_READ_MARK: ReadMark = { ts: 0, id: "" };

/** What the durable scan concluded about one channel, and which cursor it got there by. */
export type UnreadVerdict = {
  channel: string;
  unread: boolean;
  /** Stable identifier for the reason, for counting and comparing across reports. */
  reason: string;
  /** Which cursor actually decided this. `message_id` is the trustworthy one. */
  cursor: "message_id" | "timestamp" | "none";
};

/**
 * Rebuild a server's unread channel list by comparing activity heads with durable read marks.
 *
 * This is the path that survives an explicit lock, a restart and an offline catch-up, none of
 * which the live event stream covers: while locked the native bridge deliberately drops actor
 * notifications, and a restart begins with no event history at all. Without this the badges for
 * everything that arrived in the meantime are simply gone.
 *
 * The comparison is by message id wherever both sides have one, which needs no clock and so has
 * no skew to handle: either the newest message somebody else wrote is the one this device last
 * read, or it is not. Timestamps are the fallback for marks written by older builds and for
 * messages that predate stable ids, and that path reports itself as the weaker cursor it is.
 *
 * `known` filters to channels the UI actually lists, so a head for a channel that was removed
 * from the directory cannot raise a badge nothing can open.
 */
export function unreadFromHeads(
  heads: ChannelHead[],
  readMarkOf: (channel: string) => ReadMark,
  now: number,
  known?: (channel: string) => boolean,
  grace = CLOCK_SKEW_GRACE_MS,
): UnreadVerdict[] {
  const limit = now + grace;
  const out: UnreadVerdict[] = [];
  for (const head of heads) {
    if (known && !known(head.channel)) continue;
    const mark = readMarkOf(head.channel);
    const id = typeof head.latest_incoming_id === "string" ? head.latest_incoming_id : "";
    const ts = head.latest_incoming_ts;
    const has = Number.isFinite(ts) && ts > 0;
    if (!id && !has) {
      out.push({ channel: head.channel, unread: false, reason: "nothing_incoming", cursor: "none" });
      continue;
    }
    if (id && mark.id) {
      // The one comparison with no clock in it. Note that this reads "different", not "newer":
      // two ids cannot be ordered, and they do not need to be. If the newest message somebody
      // else wrote is not the one this device read, something has happened here since.
      const unread = id !== mark.id;
      out.push({
        channel: head.channel,
        unread,
        reason: unread ? "head_is_not_the_read_message" : "head_is_the_read_message",
        cursor: "message_id",
      });
      continue;
    }
    // No usable id on one side or the other, so this falls back to the sender's clock with all
    // the caveats above. It resolves itself: the next read of this channel writes an id, and
    // every later scan takes the branch above.
    if (!has) {
      out.push({ channel: head.channel, unread: false, reason: "no_incoming_timestamp", cursor: "none" });
      continue;
    }
    if (ts > limit) {
      // A stamp this far ahead cannot be measured against a read mark honestly. It used to be
      // dropped, which silently lost a real message from anyone whose clock was off by more than
      // the grace. Raising the badge instead is safe now in a way it was not before: reading the
      // channel writes an id, so the badge clears on the id branch rather than waiting for a
      // clock nobody controls to come back down.
      out.push({ channel: head.channel, unread: true, reason: "implausible_timestamp", cursor: "timestamp" });
      continue;
    }
    const unread = ts > mark.ts;
    out.push({
      channel: head.channel,
      unread,
      reason: unread ? "newer_than_read_mark" : "not_newer_than_read_mark",
      cursor: "timestamp",
    });
  }
  return out;
}

/** The channels a scan says to badge. */
export function unreadChannels(verdicts: UnreadVerdict[]): string[] {
  return verdicts.filter((v) => v.unread).map((v) => v.channel);
}

/** One channel's badge state, as the record needs to describe it before and after a transition. */
export type UnreadState = {
  /** Is this channel in the catalog at all? A badge for one that is not goes nowhere. */
  listed: boolean;
  unread: boolean;
};

/** A decision, the states either side of it, and whether the badge actually moved. */
export type UnreadTransition = {
  decision: UnreadDecision;
  before: UnreadState;
  after: UnreadState;
};

/** Did the badge state actually change? */
export function transitionApplied(t: UnreadTransition): boolean {
  return t.before.unread !== t.after.unread;
}

/**
 * The reason a decision did not produce the state it asked for, or `""` when the record is honest.
 *
 * This exists because the diagnostic used to be written before the transition it described was
 * attempted, so a record saying `mark_unread` proved only that something intended to raise a
 * badge. If the channel was missing from the catalog, or a refresh replaced the server entry
 * underneath, nothing happened and the record said otherwise. Anything this returns is a case
 * where the log and the screen disagree, which is the one thing a diagnostic must never hide.
 */
export function transitionMismatch(t: UnreadTransition): string {
  if (t.decision.decision === "mark_unread") {
    if (!t.after.listed) return "channel_not_listed";
    if (!t.after.unread) return "badge_not_raised";
    return "";
  }
  if (t.decision.decision === "seen" && t.after.unread) return "badge_not_cleared";
  return "";
}
