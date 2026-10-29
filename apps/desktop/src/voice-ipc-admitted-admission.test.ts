import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { encodeVoiceAdmittedAdmission, decodeVoiceAdmittedAdmission } from "./voice-ipc-admitted-admission.ts";
import { decodeVoiceNegotiationCapacity } from "./voice-ipc-negotiation.ts";
import { decodeVoiceIpcAdmission, encodeVoiceIpcHeader, type VoiceIpcHeader } from "./voice-ipc-frame.ts";
import { encodeVoiceAdmittedIpcHeader, encodeVoiceAdmittedIpcAdmission, decodeVoiceAdmittedIpcAdmission,
  voiceAdmittedIpcNonce, voiceAdmittedIpcResponseAad, type VoiceAdmittedIpcHeader } from "./voice-ipc-admitted-frame.ts";
const session = "12".repeat(16);
function reply(state: number, wanted = 1n, canonical = 1n, floor = 0n, identity: number | null = null): Uint8Array {
  const bytes = new Uint8Array(59), d = new DataView(bytes.buffer);
  bytes[0] = 12; bytes[1] = state; d.setBigUint64(2, wanted); d.setBigUint64(10, canonical); d.setBigUint64(18, floor);
  if (identity !== null) { bytes[26] = 1; bytes.fill(identity, 27); } return bytes;
}
test("admitted31 shared fixed vectors preserve full u64 and exclusive SetupAck0", () => {
  const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/admitted-admission-vectors.json", import.meta.url), "utf8"));
  for (const vector of vectors.requests) {
    const request = encodeVoiceAdmittedAdmission(session, vector.kind, vector.generation);
    assert.equal(request.operation, 31); assert.equal(request.kind, 0); assert.equal(request.logical, session);
    assert.equal(Buffer.from(request.body).toString("hex"), vector.body);
  }
  for (const kind of ["register", "observe", "cancel"] as const) assert.throws(() => encodeVoiceAdmittedAdmission(session, kind, "0"));
  for (const generation of ["1", "00", "-1", "18446744073709551616"]) assert.throws(() => encodeVoiceAdmittedAdmission(session, "setup-ack", generation));
  for (const generation of ["01", "-1", "", "18446744073709551616"]) assert.throws(() => encodeVoiceAdmittedAdmission(session, "register", generation));
  assert.throws(() => encodeVoiceAdmittedAdmission("0".repeat(32), "register", "1"));
});
test("SetupAck is separately decoded at g0, without v11 or requested-generation inference", () => {
  assert.equal(decodeVoiceAdmittedAdmission(reply(0, 0n, 0n), { kind: "setup-ack", generation: "0" }).canonical, "0");
  assert.equal(decodeVoiceAdmittedAdmission(reply(3, 0n, 1n, 0n, 0), { kind: "setup-ack", generation: "0" }).incarnation, "0".repeat(64));
  assert.throws(() => decodeVoiceNegotiationCapacity(reply(0, 0n, 0n), { generation: "1" }));
  assert.throws(() => decodeVoiceAdmittedAdmission(reply(0, 0n, 0n), { kind: "observe", generation: "0" }));
});
test("presence flag preserves all-zero incarnation and negatives never replace identity", () => {
  const wanted = { kind: "observe" as const, generation: "1", incarnation: "0".repeat(64) };
  assert.equal(decodeVoiceAdmittedAdmission(reply(3, 1n, 1n, 0n, 0), wanted).incarnation, wanted.incarnation);
  assert.throws(() => decodeVoiceAdmittedAdmission(reply(3), wanted));
  assert.throws(() => decodeVoiceAdmittedAdmission(reply(3, 1n, 1n, 0n, 7), wanted));
  assert.equal(decodeVoiceAdmittedAdmission(reply(7), wanted).state, "uncertain");
  assert.equal(decodeVoiceAdmittedAdmission(reply(6), wanted).incarnation, null);
});
test("unknown versions, tags, truncation, trailing data and impossible floors refuse", () => {
  for (const version of [0, 10, 11, 13, 255]) { const bytes = reply(0); bytes[0] = version; assert.throws(() => decodeVoiceAdmittedAdmission(bytes, { kind: "observe", generation: "1" })); }
  for (const bytes of [reply(9), reply(0, 2n), reply(0, 1n, 1n, 2n), new Uint8Array(60)]) assert.throws(() => decodeVoiceAdmittedAdmission(bytes, { kind: "observe", generation: "1" }));
  for (let length = 0; length < 59; length++) assert.throws(() => decodeVoiceAdmittedAdmission(reply(0).subarray(0, length), { kind: "observe", generation: "1" }));
  const absent = reply(0); absent[58] = 1;
  assert.throws(() => decodeVoiceAdmittedAdmission(absent, { kind: "observe", generation: "1" }));
});
test("canonical successor/superseded observations never wrap or forge Returned", () => {
  assert.equal(decodeVoiceAdmittedAdmission(reply(0, 2n, 1n, 1n), { kind: "observe", generation: "2" }).state, "unstarted");
  assert.equal(decodeVoiceAdmittedAdmission(reply(8, 1n, 2n, 1n, 0), { kind: "observe", generation: "1" }).state, "superseded");
  for (const bytes of [reply(3, 2n, 1n, 1n, 0), reply(5, 1n, 1n, 0n), reply(8, 1n, 1n, 1n)]) assert.throws(() => decodeVoiceAdmittedAdmission(bytes, { kind: "observe", generation: bytes[9] === 2 ? "2" : "1" }));
});
test("cancellation is Closing before actual return and lost-next-entry cancellation is noncreating", () => {
  assert.equal(decodeVoiceAdmittedAdmission(reply(4, 1n, 1n, 1n, 0), { kind: "cancel", generation: "1" }).state, "closing");
  assert.throws(() => decodeVoiceAdmittedAdmission(reply(2, 1n, 1n, 1n, 0), { kind: "cancel", generation: "1" }));
  const returned = decodeVoiceAdmittedAdmission(reply(5, 2n, 2n, 2n, 0), { kind: "cancel", generation: "2" });
  assert.equal(returned.state, "returned");
  assert.equal(returned.canonical, "2");
  assert.equal(returned.floor, "2");
});
test("intrinsic fixed typed-array view ignores hostile own properties and species", () => {
  const bytes = reply(3, 1n, 1n, 0n, 0); let reads = 0;
  for (const key of ["length", "byteLength", "byteOffset", "buffer"]) Object.defineProperty(bytes, key, { get() { reads++; throw new Error("shadow"); } });
  Object.defineProperty(bytes, Symbol.iterator, { get() { reads++; throw new Error("iterator"); } });
  assert.equal(decodeVoiceAdmittedAdmission(bytes, { kind: "observe", generation: "1" }).state, "reserved");
  assert.equal(reads, 0);
  assert.throws(() => decodeVoiceAdmittedAdmission(new Proxy(reply(0), {}), { kind: "observe", generation: "1" }));
});
test("separate admitted framing is closed31/Control and old framing remains closed1–30", () => {
  const header: VoiceAdmittedIpcHeader = { direction: 0, operation: 31, kind: 0, slot: 64, sequence: (1n << 64n) - 1n, logical: session, length: 26 };
  const admission = encodeVoiceAdmittedIpcAdmission(header, new Uint8Array(32));
  assert.deepEqual(decodeVoiceAdmittedIpcAdmission(admission).header, header);
  assert.throws(() => encodeVoiceIpcHeader(header as unknown as VoiceIpcHeader));
  assert.throws(() => decodeVoiceIpcAdmission(admission));
  for (const operation of [0, 1, 9, 29, 30, 32]) assert.throws(() => encodeVoiceAdmittedIpcHeader({ ...header, operation } as VoiceAdmittedIpcHeader));
  for (const kind of [1, 2]) assert.throws(() => encodeVoiceAdmittedIpcHeader({ ...header, kind } as VoiceAdmittedIpcHeader));
  for (const length of [16, 25, 27, 75, 272]) assert.throws(() => encodeVoiceAdmittedIpcHeader({ ...header, length }));
  const changed = Buffer.from(admission, "hex"); changed[39] = 1;
  assert.throws(() => decodeVoiceAdmittedIpcAdmission(changed.toString("hex")));
  assert.throws(() => decodeVoiceAdmittedIpcAdmission(admission + "00"));
  const response: VoiceAdmittedIpcHeader = { ...header, direction: 1, sequence: 1n, length: 75 };
  assert.equal(voiceAdmittedIpcNonce(header)[2], 0x51);
  assert.equal(voiceAdmittedIpcNonce(response)[2], 0x50);
  assert.equal(voiceAdmittedIpcResponseAad(response, header).length, 80);
  assert.throws(() => voiceAdmittedIpcResponseAad({ ...response, logical: "13".repeat(16) }, header));
});
