/** V2 endpoint crypto foundation. Not wired to calls until native signed delegation, packet
 * reservation, replay/confirmation and final-delivery commands exist. Native tickets are
 * authority; these WebCrypto helpers neither authenticate a member nor authorize a helper. */
// The broker survives HMR, so new module instances must use its original owner brand.
// This is lifecycle bookkeeping within one trusted window, not a security capability.
const OWNER = Symbol.for("mewtual.voice.crypto-owner.v2");
export type VoiceCryptoOwner = Readonly<{
  token: string; group: string; isCurrent(): boolean; [OWNER]: true;
  /** Irreversible retirement wakes native-action observers; it never cancels real engine work. */
  retire(): void;
  onRetired(listener: () => void): () => void;
}>;

/** One immutable native owner. Revocation latches: a reused outer UI object or later true
 * predicate cannot resurrect an old native capability or reset its encryption allocator. */
export function createVoiceCryptoOwner(token: string, group: string, isCurrent: () => boolean): VoiceCryptoOwner {
  if (!token || token.length > 128 || !/^[0-9a-f]{64}$/.test(group)) throw new Error("invalid native voice owner");
  let retired = false;
  const listeners = new Set<() => void>();
  const retire = () => {
    if (retired) return;
    retired = true;
    // Publish the latch first. A listener cannot register/restore work during this notification.
    const pending = [...listeners]; listeners.clear();
    for (const listener of pending) { try { listener(); } catch { /* Continue revoking every observer. */ } }
  };
  return Object.freeze({ token, group, [OWNER]: true as const, isCurrent() {
    if (!retired) { try { if (!isCurrent()) retire(); } catch { retire(); } }
    return !retired;
  }, retire, onRetired(listener: () => void) {
    if (retired) { listener(); return () => {}; }
    if (listeners.size >= 64) throw new Error("voice owner observer capacity reached");
    listeners.add(listener);
    return () => { listeners.delete(listener); };
  } });
}
export type VoiceCryptoOperation = "generate" | "derive" | "encrypt" | "decrypt";
export type VoiceEncryptedIncoming = Readonly<{ aad: Uint8Array; sequence: bigint; ciphertext: Uint8Array }>;

/** Stateless IPC adapter. Native charges survive lost WebViews; settle names one exact operation.
 * commitInbound revalidates the exact authenticated tuple after decrypt and consumes closed
 * confirmation plaintext on native. The hook must finish before any plaintext reaches RTC. */
export interface VoiceCryptoAdmission {
  admit(owner: string, operation: VoiceCryptoOperation, bytes: number): Promise<string>;
  settle(token: string): Promise<void>;
  commitInbound(owner: string, token: string, packet: VoiceEncryptedIncoming, plaintext: Uint8Array): Promise<void>;
}

const MAX_ACTIVE = 2;
const MAX_QUEUED = 32;
const MAX_GROUP_BYTES = 256 * 1024;
const MAX_BYTES = 2 * 1024 * 1024;
// Largest V2 plaintext kind; native admission still enforces each authenticated kind's bound.
// Keep this in sync with voice_v2::MAX_SDP_PLAINTEXT, including the exact-boundary tests.
const MAX_PLAINTEXT = 8 * 1024;
const MAX_CONTEXT = 4 * 1024;
const encoder = new TextEncoder();
type Job = { owner: VoiceCryptoOwner; bytes: number; start(): Promise<void>; refuse(): void };
const CONFIRM_COMPUTE = Symbol("native confirmation computation");
const MAX_ACTIONS = 32;
type ConfirmationKind = "hello" | "proof" | "finish";
type RejectionCode = "retired" | "invalid" | "capacity";
export type VoiceConfirmationIdentity = Readonly<{
  owner: string; action: string; version: string; inputTag: string;
}>;
/** IDs cross JSON as exact canonical decimal u64 strings, never JavaScript numbers. */
export type VoiceConfirmationInput = Readonly<{
  action: string; version: string; inputTag: string; handshake: string;
  deliveryToken: string; kind: ConfirmationKind; sequence: string;
  aad: Uint8Array; ciphertext: Uint8Array;
}>;
type AdmissionReply = (VoiceConfirmationIdentity & Readonly<{ kind: "admitted"; permit: string }>) |
  (VoiceConfirmationIdentity & Readonly<{ kind: "rejected"; receipt: string; code: RejectionCode }>);
