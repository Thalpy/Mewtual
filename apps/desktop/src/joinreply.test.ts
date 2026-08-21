import assert from "node:assert/strict";
import test from "node:test";

import {
  JOIN_REPLY_REPLACEMENT_ERROR,
  assistedJoinAction,
  joinReplyCandidateLabel,
  joinReplyIsExpired,
  joinReplyNeedsReplacement,
  withOrderedSwitchboardStatus,
} from "./joinreply.ts";

test("only the exact backend key-conflict enables replacement", () => {
  assert.equal(joinReplyNeedsReplacement(JOIN_REPLY_REPLACEMENT_ERROR), true);
  assert.equal(joinReplyNeedsReplacement("confirm replacement after malformed code"), false);
  assert.equal(joinReplyNeedsReplacement(new Error(JOIN_REPLY_REPLACEMENT_ERROR)), false);
});

test("assisted invites require a separate preview click before helper consent can apply", () => {
  assert.equal(assistedJoinAction(false, 2, false), "preview");
  assert.equal(assistedJoinAction(true, 2, false), "direct");
  assert.equal(assistedJoinAction(true, 2, true), "switchboard");
  assert.equal(assistedJoinAction(false, 0, false), "direct");
});

test("older or switched-away switchboard status cannot overwrite current state", () => {
  assert.equal(withOrderedSwitchboardStatus("new", 4, 4, "old", 1, 2), "new");
  assert.equal(withOrderedSwitchboardStatus("new", 5, 4, "other", 2, 2), "new");
  assert.equal(withOrderedSwitchboardStatus("old", 4, 4, "new", 2, 2), "new");
});

test("reply codes become unusable exactly at their native deadline", () => {
  assert.equal(joinReplyIsExpired(61_000, 60_999), false);
  assert.equal(joinReplyIsExpired(61_000, 61_000), true);
  assert.equal(joinReplyIsExpired(Number.NaN, 1), true);
});

test("candidate copy is bounded and grammatical", () => {
  assert.equal(joinReplyCandidateLabel(1), "1 direct listener route");
  assert.equal(joinReplyCandidateLabel(4), "4 direct listener routes");
  assert.equal(joinReplyCandidateLabel(Number.NaN), "0 direct listener routes");
});
