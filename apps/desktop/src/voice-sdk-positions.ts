/** One bounded FIFO for the browser's second provider position. The caller already pays
 * the original action/phase metadata. A queued request owns no provider position or native
 * authority; cancellation is legal only before handoff. A granted loan is returned solely by
 * the actual SDK/digest finally, even when its observer or owning call has retired. */
export type VoiceSdkPositionRequest = Readonly<{
  promise: Promise<(() => void) | null>;
  cancel(): void;
}>;
type Waiting = { resolve(loan: (() => void) | null): void; pending: boolean };
export class VoiceSdkPositions {
  private readonly waiting: Waiting[] = [];
  private active = false;
  private pumping = false;
  private readonly acquire: () => (() => void) | null;
  constructor(acquire: () => (() => void) | null) { this.acquire = acquire; }
  request(): VoiceSdkPositionRequest {
    if (this.waiting.length >= 18) throw new Error("voice SDK position queue full");
    let resolve!: Waiting["resolve"];
    const promise = new Promise<(() => void) | null>(yes => { resolve = yes; });
    const entry = { resolve, pending: true };
    this.waiting.push(entry);
    this.pump();
    return Object.freeze({ promise, cancel: () => {
      if (!entry.pending) return; // Started work must return its actual affine loan.
      entry.pending = false;
      const index = this.waiting.indexOf(entry);
      if (index !== -1) this.waiting.splice(index, 1);
      entry.resolve(null);
      this.pump();
    } });
  }
  /** Existing broker position returns call this; no polling or new native admission. */
  pump(): void {
    if (this.pumping || this.active || !this.waiting.length) return;
    this.pumping = true;
    try {
      const release = this.acquire();
      if (!release) return;
      const entry = this.waiting.shift()!;
      entry.pending = false; this.active = true;
      entry.resolve(sdkPositionReturn(this, release));
    } finally { this.pumping = false; }
  }
  /** Private coordinator boundary; the separately scoped return handle captures neither
   * the resolved queue entry nor any original SDK input, selector, plan or browser owner. */
  returned(release: () => void): void {
    this.active = false;
    release();
    this.pump();
  }
}
function sdkPositionReturn(positions: VoiceSdkPositions, release: () => void): () => void {
  let held = true;
  return () => { if (held) { held = false; positions.returned(release); } };
}
