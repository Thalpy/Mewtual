/** Bounded final negative retry custody, independent of any SDK key/driver/Call.
 * A descriptor is transferred from the original physical payment only AFTER a
 * genuine terminal receipt and actual local SDK/IO return. ACK loss/NoProof is
 * never labelled delivery/collection and never reconstructs positive work. */
import { VoiceAdmissionWake } from "./voice-admission-wake.ts";
import { VoiceByteLedger } from "./voice-byte-ledger.ts";
import { VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES } from "./voice-admitted-physical-inventory.ts";
import type { VoiceAdmittedPhysicalNative } from "./voice-admitted-physical-native.ts";
import type { VoiceIpcClock } from "./voice-crypto.ts";

type Entry = {
  readonly owner: string; readonly executor: string; readonly receipt: string | null; readonly group: string;
  native: VoiceAdmittedPhysicalNative | null; wait: VoiceAdmissionWake | null;
  abandoned: boolean; work: Promise<void> | null;
};
export class AdmittedPhysicalReceipts {
  private readonly entries = new Map<string, Entry>();
  private readonly ledger: VoiceByteLedger;
  private readonly clock: VoiceIpcClock;
  constructor(ledger: VoiceByteLedger, clock: VoiceIpcClock) { this.ledger = ledger; this.clock = clock; }
  get size(): number { return this.entries.size; }
  has(owner: string): boolean { return this.entries.has(owner); }
  /** Caller transfers SAME prepaid4KiB; this method NEVER acquires fresh credit.
   * Only the owning broker calls it, once for each actual returned driver. */
  retain(owner: string, executor: string, group: string, receipt: string | null, native: VoiceAdmittedPhysicalNative): void {
    if (this.entries.size >= 32 || this.entries.has(owner)) throw new Error("physical receipt custody is full");
    const entry: Entry = { owner, executor, group, receipt, native, wait: null, abandoned: false, work: null };
    this.entries.set(owner, entry); // BEFORE invoking callback-bearing IPC.
    const work = this.run(entry); entry.work = work;
    void work.then(() => {
      // run's actual invoke/timer finally has returned. Detach every live native
      // reference before returning its SAME fee; inert Promise shells may remain.
      entry.native = null; entry.wait = null; entry.work = null;
      if (this.entries.get(owner) === entry) {
        this.entries.delete(owner);
        this.ledger.refund(group, VOICE_ADMITTED_PHYSICAL_RECEIPT_BYTES);
      }
    });
  }
  wake(): void { for (const entry of this.entries.values()) entry.wait?.wake(); }
  async abandon(): Promise<void> {
    // Explicit last release cedes retry authority; it does NOT acknowledge
    // native history. A still-pending invoke/timer must nevertheless really join.
    const pending = [...this.entries.values()];
    for (const entry of pending) { entry.abandoned = true; entry.wait?.wake(); }
    await Promise.all(pending.map(entry => entry.work));
  }
  private async run(entry: Entry): Promise<void> {
    while (!entry.abandoned) {
      try {
        await entry.native!.acknowledgeParent(entry.owner);
        if (entry.receipt !== null) await entry.native!.acknowledgePhysical(entry.owner, entry.executor, entry.receipt);
        return;
      } catch {
        // Missing parent remains NoProof. A previously held actual receipt is
        // sufficient to keep the heavy driver gone, not to claim ACK delivery.
      }
      if (entry.abandoned) break;
      const waiting = new VoiceAdmissionWake(this.clock);
      entry.wait = waiting; // publish before schedule/cancel can reenter
      waiting.start();
      try { await waiting.join(); }
      finally { if (entry.wait === waiting) entry.wait = null; }
    }
  }
}
