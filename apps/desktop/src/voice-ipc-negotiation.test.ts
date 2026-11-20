import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { decodeVoiceAdmissionMode, decodeVoiceNegotiationCapacity,
  encodeVoiceAdmissionMode, encodeVoiceNegotiationCapacity,
  type VoiceNegotiationKind } from "./voice-ipc-negotiation.ts";
import { encodeVoiceIpcHeader, type VoiceIpcHeader } from "./voice-ipc-frame.ts";

const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/negotiation-capacity-vectors.json", import.meta.url), "utf8"));
const bytes = (name: string): Buffer => Buffer.from(vectors.replies.find((reply: { state: string }) => reply.state === name).body, "hex");
const generation = "9007199254740993";

test("mode and capacity use distinct native vectors without narrowing ordinals", () => {
  for (const vector of vectors.modeRequests) {
    const encoded = encodeVoiceAdmissionMode(vectors.session, vector.kind);
    assert.equal(encoded.operation, 29); assert.equal(encoded.logical, vectors.session);
    assert.equal(Buffer.from(encoded.body).toString("hex"), vector.body);
  }
  for (const vector of vectors.modeReplies) assert.equal(decodeVoiceAdmissionMode(Buffer.from(vector.body, "hex")).state, vector.state);
  for (const vector of vectors.requests) {
    const encoded = encodeVoiceNegotiationCapacity(vectors.session, vector.kind, vector.generation);
    assert.equal(encoded.operation, 30); assert.equal(encoded.kind, 0); assert.equal(encoded.logical, vectors.session);
    assert.equal(Buffer.from(encoded.body).toString("hex"), vector.body);
  }
  for (const vector of vectors.replies) {
    assert.deepEqual(decodeVoiceNegotiationCapacity(Buffer.from(vector.body, "hex"), { generation: vector.generation }), {
      state: vector.state, generation: vector.generation, canonical: vector.canonical,
      floor: vector.floor, incarnation: vector.incarnation,
    });
  }
});

test("capacity snapshots reject versions, truncation, trailers, changed request and impossible states", () => {
  const good = bytes("reserved");
  for (let end = 0; end < good.length; end++) assert.throws(() => decodeVoiceNegotiationCapacity(good.subarray(0, end), { generation }));
  assert.throws(() => decodeVoiceNegotiationCapacity(Buffer.concat([good, Buffer.of(0)]), { generation }));
  for (const [offset, value] of [[0, 10], [1, 9], [1, 255], [26, 2]]) {
    const changed = Buffer.from(good); changed[offset] = value;
    assert.throws(() => decodeVoiceNegotiationCapacity(changed, { generation }));
  }
  assert.throws(() => decodeVoiceNegotiationCapacity(good, { generation: "9007199254740994" }));
  for (const field of [10, 18]) {
    const changed = Buffer.from(good); changed.writeBigUInt64BE(9007199254740994n, field);
    assert.throws(() => decodeVoiceNegotiationCapacity(changed, { generation }));
  }
  for (const state of [5, 6, 8]) { // neither Returned, Closed nor Superseded can reuse this positive shape
    const changed = Buffer.from(good); changed[1] = state;
    assert.throws(() => decodeVoiceNegotiationCapacity(changed, { generation }));
  }
  const absent = Buffer.from(good); absent[26] = 0;
  assert.throws(() => decodeVoiceNegotiationCapacity(absent, { generation }), "Reserved requires explicit incarnation even if its bytes are zero");
  const stale = bytes("superseded"); stale.writeBigUInt64BE(BigInt(generation) - 1n, 18);
  assert.throws(() => decodeVoiceNegotiationCapacity(stale, { generation }));
});

test("negative replies may omit destroyed incarnation but never replace a remembered identity", () => {
  const remembered = "44".repeat(32);
  for (const state of ["closing", "returned", "closed", "uncertain", "superseded"]) {
    const reply = bytes(state); reply.fill(0, 26);
    assert.equal(decodeVoiceNegotiationCapacity(reply, { generation, incarnation: remembered }).incarnation, null);
    const changed = Buffer.from(reply); changed[26] = 1; changed.fill(0x55, 27);
    assert.throws(() => decodeVoiceNegotiationCapacity(changed, { generation, incarnation: remembered }));
  }
  for (const state of ["unstarted", "preparing"]) {
    assert.throws(() => decodeVoiceNegotiationCapacity(bytes(state), { generation: "1", incarnation: remembered }));
  }
  const waiting = bytes("waiting"); waiting.fill(0, 26);
  assert.throws(() => decodeVoiceNegotiationCapacity(waiting, { generation, incarnation: remembered }));
  const hidden = bytes("closed"); hidden[58] = 1;
  assert.throws(() => decodeVoiceNegotiationCapacity(hidden, { generation }));
});

