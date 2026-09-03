import { test } from "node:test";
import assert from "node:assert/strict";

import {
  firstDurableId,
  pageOfList,
  planJump,
  planRefresh,
  planRevealNewer,
  planRevealOlder,
  reanchorByIndex,
  unreadSummaryOf,
  type PageableMessage,
} from "./message-paging.ts";
import { readCeiling, effectiveTs } from "./unread.ts";

const ME = "me-fp";
const row = (id: string, author: string, ts: number, text = id, reply_to = ""): PageableMessage => ({
  id,
  author,
  text,
  ts,
  reply_to,
});

test("a refresh at the tail keeps at least what was loaded; away from it, it re-anchors at the first row", () => {
  const fresh = planRefresh(null, true, 320, 200);
  assert.deepEqual(fresh, { anchor: { kind: "tail" }, before: 319, after: 0 });
  const grown = planRefresh({ start: 100, ids: Array(500).fill("x").map((_, i) => `m${i}`), tailLoaded: true }, true, 320, 200);
  assert.deepEqual(grown, { anchor: { kind: "tail" }, before: 499, after: 0 });
  const away = planRefresh({ start: 100, ids: ["pending:1", "m7", "m8"], tailLoaded: false }, false, 320, 200);
  assert.deepEqual(away, { anchor: { kind: "id", id: "m7" }, before: 0, after: 202 });
  const legacy = planRefresh({ start: 5, ids: ["", ""], tailLoaded: false }, false, 320, 200);
  assert.deepEqual(legacy, { anchor: { kind: "index", index: 5 }, before: 0, after: 201 });
  assert.deepEqual(reanchorByIndex(away, 100).anchor, { kind: "index", index: 100 });
});

test("reveal plans keep the loaded rows and extend one side by one step", () => {
  const slice = { start: 40, ids: ["a", "b", "c"], tailLoaded: false };
  assert.deepEqual(planRevealOlder(slice, 200), { anchor: { kind: "id", id: "a" }, before: 200, after: 2 });
  assert.deepEqual(planRevealNewer(slice, 200), { anchor: { kind: "id", id: "c" }, before: 2, after: 200 });
  assert.deepEqual(planJump({ kind: "id", id: "q" }, 320), { anchor: { kind: "id", id: "q" }, before: 40, after: 279 });
  assert.equal(firstDurableId(["pending:1", "", "z"]), "z");
});

test("the reference page query slices around every anchor kind and resolves row context", () => {
  const list = [
    row("mine", ME, 1, "opener"),
    ...Array.from({ length: 5 }, (_, i) => row(`f${i}`, "bob", 2 + i, "x".repeat(300))),
    row("reply", "bob", 10, "re", "mine"),
    row("reply2", "bob", 11, "re2", "f0"),
    row("mention", "bob", 12, "@[Me] hi"),
    row("plain", "bob", 13, "nothing"),
  ];
  const q = (anchor: Parameters<typeof pageOfList>[1]["anchor"], before: number, after: number) =>
    pageOfList(list, { anchor, before, after }, ME, "@[Me]", 7, null);
  const ids = (p: { rows: { id: string }[] }) => p.rows.map((r) => r.id);

  const tail = q({ kind: "tail" }, 2, 0);
  assert.deepEqual([tail.total, tail.start, tail.anchor_index, ids(tail)], [10, 7, 9, ["reply2", "mention", "plain"]]);
  const around = q({ kind: "id", id: "f2" }, 1, 1);
  assert.deepEqual([around.start, ids(around)], [2, ["f1", "f2", "f3"]]);
  const missing = q({ kind: "id", id: "nope" }, 3, 3);
  assert.deepEqual([missing.total, missing.anchor_index, missing.rows.length], [10, null, 0]);
  const clamped = q({ kind: "index", index: 999 }, 0, 9);
  assert.deepEqual([clamped.start, ids(clamped)], [9, ["plain"]]);
  assert.deepEqual(ids(q({ kind: "first_reply_to", id: "f0" }, 0, 0)), ["reply2"]);
  assert.equal(pageOfList([], { anchor: { kind: "tail" }, before: 1, after: 1 }, ME, null, 0, null).anchor_index, null);

  const whole = q({ kind: "index", index: 0 }, 0, 100);
  const by = (id: string) => whole.rows.find((r) => r.id === id)!;
  assert.equal(by("mine").reply_count, 1);
  assert.equal(by("plain").reply_count, 0);
  assert.equal(by("reply").targets_me, true, "a reply to my message addresses me");
  assert.equal(by("reply").reply_to_preview?.author, ME);
  assert.equal(by("reply2").targets_me, false);
  assert.equal(by("reply2").reply_to_preview?.text.length, 200, "the parent's text is cut to the preview length");
  assert.equal(by("mention").targets_me, true);
  assert.equal(by("mine").targets_me, false, "my own row is never addressed to me");
  assert.equal(by("mine").reply_to_preview, null);
});

test("the unread summary agrees with the client's ceiling rule and ignores my own rows", () => {
  const now = 1_000_000;
  const list = [
    row("a", "bob", now - 10),
    row("b", ME, now - 5),
    row("c", "bob", now - 1),
    row("future", "bob", now + 60 * 60_000),
  ];
  const summary = unreadSummaryOf(list, ME, { divider_ts: now - 8, now_ms: now });
  const ceiling = readCeiling(list.map((m) => m.ts), now);
  assert.equal(summary.ceiling_ts, ceiling);
  // Rows past the divider from somebody else: c, and the far-future row only after it is clamped.
  const expected = list.filter((m) => m.author !== ME && effectiveTs(m.ts, ceiling) > now - 8);
  assert.equal(summary.count, expected.length);
  assert.equal(summary.first_index, 2);
  const nothing = unreadSummaryOf(list, ME, { divider_ts: null, now_ms: now });
  assert.deepEqual([nothing.count, nothing.first_index], [0, null]);
  const page = pageOfList(list, { anchor: { kind: "tail" }, before: 0, after: 0 }, ME, null, 1, { divider_ts: now - 8, now_ms: now });
  assert.deepEqual(page.unread, summary);
});
