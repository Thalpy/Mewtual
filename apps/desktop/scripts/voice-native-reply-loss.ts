/** Test-only exact native-effect loss bookkeeping. Never imported by production UI.
 * Aggregate counts cannot prove which original row/reply was lost under contention. */
export type NativeSdpRowObservation = Readonly<{ row: string; accepted: boolean; drained: boolean; applied: boolean }>;
export type NativeSdpObservation = Readonly<{ complete: boolean; firstRow: string | null;
  seal: NativeSdpRowObservation | null; open: NativeSdpRowObservation | null }>;
export class NativeSdpReplyLoss {
  private seal = false;
  private open = false;
  private returned = false;
  private applied = false;
  discard(operation: number, row: string, observed: NativeSdpObservation): boolean {
    if (operation === 19 && !this.seal && observed.seal?.row === row && observed.seal.accepted) {
      this.seal = true; return true;
    }
    if (operation === 20 && !this.open && observed.open?.row === row && observed.open.accepted) {
      this.open = true; return true;
    }
    if (operation === 26 && !this.applied && observed.open?.row === row && observed.open.applied) {
      this.applied = true; return true;
    }
    const original = observed.seal?.row === row ? observed.seal : observed.open?.row === row ? observed.open : null;
    if (operation === 24 && !this.returned && observed.firstRow === row && original?.drained) {
      this.returned = true; return true;
    }
    return false;
  }
  get count(): number { return Number(this.seal) + Number(this.open) + Number(this.returned) + Number(this.applied); }
}
