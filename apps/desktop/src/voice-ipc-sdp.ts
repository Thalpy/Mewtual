/** Closed opcode12/version5 storage-phase grammar. Decode only authenticated original
 * channel replies. Funded storage is not Ready, key use, SDK, packet or RTC permission. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
export type VoiceSdpPhaseIdentity = Readonly<{
  parentRow: string; identity: VoiceIpcActionIdentity; handshake: string; key: string; sequence: string;
}>;
export type VoiceSdpPhaseRequest = VoiceSdpPhaseIdentity & Readonly<{
  request: Readonly<{ kind: "prepare" }> | Readonly<{ kind: "release"; proof: string }>;
}>;
export type VoiceSdpPhaseReceipt = VoiceSdpPhaseIdentity & Readonly<{
  state: "preparing" | "funded" | "closing" | "drained"; proof: string;
}>;
const refuse = (): never => { throw new Error("invalid voice SDP storage phase"); };
function integer(value: string): bigint {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value)) return refuse();
  const number = BigInt(value); if (number > (1n << 64n) - 1n) return refuse(); return number;
}
function fixed(value: string, length: number): string {
  if (typeof value !== "string" || value.length !== 2 * length || !/^[0-9a-f]+$/.test(value)) return refuse(); return value;
}
function validate(value: VoiceSdpPhaseIdentity): void {
  fixed(value.parentRow, 16);
  if (BigInt("0x" + value.parentRow.slice(0, 16)) >= 32n || BigInt("0x" + value.parentRow.slice(16)) === 0n) return refuse();
  integer(value.identity.action); integer(value.identity.version); integer(value.sequence);
  fixed(value.identity.inputTag, 32); fixed(value.handshake, 32); fixed(value.key, 32);
}
function write(bytes: Uint8Array, offset: number, value: string): void {
  for (let i = 0; i < value.length / 2; i++) bytes[offset + i] = Number.parseInt(value.slice(i * 2, i * 2 + 2), 16);
}
const hex = (bytes: Uint8Array): string => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
/** Exactly one paid Control body; full header RowId selects the native stored purpose.
 * Positive sequence encoding is lossless. Current native admission accepts only1. */
export function encodeVoiceSdpPhase(value: VoiceSdpPhaseRequest): Readonly<{ operation: 12; kind: 0; logical: string; body: Uint8Array<ArrayBuffer> }> {
  validate(value);
  if (value.request.kind !== "prepare" && value.request.kind !== "release") return refuse();
  const proof = value.request.kind === "release" ? fixed(value.request.proof, 32) : null;
  const body = new Uint8Array(proof === null ? 122 : 154), view = new DataView(body.buffer);
  body[0] = 5; view.setBigUint64(1, integer(value.identity.action)); view.setBigUint64(9, integer(value.identity.version));
  write(body, 17, value.identity.inputTag); write(body, 49, value.handshake); write(body, 81, value.key);
  view.setBigUint64(113, integer(value.sequence)); body[121] = proof === null ? 0 : 1; if (proof !== null) write(body, 122, proof);
  return Object.freeze({ operation: 12, kind: 0, logical: value.parentRow, body });
}
export function decodeVoiceSdpPhase(bytes: Uint8Array, expected: VoiceSdpPhaseIdentity): VoiceSdpPhaseReceipt {
  validate(expected);
  if (!(bytes instanceof Uint8Array) || bytes.length !== 170 || bytes[0] !== 5 || bytes[1] > 3) return refuse();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (hex(bytes.subarray(2, 18)) !== expected.parentRow || view.getBigUint64(18).toString() !== expected.identity.action ||
      view.getBigUint64(26).toString() !== expected.identity.version || hex(bytes.subarray(34, 66)) !== expected.identity.inputTag ||
      hex(bytes.subarray(66, 98)) !== expected.handshake || hex(bytes.subarray(98, 130)) !== expected.key ||
      view.getBigUint64(130).toString() !== expected.sequence) return refuse();
  const states = ["preparing", "funded", "closing", "drained"] as const;
  // Retain only the fixed decoded graph. Caller metadata can be mutable or carry an
  // unrelated large graph; neither belongs to this original paid historical receipt.
  const identity = Object.freeze({ action: view.getBigUint64(18).toString(), version: view.getBigUint64(26).toString(), inputTag: hex(bytes.subarray(34, 66)) });
  return Object.freeze({ parentRow: hex(bytes.subarray(2, 18)), identity,
    handshake: hex(bytes.subarray(66, 98)), key: hex(bytes.subarray(98, 130)), sequence: view.getBigUint64(130).toString(),
    state: states[bytes[1]], proof: hex(bytes.subarray(138, 170)) });
}
