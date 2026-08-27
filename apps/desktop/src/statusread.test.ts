import assert from "node:assert/strict";
import test from "node:test";
import { CLOCK_SKEW_GRACE_MS } from "./unread.ts";
import {
  NO_STATUS_READ,
  STATUS_BOUNDARY_IDS_MAX,
  firstStatusUnreadIndex,
  isNewsItemUnseen,
  isStatusUnread,
  markStatusRead,
  newsFilter,
  newsUnseenByServer,
  newsUnseenCount,
  sameStatusCursor,
  sanitizeStatusCursor,
  splitNewsSections,
  statusCursorFor,
  statusFeedOrder,
  statusIsObserved,
  statusSurfaceOpen,
  statusUnreadCount,
  type NewsItem,
  type StatusObservation,
  type StatusCursors,
  type StatusPost,
} from "./statusread.ts";

const NOW = 1_000_000;

/** An announcement. `get_statuses` returns these newest-first. */
const post = (id: string, ts: number): StatusPost => ({ id, ts, author: "nell", text: `post ${id}` });

/** A feed as the backend sends it: newest-first. */
const feed = (...pairs: [string, number][]): StatusPost[] => pairs.map(([id, ts]) => post(id, ts));

// --- the cursor ---------------------------------------------------------------------------------

test("a cursor survives the round trip through the sealed record", () => {
  const cursor = markStatusRead(feed(["c", 950], ["b", 900], ["a", 800]), NOW);
  // The cursors ride inside the vault's UI-state JSON, so the round trip is that blob's.
  const back = sanitizeStatusCursor(JSON.parse(JSON.stringify(cursor)));
  assert.deepEqual(back, cursor);
  // And the round trip has to preserve the ANSWER, not just the fields: a cursor that reads back
  // to the same shape but stops excusing the post it was written from is a badge that never clears.
  const posts = feed(["c", 950], ["b", 900], ["a", 800]);
  assert.equal(statusUnreadCount(posts, back), 0);
});

test("a corrupt or hand-edited cursor reads as never read, never as read-everything", () => {
  for (const bad of [null, undefined, "", "a string", 7, true, [1, 2, 3], {}]) {
    assert.deepEqual(sanitizeStatusCursor(bad), NO_STATUS_READ, JSON.stringify(bad ?? null));
  }
  // Unknown fields drop, bad values re-clamp, and the good half of a mixed entry still comes back.
  const mixed = sanitizeStatusCursor({
    ts: 900.7, ids: ["a", "a", "", 7, null, "b"], mode: "everything", nope: true,
  });
  assert.deepEqual(mixed, { ts: 900, ids: ["a", "b"] });
  assert.equal((mixed as Record<string, unknown>).mode, undefined);
  // A `ts` that is not a believable moment must not become the boundary the whole feed is measured
  // against; falling back to zero shows the feed again, where trusting it would hide it forever.
  for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, -1, "900", null]) {
    assert.equal(sanitizeStatusCursor({ ts: bad, ids: [] }).ts, 0, JSON.stringify(bad));
  }
  // An entry claiming thousands of boundary ids is not a tie-break, it is a payload.
  const flood = sanitizeStatusCursor({ ts: 900, ids: Array.from({ length: 500 }, (_, i) => `id-${i}`) });
  assert.equal(flood.ids.length, STATUS_BOUNDARY_IDS_MAX);
  // An id longer than anything this app mints is dropped rather than truncated: a shortened id
  // matches nothing, so it would sit in the set looking like a tie-break while never breaking one.
  const long = sanitizeStatusCursor({ ts: 900, ids: ["ok", "x".repeat(200)] });
  assert.deepEqual(long.ids, ["ok"]);
});

