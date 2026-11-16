/** Bookkeeping for one separately prepaid browser SDP subtree. This retains custody,
 * never RTC permission. The original channel authenticates every observation first.
 * No peer object, callback, key, plaintext, SDK result or fulfilled data promise lives here. */
import type { VoiceSdpPhaseReceipt } from "./voice-ipc-sdp.ts";
import type { VoiceSdkReceipt } from "./voice-ipc-sdk.ts";
import { VoicePcCustody } from "./voice-pc-custody.ts";

export const VOICE_SDP_SDK_BYTES = 64 * 1024;
type Storage = Readonly<Pick<VoiceSdpPhaseReceipt, "state" | "proof">>;
type Sdk = Readonly<Pick<VoiceSdkReceipt, "state" | "generation" | "proof">>;
export class VoiceSdpSdkCustody {
  /** Fixed PC identity/Ready/profile/role/deadline and cleanup bits belong to the
   * separate64KiB subtree's4KiB bookkeeping allowance, not the old chain metadata.
   * At most two fixed32-byte values (bounded hex strings) and constant-size state are
   * retained here. Future SDK/UTF16/engine budgets must leave this allowance intact. */
  readonly pc = new VoicePcCustody();
  private stopped = false;
  private storageRefused = false;
  private sdkRefused = false;
  private sdkAttempted = false;
  private storageValue: Storage | null = null;
  private sdkValue: Sdk | null = null;
  get closed(): boolean { return this.stopped; }
  get storage(): Storage | null { return this.storageValue; }
  get sdk(): Sdk | null { return this.sdkValue; }
  get needsStorageRecovery(): boolean { return this.storageValue === null && !this.storageRefused; }
  get needsSdkRecovery(): boolean { return this.sdkAttempted && this.sdkValue === null && !this.sdkRefused; }
  get needsSdkRelease(): boolean { return this.sdkValue !== null && this.sdkValue.state !== "released"; }
  get needsStorageRelease(): boolean { return this.storageValue !== null && this.storageValue.state !== "drained"; }
  /** End only stops future effects. The channel must separately join its actual RPC/SDK
   * finally, recover ambiguous admission, and release both native obligations. */
  close(): void { this.stopped = true; this.pc.close(); }
  beginPc(): void {
    if (this.stopped || this.storageValue?.state !== "funded" || this.sdkValue?.state !== "held") throw new Error("voice PC intent is closed");
    this.pc.begin();
  }
  beginSdk(): void {
    if (this.sdkAttempted) return;
    if (this.stopped || this.storageValue?.state !== "funded") throw new Error("voice SDK custody is closed");
    this.sdkAttempted = true; // BEFORE the first admission RPC, including a lost reply
  }
  refuseStorage(): void { this.storageRefused = true; this.stopped = true; this.pc.close(); }
  refuseSdk(): void { this.sdkRefused = true; this.stopped = true; this.pc.close(); }
  observeStorage(receipt: VoiceSdpPhaseReceipt): void {
    const prior = this.storageValue;
    const order = { preparing: 0, funded: 1, closing: 2, drained: 3 };
    if (prior && (prior.proof !== receipt.proof || order[receipt.state] < order[prior.state])) throw new Error("voice SDP storage changed");
    this.storageValue = Object.freeze({ state: receipt.state, proof: receipt.proof });
    if (receipt.state === "closing" || receipt.state === "drained") this.close();
  }
  observeSdk(receipt: VoiceSdkReceipt): void {
    const prior = this.sdkValue;
    const order = { held: 0, closing: 1, released: 2 };
    if (!this.sdkAttempted || !this.storageValue || receipt.storage !== this.storageValue.proof ||
        (prior && (prior.generation !== receipt.generation || prior.proof !== receipt.proof || order[receipt.state] < order[prior.state]))) {
      throw new Error("voice SDK custody changed");
    }
    this.sdkValue = Object.freeze({ state: receipt.state, generation: receipt.generation, proof: receipt.proof });
    if (receipt.state !== "held") this.close();
  }
  /** Native release and actual local SDK/IO drainage are independent. This predicate
   * checks only the former; the retained channel joins actual flights outside its lane. */
  get nativeReleased(): boolean {
    return this.pc.nativeClosed && !this.needsStorageRecovery && !this.needsSdkRecovery && !this.needsSdkRelease && !this.needsStorageRelease;
  }
}
