import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { webcrypto } from "node:crypto";
import { parseAdmittedVoiceIpcBootstrap, parseVoiceIpcBootstrap, type VoiceIpcBootstrapExpected } from "./voice-ipc-bootstrap.ts";
import { decodeVoiceIpcAdmission, voiceIpcNonce } from "./voice-ipc-frame.ts";

type Vector = {
  name: string; fields: { owner: string; executor: string; join: string; group: string; policy: string | null; rawGroup: string };
  browserPoint: string; nativePoint: string; browserScalar: string; shared: string; transcript: string; hash: string; seed: string;
  request: { admission: string; ciphertext: string; plaintext: string };
};
const load = (name: string): Vector[] => JSON.parse(readFileSync(new URL(`../src-tauri/src/voice_handoff/channel/${name}`, import.meta.url), "utf8")).bootstrap;
const vectors = load("admitted-vectors.json"), existing = load("vectors.json");
const utf8 = new TextEncoder();
const bytes = (text: string) => new Uint8Array(Buffer.from(text, "hex"));
const hex = (value: ArrayBuffer | Uint8Array) => Buffer.from(value instanceof Uint8Array ? value : new Uint8Array(value)).toString("hex");
const expected = (value: Vector): VoiceIpcBootstrapExpected => ({ ...value.fields, point: bytes(value.browserPoint) });

for (const vector of vectors) test(`admitted physical bootstrap uses its original new domain and real WebCrypto: ${vector.name}`, async () => {
  const parsed = parseAdmittedVoiceIpcBootstrap(vector.transcript, expected(vector));
  assert.equal(parsed.callProfile, "admitted");
  assert.equal(parsed.generation, 9007199254740993n);
  assert.equal(parsed.room, 340282366920938463463374607431768211454n);
  assert.equal(parsed.policy, vector.fields.policy);
  assert.equal(hex(parsed.nativePoint), vector.nativePoint);
  assert.throws(() => parseVoiceIpcBootstrap(vector.transcript, expected(vector)));
  const original = existing.find(value => value.name === vector.name)!;
  assert.throws(() => parseAdmittedVoiceIpcBootstrap(original.transcript, expected(vector)));
  const point = bytes(vector.browserPoint);
  // Fixed private fixture scalar only. The production admitted parser exposes
  // no private-key import/factory or conversion to the Existing broker map.
  const key = await webcrypto.subtle.importKey("jwk", {
    kty: "EC", crv: "P-256", x: Buffer.from(point.subarray(1, 33)).toString("base64url"),
    y: Buffer.from(point.subarray(33)).toString("base64url"), d: Buffer.from(bytes(vector.browserScalar)).toString("base64url"),
    ext: false, key_ops: ["deriveBits"],
  }, { name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]);
  assert.equal(key.extractable, false);
  const peer = await webcrypto.subtle.importKey("raw", parsed.nativePoint, { name: "ECDH", namedCurve: "P-256" }, false, []);
  const shared = await webcrypto.subtle.deriveBits({ name: "ECDH", public: peer }, key, 256);
  assert.equal(hex(shared), vector.shared);
  const hash = await webcrypto.subtle.digest("SHA-256", parsed.transcript);
  assert.equal(hex(hash), vector.hash);
  const material = await webcrypto.subtle.importKey("raw", shared, "HKDF", false, ["deriveBits"]);
  const seed = await webcrypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: hash,
    info: utf8.encode("Mewtual admitted native voice bootstrap seed v2") }, material, 256);
  assert.equal(hex(seed), vector.seed);
  assert.notEqual(hex(seed), original.seed);
  const frame = await webcrypto.subtle.importKey("raw", seed, "HKDF", false, ["deriveKey"]);
  const params = (label: string) => ({ name: "HKDF", hash: "SHA-256", salt: utf8.encode("Mewtual native voice handoff v1"), info: utf8.encode(label) });
  const mac = await webcrypto.subtle.deriveKey(params("v1 c2n header HMAC-SHA256"), frame, { name: "HMAC", hash: "SHA-256", length: 256 }, false, ["verify"]);
  const body = await webcrypto.subtle.deriveKey(params("v1 c2n body AES-256-GCM"), frame, { name: "AES-GCM", length: 256 }, false, ["decrypt"]);
  const request = decodeVoiceIpcAdmission(vector.request.admission);
  assert.equal(await webcrypto.subtle.verify("HMAC", mac, request.tag, request.bytes), true);
  const oldRequest = decodeVoiceIpcAdmission(original.request.admission);
  assert.equal(await webcrypto.subtle.verify("HMAC", mac, oldRequest.tag, oldRequest.bytes), false);
  const opened = await webcrypto.subtle.decrypt({ name: "AES-GCM", iv: voiceIpcNonce(request.header), additionalData: request.bytes }, body, bytes(vector.request.ciphertext));
  assert.equal(hex(opened), vector.request.plaintext);
  new Uint8Array(shared).fill(0); new Uint8Array(seed).fill(0);
});

