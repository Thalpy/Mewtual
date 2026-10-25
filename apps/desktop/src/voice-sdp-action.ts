/** One private, prepaid first-round SDP crypto owner. Native authenticates every observation;
 * this driver neither authorizes RTC nor manufactures an applied-delivery observation. Public
 * observers resolve to void, so a completed observer cannot retain a plaintext/result buffer.
 * The retained channel keeps this owner until its actual SDK/IO and native Return have joined. */
import type { VoiceIpcActionIdentity } from "./voice-ipc-actions.ts";
import type { VoiceSdpInput, VoiceSdpInputExpectation } from "./voice-ipc-sdp-input.ts";
import { decodeVoiceSdpInput } from "./voice-ipc-sdp-input.ts";
import type { VoiceSdpHistory } from "./voice-ipc-sdp-history.ts";
import type { VoiceSdpReply } from "./voice-ipc-sdp-replies.ts";
import type { VoiceSdpEngineReply } from "./voice-ipc-sdp-engine.ts";

// This is an inventory slice INSIDE the existing64KiB phase, including the old SDK/PC
// records, both fixed row claims, callback/promise graphs and transient descriptor decode.
// It is not an additional charge or permission to grow any native/browser phase limit.
export const VOICE_SDP_BROWSER_BOOK_BYTES = 16 * 1024;
const MAX = 8192;
const utf8 = new TextEncoder();
const fail = (): never => { throw new Error("original voice SDP action unavailable"); };

/** Bounded UTF8 admission precedes Begin and any async capture of an RTC string. encodeInto
 * cannot allocate a 24KiB temporary for an 8KiB string of three-byte characters. The returned
 * view owns the entire paid8KiB backing, even when fewer bytes were written. */
export function voiceSdpUtf8(text: string): Uint8Array<ArrayBuffer> {
  if (typeof text !== "string" || text.length < 1 || text.length > MAX) return fail();
  const bytes = new Uint8Array(MAX);
  const { read, written } = utf8.encodeInto(text, bytes);
  if (read !== text.length || written === 0) { bytes.fill(0); return fail(); }
  return new Uint8Array(bytes.buffer, 0, written);
}

/** A one-way transfer of the exact decrypted Input backing. accept is synchronous and has
 * no externally retained callback: the channel transfers only AFTER read/cancel finally.
 * Borrowed AAD/body views never escape the enclosing crypto owner and have one final clearer. */
export class VoiceSdpInputLoan {
  private backing: Uint8Array<ArrayBuffer> | null = null;
  private value: VoiceSdpInput | null = null;
  private used = false;
  accept(bytes: Uint8Array<ArrayBuffer>, expected: VoiceSdpInputExpectation): void {
    if (this.used) return fail();
    const value = decodeVoiceSdpInput(bytes, expected);
    this.used = true; this.backing = new Uint8Array(value.body.buffer); this.value = value;
  }
  /** Trusted private SDK consumer only; no public observer returns this value. */
  input(): VoiceSdpInput { return this.value ?? fail(); }
  clear(): void {
    this.backing?.fill(0); this.value = null; this.backing = null;
    this.used = true; // A disposed or consumed loan cannot acquire another backing.
  }
}

export type VoiceSdpActionNative = Readonly<{
  live(): boolean;
  begin(plaintext: Uint8Array<ArrayBuffer> | null): Promise<VoiceSdpReply>;
  resolve(close: boolean): Promise<VoiceSdpReply>;
  input(row: string, identity: VoiceIpcActionIdentity, loan: VoiceSdpInputLoan): Promise<VoiceSdpReply | null>;
  admit(row: string, identity: VoiceIpcActionIdentity, engineBytes: number): Promise<VoiceSdpEngineReply>;
  compute(input: VoiceSdpInput): Promise<Uint8Array<ArrayBuffer> | null>;
  digest(output: Uint8Array<ArrayBuffer>): Promise<string>;
  result(row: string, identity: VoiceIpcActionIdentity, output: Uint8Array<ArrayBuffer> | null, tag: string | null): Promise<VoiceSdpReply>;
  ack(row: string, identity: VoiceIpcActionIdentity, digest: string): Promise<VoiceSdpReply>;
  settle(row: string, identity: VoiceIpcActionIdentity): Promise<VoiceSdpReply>;
  returned(row: string): Promise<VoiceSdpReply>;
  /** Private original-RTC adapter only. Fresh false finishes physical finally before
   * apply; true reports only actual successful SDK completion and retries metadata. */
  delivery?(row: string, identity: VoiceIpcActionIdentity, applied: boolean): Promise<VoiceSdpReply>;
  applyRemote?(plaintext: Uint8Array<ArrayBuffer>): Promise<void>;
}>;

/** One original Seal/Open action. The channel selects Offer/Answer and obtains fresh
 * authority separately; this owner never infers an Answer prerequisite from a receipt.
 * The two fixed browser row claims are reserved by the broker BEFORE constructing it. */
