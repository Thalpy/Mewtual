// Unit tests for the composer "+" picker's reference markers.
//
// Run with `npm test` (Node's built-in runner + type stripping; no extra dependencies).
//
// The thing worth pinning is the SEAM: refs.ts writes a marker, render.ts tokenizes it back. Those
// live in different files and are edited independently, so every builder here is checked against
// the renderer's own exported regexes rather than against a copy of them. A tightened tokenizer or
// a re-worded builder then fails loudly instead of silently emitting text that renders as literal
// markdown in someone's message.

import { test } from "node:test";
import assert from "node:assert/strict";

import { refLabel, fileMarker, statusMarker, wikiMarker, eventMarker, insertInto } from "./refs.ts";
import { REF_LINK_RE, EMBED_RE, WIKI_LINK_RE } from "./render.ts";

const CID = "deadbeef0123456789abcdef";
const STATUS_ID = "0f9e8d7c6b5a40312233445566778899";

// `assert.ok` doesn't narrow `RegExpExecArray | null` under svelte-check, so unwrap explicitly:
// `assert.fail` returns `never`, which does narrow, and the message names the offending marker.
function matched(m: RegExpExecArray | null, marker: string): RegExpExecArray {
  if (m === null) assert.fail(`marker did not match the renderer's grammar: ${marker}`);
  return m;
}

// --- refLabel ---------------------------------------------------------------------------------

test("refLabel strips the characters that would terminate a marker early", () => {
  assert.equal(refLabel("a]b[c"), "a b c");
  assert.equal(refLabel("line\nbreak"), "line break");
});

test("refLabel collapses whitespace and trims", () => {
  assert.equal(refLabel("  spaced   out  "), "spaced out");
});

test("refLabel bounds the label to the tokenizer's limit", () => {
  assert.equal(refLabel("x".repeat(200), 60).length, 60);
  assert.equal(refLabel("x".repeat(200), 48).length, 48);
});

// --- markers round-trip through the renderer's own grammar ------------------------------------

test("a non-media file builds a link the reflink tokenizer accepts", () => {
  const marker = fileMarker("notes.pdf", CID, false);
  const m = matched(REF_LINK_RE.exec(marker), marker);
  assert.equal(m[1], "notes.pdf");
  assert.equal(m[2], "file");
  assert.equal(m[3], CID);
});

test("a media file builds an embed the embed tokenizer accepts", () => {
  const marker = fileMarker("cat.png", CID, true);
  const m = matched(EMBED_RE.exec(marker), marker);
  assert.equal(m[1], "cat.png");
  assert.equal(m[2], CID);
});

test("a status post builds a link carrying its id", () => {
  const marker = statusMarker("shipped the thing", STATUS_ID);
  const m = matched(REF_LINK_RE.exec(marker), marker);
  assert.equal(m[2], "status");
  assert.equal(m[3], STATUS_ID);
});

test("an event builds a marker the renderer tokenizes as an event ref", () => {
  const marker = eventMarker("board-game night [session 2]", STATUS_ID);
  const m = matched(REF_LINK_RE.exec(marker), marker);
  assert.equal(m[2], "event");
  assert.equal(m[3], STATUS_ID);
  assert.ok(!m[1].includes("["), "label must not contain marker-breaking brackets");
});

test("a wiki page builds the [[Page]] form the wiki already uses", () => {
  const marker = wikiMarker("Onboarding Notes");
  const m = matched(WIKI_LINK_RE.exec(marker), marker);
  assert.equal(m[1], "Onboarding Notes");
  assert.equal(m[2], undefined, "an unpiped marker must not capture a label");
});

test("the renderer's grammar splits a piped [[Page|label]] into page and label", () => {
  const marker = `${wikiMarker("Onboarding Notes").slice(0, -2)}|start here]]`;
  const m = matched(WIKI_LINK_RE.exec(marker), marker);
  assert.equal(m[1], "Onboarding Notes");
  assert.equal(m[2], "start here");
});

// `|` now separates the page from the label, so a page whose name contains one would build a marker
// that links to the wrong page; the label sanitizer has to drop it like the brackets.
test("a page name containing a pipe cannot build a mislinking marker", () => {
  const marker = wikiMarker("Cats|Dogs");
  const m = matched(WIKI_LINK_RE.exec(marker), marker);
  assert.equal(m[1], "Cats Dogs");
  assert.equal(m[2], undefined);
});

// --- hostile labels still produce a valid marker -----------------------------------------------
// A file name or status body is arbitrary member-authored text. If it leaked a `]` into the label
// the marker would truncate and the rest would render as stray markdown, so this is the case the
// label sanitizer exists for.

test("a file name containing brackets still yields a matching marker", () => {
  const marker = fileMarker("re]port[2].pdf", CID, false);
  const m = matched(REF_LINK_RE.exec(marker), marker);
  assert.equal(m[3], CID);
  assert.ok(!m[1].includes("]"));
});

test("a multi-line status body still yields a matching marker", () => {
  const marker = statusMarker("first line\nsecond line", STATUS_ID);
  const m = matched(REF_LINK_RE.exec(marker), marker);
  assert.equal(m[3], STATUS_ID);
});

test("an over-long status body is truncated to a matching marker", () => {
  const marker = statusMarker("word ".repeat(200), STATUS_ID);
  const m = matched(REF_LINK_RE.exec(marker), marker);
  assert.ok(m[1].length <= 48);
});

test("an empty label falls back rather than emitting an unmatchable marker", () => {
  // REF_LINK_RE requires at least one label character, so a blank name must not produce `[]( … )`.
  assert.ok(REF_LINK_RE.test(fileMarker("", CID, false)));
  assert.ok(REF_LINK_RE.test(statusMarker("   ", STATUS_ID)));
  assert.ok(WIKI_LINK_RE.test(wikiMarker("")));
});

// --- caret splicing ----------------------------------------------------------------------------

test("inserting into an empty draft adds no leading space", () => {
  const { text, caret } = insertInto("", 0, 0, "[[Page]]");
  assert.equal(text, "[[Page]] ");
  assert.equal(caret, text.length);
});

test("inserting after a word separates it with a space", () => {
  const { text } = insertInto("see", 3, 3, "[[Page]]");
  assert.equal(text, "see [[Page]] ");
});

test("inserting after existing whitespace does not double it", () => {
  const { text } = insertInto("see ", 4, 4, "[[Page]]");
  assert.equal(text, "see [[Page]] ");
});

test("inserting mid-draft splices at the caret and keeps the tail", () => {
  const { text, caret } = insertInto("ab cd", 2, 2, "X");
  assert.equal(text, "ab X cd");
  assert.equal(text.slice(caret), " cd");
});

test("inserting over a selection replaces it", () => {
  const { text } = insertInto("keep DROP tail", 5, 9, "X");
  assert.equal(text, "keep X tail");
});

test("the caret lands just after the inserted text", () => {
  const { text, caret } = insertInto("hi", 2, 2, "[[P]]");
  assert.equal(text.slice(0, caret), "hi [[P]] ");
});
