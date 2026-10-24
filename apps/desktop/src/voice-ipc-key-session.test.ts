import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { decodeVoiceKeySession, encodeVoiceKeySession, type VoiceKeySessionRequest } from "./voice-ipc-key-session.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/key-session-vectors.json", import.meta.url), "utf8"));
const bytes = (hex: string): Uint8Array => Uint8Array.from(Buffer.from(hex, "hex"));
test("opcode11 fixed Rust/browser vectors preserve full u64 and exact original Claim", () => {
  for (const vector of vectors.requests) {
    const encoded = encodeVoiceKeySession(vector);
    assert.equal(encoded.operation, 11); assert.equal(encoded.logical, vector.parentRow);
    assert.equal(Buffer.from(encoded.body).toString("hex"), vector.body);
  }
  for (const vector of vectors.replies) {
    const decoded = decodeVoiceKeySession(bytes(vector.body), vectors);
    assert.equal(decoded.state, vector.state); assert.equal(decoded.receipt, vector.receipt);
    assert.equal(decoded.identity.action, "9007199254740993");
    assert.equal(decoded.identity.version, "18446744073709551615");
    assert.equal(decoded.epoch, "18446744073709551615");
  }
});
test("key custody refuses changed original binding, legacy version, truncation and trailing bytes", () => {
  const body = bytes(vectors.replies[0].body);
  for (let length = 0; length < body.length; length++) assert.throws(() => decodeVoiceKeySession(body.subarray(0, length), vectors));
  assert.throws(() => decodeVoiceKeySession(Uint8Array.from([...body, 0]), vectors));
  for (const offset of [0, 2, 17, 18, 33, 34, 65, 66, 97, 98, 105, 106, 137, 138, 169, 170, 201]) {
    const changed = body.slice(); changed[offset] ^= 1; assert.throws(() => decodeVoiceKeySession(changed, vectors), `byte ${offset}`);
  }
  for (const state of [0, 4, 255]) { const changed = body.slice(); changed[1] = state; assert.throws(() => decodeVoiceKeySession(changed, vectors)); }
});
test("key adoption encoder has closed directions, canonical identifiers and bounded integers", () => {
  const original: VoiceKeySessionRequest = vectors.requests[0];
  for (const field of ["parentRow", "handshake", "claim"] as const) {
    assert.throws(() => encodeVoiceKeySession({ ...original, [field]: "changed" }));
  }
  for (const value of ["0", "01", "18446744073709551616", "-1", "1.0"]) {
    assert.throws(() => encodeVoiceKeySession({ ...original, identity: { ...original.identity, action: value } }));
  }
  assert.throws(() => encodeVoiceKeySession({ ...original, request: { kind: "claim" } } as unknown as VoiceKeySessionRequest));
  assert.throws(() => encodeVoiceKeySession({ ...original, request: { kind: "release", receipt: "bad" } }));
});
