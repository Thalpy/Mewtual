import { test } from "node:test";
import assert from "node:assert/strict";
import { decodeVoiceSdpInput, VOICE_SDP_AAD_MAX, VOICE_SDP_INPUT_PLAIN_MAX, type VoiceSdpInputExpectation } from "./voice-ipc-sdp-input.ts";

const row = (slot: number) => slot.toString(16).padStart(16, "0") + "ffffffffffffffff";
function expected(purpose: "seal" | "open" = "seal", localIsLow = true): VoiceSdpInputExpectation {
  return { row: row(1), parentRow: row(0), identity: { action: "18446744073709551615", version: "18446744073709551615", inputTag: "04".repeat(32) },
    purpose, generation: "1", ready: "05".repeat(32), key: "06".repeat(32), sdkGeneration: "1", sdkProof: "07".repeat(32),
    storage: "08".repeat(32), pcProof: "09".repeat(32), localIsLow };
}
/** Independent explicit native descriptor layout, matching its full-u64 Rust boundary
 * case. Payload sizes and fields are varied below rather than using a decoder roundtrip. */
function encoded(e = expected(), bodyBytes = e.purpose === "seal" ? 8192 : 8208, aadBytes = VOICE_SDP_AAD_MAX) {
  const bytes = new Uint8Array(277 + aadBytes + bodyBytes), view = new DataView(bytes.buffer);
  const put = (offset: number, value: string) => { for (let i = 0; i < value.length / 2; i++) bytes[offset + i] = Number.parseInt(value.slice(2 * i, 2 * i + 2), 16); };
  bytes.set([8, 3]); put(2, e.row); put(18, e.parentRow);
  view.setBigUint64(34, BigInt(e.identity.action)); view.setBigUint64(42, BigInt(e.identity.version)); put(50, e.identity.inputTag);
  view.setBigUint64(82, 1n); put(90, e.ready); bytes[122] = 1; put(123, e.key); view.setBigUint64(155, 1n);
  put(163, e.sdkProof); put(195, e.storage); put(227, e.pcProof); view.setBigUint64(259, (1n << 64n) - 1n);
  view.setUint32(267, 4 * (aadBytes + bodyBytes + (e.purpose === "seal" ? 16 : 0)) + 3 * 8192 + 2048);
  bytes[271] = Number(e.purpose === "open"); bytes[272] = Number(e.purpose === "seal" ? !e.localIsLow : e.localIsLow);
  view.setUint16(273, aadBytes); view.setUint16(275, bodyBytes); bytes.fill(10, 277, 277 + aadBytes); bytes.fill(11, 277 + aadBytes);
  return bytes;
}

test("native SDP descriptor preserves maximum bytes, original bindings and full-u64 serials without copying payload", () => {
  for (const purpose of ["seal", "open"] as const) for (const low of [true, false]) {
    const e = expected(purpose, low), bytes = encoded(e), decoded = decodeVoiceSdpInput(bytes, e);
    assert.equal(decoded.sequence, "18446744073709551615");
    assert.deepEqual(decoded.identity, e.identity); assert.notEqual(decoded.identity, e.identity);
    assert.equal(decoded.kind, purpose === "seal" ? (low ? "offer" : "answer") : (low ? "answer" : "offer"));
    assert.equal(decoded.aad.buffer, bytes.buffer); assert.equal(decoded.body.buffer, bytes.buffer);
    assert.equal(decoded.aad.byteOffset, 277); assert.equal(decoded.aad.byteLength, 579);
    assert.equal(decoded.body.byteLength, purpose === "seal" ? 8192 : 8208);
    assert.equal(Object.isFrozen(decoded), true); assert.equal(Object.isFrozen(decoded.identity), true);
    // Only the enclosing exclusive owner may clear: the decoder has no hidden copy that
    // could continue using a retired buffer or require a second plaintext reservation.
    bytes.fill(0); assert.equal(decoded.body.every(byte => byte === 0), true);
  }
  assert.equal(VOICE_SDP_INPUT_PLAIN_MAX, 9064);
});

test("native minimum Seal and tag-only Open remain distinct from authentication failure", () => {
  for (const purpose of ["seal", "open"] as const) {
    const e = expected(purpose), bytes = encoded(e, purpose === "seal" ? 1 : 16, 0);
    const decoded = decodeVoiceSdpInput(bytes, e);
    assert.equal(decoded.aad.byteLength, 0); assert.equal(decoded.body.byteLength, purpose === "seal" ? 1 : 16);
  }
});

test("every fixed original row, Action, Ready, key, SDK, storage and PC binding is checked", () => {
  const e = expected(), bytes = encoded(e);
  // Includes every byte of every fixed binding; also pins closed version/profile flags.
  for (let offset = 0; offset < 259; offset++) {
    const changed = bytes.slice(); changed[offset] ^= 1;
    assert.throws(() => decodeVoiceSdpInput(changed, e), /invalid native voice SDP input/, `offset ${offset}`);
  }
});

test("input rejects purpose/role mismatches, zero sequence and noncanonical engine charge", () => {
  for (const purpose of ["seal", "open"] as const) {
    const e = expected(purpose), bytes = encoded(e);
    for (const offset of [267, 268, 269, 270, 271, 272]) {
      const changed = bytes.slice(); changed[offset] ^= 1; assert.throws(() => decodeVoiceSdpInput(changed, e));
    }
    const zero = bytes.slice(); zero.fill(0, 259, 267); assert.throws(() => decodeVoiceSdpInput(zero, e));
    assert.throws(() => decodeVoiceSdpInput(bytes, { ...e, localIsLow: false }));
    assert.throws(() => decodeVoiceSdpInput(bytes, { ...e, purpose: purpose === "seal" ? "open" : "seal" }));
  }
});