export type VoiceConfirmationReceipt = VoiceConfirmationIdentity & Readonly<{
  kind: "accepted"; resultTag: string; receipt: string; confirmation: "continue" | "confirmed";
}>;
type CommitReply = VoiceConfirmationReceipt |
  (VoiceConfirmationIdentity & Readonly<{ kind: "rejected"; resultTag: string; receipt: string; code: RejectionCode }>);
/** The future native adapter must implement idempotent admission AND a permanent settlement
 * fence for this exact identity. It cannot delegate to the ordinary fresh-token admit hook.
 * A rejected receipt is terminal, including capacity; temporary slot pressure must not issue
 * that receipt or consume this action. It may refuse the RPC for a later exact retry.
 * Commit records its receipt and enqueues follow-ups; it must not await follow-up WebCrypto. */
export interface VoiceConfirmationNative {
  admitAction(identity: VoiceConfirmationIdentity, bytes: number): Promise<AdmissionReply>;
  commitConfirmation(identity: VoiceConfirmationIdentity, packet: VoiceConfirmationInput, plaintext: Uint8Array, resultTag: string): Promise<CommitReply>;
  settleAction(identity: VoiceConfirmationIdentity): Promise<void>;
}
export type VoiceConfirmationOutcome =
  | Readonly<{ kind: "accepted"; receipt: VoiceConfirmationReceipt }>
  | Readonly<{ kind: "rejected"; receipt: string; code: RejectionCode }>
  | Readonly<{ kind: "uncertain" | "retired" | "failed" | "busy" }>;
export type VoiceConfirmationStatus = "queued" | "admitting" | "computing" | "committing" |
  "uncertain" | "accepted" | "rejected" | "failed" | "retired" | "busy";
/** Confirmation receipts describe a past native transition. They never grant fresh pair,
 * presence or RTC authority, and no method exports the decrypted confirmation plaintext. */
