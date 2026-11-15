import assert from "node:assert/strict";
import test from "node:test";
import { createVisualNativeTextFixture } from "./visual-native-text-fixture.ts";
import { decodeRestoredChatSession } from "./native-text-session.ts";
import { decodeNativeOriginalPage } from "./native-text-history.ts";

test("Native Text visual data uses the real decoders and never falls back to Legacy server reads", () => {
  const fixture = createVisualNativeTextFixture();
  const call = (name: string, args: Record<string, unknown> = {}) => { const result = fixture(name, args); assert.ok(result.handled); return result.value; };
  const session = decodeRestoredChatSession(call("resume_session"));
  assert.equal(session?.backend, "text-v2"); if (session?.backend !== "text-v2") return;
  const row = session.servers[0];
  const args = { server: row.server, channel: row.channel, expectedContext: row.context };
  const page = decodeNativeOriginalPage(call("get_message_page", { ...args, start: "0", endExclusive: null, limit: 8 }), "0", null, "1");
  assert.equal(page.items.length, 2);
  assert.throws(() => fixture("get_members", { server: 1 }), /Legacy server command/);
  assert.throws(() => fixture("get_message_page", { ...args, expectedContext: "wrong" }), /Unknown Native/);
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
