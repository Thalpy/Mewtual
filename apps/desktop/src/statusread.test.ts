import assert from "node:assert/strict";
import test from "node:test";
import { CLOCK_SKEW_GRACE_MS } from "./unread.ts";
import {
  NO_STATUS_READ,
  STATUS_BOUNDARY_IDS_MAX,
  firstStatusUnreadIndex,
  isNewsItemUnseen,
  isStatusUnread,
  keyNewsRows,
  markStatusRead,
  newsCeilings,
  newsFilter,
  newsUnseenByServer,
  newsUnseenCount,
  sameStatusCursor,
  sanitizeStatusCursor,
  splitNewsSections,
  statusCeiling,
  statusCeilingFor,
  statusCursorFor,
  statusFeedOrder,
  statusIsObserved,
  statusSurfaceOpen,
  statusUnreadCount,
  type NewsItem,
  type StatusObservation,
  type StatusCursors,
  type StatusPost,
  type StatusReadCursor,
} from "./statusread.ts";

const NOW = 1_000_000;

/** An announcement. `get_statuses` returns these newest-first. */
const post = (id: string, ts: number): StatusPost => ({ id, ts, author: "nell", text: `post ${id}` });

/** A feed as the backend sends it: newest-first. */
const feed = (...pairs: [string, number][]): StatusPost[] => pairs.map(([id, ts]) => post(id, ts));

/**
 * The ceiling a feed's unread questions have to be asked against.
 *
 * Written out here rather than defaulted inside the module, because the whole failure this file
 * guards is a read and a write settling a row differently: a test that could forget the ceiling
 * would be a test that stopped checking they agree.
 */
const ceilingOf = (posts: StatusPost[], now = NOW): number => statusCeiling(posts, now);

/** How many of `posts` are unread against `cursor`, settled the way `cursor` was written. */
const unreadIn = (posts: StatusPost[], cursor: StatusReadCursor, now = NOW): number =>
  statusUnreadCount(posts, cursor, ceilingOf(posts, now));

// --- the cursor ---------------------------------------------------------------------------------

test("a cursor survives the round trip through the sealed record", () => {
  const cursor = markStatusRead(feed(["c", 950], ["b", 900], ["a", 800]), NOW);
  // The cursors ride inside the vault's UI-state JSON, so the round trip is that blob's.
  const back = sanitizeStatusCursor(JSON.parse(JSON.stringify(cursor)), NOW);
  assert.deepEqual(back, cursor);
  // And the round trip has to preserve the ANSWER, not just the fields: a cursor that reads back
  // to the same shape but stops excusing the post it was written from is a badge that never clears.
  const posts = feed(["c", 950], ["b", 900], ["a", 800]);
  assert.equal(unreadIn(posts, back), 0);
});

test("a corrupt or hand-edited cursor reads as never read, never as read-everything", () => {
  for (const bad of [null, undefined, "", "a string", 7, true, [1, 2, 3], {}]) {
    assert.deepEqual(sanitizeStatusCursor(bad, NOW), NO_STATUS_READ, JSON.stringify(bad ?? null));
  }
  // Unknown fields drop, bad values re-clamp, and the good half of a mixed entry still comes back.
  const mixed = sanitizeStatusCursor({
    ts: 900.7, ids: ["a", "a", "", 7, null, "b"], mode: "everything", nope: true,
  }, NOW);
  assert.deepEqual(mixed, { ts: 900, ids: ["a", "b"] });
  assert.equal((mixed as Record<string, unknown>).mode, undefined);
  // A `ts` that is not a believable moment must not become the boundary the whole feed is measured
  // against; falling back to zero shows the feed again, where trusting it would hide it forever.
  for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, -1, "900", null]) {
    assert.equal(sanitizeStatusCursor({ ts: bad, ids: [] }, NOW).ts, 0, JSON.stringify(bad));
  }
  // An entry claiming thousands of boundary ids is not a tie-break, it is a payload.
  const flood = sanitizeStatusCursor({ ts: 900, ids: Array.from({ length: 500 }, (_, i) => `id-${i}`) }, NOW);
  assert.equal(flood.ids.length, STATUS_BOUNDARY_IDS_MAX);
  // An id longer than anything this app mints is dropped rather than truncated: a shortened id
  // matches nothing, so it would sit in the set looking like a tie-break while never breaking one.
  const long = sanitizeStatusCursor({ ts: 900, ids: ["ok", "x".repeat(200)] }, NOW);
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
  assert.equal(unreadIn(posts, NO_STATUS_READ), 3);
  assert.equal(isStatusUnread(post("c", 950), NO_STATUS_READ, 950), true);
});

