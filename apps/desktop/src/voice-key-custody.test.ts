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
