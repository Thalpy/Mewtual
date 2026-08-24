import assert from "node:assert/strict";
import test from "node:test";
import {
  CLOCK_SKEW_GRACE_MS,
  NO_READ_MARK,
  chatIsObserved,
  observationBlocker,
  transitionApplied,
  transitionMismatch,
  unreadChannels,
  unreadDecision,
  effectiveTs,
  readCeiling,
  readChannelChange,
  unreadFromHeads,
  type ChatObservation,
  type ReadMark,
  type UnreadDecision,
  type UnreadState,
} from "./unread.ts";

const change = (over: Record<string, unknown> = {}) => ({
  server: 1,
  channel: "abc",
  messages_appended: false,
  messages_changed: false,
  topic: false,
  jukebox: false,
  ...over,
});

test("only an arrival is an arrival", () => {
  assert.equal(readChannelChange(change({ messages_appended: true })).messagesAppended, true);
  // The whole reason the event grew flags: one channel document holds the log, the topic and the
  // jukebox queue, and a queue edit used to raise an unread badge for a channel nobody wrote to.
  assert.equal(readChannelChange(change({ jukebox: true })).messagesAppended, false);
  assert.equal(readChannelChange(change({ jukebox: true })).jukebox, true);
  assert.equal(readChannelChange(change({ topic: true })).messagesAppended, false);
  // An edit, a delete, a reaction or a pin re-renders the log without adding to it.
  assert.equal(readChannelChange(change({ messages_changed: true })).messagesAppended, false);
  assert.equal(readChannelChange(change({ messages_changed: true })).messagesChanged, true);
});

test("a payload carrying no flags degrades loudly, never silently", () => {
  // A badge too many is recoverable; a message that never announces itself is not.
  const c = readChannelChange({ server: 1, channel: "abc" });
  assert.equal(c.messagesAppended, true);
  assert.equal(readChannelChange(undefined).messagesAppended, true);
  assert.equal(readChannelChange(null).messagesAppended, true);
});

test("the read ceiling ignores timestamps this machine cannot believe", () => {
  const now = 1_000_000;
  assert.equal(readCeiling([500, 900, 700], now), 900);
  // A sender's clock is the sender's: a little ahead is ordinary and must still count.
  assert.equal(readCeiling([now + 1_000], now), now + 1_000);
  // Years ahead is not skew. It must not become the boundary every later message is measured
  // against, or the indicator goes quiet exactly when a real message arrives.
  assert.equal(readCeiling([500, 8.64e15], now), 500);
  assert.equal(readCeiling([], now), 0);
  assert.equal(readCeiling([now + CLOCK_SKEW_GRACE_MS + 1], now), 0);
  assert.equal(readCeiling([Number.NaN, Number.POSITIVE_INFINITY, 42], now), 42);
});

test("an implausible timestamp settles at the ceiling instead of poisoning the cursor", () => {
  const now = 1_000_000;
  const timestamps = [500, 900, 8.64e15];
  const ceiling = readCeiling(timestamps, now);
  const effective = timestamps.map((ts) => effectiveTs(ts, ceiling));
  assert.deepEqual(effective, [500, 900, 900]);

  // Reading the channel advances the mark to the newest effective timestamp, which is the newest
  // REAL message: a later legitimate message still reads as unread.
  const mark = Math.max(...effective);
  assert.equal(mark, 900);
  assert.equal(effectiveTs(950, readCeiling([...timestamps, 950], now)) > mark, true);
  // And the hostile row itself does not stick as a permanent unread divider.
  assert.equal(effectiveTs(8.64e15, ceiling) > mark, false);
});

const observed = (over: Partial<ChatObservation> = {}): ChatObservation => ({
  locked: false,
  view: "chat",
  inboxView: false,
  dmPlaceholder: false,
  spaceOpen: false,
  callFocusOpen: false,
  overlayOpen: false,
  windowFocused: true,
  documentVisible: true,
  atBottom: true,
  ...over,
});