export class VoiceSdpActionOwner {
  private beginBytes: Uint8Array<ArrayBuffer> | null;
  private beginAttempted = false;
  private row: string | null = null;
  private identity: VoiceIpcActionIdentity | null = null;
  private history: VoiceSdpHistory | null = null;
  private loan: VoiceSdpInputLoan | null = null;
  private engineAttempted = false;
  private historicalOnly = false;
  private sdkStarted = false;
  private sdkDone = false;
  private rtcStarted = false;
  private rtcDone = false;
  private failed = false;
  private output: Uint8Array<ArrayBuffer> | null = null;
  private tag: string | null = null;
  private stopped = false;
  private gone = false;
  private flight: Promise<void> | null = null;
  private readonly native: VoiceSdpActionNative;
  readonly purpose: "seal" | "open";
  constructor(native: VoiceSdpActionNative, plaintext: Uint8Array<ArrayBuffer> | null) {
    this.native = native; this.beginBytes = plaintext; this.purpose = plaintext === null ? "open" : "seal";
  }
  /** Successful live compaction releases only this action, never its original PC/phase.
   * Native24 keeps immutable terminal/delivery history for the opposite-purpose action.
   * No second Action is installed until this same Return is canonically drained. */
  get complete(): boolean {
    return !this.stopped && !this.failed && !this.historicalOnly && this.sdkDone &&
      this.history?.settled === true && this.history.acknowledged &&
      (this.purpose === "seal" ? this.history.receipt?.outcome.kind === "sealed" :
        this.history.receipt?.outcome.kind === "opened" && this.rtcDone && this.history.delivery === "applied");
  }
  finish(): Promise<void> {
    if (this.flight) return this.flight;
    if (this.gone) return Promise.resolve();
    if (!this.complete) return fail();
    const flight = Promise.resolve().then(async () => {
      if (!this.observe(await this.native.returned(this.row!))) return;
      if (this.history?.localDrained) { this.forgetBegin(); this.gone = true; }
    }).finally(() => { this.flight = null; });
    this.flight = flight; return flight;
  }
  /** End latches synchronously. It never clears an SDK input/output before its real await. */
  close(): void { this.stopped = true; }
  get returned(): boolean { return this.gone; }
  attempt(): Promise<void> {
    if (this.flight) return this.flight;
    if (this.gone) return Promise.resolve();
    const flight = Promise.resolve().then(() => this.drive()).finally(() => { this.flight = null; });
    this.flight = flight; return flight;
  }
  /** The channel joins this OUTSIDE its physical lane before SDK/storage release. A local
   * refusal leaves exact identities/output intact; its existing bounded wake retries them. */
  async cleanup(): Promise<void> { this.close(); await this.attempt(); if (!this.gone) return fail(); }
  private live(): boolean { return !this.stopped && !this.failed && this.native.live(); }
  private forgetBegin(): void {
    if (this.beginBytes) new Uint8Array(this.beginBytes.buffer).fill(0);
    this.beginBytes = null;
  }
  private observe(reply: VoiceSdpReply): boolean {
    if (reply.kind !== "history") return false;
    const value = reply.history, original = this.identity, next = value.identity;
    if (value.purpose !== this.purpose || (this.row && value.row !== this.row) ||
        (original && (!next || original.action !== next.action || original.version !== next.version || original.inputTag !== next.inputTag))) return fail();
    const prior = this.history;
    if (prior && ((prior.settled && !value.settled) || (prior.acknowledged && !value.acknowledged) ||
        (prior.localDrained && !value.localDrained) || (prior.receipt && (!value.receipt ||
          prior.receipt.digest !== value.receipt.digest || prior.receipt.outputTag !== value.receipt.outputTag)))) return fail();
    this.row = value.row; this.identity = next; this.history = value;
    if (value.receipt && !this.sdkStarted) this.historicalOnly = true;
    // A native Pending row may precede original Action publication. It is not proof that
    // native retained the LOW plaintext: an exact Begin retry must still supply it.
    if (next) this.forgetBegin();
    return true;
  }
  /** End this lexical input/descriptor frame before any Result encoding/fetch starts.
   * Clearing the loan alone would leave its views reachable through a drive-local input. */
  private async admitAndCompute(loan: VoiceSdpInputLoan): Promise<void> {
    const input = loan.input();
    this.engineAttempted = true;
    const reply = await this.native.admit(this.row!, this.identity!, input.engineBytes);
    if (reply.kind !== "admitted") {
      if (reply.kind === "history") {
        this.historicalOnly = true; this.observe(reply);
        loan.clear(); this.loan = null;
      }
      return;
    }
    if (!this.live()) return;
    this.sdkStarted = true;
    try {
      this.output = await this.native.compute(input);
      this.sdkDone = true;
      if (this.output !== null) {
        const expected = input.body.byteLength + (this.purpose === "seal" ? 16 : -16);
        if (this.output.byteLength !== expected) return fail();
        this.tag = await this.native.digest(this.output);
      } else if (this.purpose !== "open") return fail();
    } catch { this.failed = true; }
    finally { loan.clear(); this.loan = null; }
  }
  private async drive(): Promise<void> {
    try {
      // Unknown Begin is resolved before another Begin. Lost replies never select another
      // row, purpose, native Action, plaintext or counter. End uses the close-first lookup.
      if (!this.row && this.beginAttempted) {
        const reply = await this.native.resolve(!this.live());
        if (!this.observe(reply)) {
          if (reply.kind !== "missing") return;
          if (!this.live()) { if (!reply.closeRecorded) return; this.forgetBegin(); this.gone = true; return; }
        }
      }
      if (!this.row && this.live()) {
        this.beginAttempted = true; // BEFORE SDK/fetch/owner callbacks, including lost reply.
        if (!this.observe(await this.native.begin(this.beginBytes))) return;
      }
      if (!this.row) { if (!this.beginAttempted) { this.forgetBegin(); this.gone = true; } return; }
      if (!this.identity) {
        if (!this.observe(await this.native.resolve(!this.live()))) return;
        if (this.live() && !this.identity) {
          // The SAME known Pending row owns this retry. Capacity before Action attachment
          // must not strand it by dropping the original LOW bytes or minting a new purpose.
          if (!this.observe(await this.native.begin(this.beginBytes))) return;
        }
        if (this.live() && !this.identity) return;
      }
      if (this.live() && this.identity && !this.history?.receipt && !this.history?.settled && !this.sdkStarted) {
        const loan = this.loan ??= new VoiceSdpInputLoan();
        if (!this.engineAttempted) {
          const reply = await this.native.input(this.row, this.identity, loan);
          if (reply) { this.observe(reply); loan.clear(); this.loan = null; return; }
        }
        if (!this.live()) return;
        await this.admitAndCompute(loan);
        if (!this.sdkStarted && this.live() && !this.history?.receipt && !this.history?.settled) return;
      }
      if (this.live() && this.sdkDone && !this.history?.receipt) {
        if (!this.observe(await this.native.result(this.row, this.identity!, this.output, this.tag))) return;
        const receipt = this.history?.receipt;
        if (!receipt || (this.output !== null && receipt.outputTag !== this.tag)) return fail();
      }
      // Settlement follows real SDK/digest AND the last Result RPC's read/cancel finally.
      // A historical row discovered before this owner's admission is no SDK-join proof.
      if (!this.historicalOnly && (this.engineAttempted || !this.live()) && this.identity && !this.history?.settled) {
        if (!this.observe(await this.native.settle(this.row, this.identity))) return;
      }
      if (this.history?.receipt && !this.history.acknowledged) {
        if (!this.observe(await this.native.ack(this.row, this.identity!, this.history.receipt.digest))) return;
      }
      // AEAD Open and crypto settlement do not authorize RTC. A fresh same-original26
      // read/cancel must finish before decoding the private paid inbox into an SDK string.
      if (this.purpose === "open" && this.history?.receipt?.outcome.kind === "opened" &&
          this.native.delivery && this.native.applyRemote && !this.historicalOnly) {
        if (!this.rtcStarted && this.live() && this.history.delivery !== "applied") {
          if (!this.observe(await this.native.delivery(this.row, this.identity!, false))) return;
          if (!this.live() || this.history.delivery !== "admitted") return;
          if (!this.output) return fail();
          this.rtcStarted = true; // BEFORE SDK callbacks, never reset by an uncertain true26.
          try { await this.native.applyRemote(this.output); this.rtcDone = true; }
          catch { this.failed = true; }
          finally { this.output.fill(0); this.output = null; }
        }
        if (this.rtcDone && this.history.delivery !== "applied") {
          // Historical retry is legal after End. Native refuses a FIRST completion when
          // current authority has gone; that refusal must not obstruct closed cleanup.
          const reply = await this.native.delivery(this.row, this.identity!, true);
          if (!this.observe(reply) && this.live()) return;
        }
      }
      // Keep successful Open plaintext in its paid private inbox for a future fresh-delivery
      // owner. Seal ciphertext is no longer needed after canonical publication and ACK.
      if (this.purpose === "seal" && this.history?.receipt) { this.output?.fill(0); this.output = null; }
      if (!this.live()) {
        this.loan?.clear(); this.loan = null; this.output?.fill(0); this.output = null;
        if (!this.observe(await this.native.returned(this.row))) return;
        if (this.history?.localDrained) { this.forgetBegin(); this.gone = true; }
      }
    } finally {
      // A live local/network refusal retains the SAME original input/output for retry.
      // End only clears once the SDK/Result await above has actually returned.
      if (!this.live()) { this.forgetBegin(); this.loan?.clear(); this.loan = null; this.output?.fill(0); this.output = null; }
    }
  }
}
