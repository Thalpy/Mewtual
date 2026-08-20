import assert from "node:assert/strict";
import test from "node:test";
import { planLegacyReadMarkMigration, sanitizeUiContinuity } from "./ui-continuity.ts";

test("continuity sanitization keeps only bounded drafts and safe read positions", () => {
  const state = sanitizeUiContinuity({
    drafts: { good: "draft", huge: "x".repeat(32_769), ["k".repeat(257)]: "hidden" },
    readMarks: { good: 42, negative: -1, fraction: 1.5, text: "9" },
  });
  assert.deepEqual(state, { version: 1, drafts: { good: "draft" }, readMarks: { good: 42 } });
  assert.deepEqual(sanitizeUiContinuity({ drafts: [], readMarks: null }), {
    version: 1, drafts: {}, readMarks: {},
  });
});

test("a valid legacy map is saved before its plaintext key is removed", () => {
  const current = sanitizeUiContinuity({ drafts: { room: "hello" }, readMarks: {} });
  const plan = planLegacyReadMarkMigration(current, '{"server:channel":123}');
  assert.equal(plan.saveBeforeRemoval, true);
  assert.equal(plan.removeLegacy, true);
  assert.deepEqual(plan.state.readMarks, { "server:channel": 123 });
  assert.deepEqual(plan.state.drafts, { room: "hello" });
});

test("sealed state wins over stale legacy data and malformed legacy data is preserved", () => {
  const current = sanitizeUiContinuity({ drafts: {}, readMarks: { sealed: 8 } });
  const stale = planLegacyReadMarkMigration(current, '{"legacy":99}');
  assert.equal(stale.saveBeforeRemoval, false);
  assert.equal(stale.removeLegacy, true);
  assert.deepEqual(stale.state.readMarks, { sealed: 8 });

  const malformed = planLegacyReadMarkMigration(
    sanitizeUiContinuity({ drafts: {}, readMarks: {} }),
    "not json",
  );
  assert.equal(malformed.saveBeforeRemoval, false);
  assert.equal(malformed.removeLegacy, false);
});
