/** Pure metadata parser for the disabled native local-channel setup. It creates no keys,
 * Session or counter allocator. Only the retained broker's actual selected-key record may
 * consume this transcript from trusted native IPC; caller-supplied metadata is no authority. */
export type VoiceIpcBootstrapExpected = Readonly<{
  owner: string; executor: string; join: string; group: string; point: Uint8Array;
}>;
export type VoiceIpcBootstrap = Readonly<{
  session: string; slot: number; generation: bigint; server: bigint; instance: bigint;
  rawGroup: string; group: string; policy: string | null; room: bigint; device: string;
  owner: string; executor: string; join: string;
  nativePoint: Uint8Array<ArrayBuffer>; transcript: Uint8Array<ArrayBuffer>;
}>;
const DOMAIN = new TextEncoder().encode("Mewtual native voice bootstrap\0");
const ADMITTED_DOMAIN = new TextEncoder().encode("Mewtual admitted native voice bootstrap\0");
export type VoiceAdmittedIpcBootstrap = VoiceIpcBootstrap & Readonly<{ callProfile: "admitted" }>;
const hex = (data: Uint8Array) => Array.from(data, byte => byte.toString(16).padStart(2, "0")).join("");
const refused = (): never => { throw new Error("voice channel setup refused"); };

/** Canonical bounded transcript, independent of JavaScript's lossy number representation.
 * Both points retain their fixed uncompressed encoding; WebCrypto must still validate the
 * native curve point before ECDH. This parser alone authenticates neither endpoint. */
export function parseVoiceIpcBootstrap(text: string, expected: VoiceIpcBootstrapExpected): VoiceIpcBootstrap {
  return parseBootstrap(text, expected, false);
}

/** New physical bootstrap grammar only. This public metadata parser creates no
 * key/Session/role/ACK permission and cannot convert an Existing selected key.
 * A future retained admitted-key factory must perform actual ECDH separately. */
export function parseAdmittedVoiceIpcBootstrap(text: string, expected: VoiceIpcBootstrapExpected): VoiceAdmittedIpcBootstrap {
  return parseBootstrap(text, expected, true) as VoiceAdmittedIpcBootstrap;
}
function parseBootstrap(text: string, expected: VoiceIpcBootstrapExpected, admitted: boolean): VoiceIpcBootstrap | VoiceAdmittedIpcBootstrap {
  if (typeof text !== "string" || text.length === 0 || text.length > 2048 || text.length % 2 || !/^[0-9a-f]+$/.test(text)) return refused();
  // Actual room is u128BE (16 bytes), not an eight-byte JavaScript integer.
  if (admitted && (text.length < 365 * 2 || text.length > 779 * 2)) return refused();
  const transcript = new Uint8Array(text.length / 2);
  for (let index = 0; index < transcript.length; index++) transcript[index] = Number.parseInt(text.slice(index * 2, index * 2 + 2), 16);
  let offset = 0;
  const take = (length: number): Uint8Array<ArrayBuffer> => {
    if (offset + length > transcript.length) return refused();
    const part = transcript.subarray(offset, offset + length); offset += length; return part;
  };
  const integer = (length: number): bigint => {
    let value = 0n; for (const byte of take(length)) value = (value << 8n) | BigInt(byte); return value;
  };
  const domain = admitted ? ADMITTED_DOMAIN : DOMAIN;
  if (hex(take(domain.length)) !== hex(domain) || integer(1) !== (admitted ? 2n : 1n) || integer(1) !== 1n ||
      (admitted && integer(1) !== 1n)) return refused();
  const session = hex(take(16)), slot = Number(integer(1));
  if (/^0+$/.test(session) || slot < 1 || slot > 64) return refused();
  const generation = integer(8), server = integer(8), instance = integer(8);
  const groupLength = Number(integer(2));
  if (groupLength < 1 || groupLength > 256) return refused();
  const rawGroup = hex(take(groupLength)), group = hex(take(32));
  const pinned = integer(1);
  if (pinned !== 0n && pinned !== 1n) return refused();
  const policy = pinned === 1n ? hex(take(32)) : null;
  const room = integer(16), device = hex(take(32)), join = hex(take(32));
  const ownerLength = Number(integer(2));
  if (ownerLength < 1 || ownerLength > 128) return refused();
  let owner: string;
  // Preserve an initial BOM: stripping it would equate different encoded transcript bytes
  // with the selected owner. Native encodes the exact UTF-8 bytes without normalization.
  try { owner = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(take(ownerLength)); } catch { return refused(); }
  const executor = hex(take(32)), browser = take(65), native = take(65);
  if (offset !== transcript.length || browser[0] !== 4 || native[0] !== 4 ||
      owner !== expected.owner || executor !== expected.executor || join !== expected.join ||
      group !== expected.group || expected.point.length !== 65 ||
      browser.some((byte, index) => byte !== expected.point[index])) return refused();
  // One result object in either profile; do not retain/copy an intermediate
  // metadata object when the future admitted channel funds this parser frame.
  if (admitted) return Object.freeze({ session, slot, generation, server, instance, rawGroup, group, policy, room, device,
    owner, executor, join, nativePoint: native.slice(), transcript, callProfile: "admitted" as const });
  return Object.freeze({ session, slot, generation, server, instance, rawGroup, group, policy, room, device,
    owner, executor, join, nativePoint: native.slice(), transcript });
}
