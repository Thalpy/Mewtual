import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { encodeVoiceIpcAction, type VoiceIpcActionIdentity, type VoiceIpcActionRequest, type VoiceIpcPurpose } from "./voice-ipc-actions.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/vectors.json", import.meta.url), "utf8")) as {
  purpose: VoiceIpcPurpose; identity: VoiceIpcActionIdentity;
  cases: { name: string; operation: number; request: VoiceIpcActionRequest | { kind: "result"; identity: VoiceIpcActionIdentity; tag: string; output: string }; plaintext: string }[];
};
for (const item of vectors.cases) test(`native/browser action grammar: ${item.name}`, () => {
  const request = item.request.kind === "result" ? { ...item.request, output: Uint8Array.from(Buffer.from(item.request.output as string, "hex")) } : item.request;
  const encoded = encodeVoiceIpcAction(vectors.purpose, request);
  assert.equal(encoded.operation, item.operation);
  assert.equal(encoded.kind, item.operation === 3 ? 1 : 0);
  assert.equal(Buffer.from(encoded.body).toString("hex"), item.plaintext);
  assert.equal(encoded.body.byteLength, encoded.operation === 3 && request.kind === "result" ? 187 + request.output.byteLength : item.plaintext.length / 2);
  if (request.kind === "result") {
    const before = encoded.body.slice(); request.output.fill(0);
    assert.deepEqual(encoded.body, before, "same prepaid request is retained independently of provider output");
  }
});

test("canonical u64 boundaries reject lossy numbers, signed/leading-zero strings and overflow", () => {
  const input = { kind: "input" } as const;
  for (const epoch of ["0", "1", "9007199254741007", "18446744073709551615"]) {
    const { body } = encodeVoiceIpcAction({ ...vectors.purpose, epoch }, input);
    assert.equal(new DataView(body.buffer).getBigUint64(97), BigInt(epoch));
  }
  for (const bad of ["", "01", "+1", "-1", "1.0", "1e3", "18446744073709551616", "1".repeat(1000), 1, NaN, null]) {
    assert.throws(() => encodeVoiceIpcAction({ ...vectors.purpose, epoch: bad as string }, input));
    for (const field of ["action", "version"] as const) assert.throws(() => encodeVoiceIpcAction(vectors.purpose, {
      kind: "settle", identity: { ...vectors.identity, [field]: bad },
    } as VoiceIpcActionRequest));
  }
  for (const field of ["action", "version"] as const) assert.throws(() => encodeVoiceIpcAction(vectors.purpose, {
    kind: "settle", identity: { ...vectors.identity, [field]: "0" },
  }));
});

test("fixed hashes, flags, engine sizes and result sizes are checked without mutating input", () => {
  for (const field of ["peer", "join", "keyRef"] as const) for (const bad of ["", "a".repeat(63), "A".repeat(64), "g".repeat(64), "0".repeat(10000)]) {
    assert.throws(() => encodeVoiceIpcAction({ ...vectors.purpose, [field]: bad }, { kind: "input" }));
  }
  for (const retry of [0, 1, "true", null]) assert.throws(() => encodeVoiceIpcAction(vectors.purpose, { kind: "begin", retry } as VoiceIpcActionRequest));
  for (const engineBytes of [-1, 1.5, NaN, Infinity, 0x100000000]) assert.throws(() => encodeVoiceIpcAction(vectors.purpose, { kind: "admit", identity: vectors.identity, engineBytes }));
  const output = new Uint8Array(529).fill(91), tag = "55".repeat(32);
  assert.throws(() => encodeVoiceIpcAction(vectors.purpose, { kind: "result", identity: vectors.identity, tag, output }));
  assert.ok(output.every(byte => byte === 91));
  assert.throws(() => encodeVoiceIpcAction(vectors.purpose, { kind: "result", identity: vectors.identity, tag: "A".repeat(64), output: output.subarray(0, 528) }));
  assert.ok(output.every(byte => byte === 91));
});
