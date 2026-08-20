import assert from "node:assert/strict";
import test from "node:test";
import {
  CoalescedAsyncRefresh,
  SanitizedMessageCache,
  initialChatWindow,
  nearScrollBottom,
  reconcileChatWindow,
  revealNewer,
  revealOlder,
  windowAround,
} from "./chat-performance.ts";

const messages = (from: number, to: number) =>
  Array.from({ length: Math.max(0, to - from) }, (_, index) => ({ id: String(from + index) }));

test("large histories begin at a bounded recent tail", () => {
  assert.deepEqual(initialChatWindow(10_000, 300), { start: 9_700, end: 10_000 });
  assert.deepEqual(initialChatWindow(42, 300), { start: 0, end: 42 });
});

test("older and newer reveals remain bounded to the history", () => {
  assert.deepEqual(revealOlder({ start: 900, end: 1_000 }, 1_500, 200), { start: 700, end: 1_000 });
  assert.deepEqual(revealOlder({ start: 50, end: 250 }, 1_500, 200), { start: 0, end: 250 });
  assert.deepEqual(revealNewer({ start: 50, end: 250 }, 300, 200), { start: 50, end: 300 });
});

test("search and reply targets get context without mounting the whole prefix", () => {
  assert.deepEqual(windowAround(5_000, 10_000, 300), { start: 4_960, end: 5_260 });
  assert.deepEqual(windowAround(5, 10_000, 300), { start: 0, end: 300 });
  assert.deepEqual(windowAround(9_995, 10_000, 300), { start: 9_700, end: 10_000 });
});

test("a reader away from the tail keeps stable message anchors across an append", () => {
  const previous = messages(0, 1_000);
  const next = messages(0, 1_001);
  assert.deepEqual(
    reconcileChatWindow(previous, next, { start: 400, end: 700 }, false),
    { start: 400, end: 700 },
  );
  assert.deepEqual(
    reconcileChatWindow(previous, next, { start: 700, end: 1_000 }, true),
    { start: 681, end: 1_001 },
  );
});

test("deleting an edge anchor preserves a bounded nearby window", () => {
  const previous = messages(0, 1_000);
  const next = previous.filter((message) => message.id !== "400");
  assert.deepEqual(
    reconcileChatWindow(previous, next, { start: 400, end: 700 }, false),
    { start: 399, end: 699 },
  );
});

test("stable ids follow a window when rows are inserted or removed before it", () => {
  const previous = messages(0, 1_000);
  assert.deepEqual(
    reconcileChatWindow(previous, [...messages(-2, 0), ...previous], { start: 400, end: 700 }, false),
    { start: 402, end: 702 },
  );
  assert.deepEqual(
    reconcileChatWindow(previous, messages(10, 1_000), { start: 400, end: 700 }, false),
    { start: 390, end: 690 },
  );
});

test("removing both edge anchors does not throw an older reader to the live tail", () => {
  const previous = messages(0, 1_000);
  const next = previous.filter((message) => message.id !== "400" && message.id !== "699");
  assert.deepEqual(
    reconcileChatWindow(previous, next, { start: 400, end: 700 }, false),
    { start: 400, end: 700 },
  );
});

test("bottom detection tolerates fractional and near-tail layouts", () => {
  assert.equal(nearScrollBottom(800, 200, 1_000), true);
  assert.equal(nearScrollBottom(704.5, 200, 1_000), true);
  assert.equal(nearScrollBottom(600, 200, 1_000), false);
});

test("sanitized render caching is revision-aware, bounded and clearable", () => {
  const cache = new SanitizedMessageCache(2);
  let calls = 0;
  const sanitize = (text: string, mention: string) => {
    calls += 1;
    return `${text}:${mention}`;
  };
  assert.equal(cache.render("s:c", "1", "hello", 0, "me", sanitize), "hello:me");
  assert.equal(cache.render("s:c", "1", "hello", 0, "me", sanitize), "hello:me");
  assert.equal(calls, 1);
  cache.render("s:c", "1", "edited", 1, "me", sanitize);
  cache.render("s:c", "2", "two", 0, "me", sanitize);
  cache.render("s:c", "3", "three", 0, "me", sanitize);
  assert.equal(cache.size, 2);
  cache.render("s:c", "1", "edited", 1, "me", sanitize);
  assert.equal(calls, 5, "the least-recent entry was evicted and sanitized again");
  cache.clear();
  assert.equal(cache.size, 0);
});

test("bursty invalidations serialize into one active refresh and one merged follow-up", async () => {
  const calls: boolean[] = [];
  const releases: Array<() => void> = [];
  const coalescer = new CoalescedAsyncRefresh(async (animate) => {
    calls.push(animate);
    await new Promise<void>((resolve) => releases.push(resolve));
  });
  const first = coalescer.request(false);
  const sameRun = coalescer.request(true);
  coalescer.request(false);
  assert.equal(first, sameRun);
  assert.deepEqual(calls, [false]);
  releases.shift()?.();
  await new Promise<void>((resolve) => queueMicrotask(resolve));
  assert.deepEqual(calls, [false, true]);
  releases.shift()?.();
  await first;
  assert.deepEqual(calls, [false, true]);
});
