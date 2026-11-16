/** Fixed public bootstrap adapter. Private keys and byte-bearing crypto results never use these
 * commands. Keep this adapter with the retained broker; remounts must reuse its exact object. */
import type { VoiceSelectedKeyAdmission, VoiceSelectedKeyNative, VoiceSelectedKeyPublication } from "./voice-crypto.ts";

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;
export function voiceJoinKeyNative(invoke: Invoke): VoiceSelectedKeyNative {
  return Object.freeze({
    admit: (owner: string, executor: string) => invoke<VoiceSelectedKeyAdmission>("admit_voice_join_key", { owner, executor }),
    publish: (owner: string, executor: string, permit: string, point: Uint8Array) =>
      invoke<VoiceSelectedKeyPublication>("publish_voice_join_key", { owner, executor, permit,
        publicKey: [...point].map(n => n.toString(16).padStart(2, "0")).join("") }),
    settle: (owner: string, executor: string) => invoke<void>("settle_voice_join_key", { owner, executorId: executor }),
  });
}