test("a mark that did not move is recognised, so an unchanged feed seals nothing", () => {
  // Every render of the surface re-runs the mark. Each write schedules a vault save, so a cursor
  // that says the same thing has to compare equal or reading one feed rewrites the whole record.
  const posts = feed(["c", 950], ["b", 900], ["a", 800]);
  const first = markStatusRead(posts, NOW);
  assert.equal(sameStatusCursor(markStatusRead(posts, NOW, first), first), true);
  assert.equal(sameStatusCursor(NO_STATUS_READ, NO_STATUS_READ), true);
  // Order inside the boundary set is an accident of how the feed was walked, not a change.
  assert.equal(sameStatusCursor({ ts: 900, ids: ["a", "b"] }, { ts: 900, ids: ["b", "a"] }), true);
  // A real advance is a real difference, whichever half of the cursor moved.
  assert.equal(sameStatusCursor({ ts: 900, ids: ["a"] }, { ts: 901, ids: ["a"] }), false);
  assert.equal(sameStatusCursor({ ts: 900, ids: ["a"] }, { ts: 900, ids: ["a", "b"] }), false);
  assert.equal(sameStatusCursor({ ts: 900, ids: ["a"] }, { ts: 900, ids: ["b"] }), false);
  const later = markStatusRead([post("d", 1_200), ...posts], NOW, first);
  assert.equal(sameStatusCursor(later, first), false);
});

// --- reading the feed ---------------------------------------------------------------------------

test("a server nobody has opened has every announcement unread", () => {
  const posts = feed(["c", 950], ["b", 900], ["a", 800]);
  assert.equal(statusUnreadCount(posts, NO_STATUS_READ), 3);
  assert.equal(isStatusUnread(post("c", 950)), true);
});

test("reading the feed clears it, and the next announcement raises it again", () => {
  const posts = feed(["c", 950], ["b", 900], ["a", 800]);
  const cursor = markStatusRead(posts, NOW);
  assert.equal(statusUnreadCount(posts, cursor), 0);
  const later = [post("d", 1_200), ...posts];
  assert.equal(statusUnreadCount(later, cursor), 1);
  assert.equal(isStatusUnread(post("d", 1_200), cursor), true);
});

test("two announcements in the same millisecond do not collapse at the boundary", () => {
  // The failure a bare timestamp cannot see: both stamps are 900 and the mark is 900, so no
  // comparison of clocks anywhere can tell that a second announcement was posted. The id can.
  const cursor = markStatusRead(feed(["first", 900], ["old", 500]), NOW);
  assert.deepEqual(cursor.ids, ["first"]);
  assert.equal(isStatusUnread(post("second", 900), cursor), true);
  assert.equal(isStatusUnread(post("first", 900), cursor), false);
});

test("an empty feed is a no-op, not an event that marks a year of announcements read", () => {
  assert.deepEqual(markStatusRead([], NOW), NO_STATUS_READ);
  assert.equal(statusUnreadCount([], NO_STATUS_READ), 0);
  assert.equal(firstStatusUnreadIndex([], NO_STATUS_READ), -1);
  // The real shape of this: the surface opens, the fetch fails, the feed comes back empty, and
  // marking read must not walk the mark backwards to zero and un-read everything already seen.
  const read = markStatusRead(feed(["c", 950], ["b", 900]), NOW);
  assert.deepEqual(markStatusRead([], NOW, read), read);
  // Nor may a short or older page do it. Opening a filtered view is not forgetting what was read.
  assert.deepEqual(markStatusRead(feed(["a", 100]), NOW, read).ts, read.ts);
  assert.equal(statusUnreadCount(feed(["c", 950], ["b", 900]), markStatusRead([], NOW, read)), 0);
});

// --- the clock is the sender's ------------------------------------------------------------------

