/**
 * Announcement read state, derived from state rather than remembered from events.
 *
 * Today the whole feed is answered by one in-memory boolean set by a listener while the app happens
 * to be running. It cannot say WHICH server posted or HOW MANY posts are waiting, it is gone on the
 * next launch, and anything posted while the app was closed never existed at all. Four surfaces
 * need better than that: a divider and a count on the Announcements surface, a count chip on the
 * surface-bar tab, per-row "unseen" marks in the Inbox News tab, and an unread beacon on each
 * server's billboard in the orbit view. Those are one question asked four ways -- which posts has
 * this person not seen -- and the only honest answer rebuilds it from the posts that exist against
 * a durable mark, every time, rather than from whatever the app was awake to witness.
 *
 * A post's `ts` is the SENDER's clock, exactly as a message's is (see `./unread`), so the mark is
 * never advanced to a raw maximum. `readCeiling` decides what this machine is willing to believe
 * first. Without it, one broken or hostile clock in a server's announcement feed writes the mark
 * years into the future and every real announcement after it fails the "newer than the mark" test
 * in silence: the indicator goes quiet exactly when it matters. `effectiveTs` then settles the
 * implausible row AT the ceiling, so it is covered by the read rather than sticking as a permanent
 * unread divider that nothing can clear.
 *
 * That settling belongs to BOTH sides of the question, which is why every unread check here takes
 * the feed's ceiling next to the cursor. A mark written from a settled row and read back against a
 * raw one is a mark that can never cover that row: the write says "the ceiling", the read says
 * "four hundred minutes from now", and the two never meet however many times the feed is read.
 * `statusCeiling` is the one definition, and a caller holding a feed and a clock is the only thing
 * that can supply it.
 *
 * The cursor carries a small set of post ids alongside the timestamp, and only for posts that
 * settle AT the mark. Announcements arrive in batches -- a seeded or migrated server stamps a whole
 * feed in one millisecond -- and a bare timestamp cannot tell two posts in the same millisecond
 * apart, so the second one is silently already read. An id is minted with the post and means the
 * same thing on every device, so it settles the tie with no clock involved. Rows a bad clock put
 * above the ceiling settle there too and are named the same way, which is what keeps a SECOND
 * implausible post announcing itself instead of being swallowed by the read that covered the first.
 * Nothing bounds how many of those a feed can hold, so the set has a cap and an escape hatch.
 *
 * A mark is refused in the other direction as well. Reading a feed while this machine's own clock
 * is a year fast writes a mark a year ahead, and every announcement for the next year then reads as
 * already seen: the same silence, arriving through the read rather than through the post. So a mark
 * further ahead of `now` than the skew grace is not believed -- `sanitizeStatusCursor` drops it on
 * the way in and `markStatusRead` refuses to carry it forward -- and a corrected clock recovers on
 * the very next read. The cost is that announcements "read" under the bad clock announce themselves
 * once more, and that is the trade this module wants: re-reading a feed is an annoyance, a feed
 * that never speaks again is a lost announcement.
 *
 * The caller owns storage, and that storage is the unlocked vault's UI-state record -- the sealed
 * blob that already carries composer drafts and chat read marks -- never plaintext localStorage.
 * How far someone has read is a reading habit, and this app's rule is that reading habits do not
 * fall back to plaintext: chat's marks were migrated out of `catcoms.readmarks` for exactly that
 * reason. So every server's cursor travels as one `statusCursors` map inside that record, hydrated
 * when the vault opens, written when a mark advances, and dropped when the screen locks.
 * `sanitizeStatusCursor` is the door an entry comes back through, so a hand-edited or half-written
 * one reads as "never read" instead of wedging the feed.
 *
 * All pure: no DOM, no Svelte, no storage access. Values in, values out.
 */

import { CLOCK_SKEW_GRACE_MS, effectiveTs, readCeiling } from "./unread.ts";

/**
 * The part of an announcement this module reads.
 *
 * Structural, so the app's full `Msg` (edits, reactions, replies, pins) passes without conversion
 * and without this module gaining an opinion about any of it.
 */
export type StatusPost = {
  id: string;
  ts: number;
  author: string;
  text: string;
};

