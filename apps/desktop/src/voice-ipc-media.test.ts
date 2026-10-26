import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { encodeVoiceMediaOriginal, encodeVoiceMediaExisting, decodeVoiceMedia, type VoiceMediaOriginalKind, type VoiceMediaExistingKind } from "./voice-ipc-media.ts";
import { encodeVoiceIpcHeader, type VoiceIpcBodyKind, type VoiceIpcOperation } from "./voice-ipc-frame.ts";
import type { VoiceSdpOriginal } from "./voice-ipc-sdp-actions.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/media-vectors.json", import.meta.url), "utf8")) as {
  original: VoiceSdpOriginal; mediaProof: string; requests: { name: string; operation: number; kind: number; logical: string; body: string }[];
};
test("native/browser media vectors preserve all u64s and all eight closed modes", () => {
  for (const vector of vectors.requests) {
    const [family, mode] = vector.name.split("-");
    const encoded = family === "original" ? encodeVoiceMediaOriginal(vectors.original, mode as VoiceMediaOriginalKind) :
      encodeVoiceMediaExisting(vectors.original.parentRow, vectors.original.identity, vectors.mediaProof, mode as VoiceMediaExistingKind);
    assert.equal(encoded.operation, vector.operation); assert.equal(encoded.kind, vector.kind); assert.equal(encoded.logical, vector.logical);
    assert.equal(Buffer.from(encoded.body).toString("hex"), vector.body);
  }
});
test("media stays Control-only in both directions without enlarging either body cap", () => {
  for (const direction of [0, 1] as const) {
    const header = { direction, operation: 28 as const, kind: 0 as VoiceIpcBodyKind, slot: 1, sequence: 1n, logical: vectors.original.parentRow, length: 272 };
    assert.equal(encodeVoiceIpcHeader(header)[6], 28);
    assert.throws(() => encodeVoiceIpcHeader({ ...header, length: 273 }));
    for (const kind of [1, 2] as VoiceIpcBodyKind[]) assert.throws(() => encodeVoiceIpcHeader({ ...header, kind }));
    assert.throws(() => encodeVoiceIpcHeader({ ...header, operation: 29 as VoiceIpcOperation }));
  }
});
function reply(flags = 9, remaining = 123): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(39); bytes.set([9, 0, flags]); bytes.fill(23, 3, 35);
  new DataView(bytes.buffer).setUint32(35, remaining); return bytes;
}
test("historical admission and current native observation remain distinct", () => {
  assert.equal(decodeVoiceMedia(Uint8Array.of(9, 1)), null);
  const historical = decodeVoiceMedia(reply(1, 0xffffffff))!;
  assert.equal(historical.admitted, true); assert.equal(historical.current, false); assert.equal(historical.retireInMs, null);
  for (const remaining of [0, 123, 30_000]) {
    const current = decodeVoiceMedia(reply(9, remaining))!;
    assert.equal(current.current, true); assert.equal(current.retireInMs, remaining); assert.ok(Object.isFrozen(current));
  }
  assert.equal(decodeVoiceMedia(reply(7, 0xffffffff))!.released, true);
});
test("media replies reject truncation, trailing data, unknown flags and contradictory release", () => {
  const bytes = reply();
  for (let end = 0; end < bytes.length; end++) assert.throws(() => decodeVoiceMedia(bytes.subarray(0, end)));
  assert.throws(() => decodeVoiceMedia(Uint8Array.from([...bytes, 0])));
  for (const flags of [4, 5, 8, 10, 11, 12, 13, 14, 15, 16, 255]) assert.throws(() => decodeVoiceMedia(reply(flags)));
  for (const remaining of [30_001, 0xfffffffe, 0xffffffff]) assert.throws(() => decodeVoiceMedia(reply(9, remaining)));
  for (const bytes of [Uint8Array.of(8, 1), Uint8Array.of(9, 0), Uint8Array.of(9, 1, 0), Uint8Array.of(9, 255, 0, 0)]) assert.throws(() => decodeVoiceMedia(bytes));
});
test("media request validation rejects invented authority and noncanonical identities", () => {
  for (const sdkGeneration of ["0", "01", "-1", "18446744073709551616"]) assert.throws(() => encodeVoiceMediaOriginal({ ...vectors.original, sdkGeneration }, "admit"));
  for (const pcProof of ["", "ab", "AB".repeat(32)]) assert.throws(() => encodeVoiceMediaOriginal({ ...vectors.original, pcProof }, "admit"));
  assert.throws(() => encodeVoiceMediaOriginal(vectors.original, "renew" as VoiceMediaOriginalKind));
  assert.throws(() => encodeVoiceMediaExisting(vectors.original.parentRow, vectors.original.identity, vectors.mediaProof, "admit" as VoiceMediaExistingKind));
  for (const parentRow of ["00".repeat(16), "00000000000000200000000000000001", "01"]) assert.throws(() => encodeVoiceMediaExisting(parentRow, vectors.original.identity, vectors.mediaProof, "close"));
});
