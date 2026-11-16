/** One physical authenticated-IPC execution position, shared by paid media monitors and
 * one serial negotiation. Queueing grants no native/RTC authority. Each caller already
 * pays its actual closure/provider graph; this fixed queue owns only one small wake per
 * admitted waiter. Fixed request bodies may already exist before queueing and remain
 * caller-paid; framing/encryption/response work waits for the actual turn. A lost observer
 * or local End cannot retire the actual running work. */
export const VOICE_SHARED_WAITERS = 18; // At most16 Root peer incarnations + negotiation/control.
type Wake = { promise: Promise<void>; resolve(): void };
const wake = (): Wake => {
  let resolve!: () => void;
  const promise = new Promise<void>(yes => { resolve = yes; });
  return { promise, resolve };
};
export class VoiceSharedExecution {
  #held = false;
  #waiting: Wake[] = [];
  readonly #custody: Readonly<{ begin(): void; end(): void }>;
  readonly #limit: number;
  constructor(custody: Readonly<{ begin(): void; end(): void }>, limit = VOICE_SHARED_WAITERS) {
    if (!Number.isInteger(limit) || limit < 0 || limit > VOICE_SHARED_WAITERS) throw new Error("invalid voice execution bound");
    this.#custody = custody; this.#limit = limit;
  }
  async run<T>(work: () => Promise<T>): Promise<T> {
    if (this.#held) {
      if (this.#waiting.length >= this.#limit) throw new Error("voice channel execution capacity reached");
      const waiting = wake(); this.#waiting.push(waiting);
      await waiting.promise;
    } else this.#held = true;
    let entered = false;
    try {
      // These are trusted local accounting callbacks, never SDK/provider work. They
      // belong inside the release finally so even an exception cannot strand the lane.
      this.#custody.begin(); entered = true;
      return await work();
    } finally {
      try { if (entered) this.#custody.end(); }
      finally {
        // Transfer directly to the oldest admitted owner. Keep held=true between turns;
        // a reentrant/new caller must queue behind it rather than steal its position.
        const next = this.#waiting.shift();
        if (next) next.resolve(); else this.#held = false;
      }
    }
  }
}
