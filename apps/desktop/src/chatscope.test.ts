import assert from "node:assert/strict";
import test from "node:test";
import { chatScopeKey, reconcileActiveChannel, scopeHoldsConversation } from "./chatscope.ts";

const chans = (...ids: string[]) => ids.map((id) => ({ id }));

test("one conversation key, so windows, read marks and drafts cannot disagree", () => {
  assert.equal(chatScopeKey(7, "abc"), "7:abc");
  // Ids are opaque strings from the bridge (channel ids are u128, sent as hex): the key must not
  // assume anything about their shape.
  assert.equal(chatScopeKey(0, ""), "0:");
  assert.equal(chatScopeKey(12, "0f".repeat(16)), `12:${"0f".repeat(16)}`);
});

test("loaded rows belong to a conversation only when the stamped scope names it", () => {
  assert.equal(scopeHoldsConversation("7:abc", 7, "abc"), true);
  // The whole point: the coalescer's shared promise can resolve after a pass that loaded a
  // different server or a different channel, and either mismatch must read as "not mine".
  assert.equal(scopeHoldsConversation("7:abc", 8, "abc"), false);
  assert.equal(scopeHoldsConversation("7:abc", 7, "def"), false);
});

test("nothing loaded yet is never a match", () => {
  assert.equal(scopeHoldsConversation("", 7, "abc"), false);
  // A switch blanks the stamp, so the window between clearing and the first read landing must not
  // resolve as "these rows are yours" for any conversation, including a plausible-looking one.
  assert.equal(scopeHoldsConversation("", 0, ""), false);
});

test("an active channel still in the directory does not move", () => {
  assert.deepEqual(reconcileActiveChannel(chans("a", "b", "c"), "b"), { active: "b", changed: false });
  assert.deepEqual(reconcileActiveChannel(chans("a"), "a"), { active: "a", changed: false });
});

test("a channel that left the directory hands the reader to the first one", () => {
  // Nothing deletes a channel; entries disappear because the backend drops a catalog write whose
  // name does not hash to its id, or because the index has not synced and only `general` lists.
  assert.deepEqual(reconcileActiveChannel(chans("general"), "dev"), { active: "general", changed: true });
  assert.deepEqual(reconcileActiveChannel(chans("a", "b"), "gone"), { active: "a", changed: true });
});

test("no channel selected yet adopts the first and reports the move", () => {
  // `changed` is what makes the caller load the channel-scoped state, so adopting a channel from
  // nothing has to report true or the pane would name a channel it never read.
  assert.deepEqual(reconcileActiveChannel(chans("a", "b"), ""), { active: "a", changed: true });
});

test("an empty list is a failed read, not an instruction to move", () => {
  // Being walked off a channel that is probably still there is worse than keeping a stale name:
  // the read is retried, and the caller's early return keeps the pane where the user left it.
  assert.deepEqual(reconcileActiveChannel([], "dev"), { active: "dev", changed: false });
  assert.deepEqual(reconcileActiveChannel([], ""), { active: "", changed: false });
});