export type VoiceConfirmationHandle = Readonly<{
  attempt(): Promise<VoiceConfirmationOutcome>;
  inspect(): VoiceConfirmationStatus;
  retire(): void;
}>;
type Signal<T> = { promise: Promise<T>; resolve(value: T): void };
function signal<T>(): Signal<T> {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
type ConfirmationRecord = {
  owner: VoiceCryptoOwner; pair: VoiceCryptoPair | null;
  identity: VoiceConfirmationIdentity; input: VoiceConfirmationInput | null; bytes: number;
  status: VoiceConfirmationStatus; outcome: VoiceConfirmationOutcome | null;
  attempt: Signal<VoiceConfirmationOutcome> | null; retry: Signal<void> | null;
  job: Job | null; running: boolean; retired: boolean; drained: boolean; released: boolean;
  unsubscribe: () => void; handle: VoiceConfirmationHandle;
};
function u64(value: string, label: string): bigint {
  if (typeof value !== "string" || !/^[1-9][0-9]{0,19}$/.test(value)) throw new Error("invalid " + label);
  const number = BigInt(value);
  if (number > 0xffff_ffff_ffff_ffffn) throw new Error("invalid " + label);
  return number;
}
function sameBytes(a: Uint8Array, b: Uint8Array): boolean {
  return a.length === b.length && a.every((value, index) => value === b[index]);
}
function sameConfirmation(a: VoiceConfirmationInput, b: VoiceConfirmationInput): boolean {
  return a.action === b.action && a.version === b.version && a.inputTag === b.inputTag &&
    a.handshake === b.handshake && a.deliveryToken === b.deliveryToken && a.kind === b.kind &&
    a.sequence === b.sequence && sameBytes(a.aad, b.aad) && sameBytes(a.ciphertext, b.ciphertext);
}
function exactKeys(value: unknown, names: string[]): value is Record<string, unknown> {
  return !!value && typeof value === "object" && Reflect.ownKeys(value).length === names.length &&
    names.every(name => Object.hasOwn(value, name));
}
function boundText(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= 128;
}
function boundIdentity(value: Record<string, unknown>, expected: VoiceConfirmationIdentity): boolean {
  return value.owner === expected.owner && value.action === expected.action &&
    value.version === expected.version && value.inputTag === expected.inputTag;
}
function rejection(value: unknown): value is RejectionCode {
  return value === "retired" || value === "invalid" || value === "capacity";
}

function current(owner: VoiceCryptoOwner): void {
  if (owner[OWNER] !== true || !Object.isFrozen(owner) || !owner.isCurrent()) throw new Error("voice crypto owner changed");
}
function boundedBytes(bytes: Uint8Array, max: number, label: string): Uint8Array<ArrayBuffer> {
  if (bytes.byteLength > max) throw new Error(`${label} exceeds voice crypto bound`);
  return new Uint8Array(bytes);
}
function context(label: string, transcript: Uint8Array): Uint8Array<ArrayBuffer> {
  const prefix = encoder.encode(`${label}\0`), out = new Uint8Array(prefix.length + transcript.length);
  out.set(prefix); out.set(transcript, prefix.length); return out;
}

/** One surviving window's bounded broker. Native additionally bounds process-wide admitted and
 * unsettled work. Revoking an owner never frees a running promise; only its real settlement does.
 * A derive pipeline uses one permit for sequential import/ECDH/HKDF work, never parallel calls. */
export class VoiceCryptoBroker {
  readonly version = 4;
  readonly subtle: SubtleCrypto;
  private readonly native: VoiceCryptoAdmission;
  private active = 0;
  private bytes = 0;
  private readonly groupBytes = new Map<string, number>();
  private readonly queue: Job[] = [];
  private readonly sequences = new Map<string, { owner: VoiceCryptoOwner; highest: bigint; used: Set<bigint> }>();
  // This map and its combined pending/terminal cap survive HMR with this exact broker.
  private readonly confirmations = new Map<string, ConfirmationRecord>();
  private confirmationNative: VoiceConfirmationNative | null = null;

  constructor(native: VoiceCryptoAdmission, subtle: SubtleCrypto = globalThis.crypto.subtle) {
    this.native = native; this.subtle = subtle;
  }

  /** Pair wrappers come from the retained broker's module. A fresh HMR derive may finish on
   * this older broker; using its factory preserves the private computation seam's identity. */
  createPair(owner: VoiceCryptoOwner, tx: CryptoKey, rx: CryptoKey, domain: string): VoiceCryptoPair {
    current(owner);
    return new VoiceCryptoPair(this, owner, tx, rx, domain);
  }

  run<T>(owner: VoiceCryptoOwner, operation: VoiceCryptoOperation, bytes: number, work: () => Promise<T>): Promise<T> {
    try { current(owner); } catch (error) { return Promise.reject(error); }
    const group = this.groupBytes.get(owner.group) ?? 0;
    if (!Number.isSafeInteger(bytes) || bytes < 0 || bytes > MAX_GROUP_BYTES ||
        this.queue.length >= MAX_QUEUED || this.bytes + bytes > MAX_BYTES || group + bytes > MAX_GROUP_BYTES) {
      return Promise.reject(new Error("voice crypto capacity reached"));
    }
    this.bytes += bytes; this.groupBytes.set(owner.group, group + bytes);
    const result = new Promise<T>((resolve, reject) => {
      const finish = () => {
        this.bytes -= bytes;
        const remaining = (this.groupBytes.get(owner.group) ?? 0) - bytes;
        if (remaining) this.groupBytes.set(owner.group, remaining); else this.groupBytes.delete(owner.group);
      };
      this.queue.push({ owner, bytes,
        refuse() { finish(); reject(new Error("voice crypto owner changed")); },
        start: async () => {
          let token: string | null = null;
          try {
            current(owner);
            token = await this.native.admit(owner.token, operation, bytes);
            current(owner);
            const value = await work();
            current(owner);
            resolve(value);
          } catch (error) { reject(error); }
          finally {
            // Real crypto has settled before this point. A failed settlement IPC cannot refund
            // native capacity; native retains its uncertainty. It does not justify retrying work.
            if (token) { try { await this.native.settle(token); } catch { /* native charge remains */ } }
            finish();
          }
        },
      });
    });
    this.pump();
    return result;
  }

  private pump(): void {
    while (this.active < MAX_ACTIVE && this.queue.length) {
      const job = this.queue.shift()!;
      if (!job.owner.isCurrent()) { job.refuse(); continue; }
      this.active++;
      void job.start().finally(() => { this.active--; this.pump(); });
    }
  }

  async commitInbound(owner: VoiceCryptoOwner, token: string, packet: VoiceEncryptedIncoming, plaintext: Uint8Array): Promise<void> {
    current(owner);
    if (!token) throw new Error("native voice delivery token required");
    // Native compares this exact encrypted tuple with the authenticated token's stored packet,
    // then rechecks its original issuer/pair/joins. A token for another peer cannot authorize it.
    // Native gets an independent bounded copy; an IPC adapter cannot mutate the bytes returned
    // to RTC after authorization. Cleanup is best effort and makes no browser-wide RAM claim.
    const opened = boundedBytes(plaintext, MAX_PLAINTEXT, "plaintext");
    try { await this.native.commitInbound(owner.token, token, packet, opened); current(owner); }
    finally { opened.fill(0); }
  }

  counts(): { active: number; queued: number; bytes: number } {
    return { active: this.active, queued: this.queue.length, bytes: this.bytes };
  }

  /** Register one frozen native confirmation input before crypto. Same identity/input returns
   * the same handle; neither repull nor a new wrapper can restart failed/uncertain computation.
   * The first stateless native adapter remains captured across HMR, like the ordinary broker. */
  registerConfirmation(
    owner: VoiceCryptoOwner, pair: VoiceCryptoPair, native: VoiceConfirmationNative,
    input: VoiceConfirmationInput,
  ): VoiceConfirmationHandle {
    current(owner);
    if (!pair.belongsTo(this, owner)) throw new Error("voice pair belongs to another owner");
    if (!exactKeys(input, ["action", "version", "inputTag", "handshake", "deliveryToken", "kind", "sequence", "aad", "ciphertext"]) ||
        !(input.aad instanceof Uint8Array) || !(input.ciphertext instanceof Uint8Array)) {
      throw new Error("invalid native confirmation input");
    }
    u64(input.action, "action"); u64(input.version, "action version");
    u64(input.deliveryToken, "delivery token"); u64(input.sequence, "sequence");
    if (!/^[0-9a-f]{64}$/.test(input.inputTag) || !/^[0-9a-f]{64}$/.test(input.handshake) ||
        !["hello", "proof", "finish"].includes(input.kind) ||
        input.aad.length > MAX_CONTEXT || input.ciphertext.length < 16 || input.ciphertext.length > 528) {
      throw new Error("invalid native confirmation input");
    }
    const key = owner.token + "/" + input.action;
    const previous = this.confirmations.get(key);
    if (previous) {
      if (previous.owner !== owner || previous.pair !== pair || !previous.input || !sameConfirmation(previous.input, input)) {
        throw new Error("native confirmation identity changed");
      }
      return previous.handle;
    }
    // Cover frozen input, real engine result, digest and independent IPC serialization copies.
    // Conservative peak stays charged through terminal receipt retention; there is no refund
    // gap between a computing record and its completed result/tombstone.
    const bytes = 4 * (input.aad.length + input.ciphertext.length) + 3 * 512 + 2048;
    const group = this.groupBytes.get(owner.group) ?? 0;
    if (this.confirmations.size >= MAX_ACTIONS || this.bytes + bytes > MAX_BYTES || group + bytes > MAX_GROUP_BYTES) {
      throw new Error("voice confirmation capacity reached");
    }
    this.bytes += bytes; this.groupBytes.set(owner.group, group + bytes);
    const identity = Object.freeze({ owner: owner.token, action: input.action, version: input.version, inputTag: input.inputTag });
    const owned = Object.freeze({
      action: input.action, version: input.version, inputTag: input.inputTag, handshake: input.handshake,
      deliveryToken: input.deliveryToken, kind: input.kind, sequence: input.sequence,
      aad: new Uint8Array(input.aad), ciphertext: new Uint8Array(input.ciphertext),
    });
    const record: ConfirmationRecord = {
      owner, pair, identity, input: owned, bytes, status: "queued", outcome: null,
      attempt: null, retry: null, job: null, running: false, retired: false,
      drained: false, released: false, unsubscribe: () => {}, handle: null!,
    };
    record.handle = Object.freeze({
      attempt: () => this.attemptConfirmation(record),
      inspect: () => record.status,
      retire: () => this.retireConfirmation(record),
    });
    this.confirmations.set(key, record);
    this.confirmationNative ??= native;
    try { record.unsubscribe = owner.onRetired(() => this.retireConfirmation(record)); }
    catch (error) { this.releaseConfirmation(record); throw error; }
    return record.handle;
  }

  private attemptConfirmation(record: ConfirmationRecord): Promise<VoiceConfirmationOutcome> {
    if (!record.owner.isCurrent() || record.retired) {
      this.retireConfirmation(record); return Promise.resolve(Object.freeze({ kind: "retired" }));
    }
    if (record.outcome) return Promise.resolve(record.outcome);
    // Publish before queue/retry signalling. Every concurrent observer shares this one flight;
    // abandoning a Promise affects no underlying admission, engine or commit lifetime.
    if (record.attempt) return record.attempt.promise;
    record.attempt = signal<VoiceConfirmationOutcome>();
    const attempt = record.attempt.promise;
    if (record.retry) { record.retry.resolve(); return attempt; }
    if (record.job) return attempt;
    if (this.queue.length >= MAX_QUEUED) {
      // Nothing was admitted or computed. Temporary unrelated work cannot permanently burn a
      // registered native action; a later exact attempt may use returned queue capacity.
      record.status = "busy"; record.drained = true;
      record.attempt.resolve(Object.freeze({ kind: "busy" })); record.attempt = null;
      return attempt;
    }
    record.drained = false; record.status = "queued";
    const job: Job = {
      owner: record.owner, bytes: 0,
      refuse: () => { record.drained = true; this.retireConfirmation(record); },
      start: async () => {
        record.running = true;
        try { await this.driveConfirmation(record); }
        catch {
          // A local bookkeeping failure must not strand observers or become an unhandled
          // worker rejection. Its identity remains terminal; never reconstruct its engine work.
          if (!record.retired && record.owner.isCurrent()) this.finishConfirmation(record, Object.freeze({ kind: "failed" }));
        }
        finally {
          record.drained = true; record.running = false;
          if (!record.owner.isCurrent()) this.releaseConfirmation(record);
        }
      },
    };
    record.job = job; this.queue.push(job); this.pump();
    return attempt;
  }

  private finishConfirmation(record: ConfirmationRecord, outcome: VoiceConfirmationOutcome): void {
    record.outcome = outcome; record.status = outcome.kind;
    record.attempt?.resolve(outcome); record.attempt = null;
  }

  private async uncertainConfirmation(record: ConfirmationRecord): Promise<void> {
    if (record.retired || !record.owner.isCurrent()) return;
    record.status = "uncertain";
    record.retry = signal<void>();
    record.attempt?.resolve(Object.freeze({ kind: "uncertain" })); record.attempt = null;
    await record.retry.promise; record.retry = null;
  }

  private async driveConfirmation(record: ConfirmationRecord): Promise<void> {
    const native = this.confirmationNative!;
    let attemptedAdmission = false, admitted = false;
    let plaintext: Uint8Array<ArrayBuffer> | null = null, resultTag: string | null = null;
    const live = () => !record.retired && record.owner.isCurrent();
    try {
      while (live() && !admitted) {
        record.status = "admitting"; attemptedAdmission = true;
        let answer: AdmissionReply;
        try { answer = await native.admitAction(record.identity, record.bytes); }
        catch { await this.uncertainConfirmation(record); continue; }
        if (!live()) break;
        if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "permit"]) &&
            answer.kind === "admitted" && boundIdentity(answer, record.identity) && boundText(answer.permit)) {
          admitted = true;
        } else if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "receipt", "code"]) &&
                   answer.kind === "rejected" && boundIdentity(answer, record.identity) &&
                   boundText(answer.receipt) && rejection(answer.code)) {
          this.finishConfirmation(record, Object.freeze({ kind: "rejected", receipt: answer.receipt, code: answer.code }));
          return;
        } else { await this.uncertainConfirmation(record); }
      }
      if (!live()) return;
      // Engine failure is permanent for this identity. It is not retryable IPC uncertainty.
      record.status = "computing";
      try {
        plaintext = await record.pair!.computeConfirmation(CONFIRM_COMPUTE, record.input!);
        if (!live()) return;
        if (plaintext.length > 512) throw new Error("invalid confirmation engine result");
        const digest = new Uint8Array(await this.subtle.digest("SHA-256", plaintext));
        if (digest.length !== 32) throw new Error("invalid confirmation digest");
        resultTag = Array.from(digest, value => value.toString(16).padStart(2, "0")).join("");
      } catch {
        if (live()) this.finishConfirmation(record, Object.freeze({ kind: "failed" }));
        return;
      }
      while (live()) {
        record.status = "committing";
        const input = record.input!;
        const packet = Object.freeze({ ...input, aad: input.aad.slice(), ciphertext: input.ciphertext.slice() });
        const copy = plaintext.slice();
        let answer: CommitReply | null = null;
        try { answer = await native.commitConfirmation(record.identity, packet, copy, resultTag); }
        catch { /* A lost/malformed reply cannot authorize another engine operation. */ }
        finally { copy.fill(0); }
        if (!live()) return;
        if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "resultTag", "receipt", "confirmation"]) &&
            answer.kind === "accepted" && boundIdentity(answer, record.identity) &&
            answer.resultTag === resultTag && boundText(answer.receipt) &&
            (answer.confirmation === "continue" || answer.confirmation === "confirmed")) {
          this.finishConfirmation(record, Object.freeze({ kind: "accepted", receipt: Object.freeze({ ...answer }) as VoiceConfirmationReceipt }));
          return;
        }
        if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "resultTag", "receipt", "code"]) &&
            answer.kind === "rejected" && boundIdentity(answer, record.identity) &&
            answer.resultTag === resultTag && boundText(answer.receipt) && rejection(answer.code)) {
          this.finishConfirmation(record, Object.freeze({ kind: "rejected", receipt: answer.receipt, code: answer.code }));
          return;
        }
        await this.uncertainConfirmation(record);
      }
    } finally {
      plaintext?.fill(0);
      // Only actual engine and last RPC settlement reaches this fence. Native must latch it,
      // serialized with admission, so delayed admission cannot recreate a retired charge.
      if (attemptedAdmission) {
        try { await native.settleAction(record.identity); } catch { /* Native uncertainty stays charged. */ }
      }
      if (record.retired || !record.owner.isCurrent()) this.retireConfirmation(record);
    }
  }

  private retireConfirmation(record: ConfirmationRecord): void {
    record.retired = true;
    this.finishConfirmation(record, Object.freeze({ kind: "retired" }));
    record.retry?.resolve();
    if (!record.running) {
      const queued = record.job ? this.queue.indexOf(record.job) : -1;
      if (queued >= 0) this.queue.splice(queued, 1);
      record.drained = true;
    }
    // Action-only retirement keeps the terminal identity/slot while its owner is current.
    // Owner retirement may remove it only after actual work has drained.
    if (record.drained && !record.owner.isCurrent()) this.releaseConfirmation(record);
  }

  private releaseConfirmation(record: ConfirmationRecord): void {
    if (record.released) return;
    record.released = true; record.unsubscribe();
    record.input?.aad.fill(0); record.input?.ciphertext.fill(0);
    record.input = null; record.pair = null;
    this.confirmations.delete(record.identity.owner + "/" + record.identity.action);
    this.bytes -= record.bytes;
    const remaining = (this.groupBytes.get(record.owner.group) ?? 0) - record.bytes;
    if (remaining) this.groupBytes.set(record.owner.group, remaining); else this.groupBytes.delete(record.owner.group);
  }

  /** Pair key domains survive re-derivation during a live join. A second wrapper or failed
   * encryption cannot restart its nonce sequence. Native separately burns allocation at reserve. */
  burnSequence(owner: VoiceCryptoOwner, domain: string, sequence: bigint): void {
    current(owner);
    for (const [key, record] of this.sequences) if (!record.owner.isCurrent()) this.sequences.delete(key);
    const existing = this.sequences.get(domain);
    if (existing) {
      if (existing.owner.token !== owner.token || existing.used.has(sequence) || sequence <= existing.highest - 128n) throw new Error("voice packet nonce was already burned");
      // Native admission replies can reorder two valid encryptions. Retain a bounded window of
      // burned counters rather than rejecting every unused counter below the highest one.
      if (sequence > existing.highest) {
        existing.highest = sequence;
        for (const used of existing.used) if (used <= sequence - 128n) existing.used.delete(used);
      }
      existing.used.add(sequence);
      return;
    }
    let owned = 0;
    for (const record of this.sequences.values()) if (record.owner.token === owner.token) owned++;
    if (owned >= 16 || this.sequences.size >= 128) throw new Error("voice pair capacity reached; rejoin required");
    this.sequences.set(domain, { owner, highest: sequence, used: new Set([sequence]) });
  }
}

