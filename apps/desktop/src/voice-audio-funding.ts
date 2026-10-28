/** Detached microphone accounting. Nothing here owns a broker, Call, native grant,
 * channel, configuration lease, media status, SDK promise, pool or capture session.
 * An old permission prompt may keep these plain ledger records after its Call ends. */
import { VoiceByteLedger, reserveVoiceBytePayment } from "./voice-byte-ledger.ts";
import { VOICE_AUDIO_CAPTURE_BYTES, VOICE_AUDIO_CAPTURE_POOL_BYTES,
  type VoiceAudioCaptureBudget, type VoiceAudioCapturePayment } from "./voice-audio-capture.ts";

type GroupRecord = { group: string; references: number };
type GroupLease = { funding: FundingState | null; record: GroupRecord | null };
type Scope = { funding: FundingState | null; lease: GroupLease | null };
type FundingState = {
  ledger: VoiceByteLedger; groups: Map<string, GroupRecord>;
  active: Scope | null; pending: object | null; binding: boolean;
};
type TokenState = {
  funding: FundingState | null; lease: GroupLease | null; identity: object | null;
  payment: Readonly<{ release(): void }> | null;
  released: boolean; returned: boolean; task: "unassigned" | "tracked" | "none"; done: boolean;
};
export type VoiceAudioFundingScope = Readonly<{ budget: VoiceAudioCaptureBudget; close(): void }>;
const unavailable = (): never => { throw new Error("voice audio capacity unavailable"); };
const observe = Promise.prototype.then;
// Group binding has no provider/copy callout. Capture these intrinsics before SDK work
// can replace global methods; device-ID copying keeps its separate reentrant contract.
const GroupBytes = Uint8Array, GroupDecoder = TextDecoder;
const groupUnit = Function.prototype.call.bind(String.prototype.charCodeAt) as (value: string, at: number) => number;
const groupDecode = Function.prototype.call.bind(TextDecoder.prototype.decode) as (decoder: TextDecoder, bytes: Uint8Array) => string;

function ownedGroup(group: string): string {
  if (typeof group !== "string" || group.length !== 64) return unavailable();
  // Never retain a short caller substring's large backing in an orphan payment.
  const units = new GroupBytes(64);
  for (let at = 0; at < 64; at++) {
    const unit = groupUnit(group, at);
    if (!((unit >= 48 && unit <= 57) || (unit >= 97 && unit <= 102))) return unavailable();
    units[at] = unit;
  }
  // Output can reference only this independently owned64-byte buffer, never the caller.
  return groupDecode(new GroupDecoder("utf-8", { fatal: true }), units);
}
function retainGroup(funding: FundingState, group: string): GroupLease {
  let record = funding.groups.get(group);
  if (!record) {
    // Obligations, not a lifetime allowance: only active selector + old capsule.
    if (funding.groups.size >= 2) return unavailable();
    funding.ledger.bindGroup(group, VOICE_AUDIO_CAPTURE_POOL_BYTES);
    record = { group, references: 0 }; funding.groups.set(group, record);
  }
  record.references++;
  return { funding, record };
}
function releaseGroup(lease: GroupLease): void {
  const funding = lease.funding, record = lease.record;
  if (!funding || !record) return;
  if (record.references <= 0 || funding.groups.get(record.group) !== record) return unavailable();
  if (record.references === 1) {
    funding.ledger.unbindGroup(record.group, VOICE_AUDIO_CAPTURE_POOL_BYTES);
    funding.groups.delete(record.group);
  }
  record.references--;
  lease.funding = null; lease.record = null;
}
function finishToken(token: TokenState): void {
  if (token.done || !token.released || !token.returned) return;
  const funding = token.funding!, identity = token.identity!, lease = token.lease!;
  if (funding.pending !== identity) return unavailable();
  // Pure ledger-only tail: no provider callback. Exact identity prevents an old finalizer
  // from clearing a replacement. The process-fixed pool covers this empty return frame.
  releaseGroup(lease); funding.pending = null;
  token.done = true; token.funding = null; token.lease = null; token.identity = null;
}
function taskReturned(token: TokenState): void {
  token.returned = true; finishToken(token);
}
function taskObserver(token: TokenState): () => void {
  return () => taskReturned(token);
}
// Factory OUTSIDE every broker/session scope. Promise reactions retain only plain token
// state; the token never stores the task, preventing task->capsule->token->task cycles.
function capturePayment(token: TokenState): VoiceAudioCapturePayment {
  return Object.freeze({
    release() {
      if (token.released) return;
      token.payment!.release(); token.payment = null; token.released = true;
      finishToken(token);
    },
    trackTask(task: Promise<void>) {
      if (token.task !== "unassigned") return unavailable();
      token.task = "tracked";
      const returned = taskObserver(token);
      // The helper supplies its original private void finalizer Promise immediately on
      // publication, before deferred SDK entry. Observe BOTH terminal outcomes once.
      void observe.call(task, returned, returned);
    },
    noTaskEntered() {
      if (token.task !== "unassigned") return unavailable();
      token.task = "none"; taskReturned(token);
    },
  });
}
function reserveCapsule(scope: Scope): VoiceAudioCapturePayment {
  const funding = scope.funding, activeLease = scope.lease;
  if (!funding || !activeLease || funding.active !== scope || funding.pending !== null) return unavailable();
  const group = activeLease.record!.group;
  const lease = retainGroup(funding, group);
  let payment: Readonly<{ release(): void }>;
  try { payment = reserveVoiceBytePayment(funding.ledger, group, VOICE_AUDIO_CAPTURE_BYTES); }
  catch (error) { releaseGroup(lease); throw error; }
  const identity = Object.freeze({});
  const token: TokenState = { funding, lease, identity, payment,
    released: false, returned: false, task: "unassigned", done: false };
  funding.pending = identity; // Before the caller can copy constraints or enter SDK.
  return capturePayment(token);
}
function scopeHandle(scope: Scope): VoiceAudioFundingScope {
  return Object.freeze({
    budget: Object.freeze({ reserve: () => reserveCapsule(scope) }),
    close() {
      const funding = scope.funding, lease = scope.lease;
      if (!funding || !lease) return;
      if (funding.active !== scope) return unavailable();
      funding.active = null; scope.funding = null; scope.lease = null;
      releaseGroup(lease);
    },
  });
}

