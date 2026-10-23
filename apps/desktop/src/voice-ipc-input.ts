/** Canonical native Data/permit projections. Call only after original-channel authentication
 * and prepaid intake. These values are data, never native action or RTC authority. */
import type { VoiceIpcPurpose, VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
import type { VoiceIpcBootstrap } from "./voice-ipc-bootstrap.ts";
import { isVoiceIpcActionRow } from "./voice-ipc-replies.ts";
import type { VoiceDerivationInput } from "./voice-crypto.ts";

const fail = (): never => { throw new Error("invalid native voice input"); };
const MAX_TRANSCRIPT = 1024 - new TextEncoder().encode("mewtual/voice/v2/aes-gcm/high-to-low\0").length;
const identityValid = (identity: VoiceIpcActionIdentity) => identity &&
  [identity.action, identity.version].every(value => typeof value === "string" && /^[1-9][0-9]{0,19}$/.test(value) && BigInt(value) <= ((1n << 64n) - 1n)) &&
  typeof identity.inputTag === "string" && /^[0-9a-f]{64}$/.test(identity.inputTag);
function equalIdentity(actual: VoiceIpcActionIdentity, expected: VoiceIpcActionIdentity) {
  if (!identityValid(expected) || actual.action !== expected.action || actual.version !== expected.version || actual.inputTag !== expected.inputTag) fail();
}
function reader(bytes: Uint8Array) {
  let offset = 0;
  const take = (count: number) => {
    if (offset + count > bytes.length) return fail();
    const result = bytes.subarray(offset, offset + count); offset += count; return result;
  };
  const integer = (count: number) => { let n = 0n; for (const byte of take(count)) n = n * 256n + BigInt(byte); return n; };
  const hex = (count: number) => { let s = ""; for (const byte of take(count)) s += byte.toString(16).padStart(2, "0"); return s; };
  const identity = () => Object.freeze({ action: integer(8).toString(), version: integer(8).toString(), inputTag: hex(32) });
  return { take, integer, hex, identity, done: () => { if (offset !== bytes.length) fail(); } };
}

export type VoiceIpcInputExpected = Readonly<{
  row: string; identity: VoiceIpcActionIdentity; purpose: VoiceIpcPurpose;
  bootstrap: VoiceIpcBootstrap; localPoint: Uint8Array; localKey: string;
}>;

/** Copy only inside the caller's already-owned intake reservation. Root chain generation is
 * NOT the UI generation in bootstrap; its monotonic deadline is NOT browser clock time. */
export function decodeVoiceIpcDerivationInput(bytes: Uint8Array, expected: VoiceIpcInputExpected): VoiceDerivationInput {
  if (!(bytes instanceof Uint8Array) || bytes.length < 541 || bytes.length > 2048 ||
      !isVoiceIpcActionRow(expected.row) || !identityValid(expected.identity)) return fail();
  const r = reader(bytes);
  if (r.integer(1) !== 1n || r.integer(1) !== 5n || r.hex(16) !== expected.row) return fail();
  const identity = r.identity(); equalIdentity(identity, expected.identity);
  const chain = r.integer(8).toString();
  if (chain !== BigInt("0x" + expected.row.slice(16)).toString()) return fail();
  const group = r.hex(32), policyFlag = r.integer(1), policyBytes = r.hex(32), room = r.integer(16);
  if (policyFlag > 1n || (policyFlag === 0n && policyBytes !== "0".repeat(64))) return fail();
  const policy = policyFlag === 1n ? policyBytes : null;
  const epoch = r.integer(8).toString(), localDevice = r.hex(32), localJoin = r.hex(32), localKey = r.hex(32);
  const peerDevice = r.hex(32), peerJoin = r.hex(32), peerKey = r.hex(32), handshake = r.hex(32);
  const generation = r.integer(8).toString(), nativeDeadline = r.integer(8).toString(), low = r.integer(1);
  const engineBytes = Number(r.integer(4));
  if (low > 1n || generation === "0" || nativeDeadline === "0" || r.integer(1) !== 0n) return fail();
  const localPublicKey = r.take(65), peerPublicKey = r.take(65), length = Number(r.integer(2));
  const transcript = r.take(length); r.done();
  if (length > MAX_TRANSCRIPT || engineBytes !== 3 * length + 4096 + 256 ||
      group !== expected.bootstrap.group || policy !== expected.bootstrap.policy || room !== expected.bootstrap.room ||
      localDevice !== expected.bootstrap.device || localJoin !== expected.bootstrap.join || localKey !== expected.localKey ||
      epoch !== expected.purpose.epoch || peerDevice !== expected.purpose.peer || peerJoin !== expected.purpose.join || peerKey !== expected.purpose.keyRef ||
      localPublicKey[0] !== 4 || peerPublicKey[0] !== 4 || expected.localPoint.length !== 65 ||
      localPublicKey.some((byte, i) => byte !== expected.localPoint[i])) return fail();
  return Object.freeze({ ...identity, owner: expected.bootstrap.owner, engineBytes,
    binding: Object.freeze({ chain, epoch, localDevice, localJoin, localKey, peerDevice, peerJoin, peerKey, handshake, generation, nativeDeadline, localIsLow: low === 1n }),
    localPublicKey: localPublicKey.slice(), peerPublicKey: peerPublicKey.slice(), transcript: transcript.slice(),
  });
}

/** The native adapter must have actually committed engine admission. Neither a historical
 * status nor successful observation can be decoded as this separate, exact permit shape. */
export function decodeVoiceIpcPermit(bytes: Uint8Array, row: string, identity: VoiceIpcActionIdentity, engineBytes: number): string {
  if (!(bytes instanceof Uint8Array) || bytes.length !== 78 || !isVoiceIpcActionRow(row) || !identityValid(identity) ||
      !Number.isInteger(engineBytes) || engineBytes < 0 || engineBytes > 0xffff_ffff) return fail();
  const r = reader(bytes);
  if (r.integer(1) !== 1n || r.integer(1) !== 6n || r.hex(16) !== row) return fail();
  equalIdentity(r.identity(), identity);
  const permit = r.integer(8).toString();
  if (permit !== identity.action || r.integer(4) !== BigInt(engineBytes)) return fail();
  r.done(); return permit;
}
