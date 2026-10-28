import assert from "node:assert/strict";
import test from "node:test";
import { scoreSelectionMemory } from "./score-selection.ts";
import { disposeStudio, resetStudio, retireStudioServer } from "./studio-state.svelte.ts";

const scope = { server: 1, group: "aa", channel: "channel", object: "a".repeat(64) };
const selection = { selected: "1:1", picked: ["1:1"] };

test("real Studio reset and disposal clear global selection before late teardown writes", () => {
  for (const reset of [resetStudio, disposeStudio]) {
    const old = scoreSelectionMemory.openWriter(scope, () => true);
    old.remember(selection); reset();
    const fresh = scoreSelectionMemory.openWriter(scope, () => true);
    assert.equal(old.remember(selection), false);
    assert.equal(fresh.recall(), null, "a new mount after Lock cannot recall old private state");
  }
  disposeStudio();
});

test("server retirement clears selection even before a Studio session exists", () => {
  disposeStudio();
  const old = scoreSelectionMemory.openWriter(scope, () => true); old.remember(selection);
  retireStudioServer(scope.server, true);
  assert.equal(old.remember(selection), false);
  assert.equal(scoreSelectionMemory.openWriter(scope, () => true).recall(), null);
  disposeStudio();
});