test("one broken clock cannot wedge the read mark into the future and silence the feed", () => {
  // The whole reason this module does not take a raw maximum. `evil` is stamped years ahead, either
  // by a machine whose clock is wrong or by a member who chose the number.
  const evil = 8.64e15;
  const posts = feed(["evil", evil], ["b", 900], ["a", 800]);
  const cursor = markStatusRead(posts, NOW);

  // The mark lands on the newest announcement that is actually plausible, not on the hostile one.
  assert.equal(cursor.ts, 900);
  assert.ok(cursor.ts < evil, "the mark must not follow a clock this machine cannot believe");

  // So a genuine announcement posted afterwards still announces itself. Taken raw, the mark would
  // be 8.64e15 and every real announcement for the next few centuries would read as already seen.
  assert.equal(isStatusUnread(post("real", 950), cursor), true);
  assert.equal(statusUnreadCount([post("real", 950), ...posts], cursor), 1);

  // And the hostile row does not stick as a permanent unread divider nothing can clear: this read
  // covered it, its id says so, and no clock is consulted to find that out.
  assert.equal(isStatusUnread(post("evil", evil), cursor), false);
  assert.equal(statusUnreadCount(posts, cursor), 0);
  assert.equal(firstStatusUnreadIndex(posts, cursor), -1);
});

test("a sender a little ahead is ordinary skew and still counts", () => {
  // Two desktops are seconds apart, not years, and refusing that would hide real announcements.
  const nearly = NOW + 60_000;
  assert.ok(nearly < NOW + CLOCK_SKEW_GRACE_MS, "the point of the case is that it is inside the grace");
  const cursor = markStatusRead(feed(["ahead", nearly], ["a", 900]), NOW);
  assert.equal(cursor.ts, nearly);
  assert.equal(statusUnreadCount(feed(["ahead", nearly], ["a", 900]), cursor), 0);
});

test("a feed stamped entirely from the future is still readable", () => {
  // Every post past the grace, so there is no plausible ceiling anywhere in the feed. The mark
  // cannot advance on a clock, and the ids are what stop the count sticking at three forever.
  const far = NOW + 400 * 60_000;
  const posts = feed(["c", far + 2], ["b", far + 1], ["a", far]);
  assert.equal(statusUnreadCount(posts, NO_STATUS_READ), 3);
  const cursor = markStatusRead(posts, NOW);
  assert.equal(statusUnreadCount(posts, cursor), 0, "reading a skewed feed must actually clear it");
});

test("a millisecond holding more announcements than the boundary set still reaches zero", () => {
  // A seeded or migrated server stamps its whole feed alike. Truncating the id set would leave the
  // overflow permanently unread with no way to clear, so the mark steps past the boundary instead.
  const many = Array.from({ length: STATUS_BOUNDARY_IDS_MAX + 20 }, (_, i) => post(`p${i}`, 900));
  const cursor = markStatusRead(many, NOW);
  assert.equal(cursor.ts, 901);
  assert.deepEqual(cursor.ids, []);
  assert.equal(statusUnreadCount(many, cursor), 0);
  // Still a mark, not a surrender: anything genuinely newer is unread, and a second read is stable.
  assert.equal(isStatusUnread(post("later", 1_000), cursor), true);
  assert.deepEqual(markStatusRead(many, NOW, cursor), cursor);
});

// --- the divider --------------------------------------------------------------------------------

test("the divider sits above the oldest unread announcement, whichever way the feed is ordered", () => {
  const cursor = markStatusRead(feed(["b", 900], ["a", 800]), NOW);
  // Newest-first, as `get_statuses` returns it: the oldest unread is the LAST unread index.
  const newestFirst = feed(["e", 1_400], ["d", 1_200], ["b", 900], ["a", 800]);
  assert.equal(statusFeedOrder(newestFirst), "newest-first");
  assert.equal(firstStatusUnreadIndex(newestFirst, cursor), 1, "the row `d`, above which the divider goes");

  // Reversed for rendering in reading order: the same announcement, the other end of the array.
  const oldestFirst = [...newestFirst].reverse();
  assert.equal(statusFeedOrder(oldestFirst), "oldest-first");
  assert.equal(firstStatusUnreadIndex(oldestFirst, cursor), 2);
  assert.equal(oldestFirst[2].id, "d", "both orderings name the same announcement");
});