test("admitted parser rejects all truncated prefixes, trailing bytes and unbounded text", () => {
  const vector = vectors[0];
  for (let length = 0; length < vector.transcript.length; length += 2) {
    assert.throws(() => parseAdmittedVoiceIpcBootstrap(vector.transcript.slice(0, length), expected(vector)));
  }
  for (const text of [vector.transcript + "00", vector.transcript + "0", vector.transcript.toUpperCase(), "00".repeat(780)]) {
    assert.throws(() => parseAdmittedVoiceIpcBootstrap(text, expected(vector)));
  }
});

test("admitted version, suite, profile, session, slot, policy and UTF8 are closed", () => {
  const vector = vectors[0], raw = bytes(vector.transcript);
  const prefix = utf8.encode("Mewtual admitted native voice bootstrap\0").length;
  const policy = prefix + 3 + 16 + 1 + 24 + 2 + vector.fields.rawGroup.length / 2 + 32;
  const owner = Buffer.from(raw).indexOf(Buffer.from(vector.fields.owner));
  for (const [at, value] of [[0, 0], [prefix, 1], [prefix + 1, 2], [prefix + 2, 0], [prefix + 2, 2],
    [prefix + 3 + 16, 0], [prefix + 3 + 16, 65], [policy, 2], [owner, 255]]) {
    const changed = raw.slice(); changed[at] = value;
    assert.throws(() => parseAdmittedVoiceIpcBootstrap(hex(changed), expected(vector)));
  }
  const zero = raw.slice(); zero.fill(0, prefix + 3, prefix + 19);
  assert.throws(() => parseAdmittedVoiceIpcBootstrap(hex(zero), expected(vector)));
});

test("admitted metadata cannot substitute the original selected identity or point", () => {
  const vector = vectors[0];
  for (const field of ["owner", "executor", "join", "group"] as const) {
    assert.throws(() => parseAdmittedVoiceIpcBootstrap(vector.transcript, { ...expected(vector), [field]: "changed" }));
  }
  const point = bytes(vector.browserPoint); point[12] ^= 1;
  assert.throws(() => parseAdmittedVoiceIpcBootstrap(vector.transcript, { ...expected(vector), point }));
});

test("admitted metadata alone does not establish curve validity or SDK permission", async () => {
  const vector = vectors[0], raw = bytes(vector.transcript);
  raw.fill(0, raw.length - 65); raw[raw.length - 65] = 4;
  const parsed = parseAdmittedVoiceIpcBootstrap(hex(raw), expected(vector));
  await assert.rejects(webcrypto.subtle.importKey("raw", parsed.nativePoint, { name: "ECDH", namedCurve: "P-256" }, false, []));
});

test("admitted decoder accepts the complete365..779-byte grammar with a16-byte room", () => {
  const vector = vectors[0], parsed = parseAdmittedVoiceIpcBootstrap(vector.transcript, expected(vector));
  const be = (value: bigint, width: number) => {
    const out = new Uint8Array(width);
    for (let at = width - 1; at >= 0; at--) { out[at] = Number(value & 255n); value >>= 8n; }
    assert.equal(value, 0n); return out;
  };
  const encode = (rawGroup: Uint8Array, owner: string, policy: Uint8Array | null) => Buffer.concat([
    utf8.encode("Mewtual admitted native voice bootstrap\0"), new Uint8Array([2, 1, 1]), bytes(parsed.session),
    new Uint8Array([parsed.slot]), be(parsed.generation, 8), be(parsed.server, 8), be(parsed.instance, 8),
    be(BigInt(rawGroup.byteLength), 2), rawGroup, bytes(parsed.group), new Uint8Array([policy ? 1 : 0]),
    ...(policy ? [policy] : []), be(parsed.room, 16), bytes(parsed.device), bytes(parsed.join),
    be(BigInt(utf8.encode(owner).byteLength), 2), utf8.encode(owner), bytes(parsed.executor), bytes(vector.browserPoint), bytes(vector.nativePoint),
  ]);
  for (const [width, owner, policy, length] of [[1, "a", null, 365], [256, "a".repeat(128), new Uint8Array(32).fill(7), 779]] as const) {
    const encoded = encode(new Uint8Array(width).fill(3), owner, policy);
    assert.equal(encoded.byteLength, length);
    const result = parseAdmittedVoiceIpcBootstrap(encoded.toString("hex"), { ...expected(vector), owner });
    assert.equal(result.room, parsed.room);
    assert.equal(result.owner, owner);
    assert.equal(result.rawGroup.length, width * 2);
  }
});

test("admitted UTF8 BOM cannot alias the original selected owner", () => {
  const vector = vectors[0], original = Buffer.from(vector.transcript, "hex");
  const at = original.indexOf(Buffer.from(vector.fields.owner));
  const changed = Buffer.concat([original.subarray(0, at), Buffer.from([0xef, 0xbb, 0xbf]), original.subarray(at)]);
  changed.writeUInt16BE(Buffer.byteLength(vector.fields.owner) + 3, at - 2);
  assert.throws(() => parseAdmittedVoiceIpcBootstrap(changed.toString("hex"), expected(vector)));
});
