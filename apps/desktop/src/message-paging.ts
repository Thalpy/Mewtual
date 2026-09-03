/**
 * Paged history: the webview holds one contiguous slice of a channel and asks the actor for
 * bounded pages around durable anchors (message ids) instead of the whole log.
 *
 * Two things live here, both pure so they are testable without a webview:
 * - the client-side decisions (which anchor to refresh from, how to stitch a revealed page);
 * - a reference implementation of the native page query over an in-memory list, used by the
 *   visual fixture and by tests. It mirrors `Server::message_page` in `catcoms-app` exactly.
 */

import { CLOCK_SKEW_GRACE_MS } from "./unread.ts";

export type PageAnchor =
  | { kind: "tail" }
  | { kind: "id"; id: string }
  | { kind: "index"; index: number }
  | { kind: "first_reply_to"; id: string };

export type ReplyPreview = { id: string; author: string; text: string };

/** The fields a paged row carries beyond the plain message. */
export type PagedRowContext = {
  targets_me: boolean;
  reply_count: number;
  reply_to_preview: ReplyPreview | null;
};

export type UnreadSummary = { ceiling_ts: number; first_index: number | null; count: number };

export type MessagePage<Row> = {
  version: number;
  total: number;
  start: number;
  anchor_index: number | null;
  rows: Row[];
  unread: UnreadSummary | null;
};

export type UnreadProbe = { divider_ts: number | null; now_ms: number };

/** Characters of a reply parent carried in a preview (mirrors `REPLY_PREVIEW_CHARS`). */
export const REPLY_PREVIEW_CHARS = 200;

/** A request the client is about to make. */
export type PageRequest = { anchor: PageAnchor; before: number; after: number };

/**
 * Which page response is still allowed to replace what is on screen.
 *
 * Every path that can replace the loaded slice (open, refresh, reveal, jump, re-anchor) awaits a
 * promise, and those settle in whatever order the actor and the bridge produce them. Checking only
 * that the server and channel are unchanged lets a slow older response overwrite a newer one,
 * which silently puts the reader back on stale rows: an arrival, an edit or a moved unread summary
 * disappears until something else happens to refresh. Only the newest request issued for the
 * conversation may land, and leaving the conversation invalidates every request already made.
 *
 * Deliberately not a comparison of document versions. Two requests can carry the same version and
 * still need an order (two jumps to different places), and a version cannot say which one the
 * reader asked for last.
 */
export class PageAdmission {
  #current = 0;

  /** Claim the right to replace the slice. The newest claim wins. */
  begin(): number {
    this.#current += 1;
    return this.#current;
  }

  /** Whether a response for `token` may still be applied. */
  accepts(token: number): boolean {
    return token === this.#current;
  }

  /** Refuse every outstanding response, without claiming anything. */
  invalidate(): void {
    this.#current += 1;
  }
}

/** The loaded slice as the planner sees it. */
export type LoadedSlice = {
  /** Position of the first loaded row in the channel. */
  start: number;
  /** Ids of the loaded rows in order; a pending (unacknowledged) row has no durable id. */
  ids: readonly string[];
  /** Whether the slice reaches the channel's newest row. */
  tailLoaded: boolean;
};

const PENDING_PREFIX = "pending:";

/** The first durable id in a slice, if any: a pending row or a legacy id-less row is skipped. */
export function firstDurableId(ids: readonly string[]): string | null {
  for (const id of ids) if (id && !id.startsWith(PENDING_PREFIX)) return id;
  return null;
}

export function lastDurableId(ids: readonly string[]): string | null {
  for (let i = ids.length - 1; i >= 0; i -= 1) {
    const id = ids[i];
    if (id && !id.startsWith(PENDING_PREFIX)) return id;
  }
  return null;
}

/**
 * How to re-read a channel after it changed.
 *
 * At the tail (or on a fresh open) the newest rows are what matters, and the read keeps at least
 * the rows already held so revealed history is not dropped on every arrival. Away from the tail
 * the slice is re-anchored at its first row and reaches `step` rows further down, so rows that
 * merged inside or just below it are picked up without the top moving under the reader.
 */
export function planRefresh(
  slice: LoadedSlice | null,
  stickToBottom: boolean,
  initialRows: number,
  step: number,
): PageRequest {
  if (!slice || slice.ids.length === 0 || stickToBottom) {
    const keep = slice ? slice.ids.length : 0;
    return { anchor: { kind: "tail" }, before: Math.max(initialRows, keep) - 1, after: 0 };
  }
  const first = firstDurableId(slice.ids);
  const after = slice.ids.length - 1 + step;
  if (first === null) return { anchor: { kind: "index", index: slice.start }, before: 0, after };
  return { anchor: { kind: "id", id: first }, before: 0, after };
}

