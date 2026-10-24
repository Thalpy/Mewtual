import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoiceTransition, encodeVoiceTransition, type VoiceTransitionRequest } from "./voice-ipc-transition.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/transition/vectors.json", import.meta.url), "utf8"));
const bytes = (value: string) => new Uint8Array(Buffer.from(value, "hex"));

test("custody request vectors share native bytes without narrowing u64s", () => {
  for (const value of vectors.requests) {
    const result = encodeVoiceTransition(value);
    assert.equal(result.operation, 10); assert.equal(result.kind, 0); assert.equal(result.logical, value.parentRow);
    assert.equal(Buffer.from(result.body).toString("hex"), value.body);
    assert.equal(result.body.length, value.request.kind === "claim" ? 186 : 218);
  }
});
test("custody requests reject noncanonical identities before constructing body", () => {
  const base = vectors.requests[0] as VoiceTransitionRequest;
  for (const parentRow of ["00".repeat(16), "00000000000000200000000000000001", "A".repeat(32), "1"]) {
    assert.throws(() => encodeVoiceTransition({ ...base, parentRow }));
  }
  for (const action of ["0", "01", "-1", "18446744073709551616", "1e3"]) {
    assert.throws(() => encodeVoiceTransition({ ...base, identity: { ...base.identity, action } }));
  }
  assert.throws(() => encodeVoiceTransition({ ...base, handshake: "x".repeat(64) }));
  assert.throws(() => encodeVoiceTransition({ ...base, request: { kind: "release", receipt: "99" } }));
});
test("authenticated custody reply metadata pins native identity and closed status states", () => {
  for (const value of vectors.replies) {
    const result = decodeVoiceTransition(bytes(value.body), value.expected);
    assert.equal(result.state, ["", "borrowed", "closing", "released"][value.state]);
    assert.equal(result.until, value.until); assert.equal(result.remainingMs, value.remaining);
    assert.equal(result.identity.action, value.expected.identity.action);
    assert.equal(result.finishMessage, "66".repeat(32)); assert.equal(result.finishHash, "88".repeat(32));
    assert.equal(result.receipt, "99".repeat(32)); assert.ok(Object.isFrozen(result)); assert.ok(Object.isFrozen(result.identity));
  }
});
test("custody reply rejects every truncation, trailing byte and mismatched original binding", () => {
  const value = vectors.replies[0], body = bytes(value.body);
  for (let end = 0; end < body.length; end++) assert.throws(() => decodeVoiceTransition(body.subarray(0, end), value.expected));
  assert.throws(() => decodeVoiceTransition(new Uint8Array([...body, 0]), value.expected));
  for (const offset of [0,2,18,26,34,66,98]) {
    const changed = new Uint8Array(body); changed[offset] ^= 1;
    assert.throws(() => decodeVoiceTransition(changed, value.expected), `binding offset ${offset}`);
  }
});
test("custody status cannot represent current authority with an absent or renewed time allowance", () => {
  const value = vectors.replies[0];
  for (const state of [0,4,255]) { const changed = bytes(value.body); changed[1] = state;
    assert.throws(() => decodeVoiceTransition(changed, value.expected)); }
  for (const remaining of [0,30001,0xffffffff]) { const changed = bytes(value.body);
    new DataView(changed.buffer).setUint32(178, remaining); assert.throws(() => decodeVoiceTransition(changed, value.expected)); }
  for (const state of [2,3]) { const changed = bytes(value.body); changed[1] = state;
    assert.throws(() => decodeVoiceTransition(changed, value.expected)); }
  for (const until of [0n,29999n,(1n<<64n)-1n]) { const changed = bytes(value.body);
    new DataView(changed.buffer).setBigUint64(170, until); assert.throws(() => decodeVoiceTransition(changed, value.expected)); }
});
