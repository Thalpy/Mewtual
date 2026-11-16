import test from "node:test";
import assert from "node:assert/strict";
import { NativeSdpReplyLoss, type NativeSdpObservation } from "../scripts/voice-native-reply-loss.ts";
const seal = "1".repeat(32), open = "2".repeat(32);
const full: NativeSdpObservation = { complete: true, firstRow: seal,
  seal: { row: seal, accepted: true, drained: true, applied: false },
  open: { row: open, accepted: true, drained: true, applied: true } };
test("loss evidence cannot count the same Result twice after a transient empty oracle", () => {
  const loss = new NativeSdpReplyLoss();
  assert.equal(loss.discard(19, seal, { complete: false, firstRow: null, seal: null, open: null }), false);
  assert.equal(loss.discard(20, open, full), true);
  assert.equal(loss.discard(20, open, full), false);
  assert.equal(loss.discard(19, open, full), false, "wrong original row cannot stand in for Seal");
  assert.equal(loss.count, 1);
  assert.equal(loss.discard(19, seal, full), true); assert.equal(loss.count, 2);
});
test("only the actual first native row Return can satisfy first-Return loss", () => {
  const loss = new NativeSdpReplyLoss();
  assert.equal(loss.discard(24, open, full), false);
  assert.equal(loss.discard(24, seal, { ...full, seal: { ...full.seal!, drained: false } }), false);
  assert.equal(loss.discard(24, seal, full), true); assert.equal(loss.discard(24, seal, full), false);
  assert.equal(loss.discard(26, seal, full), false); assert.equal(loss.discard(26, open, full), true);
  assert.equal(loss.discard(26, open, full), false); assert.equal(loss.count, 2);
});
