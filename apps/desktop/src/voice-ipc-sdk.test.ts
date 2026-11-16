import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoiceSdkCustody, encodeVoiceSdkCustody } from "./voice-ipc-sdk.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/sdk-vectors.json", import.meta.url), "utf8"));
const expected = { parentRow: vectors.parentRow, identity: vectors.identity, handshake: vectors.handshake, key: vectors.key, sequence: vectors.sequence, storage: vectors.storage };
test("SDK custody matches closed native vectors without narrowing generation or original identities", () => {
  for (const vector of vectors.requests) {
    const encoded = encodeVoiceSdkCustody({ ...expected, request: vector.name === "admit" ? { kind: "admit" } : { kind: "release", generation: vectors.generation, proof: vectors.proof } });
    assert.equal(encoded.operation, 13); assert.equal(encoded.kind, 0); assert.equal(encoded.logical, expected.parentRow); assert.equal(Buffer.from(encoded.body).toString("hex"), vector.body);
  }
  for (const vector of vectors.replies) { const reply = decodeVoiceSdkCustody(Buffer.from(vector.body, "hex"), expected); assert.equal(reply.state, vector.name); assert.equal(reply.generation, vectors.generation); assert.equal(reply.proof, vectors.proof); }
});
test("SDK custody refuses truncated, extra, wrong-storage, zero-generation and noncanonical control metadata", () => {
  const bytes = Buffer.from(vectors.replies[0].body, "hex");
  for (let end = 0; end < bytes.length; end++) assert.throws(() => decodeVoiceSdkCustody(bytes.subarray(0, end), expected));
  assert.throws(() => decodeVoiceSdkCustody(Buffer.concat([bytes, Buffer.of(0)]), expected));
  for (const offset of [0, 2, 17, 18, 25, 26, 33, 34, 65, 66, 97, 98, 129, 130, 137, 138, 169]) { const changed = Buffer.from(bytes); changed[offset] ^= 1; assert.throws(() => decodeVoiceSdkCustody(changed, expected), `offset ${offset}`); }
  for (const state of [0, 4, 255]) { const changed = Buffer.from(bytes); changed[1] = state; assert.throws(() => decodeVoiceSdkCustody(changed, expected)); }
  const zero = Buffer.from(bytes); zero.fill(0, 170, 178); assert.throws(() => decodeVoiceSdkCustody(zero, expected));
  for (const generation of ["0", "01", "-1", "18446744073709551616"]) assert.throws(() => encodeVoiceSdkCustody({ ...expected, request: { kind: "release", generation, proof: vectors.proof } }));
  for (const storage of ["", "ab", "AB".repeat(32)]) assert.throws(() => encodeVoiceSdkCustody({ ...expected, storage, request: { kind: "admit" } }));
});
test("SDK custody receipt never aliases caller identity or retains unrelated graphs", () => {
  const source = { ...expected, identity: { ...expected.identity }, extra: new Uint8Array(8192) };
  const reply = decodeVoiceSdkCustody(Buffer.from(vectors.replies[0].body, "hex"), source); source.identity.action = "1";
  assert.equal(reply.identity.action, expected.identity.action); assert.equal(Object.hasOwn(reply, "extra"), false); assert.ok(Object.isFrozen(reply)); assert.ok(Object.isFrozen(reply.identity));
});
