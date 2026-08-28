import assert from "node:assert/strict";
import test from "node:test";
import {
  WIKI_REVIEW_UNKNOWN,
  mayEditWikiStructure,
  mayPublishLivery,
  moderationSurfaceOpen,
  sessionContinuationCurrent,
  scopeCurrent,
} from "./viewscope.ts";

test("a read still addressing the group on screen may write", () => {
  assert.equal(scopeCurrent({ generation: 3, server: 7 }, { generation: 3, server: 7 }), true);
});

test("a read from a move we have left may not write", () => {
  // The generation half. Rapid A -> B -> A returns to the same server id at a later generation, so
  // the id alone would let A's original in-flight snapshot overwrite A's fresh one.
  assert.equal(scopeCurrent({ generation: 2, server: 7 }, { generation: 4, server: 7 }), false);
});

test("a read for another group may not write at the same generation", () => {
  // The server half. Event-driven refreshes bump nothing, so this is the only thing standing
  // between a background server's answer and the pane in front of the user.
  assert.equal(scopeCurrent({ generation: 5, server: 7 }, { generation: 5, server: 8 }), false);
});

test("no active group accepts nothing", () => {
  assert.equal(scopeCurrent({ generation: 5, server: 7 }, { generation: 5, server: null }), false);
  // And a read issued with no group cannot claim one that has since opened.
  assert.equal(scopeCurrent({ generation: 5, server: null }, { generation: 5, server: 7 }), false);
});

test("a deferred create or join result cannot repopulate an explicitly locked UI", () => {
  assert.equal(sessionContinuationCurrent(4, 4, false), true);
  assert.equal(sessionContinuationCurrent(4, 5, false), false);
  assert.equal(sessionContinuationCurrent(4, 4, true), false);
});

test("an unread wiki policy denies the structural controls", () => {
  // The regression this exists to prevent: zero is a real policy, so an unread policy that cleared
  // to zero handed every member rename, delete and eager page creation on a review-gated server.
  assert.equal(mayEditWikiStructure(WIKI_REVIEW_UNKNOWN, false), false);
  assert.notEqual(WIKI_REVIEW_UNKNOWN, 0);
});

test("a read wiki policy is obeyed, and moderators are never gated", () => {
  assert.equal(mayEditWikiStructure(0, false), true); // publishes directly: members may
  assert.equal(mayEditWikiStructure(7, false), false); // review gated: members may not
  assert.equal(mayEditWikiStructure(7, true), true);
  assert.equal(mayEditWikiStructure(WIKI_REVIEW_UNKNOWN, true), true);
});

test("the moderation surface counts as open only when it is on screen", () => {
  assert.equal(moderationSurfaceOpen("moderation", false, false), true);
  assert.equal(moderationSurfaceOpen("chat", false, false), false);
});

test("an overlay hides the moderation surface without deselecting its tab", () => {
  // Both overlays replace the sidebar and main pane wholesale. Treating the tab as open behind them
  // is what made every incoming message re-sweep every channel of the server underneath.
  assert.equal(moderationSurfaceOpen("moderation", true, false), false);
  assert.equal(moderationSurfaceOpen("moderation", false, true), false);
});

test("a livery draft seeded from this server's own read livery may publish", () => {
  assert.equal(mayPublishLivery(true, 7, 7), true);
});

test("an unseeded or foreign livery draft may not publish", () => {
  // Each false here is one click away from erasing a server's branding for every member.
  assert.equal(mayPublishLivery(false, 7, 7), false); // livery never read
  assert.equal(mayPublishLivery(true, null, 7), false); // draft never seeded
  assert.equal(mayPublishLivery(true, 8, 7), false); // seeded from the server we left
  assert.equal(mayPublishLivery(true, null, null), false); // no active group at all
});
