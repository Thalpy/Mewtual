import test from "node:test";
import assert from "node:assert/strict";
import { fileAvailability, keptCopyLabel, fileInventoryRequests, mutateFileInventory } from "./file-availability.ts";

test("a connected member does not prove file possession", () => {
  assert.equal(fileAvailability(0, 1, true).label, "Remote copy unconfirmed");
  assert.equal(fileAvailability(0, 1, false).label, "No connected provider");
  assert.equal(fileAvailability(1, 2, true).label, "Partial 1/2");
  assert.equal(fileAvailability(2, 2, false).label, "Cached here");
  assert.notEqual(fileAvailability(0, 0, false).cls, "local");
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
