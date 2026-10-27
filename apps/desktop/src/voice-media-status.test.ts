import { test } from "node:test";
import assert from "node:assert/strict";
import { VoiceMediaStatus, VOICE_MEDIA_STALE_MS } from "./voice-media-status.ts";
import type { VoiceMediaReceipt } from "./voice-ipc-media.ts";
const proof = "17".repeat(32);
const live = (changes: Partial<VoiceMediaReceipt> = {}): VoiceMediaReceipt => Object.freeze({
  proof, admitted: true, closed: false, released: false, current: true, retireInMs: 0, ...changes,
});
test("native media freshness is anchored before IPC, independently of retired SDP", () => {
  const status = new VoiceMediaStatus(proof); assert.equal(status.currentAt(1000), false);
  const attempt = status.begin(1000); status.finish(attempt, live(), 4000);
  assert.equal(status.deadline, 1000 + VOICE_MEDIA_STALE_MS);
  assert.equal(status.currentAt(5999), true); assert.equal(status.currentAt(6000), false);
  assert.equal(status.closed, true); assert.throws(() => status.begin(6000));
});
test("a delayed positive native reply cannot create or reopen an expired observation", () => {
  const status = new VoiceMediaStatus(proof); const attempt = status.begin(1000);
  status.finish(attempt, live(), 6000); assert.equal(status.currentAt(6000), false); assert.equal(status.closed, true);
});
test("capacity preserves only the previous immutable deadline", () => {
  const status = new VoiceMediaStatus(proof); status.finish(status.begin(1000), live(), 1001);
  status.finish(status.begin(4000), null, 5000);
  assert.equal(status.deadline, 6000); assert.equal(status.currentAt(5999), true);
  const late = status.begin(5999); status.finish(late, live(), 6000);
  assert.equal(status.closed, true); assert.equal(status.currentAt(6000), false);
});
test("a timely refresh uses its own request start, never reply completion", () => {
  const status = new VoiceMediaStatus(proof); status.finish(status.begin(1000), live(), 1100);
  status.finish(status.begin(4000), live(), 5500); assert.equal(status.deadline, 9000);
  assert.equal(status.currentAt(8999), true); assert.equal(status.currentAt(9000), false);
});
test("Close cannot retire an outstanding RPC or be undone by its later positive reply", () => {
  const status = new VoiceMediaStatus(proof), attempt = status.begin(1000);
  status.close(); assert.equal(status.pending, true); assert.equal(status.deadline, null);
  status.finish(attempt, live(), 1100); assert.equal(status.pending, false); assert.equal(status.currentAt(1100), false);
  assert.throws(() => status.begin(1100));
});
test("foreign and duplicate completion cannot consume the actual outstanding attempt", () => {
  const status = new VoiceMediaStatus(proof), attempt = status.begin(1000);
  assert.throws(() => status.begin(1000)); assert.throws(() => status.finish({}, live(), 1001));
  assert.equal(status.pending, true); status.finish(attempt, live(), 1001);
  assert.throws(() => status.finish(attempt, live(), 1002)); assert.equal(status.deadline, 6000);
});
for (const receipt of [live({ proof: "18".repeat(32) }), live({ current: false }), live({ admitted: false }), live({ closed: true }), live({ released: true })]) {
  test(`historical, foreign or negative native observation cannot authorize media ${JSON.stringify(receipt)}`, () => {
    const status = new VoiceMediaStatus(proof); status.finish(status.begin(1000), receipt, 1001);
    assert.equal(status.closed, true); assert.equal(status.currentAt(1001), false);
  });
}
test("first local refusal is retryable without granting a standing observation", () => {
  const status = new VoiceMediaStatus(proof); status.finish(status.begin(1000), null, 2000);
  assert.equal(status.currentAt(2000), false); assert.equal(status.closed, false);
  status.finish(status.begin(2001), live(), 2002); assert.equal(status.deadline, 7001);
});
test("malformed clocks and rollback fail closed instead of extending validity", () => {
  for (const now of [NaN, Infinity, -Infinity, 999]) {
    const status = new VoiceMediaStatus(proof); status.finish(status.begin(1000), live(), 1001);
    assert.equal(status.currentAt(now), false); assert.equal(status.closed, true);
  }
});
test("metadata getter reentry cannot clear a close or begin another status flight", () => {
  const status = new VoiceMediaStatus(proof), attempt = status.begin(1000);
  const receipt = { ...live(), get current() {
    assert.throws(() => status.begin(1001)); assert.throws(() => status.finish(attempt, live(), 1001));
    status.close(); return true;
  } };
  status.finish(attempt, receipt, 1002); assert.equal(status.pending, false); assert.equal(status.currentAt(1002), false);
});
test("finite clock precision cannot round the promised freshness grace upward", () => {
  for (const now of [2 ** 65, Number.MAX_VALUE]) {
    const status = new VoiceMediaStatus(proof); assert.throws(() => status.begin(now));
    assert.equal(status.closed, true); assert.equal(status.pending, false);
  }
  const now = 2 ** 64, status = new VoiceMediaStatus(proof);
  status.finish(status.begin(now), live(), now);
  assert.ok(status.deadline! - now <= VOICE_MEDIA_STALE_MS, "rounding down is conservative");
});


test("ordinary fractional browser clocks do not turn rounding into an admission failure", () => {
  const now = 3192.004;
  assert.ok((now + VOICE_MEDIA_STALE_MS) - now > VOICE_MEDIA_STALE_MS, "pin the real floating-point counterexample");
  const status = new VoiceMediaStatus(proof);
  status.finish(status.begin(now), live(), now + 1);
  assert.ok(status.deadline! - now <= VOICE_MEDIA_STALE_MS);
  assert.ok(status.deadline! - now >= VOICE_MEDIA_STALE_MS - 1);
  assert.equal(status.currentAt(now + 4999), true); assert.equal(status.currentAt(8192), false);
});
