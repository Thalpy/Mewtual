/** Authenticated deck identity. Pause/offset/DJ heartbeat changes are not another track. */
export type JamDeckTrack = Readonly<{ entry: string; cid: string; src: string; link: string }>;

/**
 * Receiver-local archival cancellation, including tracks whose file is still being approved or
 * downloaded. The CID need not have seated an engine transport yet. Ordinary media never asks
 * this gate, but its actual track change clears the old take's suppression as well.
 */
export class JamTakeSuppression {
  private track: JamDeckTrack | null = null;
  private suppressed: string | null = null;

  reconcile(next: JamDeckTrack | null): void {
    const previous = this.track;
    const same = previous === null ? next === null : next !== null && previous.entry === next.entry &&
      previous.cid === next.cid && previous.src === next.src && previous.link === next.link;
    if (!same) this.suppressed = null;
    this.track = next ? { entry: next.entry, cid: next.cid, src: next.src, link: next.link } : null;
  }
  suppress(activeCid: string | null, currentDeckCid: string | null): void {
    this.suppressed = activeCid || currentDeckCid || this.suppressed;
  }
  allows(cid: string): boolean { return cid !== this.suppressed; }
  localGesture(): void { this.suppressed = null; }
  reset(): void { this.track = null; this.suppressed = null; }
}