test("reading the feed clears it, and the next announcement raises it again", () => {
  const posts = feed(["c", 950], ["b", 900], ["a", 800]);
  const cursor = markStatusRead(posts, NOW);
  assert.equal(unreadIn(posts, cursor), 0);
  const later = [post("d", 1_200), ...posts];
  assert.equal(unreadIn(later, cursor), 1);
  assert.equal(isStatusUnread(post("d", 1_200), cursor, ceilingOf(later)), true);
});

test("two announcements in the same millisecond do not collapse at the boundary", () => {
  // The failure a bare timestamp cannot see: both stamps are 900 and the mark is 900, so no
  // comparison of clocks anywhere can tell that a second announcement was posted. The id can.
  const posts = feed(["first", 900], ["old", 500]);
  const cursor = markStatusRead(posts, NOW);
  assert.deepEqual(cursor.ids, ["first"]);
  assert.equal(isStatusUnread(post("second", 900), cursor, ceilingOf(posts)), true);
  assert.equal(isStatusUnread(post("first", 900), cursor, ceilingOf(posts)), false);
});

test("an empty feed is a no-op, not an event that marks a year of announcements read", () => {
  assert.deepEqual(markStatusRead([], NOW), NO_STATUS_READ);
  assert.equal(statusUnreadCount([], NO_STATUS_READ, 0), 0);
  assert.equal(firstStatusUnreadIndex([], NO_STATUS_READ, 0), -1);
  // The real shape of this: the surface opens, the fetch fails, the feed comes back empty, and
  // marking read must not walk the mark backwards to zero and un-read everything already seen.
  const read = markStatusRead(feed(["c", 950], ["b", 900]), NOW);
  assert.deepEqual(markStatusRead([], NOW, read), read);
  // Nor may a short or older page do it. Opening a filtered view is not forgetting what was read.
  assert.deepEqual(markStatusRead(feed(["a", 100]), NOW, read).ts, read.ts);
  assert.equal(unreadIn(feed(["c", 950], ["b", 900]), markStatusRead([], NOW, read)), 0);
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
  const withReal = [post("real", 950), ...posts];
  assert.equal(isStatusUnread(post("real", 950), cursor, ceilingOf(withReal)), true);
  assert.equal(unreadIn(withReal, cursor), 1);

  // And the hostile row does not stick as a permanent unread divider nothing can clear: this read
  // covered it, its id says so, and no clock is consulted to find that out.
  assert.equal(isStatusUnread(post("evil", evil), cursor, ceilingOf(posts)), false);
  assert.equal(unreadIn(posts, cursor), 0);
  assert.equal(firstStatusUnreadIndex(posts, cursor, ceilingOf(posts)), -1);

  // A SECOND row from that clock is a new announcement, not the one this read covered, and the id
  // set is what can still tell them apart. Swallowing it would let one wrong clock retire a feed.
  const again = [post("evil-2", evil - 1), ...posts];
  assert.equal(isStatusUnread(post("evil-2", evil - 1), cursor, ceilingOf(again)), true);
});

test("a sender a little ahead is ordinary skew and still counts", () => {
  // Two desktops are seconds apart, not years, and refusing that would hide real announcements.
  const nearly = NOW + 60_000;
  assert.ok(nearly < NOW + CLOCK_SKEW_GRACE_MS, "the point of the case is that it is inside the grace");
  const cursor = markStatusRead(feed(["ahead", nearly], ["a", 900]), NOW);
  assert.equal(cursor.ts, nearly);
  assert.equal(unreadIn(feed(["ahead", nearly], ["a", 900]), cursor), 0);
});

test("a feed stamped entirely from the future is still readable", () => {
  // Every post past the grace, so there is no plausible ceiling anywhere in the feed. The mark
  // cannot advance on a clock, and the ids are what stop the count sticking at three forever.
  const far = NOW + 400 * 60_000;
  const posts = feed(["c", far + 2], ["b", far + 1], ["a", far]);
  assert.equal(unreadIn(posts, NO_STATUS_READ), 3);
  const cursor = markStatusRead(posts, NOW);
  assert.equal(unreadIn(posts, cursor), 0, "reading a skewed feed must actually clear it");
});

