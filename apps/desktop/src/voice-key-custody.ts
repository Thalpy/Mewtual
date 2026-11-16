/** Browser bookkeeping for the SAME protected pair. This object grants no native effect,
 * admission, SDP or RTC authority. Only the original authenticated Finish Input creates it;
 * native Claim independently checks the original plan and current session. */
import type { VoiceKeySessionReceipt } from "./voice-ipc-key-session.ts";
import type { VoiceTransitionReceipt } from "./voice-ipc-transition.ts";

export class VoiceKeyCustody {
  private limit: number;
  private observed: VoiceTransitionReceipt | null = null;
  private refused = false;
  // Installed BEFORE the adoption RPC while the SAME keys still exist. Unknown replies
  // permit only paid quarantine and exact recovery; they are never an effect grant.
  // One inline flag word keeps the existing fixed custody wrapper allowance. Terminal
  // bits never clear. The compact key receipt borrows all original identity from Claim.
  private keyFlags = 0; // 1=pending, 2=definite refusal, 4=local End
  private keys: Readonly<Pick<VoiceKeySessionReceipt, "state" | "receipt">> | null = null;
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
  /** A null deadline retains keys only. Every old action still uses its unchanged deadline. */
  get retentionDeadline(): number | null { return (this.keyFlags & 1) !== 0 || this.keys?.state === "current" ? null : this.limit; }
  get keyReceipt(): Readonly<Pick<VoiceKeySessionReceipt, "state" | "receipt">> | null { return this.keys; }
  get needsKeyRecovery(): boolean { return (this.keyFlags & 1) !== 0 && this.keys === null && (this.keyFlags & 2) === 0; }
  get needsKeyRelease(): boolean { return this.keys !== null && this.keys.state !== "released"; }
  get confirmationRetired(): boolean { return (this.keyFlags & 1) !== 0 || this.keys !== null || (this.keyFlags & 2) !== 0; }
  current(now: number): boolean {
    if (!Number.isFinite(now) || (this.keyFlags & 4) !== 0 || this.refused || (this.keyFlags & 2) !== 0) return false;
    if (this.keys) return this.keys.state === "current";
    if ((this.keyFlags & 1) !== 0) return true;
    return now < this.limit && (this.observed === null || this.observed.state === "borrowed");
  }
  /** One immutable handoff intent; not a constructor or a way to revive expired keys. */
  beginKeyAdoption(now: number): void {
    if ((this.keyFlags & 1) !== 0 || this.keys || (this.keyFlags & 2) !== 0) return;
    if (!this.observed || this.observed.state !== "borrowed" || !this.current(now)) throw new Error("voice key custody expired");
    this.keyFlags |= 1;
  }
  closeKeys(): void { this.keyFlags |= 4; }
  refuseKeys(): void { this.keyFlags |= 2; }
  observeKeys(receipt: VoiceKeySessionReceipt): void {
    const original = this.observed, previous = this.keys;
    if ((this.keyFlags & 1) === 0 || !original || receipt.parentRow !== original.parentRow || receipt.epoch !== original.epoch ||
        receipt.handshake !== original.handshake || receipt.claim !== original.receipt ||
        receipt.finishMessage !== original.finishMessage || receipt.finishHash !== original.finishHash ||
        receipt.identity.action !== original.identity.action || receipt.identity.version !== original.identity.version ||
        receipt.identity.inputTag !== original.identity.inputTag ||
        (previous && (previous.receipt !== receipt.receipt ||
          (previous.state === "released" && receipt.state !== "released") ||
          (previous.state === "closing" && receipt.state === "current")))) throw new Error("voice key session changed");
    this.keys = Object.freeze({ state: receipt.state, receipt: receipt.receipt });
    // Keep pending as immutable evidence that an ambiguous adoption must never be replaced.
    // closeKeys/refuseKeys are also irreversible; a late Current cannot restore local keys.
  }
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
