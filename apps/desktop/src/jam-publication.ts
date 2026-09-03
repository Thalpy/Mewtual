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
    return this.tryEnqueue(task) ?? Promise.reject(new JamCausalQueueOverflow());
  }

  /** Attempt admission synchronously so a caller can revoke retained work at the overflow edge. */
  tryEnqueue<T>(task: () => T | Promise<T>): Promise<T> | null {
    if (this.pending >= this.pendingMax) return null;
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

export type JamInitialPublicationAdmission = "live" | "queued" | "overflow";

/**
 * Bounded barrier for gestures admitted before the first immutable patch finishes hashing.
 *
 * Queued gestures may still feed local rendering/recording, but `flush` marks them ineligible for
 * outbound transmission. A data channel can open during the digest; replaying this queue to that
 * newly ready edge would otherwise recreate the receiver-budget burst this barrier prevents.
 */
export class JamInitialPublicationGate<P> {
  private pending: Array<(publication: P, outbound: boolean) => void> = [];
  private readonly pendingMax: number;

  constructor(pendingMax: number) {
    if (!Number.isInteger(pendingMax) || pendingMax < 1) {
      throw new RangeError("invalid initial jam publication bound");
    }
    this.pendingMax = pendingMax;
  }

  submit(
    publication: P | null,
    event: (publication: P, outbound: boolean) => void,
  ): JamInitialPublicationAdmission {
    if (publication !== null) {
      event(publication, true);
      return "live";
    }
    if (this.pending.length >= this.pendingMax) {
      this.pending = [];
      return "overflow";
    }
    this.pending.push(event);
    return "queued";
  }

  flush(publication: P): void {
    const waiting = this.pending.splice(0);
    for (const event of waiting) event(publication, false);
  }

  clear(): void {
    this.pending = [];
  }
}

/**
 * A causal queue whose overflow retires every closure retained behind its active task.
 *
 * The active asynchronous operation may need its own cancellation seam, supplied by `onOverflow`.
 * Everything that has not begun observes the advanced generation and becomes a no-op. Fresh work
 * enters a replacement queue immediately instead of sitting behind the obsolete active promise.
 */
export class JamResettableCausalQueue {
  private queue: JamCausalQueue;
  private generation = 0;
  private readonly pendingMax: number;

  constructor(pendingMax: number) {
    this.pendingMax = pendingMax;
    this.queue = new JamCausalQueue(pendingMax);
  }

  enqueue<T>(task: () => T | Promise<T>, onOverflow: () => void): Promise<T | null> {
    const admittedGeneration = this.generation;
    const result = this.queue.tryEnqueue(() =>
      admittedGeneration === this.generation ? task() : null);
    if (result) return result;

    this.generation += 1;
    this.queue = new JamCausalQueue(this.pendingMax);
    onOverflow();
    return Promise.resolve(null);
  }

  /** Retire this call's retained closures even when it did not reach the overflow bound. */
  reset(): void {
    this.generation += 1;
    this.queue = new JamCausalQueue(this.pendingMax);
  }
}

/**
 * One outbound data-channel generation's publication barrier.
 *
 * Musical edges are transient, so events produced before `open` are intentionally not retained.
 * Replaying seconds of performance history at once would collapse sender pacing into a burst that
 * can exhaust receiver frame/attack/patch buckets and separate a note-on from its note-off. Open
 * establishes only the current publication; subsequent live events keep the ordinary prerequisite
 * barrier. A future sustain-across-connect feature must use a fresh bounded state snapshot.
 */
export class JamOutboundEdge {
  private ready = false;
  private announcedKey: string | null = null;

  event(publication: JamPublishedFrame, frame: string, send: (frame: string) => boolean): boolean {
    if (!this.ready) return false;
    return this.ensureAnnouncement(publication, send) && send(frame);
  }

  /** Publish a patch immediately only after the edge's initial barrier has opened. */
  publication(publication: JamPublishedFrame, send: (frame: string) => boolean): void {
    if (this.ready) this.ensureAnnouncement(publication, send);
  }

  open(current: JamPublishedFrame, send: (frame: string) => boolean): void {
    if (this.ready) return;
    this.ready = true;
    this.ensureAnnouncement(current, send);
  }

  close(): void {
    this.ready = false;
    this.announcedKey = null;
  }

  private ensureAnnouncement(publication: JamPublishedFrame, send: (frame: string) => boolean): boolean {
    if (this.announcedKey === publication.key) return true;
    if (!send(publication.announce)) return false;
    this.announcedKey = publication.key;
    return true;
  }
}
