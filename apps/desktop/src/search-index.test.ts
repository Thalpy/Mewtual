import { test } from "node:test";
import assert from "node:assert/strict";

import {
  dayBound,
  findMatches,
  messageKinds,
  noSearchFilters,
  reactionCount,
  searchFilterCount,
  searchIsEmpty,
  textMatcher,
  type SearchCorpusChannel,
  type SearchMessage,
  type SearchSpec,
} from "./search-index.ts";

const ME = "me-fp";
const message = (over: Partial<SearchMessage> & { id: string }): SearchMessage => ({
  author: "bob",
  text: "",
  ts: 1_000,
  edited: 0,
  reactions: [],
  reply_to: "",
  pinned: false,
  ...over,
});

function spec(over: Partial<SearchSpec> = {}): SearchSpec {
  return {
    query: "",
    filters: noSearchFilters(),
    myFp: ME,
    myMentionName: "Me",
    mentionMark: "",
    mimeByCid: {},
    replyCounts: {},
    ...over,
  };
}

const corpus: SearchCorpusChannel[] = [
  {
    ch: "general",
    rows: [
      message({ id: "a", text: "the quick brown fox", ts: 10 }),
      message({ id: "b", author: ME, text: "my own note", ts: 20, edited: 30 }),
      message({ id: "c", text: "quick again, @[Me] look", ts: 30, reply_to: "a", pinned: true }),
      message({ id: "d", text: "see https://example.com/x", ts: 40, reactions: [{ emoji: "👍", by: [ME, "bob"] }] }),
    ],
  },
  { ch: "design", rows: [message({ id: "e", text: "quick sketch", ts: 50 })] },
];

test("an empty search selects nothing, and filters alone are a search", () => {
  assert.equal(searchIsEmpty(spec()), true);
  assert.deepEqual(findMatches(corpus, spec()), []);
  const pinned = { ...noSearchFilters(), isPinned: true };
  assert.equal(searchIsEmpty(spec({ filters: pinned })), false);
  assert.equal(searchFilterCount(pinned), 1);
  assert.equal(searchFilterCount(noSearchFilters()), 0);
  assert.equal(searchFilterCount({ ...noSearchFilters(), sort: "newest", caseSensitive: true }), 0);
});

test("text matching spans channels and honours the case and whole-word modifiers", () => {
  const hits = findMatches(corpus, spec({ query: "quick" }));
  assert.deepEqual(hits, [
    { ch: "general", idx: 0 },
    { ch: "general", idx: 2 },
    { ch: "design", idx: 0 },
  ]);
  assert.equal(findMatches(corpus, spec({ query: "QUICK" })).length, 3);
  const cased = { ...noSearchFilters(), caseSensitive: true };
  assert.equal(findMatches(corpus, spec({ query: "QUICK", filters: cased })).length, 0);
  const whole = { ...noSearchFilters(), wholeWord: true };
  assert.equal(findMatches(corpus, spec({ query: "quic", filters: whole })).length, 0);
  assert.equal(findMatches(corpus, spec({ query: "quick", filters: whole })).length, 3);
  assert.equal(textMatcher("   ", noSearchFilters()), null, "a blank query is not a predicate");
});

test("each facet narrows on what it names", () => {
  const only = (over: Partial<ReturnType<typeof noSearchFilters>>, extra: Partial<SearchSpec> = {}) =>
    findMatches(corpus, spec({ filters: { ...noSearchFilters(), ...over }, ...extra })).map(
      (hit) => corpus.find((c) => c.ch === hit.ch)!.rows[hit.idx].id,
    );
  assert.deepEqual(only({ fromMe: true }), ["b"]);
  assert.deepEqual(only({ from: "bob" }), ["a", "c", "d", "e"]);
  assert.deepEqual(only({ isPinned: true }), ["c"]);
  assert.deepEqual(only({ isEdited: true }), ["b"]);
  assert.deepEqual(only({ isReply: true }), ["c"]);
  assert.deepEqual(only({ mentionsMe: true }), ["c"]);
  assert.deepEqual(only({ reacted: true }), ["d"]);
  assert.deepEqual(only({ reactedByMe: true }), ["d"]);
  assert.deepEqual(only({ emoji: "👍" }), ["d"]);
  assert.deepEqual(only({ hasLink: true }), ["d"]);
  assert.deepEqual(only({ hasReplies: true }, { replyCounts: { a: 1 } }), ["a"]);
  assert.deepEqual(only({ isReply: true }, { mentionMark: "@[Nobody]" }), []);
  // Bounds are inclusive local days, so a message on the boundary day is kept.
  const day = new Date(30).toISOString().slice(0, 10);
  assert.ok(dayBound(day, false)! <= dayBound(day, true)!);
  assert.equal(dayBound("2026-13", false), null);
});

test("embeds are classified by the file index, and anything unknown is an attachment", () => {
  const text = "![a](cid:AB) ![b](cid:cd) ![c](cid:ef)";
  const kinds = messageKinds(text, { ab: "image/png", cd: "video/mp4", ef: "application/zip" });
  assert.deepEqual(kinds, { image: true, video: true, audio: false, file: true, link: false });
  assert.equal(messageKinds("![x](cid:99)", {}).file, true, "an address not in the index yet");
  assert.equal(reactionCount(message({ id: "r", reactions: [{ emoji: "x", by: ["a", "b"] }] })), 2);
});
