/** Closed opcode11 retention-only grammar. Decode only authenticated replies on the original
 * channel. Neither metadata nor a Current snapshot grants SDP, packet or RTC permission. */
import type { VoiceIpcActionIdentity, VoiceIpcPurpose } from "./voice-ipc-actions.ts";

export type VoiceKeySessionRequest = Readonly<{
  parentRow: string; purpose: VoiceIpcPurpose; identity: VoiceIpcActionIdentity; handshake: string;
  claim: string; request: Readonly<{ kind: "adopt" }> | Readonly<{ kind: "release"; receipt: string }>;
}>;
export type VoiceKeySessionReceipt = Readonly<{
  state: "current" | "closing" | "released"; parentRow: string; identity: VoiceIpcActionIdentity;
  handshake: string; epoch: string; finishMessage: string; finishHash: string; claim: string; receipt: string;
}>;
const MAX_U64 = (1n << 64n) - 1n;
const refuse = (): never => { throw new Error("invalid voice key session"); };
function integer(value: string, zero = false): bigint {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,19})$/.test(value)) return refuse();
  const number = BigInt(value);
  if (number > MAX_U64 || (!zero && number === 0n)) return refuse();
  return number;
}
function fixed(value: string, length: number): string {
  if (typeof value !== "string" || value.length !== 2 * length || !/^[0-9a-f]+$/.test(value)) return refuse();
  return value;
}
function parent(value: string): string {
  fixed(value, 16);
  if (BigInt("0x" + value.slice(0, 16)) >= 32n || BigInt("0x" + value.slice(16)) === 0n) return refuse();
  return value;
}
function write(bytes: Uint8Array, offset: number, value: string): void {
  for (let i = 0; i < value.length / 2; i++) bytes[offset + i] = Number.parseInt(value.slice(i * 2, i * 2 + 2), 16);
}
const hex = (bytes: Uint8Array): string => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");

/** Construct exactly one prepaid Control body. There is no version3 intermediate request
 * or copied parent descriptor. The channel holds this buffer through actual SDK/IO finally. */
export function encodeVoiceKeySession(request: VoiceKeySessionRequest): Readonly<{
  operation: 11; kind: 0; logical: string; body: Uint8Array<ArrayBuffer>;
}> {
  const logical = parent(request.parentRow);
  const peer = fixed(request.purpose.peer, 32), join = fixed(request.purpose.join, 32), key = fixed(request.purpose.keyRef, 32);
  const epoch = integer(request.purpose.epoch, true), action = integer(request.identity.action), version = integer(request.identity.version);
  const tag = fixed(request.identity.inputTag, 32), handshake = fixed(request.handshake, 32), claim = fixed(request.claim, 32);
  if (request.request.kind !== "adopt" && request.request.kind !== "release") return refuse();
  const release = request.request.kind === "release" ? fixed(request.request.receipt, 32) : null;
  const body = new Uint8Array(release === null ? 218 : 250), view = new DataView(body.buffer);
  body[0] = 4; write(body, 1, peer); write(body, 33, join); write(body, 65, key);
  view.setBigUint64(97, epoch); view.setBigUint64(105, action); view.setBigUint64(113, version);
  write(body, 121, tag); write(body, 153, handshake); body[185] = release === null ? 0 : 1;
  write(body, 186, claim); if (release !== null) write(body, 218, release);
  return Object.freeze({ operation: 11, kind: 0, logical, body });
}
export function decodeVoiceKeySession(bytes: Uint8Array,
  expected: Omit<VoiceKeySessionRequest, "request"> & Readonly<{ finishMessage: string; finishHash: string }>): VoiceKeySessionReceipt {
  const row = parent(expected.parentRow);
  integer(expected.purpose.epoch, true); integer(expected.identity.action); integer(expected.identity.version);
  fixed(expected.purpose.peer, 32); fixed(expected.purpose.join, 32); fixed(expected.purpose.keyRef, 32);
  fixed(expected.identity.inputTag, 32); fixed(expected.handshake, 32); fixed(expected.claim, 32);
  fixed(expected.finishMessage, 32); fixed(expected.finishHash, 32);
  if (!(bytes instanceof Uint8Array) || bytes.length !== 234 || bytes[0] !== 4 || bytes[1] < 1 || bytes[1] > 3) return refuse();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const identity = Object.freeze({ action: view.getBigUint64(18).toString(), version: view.getBigUint64(26).toString(), inputTag: hex(bytes.subarray(34, 66)) });
  const handshake = hex(bytes.subarray(66, 98)), epoch = view.getBigUint64(98).toString();
  const finishMessage = hex(bytes.subarray(106, 138)), finishHash = hex(bytes.subarray(138, 170)), claim = hex(bytes.subarray(170, 202));
  if (hex(bytes.subarray(2, 18)) !== row || identity.action !== expected.identity.action || identity.version !== expected.identity.version ||
      identity.inputTag !== expected.identity.inputTag || handshake !== expected.handshake || epoch !== expected.purpose.epoch ||
      finishMessage !== expected.finishMessage || finishHash !== expected.finishHash || claim !== expected.claim) return refuse();
  return Object.freeze({ state: bytes[1] === 1 ? "current" : bytes[1] === 2 ? "closing" : "released",
    parentRow: row, identity, handshake, epoch, finishMessage, finishHash, claim, receipt: hex(bytes.subarray(202, 234)) });
}
