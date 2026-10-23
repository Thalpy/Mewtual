/** Bounded private action-history decoder. Call only AFTER the retained channel authenticates
 * its original response, operation, counters and logical header. Decoding metadata grants no
 * factory, engine, receipt or RTC authority. This is not wired to production action dispatch. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";

const ERRORS = ["malformed", "bounds", "signature", "scope", "freshness", "unauthorized",
  "signing", "capacity", "replay", "reservation", "retry-mismatch", "rate-limited"] as const;
export type VoiceIpcProtocolError = typeof ERRORS[number];
export type VoiceIpcPending = "not-pending" | "open" | "closing" | "closed-unhanded" | "closed-handed";
export type VoiceIpcReceipt = Readonly<{
  action: string; inputTag: string; outputTag: string; retired: boolean; digest: string;
  outcome: Readonly<{ kind: "derived" | "opened" }> |
    Readonly<{ kind: "sealed"; message: string; hash: string }> |
    Readonly<{ kind: "rejected"; code: VoiceIpcProtocolError }>;
}>;
export type VoiceIpcActionReply = Readonly<{ kind: "refused"; code: "closed" | "stopped" | VoiceIpcProtocolError }> |
  Readonly<{ kind: "status"; row: string; pending: VoiceIpcPending; identity: VoiceIpcActionIdentity | null;
    settled: boolean; receipt: VoiceIpcReceipt | null; acknowledged: boolean; localDrained: boolean }>;
export type VoiceIpcFinalReply = Extract<VoiceIpcActionReply, { kind: "refused" }> |
  Readonly<{ kind: "final"; row: string; identity: VoiceIpcActionIdentity; mode: "retire";
    stage: "pending"; proof: null }> |
  Readonly<{ kind: "final"; row: string; identity: VoiceIpcActionIdentity; mode: "retire";
    stage: "issued" | "acknowledged"; proof: string }>;
const fail = (): never => { throw new Error("invalid voice action reply"); };

/** Only an authenticated original-channel reply may reach this decoder. The exact expected
 * row AND action are mandatory; history/final metadata cannot create engine or RTC authority.
 * The original paid Control driver covers these bounded objects/strings. This pure codec
 * performs no fetch, setup, provider action, ACK, retry, or cleanup on the caller's behalf. */
export function decodeVoiceIpcFinalReply(bytes: Uint8Array, expectedRow: string,
  expectedIdentity: VoiceIpcActionIdentity, expectedProof: string | null = null): VoiceIpcFinalReply {
  if (!isVoiceIpcActionRow(expectedRow)) return fail();
  // Validate expected metadata even for a refusal; do not accept lossy/aliased identities.
  const exactU64 = (text: unknown): text is string => typeof text === "string" &&
    /^[1-9][0-9]{0,19}$/.test(text) && BigInt(text) <= ((1n << 64n) - 1n);
  if (!expectedIdentity || !exactU64(expectedIdentity.action) || !exactU64(expectedIdentity.version) ||
    typeof expectedIdentity.inputTag !== "string" || !/^[0-9a-f]{64}$/.test(expectedIdentity.inputTag) ||
    (expectedProof !== null && (typeof expectedProof !== "string" || !/^[0-9a-f]{64}$/.test(expectedProof)))) return fail();
  if (!(bytes instanceof Uint8Array)) return fail();
  if (bytes.byteLength === 3) {
    const refused = decodeVoiceIpcActionReply(bytes, expectedRow);
    if (refused.kind !== "refused") return fail();
    return refused;
  }
  if (bytes.byteLength !== 100 || bytes[0] !== 1 || bytes[1] !== 4 || bytes[66] !== 1 || bytes[67] > 2) return fail();
  const hex = (start: number, end: number) => {
    let result = "";
    for (let i = start; i < end; i++) result += bytes[i].toString(16).padStart(2, "0");
    return result;
  };
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const row = hex(2, 18);
  const identity = Object.freeze({ action: view.getBigUint64(18).toString(), version: view.getBigUint64(26).toString(), inputTag: hex(34, 66) });
  if (row !== expectedRow || identity.action !== expectedIdentity.action || identity.version !== expectedIdentity.version || identity.inputTag !== expectedIdentity.inputTag) return fail();
  const proof = hex(68, 100);
  if (bytes[67] === 0) {
    if (proof !== "0".repeat(64) || expectedProof !== null) return fail();
    return Object.freeze({ kind: "final", row, identity, mode: "retire", stage: "pending", proof: null });
  }
  if (expectedProof !== null && (bytes[67] !== 2 || proof !== expectedProof)) return fail();
  return Object.freeze({ kind: "final", row, identity, mode: "retire", stage: bytes[67] === 2 ? "acknowledged" : "issued", proof });
}

