import test from "node:test";
import assert from "node:assert/strict";
import { NativeAdmissionObservations } from "../scripts/voice-native-admission-observations.ts";
import { voiceIpcResponseAad, type VoiceIpcHeader } from "./voice-ipc-frame.ts";
const first = "1".repeat(32), second = "2".repeat(32);
function aad(operation: VoiceIpcHeader["operation"], logical: string, sequence: bigint, slot = 1) {
  const request: VoiceIpcHeader = { direction: 0, operation, kind: 0, logical, sequence, slot, length: 19 };
  return voiceIpcResponseAad({ ...request, direction: 1 }, request);
}
const capacity = new Uint8Array([1, 3, 7]);
test("indexed Current Capacity cannot authorize another parent's storage or SDK retry", () => {
  const observed = new NativeAdmissionObservations();
  observed.observe(aad(28, first, 1n), capacity);
  assert.equal(observed.count([12, 13], second), 0);
  observed.observe(aad(12, first, 2n), capacity);
  assert.equal(observed.count([12, 13], second), 0, "same opcode, wrong original parent");
  observed.observe(aad(12, second, 3n), capacity);
  assert.equal(observed.count([12, 13], second), 1);
  assert.equal(observed.count([14], second), 0, "storage refusal is not PC progress");
  observed.observe(aad(13, second, 4n), capacity);
  assert.equal(observed.count([12, 13], second), 2);
});
test("replayed, foreign-session, malformed and non-Capacity observations grant no retry", () => {
  const observed = new NativeAdmissionObservations();
  const original = aad(12, first, 5n);
  observed.observe(original, capacity); observed.observe(original, capacity);
  observed.observe(aad(12, first, 4n), capacity);
  observed.observe(aad(12, first, 6n, 2), capacity);
  observed.observe(new Uint8Array(80), capacity);
  observed.observe(aad(12, first, 6n), new Uint8Array([1, 3, 8]));
  observed.observe(aad(12, first, 7n), new Uint8Array([5, 1]));
  assert.equal(observed.count([12], first), 1);
  assert.equal(observed.describe(), `12/${first}/5`);
  assert.deepEqual(capacity, new Uint8Array([1, 3, 7]));
});
