import assert from "node:assert/strict";
import test from "node:test";
import { parseEmojiSoundJson } from "./emoji-sound.ts";
import { legacyJamPatch } from "./jam-patch.ts";
const source = () => ({ v: 1, img: "ab".repeat(32), patch: legacyJamPatch("sine"), lo: 60, hi: 84, len: 120 });
test("local emoji source preserves authored length and existing validated recipe at all bounds", () => {
  for (const len of [20, 120, 1000]) { const result = parseEmojiSoundJson(JSON.stringify({ ...source(), len })); assert.equal(result.len, len); assert.equal(result.lo, 60); }
});
test("emoji file source refuses malformed, missing/unknown fields, range/length and hostile recipes", () => {
  for (const bad of ["null", "[]", "{", JSON.stringify({ ...source(), len: 19 }), JSON.stringify({ ...source(), len: 1001 }),
    JSON.stringify({ ...source(), lo: 85 }), JSON.stringify({ ...source(), hi: 128 }), JSON.stringify({ ...source(), hi: 60.5 }),
    JSON.stringify({ ...source(), img: "https://example.test" }), JSON.stringify({ ...source(), extra: true }),
    JSON.stringify({ ...source(), patch: { ...source().patch, automation: [] } }), JSON.stringify({ ...source(), v: 2 })]) {
    assert.throws(() => parseEmojiSoundJson(bad));
  }
  assert.throws(() => parseEmojiSoundJson(" ".repeat(4097)), /4KiB/);
  assert.throws(() => parseEmojiSoundJson(JSON.stringify({ ...source(), img: "é".repeat(2100) })), /4KiB/);
});