/**
 * How far a person has read one server's announcements.
 *
 * `ts` is the newest TRUSTED timestamp covered by the read: trusted meaning it passed
 * `readCeiling`, not meaning any sender vouched for it. `ids` names the posts that settle on that
 * boundary, and holds nothing else.
 *
 * Read as a rule, against the post's SETTLED timestamp rather than its raw one: a post below `ts`
 * is read; a post at or above `ts` is read only if its id is in `ids`. A post with no id at all
 * cannot be excused that way, so for those the boundary itself counts as read -- see
 * `isStatusUnread`.
 */
export type StatusReadCursor = {
  ts: number;
  ids: string[];
};

/** A server whose announcements have never been read. Everything in the feed is unread. */
export const NO_STATUS_READ: StatusReadCursor = { ts: 0, ids: [] };

/**
 * How many ids the boundary set may hold.
 *
 * Generous for the real cases -- posts sharing an exact millisecond, and rows a wrong clock parked
 * above the ceiling -- and small enough that a corrupt or hostile blob cannot turn a read check
 * into a scan of thousands of strings. Overflow does not truncate the set and hope, because a feed
 * of 200 posts all settling alike would then leave 136 of them permanently unread with no way to
 * clear: `markStatusRead` steps the mark one millisecond past the boundary instead, which reads
 * every settled row as seen. Losing a tie is recoverable; a badge that can never be cleared is not.
 */
export const STATUS_BOUNDARY_IDS_MAX = 64;

/** Longer than any id this app mints. A blob carrying one is corrupt, and its ids are dropped. */
const ID_MAX_LENGTH = 128;

/**
 * Read a stored cursor defensively: unknown fields drop, bad values re-clamp, corrupt reads fresh.
 *
 * A cursor is the one thing standing between a person and re-reading a year of announcements, but
 * it is also a record a person can edit and a half-written value a crash can leave behind. Falling
 * back to "never read" is the safe direction: the worst case is a feed that announces itself once
 * more, where trusting a garbage `ts` is a feed that never announces itself again.
 *
 * Takes the already-parsed value rather than a string, because the cursors arrive as one field of
 * the sealed UI-state record and the JSON around them was decoded before this module ever sees it.
 * `now` is an argument like everywhere else here, but it defaults to this machine's clock because
 * this is the one door a stored mark comes through with no feed beside it to date it against, and a
 * door that can be opened without a clock is a door the upper clamp below can be walked past.
 */
export function sanitizeStatusCursor(
  value: unknown,
  now = Date.now(),
  grace = CLOCK_SKEW_GRACE_MS,
): StatusReadCursor {
  const out: StatusReadCursor = { ts: 0, ids: [] };
  if (!value || typeof value !== "object" || Array.isArray(value)) return out;
  const j = value as { ts?: unknown; ids?: unknown };
  if (typeof j.ts === "number" && Number.isFinite(j.ts) && j.ts > 0) out.ts = Math.floor(j.ts);
  // A mark cannot honestly claim to have read past what this machine believes is now. Without this
  // one read taken under a clock a year fast outlives the clock by a year -- no later read walks a
  // mark backwards -- and a hand-edited record reaches the same state without a clock at all. The
  // ids go with the timestamp: they describe a boundary that is no longer being kept.
  if (out.ts > now + grace) return { ts: 0, ids: [] };
  if (Array.isArray(j.ids)) {
    const seen = new Set<string>();
    for (const id of j.ids) {
      // Dropped rather than truncated: a shortened id matches nothing, so it would sit in the
      // set looking like a tie-break while quietly never breaking one.
      if (typeof id !== "string" || !id || id.length > ID_MAX_LENGTH) continue;
      if (seen.has(id)) continue;
      seen.add(id);
      out.ids.push(id);
      if (out.ids.length >= STATUS_BOUNDARY_IDS_MAX) break;
    }
  }
  return out;
}

/**
 * Do these two cursors say the same thing?
 *
 * Asked before a write, because advancing a mark is what schedules a vault save: a re-read of an
 * unchanged feed must not re-seal the whole UI-state record on every render of the surface. Order
 * within `ids` is not part of the answer -- the set describes one millisecond, and which order that
 * millisecond's posts were walked in is an accident of the feed rather than a change to the mark.
 */
export function sameStatusCursor(a: StatusReadCursor, b: StatusReadCursor): boolean {
  if (a.ts !== b.ts || a.ids.length !== b.ids.length) return false;
  const held = new Set(a.ids);
  for (const id of b.ids) if (!held.has(id)) return false;
  return true;
}

