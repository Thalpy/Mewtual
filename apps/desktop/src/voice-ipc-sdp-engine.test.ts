import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoiceSdpEngineReply, type VoiceSdpEngineExpectation } from "./voice-ipc-sdp-engine.ts";
import { decodeVoiceSdpReply } from "./voice-ipc-sdp-replies.ts";
import { decodeVoiceIpcActionReply } from "./voice-ipc-replies.ts";
const vector = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/sdp-engine-vectors.json", import.meta.url), "utf8"));
const expected = (): VoiceSdpEngineExpectation => ({ parentRow: vector.parent, row: vector.row,
  purpose: "open", identity: { ...vector.identity }, engineBytes: vector.engineBytes });
const wire = () => new Uint8Array(Buffer.from(vector.permit, "hex"));

test("native SDP engine vector preserves exact u64s and original charge without legacy fallback", () => {
  const value = decodeVoiceSdpEngineReply(wire(), expected());
  assert.equal(value.kind, "admitted");
  if (value.kind !== "admitted") return;
  assert.equal(value.permit, vector.identity.action); assert.deepEqual(value.identity, vector.identity);
  assert.equal(value.engineBytes, vector.engineBytes); assert.equal(value.row, vector.row);
  assert.throws(() => decodeVoiceIpcActionReply(wire(), vector.row));
  assert.throws(() => decodeVoiceSdpReply(wire(), { ...expected(), resolveClose: null }));
});
test("engine metadata rejects every truncation, extension and mismatched original field", () => {
  const bytes = wire();
  for (let end = 0; end < bytes.length; end++) assert.throws(() => decodeVoiceSdpEngineReply(bytes.subarray(0, end), expected()));
  assert.throws(() => decodeVoiceSdpEngineReply(new Uint8Array([...bytes, 0]), expected()));
  for (const offset of [0, 1, 2, 17, 18, 25, 26, 33, 34, 65, 66, 73, 74, 77]) {
    const changed = bytes.slice(); changed[offset] ^= 1;
    assert.throws(() => decodeVoiceSdpEngineReply(changed, expected()), `field ${offset}`);
  }
  for (const wanted of [ { ...expected(), row: vector.parent }, { ...expected(), engineBytes: vector.engineBytes + 1 },
    { ...expected(), identity: { ...vector.identity, action: "01" } },
    { ...expected(), identity: { ...vector.identity, version: "18446744073709551616" } } ]) {
    assert.throws(() => decodeVoiceSdpEngineReply(bytes, wanted));
  }
});
test("engine decoder snapshots hostile getters and retains only fixed decoded metadata", () => {
  const count = [0, 0, 0, 0, 0, 0, 0, 0], graph = { unrelated: new Uint8Array(10000) };
  const identity = { get action() { return ++count[5] === 1 ? vector.identity.action : graph; },
    get version() { return ++count[6] === 1 ? vector.identity.version : graph; },
    get inputTag() { return ++count[7] === 1 ? vector.identity.inputTag : graph; } };
  const wanted = { get parentRow() { return ++count[0] === 1 ? vector.parent : graph; },
    get row() { return ++count[1] === 1 ? vector.row : graph; },
    get purpose() { return ++count[2] === 1 ? "open" : graph; },
    get identity() { return ++count[3] === 1 ? identity : graph; },
    get engineBytes() { return ++count[4] === 1 ? vector.engineBytes : graph; } };
  const bytes = wire(), reply = decodeVoiceSdpEngineReply(bytes, wanted as unknown as VoiceSdpEngineExpectation);
  assert.deepEqual(count, [1, 1, 1, 1, 1, 1, 1, 1]); bytes.fill(0);
  assert.equal(reply.kind, "admitted"); assert.ok(Object.isFrozen(reply));
  if (reply.kind === "admitted") { assert.ok(Object.isFrozen(reply.identity)); assert.equal(reply.identity.inputTag, vector.identity.inputTag); }
  assert.equal(Object.values(reply).includes(graph), false);
});
test("settled history and refusal recover metadata without becoming another engine permit", () => {
  const histories = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/sdp-history-vectors.json", import.meta.url), "utf8"));
  const bytes = new Uint8Array(Buffer.from(histories.cases.find((v: { name: string }) => v.name === "opened-applied").body, "hex"));
  const wanted = { ...expected(), parentRow: histories.parent, row: histories.row, identity: histories.identity };
  const reply = decodeVoiceSdpEngineReply(bytes, wanted);
  assert.equal(reply.kind, "history");
  assert.deepEqual(decodeVoiceSdpEngineReply(new Uint8Array([8, 2, 2, 7]), expected()), { kind: "refused", code: "capacity" });
  assert.throws(() => decodeVoiceSdpEngineReply(bytes, { ...wanted, identity: { ...wanted.identity, action: "1" } }));
});