// Keep the historical slot name so a retained older broker is detected and refused, rather than
// creating a second pool while its old engine/native-commit promises still own capacity.
const BROKER = Symbol.for("mewtual.voice.crypto-broker.v2");

/** Keep the broker across component/HMR remounts. A version mismatch refuses rather than
 * creating a second pool while the previous pool may still own unresolved engine operations. */
export function windowVoiceCryptoBroker(
  native: VoiceCryptoAdmission, window: object = globalThis, subtle: SubtleCrypto = globalThis.crypto.subtle,
): VoiceCryptoBroker {
  const host = window as Record<symbol, unknown>;
  const previous = host[BROKER] as VoiceCryptoBroker | undefined;
  if (previous) {
    if (previous.version !== 4 || typeof previous.run !== "function" ||
        typeof previous.registerConfirmation !== "function" || typeof previous.createPair !== "function") {
      throw new Error("voice crypto broker version changed; reload required");
    }
    return previous;
  }
  const broker = new VoiceCryptoBroker(native, subtle); host[BROKER] = broker; return broker;
}

export type VoiceJoinKey = { privateKey: CryptoKey; publicKey: Uint8Array<ArrayBuffer> };

export async function createVoiceJoinKey(broker: VoiceCryptoBroker, owner: VoiceCryptoOwner): Promise<VoiceJoinKey> {
  return broker.run(owner, "generate", 128, async () => {
    const keys = await broker.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]);
    current(owner);
    const publicKey = new Uint8Array(await broker.subtle.exportKey("raw", keys.publicKey));
    current(owner);
    return { privateKey: keys.privateKey, publicKey };
  });
}