/**
 * The newest timestamp in a feed this machine is willing to believe, which every question about
 * that feed has to be asked against.
 *
 * One definition, called by `markStatusRead` on the way in and by every caller of the unread checks
 * on the way out, because the moment the two sides settle a row differently the row becomes
 * uncoverable: the read insists it is newer than any mark this machine will write, and the mark
 * insists it already covered it.
 */
export function statusCeiling(posts: { ts: number }[], now: number, grace = CLOCK_SKEW_GRACE_MS): number {
  const list = Array.isArray(posts) ? posts : [];
  return readCeiling(list.map((p) => (p ? p.ts : 0)), now, grace);
}

/** The shared rule behind every unread question here, so no two surfaces can answer differently. */
function unreadAgainst(id: string, ts: number, cursor: StatusReadCursor, ceiling: number): boolean {
  // Decided by id where there is one on both sides: no clock is consulted, so no clock can lie.
  if (id && cursor.ids.includes(id)) return false;
  // The settled reading, which is the one the mark was written from. A row stamped past the ceiling
  // is judged where the ceiling puts it rather than where its sender's clock claimed to be.
  const at = effectiveTs(ts, ceiling);
  if (!id) {
    // Nothing can excuse this post from the boundary, so the boundary must not hold it: `>=` here
    // would leave the post the mark was written from permanently unread with no way to clear it.
    return at > cursor.ts;
  }
  return at >= cursor.ts;
}

/** Has this person seen this announcement? `ceiling` is the feed's, from `statusCeiling`. */
export function isStatusUnread(post: StatusPost, cursor: StatusReadCursor, ceiling: number): boolean {
  if (!post) return false;
  const id = typeof post.id === "string" ? post.id : "";
  return unreadAgainst(id, post.ts, cursor, ceiling);
}

/** How many announcements this person has not seen: the surface count and the surface-bar chip. */
export function statusUnreadCount(posts: StatusPost[], cursor: StatusReadCursor, ceiling: number): number {
  if (!Array.isArray(posts)) return 0;
  let n = 0;
  for (const post of posts) if (isStatusUnread(post, cursor, ceiling)) n += 1;
  return n;
}

/**
 * Advance the mark to cover everything currently in the feed.
 *
 * The failure this guards is the one `./unread` was written for. An announcement's `ts` is injected
 * by whoever posted it, so a mark taken as the raw maximum hands one broken clock -- or one member
 * who picked the number -- the power to park the mark years ahead. Every genuine announcement after
 * that is older than the mark, reads as already seen, and the divider, the counts and the billboard
 * beacon all stay silent. `readCeiling` refuses to believe anything more than `CLOCK_SKEW_GRACE_MS`
 * past this machine's clock, so the mark can only ever reach the newest post that is actually
 * plausible, and the implausible row settles at that ceiling instead of above it.
 *
 * `previous` is required rather than assumed empty because the mark must never move backwards. The
 * Announcements surface can be opened against a short page, a filtered view, or a feed that failed
 * to load and came back empty, and in every one of those the newly computed ceiling is lower than
 * what has already been read. Taking the higher of the two makes an empty feed a no-op instead of
 * an event that marks a year of announcements unread again. The one mark it will not take forward
 * is one that is ahead of `now` by more than the grace, because "never backwards" applied to a mark
 * written under a wrong clock is how a read silences a feed for as long as the bad stamp said.
 */
export function markStatusRead(
  posts: StatusPost[],
  now: number,
  previous: StatusReadCursor = NO_STATUS_READ,
  grace = CLOCK_SKEW_GRACE_MS,
): StatusReadCursor {
  const list = Array.isArray(posts) ? posts : [];
  const ceiling = statusCeiling(list, now, grace);
  const held = previous.ts > now + grace ? NO_STATUS_READ : previous;
  const ts = held.ts > ceiling ? held.ts : ceiling;
  // Carried forward only when the mark did not move: ids describe one boundary, and the ids of a
  // boundary the mark has left behind describe nothing.
  const ids = new Set<string>(ts === held.ts ? held.ids : []);
  for (const post of list) {
    const id = post && typeof post.id === "string" ? post.id : "";
    if (!id || id.length > ID_MAX_LENGTH) continue;
    // The settled reading and nothing else, because that is the only reading an unread check can
    // ask. Naming a post by its raw stamp instead puts an id in the set for a row every later read
    // still finds above the mark, which is a count that walks the mark forward and never falls.
    if (effectiveTs(post.ts, ceiling) >= ts) ids.add(id);
  }
  // More posts settled on the boundary than the set may name. Stepping the mark one millisecond
  // past it reads every one of them as seen, which loses the tie for anything that lands on that
  // same boundary after this read: another post in the boundary millisecond, or -- while the
  // ceiling stays where it is -- another row from the clock that put these above it. Both re-appear
  // if the ceiling later rises past the mark, and announcing a read post again is the safe
  // direction. A count that can never reach zero is not.
  if (ids.size > STATUS_BOUNDARY_IDS_MAX) return { ts: ts + 1, ids: [] };
  return { ts, ids: [...ids] };
}

