import test from "node:test";
import assert from "node:assert/strict";
import { fileAvailability, keptCopyLabel, keptTransferLabel, keptAllocationCount, keptTransferRows, fileInventoryRequests, mutateFileInventory } from "./file-availability.ts";
import type { KeptTransfer } from "./file-availability.ts";

test("partial progress stays separate from a completed kept copy and exhaustion needs explicit retry", () => {
  const copy: KeptTransfer = { cid: "a", manifest_version: "b", transfer: "c", phase: "paused",
    verified_chunks: 1, total_chunks: 2, running: false, exhausted: false, grant: "9007199254740993", intent: "9007199254740994", user_paused: true, blocked: false };
  assert.equal(keptTransferLabel(copy), "Paused · 1 of 2 parts saved");
  assert.equal(keptTransferLabel({ ...copy, user_paused: false }), "Waiting to resume \u00b7 1 of 2 parts saved");
  assert.equal(keptTransferLabel({ ...copy, blocked: true }), "Needs checking \u00b7 1 of 2 parts saved");
  assert.equal(keptTransferLabel({ ...copy, running: true }), "Saving… · 1 of 2 parts saved");
  assert.equal(keptTransferLabel({ ...copy, exhausted: true }), "Retry required · 1 of 2 parts saved");
  assert.equal(keptTransferLabel({ ...copy, phase: "deleting" }), "Removing…");
  assert.equal(keptCopyLabel(undefined), "No kept copy");
});

test("a connected member does not prove file possession", () => {
  assert.equal(fileAvailability(0, 1, true).label, "Remote copy unconfirmed");
  assert.equal(fileAvailability(0, 1, false).label, "No connected provider");
  assert.equal(fileAvailability(1, 2, true).label, "Partial 1/2");
  assert.equal(fileAvailability(2, 2, false).label, "Cached here");
  assert.notEqual(fileAvailability(0, 0, false).cls, "local");
});

test("a failed completed release stays visible and charged without double-counting healthy copies", () => {
  const completed: KeptTransfer = { cid: "a", manifest_version: "b", transfer: "c", phase: "completed",
    verified_chunks: 2, total_chunks: 2, running: false, exhausted: false, grant: "1", intent: "1", user_paused: false, blocked: false };
  const view = { supported: true, allocated_bytes: 200_000, limit_bytes: 1_073_741_824,
    files: [{ cid: "a", manifest_version: "b", checked: true }], transfers: [completed], error: null };
  assert.equal(keptAllocationCount(view), 1);
  assert.deepEqual(keptTransferRows(view), []);
  const faulted = { ...view, files: [], error: "retained transfer needs a storage check" };
  assert.equal(keptAllocationCount(faulted), 1);
  assert.deepEqual(keptTransferRows(faulted), [completed]);
  assert.equal(keptTransferLabel(completed), "Storage needs checking");
});

test("saved metadata alone never reports verified retention", () => {
  assert.equal(keptCopyLabel(undefined), "No kept copy");
  const file = { cid: "a", manifest_version: "b", checked: false };
  assert.equal(keptCopyLabel(file), "Saved copy · needs checking");
  assert.equal(keptCopyLabel({ ...file, checked: true }), "Kept here · verified this session");
});

test("reordered snapshots cannot resurrect a released copy or publish after lock", () => {
  const gate = fileInventoryRequests();
  const old = gate.begin();
  gate.invalidate(); // user releases the copy while its old read is still in flight
  assert.equal(gate.current(old, true), false);
  const fresh = gate.begin();
  assert.equal(gate.current(fresh, true), true);
  assert.equal(gate.current(old, true), false); // older result arrives last
  assert.equal(gate.current(fresh, false), false); // final session/view check observes lock
});

test("failed release refreshes its partially committed state only in the current unlocked view", async () => {
  let refreshes = 0;
  const failure = async () => { throw new Error("delete failed after rename"); };
  const refresh = async () => { refreshes++; };
  await assert.rejects(mutateFileInventory(failure, refresh, () => true), /after rename/);
  assert.equal(refreshes, 1);
  await assert.rejects(mutateFileInventory(failure, refresh, () => false), /after rename/);
  assert.equal(refreshes, 1);
});