/** The fallback when an id anchor no longer names a row: hold position by index. */
export function reanchorByIndex(request: PageRequest, index: number): PageRequest {
  return { ...request, anchor: { kind: "index", index: Math.max(0, index) } };
}

/** The read that reveals `step` older rows above the slice while keeping what is loaded. */
export function planRevealOlder(slice: LoadedSlice, step: number): PageRequest {
  const first = firstDurableId(slice.ids);
  const anchor: PageAnchor = first === null ? { kind: "index", index: slice.start } : { kind: "id", id: first };
  return { anchor, before: step, after: Math.max(0, slice.ids.length - 1) };
}

/** The read that reveals `step` newer rows below the slice while keeping what is loaded. */
export function planRevealNewer(slice: LoadedSlice, step: number): PageRequest {
  const last = lastDurableId(slice.ids);
  const anchor: PageAnchor =
    last === null ? { kind: "index", index: slice.start + Math.max(0, slice.ids.length - 1) } : { kind: "id", id: last };
  return { anchor, before: Math.max(0, slice.ids.length - 1), after: step };
}

/**
 * The read that makes a target renderable: the same shape `windowAround` used to slice out of the
 * whole array, fetched instead. Some context stays above the target so a centred scroll does not
 * land at a hard edge.
 */
export function planJump(anchor: PageAnchor, rows: number): PageRequest {
  const before = Math.min(40, Math.floor(rows / 4));
  return { anchor, before, after: Math.max(0, rows - 1 - before) };
}

// ---- reference implementation of the native query, for the fixture and for tests ----------------

export type PageableMessage = {
  id: string;
  author: string;
  text: string;
  ts: number;
  reply_to: string;
};

/** Unread state by the client's own rule (see `unread.ts`), over a whole list. */
export function unreadSummaryOf(messages: readonly PageableMessage[], me: string, probe: UnreadProbe): UnreadSummary {
  const limit = probe.now_ms + CLOCK_SKEW_GRACE_MS;
  let ceiling = 0;
  for (const m of messages) if (Number.isFinite(m.ts) && m.ts <= limit && m.ts > ceiling) ceiling = m.ts;
  const divider = probe.divider_ts === null ? Number.POSITIVE_INFINITY : probe.divider_ts;
  let count = 0;
  let first: number | null = null;
  messages.forEach((m, index) => {
    if (m.author !== me && Math.min(m.ts, ceiling) > divider) {
      count += 1;
      if (first === null) first = index;
    }
  });
  return { ceiling_ts: ceiling, first_index: first, count };
}

/**
 * `Server::message_page` over an in-memory list. `mentionMarker` is the `@[Name]` marker for
 * `me`, or null when there is no usable name.
 */
export function pageOfList<M extends PageableMessage>(
  messages: readonly M[],
  request: PageRequest,
  me: string,
  mentionMarker: string | null,
  version: number,
  probe: UnreadProbe | null,
): MessagePage<M & PagedRowContext> {
  const total = messages.length;
  const page: MessagePage<M & PagedRowContext> = {
    version,
    total,
    start: 0,
    anchor_index: null,
    rows: [],
    unread: probe ? unreadSummaryOf(messages, me, probe) : null,
  };
  if (total === 0) return page;
  const byId = new Map<string, number>();
  const replyCounts = new Map<string, number>();
  messages.forEach((m, index) => {
    if (m.id && !byId.has(m.id)) byId.set(m.id, index);
    if (m.reply_to) replyCounts.set(m.reply_to, (replyCounts.get(m.reply_to) ?? 0) + 1);
  });
  let anchor: number | undefined;
  const a = request.anchor;
  if (a.kind === "tail") anchor = total - 1;
  else if (a.kind === "id") anchor = byId.get(a.id);
  else if (a.kind === "index") anchor = Math.min(Math.max(0, Math.trunc(a.index)), total - 1);
  else {
    const found = messages.findIndex((m) => m.reply_to === a.id);
    anchor = found >= 0 ? found : undefined;
  }
  if (anchor === undefined) return page;
  const start = Math.max(0, anchor - Math.max(0, Math.trunc(request.before)));
  const end = Math.min(total - 1, anchor + Math.max(0, Math.trunc(request.after)));
  page.anchor_index = anchor;
  page.start = start;
  page.rows = messages.slice(start, end + 1).map((m) => {
    const parentIndex = m.reply_to ? byId.get(m.reply_to) : undefined;
    const parent = parentIndex === undefined ? undefined : messages[parentIndex];
    const targets_me =
      m.author !== me && ((mentionMarker !== null && m.text.includes(mentionMarker)) || parent?.author === me);
    return {
      ...m,
      targets_me,
      reply_count: replyCounts.get(m.id) ?? 0,
      reply_to_preview: parent
        ? { id: parent.id, author: parent.author, text: Array.from(parent.text).slice(0, REPLY_PREVIEW_CHARS).join("") }
        : null,
    };
  });
  return page;
}
