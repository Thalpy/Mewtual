import assert from "node:assert/strict";
import test from "node:test";
import {
  CLOCK_SKEW_GRACE_MS,
  chatIsObserved,
  observationBlocker,
  unreadDecision,
  effectiveTs,
  readCeiling,
  readChannelChange,
  unreadFromHeads,
  type ChatObservation,
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

const head = (channel: string, latest_incoming_ts: number) => ({
  channel,
  count: 3,
  latest_ts: latest_incoming_ts,
  latest_incoming_ts,
  latest_incoming_id: `${channel}-msg`,
});

test("unread is rebuilt from heads against durable read marks", () => {
  const now = 1_000_000;
  const marks: Record<string, number> = { general: 900, music: 900, quiet: 900 };
  const unread = unreadFromHeads(
    [head("general", 950), head("music", 900), head("quiet", 100)],
    (c) => marks[c] ?? 0,
    now,
  );
  // Only the channel whose newest incoming message is past what this device has read.
  assert.deepEqual(unread, ["general"]);
});

test("a head with nothing incoming never raises a badge", () => {
  const now = 1_000_000;
  // A channel holding only my own messages: sending is not receiving.
  assert.deepEqual(unreadFromHeads([head("general", 0)], () => 0, now), []);
  // Never read at all, and somebody else has written: that is the restart case the badge exists for.
  assert.deepEqual(unreadFromHeads([head("general", 5)], () => 0, now), ["general"]);
});

test("rebuilt unread refuses a head this machine cannot believe", () => {
  const now = 1_000_000;
  // Otherwise one wrong clock hands you a badge that can never be cleared by reading.
  assert.deepEqual(unreadFromHeads([head("general", 8.64e15)], () => 0, now), []);
  assert.deepEqual(unreadFromHeads([head("general", now + 1_000)], () => 0, now), ["general"]);
});

test("rebuilt unread only names channels the UI can open", () => {
  const now = 1_000_000;
  const heads = [head("general", 950), head("ghost", 950)];
  assert.deepEqual(unreadFromHeads(heads, () => 0, now, (c) => c === "general"), ["general"]);
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