test("oversized, short, mismatched and trailing payloads fail without normalizing AAD or allocating a copy", () => {
  for (const purpose of ["seal", "open"] as const) {
    const e = expected(purpose), max = purpose === "seal" ? 8192 : 8208;
    for (const bodyBytes of [0, purpose === "seal" ? 0 : 15, max + 1]) assert.throws(() => decodeVoiceSdpInput(encoded(e, bodyBytes, 0), e));
    assert.throws(() => decodeVoiceSdpInput(encoded(e, 17, 580), e));
    const bytes = encoded(e);
    assert.throws(() => decodeVoiceSdpInput(bytes.slice(0, -1), e));
    const extra = new Uint8Array(bytes.length + 1); extra.set(bytes); assert.throws(() => decodeVoiceSdpInput(extra, e));
    for (const offset of [273, 274, 275, 276]) {
      const changed = bytes.slice(); changed[offset] ^= 1; assert.throws(() => decodeVoiceSdpInput(changed, e));
    }
    const exact = encoded(e, 17, 1); exact[277] = 255;
    assert.equal(decodeVoiceSdpInput(exact, e).aad[0], 255, "native authenticated AAD is verbatim");
  }
});

test("a small view cannot pin an oversized backing and shared, resizable or detached backing is refused", () => {
  const e = expected(), bytes = encoded(e), larger = new Uint8Array(bytes.length + 1); larger.set(bytes);
  assert.throws(() => decodeVoiceSdpInput(larger.subarray(0, bytes.length), e));
  const shifted = new Uint8Array(bytes.length + 1); shifted.set(bytes, 1);
  assert.throws(() => decodeVoiceSdpInput(shifted.subarray(1), e));
  const shared = new Uint8Array(new SharedArrayBuffer(bytes.length)); shared.set(bytes);
  assert.throws(() => decodeVoiceSdpInput(shared as Uint8Array<ArrayBuffer>, e));
  const resizable = new Uint8Array(new ArrayBuffer(bytes.length, { maxByteLength: bytes.length + 1 })); resizable.set(bytes);
  assert.throws(() => decodeVoiceSdpInput(resizable, e));
  const detached = bytes.slice(); structuredClone(detached.buffer, { transfer: [detached.buffer] });
  assert.throws(() => decodeVoiceSdpInput(detached, e));
});

test("hostile expected getters are read once and no caller graph survives in decoded metadata", () => {
  const original = expected(), bytes = encoded(original), reads = new Map<PropertyKey, number>();
  const once = <T extends object>(value: T) => new Proxy(value, { get(target, field) {
    const count = (reads.get(field) ?? 0) + 1; reads.set(field, count); assert.equal(count, 1, String(field));
    return Reflect.get(target, field);
  } });
  const hostile = once({ ...original, identity: once({ ...original.identity }), unrelated: new Uint8Array(1024 * 1024) });
  const decoded = decodeVoiceSdpInput(bytes, hostile);
  assert.equal("unrelated" in decoded, false); assert.notEqual(decoded.identity, hostile);
  assert.equal(reads.has("unrelated"), false);
  const mutable = expected(), observed = decodeVoiceSdpInput(encoded(mutable), mutable);
  (mutable.identity as { action: string }).action = "1";
  assert.equal(observed.identity.action, "18446744073709551615");
});

test("malformed expected metadata and future generations cannot widen the first-round input contract", () => {
  const e = expected(), bytes = encoded(e);
  for (const patch of [{ row: e.parentRow }, { row: "ff".repeat(16) }, { generation: "2" }, { sdkGeneration: "2" },
    { ready: "F".repeat(64) }, { key: "0" }, { pcProof: null }, { localIsLow: 1 }, { purpose: "derive" },
    { identity: { ...e.identity, action: "0" } }, { identity: { ...e.identity, version: "18446744073709551616" } },
    { identity: { ...e.identity, inputTag: "a".repeat(65) } }]) {
    assert.throws(() => decodeVoiceSdpInput(bytes, { ...e, ...patch } as VoiceSdpInputExpectation));
  }
});


test("shadowed TypedArray accessors and species methods cannot conceal or substitute actual backing", () => {
  const e = expected(), bytes = encoded(e);
  const large = new Uint8Array(bytes.length + 1024 * 1024); large.set(bytes);
  const hidden = large.subarray(0, bytes.length);
  Object.defineProperty(hidden, "buffer", { value: bytes.buffer });
  Object.defineProperty(hidden, "byteLength", { value: bytes.length });
  assert.throws(() => decodeVoiceSdpInput(hidden, e));
  const resizable = new ArrayBuffer(bytes.length, { maxByteLength: bytes.length + 1 });
  new Uint8Array(resizable).set(bytes); Object.defineProperty(resizable, "resizable", { value: false });
  assert.throws(() => decodeVoiceSdpInput(new Uint8Array(resizable), e));
  const malicious = encoded(e);
  Object.defineProperty(malicious, "buffer", { get() { throw new Error("caller accessor invoked"); } });
  Object.defineProperty(malicious, "byteOffset", { value: 42 });
  Object.defineProperty(malicious, "byteLength", { value: 1 });
  malicious.subarray = () => { throw new Error("caller method invoked"); };
  const actual = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), "buffer")!.get!.call(malicious);
  const decoded = decodeVoiceSdpInput(malicious, e);
  assert.equal(decoded.aad.buffer, actual); assert.equal(decoded.body.buffer, actual);
  assert.equal(decoded.body.length, 8192);
});
