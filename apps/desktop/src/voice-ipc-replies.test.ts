import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoiceIpcActionReply, type VoiceIpcActionReply } from "./voice-ipc-replies.ts";

const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/replies/vectors.json", import.meta.url), "utf8")) as {
  statusCases: { name: string; expected: Extract<VoiceIpcActionReply, { kind: "status" }>; plaintext: string }[];
  refusedCases: { name: string; expected: VoiceIpcActionReply; plaintext: string }[];
};
const bytes = (hex: string) => Uint8Array.from(Buffer.from(hex, "hex"));
for (const item of [...vectors.statusCases, ...vectors.refusedCases]) test(`native/browser history reply: ${item.name}`, () => {
  const source = bytes(item.plaintext), before = source.slice();
  const decoded = decodeVoiceIpcActionReply(source, null);
  assert.deepEqual(decoded, item.expected);
  assert.deepEqual(source, before);
  assert.ok(Object.isFrozen(decoded));
  if (decoded.kind === "status") {
    assert.deepEqual(decodeVoiceIpcActionReply(source, decoded.row), decoded);
    if (decoded.identity) assert.ok(Object.isFrozen(decoded.identity));
    if (decoded.receipt) { assert.ok(Object.isFrozen(decoded.receipt)); assert.ok(Object.isFrozen(decoded.receipt.outcome)); }
  }
});
const sample = (name: string) => {
  const item = vectors.statusCases.find(item => item.name === name)!;
  return { ...item, bytes: bytes(item.plaintext) };
};

test("history replies preserve exact u64s and cannot alias another original native row", () => {
  const item = sample("derived-drained");
  const decoded = decodeVoiceIpcActionReply(item.bytes, item.expected.row);
  assert.equal(decoded.kind, "status");
  if (decoded.kind !== "status") throw new Error("status fixture");
  assert.equal(decoded.identity!.action, "9007199254741007");
  assert.equal(decoded.identity!.version, "18446744073709551615");
  for (const row of ["0".repeat(32), "0".repeat(31) + "1", "f".repeat(32), item.expected.row.toUpperCase(), "", undefined]) {
    assert.throws(() => decodeVoiceIpcActionReply(item.bytes, row as string));
  }
  for (const [start, length] of [[2, 16], [10, 8], [20, 8], [28, 8]]) {
    const corrupt = item.bytes.slice(); corrupt.fill(0, start, start + length);
    assert.throws(() => decodeVoiceIpcActionReply(corrupt, null));
  }
  const outside = item.bytes.slice(); new DataView(outside.buffer).setBigUint64(2, 32n);
  assert.throws(() => decodeVoiceIpcActionReply(outside, null));
});

test("history grammar rejects every truncation, trailing byte and noncanonical tag or flag", () => {
  const item = sample("derived-drained");
  for (let length = 0; length < item.bytes.length; length++) assert.throws(() => decodeVoiceIpcActionReply(item.bytes.subarray(0, length), null));
  assert.throws(() => decodeVoiceIpcActionReply(new Uint8Array([...item.bytes, 0]), null));
  for (const [offset, value] of [[0, 2], [1, 4], [18, 5], [19, 2], [68, 2], [69, 2], [142, 4], [207, 2], [208, 2], [209, 2]]) {
    const corrupt = item.bytes.slice(); corrupt[offset] = value;
    assert.throws(() => decodeVoiceIpcActionReply(corrupt, null), `offset ${offset}`);
  }
  for (const wire of [[1, 0, 0], [1, 1, 1], [1, 2, 1], [1, 3, 12], [1, 4, 0], [1, 1, 0, 0]]) {
    assert.throws(() => decodeVoiceIpcActionReply(Uint8Array.from(wire), null));
  }
});

test("absent fields and closed outcome padding have one canonical zero representation", () => {
  const cases: [string, number, number][] = [
    ["open", 20, 68], ["open", 70, 208], ["open", 210, 242],
    ["derived-drained", 143, 207], ["opened", 143, 207], ["rejected-malformed", 144, 207],
  ];
  for (const [name, start, end] of cases) for (let offset = start; offset < end; offset++) {
    const corrupt = sample(name).bytes; corrupt[offset] = 1;
    assert.throws(() => decodeVoiceIpcActionReply(corrupt, null), `${name} padding ${offset}`);
  }
  const rejected = sample("rejected-malformed").bytes; rejected[143] = 12;
  assert.throws(() => decodeVoiceIpcActionReply(rejected, null));
});

test("mismatched receipts or contradictory lifecycle flags never become successful history", () => {
  const item = sample("derived-drained");
  for (const [offset, value] of [[18, 1], [19, 0], [68, 0], [69, 0], [70, 1], [78, 9], [207, 0], [208, 0]]) {
    const corrupt = item.bytes.slice(); corrupt[offset] = value;
    assert.throws(() => decodeVoiceIpcActionReply(corrupt, null), `contradiction ${offset}`);
  }
  const noReceipt = sample("settled-no-receipt").bytes; noReceipt[208] = 1;
  assert.throws(() => decodeVoiceIpcActionReply(noReceipt, null));
  const noAction = sample("open").bytes; noAction[18] = 0;
  assert.throws(() => decodeVoiceIpcActionReply(noAction, null));
  for (const offset of [68, 208, 209]) {
    const pending = sample("closed-handed").bytes; pending[offset] = 1;
    assert.throws(() => decodeVoiceIpcActionReply(pending, null));
  }
  const retired = sample("derived-retired").expected.receipt!, settled = sample("derived-settled").expected.receipt!;
  assert.equal(retired.digest, settled.digest, "observational retirement never changes immutable ACK identity");
});

test("bounded decoder respects the exact view rather than another buffer prefix or suffix", () => {
  const item = sample("derived-drained"), backing = new Uint8Array(258).fill(99);
  backing.set(item.bytes, 7);
  assert.deepEqual(decodeVoiceIpcActionReply(backing.subarray(7, 249), null), item.expected);
  assert.throws(() => decodeVoiceIpcActionReply(backing, null));
});
