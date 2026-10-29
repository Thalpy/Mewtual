/** Fixed public-metadata bridge for the admitted issuer profile. Keep this exact
 * object with the retained broker. This module's brand prevents accidental
 * adapter mixing; it is not an authorization boundary against same-realm JS.
 * Native still validates its original admitted lease and generation at each effect. */
import type { VoiceSelectedKeyAdmission, VoiceSelectedKeyPublication } from "./voice-crypto.ts";

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;
type Identity = Readonly<{ owner: string; executor: string; permit: string; join: string }>;
export type VoiceAdmittedKeyPublication =
  | (VoiceSelectedKeyPublication & Readonly<{ kind: "accepted" }>)
  | (Identity & Readonly<{ kind: "rejected"; code: "capacity" | "refused" | "stopped" }>)
  | (Identity & Readonly<{ kind: "closed"; code: "ended" | "unconfirmed" }>)
  | (Identity & Readonly<{ kind: "not-started"; code: "ended" }>);
export interface VoiceAdmittedSelectedKeyNative {
  admit(owner: string, executor: string): Promise<VoiceSelectedKeyAdmission>;
  publish(owner: string, executor: string, permit: string, point: Uint8Array): Promise<VoiceAdmittedKeyPublication>;
  settle(owner: string, executor: string): Promise<void>;
  acknowledge(owner: string, executor: string): Promise<void>;
}

// Same-window module reload must retain the ORIGINAL registry as well as broker.
// It owns no adapter/native work strongly. A closed, immutable versioned slot
// refuses unknown/accessor storage instead of creating a second brand registry.
// Symbol visibility does not authenticate hostile JS in this trusted realm.
// Adapter command/profile changes require a NEW registry version AND broker ABI;
// never reinterpret or replace a retained registry beside its original work.
const REGISTRY = Symbol.for("mewtual.voice.admitted-key-native-registry.v1");
/** Closed storage validation only; this neither registers an adapter nor grants
 * access to the original registry. Prototype-shaped impostors lack WeakSet slots. */
export function isVoiceAdmittedAdapterRegistry(value: unknown): value is WeakSet<object> {
  try {
    if (!value || typeof value !== "object" || Object.getPrototypeOf(value) !== WeakSet.prototype || Reflect.ownKeys(value).length) return false;
    WeakSet.prototype.has.call(value, WeakSet.prototype); return true;
  } catch { return false; }
}
function retainedAdapters(): WeakSet<object> {
  const host = globalThis as unknown as Record<symbol, unknown>;
  const previous = Object.getOwnPropertyDescriptor(host, REGISTRY);
  if (previous) {
    if (!("value" in previous) || previous.writable || previous.configurable || !isVoiceAdmittedAdapterRegistry(previous.value))
      throw new Error("admitted voice adapter registry changed; reload required");
    return previous.value as WeakSet<object>;
  }
  const registry = new WeakSet<object>();
  Object.defineProperty(host, REGISTRY, { value: registry, writable: false, configurable: false });
  return registry;
}
const adapters = retainedAdapters();
const containsAdapter = WeakSet.prototype.has, retainAdapter = WeakSet.prototype.add;
const actualByteLength = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(Uint8Array.prototype), "byteLength")!.get!;
function publicPointHex(point: Uint8Array): string {
  let width = 0;
  try { width = actualByteLength.call(point); } catch { /* A proxy is not an actual typed-array point. */ }
  if (!(point instanceof Uint8Array) || width !== 65 || point[0] !== 4) throw new Error("invalid admitted public point");
  const ascii = new Uint8Array(130), digits = "0123456789abcdef";
  // A subclass can shadow .length/.byteLength. Only actual backing width and
  // this literal protocol bound determine allocation/work, never those fields.
  for (let i = 0; i < 65; i++) {
    ascii[2 * i] = digits.charCodeAt(point[i] >> 4);
    ascii[2 * i + 1] = digits.charCodeAt(point[i] & 15);
  }
  return new TextDecoder().decode(ascii);
}
export function isVoiceAdmittedJoinKeyNative(value: unknown): value is VoiceAdmittedSelectedKeyNative {
  return typeof value === "object" && value !== null && containsAdapter.call(adapters, value);
}
export function voiceAdmittedJoinKeyNative(invoke: Invoke): VoiceAdmittedSelectedKeyNative {
  if (typeof invoke !== "function") throw new Error("invalid admitted native invoke");
  const identity = (owner: string, executor: string) => {
    if (typeof owner !== "string" || !owner || owner.length > 128 ||
        typeof executor !== "string" || !/^[0-9a-f]{64}$/.test(executor)) throw new Error("invalid admitted key identity");
  };
  const adapter = Object.freeze({
    admit: (owner: string, executor: string) => { identity(owner, executor); return invoke<VoiceSelectedKeyAdmission>("admit_admitted_voice_join_key", { owner, executor }); },
    publish: (owner: string, executor: string, permit: string, point: Uint8Array) => {
      identity(owner, executor);
      if (typeof permit !== "string" || !permit || permit.length > 128) throw new Error("invalid admitted key permit");
      return invoke<VoiceAdmittedKeyPublication>("publish_admitted_voice_join_key", { owner, executor, permit, publicKey: publicPointHex(point) });
    },
    settle: (owner: string, executor: string) => { identity(owner, executor); return invoke<void>("settle_admitted_voice_join_key", { owner, executorId: executor }); },
    acknowledge: (owner: string, executor: string) => { identity(owner, executor); return invoke<void>("acknowledge_admitted_voice_key_publication", { owner, executorId: executor }); },
  });
  retainAdapter.call(adapters, adapter);
  return adapter;
}