test("the divider has nowhere to go when nothing is unread, and says so", () => {
  const posts = feed(["b", 900], ["a", 800]);
  assert.equal(firstStatusUnreadIndex(posts, markStatusRead(posts, NOW)), -1);
  // A feed nobody has read is unread all the way down, so the divider goes above the oldest row.
  assert.equal(firstStatusUnreadIndex(posts, NO_STATUS_READ), 1);
});

test("one announcement out of order is not a reordered feed", () => {
  // A sender's clock again. Deciding the ordering on the first pair that disagrees would flip the
  // divider to the opposite end of the feed over a single row.
  assert.equal(statusFeedOrder(feed(["a", 900], ["b", 950], ["c", 800], ["d", 700])), "newest-first");
  assert.equal(statusFeedOrder(feed(["a", 700], ["b", 800], ["c", 950], ["d", 900])), "oldest-first");
  // Empty, single and entirely tied feeds are reported as the backend sends them.
  assert.equal(statusFeedOrder([]), "newest-first");
  assert.equal(statusFeedOrder(feed(["a", 900])), "newest-first");
  assert.equal(statusFeedOrder(feed(["a", 900], ["b", 900])), "newest-first");
  // The caller can override when it knows, rather than letting the feed be re-sniffed.
  const posts = feed(["a", 900], ["b", 900]);
  assert.equal(firstStatusUnreadIndex(posts, NO_STATUS_READ, "oldest-first"), 0);
  assert.equal(firstStatusUnreadIndex(posts, NO_STATUS_READ, "newest-first"), 1);
});

// --- is anybody actually there? -----------------------------------------------------------------

const looking: StatusObservation = {
  view: "status", inboxView: false, spaceOpen: false, isDm: false,
  windowFocused: true, documentVisible: true,
};

test("an announcement is only marked read while somebody is there to read it", () => {
  assert.equal(statusIsObserved(looking), true);
  // A window parked unfocused on this surface still re-renders every time a post lands, and the
  // mark is durable: treating that as a read marks an announcement nobody saw as read forever.
  assert.equal(statusIsObserved({ ...looking, windowFocused: false }), false);
  // Focused is not visible: minimised, or on another virtual desktop.
  assert.equal(statusIsObserved({ ...looking, documentVisible: false }), false);
  // And nothing is read from a surface that is not the one on screen.
  assert.equal(statusIsObserved({ ...looking, view: "chat" }), false);
  assert.equal(statusIsObserved({ ...looking, inboxView: true }), false);
  assert.equal(statusIsObserved({ ...looking, spaceOpen: true }), false);
});

test("losing focus must not release the divider that is holding somebody's place", () => {
  // The subtle half. The NEW line is measured against a boundary snapshotted on arrival, so if the
  // surface counted as closed on every blur the snapshot would be released and retaken on return,
  // moving the divider off the row somebody had read up to. Being ON the surface is the weaker
  // question for exactly that reason, and it answers the same either side of a blur.
  assert.equal(statusSurfaceOpen(looking), true);
  assert.equal(statusSurfaceOpen({ ...looking, windowFocused: false }), true);
  assert.equal(statusSurfaceOpen({ ...looking, documentVisible: false }), true);
  // Leaving the surface does release it, which is what makes a return take a fresh boundary.
  assert.equal(statusSurfaceOpen({ ...looking, view: "wiki" }), false);
  assert.equal(statusSurfaceOpen({ ...looking, inboxView: true }), false);
  assert.equal(statusSurfaceOpen({ ...looking, spaceOpen: true }), false);
  // A DM has no noticeboard, so neither question can ever be answered yes there: no divider is
  // held and, more to the point, no read cursor is ever written under a DM's id.
  assert.equal(statusSurfaceOpen({ ...looking, isDm: true }), false);
  assert.equal(statusIsObserved({ ...looking, isDm: true }), false);
});

