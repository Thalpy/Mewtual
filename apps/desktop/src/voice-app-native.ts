/** Stable production composition. Ordinary IPC carries only fixed public owner/key
 * metadata; all byte-bearing action work stays in the retained authenticated channel. */
import { windowVoiceCryptoBroker, type VoiceCryptoAdmission, type VoiceCryptoBroker } from "./voice-crypto.ts";
import { voiceJoinKeyNative } from "./voice-join-native.ts";
import { voiceIpcNative } from "./voice-ipc-native.ts";
import type { VoiceAppActivation, VoiceAppCalls, VoiceAppNative } from "./voice-app-session.ts";
import type { VoiceRtcDeviceSettingsSource } from "./voice-rtc-configuration.ts";

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
type Retained = Readonly<{ version: 3; invoke: Invoke; broker: VoiceCryptoBroker; calls: VoiceAppCalls;
  configurationSource: VoiceRtcDeviceSettingsSource | null }>;
/** Remounts reuse the exact façade, adapters, broker, call and pending native work.
 * Replacing invoke or an incompatible retained implementation requires a reload. */
export function windowVoiceAppCalls(invoke: Invoke, host: object = globalThis,
  configurationSource: VoiceRtcDeviceSettingsSource | null = null): VoiceAppCalls {
  if (configurationSource !== null && typeof configurationSource !== "function")
    throw new Error("invalid voice RTC settings source");
  const previous = (host as Record<symbol, Retained | undefined>)[SLOT];
  if (Object.hasOwn(host, SLOT)) {
    const broker = previous?.broker, calls = previous?.calls, settings = previous?.configurationSource;
    if (!previous || previous.version !== 3 || previous.invoke !== invoke || settings !== configurationSource || !Object.isFrozen(previous) ||
        !broker || !calls || !Object.isFrozen(calls) || typeof calls.prepare !== "function" ||
        typeof calls.resume !== "function" || typeof calls.close !== "function")
      throw new Error("voice application session changed; reload required");
    // Do not let this outer cache bypass the retained broker's own version/API
    // fence. Another allocation must never replace a pool with unresolved work.
    if (windowVoiceCryptoBroker(privateOnly, host, undefined, broker) !== broker)
      throw new Error("voice application broker changed; reload required");
    return calls;
  }
  const broker = windowVoiceCryptoBroker(privateOnly, host);
  const native = voiceAppNative(invoke), selected = voiceJoinKeyNative(invoke), ipc = voiceIpcNative(invoke);
  const calls = configurationSource
    ? broker.nativeConfiguredAppCalls(native, selected, ipc, configurationSource)
    : broker.nativeAppCalls(native, selected, ipc);
  Object.defineProperty(host, SLOT, { value: Object.freeze({ version: 3, invoke, broker, calls, configurationSource }) });
  return calls;
}