/**
 * Which way a feed is ordered, since the divider has to sit on the right side of the unread block.
 *
 * `get_statuses` returns newest-first and that is the default, but the surface reverses the array
 * to render it in reading order, so both orderings reach this module and guessing wrong puts the
 * divider at the opposite end of the feed.
 */
export type StatusFeedOrder = "newest-first" | "oldest-first";

/**
 * Read a feed's ordering off the feed itself.
 *
 * By majority of adjacent pairs rather than by the first pair that differs: one post out of order
 * (a sender's clock again) is not a reordered feed, and must not flip the answer. A feed that is
 * empty, single, or entirely tied is reported as `newest-first`, which is what the backend sends.
 */
export function statusFeedOrder(posts: StatusPost[]): StatusFeedOrder {
  if (!Array.isArray(posts)) return "newest-first";
  let descending = 0;
  let ascending = 0;
  for (let i = 1; i < posts.length; i += 1) {
    const a = posts[i - 1]?.ts ?? Number.NaN;
    const b = posts[i]?.ts ?? Number.NaN;
    if (!Number.isFinite(a) || !Number.isFinite(b)) continue;
    if (a > b) descending += 1;
    else if (a < b) ascending += 1;
  }
  return ascending > descending ? "oldest-first" : "newest-first";
}

/**
 * Where the "new announcements" divider goes: the index of the OLDEST unread post.
 *
 * `first` is by reading order, not by array position. In the newest-first array `get_statuses`
 * returns, the oldest unread post is the LAST unread index, not the first; in a reversed array it
 * is the first. Both are the row the divider sits above, and both are found here so the caller
 * does not have to reason about which array it happens to be holding. `-1` when nothing is unread.
 *
 * Position only. Whether a post is unread is `isStatusUnread`'s answer, and a divider a row out of
 * place is cosmetic in a way that a miscounted feed is not.
 */
export function firstStatusUnreadIndex(
  posts: StatusPost[],
  cursor: StatusReadCursor,
  ceiling: number,
  order: StatusFeedOrder = statusFeedOrder(posts),
): number {
  if (!Array.isArray(posts) || posts.length === 0) return -1;
  if (order === "newest-first") {
    for (let i = posts.length - 1; i >= 0; i -= 1) if (isStatusUnread(posts[i], cursor, ceiling)) return i;
    return -1;
  }
  for (let i = 0; i < posts.length; i += 1) if (isStatusUnread(posts[i], cursor, ceiling)) return i;
  return -1;
}

// --- is anybody actually there? -----------------------------------------------------------------

/**
 * The app state the Announcements surface's read behaviour is decided from.
 *
 * The same question `./unread`'s `ChatObservation` asks, minus the terms that only mean something
 * for a scrolling message log. Two answers come out of it rather than one, because the divider and
 * the durable mark are not the same concern and were the same boolean once.
 */
export type StatusObservation = {
  /** The content column's active surface; only "status" shows the announcement feed. */
  view: string;
  /** The cross-server inbox has taken the main pane. */
  inboxView: boolean;
  /** The orbit/server-space overlay is up. */
  spaceOpen: boolean;
  /** A DM, which has no noticeboard: the surface is not offered there at all. */
  isDm: boolean;
  windowFocused: boolean;
  /** `document.visibilityState === "visible"`: minimised or on another virtual desktop is not. */
  documentVisible: boolean;
};

/**
 * Is the announcement feed the surface on screen?
 *
 * The weaker of the two questions, and the one the NEW divider's boundary snapshot is tied to. It
 * must NOT answer to focus: the snapshot is taken on arrival and released on the way out, so a
 * window that answered "no" on every blur would release it and move the divider somebody left
 * mid-read, which is the whole thing the snapshot exists to hold still.
 */
