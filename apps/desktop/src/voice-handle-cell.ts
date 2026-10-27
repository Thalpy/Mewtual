/** Private detachable dispatch for caller-retained voice handles and queued callbacks.
 * Only the cell crosses a helper factory scope. Live callbacks/records stay in target;
 * detachment severs that whole graph and waits for every entered synchronous dispatch.
 * This is browser custody, never native authorization or proof of SDK/IO drainage. */
const returned = Promise.resolve();
export class VoiceHandleCell<T> {
  #target: T | null;
  #entered = 0;
  #detaching = false;
  #done: Promise<void> | null = null;
  #resolve: (() => void) | null = null;
  /** Authenticated public row metadata may survive compaction while independently paid.
   * The terminal owner clears it on final disposal; no capability/proof lives here. */
  row: string | null = null;
  constructor(target: T) { this.#target = target; }
  dispatch<R>(work: (target: T) => R, retired: R): R {
    const target = this.#target;
    if (this.#detaching || target === null) return retired;
    this.#entered++;
    try {
      const value = work(target);
      // A callback may request detachment reentrantly. Do not return a fresh live view
      // after that negative publication, even though this entered frame still pays it.
      return this.#detaching ? retired : value;
    } finally {
      this.#entered--;
      if (this.#detaching && this.#entered === 0) this.#finish();
    }
  }
  detach(): Promise<void> {
    if (this.#done) return this.#done;
    this.#detaching = true; // BEFORE any completion callback/microtask can reenter.
    if (this.#entered === 0) { this.#finish(); return returned; }
    this.#done = new Promise<void>(resolve => { this.#resolve = resolve; });
    return this.#done;
  }
  #finish(): void {
    this.#target = null;
    const resolve = this.#resolve; this.#resolve = null;
    resolve?.(); // Native Promise completion cannot run a consumer before this frame returns.
  }
  get detached(): boolean { return this.#detaching && this.#target === null; }
}