test("a conversation counts as seen only when the log is actually in front of someone", () => {
  assert.equal(chatIsObserved(observed()), true);
  // Each of these leaves the channel SELECTED while the log is not on screen. Selection was the
  // old proxy for observation, and it is why messages arrived read.
  assert.equal(chatIsObserved(observed({ view: "files" })), false);
  assert.equal(chatIsObserved(observed({ view: "wiki" })), false);
  assert.equal(chatIsObserved(observed({ inboxView: true })), false);
  assert.equal(chatIsObserved(observed({ spaceOpen: true })), false);
  assert.equal(chatIsObserved(observed({ callFocusOpen: true })), false);
  assert.equal(chatIsObserved(observed({ overlayOpen: true })), false);
  assert.equal(chatIsObserved(observed({ dmPlaceholder: true })), false);
  assert.equal(chatIsObserved(observed({ locked: true })), false);
  // The window can hold focus while the app is behind another window on a second desktop.
  assert.equal(chatIsObserved(observed({ windowFocused: false })), false);
  assert.equal(chatIsObserved(observed({ documentVisible: false })), false);
  // Reading history is not seeing what just landed at the bottom.
  assert.equal(chatIsObserved(observed({ atBottom: false })), false);
});

const head = (channel: string, latest_incoming_ts: number, latest_incoming_id = `${channel}-msg`) => ({
  channel,
  count: 3,
  latest_ts: latest_incoming_ts,
  latest_incoming_ts,
  latest_incoming_id,
});

/** A read mark from an older build: a bare timestamp, no id. Exercises the fallback path. */
const tsMark = (ts: number): ReadMark => ({ ts, id: "" });
/** Shorthand for the channels a scan decides to badge. */
const badged = (...args: Parameters<typeof unreadFromHeads>) => unreadChannels(unreadFromHeads(...args));
const NOW = 1_000_000;

test("unread is rebuilt from heads against durable read marks", () => {
  const marks: Record<string, ReadMark> = {
    general: tsMark(900), music: tsMark(900), quiet: tsMark(900),
  };
  const unread = badged(
    [head("general", 950), head("music", 900), head("quiet", 100)],
    (c) => marks[c] ?? NO_READ_MARK,
    NOW,
  );
  // Only the channel whose newest incoming message is past what this device has read.
  assert.deepEqual(unread, ["general"]);
});

test("a head with nothing incoming never raises a badge", () => {
  // A channel holding only my own messages: sending is not receiving.
  assert.deepEqual(badged([head("general", 0, "")], () => NO_READ_MARK, NOW), []);
  // Never read at all, and somebody else has written: that is the restart case the badge exists for.
  assert.deepEqual(badged([head("general", 5)], () => NO_READ_MARK, NOW), ["general"]);
});

test("rebuilt unread only names channels the UI can open", () => {
  const heads = [head("general", 950), head("ghost", 950)];
  assert.deepEqual(badged(heads, () => tsMark(0), NOW, (c) => c === "general"), ["general"]);
});

// --- the cursor itself --------------------------------------------------------------------------
//
// A message timestamp is the SENDER's clock. Every test below is a way that fact used to lose a
// message, and every one of them is answered by comparing the id instead: the id is minted with
// the message and means the same thing on every device in the channel.

test("the newest incoming message being the one you read is what 'seen' means", () => {
  const mark: ReadMark = { ts: 900, id: "general-msg" };
  // Same id, so seen, and it stays seen no matter what the timestamps are doing. A stamp far in
  // the future here would previously have raised a badge on a message this device has read.
  assert.deepEqual(badged([head("general", 950)], () => mark, NOW), []);
  assert.deepEqual(badged([head("general", NOW + 10 * 60_000)], () => mark, NOW), []);
  assert.deepEqual(badged([head("general", 0)], () => mark, NOW), []);
});

test("two incoming messages in the same millisecond do not collapse at the cursor", () => {
  // The whole failure in one line: both stamps are 900 and the read mark is 900, so no timestamp
  // comparison anywhere can tell that a second message arrived. The id can.
  const mark: ReadMark = { ts: 900, id: "first" };
  assert.deepEqual(badged([head("general", 900, "second")], () => mark, NOW), ["general"]);
  // And the same head against its own id is still seen, so this is not just "always unread".
  assert.deepEqual(badged([head("general", 900, "first")], () => mark, NOW), []);
});

test("a sender clock moving backwards cannot hide the message it wrote", () => {
  // Their clock ran back, so the newer message carries an OLDER stamp than the one already read.
  // Measured by timestamp this is "nothing new here" forever.
  const mark: ReadMark = { ts: 950, id: "before-the-jump" };
  assert.deepEqual(badged([head("general", 400, "after-the-jump")], () => mark, NOW), ["general"]);
});

