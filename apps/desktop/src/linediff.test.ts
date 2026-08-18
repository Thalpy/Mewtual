// Unit tests for the line diff behind wiki history and edit review.
//
// Run with `npm test`. The invariants a reviewer relies on: every old line appears as
// `same` or `del`, every new line as `same` or `add`, in order; so replaying the script
// reconstructs either side exactly. The tests assert reconstruction rather than one
// blessed alignment, plus a few pinned shapes for the common cases.

import { test } from "node:test";
import assert from "node:assert/strict";

import { diffLines, diffStats, type DiffLine } from "./linediff.ts";

function oldSide(lines: DiffLine[]): string[] {
  return lines.filter((l) => l.kind !== "add").map((l) => l.text);
}
function newSide(lines: DiffLine[]): string[] {
  return lines.filter((l) => l.kind !== "del").map((l) => l.text);
}
function assertReconstructs(a: string, b: string) {
  const d = diffLines(a, b);
  assert.deepEqual(oldSide(d), a === "" ? [] : a.split("\n"));
  assert.deepEqual(newSide(d), b === "" ? [] : b.split("\n"));
}

test("identical bodies are all `same`", () => {
  const d = diffLines("a\nb", "a\nb");
  assert.deepEqual(d, [
    { kind: "same", text: "a" },
    { kind: "same", text: "b" },
  ]);
  assert.deepEqual(diffLines("", ""), []);
});

test("a pure insertion is `add` lines in place", () => {
  const d = diffLines("a\nc", "a\nb\nc");
  assert.deepEqual(d, [
    { kind: "same", text: "a" },
    { kind: "add", text: "b" },
    { kind: "same", text: "c" },
  ]);
});

test("a pure deletion is `del` lines in place", () => {
  const d = diffLines("a\nb\nc", "a\nc");
  assert.deepEqual(d, [
    { kind: "same", text: "a" },
    { kind: "del", text: "b" },
    { kind: "same", text: "c" },
  ]);
});

test("a changed line reads as del + add", () => {
  const d = diffLines("hello\nworld", "hello\nthere");
  assert.deepEqual(oldSide(d), ["hello", "world"]);
  assert.deepEqual(newSide(d), ["hello", "there"]);
  assert.deepEqual(diffStats(d), { added: 1, removed: 1 });
});

test("empty old side: everything is added (a page creation)", () => {
  const d = diffLines("", "a\nb");
  assert.deepEqual(d, [
    { kind: "add", text: "a" },
    { kind: "add", text: "b" },
  ]);
});

test("empty new side: everything is removed (a page delete)", () => {
  const d = diffLines("a\nb", "");
  assert.deepEqual(d, [
    { kind: "del", text: "a" },
    { kind: "del", text: "b" },
  ]);
});

test("both sides reconstruct exactly on messier edits", () => {
  assertReconstructs("a\nb\nc\nd\ne", "b\nx\nc\ny\ne\nz");
  assertReconstructs("one\ntwo\nthree", "three\ntwo\none");
  assertReconstructs("x", "y");
  assertReconstructs("a\n\n\nb", "a\nb");
});

test("duplicate lines keep their order", () => {
  assertReconstructs("a\na\na", "a\na");
  const d = diffLines("a\na\na", "a\na");
  assert.deepEqual(diffStats(d), { added: 0, removed: 1 });
});