test("a millisecond holding more announcements than the boundary set still reaches zero", () => {
  // A seeded or migrated server stamps its whole feed alike. Truncating the id set would leave the
  // overflow permanently unread with no way to clear, so the mark steps past the boundary instead.
  const many = Array.from({ length: STATUS_BOUNDARY_IDS_MAX + 20 }, (_, i) => post(`p${i}`, 900));
  const cursor = markStatusRead(many, NOW);
  assert.equal(cursor.ts, 901);
  assert.deepEqual(cursor.ids, []);
  assert.equal(unreadIn(many, cursor), 0);
  // Still a mark, not a surrender: anything genuinely newer is unread, and a second read is stable.
  const later = [post("later", 1_000), ...many];
  assert.equal(isStatusUnread(post("later", 1_000), cursor, ceilingOf(later)), true);
  assert.deepEqual(markStatusRead(many, NOW, cursor), cursor);
});

test("a feed with more rows above the ceiling than the set can name still reaches zero", () => {
  // The reported failure, in the shape it was reported: sixty-five announcements a wrong clock
  // parked four hundred minutes ahead, one ordinary post, and five reads. The boundary set took in
  // every row above the ceiling, overflowed, and the escape hatch threw away the ids that were
  // excusing them; the next read settled them at the ceiling again and found them above the mark
  // once more. The count sat at sixty-five with nothing in the app able to clear it, the NEW
  // divider parked at the bottom of the feed, and the mark crept one millisecond per read -- so
  // every announcement event anywhere re-sealed the whole UI-state record as well.
  //
  // A reader whose own clock is five minutes slow reaches this with no modified client anywhere:
  // the whole feed is "the future" to that machine, so any server with sixty-five posts trips it.
  const far = NOW + 400 * 60_000;
  const posts = [...Array.from({ length: 65 }, (_, i) => post(`f${i}`, far + i)), post("plain", 1_000)];
  let cursor = markStatusRead(posts, NOW);
  assert.equal(cursor.ts, 1_001, "the mark steps past the boundary rather than naming what it cannot");
  assert.equal(unreadIn(posts, cursor), 0);
  for (let i = 0; i < 5; i += 1) {
    const again = markStatusRead(posts, NOW, cursor);
    assert.equal(again.ts, 1_001, `read ${i + 2} walked the mark forward`);
    assert.equal(sameStatusCursor(again, cursor), true, `read ${i + 2} would re-seal the vault`);
    assert.equal(unreadIn(posts, again), 0, `read ${i + 2} left the feed unread`);
    cursor = again;
  }
});

test("every feed converges, holds still, and still announces what is genuinely new", () => {
  // The three properties a cursor owes ANY feed, whatever clock stamped it and however large it is.
  // Written as a table because the failures above were all one shape -- some adversarial feed where
  // the write settled a row one way and the read settled it another -- and the only defence against
  // the next one of those is asking every feed the same three questions.
  const far = NOW + 400 * 60_000;
  const future = (n: number) => Array.from({ length: n }, (_, i) => post(`f${i}`, far + i));
  const cases: [string, StatusPost[]][] = [
    ["empty", []],
    ["ordinary", feed(["c", 950], ["b", 900], ["a", 800])],
    ["every row above the ceiling", future(200)],
    ["the cap exactly, above the ceiling", [...future(STATUS_BOUNDARY_IDS_MAX - 1), post("plain", 900)]],
    ["one past the cap, above the ceiling", [...future(STATUS_BOUNDARY_IDS_MAX), post("plain", 900)]],
    ["far past the cap, above the ceiling", [...future(200), post("plain", 900)]],
    ["one millisecond, far past the cap", Array.from({ length: 200 }, (_, i) => post(`p${i}`, 900))],
    ["plausible and implausible mixed", [...future(70), ...feed(["c", 950], ["b", 900], ["a", 900])]],
    ["nothing carrying an id", Array.from({ length: 70 }, (_, i) => post("", far + i))],
  ];
  for (const [name, posts] of cases) {
    // Convergence: reading a feed clears it. A count with no way of reaching zero is the bug.
    let cursor = markStatusRead(posts, NOW);
    assert.equal(unreadIn(posts, cursor), 0, `${name}: reading it did not clear it`);
    // Idempotence: re-reading an unchanged feed says the same thing, so nothing is sealed again.
    for (let i = 0; i < 5; i += 1) {
      const again = markStatusRead(posts, NOW, cursor);
      assert.equal(sameStatusCursor(again, cursor), true, `${name}: read ${i + 2} moved the mark`);
      assert.equal(unreadIn(posts, again), 0, `${name}: read ${i + 2} un-read the feed`);
      cursor = again;
    }
    // Safety: an announcement this machine finds plausible, posted after the mark, is still new.
    const after = [post("new", 1_500), ...posts];
    assert.equal(isStatusUnread(post("new", 1_500), cursor, ceilingOf(after)), true, `${name}: lost a new post`);
    // And the grown feed converges in its turn, from whatever the mark had become.
    const read = markStatusRead(after, NOW, cursor);
    assert.equal(unreadIn(after, read), 0, `${name}: the grown feed would not clear`);
  }
});

