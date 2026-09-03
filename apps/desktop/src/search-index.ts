/**
 * Message search: the part that reads every message.
 *
 * Search spans whole channel histories, so the scan is the one piece of UI work whose cost is set
 * by how much has been said rather than by what is on screen. Keeping it here, pure and free of
 * anything but its inputs, is what lets it run in a worker (`search-worker.ts`) instead of between
 * two keystrokes, and lets the filters be tested without a browser.
 *
 * Everything crossing to the worker is plain data: no closures, no DOM, no reactive state.
 */

import { safeMediaMime } from "./media-safe.ts";

export type SearchSort = "oldest" | "newest" | "author" | "reactions" | "replies";

/** One reaction as search sees it. */
export type SearchReaction = { emoji: string; by: string[] };

/** The fields of a message search reads. A superset is harmless; this names what is required. */
export type SearchMessage = {
  id: string;
  author: string;
  text: string;
  ts: number;
  edited: number;
  reactions: SearchReaction[];
  reply_to: string;
  pinned: boolean;
};

/** One channel's loaded history, in log order. */
export type SearchCorpusChannel = { ch: string; rows: SearchMessage[] };

/** Where a hit is: the channel, and the row's absolute index in that channel. */
export type SearchHitRef = { ch: string; idx: number };

/**
 * Facets that select messages. `sort` and the two match modifiers are deliberately absent from
 * the "n filters" count: they shape the query and the order, they do not narrow on their own.
 */
export const SEARCH_NON_FACETS = ["sort", "caseSensitive", "wholeWord"];

export function noSearchFilters() {
  return {
    channel: "", // "" = the open channel · "*" = every channel here · else a channel id
    from: "", // author fingerprint ("" = anyone)
    mentions: "", // a member the message @-mentions
    after: "", // yyyy-mm-dd (inclusive, local day)
    before: "", // yyyy-mm-dd (inclusive, local day)
    hasImage: false,
    hasVideo: false,
    hasAudio: false,
    hasFile: false, // a non-media attachment
    hasLink: false,
    isReply: false,
    hasReplies: false,
    isPinned: false,
    isEdited: false,
    mentionsMe: false,
    fromMe: false,
    reacted: false,
    reactedByMe: false,
    emoji: "", // a specific reaction emoji
    caseSensitive: false,
    wholeWord: false,
    sort: "oldest" as SearchSort,
  };
}

export type SearchFilters = ReturnType<typeof noSearchFilters>;

/**
 * How many facets are narrowing the result: drives the "Filters (n)" badge and lets an empty
 * query still search, because filters alone are a valid search.
 */
export function searchFilterCount(filters: SearchFilters): number {
  return Object.entries(filters).filter(
    ([key, value]) => !SEARCH_NON_FACETS.includes(key) && value !== "" && value !== false,
  ).length;
}

/** Everything the scan needs that is not the corpus. Plain data, so it can cross to a worker. */
export type SearchSpec = {
  query: string;
  filters: SearchFilters;
  /** This device's fingerprint, for "from me" and "reacted by me". */
  myFp: string;
  /** The `@[Name]` marker for this member, for "mentions me". Empty when there is no name yet. */
  myMentionName: string;
  /** The `@[Name]` marker for the "mentions" filter's chosen member. Empty when unset. */
  mentionMark: string;
  /** Content address (lowercase) to declared MIME, for classifying a message's embeds. */
  mimeByCid: Record<string, string>;
  /** Message id to how many messages reply to it, for the "has replies" facet. */
  replyCounts: Record<string, number>;
};

const EMBED_RE = /!\[[^\]]*\]\(cid:([0-9a-fA-F]{1,64})\)/g;

/**
 * What a message carries. `safeMediaMime` accepts only image, video and audio, so anything else,
 * and any content address not in the index yet, reads as a plain attachment: exactly how the
 * embed resolver treats it.
 */
export function messageKinds(text: string, mimeByCid: Record<string, string>) {
  const kinds = { image: false, video: false, audio: false, file: false, link: false };
  for (const match of text.matchAll(EMBED_RE)) {
    const mime = safeMediaMime(mimeByCid[match[1].toLowerCase()] ?? "");
    if (mime.startsWith("image/")) kinds.image = true;
    else if (mime.startsWith("video/")) kinds.video = true;
    else if (mime.startsWith("audio/")) kinds.audio = true;
    else kinds.file = true;
  }
  kinds.link = /\bhttps?:\/\/\S/i.test(text);
  return kinds;
}

/**
 * A yyyy-mm-dd date input to a local-time bound: the start of that day, or its last millisecond
 * so "before" is inclusive. Null for an empty or half-typed value, which disables the bound.
 */
export function dayBound(value: string, end: boolean): number | null {
  const parts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (!parts) return null;
  const day = new Date(Number(parts[1]), Number(parts[2]) - 1, Number(parts[3]));
  if (Number.isNaN(day.getTime())) return null;
  if (end) day.setHours(23, 59, 59, 999);
  return day.getTime();
}

