/** Fixed private action-request encoding. These bytes own no native authority. The retained
 * channel authenticates its original frame; native then checks the complete purpose, original
 * selected allocation and exact header RowId before any operation. No JSON number carries u64s.
 * Application-bound native dispatch checks every effect against its original owner and row;
 * UI V2/SDP activation remains unavailable. Cleanup uses existing-history Begin(false). */
import type { VoiceIpcOperation, VoiceIpcBodyKind } from "./voice-ipc-frame.ts";

export type VoiceIpcPurpose = Readonly<{ peer: string; join: string; keyRef: string; epoch: string }>;
export type VoiceIpcActionIdentity = Readonly<{ action: string; version: string; inputTag: string }>;
export type VoiceIpcActionRequest =
  | Readonly<{ kind: "begin"; retry: boolean }>
  | Readonly<{ kind: "input" }>
  | Readonly<{ kind: "close"; mode: "return" | "retire" }>
  | Readonly<{ kind: "close-plan" }>
  | Readonly<{ kind: "admit"; identity: VoiceIpcActionIdentity; engineBytes: number }>
  | Readonly<{ kind: "result"; identity: VoiceIpcActionIdentity; tag: string; output: Uint8Array }>
  | Readonly<{ kind: "ack"; identity: VoiceIpcActionIdentity; digest: string }>
  | Readonly<{ kind: "settle"; identity: VoiceIpcActionIdentity }>
  | Readonly<{ kind: "final"; proof: string }>;

/** Lookup hints for one original native parent. The authenticated native adapter must still
 * resolve its actual allocation and the actual child; an incoming role never creates work. */
export type VoiceIpcConfirmationPurpose = Readonly<{
  parent: VoiceIpcPurpose; parentRow: string;
  request: Readonly<{ kind: "start"; predecessor: string | null }> |
    Readonly<{ kind: "respond"; triggerRow: string }> |
    Readonly<{ kind: "incoming"; role: number }>;
}>;
export type VoiceIpcConfirmationRequest = Exclude<VoiceIpcActionRequest, { kind: "close" | "close-plan" | "final" }> |
  Readonly<{ kind: "close"; mode: "return" }>;
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
function row(target: Uint8Array, offset: number, text: string) {
  if (typeof text !== "string" || !/^[0-9a-f]{32}$/.test(text) || BigInt("0x" + text.slice(0, 16)) >= 32n ||
      BigInt("0x" + text.slice(16)) === 0n) return fail();
  for (let i = 0; i < 16; i++) target[offset + i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
}

/** Allocate exactly one bounded plaintext. The caller must reserve its original channel/phase
 * copy custody BEFORE constructing it, retain it through actual SDK/IO drain, then zero it.
 * A provider's returned output is copied only into that prepaid request, never recomputed. */
export function encodeVoiceIpcAction(purpose: VoiceIpcPurpose, request: VoiceIpcActionRequest): Readonly<{
  operation: VoiceIpcOperation; kind: VoiceIpcBodyKind; body: Uint8Array<ArrayBuffer>;
}> {
  return encode(purpose, request, null);
}

/** A separate v2 prefix, written directly into ONE caller-prepaid plaintext. Never build a
 * v1 packet then expand/copy it: both copies would need custody while output is retained. */
export function encodeVoiceIpcConfirmationAction(purpose: VoiceIpcConfirmationPurpose, request: VoiceIpcConfirmationRequest): Readonly<{
  operation: VoiceIpcOperation; kind: VoiceIpcBodyKind; body: Uint8Array<ArrayBuffer>;
}> {
  if (request.kind === "close" && request.mode !== "return" ||
      (request as VoiceIpcActionRequest).kind === "close-plan" || (request as VoiceIpcActionRequest).kind === "final") return fail();
  return encode(purpose.parent, request, purpose);
}

function encode(purpose: VoiceIpcPurpose, request: VoiceIpcActionRequest, child: VoiceIpcConfirmationPurpose | null): Readonly<{
  operation: VoiceIpcOperation; kind: VoiceIpcBodyKind; body: Uint8Array<ArrayBuffer>;
}> {
  let prefix = 105, childTag = 0, childValue: string | number | null = null;
  if (child) {
    switch (child.request.kind) {
      case "start":
        childValue = child.request.predecessor;
        childTag = childValue === null ? 0 : 1; prefix = childValue === null ? 122 : 154; break;
      case "respond": childTag = 2; childValue = child.request.triggerRow; prefix = 138; break;
      case "incoming":
        childTag = 3; childValue = child.request.role;
        if (!Number.isInteger(childValue) || childValue < 1 || childValue > 9) return fail();
        prefix = 123; break;
      default: return fail();
    }
  }
  let operation: VoiceIpcOperation, length: number;
  const output = request.kind === "result" ? request.output : null;
  const outputLength = output?.byteLength ?? 0;
  switch (request.kind) {
    case "begin":
      if (typeof request.retry !== "boolean") return fail();
      operation = 8; length = 106; break;
    case "input": operation = 1; length = 105; break;
    case "admit":
      if (!Number.isInteger(request.engineBytes) || request.engineBytes < 0 || request.engineBytes > 0xffffffff) return fail();
      operation = 2; length = 157; break;
    case "result":
      if (!(output instanceof Uint8Array) || !Number.isInteger(outputLength) || outputLength < 0 || outputLength > 528) return fail();
      operation = 3; length = 187 + outputLength; break;
    case "ack": operation = 4; length = 185; break;
    case "settle": operation = 5; length = 153; break;
    case "close":
      if (request.mode !== "return" && request.mode !== "retire") return fail();
      operation = 6; length = 106; break;
    case "close-plan": operation = 6; length = 106; break;
    case "final": operation = 7; length = 137; break;
    default: return fail();
  }
  const body = new Uint8Array(length + prefix - 105), view = new DataView(body.buffer);
  try {
    body[0] = child ? 2 : 1;
    hash(body, 1, purpose.peer); hash(body, 33, purpose.join); hash(body, 65, purpose.keyRef);
    view.setBigUint64(97, u64(purpose.epoch, true));
    if (child) {
      row(body, 105, child.parentRow); body[121] = childTag;
      if (childTag === 1) hash(body, 122, childValue as string);
      else if (childTag === 2) row(body, 122, childValue as string);
      else if (childTag === 3) body[122] = childValue as number;
    }
    if ("identity" in request) {
      view.setBigUint64(prefix, u64(request.identity.action));
      view.setBigUint64(prefix + 8, u64(request.identity.version));
      hash(body, prefix + 16, request.identity.inputTag);
    }
    switch (request.kind) {
      case "begin": body[prefix] = Number(request.retry); break;
      case "close": body[prefix] = request.mode === "retire" ? 1 : 0; break;
      case "close-plan": body[prefix] = 2; break;
      case "admit": view.setUint32(prefix + 48, request.engineBytes); break;
      case "result":
        hash(body, prefix + 48, request.tag); view.setUint16(prefix + 80, outputLength);
        body.set(output!, prefix + 82); break;
      case "ack": hash(body, prefix + 48, request.digest); break;
      case "final": hash(body, prefix, request.proof); break;
    }
    return Object.freeze({ operation, kind: operation === 3 ? 1 : 0, body });
  } catch (error) {
    body.fill(0);
    throw error;
  }
}
