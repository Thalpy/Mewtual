import assert from "node:assert/strict";
import test from "node:test";
import { ScoreSelectionMemory, validScoreKey } from "./score-selection.ts";

test("a document's selection is kept and recalled; an empty selection forgets it", () => {
  const memory = new ScoreSelectionMemory();
  memory.remember("a", { selected: "4:4", picked: ["4:4", "6:6"] });
  memory.remember("b", { selected: "1:1", picked: [] });
  assert.deepEqual(memory.recall("a"), { selected: "4:4", picked: ["4:4", "6:6"] });
  assert.deepEqual(memory.recall("b"), { selected: "1:1", picked: [] });
  assert.equal(memory.recall("c"), null);
  memory.remember("a", { selected: null, picked: [] });
  assert.equal(memory.recall("a"), null, "nothing selected is nothing to remember");
  memory.forget("b"); assert.equal(memory.recall("b"), null);
});

test("keys the composer could never have written are dropped, and the store is bounded by recency", () => {
  const memory = new ScoreSelectionMemory(2);
  memory.remember("a", { selected: "9:99", picked: ["x", "256:0", "255:24", "1:2:3"] });
  assert.deepEqual(memory.recall("a"), { selected: null, picked: ["255:24"] });
  memory.remember("b", { selected: "0:0", picked: [] });
  memory.remember("a", { selected: "1:1", picked: [] });
  memory.remember("c", { selected: "2:2", picked: [] });
  assert.equal(memory.recall("b"), null, "the least recently remembered document goes first");
  assert.deepEqual(memory.recall("a"), { selected: "1:1", picked: [] });
  assert.deepEqual(memory.recall("c"), { selected: "2:2", picked: [] });
  assert.equal(validScoreKey("12:24"), true); assert.equal(validScoreKey("12:25"), false); assert.equal(validScoreKey("-1:0"), false); assert.equal(validScoreKey("1e2:0"), false);
});
