import assert from "node:assert/strict";
import test from "node:test";
import { VoiceSdpSdkCustody } from "./voice-sdp-sdk-custody.ts";
import type { VoiceSdpPhaseReceipt } from "./voice-ipc-sdp.ts";
import type { VoiceSdkReceipt } from "./voice-ipc-sdk.ts";
const identity = { parentRow: "00000000000000000000000000000001", identity: { action: "1", version: "1", inputTag: "11".repeat(32) },
  handshake: "22".repeat(32), key: "33".repeat(32), sequence: "1" };
const storage = (state: VoiceSdpPhaseReceipt["state"]): VoiceSdpPhaseReceipt => ({ ...identity, state, proof: "44".repeat(32) });
const sdk = (state: VoiceSdkReceipt["state"]): VoiceSdkReceipt => ({ ...identity, state, storage: "44".repeat(32), generation: "1", proof: "55".repeat(32) });

test("closing storage and End never release an uncertain SDK obligation", () => {
  const phase = new VoiceSdpSdkCustody();
  assert.throws(() => phase.beginSdk(), /closed/);
  phase.observeStorage(storage("funded")); phase.beginSdk(); phase.close();
  phase.observeStorage(storage("drained"));
  assert.equal(phase.nativeReleased, false); assert.equal(phase.needsSdkRecovery, true);
  phase.observeSdk(sdk("held"));
  assert.equal(phase.closed, true, "a delayed Held cannot reopen local effects");
  assert.equal(phase.needsSdkRelease, true); assert.equal(phase.nativeReleased, false);
  phase.observeSdk(sdk("released")); assert.equal(phase.nativeReleased, true);
});

test("SDK/storage observations compact fixed fields and reject replacement or backward state", () => {
  const phase = new VoiceSdpSdkCustody();
  const first = storage("funded"); phase.observeStorage(first); phase.beginSdk(); phase.observeSdk(sdk("held"));
  assert.notEqual(phase.storage, first); assert.deepEqual(Object.keys(phase.storage!), ["state", "proof"]);
  assert.deepEqual(Object.keys(phase.sdk!), ["state", "generation", "proof"]);
  assert.ok(Object.isFrozen(phase.storage)); assert.ok(Object.isFrozen(phase.sdk));
  assert.throws(() => phase.observeStorage({ ...first, proof: "66".repeat(32) }), /changed/);
  assert.throws(() => phase.observeSdk({ ...sdk("held"), storage: "66".repeat(32) }), /changed/);
  assert.throws(() => phase.observeSdk({ ...sdk("held"), generation: "2" }), /changed/);
  assert.throws(() => phase.observeSdk({ ...sdk("held"), proof: "66".repeat(32) }), /changed/);
  phase.observeSdk(sdk("closing")); assert.throws(() => phase.observeSdk(sdk("held")), /changed/);
  phase.observeStorage(storage("closing")); assert.throws(() => phase.observeStorage(first), /changed/);
  phase.observeStorage(storage("drained")); phase.observeSdk(sdk("released"));
  assert.throws(() => phase.observeSdk(sdk("closing")), /changed/);
  assert.equal(phase.nativeReleased, true);
});

test("an authenticated first-admission refusal creates no invented native release proof", () => {
  const phase = new VoiceSdpSdkCustody(); phase.close();
  assert.equal(phase.nativeReleased, false);
  phase.refuseStorage(); assert.equal(phase.storage, null); assert.equal(phase.sdk, null);
  assert.equal(phase.nativeReleased, true, "original native Final is still required by the channel");
  assert.throws(() => phase.beginSdk(), /closed/);
});


test("SDK/storage drainage cannot erase an unknown PC Close obligation", () => {
  const phase = new VoiceSdpSdkCustody();
  assert.throws(() => phase.beginPc(), /closed/);
  phase.observeStorage(storage("funded")); phase.beginSdk();
  assert.throws(() => phase.beginPc(), /closed/);
  phase.observeSdk(sdk("held")); phase.beginPc(); phase.close();
  phase.observeSdk(sdk("released")); phase.observeStorage(storage("drained"));
  assert.equal(phase.nativeReleased, false); assert.equal(phase.pc.needsClose, true);
  phase.pc.observe({ state: "closed", identity: null }, 0, 1);
  assert.equal(phase.nativeReleased, true);
  assert.throws(() => phase.beginPc(), /closed/);
});