export function reactionCount(message: SearchMessage): number {
  return message.reactions.reduce((total, reaction) => total + reaction.by.length, 0);
}

/**
 * The text predicate, honouring the case and whole-word modifiers. Null when there is no query,
 * so a caller can tell "match everything" from "match nothing".
 */
export function textMatcher(
  raw: string,
  modifiers: { caseSensitive: boolean; wholeWord: boolean },
): ((text: string) => boolean) | null {
  const query = raw.trim();
  if (!query) return null;
  if (!modifiers.wholeWord) {
    if (modifiers.caseSensitive) return (text) => text.includes(query);
    const lower = query.toLowerCase();
    return (text) => text.toLowerCase().includes(lower);
  }
  // `\b` misbehaves when the query starts or ends with punctuation, so bound on non-word-or-edge.
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`(?:^|\\W)${escaped}(?:\\W|$)`, modifiers.caseSensitive ? "" : "i");
  return (text) => pattern.test(text);
}

/** Whether this search asks for anything at all. An empty one matches nothing, not everything. */
export function searchIsEmpty(spec: SearchSpec): boolean {
  return !spec.query.trim() && searchFilterCount(spec.filters) === 0;
}

/**
 * Every message the search selects, in corpus order.
 *
 * Ordering is the caller's: sorting needs display names, which belong to the main thread, and
 * sorting the matches is cheap next to reading every message to find them.
 */
export function findMatches(
  corpus: readonly SearchCorpusChannel[],
  spec: SearchSpec,
): SearchHitRef[] {
  const out: SearchHitRef[] = [];
  if (searchIsEmpty(spec)) return out;
  const filters = spec.filters;
  const match = textMatcher(spec.query, filters);
  const after = dayBound(filters.after, false);
  const before = dayBound(filters.before, true);
  const wantKind =
    filters.hasImage || filters.hasVideo || filters.hasAudio || filters.hasFile || filters.hasLink;
  for (const channel of corpus) {
    for (let idx = 0; idx < channel.rows.length; idx += 1) {
      const m = channel.rows[idx];
      if (match && !match(m.text)) continue;
      if (filters.from && m.author !== filters.from) continue;
      if (filters.fromMe && m.author !== spec.myFp) continue;
      if (spec.mentionMark && !m.text.includes(spec.mentionMark)) continue;
      if (after !== null && m.ts < after) continue;
      if (before !== null && m.ts > before) continue;
      if (filters.isReply && !m.reply_to) continue;
      if (filters.hasReplies && !(m.id && spec.replyCounts[m.id])) continue;
      if (filters.isPinned && !m.pinned) continue;
      if (filters.isEdited && !m.edited) continue;
      if (filters.mentionsMe && !(spec.myMentionName && m.text.includes(`@[${spec.myMentionName}]`)))
        continue;
      if (filters.reacted && !m.reactions.length) continue;
      if (filters.reactedByMe && !m.reactions.some((r) => r.by.includes(spec.myFp))) continue;
      if (filters.emoji && !m.reactions.some((r) => r.emoji === filters.emoji)) continue;
      if (wantKind) {
        const kinds = messageKinds(m.text, spec.mimeByCid);
        if (filters.hasImage && !kinds.image) continue;
        if (filters.hasVideo && !kinds.video) continue;
        if (filters.hasAudio && !kinds.audio) continue;
        if (filters.hasFile && !kinds.file) continue;
        if (filters.hasLink && !kinds.link) continue;
      }
      out.push({ ch: channel.ch, idx });
    }
  }
  return out;
}

/** What the worker is asked to do. */
export type SearchRequest =
  | { type: "corpus"; corpus: SearchCorpusChannel[] }
  | { type: "query"; id: number; spec: SearchSpec };

/** What it answers. `id` names the query, so a late answer to an old one can be discarded. */
export type SearchResponse = { type: "result"; id: number; hits: SearchHitRef[] };

/**
 * Which read of a channel is still wanted.
 *
 * The corpus is a snapshot of a live conversation, so a channel can change while it is being
 * read. Dropping the stored snapshot is not enough on its own: the read that is already in
 * flight knows nothing about the change, and when it lands it looks exactly like a fresh one.
 * Stamping each read with the channel's revision at the moment it was issued is what tells the
 * two apart, so the answer that was overtaken is discarded rather than stored as current.
 *
 * A channel with nothing stored still counts. That is precisely the window where a first read is
 * outstanding and the conversation moves under it.
 */
export class CorpusRevisions {
  #wanted = new Map<string, number>();

  /** The revision to stamp a read with. Pass it back to {@link accepts} when the read lands. */
  issue(channel: string): number {
    return this.#wanted.get(channel) ?? 0;
  }

  /** Note that `channel` has changed, so any read already in flight for it is out of date. */
  invalidate(channel: string): void {
    this.#wanted.set(channel, this.issue(channel) + 1);
  }

  /** Whether a read stamped `issued` still describes the channel. */
  accepts(channel: string, issued: number): boolean {
    return this.issue(channel) === issued;
  }
}