export function isVoiceIpcActionRow(row: unknown): row is string {
  return typeof row === "string" && /^[0-9a-f]{32}$/.test(row) &&
    BigInt("0x" + row.slice(0, 16)) < 32n && BigInt("0x" + row.slice(16)) !== 0n;
}

/** `expectedRow=null` is allowed ONLY for an initial Begin, whose zero logical request header
 * has no known native row yet. Later replies must match the exact original retained row.
 * All u64s remain canonical decimal strings. No SDK call, input mutation or body copy occurs.
 * The enclosing paid Control owner must also cover the bounded decoded objects/strings;242
 * wire bytes alone are not a claim about their memory or about browser/provider drainage. */
export function decodeVoiceIpcActionReply(bytes: Uint8Array, expectedRow: string | null): VoiceIpcActionReply {
  if (expectedRow !== null && !isVoiceIpcActionRow(expectedRow)) return fail();
  if (!(bytes instanceof Uint8Array) || (bytes.byteLength !== 3 && bytes.byteLength !== 242) || bytes[0] !== 1) return fail();
  if (bytes[1] !== 0) {
    if (bytes.byteLength !== 3) return fail();
    if ((bytes[1] === 1 || bytes[1] === 2) && bytes[2] === 0) {
      return Object.freeze({ kind: "refused", code: bytes[1] === 1 ? "closed" : "stopped" });
    }
    if (bytes[1] === 3 && bytes[2] < ERRORS.length) return Object.freeze({ kind: "refused", code: ERRORS[bytes[2]] });
    return fail();
  }
  if (bytes.byteLength !== 242) return fail();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const u64 = (offset: number) => view.getBigUint64(offset).toString();
  const hex = (offset: number, length: number) => {
    let text = "";
    for (let i = offset; i < offset + length; i++) text += bytes[i].toString(16).padStart(2, "0");
    return text;
  };
  const zero = (start: number, end: number) => {
    for (let i = start; i < end; i++) if (bytes[i] !== 0) return fail();
  };
  const flag = (offset: number) => {
    if (bytes[offset] !== 0 && bytes[offset] !== 1) return fail();
    return bytes[offset] === 1;
  };
  const row = hex(2, 16);
  if (!isVoiceIpcActionRow(row) || (expectedRow !== null && row !== expectedRow)) return fail();
  const pending = (["not-pending", "open", "closing", "closed-unhanded", "closed-handed"] as const)[bytes[18]];
  if (pending === undefined) return fail();
  const hasAction = flag(19), settled = flag(68), hasReceipt = flag(69);
  const acknowledged = flag(208), localDrained = flag(209);
  let identity: VoiceIpcActionIdentity | null = null, receipt: VoiceIpcReceipt | null = null;
  if (hasAction) {
    identity = Object.freeze({ action: u64(20), version: u64(28), inputTag: hex(36, 32) });
    if (identity.action === "0" || identity.version === "0") return fail();
  } else zero(20, 68);
  if (hasReceipt) {
    let outcome: VoiceIpcReceipt["outcome"];
    switch (bytes[142]) {
      case 0: zero(143, 207); outcome = Object.freeze({ kind: "derived" }); break;
      case 1: outcome = Object.freeze({ kind: "sealed", message: hex(143, 32), hash: hex(175, 32) }); break;
      case 2: zero(143, 207); outcome = Object.freeze({ kind: "opened" }); break;
      case 3:
        if (bytes[143] >= ERRORS.length) return fail();
        zero(144, 207); outcome = Object.freeze({ kind: "rejected", code: ERRORS[bytes[143]] }); break;
      default: return fail();
    }
    receipt = Object.freeze({ action: u64(70), inputTag: hex(78, 32), outputTag: hex(110, 32),
      outcome, retired: flag(207), digest: hex(210, 32) });
    if (!identity || receipt.action !== identity.action || receipt.inputTag !== identity.inputTag) return fail();
    // This digest is an opaque native ACK identity, not a browser-created Root receipt.
    // The native adapter must compare it against its ACTUAL retained receipt before ACK.
  } else { zero(70, 208); zero(210, 242); }
  if (pending !== "not-pending") {
    if (identity || receipt || settled || acknowledged || localDrained) return fail();
  } else if (!identity) return fail();
  if (acknowledged && !receipt) return fail();
  if (localDrained && (!settled || (receipt && (!acknowledged || !receipt.retired)))) return fail();
  return Object.freeze({ kind: "status", row, pending, identity, settled, receipt, acknowledged, localDrained });
}
