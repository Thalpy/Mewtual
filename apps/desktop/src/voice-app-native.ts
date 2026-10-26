/** Stable production composition. Ordinary IPC carries only fixed public owner/key
 * metadata; all byte-bearing action work stays in the retained authenticated channel. */
import { windowVoiceCryptoBroker, type VoiceCryptoAdmission } from "./voice-crypto.ts";
import { voiceJoinKeyNative } from "./voice-join-native.ts";
import { voiceIpcNative } from "./voice-ipc-native.ts";
import type { VoiceAppActivation, VoiceAppCalls, VoiceAppNative } from "./voice-app-session.ts";

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;
const unavailable = async (): Promise<never> => { throw new Error("generic voice crypto is unavailable in the native call session"); };
// The selected-key/private-channel path never calls these compatibility methods.
// In particular there is no plaintext commitInbound command or renderer delivery API.
const privateOnly: VoiceCryptoAdmission = Object.freeze({ admit: unavailable, settle: unavailable, commitInbound: unavailable });
export function voiceAppNative(invoke: Invoke): VoiceAppNative {
  return Object.freeze({
    prepareController: () => invoke<string>("prepare_voice_controller", {}),
    prepareOwner: (controller: string, server: number, room: string) => invoke<string>("prepare_voice_owner", { controller, server, room }),
    activate: (candidate: string) => invoke<VoiceAppActivation>("activate_voice_owner", { candidate }),
    end: (owner: string) => invoke<void>("end_voice_owner", { owner }),
  });
}
const SLOT = Symbol.for("mewtual.voice.app-session.v1");
type Retained = Readonly<{ version: 1; invoke: Invoke; calls: VoiceAppCalls }>;
/** Remounts reuse the exact façade, adapters, broker, call and pending native work.
 * Replacing invoke or an incompatible retained implementation requires a reload. */
export function windowVoiceAppCalls(invoke: Invoke, host: object = globalThis): VoiceAppCalls {
  const previous = (host as Record<symbol, Retained | undefined>)[SLOT];
  if (Object.hasOwn(host, SLOT)) {
    if (!previous || previous.version !== 1 || previous.invoke !== invoke || !Object.isFrozen(previous) ||
        !previous.calls || !Object.isFrozen(previous.calls) || typeof previous.calls.prepare !== "function" ||
        typeof previous.calls.resume !== "function" || typeof previous.calls.close !== "function")
      throw new Error("voice application session changed; reload required");
    return previous.calls;
  }
  const calls = windowVoiceCryptoBroker(privateOnly, host).nativeAppCalls(voiceAppNative(invoke), voiceJoinKeyNative(invoke), voiceIpcNative(invoke));
  Object.defineProperty(host, SLOT, { value: Object.freeze({ version: 1, invoke, calls }) });
  return calls;
}
