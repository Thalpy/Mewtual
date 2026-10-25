/** Version8 SDP-only byte grammar. These are lookup hints, never native/RTC authority.
 * This codec is not wired to the live broker. A private producer must pay the complete
 * backing allocation BEFORE calling, retain it through actual IO, then zero the bytes. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
import type { VoiceIpcHeader } from "./voice-ipc-frame.ts";
import type { VoicePcIdentity } from "./voice-ipc-pc.ts";
import { validateVoiceSdpIdentity } from "./voice-ipc-sdp.ts";

export type VoiceSdpOriginal = VoicePcIdentity & Readonly<{ pcProof: string }>;
export type VoiceSdpRow = Readonly<{ parentRow: string; row: string; purpose: "seal" | "open" }>;
export type VoiceSdpActionRequest =
  | Readonly<{ kind: "input" | "settle" | "authentication-failed"; identity: VoiceIpcActionIdentity }>
  | Readonly<{ kind: "admit"; identity: VoiceIpcActionIdentity; engineBytes: number }>
  | Readonly<{ kind: "sealed" | "opened"; identity: VoiceIpcActionIdentity; tag: string; bytes: Uint8Array }>
  | Readonly<{ kind: "ack"; identity: VoiceIpcActionIdentity; digest: string }>
  | Readonly<{ kind: "delivery"; identity: VoiceIpcActionIdentity; applied: boolean }>
  | Readonly<{ kind: "return" }>
  | Readonly<{ kind: "history" }>;
type Encoded = Readonly<Pick<VoiceIpcHeader, "operation" | "kind" | "logical"> & { body: Uint8Array<ArrayBuffer> }>;
const refuse = (): never => { throw new Error("invalid voice SDP request"); };
function integer(value: string): bigint {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value)) return refuse();
  const parsed = BigInt(value); if (parsed > (1n << 64n) - 1n) return refuse(); return parsed;
}
function fixed(value: string, bytes: number): void {
  if (typeof value !== "string" || value.length !== 2 * bytes || !/^[0-9a-f]+$/.test(value)) refuse();
}
function row(value: string): void {
  fixed(value, 16);
  if (BigInt("0x" + value.slice(0, 16)) >= 32n || BigInt("0x" + value.slice(16)) === 0n) refuse();
}
function write(bytes: Uint8Array, offset: number, value: string): void {
  for (let i = 0; i < value.length / 2; i++) bytes[offset + i] = Number.parseInt(value.slice(2 * i, 2 * i + 2), 16);
}
function identity(bytes: Uint8Array<ArrayBuffer>, offset: number, value: VoiceIpcActionIdentity): void {
  const action = integer(value.action), version = integer(value.version); fixed(value.inputTag, 32);
  const view = new DataView(bytes.buffer); view.setBigUint64(offset, action); view.setBigUint64(offset + 8, version);
  write(bytes, offset + 16, value.inputTag);
}
function purpose(value: "seal" | "open"): number {
  if (value !== "seal" && value !== "open") return refuse(); return value === "seal" ? 0 : 1;
}
function original(value: VoiceSdpOriginal, length: number): Uint8Array<ArrayBuffer> {
  validateVoiceSdpIdentity(value); fixed(value.storage, 32); fixed(value.sdkProof, 32); fixed(value.pcProof, 32);
  const sdk = integer(value.sdkGeneration), bytes = new Uint8Array(length), view = new DataView(bytes.buffer);
  try {
    bytes[0] = 8; identity(bytes, 1, value.identity); write(bytes, 49, value.handshake); write(bytes, 81, value.key);
    view.setBigUint64(113, integer(value.sequence)); write(bytes, 121, value.storage);
    view.setBigUint64(153, sdk); write(bytes, 161, value.sdkProof); write(bytes, 193, value.pcProof);
    return bytes;
  } catch (error) { bytes.fill(0); throw error; }
}
/** null discovers the native incoming Open. A nonempty body asks for the original native
 * role's Seal. No caller-supplied LOW/HIGH, OfferID, Ready, round or deadline is encoded. */
export function encodeVoiceSdpBegin(value: VoiceSdpOriginal, plaintext: Uint8Array | null): Encoded {
  if (plaintext !== null && (!(plaintext instanceof Uint8Array) || plaintext.byteLength < 1 || plaintext.byteLength > 8192)) return refuse();
  const body = original(value, plaintext === null ? 225 : 227 + plaintext.byteLength);
  if (plaintext !== null) { new DataView(body.buffer).setUint16(225, plaintext.byteLength); body.set(plaintext, 227); }
  return Object.freeze({ operation: plaintext === null ? 16 : 15, kind: plaintext === null ? 0 : 2, logical: value.parentRow, body });
}
/** Recover or irreversibly close the original purpose when a lost Begin reply concealed
 * both RowId and Action. This control request remains encodable after large storage closes. */
export function encodeVoiceSdpResolve(value: VoiceSdpOriginal, wanted: "seal" | "open", close: boolean): Encoded {
  const tag = purpose(wanted); if (typeof close !== "boolean") return refuse();
  const body = original(value, 227); body[225] = tag; body[226] = Number(close);
  return Object.freeze({ operation: 27, kind: 0, logical: value.parentRow, body });
}
/** Writes one exact bounded request, including the complete native parent/purpose. The
 * output bytes are copied once into this paid request; SDK work must never be recomputed. */
export function encodeVoiceSdpAction(value: VoiceSdpRow, request: VoiceSdpActionRequest): Encoded {
  row(value.parentRow); row(value.row); const tag = purpose(value.purpose);
  if (value.row === value.parentRow) return refuse();
  let operation: Encoded["operation"], length: number, kind: Encoded["kind"] = 0;
  switch (request.kind) {
    case "input": operation = 17; length = 66; break;
    case "admit":
      if (!Number.isInteger(request.engineBytes) || request.engineBytes < 0 || request.engineBytes > 0xffffffff) return refuse();
      operation = 18; length = 70; break;
    case "sealed": case "opened": {
      const sealed = request.kind === "sealed";
      if (value.purpose !== (sealed ? "seal" : "open") || !(request.bytes instanceof Uint8Array) ||
          request.bytes.byteLength < (sealed ? 17 : 0) || request.bytes.byteLength > (sealed ? 8208 : 8192)) return refuse();
      fixed(request.tag, 32); operation = sealed ? 19 : 20; kind = 2; length = 100 + request.bytes.byteLength; break;
    }
    case "authentication-failed":
      if (value.purpose !== "open") return refuse(); operation = 21; length = 66; break;
    case "ack": fixed(request.digest, 32); operation = 22; length = 98; break;
    case "settle": operation = 23; length = 66; break;
    case "return": operation = 24; length = 18; break;
    case "history": operation = 25; length = 18; break;
    case "delivery":
      if (value.purpose !== "open" || typeof request.applied !== "boolean") return refuse();
      operation = 26; length = 67; break;
    default: return refuse();
  }
  const body = new Uint8Array(length), view = new DataView(body.buffer);
  try {
    body[0] = 8; write(body, 1, value.parentRow); body[17] = tag;
    if (request.kind !== "return" && request.kind !== "history") identity(body, 18, request.identity);
    switch (request.kind) {
      case "admit": view.setUint32(66, request.engineBytes); break;
      case "sealed": case "opened": write(body, 66, request.tag); view.setUint16(98, request.bytes.byteLength); body.set(request.bytes, 100); break;
      case "ack": write(body, 66, request.digest); break;
      case "delivery": body[66] = Number(request.applied); break;
    }
    return Object.freeze({ operation, kind, logical: value.row, body });
  } catch (error) { body.fill(0); throw error; }
}
