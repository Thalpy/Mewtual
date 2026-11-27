import test from "node:test";
import assert from "node:assert/strict";
import { decodeNativeMembershipNotice, decodeRestoredChatSession } from "./native-text-session.ts";
const row = { backend: "text-v2", server: 1, context: "01".repeat(32), channel: "340282366920938463463374607431768211455", name: "Group", display_name: "Catte", is_dm: false };
test("a native membership notice must name the original context and explicit outcome", () => {
  const notice = { server: row.server, context: row.context, removed: true };
  assert.deepEqual(decodeNativeMembershipNotice(notice), notice);
  assert.deepEqual(decodeNativeMembershipNotice({ ...notice, removed: false }), { ...notice, removed: false });
  for (const invalid of [null, { server: row.server, removed: true },
    { ...notice, server: "1" }, { ...notice, server: 0 }, { ...notice, removed: "true" },
    { ...notice, context: "00".repeat(32) }, { ...notice, context: "ab".repeat(16) },
    { ...notice, channel: row.channel }]) {
    assert.equal(decodeNativeMembershipNotice(invalid), null);
  }
});
test("session backend survives zero rows and stays distinct from locked resume", () => {
  assert.deepEqual(decodeRestoredChatSession([]), { backend: "legacy", servers: [] });
  assert.deepEqual(decodeRestoredChatSession({ backend: "text-v2", servers: [] }), { backend: "text-v2", servers: [] });
  assert.equal(decodeRestoredChatSession(null), null);
  const legacy = [{ server: 7, name: "Existing group", invite: "" }];
  assert.equal(decodeRestoredChatSession(legacy)?.servers, legacy, "existing Legacy array contract is preserved");
});
test("native session retains two exact contexts without coercing channel integers", () => {
  const native = { backend: "text-v2", servers: [row, { ...row, server: 2, context: "02".repeat(32) }] };
  const parsed = decodeRestoredChatSession(native)!;
  assert.deepEqual(parsed, native);
  assert.ok(Object.isFrozen(parsed.servers[0]));
  assert.notEqual(parsed.servers, native.servers);
});
test("removed membership opens a distinct recovery session with zero live rows", () => {
  const retired = { backend: "text-v2", servers: [], recovery: "removed" };
  assert.deepEqual(decodeRestoredChatSession(retired), retired);
  for (const patch of [{ servers: [row] }, { recovery: true }, { backend: "legacy" }, { unexpected: 1 }]) {
    assert.throws(() => decodeRestoredChatSession({ ...retired, ...patch }));
  }
});
test("session decoder rejects mixed modes, future fields and guessed defaults", () => {
  for (const value of [[row], { servers: [] }, { backend: "legacy", servers: [] }, { backend: "future", servers: [] },
    { backend: "text-v2", servers: [], ignored: true }, { backend: "text-v2", servers: [row, row] },
    { backend: "text-v2", servers: [row, { ...row, server: 2 }, { ...row, server: 3 }] }]) assert.throws(() => decodeRestoredChatSession(value));
  for (const patch of [{ backend: "legacy" }, { server: Number.MAX_SAFE_INTEGER + 1 }, { server: -1 }, { context: "00".repeat(32) },
    { context: "01".repeat(16) }, { channel: "01" }, { channel: "340282366920938463463374607431768211456" },
    { name: "" }, { name: "💜".repeat(65) }, { display_name: "\ud800" }, { is_dm: "false" }, { extra: true }]) {
    assert.throws(() => decodeRestoredChatSession({ backend: "text-v2", servers: [{ ...row, ...patch }] }));
  }
});
