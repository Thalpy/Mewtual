/** Browser bookkeeping for the SAME protected pair. This object grants no native effect,
 * admission, SDP or RTC authority. Only the original authenticated Finish Input creates it;
 * native Claim independently checks the original plan and current session. */
import type { VoiceTransitionReceipt } from "./voice-ipc-transition.ts";

export class VoiceKeyCustody {
  private limit: number;
  private observed: VoiceTransitionReceipt | null = null;
  private refused = false;
  constructor(confirmationExpires: number, now: number) {
    if (!Number.isFinite(now) || !Number.isFinite(confirmationExpires) || now >= confirmationExpires ||
        confirmationExpires + 30_000 > Number.MAX_SAFE_INTEGER) throw new Error("expired voice confirmation");
    // One provisional ceiling, set before SDK or delayed-result callbacks. It permits
    // custody only; the original confirmationExpires continues to gate every old action.
    this.limit = confirmationExpires + 30_000;
  }
  get expires(): number { return this.limit; }
  get receipt(): VoiceTransitionReceipt | null { return this.observed; }
  get needsClaim(): boolean { return this.observed === null && !this.refused; }
  get needsRelease(): boolean { return this.observed !== null && this.observed.state !== "released"; }
  current(now: number): boolean { return Number.isFinite(now) && now < this.limit && !this.refused &&
    (this.observed === null || this.observed.state === "borrowed"); }
  /** A definite authenticated refusal stops local custody. It is NOT proof that native
   * has no Claim: cleanup still requires its independent original-parent final proof. */
  refuse(): void { this.refused = true; }
  observe(receipt: VoiceTransitionReceipt, requestStarted: number): void {
    if (!Number.isFinite(requestStarted)) throw new Error("invalid voice custody clock");
    const prior = this.observed;
    if (prior && (prior.receipt !== receipt.receipt || prior.until !== receipt.until ||
        prior.finishMessage !== receipt.finishMessage || prior.finishHash !== receipt.finishHash ||
        prior.parentRow !== receipt.parentRow || prior.epoch !== receipt.epoch ||
        prior.handshake !== receipt.handshake || prior.identity.action !== receipt.identity.action ||
        prior.identity.version !== receipt.identity.version || prior.identity.inputTag !== receipt.identity.inputTag ||
        (prior.state === "released" && receipt.state !== "released") ||
        (prior.state === "closing" && receipt.state === "borrowed"))) throw new Error("voice custody identity changed");
    // Native until belongs to a different clock domain. Request START + remaining is
    // conservative even when the authenticated reply arrived much later. Retries only
    // shorten this first bound; they can never restart its 30-second window.
    this.limit = Math.min(this.limit, requestStarted + receipt.remainingMs);
    this.observed = receipt;
  }
}
