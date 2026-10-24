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
  // No public effect/grant accessor: a future RTC consumer must acquire its own exact
  // native effect permission. This historical record is insufficient even before expiry.
}
