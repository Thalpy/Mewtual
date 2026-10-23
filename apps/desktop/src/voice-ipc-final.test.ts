import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoiceIpcFinalReply, decodeVoiceIpcActionReply, type VoiceIpcFinalReply } from "./voice-ipc-replies.ts";
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";

const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/replies/vectors.json", import.meta.url), "utf8")) as {
  finalCases: { name: string; expected: Extract<VoiceIpcFinalReply, { kind: "final" }>; plaintext: string }[];
  refusedCases: { expected: VoiceIpcFinalReply; plaintext: string }[];
};
const bytes = (text: string) => Uint8Array.from(Buffer.from(text, "hex"));
const issued = vectors.finalCases[1], acked = vectors.finalCases[2];
for (const item of vectors.finalCases) test(`native/browser final receipt: ${item.name}`, () => {
  const source = bytes(item.plaintext), before = source.slice();
  const decoded = decodeVoiceIpcFinalReply(source, item.expected.row, item.expected.identity);
  assert.deepEqual(decoded, item.expected);
  assert.deepEqual(source, before);
  assert.ok(Object.isFrozen(decoded));
  if (decoded.kind === "final") assert.ok(Object.isFrozen(decoded.identity));
  assert.throws(() => decodeVoiceIpcActionReply(source, item.expected.row), "final receipt cannot masquerade as historical engine metadata");
});
test("ACK response must bind the originally issued proof and explicit acknowledged disposition", () => {
  const expected = acked.expected;
  assert.deepEqual(decodeVoiceIpcFinalReply(bytes(acked.plaintext), expected.row, expected.identity, expected.proof), expected);
  for (const item of vectors.finalCases.slice(0, 2)) assert.throws(() => decodeVoiceIpcFinalReply(bytes(item.plaintext), expected.row, expected.identity, expected.proof));
  assert.throws(() => decodeVoiceIpcFinalReply(bytes(acked.plaintext), expected.row, expected.identity, "66".repeat(32)));
});
test("final metadata rejects every truncation, trailing byte, alternate tag, mode and Pending proof", () => {
  const source = bytes(issued.plaintext), expected = issued.expected;
  for (let length = 0; length < source.length; length++) assert.throws(() => decodeVoiceIpcFinalReply(source.subarray(0, length), expected.row, expected.identity));
  assert.throws(() => decodeVoiceIpcFinalReply(new Uint8Array([...source, 0]), expected.row, expected.identity));
  for (const [offset, value] of [[0, 0], [0, 2], [1, 0], [1, 5], [66, 0], [66, 2], [67, 3], [67, 255]]) {
    const bad = source.slice(); bad[offset] = value;
    assert.throws(() => decodeVoiceIpcFinalReply(bad, expected.row, expected.identity));
  }
  for (let offset = 68; offset < 100; offset++) {
    const pending = bytes(vectors.finalCases[0].plaintext); pending[offset] = 1;
    assert.throws(() => decodeVoiceIpcFinalReply(pending, expected.row, expected.identity));
  }
});
test("a final receipt cannot alias another row, action, version or input and preserves full u64s", () => {
  const source = bytes(issued.plaintext), expected = issued.expected;
  assert.equal(expected.identity.version, "18446744073709551615");
  assert.equal(expected.identity.action, "9007199254741007");
  for (let offset = 2; offset < 66; offset++) {
    const bad = source.slice(); bad[offset] ^= 1;
    assert.throws(() => decodeVoiceIpcFinalReply(bad, expected.row, expected.identity));
  }
  for (const bad of ["0", "01", "+1", "-1", "18446744073709551616", 1, null, undefined]) {
    for (const field of ["action", "version"] as const) assert.throws(() => decodeVoiceIpcFinalReply(source, expected.row,
      { ...expected.identity, [field]: bad } as VoiceIpcActionIdentity));
  }
  for (const inputTag of [undefined, null, {}, "A".repeat(64), "44".repeat(33)]) assert.throws(() => decodeVoiceIpcFinalReply(source, expected.row,
    { ...expected.identity, inputTag } as VoiceIpcActionIdentity));
});
test("canonical refusal is never interpreted as final completion or an ACK", () => {
  for (const item of vectors.refusedCases) assert.deepEqual(decodeVoiceIpcFinalReply(bytes(item.plaintext), issued.expected.row, issued.expected.identity), item.expected);
});
