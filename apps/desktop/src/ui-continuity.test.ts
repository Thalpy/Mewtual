import assert from "node:assert/strict";
import test from "node:test";
import { planLegacyReadMarkMigration, sanitizeUiContinuity } from "./ui-continuity.ts";

test("continuity sanitization keeps only bounded drafts and safe read positions", () => {
  const state = sanitizeUiContinuity({
    drafts: { good: "draft", huge: "x".repeat(32_769), ["k".repeat(257)]: "hidden" },
    readMarks: {
      good: { ts: 42, id: "m1" },
      negative: { ts: -1, id: "m2" },
      fraction: { ts: 1.5, id: "m3" },
      text: { ts: "9" },
      unbounded: { ts: 42, id: "i".repeat(129) },
      notAMark: "42",
    },
  });
  assert.deepEqual(state, {
    version: 1, drafts: { good: "draft" }, readMarks: { good: { ts: 42, id: "m1" } },
  });
  assert.deepEqual(sanitizeUiContinuity({ drafts: [], readMarks: null }), {
    version: 1, drafts: {}, readMarks: {},
  });
});

test("a read mark from a build that only stored a timestamp upgrades in place", () => {
  // Discarding these would un-clear every badge people had already read. The empty id is not a
  // gap: it is exactly the state the unread scan reads as "fall back to the timestamp", and the
  // next read of that channel fills it in.
  const state = sanitizeUiContinuity({
    drafts: {},
    readMarks: { old: 42, negative: -1, fraction: 1.5, text: "9" },
  });
  assert.deepEqual(state.readMarks, { old: { ts: 42, id: "" } });
});

test("a valid legacy map is saved before its plaintext key is removed", () => {
  const current = sanitizeUiContinuity({ drafts: { room: "hello" }, readMarks: {} });
  const plan = planLegacyReadMarkMigration(current, '{"server:channel":123}');
  assert.equal(plan.saveBeforeRemoval, true);
  assert.equal(plan.removeLegacy, true);
  assert.deepEqual(plan.state.readMarks, { "server:channel": { ts: 123, id: "" } });
  assert.deepEqual(plan.state.drafts, { room: "hello" });
});

test("sealed state wins over stale legacy data and malformed legacy data is preserved", () => {
  const current = sanitizeUiContinuity({ drafts: {}, readMarks: { sealed: { ts: 8, id: "m" } } });
  const stale = planLegacyReadMarkMigration(current, '{"legacy":99}');
  assert.equal(stale.saveBeforeRemoval, false);
  assert.equal(stale.removeLegacy, true);
  assert.deepEqual(stale.state.readMarks, { sealed: { ts: 8, id: "m" } });

  const malformed = planLegacyReadMarkMigration(
    sanitizeUiContinuity({ drafts: {}, readMarks: {} }),
    "not json",
  );
  assert.equal(malformed.saveBeforeRemoval, false);
  assert.equal(malformed.removeLegacy, false);
});