/** Exact native u64 direction counter; zero/overflow never wraps into a previously used IV. */
export function voicePacketIv(sequence: bigint): Uint8Array<ArrayBuffer> {
  if (sequence < 1n || sequence > 0xffff_ffff_ffff_ffffn) throw new Error("voice packet sequence exhausted");
  const iv = new Uint8Array(12); new DataView(iv.buffer).setBigUint64(4, sequence, false); return iv;
}

/** Keys stay opaque and non-extractable. Plaintext cannot leave open() until its exact native
 * incoming token is reauthorized, then the caller still checks its RTC peer/offer generation. */
export class VoiceCryptoPair {
  #tx: CryptoKey;
  #rx: CryptoKey;
  #broker: VoiceCryptoBroker;
  #owner: VoiceCryptoOwner;
  #domain: string;

  constructor(broker: VoiceCryptoBroker, owner: VoiceCryptoOwner, tx: CryptoKey, rx: CryptoKey, domain: string) {
    this.#broker = broker; this.#owner = owner; this.#tx = tx; this.#rx = rx;
    this.#domain = domain;
  }

  belongsTo(broker: VoiceCryptoBroker, owner: VoiceCryptoOwner): boolean { return this.#broker === broker && this.#owner === owner; }

  /** Internal engine seam. Only this module's broker can supply the private computation mark;
   * native admission and action/result custody have already been reserved before reaching it. */
  async computeConfirmation(mark: symbol, input: VoiceConfirmationInput): Promise<Uint8Array<ArrayBuffer>> {
    if (mark !== CONFIRM_COMPUTE) throw new Error("native confirmation computation required");
    current(this.#owner);
    return new Uint8Array(await this.#broker.subtle.decrypt({
      name: "AES-GCM", iv: voicePacketIv(u64(input.sequence, "sequence")),
      additionalData: input.aad as Uint8Array<ArrayBuffer>, tagLength: 128,
    }, this.#rx, input.ciphertext as Uint8Array<ArrayBuffer>));
  }

  encrypt(aad: Uint8Array, sequence: bigint, plaintext: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
    const header = boundedBytes(aad, MAX_CONTEXT, "header"), body = boundedBytes(plaintext, MAX_PLAINTEXT, "plaintext");
    const iv = voicePacketIv(sequence);
    return this.#broker.run(this.#owner, "encrypt", header.length + body.length, async () => {
      this.#broker.burnSequence(this.#owner, this.#domain, sequence);
      return new Uint8Array(await this.#broker.subtle.encrypt({ name: "AES-GCM", iv, additionalData: header, tagLength: 128 }, this.#tx, body));
    });
  }

  async open(aad: Uint8Array, sequence: bigint, ciphertext: Uint8Array, deliveryToken: string): Promise<Uint8Array<ArrayBuffer>> {
    const header = boundedBytes(aad, MAX_CONTEXT, "header"), body = boundedBytes(ciphertext, MAX_PLAINTEXT + 16, "ciphertext");
    const iv = voicePacketIv(sequence);
    // Keep the bounded window/native charge through final authorization as well as engine work.
    // Releasing it before this await would allow unbounded decrypted results awaiting native.
    const owned: { plaintext?: Uint8Array<ArrayBuffer> } = {};
    try {
      return await this.#broker.run(this.#owner, "decrypt", header.length + body.length, async () => {
        const opened = new Uint8Array(await this.#broker.subtle.decrypt({ name: "AES-GCM", iv, additionalData: header, tagLength: 128 }, this.#rx, body));
        owned.plaintext = opened;
        await this.#broker.commitInbound(this.#owner, deliveryToken, { aad: header, sequence, ciphertext: body }, opened);
        return opened;
      });
    } catch (error) {
      // run also performs a final ownership check AFTER work returns. Keep this reference outside
      // work so retirement in that last microtask boundary clears the withheld engine result too.
      owned.plaintext?.fill(0); throw error;
    }
  }
}