test("the overflow escape hatch re-announces rather than swallowing, and settles again", () => {
  // What stepping the mark past the boundary gives up, said plainly. Rows a wrong clock put above
  // the ceiling are covered because the ceiling is where they settle, so a ceiling that later RISES
  // -- an ordinary announcement arrives, or this machine's clock catches up with those stamps --
  // puts them above the mark and they announce themselves once more. That is the direction to fail
  // in: one read covers them again, where a mark nothing could ever catch up with retires the feed.
  const far = NOW + 400 * 60_000;
  const future = Array.from({ length: 80 }, (_, i) => post(`f${i}`, far + i));
  const first = markStatusRead(future, NOW);
  assert.equal(unreadIn(future, first), 0);
  const grown = [post("plain", 1_200), ...future];
  assert.ok(unreadIn(grown, first) > 1, "the rows above the old ceiling are shown once more");
  const second = markStatusRead(grown, NOW, first);
  assert.equal(unreadIn(grown, second), 0, "and one read is all it takes to cover them again");
  assert.equal(sameStatusCursor(markStatusRead(grown, NOW, second), second), true);
});

test("a mark written under a wrong clock does not outlive the clock", () => {
  // A machine a year fast reads a feed stamped by a sender a year fast, and writes a mark a year
  // ahead. Nothing walks a mark backwards, so once the clock is corrected every genuine
  // announcement is older than the mark and reads as already seen: the divider, both counts and the
  // billboard beacon go quiet for a year. Chat survives the same insult because its scan falls
  // through to a clock-free comparison of message ids; a cursor has no such escape, so the mark
  // itself has to be the thing this machine refuses to believe.
  const year = 365 * 86_400_000;
  const wrong = NOW + year;
  const posted = markStatusRead(feed(["ahead", wrong]), wrong);
  assert.equal(posted.ts, wrong, "under its own clock that stamp was perfectly plausible");

  // The clock is put right. The very next read recovers it: a mark that far ahead of `now` is not
  // carried forward, so the honestly computed ceiling wins and the feed is measurable again.
  const corrected = markStatusRead(feed(["real", 950]), NOW, posted);
  assert.equal(corrected.ts, 950);
  assert.equal(isStatusUnread(post("later", 1_200), corrected, 1_200), true);

  // The door a stored cursor comes back through refuses it too, which is what recovers a server
  // nobody opens: without this the indicators stay silent, and silent indicators are why nobody
  // opens it. A hand-edited or half-written record arrives here with no clock involved at all.
  assert.deepEqual(sanitizeStatusCursor({ ts: wrong, ids: ["ahead"] }, NOW), NO_STATUS_READ);
  assert.deepEqual(sanitizeStatusCursor({ ts: 8.64e15, ids: [] }, NOW), NO_STATUS_READ);
  // The cost, out loud: announcements "read" under the bad clock announce themselves once more.
  // Reading a feed twice is an annoyance. A feed that never speaks again is a lost announcement.
  const posts = feed(["b", 900], ["a", 800]);
  assert.equal(unreadIn(posts, sanitizeStatusCursor({ ts: wrong, ids: [] }, NOW)), 2);

  // Ordinary skew is not a wrong clock. A mark inside the grace is kept, exactly as a stamp is.
  const skewed = { ts: NOW + 60_000, ids: ["ahead"] };
  assert.deepEqual(sanitizeStatusCursor(skewed, NOW), skewed);
  assert.equal(markStatusRead(feed(["a", 900]), NOW, skewed).ts, skewed.ts);
});

// --- the divider --------------------------------------------------------------------------------

