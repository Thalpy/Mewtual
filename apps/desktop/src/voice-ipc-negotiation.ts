/** Local Session mode/capacity codecs only. Decode AFTER the original retained
 * channel authenticates response MAC/AEAD against the exact physical request.
 * These objects grant no Ready, peer/role, ledger, SDK, SDP or RTC authority. */
export type VoiceNegotiationKind = "register" | "observe" | "cancel";
export type VoiceModeReceipt = Readonly<{ state: "unselected" | "pinned" | "canceled" | "closed" }>;
export type VoiceCapacityState = "unstarted" | "preparing" | "waiting" | "reserved" | "closing" |
  "returned" | "closed" | "uncertain" | "superseded";
export type VoiceCapacityReceipt = Readonly<{
  state: VoiceCapacityState; generation: string; canonical: string; floor: string; incarnation: string | null;
}>;
const MAX = (1n << 64n) - 1n;
const refuse = (): never => { throw new Error("invalid voice negotiation custody"); };
const typedArrayPrototype = Object.getPrototypeOf(Uint8Array.prototype);
const nativeLength = Object.getOwnPropertyDescriptor(typedArrayPrototype, "byteLength")!.get!;
const nativeOffset = Object.getOwnPropertyDescriptor(typedArrayPrototype, "byteOffset")!.get!;
const nativeBuffer = Object.getOwnPropertyDescriptor(typedArrayPrototype, "buffer")!.get!;
function view(bytes: Uint8Array, length: number): DataView {
  // Public helpers may receive a genuine typed array with shadowed properties,
  // subarray/species or iterators. Read only its intrinsic fixed view and numeric
  // positions; never let a caller substitute bytes outside the authenticated body.
  if (!(bytes instanceof Uint8Array) || Reflect.apply(nativeLength, bytes, []) !== length) return refuse();
  const buffer = Reflect.apply(nativeBuffer, bytes, []);
  if (!(buffer instanceof ArrayBuffer)) return refuse();
  return new DataView(buffer, Reflect.apply(nativeOffset, bytes, []), length);
}
function ordinal(value: string): bigint {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value)) return refuse();
  const number = BigInt(value); if (number > MAX) return refuse(); return number;
}
function session(value: string): string {
  if (typeof value !== "string" || !/^[0-9a-f]{32}$/.test(value)) return refuse();
  return value;
}
function incarnation(value: string): string {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) return refuse();
  return value;
}
function request(version: 10 | 11, kind: VoiceNegotiationKind, generation: bigint): Uint8Array<ArrayBuffer> {
  const mode = kind === "register" ? 0 : kind === "observe" ? 1 : kind === "cancel" ? 2 : refuse();
  const body = new Uint8Array(10); body[0] = version; body[1] = mode;
  new DataView(body.buffer).setBigUint64(2, generation); return body;
}
export function encodeVoiceAdmissionMode(physicalSession: string, kind: VoiceNegotiationKind):
  Readonly<{ operation: 29; kind: 0; logical: string; body: Uint8Array<ArrayBuffer> }> {
  session(physicalSession);
  return Object.freeze({ operation: 29, kind: 0, logical: physicalSession, body: request(10, kind, 1n) });
}
export function decodeVoiceAdmissionMode(bytes: Uint8Array): VoiceModeReceipt {
  const data = view(bytes, 18);
  if (data.getUint8(0) !== 10 || data.getUint8(1) > 3) return refuse();
  const generation = data.getBigUint64(2), floor = data.getBigUint64(10), state = data.getUint8(1);
  if (generation > 1n || floor > generation || state === 0 && (generation !== 0n || floor !== 0n) ||
      state === 1 && (generation !== 1n || floor !== 0n) ||
      state === 2 && (generation !== 1n || floor !== 1n)) return refuse();
  return Object.freeze({ state: (["unselected", "pinned", "canceled", "closed"] as const)[state] });
}
export function encodeVoiceNegotiationCapacity(physicalSession: string, kind: VoiceNegotiationKind, generation: string):
  Readonly<{ operation: 30; kind: 0; logical: string; body: Uint8Array<ArrayBuffer> }> {
  session(physicalSession);
  return Object.freeze({ operation: 30, kind: 0, logical: physicalSession, body: request(11, kind, ordinal(generation)) });
}
/** Preserve a remembered incarnation at the retained owner, not by mutating it
 * from this snapshot. Negative replies may omit a destroyed owner; a changed
 * present identity is never accepted, even on a negative response. */
export function decodeVoiceNegotiationCapacity(bytes: Uint8Array,
  expected: Readonly<{ generation: string; incarnation?: string | null }>): VoiceCapacityReceipt {
  // Caller getters may reenter or substitute values. Capture each primitive
  // once before validation; never validate one identity and compare another.
  const requested = expected.generation, priorIncarnation = expected.incarnation;
  const wanted = ordinal(requested);
  const remembered = priorIncarnation == null ? null : incarnation(priorIncarnation);
  const data = view(bytes, 59);
  if (data.getUint8(0) !== 11 || data.getUint8(1) > 8 || data.getUint8(26) > 1) return refuse();
  const echoed = data.getBigUint64(2), canonical = data.getBigUint64(10), floor = data.getBigUint64(18);
  if (echoed !== wanted || floor > canonical || wanted > canonical && (canonical === MAX || wanted !== canonical + 1n || floor !== canonical)) return refuse();
  const states = ["unstarted", "preparing", "waiting", "reserved", "closing", "returned", "closed", "uncertain", "superseded"] as const;
  const state = states[data.getUint8(1)], present = data.getUint8(26) === 1;
  let copied = "";
  for (let index = 27; index < 59; index++) {
    const byte = data.getUint8(index);
    if (!present && byte !== 0) return refuse();
    if (present) copied += byte.toString(16).padStart(2, "0");
  }
  const identity = present ? copied : null;
  const negative = state === "closing" || state === "returned" || state === "closed" || state === "uncertain" || state === "superseded";
  if (remembered !== null && (identity !== null ? identity !== remembered : !negative)) return refuse();
  if ((state === "unstarted" || state === "preparing") && wanted < canonical ||
      state === "waiting" && (wanted !== canonical || floor >= wanted) ||
      state === "reserved" && (wanted !== canonical || floor >= wanted || !present) ||
      state === "returned" && (wanted !== canonical || floor !== wanted) ||
      state === "closed" && present ||
      state === "superseded" && (wanted >= canonical || floor < wanted)) return refuse();
  // Copy primitives only; neither caller identity objects nor unrelated graphs
  // may survive into the small retained receipt. This function authenticates nothing.
  return Object.freeze({ state, generation: echoed.toString(), canonical: canonical.toString(), floor: floor.toString(), incarnation: identity });
}
