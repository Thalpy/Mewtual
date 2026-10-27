/** Closed Control28/version9 media-obligation metadata. These codecs grant no RTC or
 * native authority. Use only inside the original authenticated, paid physical channel;
 * historical admission must never replace fresh native currentness before PC transfer. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
import type { VoiceSdpOriginal } from "./voice-ipc-sdp-actions.ts";
import { validateVoiceSdpIdentity } from "./voice-ipc-sdp.ts";

export type VoiceMediaOriginalKind = "admit" | "resolve" | "close" | "release";
export type VoiceMediaExistingKind = "current" | "close" | "release" | "detach" | "retirehistory";
export type VoiceMediaReceipt = Readonly<{
  proof: string; admitted: boolean; closed: boolean; released: boolean; current: boolean;
  /** null is historical-only; a number is original Source retirement time remaining. */
  retireInMs: number | null;
}>;
type Encoded = Readonly<{ operation: 28; kind: 0; logical: string; body: Uint8Array<ArrayBuffer> }>;
const refuse = (): never => { throw new Error("invalid native media custody"); };
function integer(value: string): bigint {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value)) return refuse();
  const parsed = BigInt(value); if (parsed > (1n << 64n) - 1n) return refuse(); return parsed;
}
function fixed(value: string, length: number): void {
  if (typeof value !== "string" || value.length !== length * 2 || !/^[0-9a-f]+$/.test(value)) refuse();
}
function row(value: string): void {
  fixed(value, 16);
  if (BigInt("0x" + value.slice(0, 16)) >= 32n || BigInt("0x" + value.slice(16)) === 0n) refuse();
}
function write(bytes: Uint8Array, offset: number, value: string): void {
  for (let i = 0; i < value.length / 2; i++) bytes[offset + i] = Number.parseInt(value.slice(i * 2, i * 2 + 2), 16);
}
function identity(bytes: Uint8Array<ArrayBuffer>, value: VoiceIpcActionIdentity): void {
  const action = integer(value.action), version = integer(value.version); fixed(value.inputTag, 32);
  const view = new DataView(bytes.buffer); view.setBigUint64(2, action); view.setBigUint64(10, version);
  write(bytes, 18, value.inputTag);
}
/** Full original obligations recover uncertainty even when the first media reply was lost.
 * No role, epoch, Ready, deadline, network address or renderer media status is accepted. */
export function encodeVoiceMediaOriginal(value: VoiceSdpOriginal, request: VoiceMediaOriginalKind): Encoded {
  validateVoiceSdpIdentity(value); fixed(value.storage, 32); fixed(value.sdkProof, 32); fixed(value.pcProof, 32);
  const mode = ["admit", "resolve", "close", "release"].indexOf(request);
  const sdk = integer(value.sdkGeneration); if (mode < 0) return refuse();
  const body = new Uint8Array(226), view = new DataView(body.buffer);
  try {
    body[0] = 9; body[1] = mode; identity(body, value.identity);
    write(body, 50, value.handshake); write(body, 82, value.key); view.setBigUint64(114, integer(value.sequence));
    write(body, 122, value.storage); view.setBigUint64(154, sdk); write(body, 162, value.sdkProof); write(body, 194, value.pcProof);
    return Object.freeze({ operation: 28, kind: 0, logical: value.parentRow, body });
  } catch (error) { body.fill(0); throw error; }
}
/** Compact commands require a previously stored exact claim on the native side. A known
 * proof cannot create one. Release is owed only after the actual transferred PC drains. */
export function encodeVoiceMediaExisting(parentRow: string, action: VoiceIpcActionIdentity, proof: string, request: VoiceMediaExistingKind): Encoded {
  row(parentRow); fixed(proof, 32);
  const mode = ["current", "close", "release", "detach", "retirehistory"].indexOf(request); if (mode < 0) return refuse();
  const body = new Uint8Array(82);
  try {
    body[0] = 9; body[1] = 4 + mode; identity(body, action); write(body, 50, proof);
    return Object.freeze({ operation: 28, kind: 0, logical: parentRow, body });
  } catch (error) { body.fill(0); throw error; }
}
/** Decode ONLY the authenticated reply to the same original physical request. This
 * immutable observation is not transferable native permission. null means no stored claim. */
export function decodeVoiceMedia(bytes: Uint8Array): VoiceMediaReceipt | null {
  if (!(bytes instanceof Uint8Array) || bytes[0] !== 9) return refuse();
  if (bytes.length === 2 && bytes[1] === 1) return null;
  if (bytes.length !== 39 || bytes[1] !== 0 || bytes[2] > 15) return refuse();
  const flags = bytes[2], admitted = !!(flags & 1), closed = !!(flags & 2), released = !!(flags & 4), current = !!(flags & 8);
  const remaining = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(35);
  if ((released && !closed) || (current && (!admitted || closed || released || remaining === 0xffffffff)) ||
      (remaining !== 0xffffffff && remaining > 30_000)) return refuse();
  let proof = ""; for (const byte of bytes.subarray(3, 35)) proof += byte.toString(16).padStart(2, "0");
  return Object.freeze({ proof, admitted, closed, released, current, retireInMs: remaining === 0xffffffff ? null : remaining });
}
