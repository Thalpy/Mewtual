import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { webcrypto } from "node:crypto";
import { parseVoiceIpcBootstrap, type VoiceIpcBootstrapExpected } from "./voice-ipc-bootstrap.ts";
import { decodeVoiceIpcAdmission, voiceIpcNonce } from "./voice-ipc-frame.ts";

type Vector = {
  name: string; fields: { owner: string; executor: string; join: string; group: string; policy: string | null; slot: number; rawGroup: string };
  browserScalar: string; browserPoint: string; nativePoint: string; shared: string; transcript: string; hash: string; seed: string;
  request: { admission: string; ciphertext: string; plaintext: string };
};
const vectors = (JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/channel/vectors.json", import.meta.url), "utf8")) as { bootstrap: Vector[] }).bootstrap;
const bytes = (value: string): Uint8Array<ArrayBuffer> => Uint8Array.from(Buffer.from(value, "hex"));
const hex = (value: ArrayBuffer | Uint8Array) => Buffer.from(value instanceof ArrayBuffer ? new Uint8Array(value) : value).toString("hex");
const expected = (vector: Vector): VoiceIpcBootstrapExpected => ({ ...vector.fields, point: bytes(vector.browserPoint) });
const utf8 = new TextEncoder();

for (const vector of vectors) test(`real WebCrypto/native local bootstrap vector: ${vector.name}`, async () => {
  const parsed = parseVoiceIpcBootstrap(vector.transcript, expected(vector));
  assert.equal(parsed.generation, 9007199254740993n);
  assert.equal(parsed.room, 340282366920938463463374607431768211454n);
  assert.equal(parsed.policy, vector.fields.policy);
  assert.equal(hex(parsed.nativePoint), vector.nativePoint);
  const point = bytes(vector.browserPoint);
  // Deterministic private scalar import is fixture-only. App setup must use its retained
  // actual selected nonextractable key; there is deliberately no production import API.
  const privateKey = await webcrypto.subtle.importKey("jwk", {
    kty: "EC", crv: "P-256", x: Buffer.from(point.subarray(1, 33)).toString("base64url"),
    y: Buffer.from(point.subarray(33)).toString("base64url"), d: Buffer.from(bytes(vector.browserScalar)).toString("base64url"),
    ext: false, key_ops: ["deriveBits"],
  }, { name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]);
  const native = await webcrypto.subtle.importKey("raw", parsed.nativePoint, { name: "ECDH", namedCurve: "P-256" }, false, []);
  const secret = await webcrypto.subtle.deriveBits({ name: "ECDH", public: native }, privateKey, 256);
  assert.equal(secret.byteLength, 32); assert.equal(hex(secret), vector.shared);
  if (vector.name === "leading-zero-shared") assert.equal(new Uint8Array(secret)[0], 0);
  const hash = await webcrypto.subtle.digest("SHA-256", parsed.transcript);
  assert.equal(hex(hash), vector.hash);
  const material = await webcrypto.subtle.importKey("raw", secret, "HKDF", false, ["deriveBits"]);
  const seed = await webcrypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: hash, info: utf8.encode("Mewtual native voice bootstrap seed v1") }, material, 256);
  assert.equal(hex(seed), vector.seed);
  const frameMaterial = await webcrypto.subtle.importKey("raw", seed, "HKDF", false, ["deriveKey"]);
  const parameters = (label: string) => ({ name: "HKDF", hash: "SHA-256", salt: utf8.encode("Mewtual native voice handoff v1"), info: utf8.encode(label) });
  const mac = await webcrypto.subtle.deriveKey(parameters("v1 c2n header HMAC-SHA256"), frameMaterial, { name: "HMAC", hash: "SHA-256", length: 256 }, false, ["verify"]);
  const body = await webcrypto.subtle.deriveKey(parameters("v1 c2n body AES-256-GCM"), frameMaterial, { name: "AES-GCM", length: 256 }, false, ["decrypt"]);
  const request = decodeVoiceIpcAdmission(vector.request.admission);
  assert.equal(await webcrypto.subtle.verify("HMAC", mac, request.tag, request.bytes), true);
  const plaintext = await webcrypto.subtle.decrypt({ name: "AES-GCM", iv: voiceIpcNonce(request.header), additionalData: request.bytes }, body, bytes(vector.request.ciphertext));
  assert.equal(hex(plaintext), vector.request.plaintext);
  new Uint8Array(secret).fill(0); new Uint8Array(seed).fill(0);
});

test("bootstrap transcript rejects every truncated prefix and trailing data", () => {
  const vector = vectors[0];
  for (let length = 0; length < vector.transcript.length; length += 2) assert.throws(() => parseVoiceIpcBootstrap(vector.transcript.slice(0, length), expected(vector)));
  assert.throws(() => parseVoiceIpcBootstrap(vector.transcript + "00", expected(vector)));
  for (const text of ["00".repeat(1025), vector.transcript.toUpperCase(), vector.transcript + "0"]) assert.throws(() => parseVoiceIpcBootstrap(text, expected(vector)));
});

test("metadata cannot substitute selected owner, executor, join, group or browser point", () => {
  const vector = vectors[0];
  for (const field of ["owner", "executor", "join", "group"] as const) {
    assert.throws(() => parseVoiceIpcBootstrap(vector.transcript, { ...expected(vector), [field]: "changed" }));
  }
  const changed = bytes(vector.browserPoint); changed[10] ^= 1;
  assert.throws(() => parseVoiceIpcBootstrap(vector.transcript, { ...expected(vector), point: changed }));
});

test("canonical version, suite, slot, session, policy discriminator and UTF8 are closed", () => {
  const vector = vectors[0], original = bytes(vector.transcript);
  const domain = utf8.encode("Mewtual native voice bootstrap\0").length;
  const policy = domain + 2 + 16 + 1 + 24 + 2 + vector.fields.rawGroup.length / 2 + 32;
  const owner = Buffer.from(original).indexOf(Buffer.from(vector.fields.owner));
  for (const [at, value] of [[0, 0], [domain, 2], [domain + 1, 2], [domain + 2 + 16, 0], [policy, 2], [owner, 255]]) {
    const changed = original.slice(); changed[at] = value;
    assert.throws(() => parseVoiceIpcBootstrap(hex(changed), expected(vector)));
  }
  const zeroSession = original.slice(); zeroSession.fill(0, domain + 2, domain + 18);
  assert.throws(() => parseVoiceIpcBootstrap(hex(zeroSession), expected(vector)));
});

test("valid-prefix off-curve native point is refused by the actual WebCrypto importer", async () => {
  const vector = vectors[0], changed = bytes(vector.transcript);
  changed.fill(0, changed.length - 65); changed[changed.length - 65] = 4;
  const parsed = parseVoiceIpcBootstrap(hex(changed), expected(vector));
  await assert.rejects(webcrypto.subtle.importKey("raw", parsed.nativePoint, { name: "ECDH", namedCurve: "P-256" }, false, []));
});

test("a UTF8 BOM cannot alias the exact selected owner", () => {
  const vector = vectors[0], original = Buffer.from(bytes(vector.transcript));
  const at = original.indexOf(Buffer.from(vector.fields.owner));
  const changed = Buffer.concat([original.subarray(0, at), Buffer.from([0xef, 0xbb, 0xbf]), original.subarray(at)]);
  changed.writeUInt16BE(Buffer.byteLength(vector.fields.owner) + 3, at - 2);
  assert.throws(() => parseVoiceIpcBootstrap(changed.toString("hex"), expected(vector)));
});
