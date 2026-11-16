/** Pure operation31 framing for a separately keyed admitted physical Session.
 * No keys, sequence allocator, transport, SDK closure or profile conversion lives
 * here. The original retained driver must verify MAC/AEAD and all native fences. */
export type VoiceAdmittedIpcHeader = Readonly<{
  direction: 0 | 1; operation: 31; kind: 0; slot: number;
  sequence: bigint; logical: string; length: number;
}>;
const MAX = (1n << 64n) - 1n;
const refuse = (): never => { throw new Error("invalid admitted voice frame"); };
const proto = Object.getPrototypeOf(Uint8Array.prototype);
const lengthOf = Object.getOwnPropertyDescriptor(proto, "byteLength")!.get!;
const offsetOf = Object.getOwnPropertyDescriptor(proto, "byteOffset")!.get!;
const bufferOf = Object.getOwnPropertyDescriptor(proto, "buffer")!.get!;
function intrinsic(bytes: Uint8Array, size: number): DataView {
  if (!(bytes instanceof Uint8Array) || Reflect.apply(lengthOf, bytes, []) !== size) return refuse();
  const buffer = Reflect.apply(bufferOf, bytes, []);
  if (!(buffer instanceof ArrayBuffer)) return refuse();
  return new DataView(buffer, Reflect.apply(offsetOf, bytes, []), size);
}
function hex(bytes: Uint8Array, size: number): string {
  const view = intrinsic(bytes, size); let text = "";
  for (let index = 0; index < size; index++) text += view.getUint8(index).toString(16).padStart(2, "0");
  return text;
}
export function encodeVoiceAdmittedIpcHeader(header: VoiceAdmittedIpcHeader): Uint8Array<ArrayBuffer> {
  // Snapshot primitive inputs once. These public labels never supply authority.
  const { direction, operation, kind, slot, sequence, logical, length } = header;
  if ((direction !== 0 && direction !== 1) || operation !== 31 || kind !== 0 ||
      !Number.isInteger(slot) || slot < 1 || slot > 64 || typeof sequence !== "bigint" || sequence < 1n || sequence > MAX ||
      typeof logical !== "string" || !/^[0-9a-f]{32}$/.test(logical) || /^0+$/.test(logical) ||
      length !== (direction === 0 ? 26 : 75)) return refuse();
  const bytes = new Uint8Array(40), data = new DataView(bytes.buffer);
  bytes.set([0x4d, 0x57, 0x56, 0x48, 1, direction, 31, slot]);
  data.setBigUint64(8, sequence);
  for (let index = 0; index < 16; index++) bytes[16 + index] = Number.parseInt(logical.slice(index * 2, index * 2 + 2), 16);
  data.setUint32(32, length); // kind/reserved suffix is the closed all-zero encoding
  return bytes;
}
export function encodeVoiceAdmittedIpcAdmission(header: VoiceAdmittedIpcHeader, tag: Uint8Array): string {
  return hex(encodeVoiceAdmittedIpcHeader(header), 40) + hex(tag, 32);
}
export function decodeVoiceAdmittedIpcAdmission(text: string): Readonly<{
  header: VoiceAdmittedIpcHeader; bytes: Uint8Array<ArrayBuffer>; tag: Uint8Array<ArrayBuffer>;
}> {
  if (typeof text !== "string" || text.length !== 144 || !/^[0-9a-f]+$/.test(text)) return refuse();
  const bytes = new Uint8Array(40), tag = new Uint8Array(32);
  for (let index = 0; index < 72; index++) {
    const byte = Number.parseInt(text.slice(index * 2, index * 2 + 2), 16);
    if (index < 40) bytes[index] = byte; else tag[index - 40] = byte;
  }
  const data = new DataView(bytes.buffer);
  const header: VoiceAdmittedIpcHeader = Object.freeze({ direction: bytes[5] as 0 | 1, operation: bytes[6] as 31,
    kind: bytes[36] as 0, slot: bytes[7], sequence: data.getBigUint64(8), logical: hex(bytes.subarray(16, 32), 16), length: data.getUint32(32) });
  const canonical = encodeVoiceAdmittedIpcHeader(header);
  for (let index = 0; index < 40; index++) if (bytes[index] !== canonical[index]) return refuse();
  return Object.freeze({ header, bytes, tag });
}
export function voiceAdmittedIpcNonce(header: VoiceAdmittedIpcHeader): Uint8Array<ArrayBuffer> {
  const bytes = encodeVoiceAdmittedIpcHeader(header), nonce = new Uint8Array(12);
  nonce.set(bytes[5] === 0 ? [0x56, 0x48, 0x51, 0x31] : [0x56, 0x48, 0x50, 0x31]);
  nonce.set(bytes.subarray(8, 16), 4); return nonce;
}
export function voiceAdmittedIpcResponseAad(response: VoiceAdmittedIpcHeader, request: VoiceAdmittedIpcHeader): Uint8Array<ArrayBuffer> {
  const reply = encodeVoiceAdmittedIpcHeader(response), original = encodeVoiceAdmittedIpcHeader(request);
  if (reply[5] !== 1 || original[5] !== 0 || reply[7] !== original[7]) return refuse();
  for (let index = 16; index < 32; index++) if (reply[index] !== original[index]) return refuse();
  const bytes = new Uint8Array(80); bytes.set(reply); bytes.set(original, 40); return bytes;
}
