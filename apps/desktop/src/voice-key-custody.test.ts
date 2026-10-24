import assert from "node:assert/strict";
import test from "node:test";
import { VoiceKeyCustody } from "./voice-key-custody.ts";
import type { VoiceTransitionReceipt } from "./voice-ipc-transition.ts";
const receipt: VoiceTransitionReceipt = Object.freeze({ state: "borrowed", parentRow: "parent",
  identity: Object.freeze({ action: "1", version: "1", inputTag: "tag" }), handshake: "handshake", epoch: "0",
  finishMessage: "message", finishHash: "hash", until: "9007199254740993", remainingMs: 29_999, receipt: "receipt" });
test("provisional Finish custody has one fixed ceiling without renewing confirmation", () => {
  const custody = new VoiceKeyCustody(15_000, 14_999);
  assert.equal(custody.expires, 45_000);
  assert.ok(custody.current(15_001)); assert.equal(custody.current(45_000), false);
  assert.throws(() => new VoiceKeyCustody(15_000, 15_000));
  assert.equal(new VoiceKeyCustody(15_000.25, 14_999.75).expires, 45_000.25,
    "real performance.now deadlines need not be integer milliseconds");
});
test("late Claim uses request start and opaque native until; repeats only shrink", () => {
  const custody = new VoiceKeyCustody(15_000, 14_999);
  custody.observe(receipt, 16_000);
  assert.equal(custody.expires, 45_000);
  custody.observe({ ...receipt, remainingMs: 100 }, 20_000);
  assert.equal(custody.expires, 20_100);
  custody.observe(receipt, 20_001);
  assert.equal(custody.expires, 20_100);
  assert.equal(custody.current(20_100), false);
  assert.equal(custody.needsRelease, true, "expired custody still owes its exact Release");
});
test("closing/lost Release retain exact proof and released custody never reopens", () => {
  const custody = new VoiceKeyCustody(15_000, 14_999);
  custody.observe({ ...receipt, state: "closing", remainingMs: 0 }, 16_000);
  assert.equal(custody.current(16_000), false); assert.equal(custody.needsClaim, false);
  assert.equal(custody.needsRelease, true);
  assert.throws(() => custody.observe(receipt, 16_001), "Closing cannot become Borrowed again");
  custody.observe({ ...receipt, state: "released", remainingMs: 0 }, 16_001);
  assert.equal(custody.needsRelease, false);
  assert.throws(() => custody.observe(receipt, 16_002));
});
test("changed native immutable evidence refuses rather than replacing the original Claim", () => {
  for (const key of ["receipt", "until", "finishMessage", "finishHash", "parentRow", "epoch", "handshake"] as const) {
    const custody = new VoiceKeyCustody(15_000, 14_999); custody.observe(receipt, 100);
    assert.throws(() => custody.observe({ ...receipt, [key]: "changed" }, 200));
    assert.equal(custody.receipt, receipt);
  }
});
test("definite refusal ends browser custody but makes no native absence claim", () => {
  const custody = new VoiceKeyCustody(15_000, 14_999); custody.refuse();
  assert.equal(custody.current(14_999), false); assert.equal(custody.needsClaim, false);
  assert.equal(custody.receipt, null);
});


const keyReceipt = Object.freeze({ state: "current" as const, parentRow: receipt.parentRow,
  identity: receipt.identity, handshake: receipt.handshake, epoch: receipt.epoch,
  finishMessage: receipt.finishMessage, finishHash: receipt.finishHash, claim: receipt.receipt, receipt: "key receipt" });

test("pending adoption precedes RPC and preserves existing key custody beyond old deadline only", () => {
  const custody = new VoiceKeyCustody(15_000, 14_999); custody.observe(receipt, 100);
  const originalDeadline = custody.expires;
  custody.beginKeyAdoption(101);
  assert.equal(custody.needsKeyRecovery, true); assert.equal(custody.retentionDeadline, null);
  assert.equal(custody.current(100_000), true, "uncertain original adoption is quarantine, not recreated keys");
  assert.equal(custody.expires, originalDeadline, "old finite Claim never renews");
  custody.observeKeys(keyReceipt);
  assert.equal(custody.needsKeyRecovery, false); assert.equal(custody.needsKeyRelease, true);
  assert.equal(custody.current(100_000), true); assert.equal(custody.confirmationRetired, true);
});

test("adoption cannot begin with no Claim, expired custody, released Claim or after local End", () => {
  for (const mode of ["no claim", "expired", "released", "ended"] as const) {
    const custody = new VoiceKeyCustody(15_000, 14_999);
    if (mode !== "no claim") custody.observe({ ...receipt, state: mode === "released" ? "released" : "borrowed", remainingMs: mode === "released" ? 0 : 100 }, 100);
    if (mode === "ended") custody.closeKeys();
    assert.throws(() => custody.beginKeyAdoption(mode === "expired" ? 200 : 101), mode);
    assert.equal(custody.needsKeyRecovery, false);
  }
});

test("Closing, Released, definite refusal and End are irreversible across delayed Current replies", () => {
  for (const terminal of ["closing", "released", "refused", "ended"] as const) {
    const custody = new VoiceKeyCustody(15_000, 14_999); custody.observe(receipt, 100); custody.beginKeyAdoption(101);
    if (terminal === "refused") custody.refuseKeys();
    else if (terminal === "ended") custody.closeKeys();
    else custody.observeKeys({ ...keyReceipt, state: terminal });
    if (terminal === "closing" || terminal === "released") assert.throws(() => custody.observeKeys(keyReceipt));
    else custody.observeKeys(keyReceipt);
    assert.equal(custody.current(102), false, terminal);
  }
});

test("old Release and its deadline remain independent of the separate key obligation", () => {
  const custody = new VoiceKeyCustody(15_000, 14_999); custody.observe(receipt, 100); custody.beginKeyAdoption(101);
  custody.observeKeys(keyReceipt);
  custody.observe({ ...receipt, state: "released", remainingMs: 0 }, 102);
  assert.equal(custody.needsRelease, false); assert.equal(custody.needsKeyRelease, true);
  assert.equal(custody.current(103), true, "old cleanup alone cannot erase adopted key custody");
  custody.observeKeys({ ...keyReceipt, state: "released" });
  assert.equal(custody.current(103), false); assert.equal(custody.needsKeyRelease, false);
});

test("equal-looking key receipts cannot replace original Claim, action, Finish or adoption proof", () => {
  for (const field of ["parentRow", "handshake", "epoch", "finishMessage", "finishHash", "claim", "receipt"] as const) {
    const custody = new VoiceKeyCustody(15_000, 14_999); custody.observe(receipt, 100); custody.beginKeyAdoption(101);
    custody.observeKeys(keyReceipt);
    assert.throws(() => custody.observeKeys({ ...keyReceipt, [field]: "changed" }), field);
    assert.deepEqual(custody.keyReceipt, { state: keyReceipt.state, receipt: keyReceipt.receipt });
  }
});
