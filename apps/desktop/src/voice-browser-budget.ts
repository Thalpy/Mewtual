/** Local application-owned voice buffers only, not total browser/RTC/OS memory.
 * These limits never alter a packet, action, peer, execution or native Root limit.
 * A device setting may choose the policy BEFORE constructing the retained broker;
 * live calls and their unresolved cleanup always keep the original ledger/policy. */
export type VoiceBrowserBudget = Readonly<{ groupBytes: number; processBytes: number }>;

export const VOICE_BROWSER_PROCESS_BYTES = 2 * 1024 * 1024;
export const VOICE_BROWSER_DEFAULT_GROUP_KIB = 512;
export const VOICE_BROWSER_MIN_GROUP_KIB = 256;
export const VOICE_BROWSER_MAX_GROUP_KIB = 1024;

const invalid = (): never => { throw new Error("invalid voice browser memory budget"); };

/** Bounded local-device policy seam. Future settings persistence must pass through
 * this validation; incoming peer/server metadata is never a source for this value.
 * Apply a changed selection on reload, not by replacing an occupied byte ledger. */
export function voiceBrowserBudget(groupKiB: number = VOICE_BROWSER_DEFAULT_GROUP_KIB): VoiceBrowserBudget {
  if (!Number.isSafeInteger(groupKiB) || groupKiB < VOICE_BROWSER_MIN_GROUP_KIB ||
      groupKiB > VOICE_BROWSER_MAX_GROUP_KIB) return invalid();
  return Object.freeze({ groupBytes: groupKiB * 1024, processBytes: VOICE_BROWSER_PROCESS_BYTES });
}

export const DEFAULT_VOICE_BROWSER_BUDGET = voiceBrowserBudget();

/** Capture caller fields once before ledger/job/SDK construction. Keep an owned
 * immutable primitive policy, never the caller object or its getters. The aggregate
 * process ceiling is deliberately fixed even when a local group budget is changed. */
export function copyVoiceBrowserBudget(input: VoiceBrowserBudget): VoiceBrowserBudget {
  if (!input || typeof input !== "object") return invalid();
  const groupBytes = input.groupBytes, processBytes = input.processBytes;
  if (processBytes !== VOICE_BROWSER_PROCESS_BYTES || !Number.isSafeInteger(groupBytes) || groupBytes % 1024 !== 0)
    return invalid();
  return voiceBrowserBudget(groupBytes / 1024);
}
