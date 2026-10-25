import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { encodeVoiceSdpAction, encodeVoiceSdpBegin, encodeVoiceSdpResolve, type VoiceSdpOriginal, type VoiceSdpRow, type VoiceSdpActionRequest } from "./voice-ipc-sdp-actions.ts";
import { encodeVoiceIpcHeader, type VoiceIpcHeader, type VoiceIpcBodyKind, type VoiceIpcOperation } from "./voice-ipc-frame.ts";
import { encodeVoiceIpcAction } from "./voice-ipc-actions.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/sdp-action-vectors.json", import.meta.url), "utf8"));
const original: VoiceSdpOriginal = vectors.original;
const identity = original.identity;
const logical = (serial: number): string => "0".repeat(16) + serial.toString(16).padStart(16, "0");
const target = (purpose: "seal" | "open"): VoiceSdpRow => ({ parentRow: logical(1), row: logical(2), purpose });
function requests() {
  const list = [
    { name: "seal-begin", ...encodeVoiceSdpBegin(original, new TextEncoder().encode("v=0")) },
    { name: "open-begin", ...encodeVoiceSdpBegin(original, null) },
  ];
  for (const p of ["seal", "open"] as const) for (const close of [false, true]) list.push({ name: `resolve-${p}-${close}`, ...encodeVoiceSdpResolve(original, p, close) });
  for (const p of ["seal", "open"] as const) {
    const value = target(p);
    for (const request of [
      { kind: "input", identity }, { kind: "admit", identity, engineBytes: 0xffffffff }, { kind: "ack", identity, digest: "17".repeat(32) },
      { kind: "settle", identity }, { kind: "return" }, { kind: "history" },
    ] as const) list.push({ name: `${p}-${request.kind}`, ...encodeVoiceSdpAction(value, request) });
    const kind = p === "seal" ? "sealed" : "opened";
    list.push({ name: kind, ...encodeVoiceSdpAction(value, { kind, identity, tag: "17".repeat(32), bytes: new Uint8Array(p === "seal" ? 17 : 1).fill(42) }) });
    if (p === "open") {
      list.push({ name: "authentication-failed", ...encodeVoiceSdpAction(value, { kind: "authentication-failed", identity }) });
      for (const applied of [false, true]) list.push({ name: `delivery-${applied}`, ...encodeVoiceSdpAction(value, { kind: "delivery", identity, applied }) });
    }
  }
  return list;
}
test("Rust/browser SDP vectors pin every request and full-u64 original obligations", () => {
  const actual = requests().map(v => ({ ...v, body: Buffer.from(v.body).toString("hex") }));
  assert.deepEqual(actual, vectors.requests);
  for (const request of requests()) {
    assert.equal(request.body.buffer.byteLength, request.body.byteLength, "one exact backing allocation");
    assert.equal(encodeVoiceIpcHeader({ ...request, direction: 0, slot: 1, sequence: 1n, length: request.body.length + 16 }).length, 40);
  }
});
test("lost-Begin history and Return use only original bounded control obligations", () => {
  for (const wanted of ["seal", "open"] as const) for (const close of [false, true]) {
    const encoded = encodeVoiceSdpResolve(original, wanted, close);
    assert.equal(encoded.operation, 27); assert.equal(encoded.kind, 0); assert.equal(encoded.logical, original.parentRow);
    assert.equal(encoded.body.length, 227); assert.equal(encoded.body[225], wanted === "seal" ? 0 : 1); assert.equal(encoded.body[226], Number(close));
    assert.equal(new DataView(encoded.body.buffer).getBigUint64(1), 9007199254740993n);
  }
  assert.throws(() => encodeVoiceSdpResolve(original, "seal", 1 as unknown as boolean));
  assert.throws(() => encodeVoiceSdpResolve(original, "other" as "seal", false));
});
test("SDP maxima do not widen confirmation results or old Data framing", () => {
  assert.equal(encodeVoiceSdpBegin(original, new Uint8Array(8192)).body.length + 16, 8435);
  assert.throws(() => encodeVoiceSdpBegin(original, new Uint8Array(8193)));
  assert.throws(() => encodeVoiceSdpBegin(original, new Uint8Array(0)));
  for (const [p, kind, maximum] of [["seal", "sealed", 8208], ["open", "opened", 8192]] as const) {
    const request = { kind, identity, tag: "17".repeat(32), bytes: new Uint8Array(maximum) };
    const encoded = encodeVoiceSdpAction(target(p), request);
    assert.equal(encoded.body.length, 100 + maximum);
    assert.throws(() => encodeVoiceSdpAction(target(p), { ...request, bytes: new Uint8Array(maximum + 1) }));
    if (p === "seal") assert.throws(() => encodeVoiceSdpAction(target(p), { ...request, bytes: new Uint8Array(16) }));
    assert.throws(() => encodeVoiceSdpAction(target(p === "seal" ? "open" : "seal"), request));
  }
  assert.throws(() => encodeVoiceIpcAction({ peer: "11".repeat(32), join: "12".repeat(32), keyRef: "13".repeat(32), epoch: "0" }, { kind: "result", identity, tag: "17".repeat(32), output: new Uint8Array(529) }));
});
test("SDP frame matrix is closed in both directions and refuses every other class", () => {
  for (const direction of [0, 1] as const) for (let op = 15; op <= 27; op++) for (const kind of [0, 1, 2] as VoiceIpcBodyKind[]) {
    const largeRequest = direction === 0 && [15, 19, 20].includes(op);
    const largeResponse = direction === 1 && op === 17;
    const max = kind === 0 ? (largeRequest ? null : 272) : kind === 2 ? (largeRequest ? ({ 15: 8435, 19: 8324, 20: 8308 }[op] ?? null) : largeResponse ? 9232 : null) : null;
    const base: VoiceIpcHeader = { direction, operation: op as VoiceIpcOperation, kind, slot: 1, sequence: 1n, logical: logical(1), length: 16 };
    if (max === null) { assert.throws(() => encodeVoiceIpcHeader(base)); continue; }
    for (const length of [16, max]) assert.equal(encodeVoiceIpcHeader({ ...base, length }).length, 40);
    for (const length of [15, max + 1]) assert.throws(() => encodeVoiceIpcHeader({ ...base, length }));
  }
});
test("malformed original IDs, direction/purpose unions and action identities refuse", () => {
  for (const key of ["parentRow", "handshake", "key", "storage", "sdkProof", "pcProof"] as const) {
    assert.throws(() => encodeVoiceSdpBegin({ ...original, [key]: "x" }, null));
  }
  for (const value of ["0", "01", "-1", "18446744073709551616", "1.5"]) for (const key of ["sequence", "sdkGeneration"] as const) {
    assert.throws(() => encodeVoiceSdpBegin({ ...original, [key]: value }, null));
  }
  assert.throws(() => encodeVoiceSdpAction({ ...target("open"), row: logical(1) }, { kind: "history" }));
  for (const request of [{ kind: "delivery", identity, applied: false }, { kind: "authentication-failed", identity }] as const) assert.throws(() => encodeVoiceSdpAction(target("seal"), request));
  assert.throws(() => encodeVoiceSdpAction(target("open"), { kind: "delivery", identity, applied: 2 } as unknown as VoiceSdpActionRequest));
  for (const action of ["0", "01", "18446744073709551616"]) assert.throws(() => encodeVoiceSdpAction(target("seal"), { kind: "input", identity: { ...identity, action } }));
  // Extra caller metadata must not turn a history request into an Action write or enlarge
  // its fixed control allocation. Only the closed selected variant supplies field meaning.
  assert.equal(encodeVoiceSdpAction(target("open"), { kind: "history", identity } as VoiceSdpActionRequest).body.length, 18);
});


test("empty successful Open remains an exact class2 result distinct from AEAD failure", () => {
  const opened = encodeVoiceSdpAction(target("open"), { kind: "opened", identity, tag: "17".repeat(32), bytes: new Uint8Array() });
  assert.equal(opened.operation, 20); assert.equal(opened.kind, 2); assert.equal(opened.body.length, 100);
  assert.equal(new DataView(opened.body.buffer).getUint16(98), 0);
  const failed = encodeVoiceSdpAction(target("open"), { kind: "authentication-failed", identity });
  assert.equal(failed.operation, 21); assert.equal(failed.kind, 0); assert.equal(failed.body.length, 66);
  assert.throws(() => encodeVoiceSdpAction(target("seal"), { kind: "opened", identity, tag: "17".repeat(32), bytes: new Uint8Array() }));
  assert.throws(() => encodeVoiceSdpBegin(original, new Uint8Array()));
});
