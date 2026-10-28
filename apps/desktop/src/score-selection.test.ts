import assert from "node:assert/strict";
import test from "node:test";
import { ScoreSelectionMemory, validScoreKey, type ScoreSelectionScope } from "./score-selection.ts";
import { SCORE_MAX_STEPS, SCORE_ROWS } from "./studio-contract.ts";

const scope = (object: string, extra: Partial<ScoreSelectionScope> = {}): ScoreSelectionScope =>
  ({ server: 1, group: "aa", channel: "channel", object, ...extra });
const selected = (key: string) => ({ selected: key, picked: [key] });

// The original document-only API is replaced by scoped, revocable writer ownership.
test("selection is recalled by scope and an empty selection forgets it", () => {
  const memory = new ScoreSelectionMemory();
  memory.openWriter(scope("a"), () => true).remember({ selected: "4:4", picked: ["4:4", "6:6"] });
  memory.openWriter(scope("b"), () => true).remember(selected("1:1"));
  const a = memory.openWriter(scope("a"), () => true);
  assert.deepEqual(a.recall(), { selected: "4:4", picked: ["4:4", "6:6"] });
  a.remember({ selected: null, picked: [] }); assert.equal(a.recall(), null);
  assert.deepEqual(memory.openWriter(scope("b"), () => true).recall(), selected("1:1"));
  assert.equal(memory.openWriter(scope("c"), () => true).recall(), null);
});

test("invalid keys are dropped and the least recently remembered scope goes first", () => {
  const memory = new ScoreSelectionMemory(2), a = memory.openWriter(scope("a"), () => true);
  a.remember({ selected: "9:99", picked: ["x", "256:0", "255:24", "1:2:3"] });
  assert.deepEqual(a.recall(), { selected: null, picked: ["255:24"] });
  memory.openWriter(scope("b"), () => true).remember(selected("0:0"));
  memory.openWriter(scope("a"), () => true).remember(selected("1:1"));
  memory.openWriter(scope("c"), () => true).remember(selected("2:2"));
  assert.equal(memory.openWriter(scope("b"), () => true).recall(), null);
  assert.deepEqual(memory.openWriter(scope("a"), () => true).recall(), selected("1:1"));
  assert.deepEqual(memory.openWriter(scope("c"), () => true).recall(), selected("2:2"));
  assert.equal(validScoreKey("12:24"), true); assert.equal(validScoreKey("12:25"), false);
  assert.equal(validScoreKey("-1:0"), false); assert.equal(validScoreKey("1e2:0"), false);
});

test("equal object IDs in different native groups or channels do not share selections", () => {
  const memory = new ScoreSelectionMemory();
  memory.openWriter(scope("a"), () => true).remember(selected("1:1"));
  assert.equal(memory.openWriter(scope("a", { group: "bb" }), () => true).recall(), null);
  assert.equal(memory.openWriter(scope("a", { channel: "other" }), () => true).recall(), null);
  assert.deepEqual(memory.openWriter(scope("a"), () => true).recall(), selected("1:1"));
});

test("reset and A to B to A keep old mount callbacks inert", () => {
  const memory = new ScoreSelectionMemory(), old = memory.openWriter(scope("a"), () => true);
  old.remember(selected("1:1"));
  memory.openWriter(scope("b"), () => true).remember(selected("2:2"));
  const fresh = memory.openWriter(scope("a"), () => true);
  assert.equal(old.remember(selected("3:3")), false);
  assert.deepEqual(fresh.recall(), selected("1:1"));
  memory.clear();
  const next = memory.openWriter(scope("a"), () => true);
  assert.equal(fresh.remember(selected("4:4")), false); assert.equal(next.recall(), null);
  next.remember(selected("5:5")); old.revoke();
  assert.deepEqual(next.recall(), selected("5:5"), "old cleanup cannot revoke its replacement");
});

test("server retirement clears its data and writer without clearing another server", () => {
  const memory = new ScoreSelectionMemory();
  const other = scope("b", { server: 2, group: "bb" });
  memory.openWriter(other, () => true).remember(selected("2:2"));
  const a = memory.openWriter(scope("a"), () => true); a.remember(selected("1:1"));
  memory.retireServer(1);
  assert.equal(a.remember(selected("3:3")), false);
  assert.equal(memory.openWriter(scope("a"), () => true).recall(), null);
  assert.deepEqual(memory.openWriter(other, () => true).recall(), selected("2:2"));
});

test("selection arrays are copied, deduplicated and bounded to the complete grid", () => {
  const writer = new ScoreSelectionMemory().openWriter(scope("a"), () => true), picked = ["1:1", "1:1", "2:2"];
  writer.remember({ selected: "1:1", picked }); picked.push("3:3");
  assert.deepEqual(writer.recall(), { selected: "1:1", picked: ["1:1", "2:2"] });
  (writer.recall()!.picked as string[]).push("4:4"); assert.equal(writer.recall()!.picked.length, 2);
  const phrase = Array.from({ length: SCORE_MAX_STEPS * SCORE_ROWS }, (_, i) => `${Math.floor(i / SCORE_ROWS)}:${i % SCORE_ROWS}`);
  assert.equal(writer.remember({ selected: null, picked: phrase }), true);
  assert.equal(writer.remember({ selected: null, picked: [...phrase, "0:0"] }), false);
  assert.equal(writer.recall()!.picked.length, phrase.length);
});

test("reentrant currentness cannot publish through a replaced writer", () => {
  const memory = new ScoreSelectionMemory(); let reenter = false;
  const a = memory.openWriter(scope("a"), () => { if (reenter) memory.openWriter(scope("b"), () => true); return true; });
  a.remember(selected("1:1")); reenter = true;
  assert.equal(a.remember(selected("2:2")), false);
  assert.deepEqual(memory.openWriter(scope("a"), () => true).recall(), selected("1:1"));
  const closed = memory.openWriter(scope("c"), () => false);
  assert.equal(closed.remember(selected("3:3")), false); assert.equal(closed.recall(), null);
});

test("nonfinite or oversized constructor limits cannot remove the fixed ceiling", () => {
  for (const limit of [NaN, Infinity, 9999]) {
    const memory = new ScoreSelectionMemory(limit);
    for (let i = 0; i < 33; i++) memory.openWriter(scope(String(i)), () => true).remember(selected("1:1"));
    assert.equal(memory.openWriter(scope("0"), () => true).recall(), null);
    assert.deepEqual(memory.openWriter(scope("32"), () => true).recall(), selected("1:1"));
  }
  const zero = new ScoreSelectionMemory(-1).openWriter(scope("a"), () => true);
  zero.remember(selected("1:1")); assert.equal(zero.recall(), null);
});
