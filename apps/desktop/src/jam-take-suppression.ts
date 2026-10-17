/** Authenticated deck identity. Pause/offset/DJ heartbeat changes are not another track. */
export type JamDeckTrack = Readonly<{ entry: string; cid: string; src: string; link: string }>;

/**
 * Receiver-local archival cancellation, including tracks whose file is still being approved or
 * downloaded. The CID need not have seated an engine transport yet. Ordinary media never asks
 * this gate, but its actual track change clears the old take's suppression as well.
 */
export class JamTakeSuppression {
  private track: JamDeckTrack | null = null;
  // During a replacement the outgoing transport's CID and the adopted, still-loading deck CID
  // differ. Cancellation covers both until a real track change or a local gesture.
  private readonly suppressed = new Set<string>();

  reconcile(next: JamDeckTrack | null): void {
    const previous = this.track;
    const same = previous === null ? next === null : next !== null && previous.entry === next.entry &&
      previous.cid === next.cid && previous.src === next.src && previous.link === next.link;
    if (!same) this.suppressed.clear();
    this.track = next ? { entry: next.entry, cid: next.cid, src: next.src, link: next.link } : null;
  }
  suppress(activeCid: string | null, currentDeckCid: string | null): void {
    if (activeCid) this.suppressed.add(activeCid);
    if (currentDeckCid) this.suppressed.add(currentDeckCid);
  }
  allows(cid: string): boolean { return !this.suppressed.has(cid); }
  localGesture(): void { this.suppressed.clear(); }
  reset(): void { this.track = null; this.suppressed.clear(); }
}
