import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoiceSdpPhase, encodeVoiceSdpPhase } from "./voice-ipc-sdp.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/sdp-vectors.json", import.meta.url), "utf8"));
const expected = { parentRow: vectors.parentRow, identity: vectors.identity, handshake: vectors.handshake, key: vectors.key, sequence: vectors.sequence };
test("SDP storage control matches native fixed vectors with full u64 identities", () => {
  for (const vector of vectors.requests) {
    const encoded = encodeVoiceSdpPhase({ ...expected, request: vector.name === "prepare" ? { kind: "prepare" } : { kind: "release", proof: vectors.proof } });
    assert.equal(encoded.operation, 12); assert.equal(encoded.kind, 0); assert.equal(encoded.logical, expected.parentRow);
    assert.equal(Buffer.from(encoded.body).toString("hex"), vector.body);
  }
  for (const vector of vectors.replies) {
    const decoded = decodeVoiceSdpPhase(Buffer.from(vector.body, "hex"), expected);
    assert.equal(decoded.state, vector.name); assert.equal(decoded.proof, vectors.proof);
  }
});
test("SDP storage replies refuse every truncation, trailing byte and changed original binding", () => {
  const bytes = Buffer.from(vectors.replies[1].body, "hex");
  for (let end = 0; end < bytes.length; end++) assert.throws(() => decodeVoiceSdpPhase(bytes.subarray(0, end), expected));
  assert.throws(() => decodeVoiceSdpPhase(Buffer.concat([bytes, Buffer.of(0)]), expected));
  for (const offset of [0, 2, 17, 18, 25, 26, 33, 34, 65, 66, 97, 98, 129, 130, 137]) {
    const changed = Buffer.from(bytes); changed[offset] ^= 1; assert.throws(() => decodeVoiceSdpPhase(changed, expected), `offset ${offset}`);
  }
  for (const state of [4, 255]) { const changed = Buffer.from(bytes); changed[1] = state; assert.throws(() => decodeVoiceSdpPhase(changed, expected)); }
  for (const field of ["action", "version"]) {
    assert.throws(() => encodeVoiceSdpPhase({ ...expected, identity: { ...expected.identity, [field]: "0" }, request: { kind: "prepare" } }));
  }
  for (const sequence of ["0", "01", "-1", "18446744073709551616"]) assert.throws(() => encodeVoiceSdpPhase({ ...expected, sequence, request: { kind: "prepare" } }));
  for (const parentRow of ["0".repeat(32), "0000000000000020" + "0".repeat(15) + "1", expected.parentRow.toUpperCase()]) assert.throws(() => encodeVoiceSdpPhase({ ...expected, parentRow, request: { kind: "prepare" } }));
});

test("SDP storage receipt owns a fixed immutable graph independent of caller metadata", () => {
  const source = { ...expected, identity: { ...expected.identity }, unrelated: { large: new Uint8Array(8192) } };
  const receipt = decodeVoiceSdpPhase(Buffer.from(vectors.replies[1].body, "hex"), source);
  source.identity.action = "1"; source.identity.inputTag = "ff".repeat(32);
  assert.equal(receipt.identity.action, expected.identity.action); assert.equal(receipt.identity.inputTag, expected.identity.inputTag);
  assert.equal(Object.hasOwn(receipt, "unrelated"), false);
  assert.ok(Object.isFrozen(receipt)); assert.ok(Object.isFrozen(receipt.identity));
});