test("mode decoding rejects impossible floor/selection and capacity versions", () => {
  const pinned = Buffer.from(vectors.modeReplies[1].body, "hex");
  for (const [offset, value] of [[0, 11], [1, 4], [9, 0], [17, 1]]) {
    const changed = Buffer.from(pinned); changed[offset] = value;
    assert.throws(() => decodeVoiceAdmissionMode(changed));
  }
  for (let end = 0; end < pinned.length; end++) assert.throws(() => decodeVoiceAdmissionMode(pinned.subarray(0, end)));
  assert.throws(() => decodeVoiceAdmissionMode(Buffer.concat([pinned, Buffer.of(0)])));
});

test("capacity requests reject noncanonical ordinals/session and unknown mode without numeric coercion", () => {
  for (const ordinal of ["0", "01", "-1", "+1", "1.0", " 1", "18446744073709551616", 1]) {
    assert.throws(() => encodeVoiceNegotiationCapacity(vectors.session, "register", ordinal as string));
  }
  for (const physical of ["", "ab", vectors.session.toUpperCase(), "f".repeat(33)]) {
    assert.throws(() => encodeVoiceNegotiationCapacity(physical, "cancel", "1"));
  }
  assert.throws(() => encodeVoiceNegotiationCapacity(vectors.session, "ready" as VoiceNegotiationKind, "1"));
  const max = encodeVoiceNegotiationCapacity(vectors.session, "cancel", "18446744073709551615");
  assert.equal(new DataView(max.body.buffer).getBigUint64(2), (1n << 64n) - 1n);
});

test("receipt copies primitives without retaining caller objects or the input backing", () => {
  const data = bytes("waiting"), expected = { generation, incarnation: "44".repeat(32), extra: new Uint8Array(8192) };
  const receipt = decodeVoiceNegotiationCapacity(data, expected);
  data.fill(0); expected.incarnation = "55".repeat(32);
  assert.equal(receipt.incarnation, "44".repeat(32));
  assert.equal(Object.hasOwn(receipt, "extra"), false);
  assert.ok(Object.isFrozen(receipt));
});

test("caller getters cannot replace a remembered identity between validation and comparison", () => {
  let generations = 0, incarnations = 0;
  const expected = {
    get generation() { generations++; return generation; },
    get incarnation() { return ++incarnations === 1 ? "55".repeat(32) : "44".repeat(32); },
  };
  assert.throws(() => decodeVoiceNegotiationCapacity(bytes("waiting"), expected));
  assert.equal(generations, 1); assert.equal(incarnations, 1);
});

test("shadowed typed-array views and subarray cannot replace the authenticated fixed body", () => {
  const body = new Uint8Array(bytes("waiting"));
  let substituted = 0;
  Object.defineProperty(body, "subarray", { value: () => { substituted++; return new Uint8Array(1024 * 1024); } });
  for (const key of ["buffer", "byteOffset", "byteLength", "length"]) {
    Object.defineProperty(body, key, { get() { throw new Error("shadowed metadata used"); } });
  }
  const reply = decodeVoiceNegotiationCapacity(body, { generation });
  assert.equal(reply.incarnation, "44".repeat(32));
  assert.equal(reply.incarnation?.length, 64); assert.equal(substituted, 0);
  for (const length of [58, 60, 1024 * 1024]) {
    const wrong = new Uint8Array(length); wrong[0] = 11;
    Object.defineProperty(wrong, "length", { value: 59 });
    Object.defineProperty(wrong, "byteLength", { value: 59 });
    assert.throws(() => decodeVoiceNegotiationCapacity(wrong, { generation }));
  }
});

test("new Session controls stay Control-only in both header directions with unchanged class bounds", () => {
  for (const operation of [29, 30] as const) for (const direction of [0, 1] as const) {
    const header: VoiceIpcHeader = { operation, direction, kind: 0, slot: 1, sequence: 1n, logical: vectors.session, length: 272 };
    assert.equal(encodeVoiceIpcHeader(header).length, 40);
    for (const kind of [1, 2] as const) assert.throws(() => encodeVoiceIpcHeader({ ...header, kind }));
    assert.throws(() => encodeVoiceIpcHeader({ ...header, length: 273 }));
  }
  assert.throws(() => encodeVoiceIpcHeader({ operation: 31, direction: 0, kind: 0, slot: 1, sequence: 1n, logical: vectors.session, length: 26 } as VoiceIpcHeader));
});
