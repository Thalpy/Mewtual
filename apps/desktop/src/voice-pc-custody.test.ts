import assert from "node:assert/strict";
import test from "node:test";
import { VoicePcCustody } from "./voice-pc-custody.ts";
import type { VoicePcReceipt } from "./voice-ipc-pc.ts";

const identity = Object.freeze({ generation: "1" as const, proof: "22".repeat(32) });
const admitted = (remainingMs = 100): Extract<VoicePcReceipt, { state: "admitted" }> =>
  ({ state: "admitted", identity, profile: "complete-sdp-v1", ready: "00".repeat(32), localIsLow: true, remainingMs });

test("never-started PC custody has no synthetic Close; unknown admission always owes it", () => {
  const unused = new VoicePcCustody(); unused.close();
  assert.equal(unused.nativeClosed, true); assert.throws(() => unused.begin(), /closed/);
  const unknown = new VoicePcCustody(); unknown.begin(); unknown.close();
  assert.equal(unknown.needsClose, true);
  unknown.observe({ state: "closed", identity: null }, 0, 1);
  assert.equal(unknown.nativeClosed, true); assert.throws(() => unknown.begin(), /closed/);
  assert.throws(() => unknown.observe({ state: "closed", identity }, 1, 2), /identity changed/);
});

test("preparing/late admitted observations retain exact identity and cannot reopen after End", () => {
  const pc = new VoicePcCustody(); pc.begin();
  pc.observe({ state: "preparing", identity }, 0, 1); pc.close();
  pc.observe(admitted(), 1, 2); assert.equal(pc.needsClose, true);
  assert.throws(() => pc.begin(), /closed/);
  assert.throws(() => pc.observe({ state: "closed", identity: null }, 2, 3), /identity changed/);
  assert.throws(() => pc.observe({ ...admitted(), identity: { ...identity, proof: "33".repeat(32) } }, 2, 3), /identity changed/);
  pc.observe({ state: "closed", identity }, 2, 3);
  assert.equal(pc.nativeClosed, true);
  assert.throws(() => pc.observe(admitted(), 3, 4), /reopened/);
});

test("first Ready/role/profile cannot change and preparing cannot replace admission", () => {
  const pc = new VoicePcCustody(); pc.begin(); pc.observe(admitted(), 0, 1);
  assert.throws(() => pc.observe({ ...admitted(), ready: "33".repeat(32) }, 1, 2), /admission changed/);
  assert.throws(() => pc.observe({ ...admitted(), localIsLow: false }, 1, 2), /admission changed/);
  assert.throws(() => pc.observe({ state: "preparing", identity }, 1, 2), /backwards/);
  pc.observe(admitted(50), 2, 3); pc.begin(); // Same finite intent; no new identity.
  pc.close(); assert.equal(pc.needsClose, true);
});

test("request-start deadline only shortens and delayed replies never renew the first window", () => {
  const pc = new VoicePcCustody(); pc.begin(); pc.observe(admitted(100), 10, 20); // deadline110
  pc.observe(admitted(100), 30, 40); // candidate130 cannot extend110
  pc.observe(admitted(100), 50, 110); // original deadline reached
  assert.throws(() => pc.begin(), /closed/); assert.equal(pc.needsClose, true);
  const late = new VoicePcCustody(); late.begin(); late.observe(admitted(10), 0, 10);
  assert.throws(() => late.begin(), /closed/);
  const shorter = new VoicePcCustody(); shorter.begin(); shorter.observe(admitted(100), 0, 1);
  shorter.observe(admitted(5), 10, 15); assert.throws(() => shorter.begin(), /closed/);
});

test("invalid clock observations leave actual PC cleanup unresolved", () => {
  const pc = new VoicePcCustody(); pc.begin();
  for (const [start, end] of [[NaN, 1], [0, Infinity], [-1, 0], [10, 9]]) {
    assert.throws(() => pc.observe({ state: "closed", identity: null }, start, end), /observation changed/);
    assert.equal(pc.needsClose, true);
  }
  pc.observe({ state: "closed", identity: null }, 0, 0); assert.equal(pc.nativeClosed, true);
});

test("PC identity snapshots do not retain caller-mutable metadata", () => {
  const pc = new VoicePcCustody(); pc.begin();
  const mutable = { ...identity }; pc.observe({ state: "preparing", identity: mutable }, 0, 1);
  mutable.proof = "33".repeat(32);
  assert.throws(() => pc.observe({ state: "closed", identity: mutable }, 1, 2), /identity changed/);
  pc.observe({ state: "closed", identity }, 1, 2); assert.equal(pc.nativeClosed, true);
});
