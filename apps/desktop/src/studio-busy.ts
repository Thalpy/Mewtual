// Bounded automatic retry of Studio requests that native refused BEFORE executing them.
//
// Native `authorize()` (apps/desktop/src-tauri/src/studio.rs) never waits for a vault guard once
// the actor has advertised Ready: if any guard (per-server persist lock, UI-session commit, store,
// registry) is held by other native work at that instant, the request is refused with
// `STUDIO_STORAGE_BUSY`, every guard is released and the actor resumes. With a single person
// editing, that other work is routinely the app's own bookkeeping: the full server snapshot that
// native seals after every Studio save, the UI-continuity write, a chat read. Each of those holds
// a guard for tens to hundreds of milliseconds, and the renderer's follow-up reads and the next
// autosave land squarely inside that window. Surfacing every such refusal as a save card or a
// status banner made the composer unusable, so the session repeats the exact same request after
// a short, bounded, growing pause. The request is byte-identical (same nonce, epoch and body):
// a write that was refused here never began, and an identical retry of an accepted write is
// recognised by native as already done. The schedule is short on purpose: this is not a queue
// for a wedged vault, and after the last pause the refusal surfaces exactly as before.
//
// This is the first of two layers. What surfaces after the last pause reaches the session, whose
// Save worker and document/Index reads resend every native no-write refusal again after longer
// backoffs (BUSY_BACKOFF_MS and resendLater in studio-session.ts). Which refusals count as "nothing
// was written" is decided there, by `nativeNoWrite`, not here; studio-native-no-write.test.ts pins
// its texts to their Rust sources, and studio-busy.test.ts checks this file's text is among them.

import type { StudioIpc } from "./studio-native.ts";

/** Exact native text; `authorize()` in src-tauri/src/studio.rs is its only producer. */
export const STUDIO_STORAGE_BUSY = "Studio storage busy; retry the same request";
/** The actor did not reach Ready inside native's bounded wait; raised before execution too, but
 *  never repeated here (see busyRetrying). */
export const STUDIO_ACTOR_BUSY = "Studio actor busy; retry";

/** Pauses before each repeat, in order; their sum (1.24 s) bounds the longest silent wait. */
export const STUDIO_BUSY_RETRY_MS: readonly number[] = [40, 80, 160, 320, 640];

/** The text native sent, whether it arrived as a bare string (Tauri) or wrapped in an Error. */
export function nativeErrorText(e: unknown): string {
  if (e instanceof Error) return e.message;
  return typeof e === "string" ? e : String(e);
}

/**
 * Wrap a bridge so each `invoke` repeats the same command and arguments after a storage-busy
 * refusal, pausing per `STUDIO_BUSY_RETRY_MS` through the session's timer seam. Retrying stops
 * when `generation()` no longer returns what it did at the first attempt: a reset or scope change
 * must not keep an old request alive. Only `STUDIO_STORAGE_BUSY` is repeated; the actor-busy
 * refusal already waited five seconds inside native and is reported at once.
 */
export function busyRetrying(
  ipc: StudioIpc,
  schedule: (fn: () => void, ms: number) => () => void,
  generation: () => number,
  pauses: readonly number[] = STUDIO_BUSY_RETRY_MS,
): StudioIpc {
  return {
    listen: (name, handler) => ipc.listen(name, handler),
    invoke: async <T>(command: string, args: Record<string, unknown> = {}): Promise<T> => {
      const started = generation();
      for (let attempt = 0; ; attempt++) {
        try {
          return await ipc.invoke<T>(command, args);
        } catch (e) {
          if (nativeErrorText(e) !== STUDIO_STORAGE_BUSY || attempt >= pauses.length || generation() !== started) throw e;
          await new Promise<void>((resolve) => { schedule(resolve, pauses[attempt]); });
          if (generation() !== started) throw e;
        }
      }
    },
  };
}
