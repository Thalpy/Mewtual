import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
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
    version: 1, drafts: { good: "draft" }, readMarks: { good: { ts: 42, id: "m1" } }, statusCursors: {}, fileTrustPolicies: {},
  });
  assert.deepEqual(sanitizeUiContinuity({ drafts: [], readMarks: null }), {
    version: 1, drafts: {}, readMarks: {}, statusCursors: {}, fileTrustPolicies: {},
  });
});

test("announcement read cursors are sealed here too, keyed by a real server id", () => {
  // These were plaintext localStorage before this: how far somebody has read is a reading habit,
  // and reading habits belong in the vault beside the chat marks rather than next to them in the
  // clear. The bounds are the cursor module's; what this adds is the key.
  const state = sanitizeUiContinuity({
    drafts: {},
    readMarks: {},
    statusCursors: {
      0: { ts: 5, ids: ["a"] },            // server 0 is a real id
      7: { ts: 900.7, ids: ["b", "b", 3] },
      "": { ts: 8, ids: [] },              // names no server; must not become server 0
      " 9": { ts: 8, ids: [] },
      "1e3": { ts: 8, ids: [] },
      "-1": { ts: 8, ids: [] },
      dm: { ts: 8, ids: [] },
      4: "not a cursor",
      5: { ts: 0, ids: [] },               // indistinguishable from having none
    },
  });
  assert.deepEqual(state.statusCursors, { 0: { ts: 5, ids: ["a"] }, 7: { ts: 900, ids: ["b"] } });
  assert.deepEqual(sanitizeUiContinuity({ drafts: {}, readMarks: {} }).statusCursors, {});
});

test("file trust policies are vault continuity and fail closed when malformed", () => {
  const state = sanitizeUiContinuity({
    drafts: {},
    readMarks: {},
    fileTrustPolicies: {
      4: { mode: "specific", trustedAuthors: ["member-a", "member-a", "member-b"] },
      5: { mode: "not-a-mode", trustedAuthors: "everyone" },
    },
  });
  assert.deepEqual(state.fileTrustPolicies, {
    4: { mode: "specific", trustedAuthors: ["member-a", "member-b"] },
    5: { mode: "on-demand", trustedAuthors: [] },
  });
});

test("ordinary window close awaits the native final vault snapshot before destruction", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("appWindow.onCloseRequested(async (event) => {");
  const end = source.indexOf("// Capture is a native setting", start);
  assert.ok(start >= 0 && end > start);
  const handler = source.slice(start, end);
  assert.match(handler, /event\.preventDefault\(\)/);
  assert.ok(handler.indexOf('await invoke("lock_session"') < handler.indexOf("await appWindow.destroy()"));
  assert.match(handler, /if \(nativeLocked\) lockScreen\(true\)/);
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
  const current = sanitizeUiContinuity({
    drafts: { room: "hello" }, readMarks: {}, statusCursors: { 3: { ts: 7, ids: [] } },
    fileTrustPolicies: { 3: { mode: "everyone", trustedAuthors: [] } },
  });
  const plan = planLegacyReadMarkMigration(current, '{"server:channel":123}');
  assert.equal(plan.saveBeforeRemoval, true);
  assert.equal(plan.removeLegacy, true);
  assert.deepEqual(plan.state.readMarks, { "server:channel": { ts: 123, id: "" } });
  assert.deepEqual(plan.state.drafts, { room: "hello" });
  // The legacy key only ever held chat marks, so adopting it must not drop what sits beside them.
  assert.deepEqual(plan.state.statusCursors, { 3: { ts: 7, ids: [] } });
  assert.deepEqual(plan.state.fileTrustPolicies, { 3: { mode: "everyone", trustedAuthors: [] } });
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
