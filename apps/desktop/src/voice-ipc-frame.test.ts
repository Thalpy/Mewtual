import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { webcrypto } from "node:crypto";
import {
  decodeVoiceIpcAdmission, encodeVoiceIpcAdmission, encodeVoiceIpcHeader,
  voiceIpcNonce, voiceIpcResponseAad, VoiceIpcFrameError,
  type VoiceIpcHeader, type VoiceIpcDirection, type VoiceIpcOperation, type VoiceIpcBodyKind,
} from "./voice-ipc-frame.ts";

// Raw seed import exists only in this deterministic fixture. App code deliberately cannot
// reset a physical channel's nonce allocator by repeating a seed after a remount/lost reply.
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/channel/vectors.json", import.meta.url), "utf8")) as {
  seed: string; cases: { name: string; request: Vector; response: Vector }[];
};
type Vector = { header: string; admission: string; plaintext: string; ciphertext: string };
const bytes = (text: string): Uint8Array<ArrayBuffer> => Uint8Array.from(Buffer.from(text, "hex"));
const hex = (data: BufferSource) => Buffer.from(data instanceof ArrayBuffer ? data : new Uint8Array(data.buffer, data.byteOffset, data.byteLength)).toString("hex");
const utf8 = new TextEncoder();
async function keys() {
  const root = await webcrypto.subtle.importKey("raw", bytes(vectors.seed), "HKDF", false, ["deriveKey"]);
  const derive = (label: string, algorithm: AesKeyGenParams | HmacImportParams, usages: KeyUsage[]) => webcrypto.subtle.deriveKey({
    name: "HKDF", hash: "SHA-256", salt: utf8.encode("Mewtual native voice handoff v1"), info: utf8.encode(label),
  }, root, algorithm, false, usages);
  return {
    requestMac: await derive("v1 c2n header HMAC-SHA256", { name: "HMAC", hash: "SHA-256", length: 256 }, ["sign", "verify"]),
    requestBody: await derive("v1 c2n body AES-256-GCM", { name: "AES-GCM", length: 256 }, ["encrypt", "decrypt"]),
    responseMac: await derive("v1 n2c header HMAC-SHA256", { name: "HMAC", hash: "SHA-256", length: 256 }, ["sign", "verify"]),
    responseBody: await derive("v1 n2c body AES-256-GCM", { name: "AES-GCM", length: 256 }, ["encrypt", "decrypt"]),
  };
}
const base: VoiceIpcHeader = { direction: 0, operation: 9, kind: 0, slot: 1, sequence: 1n, logical: "09".repeat(16), length: 17 };

for (const vector of vectors.cases) test(`Rust / real WebCrypto fixed frame vector: ${vector.name}`, async () => {
  const key = await keys();
  const request = decodeVoiceIpcAdmission(vector.request.admission), response = decodeVoiceIpcAdmission(vector.response.admission);
  assert.equal(hex(encodeVoiceIpcHeader(request.header)), vector.request.header);
  assert.equal(hex(encodeVoiceIpcHeader(response.header)), vector.response.header);
  const aad = voiceIpcResponseAad(response.header, request.header);
  for (const [frame, expected, mac, body, associatedData] of [
    [request, vector.request, key.requestMac, key.requestBody, request.bytes],
    [response, vector.response, key.responseMac, key.responseBody, aad],
  ] as const) {
    assert.equal(mac.extractable, false); assert.equal(body.extractable, false);
    const tag = await webcrypto.subtle.sign("HMAC", mac, associatedData);
    assert.equal(encodeVoiceIpcAdmission(frame.header, new Uint8Array(tag)), expected.admission);
    assert.equal(await webcrypto.subtle.verify("HMAC", mac, frame.tag, associatedData), true);
    const algorithm = { name: "AES-GCM", iv: voiceIpcNonce(frame.header), additionalData: associatedData, tagLength: 128 };
    assert.equal(hex(await webcrypto.subtle.encrypt(algorithm, body, bytes(expected.plaintext))), expected.ciphertext);
    assert.equal(hex(await webcrypto.subtle.decrypt(algorithm, body, bytes(expected.ciphertext))), expected.plaintext);
  }
});

