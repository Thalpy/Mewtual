import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";

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

test("the DM surface exposes fresh reply codes and both repair paths", () => {
  const app = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  assert.match(app, /listen<JoinReplyReady>\("join-reply-ready"/);
  assert.match(app, /listen<JoinReplyReady>\("join-reply-ready", \(e\) => \{[\s\S]*if \(locked \|\| !joinAttemptPending\) return;[\s\S]*joinReplyReady = e\.payload/);
  assert.match(app, /function lockScreen\([^)]*\)[\s\S]*joinReplyReady = null;[\s\S]*memberRecoveryReady = null;[\s\S]*invoke\("lock_session"/);
  assert.match(app, /async function mintMemberRecovery\(\)[\s\S]*operationGeneration = viewGeneration;[\s\S]*await invoke<MemberRecoveryReady>[\s\S]*sessionContinuationCurrent\(operationGeneration, viewGeneration, locked\)[\s\S]*memberRecoveryReady = ready/);
  assert.match(app, /async function applyMemberRecovery\(\)[\s\S]*operationGeneration = viewGeneration;[\s\S]*await invoke<MemberRecoveryApplied>[\s\S]*sessionContinuationCurrent\(operationGeneration, viewGeneration, locked\)[\s\S]*memberRecoveryInput = ""/);
  assert.match(app, /memberRecoveryExpired = \$derived\([\s\S]*joinReplyIsExpired\(memberRecoveryReady\.expires_at_ms, joinReplyNow\)/);
  assert.match(app, /memberRecoveryServer === activeServerId && !memberRecoveryExpired[\s\S]*Copy code[\s\S]*That recovery code expired/);
  assert.match(app, /async function applyJoinReply\(replace = false\)[\s\S]*operationGeneration = viewGeneration;[\s\S]*operation = \+\+joinReplyOperation;[\s\S]*await invoke<[\s\S]*\("apply_join_reply"[\s\S]*sessionContinuationCurrent\(operationGeneration, viewGeneration, locked\)[\s\S]*joinReplyInput = ""/);
  assert.match(app, /#snippet outboundJoinReplyPanel\(\)[\s\S]*joinReplyReady\.code/);
  assert.match(app, /Direct messages[\s\S]*@render outboundJoinReplyPanel\(\)/);
  assert.match(app, /class="dm-connection-repair"[\s\S]*applyJoinReply/);
  assert.match(app, /class="dm-connection-repair"[\s\S]*@render memberRecoveryPanel\(\)/);
  assert.match(app, /function clearServerView\(\)[\s\S]*memberRecoveryOperation \+= 1;[\s\S]*memberRecoveryInput = "";[\s\S]*memberRecoveryBusy = false/);
  assert.match(app, /function clearServerView\(\)[\s\S]*joinReplyOperation \+= 1;[\s\S]*joinReplyInput = "";[\s\S]*joinReplyApplying = false;[\s\S]*joinReplyNeedsReplace = false/);
  assert.match(app, /async function addFriend\(\)[\s\S]*await invokeDebugged<Found>\("join_server"[\s\S]*joinReplyReady = null;[\s\S]*addServer/);
  assert.match(app, /async function acceptDmRequest\(req: DmRequest\)[\s\S]*await invokeDebugged<Found>\("join_server"[\s\S]*joinReplyReady = null;[\s\S]*addServer/);
});