// --- the News tab -------------------------------------------------------------------------------

const news = (server: number, kind: "status" | "event", ts: number, id?: string): NewsItem =>
  id === undefined ? { server, kind, ts } : { server, kind, ts, id };

test("a News row is unseen against the cursor of the server that posted it", () => {
  const cursors: StatusCursors = { 1: { ts: 900, ids: ["a"] }, 2: { ts: 5_000, ids: [] } };
  assert.equal(isNewsItemUnseen(news(1, "status", 1_200), cursors), true);
  assert.equal(isNewsItemUnseen(news(1, "status", 800), cursors), false);
  // The same timestamp, a different server: each server's mark answers only for its own feed.
  assert.equal(isNewsItemUnseen(news(2, "status", 1_200), cursors), false);
  // A server with no cursor at all has never been read, so everything it posted is unseen.
  assert.equal(isNewsItemUnseen(news(9, "status", 1), cursors), true);
  assert.deepEqual(statusCursorFor(cursors, 9), NO_STATUS_READ);
  assert.deepEqual(statusCursorFor({} as StatusCursors, 1), NO_STATUS_READ);
});

test("a News row without an id is seen at the boundary rather than stuck on it", () => {
  // The tab's rows are flattened from two feeds and carry no id today. Nothing can excuse such a
  // row from the mark, so the mark must not hold it: the row the cursor was written from would
  // otherwise stay unseen forever, and the rail badge above it would never reach zero.
  const cursors: StatusCursors = { 1: { ts: 900, ids: ["b"] } };
  assert.equal(isNewsItemUnseen(news(1, "status", 900), cursors), false);
  assert.equal(isNewsItemUnseen(news(1, "status", 901), cursors), true);
  // With an id the tie is settled properly instead: the read one is seen, its neighbour is not.
  assert.equal(isNewsItemUnseen(news(1, "status", 900, "b"), cursors), false);
  assert.equal(isNewsItemUnseen(news(1, "status", 900, "c"), cursors), true);
});

test("an event is never unseen, because its timestamp is when it starts", () => {
  // An event's `ts` is its scheduled start, not the moment it was created, so it is normally in the
  // future: measured against any announcement cursor it reads unseen until the day it happens, and
  // a mark advanced past it to shut it up would swallow every announcement posted since.
  const cursors: StatusCursors = { 1: { ts: 900, ids: [] } };
  assert.equal(isNewsItemUnseen(news(1, "event", NOW + 7 * 86_400_000), cursors), false);
  assert.equal(isNewsItemUnseen(news(1, "event", 1_200), cursors), false);
  assert.equal(isNewsItemUnseen(news(1, "status", 1_200), cursors), true);
});

test("the News chips filter by kind and by server, and mean the same thing everywhere", () => {
  const items = [
    news(1, "status", 1_200),
    news(1, "event", 9_000),
    news(2, "status", 1_100),
    news(2, "event", 9_100),
  ];
  assert.equal(items.filter(newsFilter("all")).length, 4);
  assert.equal(items.filter(newsFilter("status")).length, 2);
  assert.equal(items.filter(newsFilter("event")).length, 2);
  assert.equal(items.filter(newsFilter("all", 1)).length, 2);
  assert.deepEqual(items.filter(newsFilter("status", 2)), [news(2, "status", 1_100)]);
  // Server 0 is a real id and must not be read as "no server chosen"; absent means all servers.
  assert.equal([news(0, "status", 1)].filter(newsFilter("all", 0)).length, 1);
  assert.equal(items.filter(newsFilter("all", null)).length, 4);
  assert.equal(items.filter(newsFilter("all", undefined)).length, 4);
  // The count runs the same rule over whatever the chip selected, so the badge above the list can
  // never disagree with the list under it.
  const cursors: StatusCursors = { 1: { ts: 900, ids: [] }, 2: { ts: 900, ids: [] } };
  assert.equal(newsUnseenCount(items.filter(newsFilter("all", 1)), cursors), 1);
});

