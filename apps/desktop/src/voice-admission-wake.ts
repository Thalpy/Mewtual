import type { VoiceIpcClock } from "./voice-crypto.ts";

type TimerCell = { work: ((generation: object) => void) | null };
// Outside the coordinator/registration scope: a queued callback left after a
// successful cancel retains only this detachable capsule and an inert marker.
function timerCallback(cell: TimerCell, generation: object): () => void {
  return () => cell.work?.(generation);
}

/** One prepaid admission cadence. Publication must precede start()/wake(): Clock
 * callbacks may reenter either. A register-then-throw is unknown custody, not an
 * absent timer; only that original callback can join it. Cancel failure retains
 * the SAME handle. Never replace either debt with a second registration.
 * The enclosing Session's separate8KiB coordinator pays this whole graph. */
export class VoiceAdmissionWake {
  #clock: VoiceIpcClock | null;
  #started = false;
  #done = false;
  #woken = false;
  #reached = false;
  #due = 0;
  #last = -Infinity;
  #handle: unknown = null;
  #known = false;
  #unknown = false;
  #generation: object | null = null;
  #arming = false;
  #installing = false;
  #cancelling = false;
  #inline = false;
  #entered = 0;
  #faulted = false;
  readonly #milliseconds: number;
  #onFault: (() => void) | null;
  readonly #cell: TimerCell = { work: null };
  readonly #returned: Promise<void>;
  #resolve: (() => void) | null = null;

  constructor(clock: VoiceIpcClock, milliseconds = 1_000, onFault: (() => void) | null = null) {
    this.#clock = clock;
    this.#milliseconds = milliseconds;
    this.#onFault = onFault;
    this.#returned = new Promise(resolve => { this.#resolve = resolve; });
    this.#cell.work = generation => this.#onTimer(generation);
  }
  join(): Promise<void> { return this.#returned; }
  get faulted(): boolean { return this.#faulted; }
  get elapsed(): boolean { return this.#reached; }
  start(): void {
    if (this.#started || this.#done) return;
    this.#started = true;
    // An invalid clock cannot manufacture elapsed cadence. Retain until an
    // explicit availability/End wake; do not spin through native requests.
    try {
      const began = this.#now(); this.#due = began + this.#milliseconds;
      if (!Number.isFinite(this.#due) || this.#due <= began) { this.#fault(); return; }
      this.#arm();
    } catch { this.#fault(); /* Explicit wake can join a proven unregistered wait. */ }
  }
  wake(): void {
    if (this.#done) return;
    this.#woken = true; // Before Clock.cancel, which may synchronously reenter.
    try { this.#cancel(); } catch { this.#fault(); /* Same known registration stays paid. */ }
    this.#finish();
  }
  #now(): number {
    const now = this.#clock!.now();
    if (!Number.isFinite(now) || now < this.#last) throw new Error("invalid voice admission clock");
    this.#last = now; return now;
  }
  #fault(): void {
    if (this.#faulted) return;
    this.#faulted = true;
    // Notification is NOT return. A hanging IO owner must abort promptly while
    // this SAME timer/entered callback remains joined and independently paid.
    const observer = this.#onFault; this.#onFault = null;
    try { observer?.(); } catch { /* Fault/ownership stay latched even if an observer throws. */ }
  }
  #cancel(): void {
    if (!this.#known || this.#cancelling) return;
    this.#cancelling = true;
    try {
      this.#clock!.cancel(this.#handle);
      this.#known = false; this.#handle = null;
    } finally { this.#cancelling = false; }
  }
  #arm(): void {
    if (this.#done || this.#arming || this.#installing || this.#cancelling) return;
    this.#arming = true;
    try {
      if (this.#woken) return;
      const remaining = this.#due - this.#now();
      if (this.#woken) return;
      if (remaining <= 0) { this.#reached = true; return; }
      const generation = {}; this.#generation = generation;
      this.#unknown = true; this.#installing = true; this.#inline = false;
      try {
        this.#handle = this.#clock!.schedule(timerCallback(this.#cell, generation), remaining);
        this.#known = true; this.#unknown = false;
      } catch {
        // No handle means no safe cancellation. Wait for this exact callback;
        // a concurrent End cannot collect/refund its retained owner meanwhile.
        this.#woken = true; this.#fault();
      } finally { this.#installing = false; }
      // Inline delivery is not elapsed cadence: cancel its handle but keep the
      // wait until explicit availability/End. Otherwise the OUTER RPC loop spins.
      if (this.#inline) this.#fault();
      if (this.#woken || this.#faulted) this.#cancel();
    } catch { this.#fault(); /* Keep exact registration; wake retries cancel. */ }
    finally { this.#arming = false; this.#finish(); }
  }
  #onTimer(generation: object): void {
    if (this.#done || this.#generation !== generation) return;
    this.#entered++;
    try {
      if (this.#installing) { this.#inline = true; this.#unknown = false; return; }
      this.#known = false; this.#unknown = false; this.#handle = null;
      if (!this.#woken) this.#reached = this.#now() >= this.#due;
    } catch { this.#fault(); }
    finally {
      this.#entered--;
      if (!this.#woken && !this.#faulted && !this.#reached && !this.#installing) this.#arm();
      this.#finish();
    }
  }
  #finish(): void {
    if (this.#done || this.#arming || this.#installing || this.#cancelling || this.#entered !== 0 ||
        this.#known || this.#unknown || !this.#woken && !this.#reached) return;
    this.#done = true; this.#cell.work = null; this.#generation = null; this.#clock = null; this.#onFault = null;
    const resolve = this.#resolve; this.#resolve = null;
    resolve?.(); // Native Promise continuations run after the actual callback frame returns.
  }
}
