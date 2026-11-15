import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { VoiceMediaEncoding } from "./voice-media-encoding.ts";
import { encodeVoiceMediaOriginal, encodeVoiceMediaExisting } from "./voice-ipc-media.ts";
import type { VoiceSdpOriginal } from "./voice-ipc-sdp-actions.ts";

const original = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/requests/media-vectors.json", import.meta.url), "utf8")).original as VoiceSdpOriginal;
const proof = "ab".repeat(32);

test("compact media encoding preserves every existing canonical mode, including maximum integers", () => {
  for (const value of [original, { ...original, sequence: "18446744073709551615", sdkGeneration: "18446744073709551615",
    identity: { ...original.identity, action: "18446744073709551615", version: "18446744073709551615" } }]) {
    const compact = new VoiceMediaEncoding(value);
    for (const kind of ["admit", "resolve", "close", "release"] as const)
      assert.deepEqual(compact.original(kind), encodeVoiceMediaOriginal(value, kind));
    for (const kind of ["current", "close", "release", "detach", "retirehistory"] as const)
      assert.deepEqual(compact.existing(proof, kind), encodeVoiceMediaExisting(value.parentRow, value.identity, proof, kind));
    compact.dispose();
  }
});

test("clearing or mutating a returned request cannot change its template or a sibling request", () => {
  const compact = new VoiceMediaEncoding(original);
  const first = compact.original("admit"), second = compact.original("admit");
  first.body.fill(0);
  assert.deepEqual(second, encodeVoiceMediaOriginal(original, "admit"));
  second.body.fill(255);
  const current = compact.existing(proof, "current"); current.body.fill(0);
  assert.deepEqual(compact.original("resolve"), encodeVoiceMediaOriginal(original, "resolve"));
  assert.deepEqual(compact.existing(proof, "current"), encodeVoiceMediaExisting(original.parentRow, original.identity, proof, "current"));
  compact.dispose();
});

test("input replacement and disposal cannot revive or alter an existing compact binding", () => {
  const value = { ...original, identity: { ...original.identity } };
  const compact = new VoiceMediaEncoding(value), outstanding = compact.original("release");
  value.parentRow = "f".repeat(32); value.identity.inputTag = "f".repeat(64);
  assert.deepEqual(compact.original("resolve"), encodeVoiceMediaOriginal(original, "resolve"));
  compact.dispose(); compact.dispose();
  assert.deepEqual(outstanding, encodeVoiceMediaOriginal(original, "release"));
  assert.throws(() => compact.original("admit"));
  assert.throws(() => compact.existing(proof, "current"));
  assert.throws(() => new VoiceMediaEncoding({ ...original, parentRow: "invalid" }));
  const live = new VoiceMediaEncoding(original);
  assert.throws(() => live.existing("00", "current"));
  assert.throws(() => live.original("invalid" as "admit"));
  assert.throws(() => live.existing(proof, "invalid" as "current"));
  assert.throws(() => Object.create(VoiceMediaEncoding.prototype).original("admit"));
  live.dispose();
});
