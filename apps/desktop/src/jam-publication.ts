/** A fully verified patch announcement that an event may depend on. */
export type JamPublishedFrame = Readonly<{
  /** Session nonce plus patch id; sufficient to identify the exact prerequisite announcement. */
  key: string;
  announce: string;
}>;

/**
 * Serialises asynchronous receive-side work without allowing one rejection to wedge the lane.
 * RTCDataChannel ordering stops at delivery; this queue extends it across WebCrypto awaits.
 */
export class JamCausalQueue {
  private tail: Promise<void> = Promise.resolve();
  private pending = 0;
  private readonly pendingMax: number;

  constructor(pendingMax = Number.MAX_SAFE_INTEGER) {
    if (!Number.isInteger(pendingMax) || pendingMax < 1) throw new RangeError("invalid jam causal pending bound");
    this.pendingMax = pendingMax;
  }

  enqueue<T>(task: () => T | Promise<T>): Promise<T> {
    if (this.pending >= this.pendingMax) return Promise.reject(new JamCausalQueueOverflow());
    this.pending += 1;
    const result = this.tail.then(task);
    this.tail = result.then(() => undefined, () => undefined);
    void result.then(
      () => { this.pending -= 1; },
      () => { this.pending -= 1; },
    );
    return result;
  }
}

export class JamCausalQueueOverflow extends Error {
  constructor() {
    super("jam causal queue overflow");
    this.name = "JamCausalQueueOverflow";
  }
}

type LatestTask<T> = {
  task: () => T | Promise<T>;
  resolve: (value: T | null) => void;
  reject: (reason: unknown) => void;
};

/**
 * Runs one asynchronous task and retains at most one newer replacement.
 *
 * Patch-editor updates are state, not an event log: while WebCrypto is hashing an older snapshot,
 * intermediate drafts have no value. Superseded callers resolve to null immediately, which keeps
 * both retained closures and recovery work O(1) without turning normal churn into rejections.
 */
export class JamLatestTaskQueue<T> {
  private running = false;
  private pending: LatestTask<T> | null = null;

  submit(task: () => T | Promise<T>): Promise<T | null> {
    return new Promise<T | null>((resolve, reject) => {
      const next = { task, resolve, reject };
      if (!this.running) {
        this.running = true;
        this.execute(next);
        return;
      }
      this.pending?.resolve(null);
      this.pending = next;
    });
  }

  private execute(item: LatestTask<T>): void {
    void Promise.resolve()
      .then(item.task)
      .then(item.resolve, item.reject)
      .finally(() => {
        const next = this.pending;
        this.pending = null;
        if (next) this.execute(next);
        else this.running = false;
      });
  }
}

/** Monotonic token preventing an obsolete patch digest from publishing over a newer editor draft. */
export class JamPublicationGeneration {
  private value = 0;

  current(): number {
    return this.value;
  }

  advance(): number {
    this.value += 1;
    return this.value;
  }

  isCurrent(generation: number): boolean {
    return generation === this.value;
  }
}

/** Deterministic sender-side pacing for distinct global recipe publications. */
export class JamPublicationPacer {
  private lastMs: number | null = null;
  private readonly minIntervalMs: number;

  constructor(minIntervalMs: number) {
    if (!Number.isFinite(minIntervalMs) || minIntervalMs < 0) throw new RangeError("invalid jam publication interval");
    this.minIntervalMs = minIntervalMs;
  }

  remaining(nowMs: number): number {
    if (!Number.isFinite(nowMs)) return this.minIntervalMs;
    if (this.lastMs === null) return 0;
    return Math.max(0, this.minIntervalMs - (nowMs - this.lastMs));
  }

  publish(nowMs: number): boolean {
    if (this.remaining(nowMs) > 0) return false;
    this.lastMs = nowMs;
    return true;
  }

  reset(): void {
    this.lastMs = null;
  }
}

type PendingEvent = Readonly<{ publication: JamPublishedFrame; frame: string }>;

/**
 * One outbound data-channel generation's publication barrier.
 *
 * Events produced before `open` retain the exact published patch they referenced. Opening sends
 * each prerequisite announcement before its dependent event, even if a newer patch finished
 * hashing in the meantime. Overflow drops the whole unopened transient rather than retaining an
 * unbounded queue or dropping only note-offs and stranding a chord.
 */
export class JamOutboundEdge {
  private readonly pendingMax: number;
  private ready = false;
  private announcedKey: string | null = null;
  private pending: PendingEvent[] = [];
  private overflowed = false;

  constructor(pendingMax: number) {
    if (!Number.isInteger(pendingMax) || pendingMax < 1) throw new RangeError("invalid jam edge pending bound");
    this.pendingMax = pendingMax;
  }

  event(publication: JamPublishedFrame, frame: string, send: (frame: string) => boolean): boolean {
    if (this.ready) {
      return this.ensureAnnouncement(publication, send) && send(frame);
    }
    if (this.overflowed) return false;
    if (this.pending.length >= this.pendingMax) {
      this.pending = [];
      this.overflowed = true;
      return false;
    }
    this.pending.push({ publication, frame });
    return true;
  }

  /** Publish a patch immediately only after the edge's initial barrier has opened. */
  publication(publication: JamPublishedFrame, send: (frame: string) => boolean): void {
    if (this.ready) this.ensureAnnouncement(publication, send);
  }

  open(current: JamPublishedFrame, send: (frame: string) => boolean): void {
    if (this.ready) return;
    this.ready = true;
    if (!this.overflowed) {
      for (const event of this.pending) {
        // A failed prerequisite must never be followed by its dependent event. Drop the rest of
        // this unopened transient as a unit; a later live event retries the current announcement.
        if (!this.ensureAnnouncement(event.publication, send) || !send(event.frame)) break;
      }
    }
    this.pending = [];
    this.overflowed = false;
    this.ensureAnnouncement(current, send);
  }

  close(): void {
    this.ready = false;
    this.announcedKey = null;
    this.pending = [];
    this.overflowed = false;
  }

  private ensureAnnouncement(publication: JamPublishedFrame, send: (frame: string) => boolean): boolean {
    if (this.announcedKey === publication.key) return true;
    if (!send(publication.announce)) return false;
    this.announcedKey = publication.key;
    return true;
  }
}
