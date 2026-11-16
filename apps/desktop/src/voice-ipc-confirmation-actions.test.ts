import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { encodeVoiceIpcAction, encodeVoiceIpcConfirmationAction,
  type VoiceIpcPurpose, type VoiceIpcActionIdentity, type VoiceIpcConfirmationPurpose,
  type VoiceIpcConfirmationRequest } from "./voice-ipc-actions.ts";

type Vector = {
  name: string; operation: number; bodyKind: number; plaintext: string; logical: string;
  purpose: VoiceIpcConfirmationPurpose["request"];
  request: VoiceIpcConfirmationRequest | { kind: "result"; identity: VoiceIpcActionIdentity; tag: string; output: string };
};
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/confirmation/vectors.json", import.meta.url), "utf8")) as {
  parent: VoiceIpcPurpose; parentRow: string; childRow: string; identity: VoiceIpcActionIdentity; cases: Vector[];
};
const purpose = (request: VoiceIpcConfirmationPurpose["request"]): VoiceIpcConfirmationPurpose => ({
  parent: vectors.parent, parentRow: vectors.parentRow, request,
});
for (const item of vectors.cases) test(`native/browser child grammar: ${item.name}`, () => {
  const request = item.request.kind === "result" ? { ...item.request, output: Uint8Array.from(Buffer.from(item.request.output as string, "hex")) } : item.request;
  const encoded = encodeVoiceIpcConfirmationAction(purpose(item.purpose), request);
  assert.equal(encoded.operation, item.operation); assert.equal(encoded.kind, item.bodyKind);
  assert.equal(Buffer.from(encoded.body).toString("hex"), item.plaintext);
  assert.equal(encoded.body.buffer.byteLength, item.plaintext.length / 2, "one exact final plaintext, no padding capacity");
  assert.equal(new DataView(encoded.body.buffer).getBigUint64(97), (1n << 64n) - 1n);
  if (encoded.kind === 0) assert.ok(encoded.body.byteLength <= 256);
  else assert.ok(encoded.body.byteLength <= 764);
  if (request.kind === "result") {
    const before = encoded.body.slice(); request.output.fill(0);
    assert.deepEqual(encoded.body, before, "the original prepaid request survives provider output collection");
  }
});

test("child purpose tags, incoming roles and parent/trigger rows are closed", () => {
  const input = { kind: "input" } as const;
  for (const role of [1, 2, 3, 4, 5, 6, 7, 8, 9]) {
    const encoded = encodeVoiceIpcConfirmationAction(purpose({ kind: "incoming", role }), input);
    assert.equal(encoded.body.length, 123); assert.equal(encoded.body[121], 3); assert.equal(encoded.body[122], role);
  }
  for (const role of [0, 10, 255, -1, 1.5, NaN, Infinity, "2", null]) assert.throws(() =>
    encodeVoiceIpcConfirmationAction(purpose({ kind: "incoming", role } as VoiceIpcConfirmationPurpose["request"]), input));
  for (const invalid of ["", "A".repeat(32), "0".repeat(32), "00000000000000200000000000000001", "0".repeat(33), "g".repeat(32)]) {
    assert.throws(() => encodeVoiceIpcConfirmationAction({ ...purpose({ kind: "start", predecessor: null }), parentRow: invalid }, input));
    assert.throws(() => encodeVoiceIpcConfirmationAction(purpose({ kind: "respond", triggerRow: invalid }), input));
  }
  for (const kind of ["", "derive", "proof", "finish", "Incoming"]) assert.throws(() =>
    encodeVoiceIpcConfirmationAction(purpose({ kind } as VoiceIpcConfirmationPurpose["request"]), input));
  for (const predecessor of ["", "A".repeat(64), "0".repeat(63), undefined]) assert.throws(() =>
    encodeVoiceIpcConfirmationAction(purpose({ kind: "start", predecessor } as VoiceIpcConfirmationPurpose["request"]), input));
});

test("a child cannot encode whole-plan retirement or final ACK, and result528 remains the bound", () => {
  const selected = purpose({ kind: "start", predecessor: "66".repeat(32) });
  for (const request of [{ kind: "close", mode: "retire" }, { kind: "close-plan" }, { kind: "final", proof: "55".repeat(32) }]) {
    assert.throws(() => encodeVoiceIpcConfirmationAction(selected, request as VoiceIpcConfirmationRequest));
  }
  const output = new Uint8Array(529).fill(91);
  assert.throws(() => encodeVoiceIpcConfirmationAction(selected, { kind: "result", identity: vectors.identity, tag: "55".repeat(32), output }));
  assert.ok(output.every(value => value === 91));
  const maximum = encodeVoiceIpcConfirmationAction(selected, { kind: "result", identity: vectors.identity, tag: "55".repeat(32), output: output.subarray(0, 528) });
  assert.equal(maximum.body.byteLength, 764); assert.equal(maximum.kind, 1);
  const ack = encodeVoiceIpcConfirmationAction(selected, { kind: "ack", identity: vectors.identity, digest: "55".repeat(32) });
  assert.equal(ack.body.byteLength, 234); assert.equal(ack.kind, 0);
  // Parent formats keep their original meaning and remain available only through v1.
  assert.equal(encodeVoiceIpcAction(vectors.parent, { kind: "close-plan" }).body[0], 1);
});

test("result length is captured once before allocation even when a local caller supplies an accessor", () => {
  const output = new Uint8Array(1).fill(7); let reads = 0;
  Object.defineProperty(output, "byteLength", { get() { return ++reads === 1 ? 1 : 1_000_000; } });
  const encoded = encodeVoiceIpcConfirmationAction(purpose({ kind: "start", predecessor: null }), {
    kind: "result", identity: vectors.identity, tag: "55".repeat(32), output,
  });
  assert.equal(reads, 1); assert.equal(encoded.body.length, 122 + 82 + 1);
  assert.equal(encoded.body.at(-1), 7);
});
