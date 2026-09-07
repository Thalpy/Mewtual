import assert from "node:assert/strict";
import test from "node:test";
import {
  confirmQuestion,
  confirmVerb,
  parseTally,
  placeMenu,
  tallyReaction,
  topReactions,
  typeaheadIndex,
} from "./context-menu.ts";

// ---- placement ----------------------------------------------------------------------------

test("a menu with room opens down and right with its corner on the pointer", () => {
  assert.deepEqual(placeMenu(100, 100, 200, 300, 1200, 800), { left: 100, top: 100, flipX: false, flipY: false });
});

test("near the right edge it mirrors to the left of the pointer instead of sliding over it", () => {
  const p = placeMenu(1100, 100, 200, 300, 1200, 800);
  assert.equal(p.flipX, true);
  assert.equal(p.left, 900, "the right edge of the menu sits on the pointer");
  assert.equal(p.flipY, false);
});

test("near the bottom edge it grows upward", () => {
  const p = placeMenu(100, 700, 200, 300, 1200, 800);
  assert.equal(p.flipY, true);
  assert.equal(p.top, 400);
});

test("a bottom-right pointer flips on both axes", () => {
  const p = placeMenu(1150, 780, 200, 300, 1200, 800);
  assert.deepEqual([p.flipX, p.flipY], [true, true]);
  assert.deepEqual([p.left, p.top], [950, 480]);
});

test("when neither side fits it clamps rather than leaving the viewport", () => {
  // A menu taller than the space above AND below: no flip (flipping would go negative), so clamp.
  const p = placeMenu(10, 500, 200, 700, 1200, 800);
  assert.equal(p.flipY, false);
  assert.equal(p.top, 800 - 700 - 4);
  assert.ok(p.top >= 4);
});

test("a viewport smaller than the menu still yields a non-negative origin", () => {
  const p = placeMenu(5, 5, 400, 400, 300, 300);
  assert.deepEqual([p.left, p.top], [4, 4]);
});

// ---- recent reactions ---------------------------------------------------------------------

test("a fresh install shows the defaults verbatim", () => {
  assert.deepEqual(topReactions({}, ["a", "b", "c", "d"], 3), ["a", "b", "c"]);
});

test("picks rise above the defaults, and a picked default is not shown twice", () => {
  const t = tallyReaction(tallyReaction(tallyReaction({}, "🐱"), "🐱"), "b");
  assert.deepEqual(topReactions(t, ["a", "b", "c", "d"], 4), ["🐱", "b", "a", "c"]);
});

test("ties keep the defaults' own order", () => {
  const t = { d: 1, a: 1, zz: 1 };
  assert.deepEqual(topReactions(t, ["a", "b", "c", "d"], 5), ["a", "d", "zz", "b", "c"]);
});

test("tallyReaction never mutates its input", () => {
  const t = { x: 1 };
  const u = tallyReaction(t, "x");
  assert.equal(t.x, 1);
  assert.equal(u.x, 2);
});

test("parseTally survives garbage and drops non-counts", () => {
  assert.deepEqual(parseTally(null), {});
  assert.deepEqual(parseTally("nope"), {});
  assert.deepEqual(parseTally("[1,2]"), {});
  assert.deepEqual(parseTally('{"a":2,"b":"x","c":-1,"d":1.7,"e":0}'), { a: 2, d: 1 });
  assert.deepEqual(parseTally(JSON.stringify({ [`${"k".repeat(65)}`]: 3 })), {}, "an oversized key is not a reaction");
});

// ---- typeahead ------------------------------------------------------------------------------

test("typeahead finds the first label with the prefix, case-insensitively", () => {
  const labels = ["Reply", "Quote in reply", "Copy text", "Delete"];
  assert.equal(typeaheadIndex(labels, "de", -1), 3);
  assert.equal(typeaheadIndex(labels, "Q", -1), 1);
});

test("repeating a letter cycles through its matches and wraps", () => {
  const labels = ["Copy text", "Copy link", "Delete"];
  assert.equal(typeaheadIndex(labels, "c", -1), 0);
  assert.equal(typeaheadIndex(labels, "c", 0), 1);
  assert.equal(typeaheadIndex(labels, "c", 1), 0, "wraps back to the first match");
});

test("an empty buffer matches nothing, and so does a prefix nobody has", () => {
  assert.equal(typeaheadIndex(["Reply"], "", -1), -1);
  assert.equal(typeaheadIndex(["Reply"], "   ", -1), -1);
  assert.equal(typeaheadIndex(["Reply"], "x", -1), -1);
  assert.equal(typeaheadIndex([], "r", -1), -1);
});

// ---- confirm prompt -------------------------------------------------------------------------

test("the confirm verb is the question's first word, and the question ends in one mark", () => {
  assert.equal(confirmVerb("Delete this message?"), "Delete");
  assert.equal(confirmVerb("Remove mika"), "Remove");
  assert.equal(confirmVerb("   "), "Confirm");
  assert.equal(confirmQuestion("Leave art room"), "Leave art room?");
  assert.equal(confirmQuestion("Delete this message?"), "Delete this message?");
  assert.equal(confirmQuestion("Sure?!"), "Sure?");
});