export async function deriveVoicePair(
  broker: VoiceCryptoBroker, owner: VoiceCryptoOwner, privateKey: CryptoKey,
  peerPublicKey: Uint8Array, nativeTranscript: Uint8Array, localIsLow: boolean,
): Promise<VoiceCryptoPair> {
  const point = boundedBytes(peerPublicKey, 65, "public key"), transcript = boundedBytes(nativeTranscript, MAX_CONTEXT, "transcript");
  if (point.length !== 65 || point[0] !== 4 || privateKey.type !== "private" || privateKey.extractable ||
      privateKey.algorithm.name !== "ECDH" || (privateKey.algorithm as EcKeyAlgorithm).namedCurve !== "P-256") {
    throw new Error("invalid voice join key");
  }
  return broker.run(owner, "derive", point.length + transcript.length + 128, async () => {
    const publicKey = await broker.subtle.importKey("raw", point, { name: "ECDH", namedCurve: "P-256" }, false, []);
    current(owner);
    const secret = new Uint8Array(await broker.subtle.deriveBits({ name: "ECDH", public: publicKey }, privateKey, 256));
    let base: CryptoKey;
    try { current(owner); base = await broker.subtle.importKey("raw", secret, "HKDF", false, ["deriveKey"]); }
    finally { secret.fill(0); }
    current(owner);
    const salt = new Uint8Array(await broker.subtle.digest("SHA-256", context("mewtual/voice/v2/transcript", transcript)));
    const direction = async (label: string, usage: KeyUsage) => {
      current(owner);
      return broker.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt, info: context(`mewtual/voice/v2/aes-gcm/${label}`, transcript) }, base, { name: "AES-GCM", length: 256 }, false, [usage]);
    };
    const tx = await direction(localIsLow ? "low-to-high" : "high-to-low", "encrypt");
    const rx = await direction(localIsLow ? "high-to-low" : "low-to-high", "decrypt");
    current(owner);
    const domain = Array.from(salt, byte => byte.toString(16).padStart(2, "0")).join("") + (localIsLow ? "/low" : "/high");
    return broker.createPair(owner, tx, rx, domain);
  });
}