test("the divider sits above the oldest unread announcement, whichever way the feed is ordered", () => {
  const cursor = markStatusRead(feed(["b", 900], ["a", 800]), NOW);
  // Newest-first, as `get_statuses` returns it: the oldest unread is the LAST unread index.
  const newestFirst = feed(["e", 1_400], ["d", 1_200], ["b", 900], ["a", 800]);
  const ceiling = ceilingOf(newestFirst);
  assert.equal(statusFeedOrder(newestFirst), "newest-first");
  assert.equal(firstStatusUnreadIndex(newestFirst, cursor, ceiling), 1, "the row `d`, above which the divider goes");

  // Reversed for rendering in reading order: the same announcement, the other end of the array.
  const oldestFirst = [...newestFirst].reverse();
  assert.equal(statusFeedOrder(oldestFirst), "oldest-first");
  assert.equal(firstStatusUnreadIndex(oldestFirst, cursor, ceiling), 2);
  assert.equal(oldestFirst[2].id, "d", "both orderings name the same announcement");
});

test("the divider has nowhere to go when nothing is unread, and says so", () => {
  const posts = feed(["b", 900], ["a", 800]);
  assert.equal(firstStatusUnreadIndex(posts, markStatusRead(posts, NOW), ceilingOf(posts)), -1);
  // A feed nobody has read is unread all the way down, so the divider goes above the oldest row.
  assert.equal(firstStatusUnreadIndex(posts, NO_STATUS_READ, ceilingOf(posts)), 1);
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
  assert.equal(firstStatusUnreadIndex(posts, NO_STATUS_READ, ceilingOf(posts), "oldest-first"), 0);
  assert.equal(firstStatusUnreadIndex(posts, NO_STATUS_READ, ceilingOf(posts), "newest-first"), 1);
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
  const items = [news(1, "status", 1_200), news(1, "status", 800), news(2, "status", 1_200), news(9, "status", 1)];
  const ceilings = newsCeilings(items, NOW);
  assert.equal(isNewsItemUnseen(news(1, "status", 1_200), cursors, ceilings), true);
  assert.equal(isNewsItemUnseen(news(1, "status", 800), cursors, ceilings), false);
  // The same timestamp, a different server: each server's mark answers only for its own feed.
  assert.equal(isNewsItemUnseen(news(2, "status", 1_200), cursors, ceilings), false);
  // A server with no cursor at all has never been read, so everything it posted is unseen.
  assert.equal(isNewsItemUnseen(news(9, "status", 1), cursors, ceilings), true);
  assert.deepEqual(statusCursorFor(cursors, 9), NO_STATUS_READ);
  assert.deepEqual(statusCursorFor({} as StatusCursors, 1), NO_STATUS_READ);
  // A ceiling belongs to one server's feed, so the map is keyed the way the cursors are, and a
  // server nobody posted to answers the same as one whose whole feed this machine cannot believe.
  assert.deepEqual(ceilings, { 1: 1_200, 2: 1_200, 9: 1 });
  assert.equal(statusCeilingFor(ceilings, 4), 0);
  assert.equal(statusCeilingFor(newsCeilings([news(5, "status", NOW + 400 * 60_000)], NOW), 5), 0);
});

test("a News row without an id is seen at the boundary rather than stuck on it", () => {
  // The tab's rows are flattened from two feeds and carry no id today. Nothing can excuse such a
  // row from the mark, so the mark must not hold it: the row the cursor was written from would
  // otherwise stay unseen forever, and the rail badge above it would never reach zero.
  const cursors: StatusCursors = { 1: { ts: 900, ids: ["b"] } };
  const ceilings = newsCeilings([news(1, "status", 900), news(1, "status", 901)], NOW);
  assert.equal(isNewsItemUnseen(news(1, "status", 900), cursors, ceilings), false);
  assert.equal(isNewsItemUnseen(news(1, "status", 901), cursors, ceilings), true);
  // With an id the tie is settled properly instead: the read one is seen, its neighbour is not.
  assert.equal(isNewsItemUnseen(news(1, "status", 900, "b"), cursors, ceilings), false);
  assert.equal(isNewsItemUnseen(news(1, "status", 900, "c"), cursors, ceilings), true);
});

