/** Closed opcode13/version6 SDK custody. Held means a release obligation exists;
 * it is not permission to use keys, construct a PC, or deliver SDP into RTC. */
import { validateVoiceSdpIdentity, type VoiceSdpPhaseIdentity } from "./voice-ipc-sdp.ts";
export type VoiceSdkIdentity = VoiceSdpPhaseIdentity & Readonly<{ storage: string }>;
export type VoiceSdkRequest = VoiceSdkIdentity & Readonly<{
  request: Readonly<{ kind: "admit" }> | Readonly<{ kind: "release"; generation: string; proof: string }>;
}>;
export type VoiceSdkReceipt = VoiceSdkIdentity & Readonly<{
  state: "held" | "closing" | "released"; generation: string; proof: string;
}>;
const refuse = (): never => { throw new Error("invalid voice SDK custody"); };
function fixed(value: string): string { if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) return refuse(); return value; }
function generation(value: string): bigint {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value)) return refuse();
  const number = BigInt(value); if (number > (1n << 64n) - 1n) return refuse(); return number;
}
function write(bytes: Uint8Array, offset: number, value: string): void { for (let i = 0; i < value.length / 2; i++) bytes[offset + i] = Number.parseInt(value.slice(i * 2, i * 2 + 2), 16); }
const hex = (bytes: Uint8Array): string => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");
export function encodeVoiceSdkCustody(value: VoiceSdkRequest): Readonly<{ operation: 13; kind: 0; logical: string; body: Uint8Array<ArrayBuffer> }> {
  validateVoiceSdpIdentity(value); fixed(value.storage);
  if (value.request.kind !== "admit" && value.request.kind !== "release") return refuse();
  const release = value.request.kind === "release" ? { generation: generation(value.request.generation), proof: fixed(value.request.proof) } : null;
  const body = new Uint8Array(release === null ? 154 : 194), view = new DataView(body.buffer);
  body[0] = 6; view.setBigUint64(1, BigInt(value.identity.action)); view.setBigUint64(9, BigInt(value.identity.version));
  write(body, 17, value.identity.inputTag); write(body, 49, value.handshake); write(body, 81, value.key);
  view.setBigUint64(113, BigInt(value.sequence)); write(body, 121, value.storage); body[153] = release === null ? 0 : 1;
  if (release !== null) { view.setBigUint64(154, release.generation); write(body, 162, release.proof); }
  return Object.freeze({ operation: 13, kind: 0, logical: value.parentRow, body });
}
export function decodeVoiceSdkCustody(bytes: Uint8Array, expected: VoiceSdkIdentity): VoiceSdkReceipt {
  validateVoiceSdpIdentity(expected); fixed(expected.storage);
  if (!(bytes instanceof Uint8Array) || bytes.length !== 210 || bytes[0] !== 6 || bytes[1] < 1 || bytes[1] > 3) return refuse();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (hex(bytes.subarray(2, 18)) !== expected.parentRow || view.getBigUint64(18).toString() !== expected.identity.action ||
      view.getBigUint64(26).toString() !== expected.identity.version || hex(bytes.subarray(34, 66)) !== expected.identity.inputTag ||
      hex(bytes.subarray(66, 98)) !== expected.handshake || hex(bytes.subarray(98, 130)) !== expected.key ||
      view.getBigUint64(130).toString() !== expected.sequence || hex(bytes.subarray(138, 170)) !== expected.storage || view.getBigUint64(170) === 0n) return refuse();
  const states = ["held", "closing", "released"] as const;
  return Object.freeze({ parentRow: hex(bytes.subarray(2, 18)), identity: Object.freeze({ action: view.getBigUint64(18).toString(), version: view.getBigUint64(26).toString(), inputTag: hex(bytes.subarray(34, 66)) }),
    handshake: hex(bytes.subarray(66, 98)), key: hex(bytes.subarray(98, 130)), sequence: view.getBigUint64(130).toString(), storage: hex(bytes.subarray(138, 170)),
    generation: view.getBigUint64(170).toString(), proof: hex(bytes.subarray(178, 210)), state: states[bytes[1] - 1] });
}
