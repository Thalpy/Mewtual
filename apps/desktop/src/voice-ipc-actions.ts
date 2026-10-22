/** Fixed private action-request encoding. These bytes own no native authority. The retained
 * channel authenticates its original frame; native then checks the complete purpose, original
 * selected allocation and exact header RowId before any operation. No JSON number carries u64s.
 * Production admits only existing-history Begin(retry=false) beside Status; effect requests stay disabled. */
import type { VoiceIpcOperation, VoiceIpcBodyKind } from "./voice-ipc-frame.ts";

export type VoiceIpcPurpose = Readonly<{ peer: string; join: string; keyRef: string; epoch: string }>;
export type VoiceIpcActionIdentity = Readonly<{ action: string; version: string; inputTag: string }>;
export type VoiceIpcActionRequest =
  | Readonly<{ kind: "begin"; retry: boolean }>
  | Readonly<{ kind: "input" | "close" }>
  | Readonly<{ kind: "admit"; identity: VoiceIpcActionIdentity; engineBytes: number }>
  | Readonly<{ kind: "result"; identity: VoiceIpcActionIdentity; tag: string; output: Uint8Array }>
  | Readonly<{ kind: "ack"; identity: VoiceIpcActionIdentity; digest: string }>
  | Readonly<{ kind: "settle"; identity: VoiceIpcActionIdentity }>
  | Readonly<{ kind: "final"; proof: string }>;
const U64_MAX = (1n << 64n) - 1n;
const fail = (): never => { throw new Error("invalid voice action request"); };
function u64(text: string, zero = false): bigint {
  if (typeof text !== "string" || !/^(0|[1-9][0-9]{0,19})$/.test(text)) return fail();
  const value = BigInt(text);
  if (value > U64_MAX || (!zero && value === 0n)) return fail();
  return value;
}
function hash(target: Uint8Array, offset: number, text: string) {
  if (typeof text !== "string" || !/^[0-9a-f]{64}$/.test(text)) return fail();
  for (let i = 0; i < 32; i++) target[offset + i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
}

/** Allocate exactly one bounded plaintext. The caller must reserve its original channel/phase
 * copy custody BEFORE constructing it, retain it through actual SDK/IO drain, then zero it.
 * A provider's returned output is copied only into that prepaid request, never recomputed. */
export function encodeVoiceIpcAction(purpose: VoiceIpcPurpose, request: VoiceIpcActionRequest): Readonly<{
  operation: VoiceIpcOperation; kind: VoiceIpcBodyKind; body: Uint8Array<ArrayBuffer>;
}> {
  let operation: VoiceIpcOperation, length: number;
  switch (request.kind) {
    case "begin":
      if (typeof request.retry !== "boolean") return fail();
      operation = 8; length = 106; break;
    case "input": operation = 1; length = 105; break;
    case "admit":
      if (!Number.isInteger(request.engineBytes) || request.engineBytes < 0 || request.engineBytes > 0xffffffff) return fail();
      operation = 2; length = 157; break;
    case "result":
      if (!(request.output instanceof Uint8Array) || request.output.byteLength > 528) return fail();
      operation = 3; length = 187 + request.output.byteLength; break;
    case "ack": operation = 4; length = 185; break;
    case "settle": operation = 5; length = 153; break;
    case "close": operation = 6; length = 105; break;
    case "final": operation = 7; length = 137; break;
    default: return fail();
  }
  const body = new Uint8Array(length), view = new DataView(body.buffer);
  try {
    body[0] = 1;
    hash(body, 1, purpose.peer); hash(body, 33, purpose.join); hash(body, 65, purpose.keyRef);
    view.setBigUint64(97, u64(purpose.epoch, true));
    if ("identity" in request) {
      view.setBigUint64(105, u64(request.identity.action));
      view.setBigUint64(113, u64(request.identity.version));
      hash(body, 121, request.identity.inputTag);
    }
    switch (request.kind) {
      case "begin": body[105] = Number(request.retry); break;
      case "admit": view.setUint32(153, request.engineBytes); break;
      case "result":
        hash(body, 153, request.tag); view.setUint16(185, request.output.byteLength);
        body.set(request.output, 187); break;
      case "ack": hash(body, 153, request.digest); break;
      case "final": hash(body, 105, request.proof); break;
    }
    return Object.freeze({ operation, kind: operation === 3 ? 1 : 0, body });
  } catch (error) {
    body.fill(0);
    throw error;
  }
}
