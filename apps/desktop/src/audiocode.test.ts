// Unit tests for the acoustic pairing codec; pure DSP, so the whole modem round-trips
// in Node with no audio hardware. Run with `npm test`.

import { test } from "node:test";
import assert from "node:assert/strict";

import { crc16, decodeAudio, encodeAudio, MAX_AUDIO_PAYLOAD } from "./audiocode.ts";

const SR = 48_000;

function bytes(n: number, seed = 7): Uint8Array {
  // Deterministic pseudo-random payload (xorshift); no Math.random in tests.
  const out = new Uint8Array(n);
  let x = seed >>> 0 || 1;
  for (let i = 0; i < n; i++) {
    x ^= x << 13;
    x >>>= 0;
    x ^= x >> 17;
    x ^= x << 5;
    x >>>= 0;
    out[i] = x & 0xff;
  }
  return out;
}

function noisy(samples: Float32Array, amp: number, seed = 99): Float32Array {
  const out = new Float32Array(samples.length);
  let x = seed >>> 0;
  for (let i = 0; i < samples.length; i++) {
    x ^= x << 13;
    x >>>= 0;
    x ^= x >> 17;
    x ^= x << 5;
    x >>>= 0;
    out[i] = samples[i] + amp * ((x / 0xffffffff) * 2 - 1);
  }
  return out;
}

test("crc16 pins the CCITT vector", () => {
  // "123456789" → 0x29B1 is the classic CRC16-CCITT check value.
  assert.equal(crc16(new TextEncoder().encode("123456789")), 0x29b1);
});

test("a payload round-trips clean", () => {
  const p = bytes(120);
  const wave = encodeAudio(p, SR);
  assert.deepEqual(decodeAudio(wave, SR), p);
});

test("round-trips at 44.1kHz too", () => {
  const p = bytes(64);
  assert.deepEqual(decodeAudio(encodeAudio(p, 44_100), 44_100), p);
});

test("survives noise and surrounding silence", () => {
  const p = bytes(80, 3);
  const wave = encodeAudio(p, SR);
  const padded = new Float32Array(SR + wave.length + SR); // a second of silence each side
  padded.set(noisy(wave, 0.15), SR);
  assert.deepEqual(decodeAudio(padded, SR), p);
});

test("survives an unaligned recording start", () => {
  const p = bytes(40, 5);
  const wave = encodeAudio(p, SR);
  const cut = wave.slice(1_113); // recorder started late, mid-lead-in padding
  assert.deepEqual(decodeAudio(cut, SR), p);
});

test("a truncated transmission decodes to null, never garbage", () => {
  const p = bytes(60, 9);
  const wave = encodeAudio(p, SR);
  assert.equal(decodeAudio(wave.slice(0, wave.length >> 1), SR), null);
});

test("corruption is caught by the CRC", () => {
  const p = bytes(48, 11);
  const wave = encodeAudio(p, SR);
  // Zero out a symbol's worth of samples in the middle of the payload region.
  const mangled = wave.slice();
  const mid = mangled.length >> 1;
  for (let i = 0; i < 2_400; i++) mangled[mid + i] = 0;
  assert.equal(decodeAudio(mangled, SR), null);
});

test("silence and pure noise decode to null", () => {
  assert.equal(decodeAudio(new Float32Array(SR * 2), SR), null);
  assert.equal(decodeAudio(noisy(new Float32Array(SR * 2), 0.5), SR), null);
});

test("payload bounds are enforced at encode", () => {
  assert.throws(() => encodeAudio(new Uint8Array(0), SR));
  assert.throws(() => encodeAudio(new Uint8Array(MAX_AUDIO_PAYLOAD + 1), SR));
  assert.ok(encodeAudio(new Uint8Array(MAX_AUDIO_PAYLOAD).fill(1), SR).length > 0);
});
