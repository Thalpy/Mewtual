/** Closed version8 SDP history. Call only after original retained-channel authentication.
 * These fixed observations grant no engine, input, packet, SDK or RTC authority. This pure
 * decoder is staged separately; the live broker still refuses the SDP action family. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
import { isVoiceIpcActionRow, type VoiceIpcPending, type VoiceIpcProtocolError, type VoiceIpcReceipt } from "./voice-ipc-replies.ts";
const ERRORS = ["malformed", "bounds", "signature", "scope", "freshness", "unauthorized", "signing", "capacity", "replay", "reservation", "retry-mismatch", "rate-limited"] as const satisfies readonly VoiceIpcProtocolError[];
export type VoiceSdpHistory = Readonly<{
  row: string; parentRow: string; purpose: "seal" | "open"; pending: VoiceIpcPending;
  identity: VoiceIpcActionIdentity | null; settled: boolean; receipt: VoiceIpcReceipt | null;
  acknowledged: boolean; localDrained: boolean; delivery: "none" | "prepared" | "admitted" | "applied";
}>;
const refuse = (): never => { throw new Error("invalid voice SDP history"); };
/** null row is allowed only while resolving the original lost Begin. Parent and purpose
 * remain mandatory. The enclosing paid Control owner covers bounded decoded strings/objects;
 * no caller metadata, response backing buffer or plaintext is retained in the returned graph. */
export function decodeVoiceSdpHistory(bytes: Uint8Array, expected: Readonly<{ parentRow: string; row: string | null; purpose: "seal" | "open" }>): VoiceSdpHistory {
  if (!expected) return refuse();
  // A typed caller can still supply getters/proxies. Snapshot each field exactly once;
  // only these validated scalars may reach the returned graph or later comparisons.
  const wantedParent = expected.parentRow, wantedRow = expected.row, wantedPurpose = expected.purpose;
  if (!isVoiceIpcActionRow(wantedParent) || (wantedRow !== null && !isVoiceIpcActionRow(wantedRow)) ||
      (wantedPurpose !== "seal" && wantedPurpose !== "open") || !(bytes instanceof Uint8Array) || bytes.byteLength !== 220 || bytes[0] !== 8 || bytes[1] !== 0) return refuse();
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const hex = (start: number, end: number): string => {
    let value = ""; for (let i = start; i < end; i++) value += bytes[i].toString(16).padStart(2, "0"); return value;
  };
  const zero = (start: number, end: number): void => { for (let i = start; i < end; i++) if (bytes[i] !== 0) return refuse(); };
  const flag = (offset: number): boolean => { if (bytes[offset] > 1) return refuse(); return bytes[offset] === 1; };
  const row = hex(2, 18), parentRow = hex(18, 34);
  if (!isVoiceIpcActionRow(row) || row === parentRow || parentRow !== wantedParent || (wantedRow !== null && row !== wantedRow) || bytes[34] > 1 || bytes[34] !== Number(wantedPurpose === "open")) return refuse();
  const pending = (["not-pending", "open", "closing", "closed-unhanded", "closed-handed"] as const)[bytes[35]];
  const delivery = (["none", "prepared", "admitted", "applied"] as const)[bytes[219]];
  if (pending === undefined || delivery === undefined) return refuse();
  const localDrained = flag(36), hasAction = flag(37), settled = flag(86), hasReceipt = flag(87), acknowledged = flag(186);
  let identity: VoiceIpcActionIdentity | null = null, receipt: VoiceIpcReceipt | null = null;
  if (hasAction) {
    const action = view.getBigUint64(38).toString(), version = view.getBigUint64(46).toString();
    if (action === "0" || version === "0") return refuse();
    identity = Object.freeze({ action, version, inputTag: hex(54, 86) });
  } else zero(38, 86);
  if (hasReceipt) {
    if (!identity) return refuse();
    let outcome: VoiceIpcReceipt["outcome"];
    switch (bytes[120]) {
      case 1:
        if (wantedPurpose !== "seal" || delivery !== "none") return refuse();
        outcome = Object.freeze({ kind: "sealed", message: hex(121, 153), hash: hex(153, 185) }); break;
      case 2:
        if (wantedPurpose !== "open" || delivery === "none") return refuse();
        zero(121, 185); outcome = Object.freeze({ kind: "opened" }); break;
      case 3:
        if (bytes[121] >= ERRORS.length || delivery !== "none") return refuse();
        zero(122, 185); outcome = Object.freeze({ kind: "rejected", code: ERRORS[bytes[121]] }); break;
      default: return refuse();
    }
    receipt = Object.freeze({ action: identity.action, inputTag: identity.inputTag, outputTag: hex(88, 120),
      outcome, retired: flag(185), digest: hex(187, 219) });
  } else { zero(88, 186); zero(187, 219); }
  if (!identity && (pending === "not-pending" || settled || receipt || acknowledged || delivery !== "none" ||
      (localDrained && pending !== "closed-unhanded" && pending !== "closed-handed"))) return refuse();
  if ((acknowledged && !receipt) || (!receipt && delivery !== "none") ||
      (localDrained && identity && (!settled || (receipt && (!acknowledged || !receipt.retired))))) return refuse();
  // Construct a fixed owned graph. Never spread an expected object supplied by a caller.
  return Object.freeze({ row, parentRow, purpose: wantedPurpose, pending, identity, settled, receipt, acknowledged, localDrained, delivery });
}
