import assert from "node:assert/strict";
import test from "node:test";
import { addPendingSend, sanitizePendingSends, MAX_PENDING_SENDS, type PendingSend } from "./pending-sends.ts";
import { sanitizeUiContinuity, planLegacyReadMarkMigration } from "./ui-continuity.ts";

const intent: PendingSend = { token: "1".repeat(32), server: 1, channel: "4",
  expectedContext: "2".repeat(64), text: "still mine", replyTo: "" };

test("the complete retry identity survives continuity hydration and legacy read-mark migration", () => {
  const state = sanitizeUiContinuity({ pendingSends: { [intent.token]: intent } });
  assert.deepEqual(state.pendingSends, { [intent.token]: intent });
  const migrated = planLegacyReadMarkMigration(state, '{"room":1}');
  assert.deepEqual(migrated.state.pendingSends, state.pendingSends);
});

test("malformed scope and context cannot become resumable message intents", () => {
  for (const patch of [{ channel: "01" }, { channel: (1n << 128n).toString() },
    { expectedContext: "" }, { server: -1 }, { token: "different" }, { text: "x".repeat(65537) }]) {
    assert.deepEqual(sanitizePendingSends({ [intent.token]: { ...intent, ...patch } }), {});
  }
});

test("capacity pressure and token conflicts preserve every existing pending message", () => {
  let pending = {};
  for (let i = 0; i < MAX_PENDING_SENDS; i++) {
    const token = i.toString(16).padStart(32, "0");
    pending = addPendingSend(pending, { ...intent, token });
  }
  assert.throws(() => addPendingSend(pending, intent), /need attention/);
  assert.equal(Object.keys(pending).length, MAX_PENDING_SENDS);
  assert.throws(() => addPendingSend({ [intent.token]: intent }, { ...intent, text: "changed" }), /conflicting/);
});