export function statusSurfaceOpen(o: StatusObservation): boolean {
  return o.view === "status" && !o.inboxView && !o.spaceOpen && !o.isDm;
}

/**
 * Is somebody actually reading it?
 *
 * The stronger question, and the only one allowed to advance the durable mark. A window parked
 * unfocused on this surface still re-renders every time a post lands, and treating that as a read
 * marks an announcement nobody was there to see as read forever -- the failure `chatIsObserved`
 * was written for, arriving on the other surface.
 */
export function statusIsObserved(o: StatusObservation): boolean {
  return statusSurfaceOpen(o) && o.windowFocused && o.documentVisible;
}

// --- the Inbox News tab -------------------------------------------------------------------------

/** What a News row is about. */
export type NewsKind = "status" | "event";

/**
 * One row of the cross-server News tab.
 *
 * `id` is optional because the tab's rows are built by flattening two different feeds and the event
 * half has never carried one. A row without an id falls back to the timestamp, with the weaker
 * boundary behaviour `unreadAgainst` documents.
 */
export type NewsItem = {
  server: number;
  kind: NewsKind;
  ts: number;
  id?: string;
};

/** Per-server cursors, keyed the way the caller stores them. */
export type StatusCursors = Record<number, StatusReadCursor>;

/** One server's cursor, or the never-read one. */
export function statusCursorFor(cursors: StatusCursors, server: number): StatusReadCursor {
  const found = cursors ? cursors[server] : undefined;
  return found && Array.isArray(found.ids) && typeof found.ts === "number" ? found : NO_STATUS_READ;
}

/** Per-server read ceilings, the cross-server shape of what `statusCeiling` answers for one feed. */
export type StatusCeilings = Record<number, number>;

/**
 * Each server's ceiling, over the announcements one aggregation is holding.
 *
 * The News tab flattens every server's whole feed into one list, and a ceiling belongs to a feed:
 * measuring one server's rows against the newest plausible stamp in SOMEBODY ELSE'S feed settles
 * them somewhere the mark for their own server was never written from. Computed once for the whole
 * list rather than per chip, so filtering the tab cannot change what a row is measured against.
 */
export function newsCeilings(items: NewsItem[], now: number, grace = CLOCK_SKEW_GRACE_MS): StatusCeilings {
  const feeds = new Map<number, number[]>();
  for (const item of Array.isArray(items) ? items : []) {
    if (!item || item.kind !== "status") continue;
    const held = feeds.get(item.server);
    if (held) held.push(item.ts);
    else feeds.set(item.server, [item.ts]);
  }
  const out: StatusCeilings = {};
  for (const [server, timestamps] of feeds) out[server] = readCeiling(timestamps, now, grace);
  return out;
}

/**
 * One server's ceiling.
 *
 * Zero for a server that is not in the map, which is the same answer a server whose entire feed is
 * implausible gets: there is no stamp in it this machine will believe, so nothing settles above the
 * floor. Both mean the same thing to a read, so neither needs to be told apart from the other.
 */
export function statusCeilingFor(ceilings: StatusCeilings, server: number): number {
  const found = ceilings ? ceilings[server] : undefined;
  return typeof found === "number" && Number.isFinite(found) ? found : 0;
}

/**
 * Is this News row something the person has not seen?
 *
 * Only announcements can be unseen. An event row is deliberately never unseen, and not for want of
 * somewhere to put the state: an event's `ts` is its scheduled START, not the moment it was
 * created, so it is normally in the future and would read as unread against any cursor until the
 * day it happens -- and a cursor advanced past it to shut it up would mark every announcement
 * posted since as read too. The two are not on the same timeline and cannot share one mark. The
 * tab already surfaces events under "Upcoming", which is the indicator they actually want; when
 * events grow a creation timestamp of their own, this is where the second rule goes.
 */
export function isNewsItemUnseen(item: NewsItem, cursors: StatusCursors, ceilings: StatusCeilings): boolean {
  if (!item || item.kind !== "status") return false;
  const id = typeof item.id === "string" ? item.id : "";
  const server = item.server;
  return unreadAgainst(id, item.ts, statusCursorFor(cursors, server), statusCeilingFor(ceilings, server));
}

/** What the News tab's kind chips can select. */
export type NewsKindFilter = "all" | NewsKind;