test("an event is never unseen, because its timestamp is when it starts", () => {
  // An event's `ts` is its scheduled start, not the moment it was created, so it is normally in the
  // future: measured against any announcement cursor it reads unseen until the day it happens, and
  // a mark advanced past it to shut it up would swallow every announcement posted since.
  const cursors: StatusCursors = { 1: { ts: 900, ids: [] } };
  const ceilings = newsCeilings([news(1, "status", 1_200)], NOW);
  assert.equal(isNewsItemUnseen(news(1, "event", NOW + 7 * 86_400_000), cursors, ceilings), false);
  assert.equal(isNewsItemUnseen(news(1, "event", 1_200), cursors, ceilings), false);
  assert.equal(isNewsItemUnseen(news(1, "status", 1_200), cursors, ceilings), true);
  // An event's start is never a ceiling either: it is not an announcement, so it cannot decide
  // where one settles. A feed of nothing but events leaves its server out of the map entirely.
  assert.deepEqual(newsCeilings([news(1, "event", NOW + 7 * 86_400_000)], NOW), {});
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
  // never disagree with the list under it. The ceilings come from the whole list either way: a
  // chip narrows what is counted, never what a row's stamp is settled against.
  const cursors: StatusCursors = { 1: { ts: 900, ids: [] }, 2: { ts: 900, ids: [] } };
  const ceilings = newsCeilings(items, NOW);
  assert.equal(newsUnseenCount(items.filter(newsFilter("all", 1)), cursors, ceilings), 1);
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
  const ceilings = newsCeilings(items, NOW);
  // Two on server 1, none on server 2 (already read past it), one on server 3 (never read).
  assert.equal(newsUnseenCount(items, cursors, ceilings), 3);
  assert.deepEqual(newsUnseenByServer(items, cursors, ceilings), { 1: 2, 3: 1 });
  // The billboards and the rail badge are the same pass, so no beacon can disagree with the total.
  const byServer = newsUnseenByServer(items, cursors, ceilings);
  const summed = Object.values(byServer).reduce((a, b) => a + b, 0);
  assert.equal(summed, newsUnseenCount(items, cursors, ceilings));
  // A server with nothing unseen is absent rather than a zero, so a beacon is never rendered for it.
  assert.equal(byServer[2], undefined);
  assert.deepEqual(newsUnseenByServer([], cursors, ceilings), {});
  assert.equal(newsUnseenCount([], cursors, ceilings), 0);
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

test("two id-less announcements in one millisecond are two rows, not one key", () => {
  // A keyed `{#each}` raises `each_key_duplicate` on a repeat, in production as well as in
  // development, and the throw takes the whole News render with it rather than the one row. Two
  // posts old enough to predate minted ids, from one server in one millisecond, describe themselves
  // identically -- so a key spelled out of the row's own fields is a blank Inbox waiting to happen.
  const rows = [
    news(1, "status", 900),
    news(1, "status", 900),
    news(1, "status", 900, "minted"),
    news(2, "status", 900),
    news(1, "event", 900),
  ];
  const keyed = keyNewsRows(rows);
  assert.equal(new Set(keyed.map((r) => r.key)).size, rows.length, "every row has a key of its own");
  // Stable as well as unique: a re-aggregation of the same list hands every row the key it already
  // had, so the list is moved rather than torn down and rebuilt underneath whoever is reading it.
  assert.deepEqual(keyNewsRows(rows).map((r) => r.key), keyed.map((r) => r.key));
  // The row is otherwise untouched, and the key survives the cut into the sections that render it.
  assert.equal(keyed[2].id, "minted");
  const sections = splitNewsSections(keyed.map((r) => ({ ...r, pinned: false })), 30);
  assert.equal(sections.feed.every((r) => typeof r.key === "string" && r.key.length > 0), true);
  assert.deepEqual(keyNewsRows([]), []);
});

test("reading one server's announcements clears only that server's beacon", () => {
  const posts = feed(["c", 1_200], ["b", 1_100]);
  const items = [news(1, "status", 1_200), news(1, "status", 1_100), news(2, "status", 1_200)];
  const cursors: StatusCursors = { 1: NO_STATUS_READ, 2: NO_STATUS_READ };
  const ceilings = newsCeilings(items, NOW);
  assert.deepEqual(newsUnseenByServer(items, cursors, ceilings), { 1: 2, 2: 1 });
  cursors[1] = markStatusRead(posts, NOW, cursors[1]);
  assert.deepEqual(newsUnseenByServer(items, cursors, ceilings), { 2: 1 });
  assert.equal(newsUnseenCount(items, cursors, ceilings), 1);
});