test("closed direction / operation / body matrix accepts only the native classes at exact bounds", () => {
  for (const direction of [0, 1] as VoiceIpcDirection[]) for (let operation = 1; operation <= 10; operation++) {
    for (const kind of [0, 1] as VoiceIpcBodyKind[]) {
      const header = { ...base, direction, operation: operation as VoiceIpcOperation, kind };
      const data = (direction === 0 && operation === 3) || (direction === 1 && operation === 1);
      const legal = kind === 1 ? data : !(direction === 0 && operation === 3);
      if (!legal) { assert.throws(() => encodeVoiceIpcHeader(header), VoiceIpcFrameError); continue; }
      const max = kind === 1 ? 2064 : 272;
      for (const length of [16, max]) assert.equal(encodeVoiceIpcHeader({ ...header, length }).length, 40);
      for (const length of [0, 15, max + 1, NaN, 16.5]) assert.throws(() => encodeVoiceIpcHeader({ ...header, length }), VoiceIpcFrameError);
    }
  }
});

test("unknown fields and noncanonical ASCII refuse even with a correctly recomputed request MAC", async () => {
  const key = await keys(), canonical = encodeVoiceIpcHeader(base);
  for (const [index, value] of [[0, 0], [4, 2], [5, 2], [6, 0], [6, 255], [7, 0], [7, 65], [36, 1], [36, 2], [37, 1], [38, 1], [39, 1]]) {
    const changed = canonical.slice(); changed[index] = value;
    const admission = hex(changed) + hex(await webcrypto.subtle.sign("HMAC", key.requestMac, changed));
    assert.throws(() => decodeVoiceIpcAdmission(admission), VoiceIpcFrameError, `offset ${index}`);
  }
  const valid = vectors.cases[0].request.admission;
  for (const text of [valid.toUpperCase(), valid + "0", valid.slice(1), valid.slice(0, -1) + "g", "", "0".repeat(10000)]) {
    assert.throws(() => decodeVoiceIpcAdmission(text), VoiceIpcFrameError);
  }
});

test("u64 sequences retain exact bytes beyond JS safe integers and never wrap", () => {
  for (const sequence of [1n, 9007199254741007n, (1n << 64n) - 1n]) {
    const header = { ...base, sequence };
    assert.equal(decodeVoiceIpcAdmission(encodeVoiceIpcAdmission(header, new Uint8Array(32))).header.sequence, sequence);
  }
  for (const sequence of [0n, -1n, 1n << 64n]) assert.throws(() => encodeVoiceIpcHeader({ ...base, sequence }), VoiceIpcFrameError);
});

test("direction reflection and substitution of the original physical request fail real WebCrypto authentication", async () => {
  const key = await keys(), vector = vectors.cases[0];
  const request = decodeVoiceIpcAdmission(vector.request.admission), response = decodeVoiceIpcAdmission(vector.response.admission);
  const original = voiceIpcResponseAad(response.header, request.header);
  const substituted = voiceIpcResponseAad(response.header, { ...request.header, sequence: request.header.sequence + 1n });
  assert.equal(await webcrypto.subtle.verify("HMAC", key.responseMac, response.tag, substituted), false);
  await assert.rejects(webcrypto.subtle.decrypt({ name: "AES-GCM", iv: voiceIpcNonce(response.header), additionalData: substituted, tagLength: 128 }, key.responseBody, bytes(vector.response.ciphertext)));
  assert.equal(await webcrypto.subtle.verify("HMAC", key.requestMac, response.tag, original), false);
  await assert.rejects(webcrypto.subtle.decrypt({ name: "AES-GCM", iv: voiceIpcNonce(response.header), additionalData: original, tagLength: 128 }, key.requestBody, bytes(vector.response.ciphertext)));
  for (const changed of [{ ...response.header, slot: 2 }, { ...response.header, operation: 9 as const }, { ...response.header, logical: "00".repeat(16) }, { ...response.header, direction: 0 as const }]) {
    assert.throws(() => voiceIpcResponseAad(changed, request.header), VoiceIpcFrameError);
  }
});
