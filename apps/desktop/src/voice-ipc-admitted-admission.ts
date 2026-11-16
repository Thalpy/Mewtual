/** Closed operation31/version12 only. Decode AFTER the original admitted
 * Session authenticates MAC/AEAD and the exact request. These are historical
 * native capacity observations, never Ready, SDK return or call permission. */
export type VoiceAdmittedAdmissionKind = "setup-ack" | "register" | "observe" | "cancel";
export type VoiceAdmittedAdmissionState = "unstarted" | "preparing" | "waiting" | "reserved" |
  "closing" | "returned" | "closed" | "uncertain" | "superseded";
export type VoiceAdmittedAdmissionReceipt = Readonly<{
  kind: VoiceAdmittedAdmissionKind; state: VoiceAdmittedAdmissionState;
  generation: string; canonical: string; floor: string; incarnation: string | null;
}>;
const MAX = (1n << 64n) - 1n;
const states = ["unstarted", "preparing", "waiting", "reserved", "closing", "returned", "closed", "uncertain", "superseded"] as const;
const refuse = (): never => { throw new Error("invalid admitted voice capacity observation"); };
const proto = Object.getPrototypeOf(Uint8Array.prototype);
const lengthOf = Object.getOwnPropertyDescriptor(proto, "byteLength")!.get!;
const offsetOf = Object.getOwnPropertyDescriptor(proto, "byteOffset")!.get!;
const bufferOf = Object.getOwnPropertyDescriptor(proto, "buffer")!.get!;
function data(bytes: Uint8Array): DataView {
  // Ignore shadowed properties/species/iterators; proxies have no internal slot.
  if (!(bytes instanceof Uint8Array) || Reflect.apply(lengthOf, bytes, []) !== 59) return refuse();
  const buffer = Reflect.apply(bufferOf, bytes, []);
  if (!(buffer instanceof ArrayBuffer)) return refuse();
  return new DataView(buffer, Reflect.apply(offsetOf, bytes, []), 59);
}
function ordinal(value: string, zero: boolean): bigint {
  if (typeof value !== "string" || !(zero ? /^0$/ : /^[1-9][0-9]{0,19}$/).test(value)) return refuse();
  const number = BigInt(value); if (number > MAX) return refuse(); return number;
}
function tag(kind: VoiceAdmittedAdmissionKind): number {
  return kind === "setup-ack" ? 0 : kind === "register" ? 1 : kind === "observe" ? 2 : kind === "cancel" ? 3 : refuse();
}
export function encodeVoiceAdmittedAdmission(session: string, kind: VoiceAdmittedAdmissionKind, generation: string):
  Readonly<{ operation: 31; kind: 0; logical: string; body: Uint8Array<ArrayBuffer> }> {
  if (typeof session !== "string" || !/^[0-9a-f]{32}$/.test(session) || /^0+$/.test(session)) return refuse();
  const mode = tag(kind), value = ordinal(generation, mode === 0);
  const body = new Uint8Array(10); body[0] = 12; body[1] = mode;
  new DataView(body.buffer).setBigUint64(2, value);
  return Object.freeze({ operation: 31, kind: 0, logical: session, body });
}
/** SetupAck has its OWN g0 rule: never run it through the old v11 validator.
 * Remembered identities are immutable at the owner; even a negative reply may
 * omit but may never replace one. An all-zero PRESENT identity stays present. */
export function decodeVoiceAdmittedAdmission(bytes: Uint8Array,
  expected: Readonly<{ kind: VoiceAdmittedAdmissionKind; generation: string; incarnation?: string | null }>): VoiceAdmittedAdmissionReceipt {
  const kind = expected.kind, generation = expected.generation, previous = expected.incarnation;
  const mode = tag(kind), wanted = ordinal(generation, mode === 0);
  if (previous != null && (typeof previous !== "string" || !/^[0-9a-f]{64}$/.test(previous))) return refuse();
  const remembered = previous ?? null, view = data(bytes);
  if (view.getUint8(0) !== 12 || view.getUint8(1) > 8 || view.getUint8(26) > 1) return refuse();
  const state = states[view.getUint8(1)], echoed = view.getBigUint64(2), canonical = view.getBigUint64(10), floor = view.getBigUint64(18);
  if (echoed !== wanted || floor > canonical) return refuse();
  if (mode !== 0 && wanted > canonical && (canonical === MAX || wanted !== canonical + 1n || floor !== canonical)) return refuse();
  const present = view.getUint8(26) === 1;
  let copied = "";
  for (let index = 27; index < 59; index++) {
    const byte = view.getUint8(index);
    if (!present && byte !== 0) return refuse();
    if (present) copied += byte.toString(16).padStart(2, "0");
  }
  const incarnation = present ? copied : null;
  const negative = state === "closing" || state === "returned" || state === "closed" || state === "uncertain" || state === "superseded";
  if (remembered !== null && (incarnation !== null ? incarnation !== remembered : !negative)) return refuse();
  if (canonical === 0n && present || state === "closed" && present) return refuse();
  // ACK releases setup custody only. Its requested g0 is intentionally not a
  // claim about a later canonical generation already running on this Session.
  if (mode !== 0 && (
    (state === "unstarted" || state === "preparing") && wanted < canonical ||
    state === "waiting" && (wanted !== canonical || floor >= wanted) ||
    state === "reserved" && (wanted !== canonical || floor >= wanted || !present) ||
    state === "returned" && (wanted !== canonical || floor !== wanted) ||
    state === "superseded" && (wanted >= canonical || floor < wanted)
  )) return refuse();
  return Object.freeze({ kind, state, generation: echoed.toString(), canonical: canonical.toString(), floor: floor.toString(), incarnation });
}
