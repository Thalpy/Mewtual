/** Canonical metadata for the disabled native voice handoff candidate. These helpers own no
 * keys/counters and authenticate nothing. The retained paid channel pins the original bytes,
 * verifies the directional MAC and debits its original session before reading any body. Seed
 * import remains fixture-only; production construction uses the original selected key. */
export type VoiceIpcDirection = 0 | 1;
export type VoiceIpcOperation = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12;
export type VoiceIpcBodyKind = 0 | 1;
export type VoiceIpcHeader = Readonly<{
  direction: VoiceIpcDirection; operation: VoiceIpcOperation; kind: VoiceIpcBodyKind;
  slot: number; sequence: bigint; logical: string; length: number;
}>;
export const VOICE_IPC_HEADER_BYTES = 40;
export const VOICE_IPC_ADMISSION_CHARS = 144;
const MAX_SEQUENCE = (1n << 64n) - 1n;
const hex = (bytes: Uint8Array) => Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("");

export class VoiceIpcFrameError extends Error {
  constructor() { super("voice handoff header refused"); this.name = "VoiceIpcFrameError"; }
}
const refuse = (): never => { throw new VoiceIpcFrameError(); };

/** Closed direction × operation × body-class matrix, shared with native channel/crypto.rs.
 * Only Result requests and Input responses carry data. Input responses may instead report
 * fixed control metadata; Result requests cannot substitute a control body. */
function maximum(header: VoiceIpcHeader): number {
  if ((header.direction !== 0 && header.direction !== 1) ||
      !Number.isInteger(header.operation) || header.operation < 1 || header.operation > 12 ||
      (header.kind !== 0 && header.kind !== 1)) return refuse();
  if ((header.direction === 0 && header.operation === 3 && header.kind === 1) ||
      (header.direction === 1 && header.operation === 1 && header.kind === 1)) return 2064;
  if (header.kind === 0 && !(header.direction === 0 && header.operation === 3)) return 272;
  return refuse();
}

/** Fixed allocation, canonical big-endian u64 sequence. Never narrow sequences to JS number. */
export function encodeVoiceIpcHeader(header: VoiceIpcHeader): Uint8Array<ArrayBuffer> {
  const limit = maximum(header);
  if (!Number.isInteger(header.slot) || header.slot < 1 || header.slot > 64 ||
      typeof header.sequence !== "bigint" || header.sequence < 1n || header.sequence > MAX_SEQUENCE ||
      typeof header.logical !== "string" || !/^[0-9a-f]{32}$/.test(header.logical) ||
      !Number.isInteger(header.length) || header.length < 16 || header.length > limit) return refuse();
  const bytes = new Uint8Array(VOICE_IPC_HEADER_BYTES);
  bytes.set([0x4d, 0x57, 0x56, 0x48, 1, header.direction, header.operation, header.slot]);
  const view = new DataView(bytes.buffer);
  view.setBigUint64(8, header.sequence);
  for (let i = 0; i < 16; i++) bytes[16 + i] = Number.parseInt(header.logical.slice(i * 2, i * 2 + 2), 16);
  view.setUint32(32, header.length);
  bytes[36] = header.kind;
  return bytes;
}

/** Decode bounded canonical ASCII only. Returned fields are metadata, not verified admission.
 * Reject unknown/reserved fields by exact reconstruction, before any body allocation. */
export function decodeVoiceIpcAdmission(text: string): Readonly<{
  header: VoiceIpcHeader; bytes: Uint8Array<ArrayBuffer>; tag: Uint8Array<ArrayBuffer>;
}> {
  if (typeof text !== "string" || text.length !== VOICE_IPC_ADMISSION_CHARS || !/^[0-9a-f]+$/.test(text)) return refuse();
  const bytes = new Uint8Array(VOICE_IPC_HEADER_BYTES), tag = new Uint8Array(32);
  for (let i = 0; i < 72; i++) {
    const value = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
    if (i < 40) bytes[i] = value; else tag[i - 40] = value;
  }
  const view = new DataView(bytes.buffer);
  const header: VoiceIpcHeader = Object.freeze({
    direction: bytes[5] as VoiceIpcDirection, operation: bytes[6] as VoiceIpcOperation,
    slot: bytes[7], sequence: view.getBigUint64(8), logical: hex(bytes.subarray(16, 32)),
    length: view.getUint32(32), kind: bytes[36] as VoiceIpcBodyKind,
  });
  const canonical = encodeVoiceIpcHeader(header);
  if (canonical.some((byte, index) => byte !== bytes[index])) return refuse();
  return Object.freeze({ header, bytes, tag });
}

/** Assemble bounded framing without accepting arbitrary-length header/tag allocations. */
export function encodeVoiceIpcAdmission(header: VoiceIpcHeader, tag: Uint8Array): string {
  if (tag.byteLength !== 32) return refuse();
  return hex(encodeVoiceIpcHeader(header)) + hex(tag);
}

/** Direction-specific AEAD nonce. This helper does not allocate a sequence or authorize reuse. */
export function voiceIpcNonce(header: VoiceIpcHeader): Uint8Array<ArrayBuffer> {
  const encoded = encodeVoiceIpcHeader(header), nonce = new Uint8Array(12);
  nonce.set(header.direction === 0 ? [0x56, 0x48, 0x51, 0x31] : [0x56, 0x48, 0x50, 0x31]);
  nonce.set(encoded.subarray(8, 16), 4);
  return nonce;
}

/** Response MAC and AEAD bind the exact original physical request, not merely its logical row.
 * The caller still verifies the MAC, original session/position and replay floor independently. */
export function voiceIpcResponseAad(response: VoiceIpcHeader, request: VoiceIpcHeader): Uint8Array<ArrayBuffer> {
  if (response.direction !== 1 || request.direction !== 0 || response.slot !== request.slot ||
      response.operation !== request.operation || response.logical !== request.logical) return refuse();
  const bytes = new Uint8Array(VOICE_IPC_HEADER_BYTES * 2);
  bytes.set(encodeVoiceIpcHeader(response)); bytes.set(encodeVoiceIpcHeader(request), VOICE_IPC_HEADER_BYTES);
  return bytes;
}