test("the inbox badge counts unseen announcements across every server", () => {
  const items = [
    news(1, "status", 1_200),
    news(1, "status", 1_100),
    news(1, "status", 100),
    news(2, "status", 1_100),
    news(2, "event", 9_000),
    news(3, "status", 50),
  ];
  const cursors: StatusCursors = { 1: { ts: 1_000, ids: [] }, 2: { ts: 5_000, ids: [] } };
  // Two on server 1, none on server 2 (already read past it), one on server 3 (never read).
  assert.equal(newsUnseenCount(items, cursors), 3);
  assert.deepEqual(newsUnseenByServer(items, cursors), { 1: 2, 3: 1 });
  // The billboards and the rail badge are the same pass, so no beacon can disagree with the total.
  const byServer = newsUnseenByServer(items, cursors);
  const summed = Object.values(byServer).reduce((a, b) => a + b, 0);
  assert.equal(summed, newsUnseenCount(items, cursors));
  // A server with nothing unseen is absent rather than a zero, so a beacon is never rendered for it.
  assert.equal(byServer[2], undefined);
  assert.deepEqual(newsUnseenByServer([], cursors), {});
  assert.equal(newsUnseenCount([], cursors), 0);
});

test("a pinned announcement heads the News tab instead of being listed in it twice", () => {
  type Row = NewsItem & { pinned: boolean };
  const status = (server: number, ts: number, id: string, pinned = false): Row =>
    ({ server, kind: "status", ts, id, pinned });
  const items: Row[] = [
    status(1, 1_400, "new-pin", true),
    status(1, 1_300, "plain"),
    status(1, 200, "old-pin", true),
    status(2, 1_200, "other-pin", true),
    { server: 2, kind: "event", ts: 9_000, id: "party", pinned: false },
  ];
  const { pinned, feed } = splitNewsSections(items, 30);
  // One pin per server, newest first: what each server has most recently lifted to the top.
  assert.deepEqual(pinned.map((n) => n.id), ["new-pin", "other-pin"]);
  // Exactly those rows leave the feed, and no more. `old-pin` is pinned but is NOT in the section
  // above, so it keeps its chronological place instead of vanishing from the tab altogether.
  assert.deepEqual(feed.map((n) => n.id), ["plain", "old-pin"]);
  // Neither half is an event: this is the announcement half of the tab.
  assert.equal(feed.concat(pinned).some((n) => n.kind === "event"), false);
  // And nothing is in both, which is the duplicate the two sections used to show.
  assert.equal(feed.some((n) => pinned.includes(n)), false);
  // The feed is the only half that is trimmed, and it is trimmed newest-first.
  assert.deepEqual(splitNewsSections(items, 1).feed.map((n) => n.id), ["plain"]);
  assert.deepEqual(splitNewsSections(items, 1).pinned.map((n) => n.id), ["new-pin", "other-pin"]);
  assert.deepEqual(splitNewsSections([] as Row[], 30), { pinned: [], feed: [] });
});

test("reading one server's announcements clears only that server's beacon", () => {
  const posts = feed(["c", 1_200], ["b", 1_100]);
  const items = [news(1, "status", 1_200), news(1, "status", 1_100), news(2, "status", 1_200)];
  const cursors: StatusCursors = { 1: NO_STATUS_READ, 2: NO_STATUS_READ };
  assert.deepEqual(newsUnseenByServer(items, cursors), { 1: 2, 2: 1 });
  cursors[1] = markStatusRead(posts, NOW, cursors[1]);
  assert.deepEqual(newsUnseenByServer(items, cursors), { 2: 1 });
  assert.equal(newsUnseenCount(items, cursors), 1);
});
