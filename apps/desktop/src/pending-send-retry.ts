/** One session-scoped wake for unresolved sealed intents, never one timer per message. */
export const PENDING_RETRY_INITIAL_MS = 5_000;
export const PENDING_RETRY_MAX_MS = 60_000;

export type RetryClock = {
  setTimeout(callback: () => void, delay: number): unknown;
  clearTimeout(handle: unknown): void;
};

const browserClock: RetryClock = {
  setTimeout: (callback, delay) => setTimeout(callback, delay),
  clearTimeout: handle => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export class PendingSendRetry {
  private run: (session: number) => Promise<void>;
  private clock: RetryClock;
  private session: number | null = null;
  private generation = 0;
  private hasWork = false;
  private busy = false;
  private running = false;
  private timer: unknown;
  private delay = PENDING_RETRY_INITIAL_MS;

  constructor(run: (session: number) => Promise<void>, clock: RetryClock = browserClock) {
    this.run = run;
    this.clock = clock;
  }

  /** Repeated updates leave an existing deadline alone; error text is deliberately not input. */
  update(session: number | null, hasWork: boolean, busy: boolean): void {
    if (session !== this.session) {
      this.cancel();
      this.session = session;
    }
    this.hasWork = hasWork;
    this.busy = busy;
    if (!hasWork) this.delay = PENDING_RETRY_INITIAL_MS;
    if (session === null || !hasWork || busy) this.clearTimer();
    else this.arm();
  }

  /** Invalidate callbacks already queued by the host as well as the outstanding timer. */
  cancel(): void {
    this.generation++;
    this.clearTimer();
    this.session = null;
    this.hasWork = false;
    this.busy = false;
    this.running = false;
    this.delay = PENDING_RETRY_INITIAL_MS;
  }

  private clearTimer(): void {
    if (this.timer !== undefined) this.clock.clearTimeout(this.timer);
    this.timer = undefined;
  }

  private arm(): void {
    if (this.timer !== undefined || this.running || this.session === null || !this.hasWork || this.busy) return;
    const generation = this.generation;
    const session = this.session;
    this.timer = this.clock.setTimeout(() => {
      if (generation !== this.generation) return;
      this.timer = undefined;
      if (this.session !== session || !this.hasWork || this.busy || this.running) return;
      this.running = true;
      // The submission path reports failures and retains exact identities. A refresh/IPC
      // exception must still release the single pass and schedule the next bounded retry.
      void this.run(session).catch(() => {}).finally(() => {
        if (generation !== this.generation) return;
        this.running = false;
        this.delay = this.hasWork ? Math.min(this.delay * 2, PENDING_RETRY_MAX_MS) : PENDING_RETRY_INITIAL_MS;
        this.arm();
      });
    }, this.delay);
  }
}
