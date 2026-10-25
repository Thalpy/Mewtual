import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoiceSdpReply, type VoiceSdpReplyExpectation } from "./voice-ipc-sdp-replies.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/sdp-history-vectors.json", import.meta.url), "utf8"));
const expectation = (purpose: "seal" | "open" = "open", close: boolean | null = null): VoiceSdpReplyExpectation =>
  ({ parentRow: vectors.parent, row: null, purpose, resolveClose: close });
const body = (hex: string) => new Uint8Array(Buffer.from(hex, "hex"));

test("native v8 controls distinguish absent purpose, recorded closure and every closed refusal", () => {
  for (const item of vectors.controls) {
    const value = decodeVoiceSdpReply(body(item.body), expectation(item.purpose, item.close ?? null));
    assert.equal(value.kind, item.kind);
    if (value.kind === "missing") {
      assert.equal(value.parentRow, vectors.parent); assert.equal(value.purpose, item.purpose);
      assert.equal(value.closeRecorded, item.close); assert.equal("localDrained" in value, false);
    } else if (value.kind === "refused") assert.equal(value.code, item.code);
    assert.ok(Object.isFrozen(value));
  }
  for (const item of vectors.cases) {
    const value = decodeVoiceSdpReply(body(item.body), expectation(item.purpose));
    assert.equal(value.kind, "history");
  }
});
test("missing binds the requested Resolve, original parent, purpose and close intent", () => {
  const wire = body(vectors.controls.find((v: {purpose: string; close: boolean}) => v.purpose === "open" && v.close).body);
  for (const wanted of [expectation("open"), expectation("seal", true), expectation("open", false),
    { ...expectation("open", true), parentRow: vectors.row }, { ...expectation("open", true), row: vectors.row }]) {
    assert.throws(() => decodeVoiceSdpReply(wire, wanted));
  }
  const value = decodeVoiceSdpReply(wire, expectation("open", true));wire.fill(0);
  assert.equal(value.kind, "missing");
});
test("v8 control grammar refuses all truncations, trailing bytes, legacy and invalid flags", () => {
  for (const item of vectors.controls) {
    const wire = body(item.body), wanted = expectation(item.purpose, item.close ?? null);
    for (let i = 0; i < wire.length; i++) assert.throws(() => decodeVoiceSdpReply(wire.subarray(0, i), wanted));
    assert.throws(() => decodeVoiceSdpReply(new Uint8Array([...wire, 0]), wanted));
    const legacy = wire.slice();legacy[0] = 1;assert.throws(() => decodeVoiceSdpReply(legacy, wanted));
  }
  for (const wire of [[8,2,0,1],[8,2,1,1],[8,2,2,12],[8,2,3,0],[8,3,0,0]]) {
    assert.throws(() => decodeVoiceSdpReply(new Uint8Array(wire), expectation()));
  }
  for (const offset of [18,19]) {
    const wire = body(vectors.controls[0].body);wire[offset] = 2;
    assert.throws(() => decodeVoiceSdpReply(wire, expectation("seal", false)));
  }
});
test("expectation getters cannot replace validated scalars or retain a caller graph", () => {
  const counts = [0,0,0,0], graph = { data: new Uint8Array(10000) };
  const wanted = {
    get parentRow() { return ++counts[0] === 1 ? vectors.parent : graph; },
    get row() { return ++counts[1] === 1 ? null : graph; },
    get purpose() { return ++counts[2] === 1 ? "seal" : graph; },
    get resolveClose() { return ++counts[3] === 1 ? false : graph; },
  };
  const value = decodeVoiceSdpReply(body(vectors.controls[0].body), wanted as unknown as VoiceSdpReplyExpectation);
  assert.deepEqual(counts, [1,1,1,1]);assert.equal(Object.values(value).includes(graph), false);
  assert.throws(() => decodeVoiceSdpReply(new Uint8Array([8,2,0,0]), { ...expectation(), resolveClose: undefined } as unknown as VoiceSdpReplyExpectation));
});
