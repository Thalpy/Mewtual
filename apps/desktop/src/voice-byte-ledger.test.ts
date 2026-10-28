import assert from "node:assert/strict";
import test from "node:test";
import { VoiceByteLedger, reserveVoiceBytePayment } from "./voice-byte-ledger.ts";

test("detached payment shares ordinary accounting and refunds only its original group", () => {
  const ledger = new VoiceByteLedger(64, 32);
  ledger.charge("a", 8);
  const old = reserveVoiceBytePayment(ledger, "a", 16);
  ledger.charge("b", 24);
  assert.equal(ledger.bytes, 48);
  old.release(); old.release();
  assert.equal(ledger.bytes, 32);
  assert.deepEqual([...ledger.groups], [["a", 8], ["b", 24]]);
  assert.equal(Object.isFrozen(old), true);
  assert.deepEqual(Reflect.ownKeys(old), ["release"]);
  ledger.refund("a", 8); ledger.refund("b", 24);
  assert.equal(ledger.bytes, 0); assert.equal(ledger.groups.size, 0);
});

test("group and process pressure refuse atomically while other admitted work remains intact", () => {
  const ledger = new VoiceByteLedger(48, 32);
  const first = reserveVoiceBytePayment(ledger, "a", 32);
  const before = [...ledger.groups];
  assert.throws(() => reserveVoiceBytePayment(ledger, "a", 1));
  assert.throws(() => reserveVoiceBytePayment(ledger, "b", 17));
  assert.equal(ledger.bytes, 32); assert.deepEqual([...ledger.groups], before);
  const second = reserveVoiceBytePayment(ledger, "b", 16);
  assert.equal(ledger.bytes, 48);
  first.release(); second.release();
  assert.equal(ledger.bytes, 0); assert.equal(ledger.groups.size, 0);
});

test("late original release cannot remove a same-group replacement charge", () => {
  const ledger = new VoiceByteLedger(64, 32);
  const old = reserveVoiceBytePayment(ledger, "a", 16);
  old.release();
  const next = reserveVoiceBytePayment(ledger, "a", 24);
  old.release();
  assert.equal(ledger.bytes, 24); assert.equal(ledger.groups.get("a"), 24);
  next.release(); assert.equal(ledger.bytes, 0);
});

test("one fixed process graph can carry two conservative group obligations", () => {
  const ledger = new VoiceByteLedger(64, 32);
  ledger.chargeProcess(4);
  ledger.bindGroup("old", 4); ledger.bindGroup("next", 4);
  const capsule = reserveVoiceBytePayment(ledger, "old", 8);
  assert.equal(ledger.bytes, 12);
  assert.deepEqual([...ledger.groups], [["old", 12], ["next", 4]]);
  capsule.release(); ledger.unbindGroup("old", 4);
  assert.equal(ledger.bytes, 4); assert.deepEqual([...ledger.groups], [["next", 4]]);
  ledger.unbindGroup("next", 4); assert.equal(ledger.groups.size, 0);
  assert.equal(ledger.bytes, 4, "window infrastructure remains paid");
});

test("invalid amounts and oversized group bindings never change ownership", () => {
  const ledger = new VoiceByteLedger(64, 32);
  for (const bytes of [0, -1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER]) {
    assert.throws(() => reserveVoiceBytePayment(ledger, "a", bytes));
    assert.equal(ledger.bytes, 0); assert.equal(ledger.groups.size, 0);
  }
  ledger.bindGroup("a", 32);
  assert.throws(() => ledger.bindGroup("a", 1));
  assert.throws(() => ledger.unbindGroup("a", 33));
  assert.equal(ledger.groups.get("a"), 32);
  ledger.unbindGroup("a", 32);
});