export type VoicePacketReservation = { token: string; sequence: bigint; aad: Uint8Array };

/** One reservation, one encryption attempt. Inputs are frozen now, before any queued work.
 * Failures stay failed; exact retries return copies of cached ciphertext, never another seal. */
export function freezeVoicePacket(
  broker: VoiceCryptoBroker, owner: VoiceCryptoOwner, pair: VoiceCryptoPair,
  reservation: VoicePacketReservation, plaintext: Uint8Array,
): { encrypt(): Promise<Uint8Array<ArrayBuffer>> } {
  const aad = boundedBytes(reservation.aad, MAX_CONTEXT, "header"), body = boundedBytes(plaintext, MAX_PLAINTEXT, "plaintext");
  const sequence = reservation.sequence;
  voicePacketIv(sequence);
  if (!reservation.token) throw new Error("native packet reservation required");
  if (!pair.belongsTo(broker, owner)) throw new Error("voice pair belongs to another owner");
  let once: Promise<Uint8Array<ArrayBuffer>> | null = null;
  return { encrypt() {
    try { current(owner); } catch (error) { return Promise.reject(error); }
    // Set before yielding: even concurrent calls share a single attempt and nonce.
    once ??= pair.encrypt(aad, sequence, body);
    return once.then(ciphertext => { current(owner); return ciphertext.slice(); });
  } };
}