test("a sender an hour ahead is reported, not silently dropped", () => {
  const anHourAhead = NOW + 60 * 60_000;
  // With an id on both sides the clock is not consulted at all, so an hour of skew is a non-event.
  const mark: ReadMark = { ts: 900, id: "old" };
  const verdicts = unreadFromHeads([head("general", anHourAhead, "new")], () => mark, NOW);
  assert.deepEqual(verdicts, [
    { channel: "general", unread: true, reason: "head_is_not_the_read_message", cursor: "message_id" },
  ]);

  // Falling back to the clock (a mark from an older build), the message is still not lost. It
  // used to be skipped outright, which is how a badge went missing for anyone whose clock was off
  // by more than the grace. It says which cursor it had to use, because that one is the weak one.
  const legacy = unreadFromHeads([head("general", anHourAhead, "new")], () => tsMark(900), NOW);
  assert.deepEqual(legacy, [
    { channel: "general", unread: true, reason: "implausible_timestamp", cursor: "timestamp" },
  ]);
  assert.ok(anHourAhead > NOW + CLOCK_SKEW_GRACE_MS, "the point of the case is that it is past the grace");
});

test("senders with unrelated clocks are each judged on their own message, not a shared timeline", () => {
  // Three channels, three senders, three clocks: one behind, one ahead, one about right. Whether
  // each has something new is a fact about ids, and no sender's clock can answer for another's.
  const marks: Record<string, ReadMark> = {
    behind: { ts: 900, id: "behind-1" },
    ahead: { ts: 900, id: "ahead-1" },
    normal: { ts: 900, id: "normal-1" },
  };
  const heads = [
    head("behind", 100, "behind-2"),
    head("ahead", NOW + 30 * 60_000, "ahead-1"),
    head("normal", 950, "normal-2"),
  ];
  // `ahead` is seen despite being the most skewed of the three, because its id is the read one.
  assert.deepEqual(badged(heads, (c) => marks[c] ?? NO_READ_MARK, NOW), ["behind", "normal"]);
});

test("catching up after a lock finds what arrived, and reading it settles", () => {
  // The path the whole scan exists for: while locked the native bridge drops actor notifications,
  // so nothing arriving in that window has an event left to raise its badge.
  let mark: ReadMark = { ts: 900, id: "before-lock" };
  const arrived = [head("general", 1200, "during-lock")];
  assert.deepEqual(badged(arrived, () => mark, NOW), ["general"], "the arrival is recovered");
  // Reading writes the id of the newest incoming row, and the next scan is quiet. This is what
  // makes reporting a skewed message safe: clearing no longer waits on anybody's clock.
  mark = { ts: 1200, id: "during-lock" };
  assert.deepEqual(badged(arrived, () => mark, NOW), [], "and it clears by being read");
});

test("a mark written by an older build still works, and upgrades on the next read", () => {
  // Bare-timestamp marks have no id, so they use the clock until the channel is next read.
  const legacy = unreadFromHeads([head("general", 950)], () => tsMark(900), NOW);
  assert.deepEqual(legacy[0], {
    channel: "general", unread: true, reason: "newer_than_read_mark", cursor: "timestamp",
  });
  // Once read, the same head is decided by id and never touches a clock again.
  const upgraded = unreadFromHeads([head("general", 950)], () => ({ ts: 950, id: "general-msg" }), NOW);
  assert.deepEqual(upgraded[0], {
    channel: "general", unread: false, reason: "head_is_the_read_message", cursor: "message_id",
  });
});

// --- the record and the screen must agree -------------------------------------------------------
//
// The diagnostic used to be written before the transition it described was attempted, so a record
// saying `mark_unread` proved only that something intended to raise a badge.

const state = (over: Partial<UnreadState> = {}): UnreadState => ({ listed: true, unread: false, ...over });
const markUnread: UnreadDecision = { decision: "mark_unread", reason: "not_the_open_channel" };
const seen: UnreadDecision = { decision: "seen", reason: "chat_on_screen" };

test("a badge that went up is recorded as applied", () => {
  const t = { decision: markUnread, before: state(), after: state({ unread: true }) };
  assert.equal(transitionApplied(t), true);
  assert.equal(transitionMismatch(t), "");
});

test("marking a channel that was already unread changed nothing, and says so", () => {
  // Not a mismatch: the badge is up, which is what the decision asked for. It is simply not a
  // transition, and the review's invariant is about transitions.
  const t = { decision: markUnread, before: state({ unread: true }), after: state({ unread: true }) };
  assert.equal(transitionApplied(t), false);
  assert.equal(transitionMismatch(t), "");
});

test("a channel-list event racing a message event is caught, not reported as a raised badge", () => {
  // `markChannelUnread` returns early for a channel the catalog does not list, so an arrival for
  // one that a channels-changed refresh has just dropped raises nothing at all. Recorded before
  // the attempt, this read as a successful `mark_unread`.
  const t = { decision: markUnread, before: state({ listed: false }), after: state({ listed: false }) };
  assert.equal(transitionApplied(t), false);
  assert.equal(transitionMismatch(t), "channel_not_listed");
});

