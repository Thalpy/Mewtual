/** Shared byte accounting with NO broker, Call, native owner, channel or SDK backedge.
 * Detached microphone work can retain this small ledger after its original call closes.
 * It is capacity only: callers still need actual native/media authority before effects. */
export class VoiceByteLedger {
  bytes = 0;
  readonly groups = new Map<string, number>();
  readonly processLimit: number;
  readonly groupLimit: number;
  constructor(processLimit: number, groupLimit: number) {
    if (!Number.isSafeInteger(processLimit) || !Number.isSafeInteger(groupLimit) ||
        groupLimit <= 0 || processLimit < groupLimit) throw new Error("invalid voice byte limits");
    this.processLimit = processLimit; this.groupLimit = groupLimit;
  }
  private amount(bytes: number): void {
    if (!Number.isSafeInteger(bytes) || bytes <= 0 || bytes > this.groupLimit)
      throw new Error("invalid voice byte charge");
  }
  /** One ordinary group/process payment. No callback or asynchronous work occurs here. */
  charge(group: string, bytes: number): void {
    this.amount(bytes);
    const previous = this.groups.get(group) ?? 0;
    if (this.bytes + bytes > this.processLimit || previous + bytes > this.groupLimit)
      throw new Error("voice crypto capacity reached");
    this.bytes += bytes; this.groups.set(group, previous + bytes);
  }
  refund(group: string, bytes: number): void {
    this.amount(bytes);
    const previous = this.groups.get(group) ?? 0;
    if (previous < bytes || this.bytes < bytes) throw new Error("voice byte payment is not owned");
    this.bytes -= bytes;
    if (previous === bytes) this.groups.delete(group); else this.groups.set(group, previous - bytes);
  }
  /** Fixed window infrastructure is charged once in the process. Accepted groups bind
   * conservative portions separately; two groups sharing the same fixed graph do not
   * manufacture another process allocation or increase either ceiling. */
  chargeProcess(bytes: number): void {
    this.amount(bytes);
    if (this.bytes + bytes > this.processLimit) throw new Error("voice crypto capacity reached");
    this.bytes += bytes;
  }
  bindGroup(group: string, bytes: number): void {
    this.amount(bytes);
    const previous = this.groups.get(group) ?? 0;
    if (previous + bytes > this.groupLimit) throw new Error("voice crypto capacity reached");
    this.groups.set(group, previous + bytes);
  }
  unbindGroup(group: string, bytes: number): void {
    this.amount(bytes);
    const previous = this.groups.get(group) ?? 0;
    if (previous < bytes) throw new Error("voice group payment is not owned");
    if (previous === bytes) this.groups.delete(group); else this.groups.set(group, previous - bytes);
  }
}

/** Construct outside any broker/Call scope: the one-shot closure retains only this
 * ledger, the original immutable group identity, amount and negative release latch. */
export function reserveVoiceBytePayment(ledger: VoiceByteLedger, group: string, bytes: number): Readonly<{ release(): void }> {
  ledger.charge(group, bytes);
  const state: { ledger: VoiceByteLedger | null; group: string; bytes: number } = { ledger, group, bytes };
  return Object.freeze({ release() {
    const original = state.ledger;
    if (!original) return;
    original.refund(state.group, state.bytes);
    state.ledger = null; state.group = ""; state.bytes = 0;
  } });
}
