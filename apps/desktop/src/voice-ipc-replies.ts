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
const fail = (): never => { throw new Error("invalid voice action reply"); };

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
