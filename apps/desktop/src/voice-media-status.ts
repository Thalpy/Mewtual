/** Passive metadata for a future independently paid media/status driver. This stores no
 * Clock, timer, callback, peer, channel, old chain, promise or native capability. It does
 * not authenticate replies or execute revocation; the retained driver must do both. */
import type { VoiceMediaReceipt } from "./voice-ipc-media.ts";
/** Native IPC refresh grace, independent of the original SDP Source retirement deadline. */
export const VOICE_MEDIA_STALE_MS = 5000;
const refused = (): never => { throw new Error("original media status unavailable"); };

/** Conservative absolute deadline for a fractional monotonic browser clock. Adding
 * directly can round UP even at ordinary values (3192.004 + 5000), accidentally refusing
 * healthy peers in a strict delta check. Floor first: ordinary clock magnitudes lose
 * under one millisecond, never gain time. Huge accepted clocks can round down further;
 * the final represented-delta check still rejects unsafe upward/non-advancing results. */
export function voiceMediaDeadline(now: number): number | null {
  if (!Number.isFinite(now)) return null;
  const until = Math.floor(now) + VOICE_MEDIA_STALE_MS;
  return Number.isFinite(until) && until > now && until - now <= VOICE_MEDIA_STALE_MS ? until : null;
}

export class VoiceMediaStatus {
  readonly #proof: string;
  #closed = false;
  #current = false;
  #last = -Infinity;
  #until = 0;
  #flight: object | null = null;
  #attemptUntil = 0;
  #completing = false;
  constructor(proof: string) {
    if (typeof proof !== "string" || !/^[0-9a-f]{64}$/.test(proof)) refused();
    this.#proof = proof;
  }
  get closed(): boolean { return this.#closed; }
  get pending(): boolean { return this.#flight !== null; }
  get deadline(): number | null { return this.#current && !this.#closed ? this.#until : null; }
  /** Close is irreversible and does NOT make an outstanding actual RPC drained. */
  close(): void { this.#closed = true; }
  #observe(now: number): void {
    if (!Number.isFinite(now) || now < this.#last) this.#closed = true;
    else this.#last = now;
    if (this.#current && now >= this.#until) this.#closed = true;
  }
  currentAt(now: number): boolean {
    this.#observe(now); return this.#current && !this.#closed;
  }
  /** Publish one opaque attempt BEFORE its physical request/provider callbacks. The
   * successful reply may use only this start-anchored bound, never reply-time + grace.
   * Caller owns the actual clock seam, fixed record payment and one shared-session lane. */
  begin(now: number): object {
    this.#observe(now);
    if (this.#closed || this.#flight || this.#completing) return refused();
    const until = voiceMediaDeadline(now);
    if (until === null) { this.close(); return refused(); }
    const token = Object.freeze({});
    this.#attemptUntil = until; this.#flight = token; return token;
  }
  /** Actual physical completion only. null denotes a retryable refusal/uncertain reply;
   * it cannot extend an older observation. The driver must pass the SAME authenticated
   * request's immutable decoded receipt, not a renderer's current/connected assertion.
   * Historical admission alone never starts or refreshes a valid media observation. */
  finish(token: object, receipt: VoiceMediaReceipt | null, now: number): void {
    if (this.#flight !== token || this.#completing) return refused();
    this.#completing = true;
    try {
      // Snapshot external metadata before the final private state updates. Unexpected
      // getter reentry cannot finish twice, start another flight or erase a close latch.
      const proof = receipt?.proof;
      const valid = receipt !== null && receipt.admitted && receipt.current && !receipt.closed && !receipt.released;
      this.#observe(now);
      if (receipt !== null && proof !== this.#proof) this.#closed = true;
      if (this.#closed) return;
      if (receipt === null) return;
      if (!valid || now >= this.#attemptUntil) { this.#closed = true; return; }
      this.#until = this.#attemptUntil; this.#current = true;
    } finally {
      // A thrown malformed provider also cannot strand an entered completion forever.
      // Its driver retains the separate physical fee through actual callback return.
      this.#flight = null; this.#completing = false;
    }
  }
}
