// Unit tests for the wiki sidebar's page tree (`/`-separated names -> nested folders).
//
// Run with `npm test`. What matters here: a page and a folder may share a path (the
// "Guides" page beside "Guides/Setup"), collapse hides descendants but never the folder
// row itself, and the flat list orders one alphabet per level.

import { test } from "node:test";
import assert from "node:assert/strict";

import { ancestorsOf, buildWikiTree, folderPaths, visibleRows } from "./wikitree.ts";

test("flat names build a flat tree", () => {
  const tree = buildWikiTree(["Rules", "Home"]);
  assert.deepEqual(
    tree.map((n) => ({ label: n.label, page: n.page, kids: n.children.length })),
    [
      { label: "Home", page: "Home", kids: 0 },
      { label: "Rules", page: "Rules", kids: 0 },
    ],
  );
});

test("slash names nest, and intermediate folders need no page", () => {
  const tree = buildWikiTree(["Guides/Setup/Linux", "Guides/Setup/Windows", "Home"]);
  const guides = tree.find((n) => n.label === "Guides");
  assert.ok(guides);
  assert.equal(guides.page, null, "no page named exactly 'Guides'");
  assert.equal(guides.children.length, 1);
  const setup = guides.children[0];
  assert.equal(setup.path, "Guides/Setup");
  assert.deepEqual(
    setup.children.map((n) => n.page),
    ["Guides/Setup/Linux", "Guides/Setup/Windows"],
  );
});

test("a page and a folder can share a path", () => {
  const tree = buildWikiTree(["Guides", "Guides/Setup"]);
  assert.equal(tree.length, 1);
  assert.equal(tree[0].page, "Guides", "the node is a page...");
  assert.equal(tree[0].children[0].page, "Guides/Setup", "...and a folder");
});

test("empty segments collapse instead of minting blank folders", () => {
  const tree = buildWikiTree(["a//b", "a/b"]);
  assert.equal(tree.length, 1);
  assert.equal(tree[0].children.length, 1);
  // Both names land on the same node path; the later-sorted name keeps the page slot.
  assert.equal(tree[0].children[0].path, "a/b");
});

test("levels sort case-insensitively with folders and pages intermixed", () => {
  const tree = buildWikiTree(["zebra", "Apple/one", "apple pie", "Banana"]);
  assert.deepEqual(
    tree.map((n) => n.label),
    ["Apple", "apple pie", "Banana", "zebra"],
  );
});

test("folderPaths lists every expandable node", () => {
  const tree = buildWikiTree(["a/b/c", "a/d", "e"]);
  assert.deepEqual(folderPaths(tree), ["a", "a/b"]);
});

test("ancestorsOf gives the folders that must expand to reveal a page", () => {
  assert.deepEqual(ancestorsOf("a/b/c"), ["a", "a/b"]);
  assert.deepEqual(ancestorsOf("solo"), []);
});

test("visibleRows hides a collapsed folder's descendants but keeps its row", () => {
  const tree = buildWikiTree(["a/b/c", "a/d", "e"]);
  const all = visibleRows(tree, new Set());
  assert.deepEqual(
    all.map((r) => `${r.depth}:${r.node.path}`),
    ["0:a", "1:a/b", "2:a/b/c", "1:a/d", "0:e"],
  );
  const collapsed = visibleRows(tree, new Set(["a/b"]));
  assert.deepEqual(
    collapsed.map((r) => r.node.path),
    ["a", "a/b", "a/d", "e"],
  );
  const rootCollapsed = visibleRows(tree, new Set(["a"]));
  assert.deepEqual(
    rootCollapsed.map((r) => r.node.path),
    ["a", "e"],
  );
});