/**
 * The predicate behind the News tab's chips: a kind, and optionally one server.
 *
 * A builder rather than a filtered array so the count, the list and the per-server beacon all run
 * the same rule over whatever they are holding, and a chip cannot end up meaning one thing in the
 * list and another in the badge above it.
 */
export function newsFilter(kind: NewsKindFilter, server?: number | null): (item: NewsItem) => boolean {
  const wantServer = typeof server === "number" && Number.isFinite(server) ? server : null;
  return (item: NewsItem) => {
    if (!item) return false;
    if (kind !== "all" && item.kind !== kind) return false;
    if (wantServer !== null && item.server !== wantServer) return false;
    return true;
  };
}

/**
 * The News tab's two announcement sections, cut so that no post can appear in both.
 *
 * The Pinned section shows one post per server -- the latest pin, because a pin from months ago has
 * already said its piece and the section is meant to be short enough to read past -- and "Recent
 * announcements" shows the rest, newest-first. The two are cut in one pass rather than filtered
 * independently, because filtering independently is what listed the latest pin twice: once for
 * being pinned and once for being recent. Exclusion is by identity, so an OLDER pin, which the
 * Pinned section is NOT showing, still appears in the feed at the position its timestamp earns.
 *
 * Generic over the row so the caller can keep the text, author and server name this module has no
 * opinion about. Events are dropped by both halves: this is the announcement half of the tab.
 */
export function splitNewsSections<T extends NewsItem & { pinned?: boolean }>(
  items: T[],
  limit: number,
): { pinned: T[]; feed: T[] } {
  const posts = Array.isArray(items) ? items.filter((item) => item && item.kind === "status") : [];
  const latest = new Map<number, T>();
  for (const item of posts) {
    if (!item.pinned) continue;
    const held = latest.get(item.server);
    if (!held || item.ts > held.ts) latest.set(item.server, item);
  }
  const pinned = [...latest.values()].sort((a, b) => b.ts - a.ts);
  const shown = new Set<T>(pinned);
  return { pinned, feed: posts.filter((item) => !shown.has(item)).sort((a, b) => b.ts - a.ts).slice(0, limit) };
}

/**
 * Stamp every News row with a render key that is unique BY CONSTRUCTION.
 *
 * A keyed `{#each}` needs one per row, and a key spelled out of the row's own fields is not one: an
 * announcement old enough to predate minted ids carries an empty one, so two of those from the same
 * server in the same millisecond describe themselves identically. Svelte raises `each_key_duplicate`
 * on that, in production as well as in development, and the throw takes the whole News render down
 * rather than the one row -- a pair of legacy posts for a blank tab. The exposure grew with the
 * aggregation, which now holds each server's whole feed instead of its five newest posts.
 *
 * The occurrence counter is what makes the key unique; the fields in front of it are what make it
 * STABLE, so a row keeps its identity across a re-aggregation and Svelte moves the node it already
 * has instead of tearing it down and building another.
 */
export function keyNewsRows<T extends NewsItem>(items: T[]): (T & { key: string })[] {
  const seen = new Map<string, number>();
  const out: (T & { key: string })[] = [];
  for (const item of Array.isArray(items) ? items : []) {
    if (!item) continue;
    const base = `${item.server}:${item.kind}:${item.id ?? ""}:${item.ts}`;
    const nth = seen.get(base) ?? 0;
    seen.set(base, nth + 1);
    out.push({ ...item, key: `${base}#${nth}` });
  }
  return out;
}

/** The News half of the left rail's inbox badge, to be summed with the mentions count. */
export function newsUnseenCount(items: NewsItem[], cursors: StatusCursors, ceilings: StatusCeilings): number {
  if (!Array.isArray(items)) return 0;
  let n = 0;
  for (const item of items) if (isNewsItemUnseen(item, cursors, ceilings)) n += 1;
  return n;
}

/**
 * The same count split by server, for the billboard beacons in the orbit view.
 *
 * One pass producing every billboard's number, so no server's beacon can disagree with the rail
 * badge that is summing them. Servers with nothing unseen are absent rather than present as `0`.
 */
export function newsUnseenByServer(
  items: NewsItem[],
  cursors: StatusCursors,
  ceilings: StatusCeilings,
): Record<number, number> {
  const out: Record<number, number> = {};
  if (!Array.isArray(items)) return out;
  for (const item of items) {
    if (!isNewsItemUnseen(item, cursors, ceilings)) continue;
    out[item.server] = (out[item.server] ?? 0) + 1;
  }
  return out;
}
