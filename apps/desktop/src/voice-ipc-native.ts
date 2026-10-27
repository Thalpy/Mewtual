/** Fixed public metadata only. Use windowVoiceIpcNative at application composition so HMR
 * and component remounts reuse the exact adapter captured by the retained broker. No secret,
 * action bytes, Session constructors, drain flags, or configurable endpoint crosses here. */
import type { VoiceIpcNative, VoiceIpcSetupReply } from "./voice-crypto.ts";

type Invoke = <T>(command: string, args: Record<string, unknown>) => Promise<T>;

export function voiceIpcNative(invoke: Invoke): VoiceIpcNative {
  return Object.freeze({
    setup: (owner: string, executor: string) =>
      invoke<VoiceIpcSetupReply>("setup_voice_channel", { owner, executor }),
    settleSetup: (owner: string, executor: string) =>
      invoke<void>("settle_voice_channel_setup", { owner, executor }),
    collectChannel: (owner: string, executor: string) =>
      invoke<void>("collect_voice_channel", { owner, executor }),
    readWork: (owner: string, executor: string, fromSlot: number) =>
      invoke<unknown>("read_voice_channel_work", { owner, executor, fromSlot }),
  });
}

const ADAPTER = Symbol.for("mewtual.voice.ipc-native.v1");
type Retained = Readonly<{ version: 2; invoke: Invoke; native: VoiceIpcNative }>;

/** The broker compares adapter identity as well as actual selected-key ownership. Keep the
 * original invoke and adapter together; an incompatible retained transport requires reload,
 * never a second adapter beside unresolved native/browser work. */
export function windowVoiceIpcNative(invoke: Invoke, host: object = globalThis): VoiceIpcNative {
  const previous = (host as Record<symbol, Retained | undefined>)[ADAPTER];
  if (Object.hasOwn(host, ADAPTER)) {
    if (!previous || typeof previous !== "object" || previous.version !== 2 || previous.invoke !== invoke || !Object.isFrozen(previous) ||
        !previous.native || typeof previous.native !== "object" || !Object.isFrozen(previous.native) || typeof previous.native.setup !== "function" ||
        typeof previous.native.settleSetup !== "function" || typeof previous.native.collectChannel !== "function" || typeof previous.native.readWork !== "function") {
      throw new Error("voice channel transport changed; reload required");
    }
    return previous.native;
  }
  const native = voiceIpcNative(invoke);
  Object.defineProperty(host, ADAPTER, { value: Object.freeze({ version: 2, invoke, native }) });
  return native;
}
