/** Fixed first-round PC intent under the already prepaid SDK subtree. These are
 * authenticated observations and cleanup duties, not RTC/key/action permissions. The
 * retained channel owns the actual request and its finally separately from this record. */
import type { VoicePcReceipt } from "./voice-ipc-pc.ts";

type Identity = NonNullable<VoicePcReceipt["identity"]>;
type Admission = Extract<VoicePcReceipt, { state: "admitted" }>;
export class VoicePcCustody {
  private attempted = false;
  private stopped = false;
  private released = false;
  private identity: Identity | null = null;
  private admission: Readonly<Pick<Admission, "profile" | "ready" | "localIsLow">> | null = null;
  private deadline: number | null = null;

  get needsClose(): boolean { return this.attempted && !this.released; }
  get nativeClosed(): boolean { return !this.needsClose; }
  /** Original lookup metadata for media recovery/cleanup after Ready expiry. This is
   * not currentness or permission; the retained media path obtains fresh native Current. */
  originalProof(): string {
    if (!this.identity || this.identity.generation !== "1") throw new Error("original voice PC identity unavailable");
    return this.identity.proof;
  }
  /** Latch BEFORE the first cryptographic/transport callback. A lost admission reply
   * still requires Close on the original obligations, even with no returned identity. */
  begin(): void {
    if (this.stopped || this.released) throw new Error("voice PC intent is closed");
    this.attempted = true;
  }
  /** Irreversible local fence. Neither this nor a deadline proves native or RTC drainage. */
  close(): void { this.stopped = true; }
  /** The caller has authenticated a response to its SAME original Control14 request.
   * Identity and Ready cannot be replaced, and retries may only shorten the conservative
   * browser deadline. A late admission after End is retained for cleanup only. */
  observe(receipt: VoicePcReceipt, requestStarted: number, receivedAt: number): void {
    if (!this.attempted || !Number.isFinite(requestStarted) || requestStarted < 0 ||
        !Number.isFinite(receivedAt) || receivedAt < requestStarted) throw new Error("voice PC observation changed");
    if (this.identity && (!receipt.identity || receipt.identity.generation !== this.identity.generation ||
        receipt.identity.proof !== this.identity.proof)) throw new Error("voice PC identity changed");
    if (this.released && receipt.state !== "closed") throw new Error("voice PC intent reopened");
    if (this.released && this.identity === null && receipt.identity !== null) throw new Error("voice PC identity changed");
    if (this.admission && receipt.state === "preparing") throw new Error("voice PC intent moved backwards");
    if (receipt.state === "admitted") {
      const first = this.admission;
      if (first && (first.profile !== receipt.profile || first.ready !== receipt.ready ||
          first.localIsLow !== receipt.localIsLow)) throw new Error("voice PC admission changed");
      const deadline = requestStarted + receipt.remainingMs;
      if (!Number.isFinite(deadline)) throw new Error("voice PC deadline changed");
      this.deadline = this.deadline === null ? deadline : Math.min(this.deadline, deadline);
      this.admission ??= Object.freeze({ profile: receipt.profile, ready: receipt.ready, localIsLow: receipt.localIsLow });
      if (receivedAt >= this.deadline) this.stopped = true;
    }
    if (!this.identity && receipt.identity) {
      this.identity = Object.freeze({ generation: receipt.identity.generation, proof: receipt.identity.proof });
    }
    if (receipt.state === "closed") { this.stopped = true; this.released = true; }
  }
  /** Private crypto lookup bindings only. This conservative local observation grants no
   * SDK or RTC effect; every native Input/engine/Result repeats actual current admission.
   * Snapshot scalars so the crypto owner retains neither this record nor an RTC object. */
  cryptoBinding(now: number): Readonly<{ generation: "1"; proof: string; ready: string; localIsLow: boolean }> {
    if (this.stopped || this.released || !this.identity || !this.admission || this.identity.generation !== "1" ||
        this.deadline === null || !Number.isFinite(now) || now < 0 || now >= this.deadline) throw new Error("voice PC intent is closed");
    return Object.freeze({ generation: "1", proof: this.identity.proof, ready: this.admission.ready, localIsLow: this.admission.localIsLow });
  }
  /** Historical timing only. A private local-RTC consumer must first join a NEW same-
   * original14 admission RPC; this deadline never turns an old receipt into authority. */
  localPreparationDeadline(now: number): number {
    this.cryptoBinding(now);
    return this.deadline!;
  }
  // No public effect/grant accessor: a future RTC consumer must acquire its own exact
  // native effect permission. This historical record is insufficient even before expiry.
}
