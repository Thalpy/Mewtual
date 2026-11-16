import test from "node:test";
import assert from "node:assert/strict";
import { decodeVoiceIpcDerivationInput, decodeVoiceIpcPermit } from "./voice-ipc-input.ts";
import type { VoiceIpcBootstrap } from "./voice-ipc-bootstrap.ts";
const hash = (n: number) => n.toString(16).padStart(64, "0");
const integer = (n: bigint, size: number) => Buffer.from(n.toString(16).padStart(size * 2, "0"), "hex");
const point = Uint8Array.from(Buffer.from("046b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c2964fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5", "hex"));
// Fixed metadata only. No parser fixture manufactures a Session, native grant or curve key.
function fixture() {
  const localPoint = new Uint8Array(65); localPoint[0] = 4; localPoint.set(point.subarray(1), 1);
  const purpose = Object.freeze({ peer: hash(7), join: hash(8), keyRef: hash(9), epoch: "18446744073709551615" });
  const identity = Object.freeze({ action: "9007199254741007", version: "18446744073709551615", inputTag: hash(11) });
  const bootstrap = Object.freeze({ group: hash(1), policy: hash(2), room: (1n << 128n) - 1n, device: hash(4), join: hash(5),
    owner: "original-owner", generation: 1n }) as VoiceIpcBootstrap;
  const expected = { row: "000000000000001f0000000000000001", identity, purpose, bootstrap, localPoint, localKey: hash(6) };
  const transcript = Buffer.from("native selected transcript"), engineBytes = 3 * transcript.length + 4096 + 256;
  const data = Uint8Array.from(Buffer.concat([Buffer.from([1, 5]), Buffer.from(expected.row, "hex"), integer(BigInt(identity.action), 8), integer(BigInt(identity.version), 8), Buffer.from(identity.inputTag, "hex"),
    integer(1n, 8), Buffer.from(bootstrap.group, "hex"), Buffer.from([1]), Buffer.from(bootstrap.policy!, "hex"), integer(bootstrap.room, 16), integer(BigInt(purpose.epoch), 8),
    ...[bootstrap.device, bootstrap.join, expected.localKey, purpose.peer, purpose.join, purpose.keyRef, hash(10)].map(h => Buffer.from(h, "hex")),
    integer(9007199254741011n, 8), integer(500000n, 8), Buffer.from([1]), integer(BigInt(engineBytes), 4), Buffer.from([0]), localPoint, localPoint,
    integer(BigInt(transcript.length), 2), transcript]));
  return { data, expected, engineBytes };
}
test("Input binds original scope/purpose/row/action while preserving distinct clocks and generations", () => {
  const f = fixture(), before = f.data.slice();
  const input = decodeVoiceIpcDerivationInput(f.data, f.expected);
  assert.equal(input.binding.generation, "9007199254741011"); assert.notEqual(input.binding.generation, f.expected.bootstrap.generation.toString());
  assert.equal(input.binding.nativeDeadline, "500000"); assert.equal(input.binding.epoch, f.expected.purpose.epoch);
  assert.equal(input.action, f.expected.identity.action); assert.equal(input.version, f.expected.identity.version);
  assert.deepEqual(f.data, before);
  f.data.fill(0); assert.ok(input.localPublicKey.some(n => n !== 0)); assert.ok(input.transcript.some(n => n !== 0));
});
test("Input rejects every truncation, trailing bytes, noncanonical tag and changed binding", () => {
  const f = fixture();
  for (let length = 0; length < f.data.length; length++) assert.throws(() => decodeVoiceIpcDerivationInput(f.data.subarray(0, length), f.expected));
  assert.throws(() => decodeVoiceIpcDerivationInput(Uint8Array.from([...f.data, 0]), f.expected));
  for (const [offset, value] of [[0, 2], [1, 6], [74, 2], [106, 2], [403, 2], [408, 1], [409, 3], [474, 3]]) {
    const bad = f.data.slice(); bad[offset] = value; assert.throws(() => decodeVoiceIpcDerivationInput(bad, f.expected), "offset" + offset);
  }
  for (const offset of [17, 25, 33, 65, 73, 105, 138, 154, 162, 194, 226, 258, 290, 322, 354, 407, 420, 540]) {
    const bad = f.data.slice(); bad[offset] ^= 1; assert.throws(() => decodeVoiceIpcDerivationInput(bad, f.expected), "binding" + offset);
  }
  const padding = f.data.slice(); padding[106] = 0; // policy flag false requires zero padding
  assert.throws(() => decodeVoiceIpcDerivationInput(padding, f.expected));
});
test("permit is a separate exact admission shape and cannot be inferred from history or wrong engine", () => {
  const f = fixture();
  const bytes = Uint8Array.from(Buffer.concat([Buffer.from([1, 6]), Buffer.from(f.expected.row, "hex"),
    integer(BigInt(f.expected.identity.action), 8), integer(BigInt(f.expected.identity.version), 8), Buffer.from(f.expected.identity.inputTag, "hex"),
    integer(BigInt(f.expected.identity.action), 8), integer(BigInt(f.engineBytes), 4)]));
  assert.equal(decodeVoiceIpcPermit(bytes, f.expected.row, f.expected.identity, f.engineBytes), f.expected.identity.action);
  for (let length = 0; length < bytes.length; length++) assert.throws(() => decodeVoiceIpcPermit(bytes.subarray(0, length), f.expected.row, f.expected.identity, f.engineBytes));
  for (const offset of [0, 1, 17, 25, 33, 65, 73, 77]) {
    const bad = bytes.slice(); bad[offset] ^= 1; assert.throws(() => decodeVoiceIpcPermit(bad, f.expected.row, f.expected.identity, f.engineBytes));
  }
  assert.throws(() => decodeVoiceIpcPermit(new Uint8Array(242), f.expected.row, f.expected.identity, f.engineBytes));
});
