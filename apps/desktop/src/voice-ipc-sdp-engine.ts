/** Private opcode18/version8 engine replies. Decode only after authenticating the exact
 * original request/response pair. A repeated permit names the SAME engine; it never
 * authorizes a second SDK invocation. Production SDP admission remains disabled. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
import { isVoiceIpcActionRow } from "./voice-ipc-replies.ts";
import { decodeVoiceSdpReply, type VoiceSdpReply } from "./voice-ipc-sdp-replies.ts";

export type VoiceSdpEngineExpectation = Readonly<{
  parentRow: string; row: string; purpose: "seal" | "open";
  identity: VoiceIpcActionIdentity; engineBytes: number;
}>;
export type VoiceSdpEngineReply = Extract<VoiceSdpReply, { kind: "history" | "refused" }> |
  Readonly<{ kind: "admitted"; row: string; identity: VoiceIpcActionIdentity; permit: string; engineBytes: number }>;
const fail = (): never => { throw new Error("invalid voice SDP engine reply"); };
function positive(value: string): void {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value) || BigInt(value) > (1n << 64n) - 1n) return fail();
}

/** The existing paid Control owner covers this fixed decoded graph. Read caller fields
 * once, retain no caller/response backing, and never fall back to confirmation grammar. */
export function decodeVoiceSdpEngineReply(bytes: Uint8Array, expected: VoiceSdpEngineExpectation): VoiceSdpEngineReply {
  if (!expected) return fail();
  const parentRow = expected.parentRow, row = expected.row, purpose = expected.purpose;
  const original = expected.identity, engineBytes = expected.engineBytes;
  if (!original) return fail();
  const action = original.action, version = original.version, inputTag = original.inputTag;
  if (!isVoiceIpcActionRow(parentRow) || !isVoiceIpcActionRow(row) || row === parentRow ||
      (purpose !== "seal" && purpose !== "open") || typeof inputTag !== "string" || !/^[0-9a-f]{64}$/.test(inputTag) ||
      !Number.isInteger(engineBytes) || engineBytes < 1 || engineBytes > 0xffffffff ||
      !(bytes instanceof Uint8Array) || bytes[0] !== 8) return fail();
  positive(action); positive(version);
  if (bytes.byteLength !== 78 || bytes[1] !== 4) {
    const reply = decodeVoiceSdpReply(bytes, { parentRow, row, purpose, resolveClose: null });
    if (reply.kind === "missing") return fail();
    if (reply.kind === "history") {
      const observed = reply.history.identity;
      if (!observed || observed.action !== action || observed.version !== version || observed.inputTag !== inputTag) return fail();
    }
    return reply;
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const hex = (start: number, end: number): string => {
    let value = ""; for (let i = start; i < end; i++) value += bytes[i].toString(16).padStart(2, "0"); return value;
  };
  const observedRow = hex(2, 18), observedAction = view.getBigUint64(18).toString();
  const observedVersion = view.getBigUint64(26).toString(), observedTag = hex(34, 66);
  const permit = view.getBigUint64(66).toString(), charged = view.getUint32(74);
  if (observedRow !== row || observedAction !== action || observedVersion !== version || observedTag !== inputTag ||
      permit !== action || charged !== engineBytes) return fail();
  return Object.freeze({ kind: "admitted", row: observedRow, permit, engineBytes: charged,
    identity: Object.freeze({ action: observedAction, version: observedVersion, inputTag: observedTag }) });
}
