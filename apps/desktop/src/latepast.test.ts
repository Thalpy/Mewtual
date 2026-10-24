import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_LATE_PAST,
  NO_READ_MARK,
  addLatePast,
  clearLatePast,
  lateArrivals,
  type ReadMark,
} from "./unread.ts";
import { sanitizeUiContinuity } from "./ui-continuity.ts";

const me = "me";
const row = (id: string, ts: number, author = "them") => ({ id, ts, author });
const CEILING = 10_000;

test("an arrival that sorts at or before the read mark is late; one after it is ordinary unread", () => {
  const mark: ReadMark = { ts: 5_000, id: "read" };
  const late = lateArrivals(
    [row("before", 4_000), row("same-ms", 5_000), row("after", 6_000)],
    mark,
    me,
    CEILING,
  );
  assert.deepEqual(late, ["before", "same-ms"], "same millisecond is behind the cursor, not after it");
});

test("nothing is late in a conversation that has never been read", () => {
  // The first open pulls the whole history "into the past", and none of it was missed.
  assert.deepEqual(lateArrivals([row("a", 1), row("b", 2)], NO_READ_MARK, me, CEILING), []);
});

test("own messages and the read row itself are never late", () => {
  const mark: ReadMark = { ts: 5_000, id: "read" };
  const late = lateArrivals([row("mine", 1_000, me), row("read", 5_000), row("theirs", 1_000)], mark, me, CEILING);
  assert.deepEqual(late, ["theirs"]);
});

test("a late row stamped by a broken clock is clamped to the ceiling before it is placed", () => {
  // Read state clamps an implausible stamp to the newest plausible one; the same rule applies
  // here, or a future-stamped row could never be late even when it renders behind the cursor.
  const mark: ReadMark = { ts: CEILING, id: "read" };
  assert.deepEqual(lateArrivals([row("future", CEILING + 60_000)], mark, me, CEILING), ["future"]);
  // Below the ceiling the stamp is used as written, and a plausible later stamp is not late.
  assert.deepEqual(lateArrivals([row("later", CEILING + 60_000)], { ts: 5_000, id: "read" }, me, CEILING + 120_000), []);
});

test("a timestamp-only mark from an older build still decides lateness", () => {
  assert.deepEqual(lateArrivals([row("old", 10), row("new", 20)], { ts: 15, id: "" }, me, CEILING), ["old"]);
});

test("the late set deduplicates, appends newest last and forgets its oldest past the bound", () => {
  assert.deepEqual(addLatePast(["a"], ["a", "b", "", "b"]), ["a", "b"]);
  const many = Array.from({ length: MAX_LATE_PAST + 3 }, (_, i) => `m${i}`);
  const bounded = addLatePast([], many);
  assert.equal(bounded.length, MAX_LATE_PAST);
  assert.equal(bounded[0], "m3", "the oldest three are forgotten, not the newest");
  const same = ["x", "y"];
  assert.equal(addLatePast(same, []), same, "nothing to add returns the same list, so no save is scheduled");
});

test("seeing rows clears exactly those ids and leaves the list untouched otherwise", () => {
  const current = ["a", "b", "c"];
  assert.deepEqual(clearLatePast(current, ["b"]), ["a", "c"]);
  assert.equal(clearLatePast(current, ["zzz"]), current, "an unrelated id changes nothing");
  assert.equal(clearLatePast(current, []), current);
});

test("late ids survive the sealed continuity record, bounded and deduplicated", () => {
  const state = sanitizeUiContinuity({
    drafts: {},
    readMarks: {},
    latePast: {
      "1:general": ["a", "a", "", 7, "b"],
      "1:empty": [],
      "1:notalist": "a",
      ["k".repeat(257)]: ["hidden"],
      "1:long": Array.from({ length: MAX_LATE_PAST + 2 }, (_, i) => `id${i}`),
    },
  });
  assert.deepEqual(state.latePast["1:general"], ["a", "b"]);
  assert.equal(state.latePast["1:empty"], undefined);
  assert.equal(state.latePast["1:notalist"], undefined);
  assert.equal(Object.keys(state.latePast).length, 2);
  assert.equal(state.latePast["1:long"].length, MAX_LATE_PAST);
  assert.equal(state.latePast["1:long"][0], "id2", "an over-long list keeps its newest");
  assert.deepEqual(sanitizeUiContinuity({}).latePast, {}, "absent in an older record reads as empty");
});
