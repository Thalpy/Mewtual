/** Private v8 Control reply union. Invoke only after exact retained-channel authentication.
 * Missing and closeRecorded describe a purpose lookup/fence, never whole-PC drainage or
 * permission to create input, run WebCrypto, or touch RTC. Production admission stays closed. */
import { decodeVoiceSdpHistory, type VoiceSdpHistory } from "./voice-ipc-sdp-history.ts";
import { isVoiceIpcActionRow, type VoiceIpcActionReply, type VoiceIpcProtocolError } from "./voice-ipc-replies.ts";
const ERRORS = ["malformed", "bounds", "signature", "scope", "freshness", "unauthorized", "signing", "capacity", "replay", "reservation", "retry-mismatch", "rate-limited"] as const satisfies readonly VoiceIpcProtocolError[];
export type VoiceSdpReplyExpectation = Readonly<{ parentRow: string; row: string | null;
  purpose: "seal" | "open"; resolveClose: boolean | null }>;
export type VoiceSdpReply = Extract<VoiceIpcActionReply, { kind: "refused" }> |
  Readonly<{ kind: "history"; history: VoiceSdpHistory }> |
  Readonly<{ kind: "missing"; parentRow: string; purpose: "seal" | "open"; closeRecorded: boolean }>;
const fail = (): never => { throw new Error("invalid voice SDP reply"); };
/** resolveClose is null for non-Resolve operations; a missing reply is legal only for the
 * exact Resolve that requested it. Snapshot hostile caller fields once before validation. */
export function decodeVoiceSdpReply(bytes: Uint8Array, expected: VoiceSdpReplyExpectation): VoiceSdpReply {
  if (!expected) return fail();
  const parentRow = expected.parentRow, row = expected.row, purpose = expected.purpose, close = expected.resolveClose;
  if (!isVoiceIpcActionRow(parentRow) || (row !== null && !isVoiceIpcActionRow(row)) ||
      (purpose !== "seal" && purpose !== "open") || (close !== null && typeof close !== "boolean") ||
      (close !== null && row !== null) || !(bytes instanceof Uint8Array) || bytes[0] !== 8) return fail();
  if (bytes.byteLength === 220 && bytes[1] === 0) {
    const history = decodeVoiceSdpHistory(bytes, { parentRow, row, purpose });
    return Object.freeze({ kind: "history", history });
  }
  if (bytes.byteLength === 20 && bytes[1] === 1) {
    if (close === null || bytes[18] !== Number(purpose === "open") || bytes[19] !== Number(close)) return fail();
    let observed = "";
    for (let i = 2; i < 18; i++) observed += bytes[i].toString(16).padStart(2, "0");
    if (observed !== parentRow) return fail();
    return Object.freeze({ kind: "missing", parentRow, purpose, closeRecorded: close });
  }
  if (bytes.byteLength !== 4 || bytes[1] !== 2) return fail();
  if ((bytes[2] === 0 || bytes[2] === 1) && bytes[3] === 0) {
    return Object.freeze({ kind: "refused", code: bytes[2] === 0 ? "closed" : "stopped" });
  }
  if (bytes[2] !== 2 || bytes[3] >= ERRORS.length) return fail();
  return Object.freeze({ kind: "refused", code: ERRORS[bytes[3]] });
}
