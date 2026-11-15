/** Fixed admitted physical-purpose adapter. The retained module brand prevents
 * accidental profile substitution; native remains the authorization boundary.
 * No private key, seed, SDP or ICE crosses these metadata commands. */
import { voiceAdmittedJoinKeyNative, type VoiceAdmittedSelectedKeyNative } from "./voice-admitted-join-native.ts";
import type { VoiceIpcSetupReply } from "./voice-crypto.ts";

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;
export type VoiceAdmittedPhysicalObservation =
  | Readonly<{ kind: "no-physical-intent"; owner: string }>
  | Readonly<{ kind: "pending"; owner: string; executor: string; join: string }>
  | Readonly<{ kind: "not-started"; owner: string; executor: string; join: string; receipt: string }>
  | Readonly<{ kind: "collected"; owner: string; executor: string; join: string; receipt: string; session: string }>;
export interface VoiceAdmittedPhysicalNative extends VoiceAdmittedSelectedKeyNative {
  setup(owner: string, executor: string): Promise<VoiceIpcSetupReply>;
  settleSetup(owner: string, executor: string): Promise<void>;
  collectChannel(owner: string, executor: string): Promise<void>;
  observe(owner: string, executor: string): Promise<VoiceAdmittedPhysicalObservation>;
  acknowledgePhysical(owner: string, executor: string, receipt: string): Promise<void>;
  acknowledgeParent(owner: string): Promise<void>;
  end(owner: string): Promise<void>;
}

const SLOT = Symbol.for("mewtual.voice.admitted-physical-native-registry.v1");
const host = globalThis as unknown as Record<symbol, unknown>;
const descriptor = Object.getOwnPropertyDescriptor(host, SLOT);
let registry: WeakSet<object>;
if (descriptor) {
  const value = "value" in descriptor ? descriptor.value : null;
  let valid = false;
  try {
    valid = !!value && typeof value === "object" && Object.getPrototypeOf(value) === WeakSet.prototype && !Reflect.ownKeys(value).length;
    if (valid) WeakSet.prototype.has.call(value, WeakSet.prototype);
  } catch { valid = false; }
  if (!valid || descriptor.writable || descriptor.configurable) throw new Error("physical voice adapter registry changed; reload required");
  registry = value as WeakSet<object>;
} else {
  registry = new WeakSet<object>();
  Object.defineProperty(host, SLOT, { value: registry, writable: false, configurable: false });
}
const contains = WeakSet.prototype.has, add = WeakSet.prototype.add;
export function isVoiceAdmittedPhysicalNative(value: unknown): value is VoiceAdmittedPhysicalNative {
  return typeof value === "object" && value !== null && contains.call(registry, value);
}

export function voiceAdmittedPhysicalNative(invoke: Invoke): VoiceAdmittedPhysicalNative {
  if (typeof invoke !== "function") throw new Error("invalid physical native invoke");
  // Reuse the SAME bounded public-point encoder and fixed publication/settlement
  // contract. Its temporary key-only adapter is never exposed or retained by a
  // physical key record; this new object has ONLY the distinct physical brand.
  const key = voiceAdmittedJoinKeyNative(invoke);
  const identity = (owner: string, executor: string) => {
    if (typeof owner !== "string" || !owner || owner.length > 64 ||
        typeof executor !== "string" || !/^[0-9a-f]{64}$/.test(executor)) throw new Error("invalid physical voice identity");
  };
  const adapter = Object.freeze({
    admit: (owner: string, executor: string) => { identity(owner, executor); return invoke<Awaited<ReturnType<VoiceAdmittedSelectedKeyNative["admit"]>>>("admit_admitted_physical_voice_join_key", { owner, executor }); },
    publish: key.publish, settle: key.settle, acknowledge: key.acknowledge,
    setup: (owner: string, executor: string) => { identity(owner, executor); return invoke<VoiceIpcSetupReply>("setup_admitted_voice_channel", { owner, executor }); },
    settleSetup: (owner: string, executor: string) => { identity(owner, executor); return invoke<void>("settle_admitted_voice_channel_setup", { owner, executor }); },
    collectChannel: (owner: string, executor: string) => { identity(owner, executor); return invoke<void>("collect_admitted_voice_channel", { owner, executor }); },
    observe: (owner: string, executor: string) => { identity(owner, executor); return invoke<VoiceAdmittedPhysicalObservation>("observe_admitted_voice_channel", { owner, executor }); },
    acknowledgePhysical: (owner: string, executor: string, receipt: string) => {
      identity(owner, executor);
      if (typeof receipt !== "string" || !/^[0-9a-f]{64}$/.test(receipt)) throw new Error("invalid physical voice receipt");
      return invoke<void>("acknowledge_admitted_voice_channel", { owner, executor, receipt });
    },
    acknowledgeParent: (owner: string) => {
      if (typeof owner !== "string" || !owner || owner.length > 64) throw new Error("invalid physical voice owner");
      return invoke<void>("acknowledge_admitted_voice_activation", { owner });
    },
    end: (owner: string) => {
      if (typeof owner !== "string" || !owner || owner.length > 64) throw new Error("invalid physical voice owner");
      return invoke<void>("end_voice_owner", { owner });
    },
  });
  add.call(registry, adapter);
  return adapter;
}