/** One fixed process inventory, with at most two conservative group bindings. A pending
 * capsule owns a separate OS-permission position; it NEVER borrows the shared RTC SDK
 * position or ICE configuration. No API waits for that permission from Call teardown. */
export class VoiceAudioFunding {
  readonly #state: FundingState;
  private constructor(ledger: VoiceByteLedger) {
    this.#state = { ledger, groups: new Map(), active: null, pending: null, binding: false };
  }
  static create(ledger: VoiceByteLedger): VoiceAudioFunding {
    ledger.chargeProcess(VOICE_AUDIO_CAPTURE_POOL_BYTES); // BEFORE constructor/records/pool.
    return new VoiceAudioFunding(ledger);
  }
  bind(group: string): VoiceAudioFundingScope {
    const state = this.#state;
    if (state.active || state.binding) return unavailable();
    state.binding = true;
    try {
      const owned = ownedGroup(group), lease = retainGroup(state, owned);
      const scope: Scope = { funding: state, lease };
      state.active = scope;
      return scopeHandle(scope);
    } finally { state.binding = false; }
  }
  inspect(): Readonly<{ groups: number; active: boolean; pending: boolean }> {
    return Object.freeze({ groups: this.#state.groups.size, active: this.#state.active !== null,
      pending: this.#state.pending !== null });
  }
}

/** Conservative owned-JS metadata, not opaque VM/SDK heaps. The same fixed pool pays
 * the final empty-token observer after the old group's debit returns. One reaction only. */
export function voiceAudioFundingInventory(): number {
  const retained = 512 // ledger/funding/maps
    + 256 // capture pool
    + 256 // active selector/scope
    + 2 * 352 // bounded original-group records/leases/owned labels
    + 896 // emptied capsule/token plus independently retained passive status
    + 512 // one actual-task observer and completion promise
    + 384 // negative cleanup/refund frame
    + 256; // fixed handles/slack
  // Binding has no active selector/second record yet. Keep negative cleanup included:
  // a trusted SDK stop callback can ENTER binding while its original frame still runs.
  // The non-reentrant serial copy needs owned buffer192 + string160 + decoder128 +
  // decoder scratch256 + frame128. That scratch ends before the new record/scope exists.
  const binding = retained - 256 - 352 + 192 + 160 + 128 + 256 + 128;
  return Math.max(retained, binding);
}
