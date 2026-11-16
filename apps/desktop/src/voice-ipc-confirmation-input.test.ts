import test from "node:test";
import assert from "node:assert/strict";
import { decodeVoiceIpcConfirmationInput } from "./voice-ipc-confirmation-input.ts";
import type { VoiceIpcBootstrap } from "./voice-ipc-bootstrap.ts";
import type { VoiceCryptoChainBinding } from "./voice-crypto.ts";
const hash = (n: number) => n.toString(16).padStart(64, "0");
const integer = (n: bigint, size: number) => Buffer.from(n.toString(16).padStart(size * 2, "0"), "hex");
function fixture(row = 1, low = true) {
  const parent = "000000000000001fffffffffffffffff", child = "00000000000000000000000000000002";
  const identity = { action: "9007199254741007", version: "18446744073709551615", inputTag: hash(11) };
  const bootstrap = { group: hash(1), policy: hash(2), room: (1n << 128n) - 1n,
    device: hash(4), join: hash(5), owner: "original-owner", generation: 1n } as VoiceIpcBootstrap;
  const binding: VoiceCryptoChainBinding = { chain: "18446744073709551615", epoch: "18446744073709551615",
    localDevice: bootstrap.device, localJoin: bootstrap.join, localKey: hash(6), peerDevice: hash(7),
    peerJoin: hash(8), peerKey: hash(9), handshake: hash(10), generation: "9007199254741011",
    nativeDeadline: "500000", localIsLow: low };
  const seal = low ? (row - 1) % 3 !== 1 : (row - 1) % 3 === 1;
  const aad = Buffer.from([1, 3, 5, 7]), body = Buffer.alloc(seal ? 512 : 528, 23);
  const engineBytes = 4 * (aad.length + body.length + (seal ? 16 : 0)) + 3 * 512 + 2048;
  const data = Uint8Array.from(Buffer.concat([Buffer.from([1, 7]), Buffer.from(child, "hex"),
    integer(BigInt(identity.action), 8), integer(BigInt(identity.version), 8), Buffer.from(identity.inputTag, "hex"),
    Buffer.from(parent, "hex"), Buffer.from(bootstrap.group, "hex"), Buffer.from([1]), Buffer.from(bootstrap.policy!, "hex"),
    integer(bootstrap.room, 16), integer(BigInt(binding.epoch), 8),
    ...[binding.localDevice, binding.localJoin, binding.localKey, binding.peerDevice, binding.peerJoin, binding.peerKey, binding.handshake].map(h => Buffer.from(h, "hex")),
    integer(BigInt(binding.generation), 8), integer(BigInt(binding.nativeDeadline), 8), Buffer.from([Number(low)]),
    integer(BigInt(engineBytes), 4), Buffer.from([seal ? 1 : 2, row]), integer(0xffff_ffff_ffff_ffffn, 8),
    integer(BigInt(aad.length), 2), integer(BigInt(body.length), 2), aad, body]));
  assert.equal(data.length, 430 + aad.length + body.length);
  return { data, expected: { row: child, parent, identity, bootstrap, binding }, aad, body, seal, engineBytes };
}

for (const low of [true, false]) for (let role = 1; role <= 9; role++) {
  test(`child Input preserves original chain and closed role ${role}, localLow=${low}`, () => {
    const f = fixture(role, low), before = f.data.slice();
    const input = decodeVoiceIpcConfirmationInput(f.data, f.expected);
    assert.deepEqual(input.binding, f.expected.binding);
    assert.equal(input.binding.chain, "18446744073709551615");
    assert.notEqual(input.binding.chain, BigInt("0x" + f.expected.row.slice(16)).toString());
    assert.equal(input.row, role);
    assert.equal(input.kind, ["hello", "proof", "finish"][(role - 1) % 3]);
    assert.equal(input.operation, f.seal ? "seal" : "open");
    assert.equal(input.sequence, "18446744073709551615");
    assert.equal(input.engineBytes, f.engineBytes);
    assert.deepEqual(input.aad, Uint8Array.from(f.aad));
    assert.deepEqual(input.body, Uint8Array.from(f.body));
    assert.deepEqual(f.data, before);
    f.data.fill(0); assert.ok(input.body.some(n => n !== 0));
    assert.ok(Object.isFrozen(input) && Object.isFrozen(input.binding));
  });
}
test("child Input rejects every truncation, trailing bytes, and all scope/identity substitutions", () => {
  const f = fixture();
  for (let n = 0; n < f.data.length; n++) {
    assert.throws(() => decodeVoiceIpcConfirmationInput(f.data.subarray(0, n), f.expected));
  }
  assert.throws(() => decodeVoiceIpcConfirmationInput(Uint8Array.from([...f.data, 0]), f.expected));
  for (const offset of [0, 1, 17, 25, 33, 65, 81, 113, 114, 146, 162, 170, 202, 234, 266, 298, 330, 362, 394, 402, 410, 411, 415]) {
    const changed = f.data.slice(); changed[offset] ^= 1;
    assert.throws(() => decodeVoiceIpcConfirmationInput(changed, f.expected), "field offset " + offset);
  }
  assert.throws(() => decodeVoiceIpcConfirmationInput(f.data, { ...f.expected, parent: f.expected.row }));
  assert.throws(() => decodeVoiceIpcConfirmationInput(f.data, { ...f.expected,
    binding: { ...f.expected.binding, chain: "2" } }));
  assert.throws(() => decodeVoiceIpcConfirmationInput(f.data, { ...f.expected,
    bootstrap: { ...f.expected.bootstrap, owner: "replacement", device: hash(22) } }));
});
test("closed operations, role bounds, nonce, lengths and engine composition refuse", () => {
  const f = fixture();
  for (const operation of [0, 2, 3, 255]) {
    const changed = f.data.slice(); changed[416] = operation;
    assert.throws(() => decodeVoiceIpcConfirmationInput(changed, f.expected));
  }
  for (const role of [0, 2, 10, 11, 255]) {
    const changed = f.data.slice(); changed[417] = role;
    assert.throws(() => decodeVoiceIpcConfirmationInput(changed, f.expected));
  }
  for (const [start, end] of [[418, 426], [426, 428], [428, 430], [403, 411]]) {
    const changed = f.data.slice(); changed.fill(0, start, end);
    assert.throws(() => decodeVoiceIpcConfirmationInput(changed, f.expected));
  }
  const reserved = f.data.slice(); reserved[114] = 2;
  assert.throws(() => decodeVoiceIpcConfirmationInput(reserved, f.expected));
  const padding = f.data.slice(); padding[114] = 0;
  assert.throws(() => decodeVoiceIpcConfirmationInput(padding, f.expected));
  const tooBig = new Uint8Array(2049);
  assert.throws(() => decodeVoiceIpcConfirmationInput(tooBig, f.expected));
});
