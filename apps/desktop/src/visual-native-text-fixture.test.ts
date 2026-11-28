import assert from "node:assert/strict";
import test from "node:test";
import { createVisualNativeTextFixture } from "./visual-native-text-fixture.ts";
import { decodeRestoredChatSession } from "./native-text-session.ts";
import { decodeNativeOriginalPage, readNativeHeldPosts, readNativeReceivedArchive } from "./native-text-history.ts";

test("Native Text visual data uses the real decoders and never falls back to Legacy server reads", async () => {
  const fixture = createVisualNativeTextFixture();
  const call = (name: string, args: Record<string, unknown> = {}) => { const result = fixture(name, args); assert.ok(result.handled); return result.value; };
  const session = decodeRestoredChatSession(call("resume_session"));
  assert.equal(session?.backend, "text-v2"); if (session?.backend !== "text-v2") return;
  const row = session.servers[0];
  const args = { server: row.server, channel: row.channel, expectedContext: row.context };
  const page = decodeNativeOriginalPage(call("get_message_page", { ...args, start: "0", endExclusive: null, limit: 8 }), "0", null, "1");
  assert.equal(page.items.length, 2);
  const archive = await readNativeReceivedArchive(
    async (command, args) => call(command, args),
    { server: row.server, channel: row.channel, context: row.context },
    () => true,
    null,
  );
  assert.equal(archive?.items[0]?.text, "A message recovered after reconnecting.");
  assert.deepEqual(archive?.covered, archive?.ids);
  assert.throws(() => fixture("get_members", { server: 1 }), /Legacy server command/);
  assert.throws(() => fixture("get_message_page", { ...args, expectedContext: "wrong" }), /Unknown Native/);
  const scope = { server: row.server, channel: row.channel, context: row.context };
  const readHeld = () => readNativeHeldPosts(async (command, args) => call(command, args), scope, null, () => true);
  const held = (await readHeld())?.items?.[0];
  assert.ok(held && held.decision.status === "undecided");
  const decision = { ...args, caseId: held.caseId, action: held.action, originalDigest: held.originalDigest,
    removalHeader: held.removalHeader, removalEpoch: held.removalEpoch, previousDecision: null,
    choice: "reject", intent: "9a".repeat(16) };
  const savedDecision = call("decide_native_chat_held_post", decision);
  assert.deepEqual(call("decide_native_chat_held_post", decision), savedDecision);
  assert.throws(() => call("decide_native_chat_held_post", { ...decision, choice: "approve" }), /saved decision request changed/);
  const reviewed = (await readHeld())?.items?.[0]?.decision;
  assert.equal(reviewed?.status === "selected" && reviewed.choice, "reject");
  const token = "d1".repeat(32);
  const saved = call("send_message", { ...args, retryToken: token, text: "Saved by the visual check" });
  assert.deepEqual(call("send_message", { ...args, retryToken: token, text: "Saved by the visual check" }), saved);
  assert.throws(() => call("send_message", { ...args, retryToken: token, text: "Changed" }), /retry conflict/);
  const after = decodeNativeOriginalPage(call("get_message_page", { ...args, start: "0", endExclusive: null, limit: 8 }), "0", null, "1");
  assert.equal(after.items.length, 3); assert.equal(after.items[2].action, token);
  call("save_ui_state", { json: '{"version":4}' }); assert.equal(call("get_ui_state"), '{"version":4}');
  const empty = createVisualNativeTextFixture(true)("resume_session", {});
  assert.ok(empty.handled); assert.deepEqual(decodeRestoredChatSession(empty.value), { backend: "text-v2", servers: [] });
});
