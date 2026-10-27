/** Exact version8/tag3 native Input. Invoke only after authenticating and decrypting the
 * original opcode17 response. This is a borrowed data view, never an engine/SDK/RTC grant.
 * The future browser owner must exclusively retain and fund its entire backing allocation
 * until every asynchronous consumer settles, then clear it. No payload copy is made here. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
import { isVoiceIpcActionRow } from "./voice-ipc-replies.ts";

export const VOICE_SDP_INPUT_FIXED_BYTES = 277;
export const VOICE_SDP_AAD_MAX = 579;
const SDP_MAX = 8192;
export const VOICE_SDP_INPUT_PLAIN_MAX = VOICE_SDP_INPUT_FIXED_BYTES + VOICE_SDP_AAD_MAX + SDP_MAX + 16;

/** All values come from the SAME original native row and PC/SDK/storage custody. Row
 * Action identity is distinct from packet sequence and the parent's confirmation Action. */
export type VoiceSdpInputExpectation = Readonly<{
  row: string; parentRow: string; identity: VoiceIpcActionIdentity; purpose: "seal" | "open";
  generation: "1"; ready: string; key: string; sdkGeneration: "1"; sdkProof: string;
  storage: string; pcProof: string; localIsLow: boolean;
}>;
export type VoiceSdpInput = Readonly<{
  row: string; parentRow: string; identity: VoiceIpcActionIdentity; purpose: "seal" | "open";
  generation: "1"; ready: string; key: string; sdkGeneration: "1"; sdkProof: string;
  storage: string; pcProof: string; sequence: string; engineBytes: number;
  kind: "offer" | "answer"; aad: Uint8Array<ArrayBuffer>; body: Uint8Array<ArrayBuffer>;
}>;
const fail = (): never => { throw new Error("invalid native voice SDP input"); };
const fixed = (value: unknown): value is string => typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
const positive = (value: unknown): value is string => typeof value === "string" &&
  /^[1-9][0-9]{0,19}$/.test(value) && BigInt(value) <= (1n << 64n) - 1n;

// TypedArray accessors and subarray are shadowable even on a real Uint8Array. Use
// intrinsic brand/length checks, never caller accessors or species methods, to establish
// the actual owned backing and construct the two returned views directly from it.
const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
const arrayBufferOf = Object.getOwnPropertyDescriptor(typedArrayPrototype, "buffer")!.get!;
const byteOffsetOf = Object.getOwnPropertyDescriptor(typedArrayPrototype, "byteOffset")!.get!;
const byteLengthOf = Object.getOwnPropertyDescriptor(typedArrayPrototype, "byteLength")!.get!;
const backingLengthOf = Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, "byteLength")!.get!;
const resizableOf = Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, "resizable")?.get;
function originalBacking(bytes: Uint8Array): ArrayBuffer {
  try {
    const buffer = arrayBufferOf.call(bytes) as ArrayBuffer;
    const length = backingLengthOf.call(buffer) as number;
    if (byteOffsetOf.call(bytes) !== 0 || byteLengthOf.call(bytes) !== length ||
        (resizableOf?.call(buffer) ?? false) || length < VOICE_SDP_INPUT_FIXED_BYTES + 1 ||
        length > VOICE_SDP_INPUT_PLAIN_MAX) return fail();
    return buffer;
  } catch { return fail(); } // Shared, detached, Proxy and non-TypedArray brands refuse.
}

/** Decode the native sdp_input_codec::View layout. Read caller fields once, and retain
 * neither caller objects nor an oversized caller backing store. AAD is used verbatim;
 * reconstructing/normalizing authenticated bytes would change the cryptographic input.
 * Open may legitimately produce an empty plaintext from its16-byte input tag; that is
 * separate from AEAD authentication failure and later SDP/RTC validity. */
export function decodeVoiceSdpInput(bytes: Uint8Array<ArrayBuffer>, expected: VoiceSdpInputExpectation): VoiceSdpInput {
  if (!expected) return fail();
  const row = expected.row, parentRow = expected.parentRow, original = expected.identity, purpose = expected.purpose;
  const generation = expected.generation, ready = expected.ready, key = expected.key;
  const sdkGeneration = expected.sdkGeneration, sdkProof = expected.sdkProof, storage = expected.storage;
  const pcProof = expected.pcProof, localIsLow = expected.localIsLow;
  if (!original) return fail();
  const action = original.action, version = original.version, inputTag = original.inputTag;
  if (!isVoiceIpcActionRow(row) || !isVoiceIpcActionRow(parentRow) || row === parentRow ||
      !positive(action) || !positive(version) || !fixed(inputTag) ||
      (purpose !== "seal" && purpose !== "open") || generation !== "1" || sdkGeneration !== "1" ||
      !fixed(ready) || !fixed(key) || !fixed(sdkProof) || !fixed(storage) || !fixed(pcProof) || typeof localIsLow !== "boolean" ||
      !(bytes instanceof Uint8Array)) return fail();
  const buffer = originalBacking(bytes);
  // One intrinsic view excludes caller methods/accessors without copying payload bytes.
  bytes = new Uint8Array(buffer);
  if (bytes[0] !== 8 || bytes[1] !== 3) return fail();
  const view = new DataView(buffer);
  const hex = (start: number, end: number): string => {
    let value = ""; for (let i = start; i < end; i++) value += bytes[i].toString(16).padStart(2, "0"); return value;
  };
  if (hex(2, 18) !== row || hex(18, 34) !== parentRow || view.getBigUint64(34).toString() !== action ||
      view.getBigUint64(42).toString() !== version || hex(50, 82) !== inputTag || view.getBigUint64(82) !== 1n ||
      hex(90, 122) !== ready || bytes[122] !== 1 || hex(123, 155) !== key || view.getBigUint64(155) !== 1n ||
      hex(163, 195) !== sdkProof || hex(195, 227) !== storage || hex(227, 259) !== pcProof) return fail();
  const sequence = view.getBigUint64(259), engineBytes = view.getUint32(267);
  const operation = bytes[271], kind = bytes[272], aadBytes = view.getUint16(273), bodyBytes = view.getUint16(275);
  const seal = purpose === "seal", ciphertextBytes = bodyBytes + (seal ? 16 : 0);
  if (sequence === 0n || operation !== Number(!seal) || kind !== Number(seal ? !localIsLow : localIsLow) ||
      aadBytes > VOICE_SDP_AAD_MAX || bodyBytes < (seal ? 1 : 16) || bodyBytes > SDP_MAX + (seal ? 0 : 16) ||
      bytes.byteLength !== VOICE_SDP_INPUT_FIXED_BYTES + aadBytes + bodyBytes ||
      engineBytes !== 4 * (aadBytes + ciphertextBytes) + 3 * SDP_MAX + 2048) return fail();
  // Only validated scalars and two small views survive. Their backing is the SAME original
  // paid response, so caller mutation/disposal must remain excluded throughout SDK use.
  return Object.freeze({ row, parentRow, identity: Object.freeze({ action, version, inputTag }), purpose,
    generation, ready, key, sdkGeneration, sdkProof, storage, pcProof, sequence: sequence.toString(), engineBytes,
    kind: kind === 0 ? "offer" : "answer", aad: new Uint8Array(buffer, 277, aadBytes), body: new Uint8Array(buffer, 277 + aadBytes, bodyBytes) });
}