test("a badge that simply failed to go up is a mismatch of its own", () => {
  // The channel is listed, so "not in the catalog" does not explain it. Something else swallowed
  // the transition, and the record must not imply the badge is on screen.
  const t = { decision: markUnread, before: state(), after: state() };
  assert.equal(transitionMismatch(t), "badge_not_raised");
});

test("changing server while a refresh is in flight cannot leave a stale badge unexplained", () => {
  // The refresh that settles read state resolves against whatever conversation is loaded by then,
  // so a decision of "seen" can land on a server the user has already left. If the badge is still
  // up afterwards the record says so rather than claiming it was cleared.
  const stale = { decision: seen, before: state({ unread: true }), after: state({ unread: true }) };
  assert.equal(transitionMismatch(stale), "badge_not_cleared");
  const settled = { decision: seen, before: state({ unread: true }), after: state() };
  assert.equal(transitionMismatch(settled), "");
  assert.equal(transitionApplied(settled), true);
});

// --- why a badge did or did not appear ---------------------------------------------------------
//
// The review's highest-value invariant: an append either produces an unread transition or a valid
// explanation of why it was already seen. Both halves used to live in branches of an event
// handler, so a badge that failed to appear left no record of which branch decided against it.

test("every way the log can be covered has its own reason, not one shared boolean", () => {
  assert.equal(observationBlocker(observed()), null, "nothing covering it");
  assert.equal(observationBlocker(observed({ locked: true })), "locked");
  assert.equal(observationBlocker(observed({ view: "files" })), "another_view");
  assert.equal(observationBlocker(observed({ inboxView: true })), "inbox_open");
  assert.equal(observationBlocker(observed({ dmPlaceholder: true })), "dm_placeholder");
  assert.equal(observationBlocker(observed({ spaceOpen: true })), "space_open");
  assert.equal(observationBlocker(observed({ callFocusOpen: true })), "call_focus");
  assert.equal(observationBlocker(observed({ overlayOpen: true })), "overlay_open");
  assert.equal(observationBlocker(observed({ windowFocused: false })), "window_unfocused");
  assert.equal(observationBlocker(observed({ documentVisible: false })), "window_hidden");
  assert.equal(observationBlocker(observed({ atBottom: false })), "scrolled_up");
});

test("the reason and the boolean can never disagree, because one is built from the other", () => {
  const cases: Partial<ChatObservation>[] = [
    {}, { locked: true }, { view: "wiki" }, { overlayOpen: true },
    { windowFocused: false }, { atBottom: false }, { spaceOpen: true, atBottom: false },
  ];
  for (const over of cases) {
    const o = observed(over);
    assert.equal(chatIsObserved(o), observationBlocker(o) === null, JSON.stringify(over));
  }
});

test("a reaction or an edit is never an arrival, whatever is on screen", () => {
  // Treating these as arrivals is what made unread badges untrustworthy in the first place.
  const decision = unreadDecision({ messagesAppended: false }, observed(), true);
  assert.deepEqual(decision, { decision: "not_an_arrival", reason: "no_messages_appended" });
});

test("an arrival in a channel nobody has open is unread, and says so", () => {
  assert.deepEqual(unreadDecision({ messagesAppended: true }, observed(), false), {
    decision: "mark_unread",
    reason: "not_the_open_channel",
  });
});

test("an arrival someone is looking straight at is seen", () => {
  assert.deepEqual(unreadDecision({ messagesAppended: true }, observed(), true), {
    decision: "seen",
    reason: "chat_on_screen",
  });
});

/**
 * The case that produced the reports. The channel is selected, so it looks observed, but something
 * is covering it or the window is not in front, and the badge has to appear anyway. Which of those
 * it was is exactly what nobody could tell afterwards.
 */
test("an arrival in the open channel is still unread when something is covering it", () => {
  for (const [over, reason] of [
    [{ overlayOpen: true }, "overlay_open"],
    [{ windowFocused: false }, "window_unfocused"],
    [{ documentVisible: false }, "window_hidden"],
    [{ callFocusOpen: true }, "call_focus"],
    [{ atBottom: false }, "scrolled_up"],
  ] as [Partial<ChatObservation>, string][]) {
    assert.deepEqual(
      unreadDecision({ messagesAppended: true }, observed(over), true),
      { decision: "mark_unread", reason },
      JSON.stringify(over),
    );
  }
});
