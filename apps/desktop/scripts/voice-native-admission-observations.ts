/** Test-only observations AFTER real native response MAC/AEAD verification. No count
 * grants an operation or proves success. Scope refusals to the request embedded in
 * the authenticated response AAD, rather than whichever peer the fixture selected last. */
import { decodeVoiceIpcAdmission } from "../src/voice-ipc-frame.ts";

export class NativeAdmissionObservations {
  private slot: number | null = null;
  private sequence = 0n;
  private readonly refusals = new Map<string, number>();
  private readonly trace: string[] = [];
  observe(aad: unknown, opened: Uint8Array): void {
    if (!(aad instanceof Uint8Array) || aad.byteLength !== 80) return;
    const decode = (bytes: Uint8Array) => decodeVoiceIpcAdmission(
      Array.from(bytes, value => value.toString(16).padStart(2, "0")).join("") + "0".repeat(64)).header;
    let response, request;
    try { response = decode(aad.subarray(0, 40)); request = decode(aad.subarray(40)); }
    catch { return; }
    if (request.direction !== 0 || response.direction !== 1 || response.slot !== request.slot ||
        response.logical !== request.logical || response.operation !== request.operation) return;
    this.slot ??= request.slot;
    if (request.slot !== this.slot || request.sequence <= this.sequence) return;
    this.sequence = request.sequence; // Duplicate observations never credit another retry.
    if (opened.length !== 3 || opened[0] !== 1 || opened[1] !== 3 || opened[2] !== 7) return;
    const key = request.operation + ":" + request.logical;
    if (!this.refusals.has(key) && this.refusals.size === 64) throw new Error("bounded fixture refusal table full");
    this.refusals.set(key, (this.refusals.get(key) ?? 0) + 1);
    if (this.trace.length === 8) this.trace.shift();
    this.trace.push(`${request.operation}/${request.logical}/${request.sequence}`);
  }
  count(operations: readonly number[], parent: string): number {
    return operations.reduce((sum, operation) => sum + (this.refusals.get(operation + ":" + parent) ?? 0), 0);
  }
  describe(): string { return this.trace.join(","); }
}
