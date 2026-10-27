import assert from "node:assert/strict";
import test from "node:test";
import { parseFaultView, readStudioFault, repairStudioFault } from "./studio-fault.ts";
import type { StudioIpc } from "./studio-native.ts";
const target = { server: 1, channel: "7", object: "1".repeat(32) };
const a = "a".repeat(64), b = "b".repeat(64);
const candidate = (receipt: string) => ({ receipt, closedEpoch: "3", closeRecord: "c".repeat(64), seedChange: "d".repeat(64), inheritedEpoch: null, locallyInstalled: receipt === a });
const raw = () => ({ v: 1, kind: "fault", channel: "7", object: target.object, scope: { kind: "source" }, source: { phase: "fault" }, candidates: [candidate(a), candidate(b)], mayDecide: true, blockedBy: null, waiting: 0, preservedOperations: 2, repair: null, lastAttempt: null });
test("fault observations preserve distinct actual candidates and reject mismatched scope/target", () => {
  const view = parseFaultView(raw(), target, "source"); assert.equal(view.candidates[0].receipt, a); assert.equal(view.preservedOperations, 2);
  for (const value of [{ ...raw(), channel: "8" }, { ...raw(), object: "2".repeat(32) }, { ...raw(), candidates: [candidate(a), candidate(a)] }, { ...raw(), candidates: null }, { ...raw(), scope: { kind: "registryBucket", bucket: 3 } }]) assert.throws(() => parseFaultView(value, target, "source"));
  assert.throws(() => parseFaultView(raw(), target, "registry"));
});
test("repair echoes only candidate hashes and reports scheduled, never repaired", async () => {
  const calls: { command: string; args: unknown }[] = [];
  const ipc = { invoke: async (command: string, args: unknown) => { calls.push({ command, args }); return command === "studio_fault_read" ? raw() : { v: 1, kind: "faultRepairStarted", channel: "7", scope: { kind: "source" }, start: "scheduled", refreshRequired: true }; } } as StudioIpc;
  const view = await readStudioFault(ipc, target, "source");
  assert.equal(await repairStudioFault(ipc, target, "source", view, b), "scheduled");
  assert.deepEqual(calls[1].args, { ...target, scope: "source", decision: { receiptA: a, receiptB: b, selected: b } });
  await assert.rejects(repairStudioFault(ipc, target, "source", view, "e".repeat(64))); assert.equal(calls.length, 2);
  await assert.rejects(repairStudioFault(ipc, target, "source", { ...view, mayDecide: false }, a));
});

test("repair failure/hold diagnostics stay bounded plain display text", () => {
  const failed = parseFaultView({ ...raw(), lastAttempt: { outcome: "failed", error: "<script>" + "x".repeat(3000) } }, target, "source");
  assert.equal(failed.lastOutcome, "failed"); assert.equal(failed.lastDetail.length, 2048); assert.ok(failed.lastDetail.startsWith("<script>"));
  const held = parseFaultView({ ...raw(), lastAttempt: { outcome: "held", hold: "repairInProgress" } }, target, "source");
  assert.equal(held.lastDetail, "repairInProgress");
});
