/** Closed Control14/version7 on the original authenticated frame/request only.
 * A receipt is a finite native observation, never a standalone RTC or key permit. */
import { validateVoiceSdpIdentity, type VoiceSdpPhaseIdentity } from "./voice-ipc-sdp.ts";
export type VoicePcIdentity = VoiceSdpPhaseIdentity & Readonly<{ storage: string; sdkGeneration: string; sdkProof: string }>;
export type VoicePcRequest = VoicePcIdentity & Readonly<{ request: "admit" | "close" }>;
type Identity = Readonly<{ generation: "1"; proof: string }>;
export type VoicePcReceipt =
  | Readonly<{ state: "preparing"; identity: Identity }>
  | Readonly<{ state: "admitted"; identity: Identity; profile: "complete-sdp-v1"; ready: string; localIsLow: boolean; remainingMs: number }>
  | Readonly<{ state: "closed"; identity: Identity | null }>;
const refuse = (): never => { throw new Error("invalid native voice admission"); };
function integer(value: string): bigint {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value)) return refuse();
  const parsed = BigInt(value); if (parsed > (1n << 64n) - 1n) return refuse(); return parsed;
}
function fixed(value: string): void { if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) refuse(); }
function write(bytes: Uint8Array,offset: number,value: string): void {
  for (let i=0;i<32;i++) bytes[offset+i] = Number.parseInt(value.slice(2*i,2*i+2),16);
}
const hex = (bytes: Uint8Array): string => Array.from(bytes,b=>b.toString(16).padStart(2,"0")).join("");
/** Callers supply only original release obligations. Close after a lost first reply
 * needs no PC receipt. Native derives Ready, role, generation and finite time itself. */
export function encodeVoicePcIntent(value: VoicePcRequest): Readonly<{ operation: 14; kind: 0; logical: string; body: Uint8Array<ArrayBuffer> }> {
  validateVoiceSdpIdentity(value); fixed(value.storage); fixed(value.sdkProof);
  const sdk = integer(value.sdkGeneration);
  if (value.request !== "admit" && value.request !== "close") return refuse();
  const body = new Uint8Array(194),view = new DataView(body.buffer);
  body[0]=7; view.setBigUint64(1,integer(value.identity.action)); view.setBigUint64(9,integer(value.identity.version));
  write(body,17,value.identity.inputTag); write(body,49,value.handshake); write(body,81,value.key);
  view.setBigUint64(113,integer(value.sequence)); write(body,121,value.storage);
  view.setBigUint64(153,sdk); write(body,161,value.sdkProof); body[193]=value.request === "admit" ? 0 : 1;
  return Object.freeze({ operation:14,kind:0,logical:value.parentRow,body });
}
/** Decode only the authenticated reply to this SAME pending request. The eventual
 * consumer must apply request-start+remaining as a conservative deadline before any RTC. */
export function decodeVoicePcIntent(bytes: Uint8Array): VoicePcReceipt {
  if (!(bytes instanceof Uint8Array) || bytes.length < 3 || bytes[0] !== 7 || bytes[1]>2 || bytes[2]>1) return refuse();
  const state=bytes[1],hasIdentity=bytes[2]===1;
  if (!hasIdentity) {
    if (state!==2 || bytes.length!==3) return refuse();
    return Object.freeze({ state:"closed",identity:null });
  }
  if (bytes.length !== (state===1 ? 85 : 43)) return refuse();
  const view = new DataView(bytes.buffer,bytes.byteOffset,bytes.byteLength);
  if (view.getBigUint64(3)!==1n) return refuse();
  const identity: Identity = Object.freeze({ generation:"1",proof:hex(bytes.subarray(11,43)) });
  if (state===0) return Object.freeze({ state:"preparing",identity });
  if (state===2) return Object.freeze({ state:"closed",identity });
  const remaining=view.getBigUint64(77);
  if (bytes[43]!==1 || bytes[44]>1 || remaining===0n || remaining>30_000n) return refuse();
  return Object.freeze({ state:"admitted",identity,profile:"complete-sdp-v1",ready:hex(bytes.subarray(45,77)),localIsLow:bytes[44]===1,remainingMs:Number(remaining) });
}
