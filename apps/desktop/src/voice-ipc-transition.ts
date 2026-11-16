/** Closed custody-only metadata codec. Authenticate the original channel reply BEFORE
 * decoding. These values cannot create a native proof, derive keys, or authorize SDP/RTC. */
import type { VoiceIpcActionIdentity, VoiceIpcPurpose } from "./voice-ipc-actions.ts";

export type VoiceTransitionRequest = Readonly<{
  parentRow: string; purpose: VoiceIpcPurpose; identity: VoiceIpcActionIdentity; handshake: string;
  request: Readonly<{ kind: "claim" }> | Readonly<{ kind: "release"; receipt: string }>;
}>;
export type VoiceTransitionReceipt = Readonly<{
  state: "borrowed" | "closing" | "released"; parentRow: string;
  identity: VoiceIpcActionIdentity; handshake: string; epoch: string;
  finishMessage: string; finishHash: string; until: string; remainingMs: number; receipt: string;
}>;
const MAX_U64 = (1n << 64n) - 1n;
const refuse = (): never => { throw new Error("invalid voice custody transition"); };
function integer(value: string, zero = false): bigint {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]{0,19})$/.test(value)) return refuse();
  const result = BigInt(value);
  if (result > MAX_U64 || (!zero && result === 0n)) return refuse();
  return result;
}
function fixed(value: string, length: number): string {
  if (typeof value !== "string" || value.length !== length * 2 || !/^[0-9a-f]+$/.test(value)) return refuse();
  return value;
}
function parent(value: string): string {
  fixed(value, 16);
  if (BigInt("0x" + value.slice(0, 16)) >= 32n || BigInt("0x" + value.slice(16)) === 0n) return refuse();
  return value;
}
function write(bytes: Uint8Array, position: number, value: string): void {
  for (let i = 0; i < value.length / 2; i++) bytes[position + i] = Number.parseInt(value.slice(i * 2, i * 2 + 2), 16);
}
const hex = (bytes: Uint8Array): string => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");

/** The enclosing original channel must prepay this one Control buffer before construction
 * and keep it until actual SDK/IO drainage. No v1/v2 intermediate body is manufactured. */
export function encodeVoiceTransition(request: VoiceTransitionRequest): Readonly<{
  operation: 10; kind: 0; logical: string; body: Uint8Array<ArrayBuffer>;
}> {
  const logical = parent(request.parentRow);
  const peer = fixed(request.purpose.peer, 32), join = fixed(request.purpose.join, 32);
  const key = fixed(request.purpose.keyRef, 32), epoch = integer(request.purpose.epoch, true);
  const action = integer(request.identity.action), version = integer(request.identity.version);
  const tag = fixed(request.identity.inputTag, 32), handshake = fixed(request.handshake, 32);
  if (request.request.kind !== "claim" && request.request.kind !== "release") return refuse();
  const receipt = request.request.kind === "release" ? fixed(request.request.receipt, 32) : null;
  const body = new Uint8Array(receipt === null ? 186 : 218), view = new DataView(body.buffer);
  body[0] = 3; write(body, 1, peer); write(body, 33, join); write(body, 65, key);
  view.setBigUint64(97, epoch); view.setBigUint64(105, action); view.setBigUint64(113, version);
  write(body, 121, tag); write(body, 153, handshake);
  body[185] = receipt === null ? 0 : 1;
  if (receipt !== null) write(body, 186, receipt);
  return Object.freeze({ operation: 10, kind: 0, logical, body });
}

/** Decode only an authenticated original-parent response. Full expected action/purpose
 * binding is mandatory even for historical Closing/Released replies used by cleanup.
 * Native until is an opaque clock-domain identity; it must never be compared with JS time. */
export function decodeVoiceTransition(bytes: Uint8Array, expected: Omit<VoiceTransitionRequest, "request">): VoiceTransitionReceipt {
  const row = parent(expected.parentRow);
  integer(expected.purpose.epoch, true); integer(expected.identity.action); integer(expected.identity.version);
  fixed(expected.purpose.peer, 32); fixed(expected.purpose.join, 32); fixed(expected.purpose.keyRef, 32);
  fixed(expected.identity.inputTag, 32); fixed(expected.handshake, 32);
  if (!(bytes instanceof Uint8Array) || bytes.length !== 214 || bytes[0] !== 3 || bytes[1] < 1 || bytes[1] > 3) return refuse();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const identity = Object.freeze({ action: view.getBigUint64(18).toString(), version: view.getBigUint64(26).toString(), inputTag: hex(bytes.subarray(34, 66)) });
  const handshake = hex(bytes.subarray(66, 98)), epoch = view.getBigUint64(98).toString();
  if (hex(bytes.subarray(2, 18)) !== row || identity.action !== expected.identity.action || identity.version !== expected.identity.version ||
      identity.inputTag !== expected.identity.inputTag || handshake !== expected.handshake || epoch !== expected.purpose.epoch) return refuse();
  const until = view.getBigUint64(170), remainingMs = view.getUint32(178);
  if (until < 30_000n || until === MAX_U64 || (bytes[1] === 1 ? remainingMs < 1 || remainingMs > 30_000 : remainingMs !== 0)) return refuse();
  return Object.freeze({ state: bytes[1] === 1 ? "borrowed" : bytes[1] === 2 ? "closing" : "released",
    parentRow: row, identity, handshake, epoch, finishMessage: hex(bytes.subarray(106, 138)),
    finishHash: hex(bytes.subarray(138, 170)), until: until.toString(), remainingMs, receipt: hex(bytes.subarray(182, 214)) });
}
