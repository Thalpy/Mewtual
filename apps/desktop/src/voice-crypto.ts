import { VoiceAppSession, VOICE_APP_SESSION_BYTES, type VoiceAppCalls, type VoiceAppNative } from "./voice-app-session.ts";
import { VoiceHandleCell } from "./voice-handle-cell.ts";
import { VoiceSharedExecution } from "./voice-shared-execution.ts";
import { VoiceSdkPositions, type VoiceSdkPositionRequest } from "./voice-sdk-positions.ts";
/** Retained V2 endpoint crypto/native custody. App activation still requires production
 * adapter/RTC-track composition and acceptance. Native tickets and original scoped grants
 * are authority; WebCrypto alone neither authenticates a member nor authorizes a helper. */
import { parseVoiceIpcBootstrap, type VoiceIpcBootstrap } from "./voice-ipc-bootstrap.ts";
import {
  decodeVoiceIpcAdmission, encodeVoiceIpcAdmission, encodeVoiceIpcHeader,
  voiceIpcNonce, voiceIpcResponseAad, type VoiceIpcHeader,
} from "./voice-ipc-frame.ts";
import { readVoiceIpcResponse, readVoiceIpcSdpInputResponse } from "./voice-ipc-body.ts";
import { encodeVoiceIpcAction, encodeVoiceIpcConfirmationAction, type VoiceIpcPurpose, type VoiceIpcActionIdentity, type VoiceIpcActionRequest, type VoiceIpcConfirmationPurpose, type VoiceIpcConfirmationRequest } from "./voice-ipc-actions.ts";
import { decodeVoiceIpcDerivationInput, decodeVoiceIpcPermit } from "./voice-ipc-input.ts";
import { decodeVoiceIpcConfirmationInput } from "./voice-ipc-confirmation-input.ts";
import { decodeVoiceTransition, encodeVoiceTransition, type VoiceTransitionRequest } from "./voice-ipc-transition.ts";
import { decodeVoiceKeySession, encodeVoiceKeySession, type VoiceKeySessionRequest } from "./voice-ipc-key-session.ts";
import { VoiceKeyCustody } from "./voice-key-custody.ts";
import { VoiceSdpRtcOwner } from "./voice-sdp-rtc.ts";
import { VoiceRtcConfigurationOwner, VOICE_RTC_CONFIGURATION_BYTES, releaseVoiceRtcConfiguration,
  type VoiceRtcConfigurationLease, type VoiceRtcDeviceSettingsSource } from "./voice-rtc-configuration.ts";
import { VoiceNativeMedia, VOICE_NATIVE_MEDIA_BYTES, type VoiceNativeMediaContext } from "./voice-native-media.ts";
import { decodeVoiceMedia } from "./voice-ipc-media.ts";
import { VoiceSdpSdkCustody, VOICE_SDP_SDK_BYTES } from "./voice-sdp-sdk-custody.ts";
import { decodeVoiceSdpPhase, encodeVoiceSdpPhase, type VoiceSdpPhaseRequest } from "./voice-ipc-sdp.ts";
import { decodeVoiceSdkCustody, encodeVoiceSdkCustody, type VoiceSdkRequest } from "./voice-ipc-sdk.ts";
import { decodeVoicePcIntent, encodeVoicePcIntent } from "./voice-ipc-pc.ts";
import { encodeVoiceSdpAction, encodeVoiceSdpBegin, encodeVoiceSdpResolve } from "./voice-ipc-sdp-actions.ts";
import { decodeVoiceSdpReply } from "./voice-ipc-sdp-replies.ts";
import { decodeVoiceSdpEngineReply } from "./voice-ipc-sdp-engine.ts";
import type { VoiceSdpInput, VoiceSdpInputExpectation } from "./voice-ipc-sdp-input.ts";
import { VoiceSdpActionOwner, VoiceSdpInputLoan, voiceSdpUtf8 } from "./voice-sdp-action.ts";
import { decodeVoiceWorkSnapshot, type VoiceWorkDescriptor } from "./voice-ipc-work.ts";
import { NativeWorkPump, type VoiceNativeWorkObserver } from "./voice-ipc-pump.ts";
import { VoiceSessionLifecycle, VOICE_SESSION_LIFECYCLE_BYTES, type VoiceSessionLifecycleObserver,
  type VoiceLifecycleObservation, type VoiceLifecycleStage } from "./voice-session-lifecycle.ts";
import { decodeVoiceIpcActionReply, decodeVoiceIpcFinalReply, isVoiceIpcActionRow, type VoiceIpcActionReply } from "./voice-ipc-replies.ts";
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
  if (typeof token !== "string" || !token || token.length > 128 || typeof group !== "string" || !/^[0-9a-f]{64}$/.test(group)) throw new Error("invalid native voice owner");
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
// Node/OpenSSL's WebCrypto HKDF info limit is 1024 bytes, including our longest
// direction prefix. Current native pair_transcript is smaller; refuse larger projections
// before admission instead of permanently burning a derive identity at deriveKey.
const MAX_DERIVATION_CONTEXT = 1024 - encoder.encode("mewtual/voice/v2/aes-gcm/high-to-low\0").length;
type Job = { owner: VoiceCryptoOwner; bytes: number; start(): Promise<void>; refuse(): void; returned?(): void };
const CONFIRM_COMPUTE = Symbol("native confirmation computation");
const SDP_COMPUTE = Symbol("original native SDP computation");
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
/** An inactive native chain projection. These values are selected and checked on native;
 * constructing a browser object confers no native authority. Epoch/deadline remain exact u64s. */
export type VoiceCryptoChainBinding = Readonly<{
  chain: string; epoch: string; localDevice: string; localJoin: string; localKey: string;
  peerDevice: string; peerJoin: string; peerKey: string; handshake: string;
  generation: string; nativeDeadline: string; localIsLow: boolean;
}>;
export type VoiceDerivationInput = VoiceConfirmationIdentity & Readonly<{
  binding: VoiceCryptoChainBinding; localPublicKey: Uint8Array; peerPublicKey: Uint8Array;
  transcript: Uint8Array; engineBytes: number;
}>;
export type VoiceDerivationReceipt = VoiceConfirmationIdentity & Readonly<{
  kind: "accepted"; resultTag: string; receipt: string; derived: true;
}>;
type DerivationCommitReply = VoiceDerivationReceipt | Exclude<CommitReply, VoiceConfirmationReceipt>;
export interface VoiceDerivationNative {
  /** A terminal result may be returned before engine admission. A historical Derived marker
   * never reconstructs keys in a fresh browser. The empty output fingerprint is native-owned. */
  admitAction(identity: VoiceConfirmationIdentity, bytes: number): Promise<AdmissionReply | DerivationCommitReply>;
  commitDerivation(identity: VoiceConfirmationIdentity, resultTag: string): Promise<DerivationCommitReply>;
  settleAction(identity: VoiceConfirmationIdentity): Promise<void>;
}
export type VoiceDerivationOutcome =
  | Readonly<{ kind: "accepted"; receipt: VoiceDerivationReceipt }>
  | Exclude<VoiceConfirmationOutcome, { kind: "accepted" }>;
export type VoiceDerivationHandle = Readonly<{
  attempt(): Promise<VoiceDerivationOutcome>;
  inspect(): VoiceConfirmationStatus;
  /** Null for historical-only results, before native acceptance, and after plan/owner close. */
  pair(): VoiceCryptoPair | null;
  retire(): void;
}>;
export type VoiceChannelDerivationOutcome =
  | Readonly<{ kind: "ready"; chain: VoiceCryptoChain; derivation: VoiceDerivationHandle }>
  | VoiceIpcActionReply | Readonly<{ kind: "uncertain" | "retired" }>;
export type VoiceChannelDerivationHandle = Readonly<{ attempt(): Promise<VoiceChannelDerivationOutcome> }>;
/** Inactive continuation of the SAME original derivation chain. A first/lost Begin keeps
 * its original purpose and prepaid claim until authenticated history or parent-final cleanup
 * resolves it. Neither a historical result nor a cleanup proof confers RTC readiness. */
export type VoiceChannelConfirmationOutcome = VoiceConfirmationOutcome | VoiceIpcActionReply;
export type VoiceChannelConfirmationHandle = Readonly<{
  attempt(): Promise<VoiceChannelConfirmationOutcome>;
  /** Already authenticated native child RowId, or null before native adoption. A supplied
   * hint alone never appears here. This shares the original immutable string and conveys
   * no action, engine, receipt or Respond authority; native validates that independently. */
  row(): string | null;
}>;
/** Opaque: only the creating broker's WeakMap contains the non-extractable private key. */
export type VoiceJoinKeyHandle = Readonly<{ publicKey(): Uint8Array<ArrayBuffer> }>;
export type VoiceSelectedKeyAdmission = Readonly<{
  owner: string; executor: string; permit: string; join: string; kind: "admitted" | "closed" | "refused";
}>;
export type VoiceSelectedKeyPublication = Readonly<{
  owner: string; executor: string; permit: string; join: string; keyRef: string;
}>;
/** Fixed public bootstrap metadata only. Settlement resolves after actual native publication
 * drainage; it also fences a delayed admission when its permit reply was never received. */
export interface VoiceSelectedKeyNative {
  admit(owner: string, executor: string): Promise<VoiceSelectedKeyAdmission>;
  publish(owner: string, executor: string, permit: string, point: Uint8Array): Promise<VoiceSelectedKeyPublication>;
  settle(owner: string, executor: string): Promise<void>;
}
export type VoiceSelectedKeyOutcome = Readonly<{ kind: "accepted"; key: VoiceJoinKeyHandle }> |
  Readonly<{ kind: "uncertain" | "retired" | "failed" | "busy" | "unavailable" }>;
export type VoiceSelectedKeyHandle = Readonly<{ attempt(): Promise<VoiceSelectedKeyOutcome> }>;
export type VoiceCryptoChain = Readonly<{ close(): void; isCurrent(): boolean }>;
export type VoiceDerivationSetup = Readonly<{ localPublicKey: Uint8Array; peerPublicKey: Uint8Array; transcript: Uint8Array }>;

/** Projection of one actual native protected row. body is native plaintext for seal, signed
 * ciphertext for open. These data never create native authority; the adapter resolves the
 * original registered action again before signing/consuming its token. */
export type VoiceProtectedConfirmationInput = VoiceConfirmationIdentity & Readonly<{
  binding: VoiceCryptoChainBinding; row: number; operation: "seal" | "open";
  kind: ConfirmationKind; sequence: string; aad: Uint8Array; body: Uint8Array; engineBytes: number;
}>;
export interface VoiceProtectedConfirmationNative {
  admitAction(identity: VoiceConfirmationIdentity, bytes: number): Promise<AdmissionReply | CommitReply>;
  commitProtectedConfirmation(identity: VoiceConfirmationIdentity, output: Uint8Array, resultTag: string): Promise<CommitReply>;
  settleAction(identity: VoiceConfirmationIdentity): Promise<void>;
}
type ActionOutcome = VoiceConfirmationOutcome | VoiceDerivationOutcome;
type ActionHandle = Readonly<{
  attempt(): Promise<ActionOutcome>; inspect(): VoiceConfirmationStatus; retire(): void;
}>;
type JoinKeyRecord = {
  owner: VoiceCryptoOwner; privateKey: CryptoKey | null; point: Uint8Array<ArrayBuffer>;
  join: string | null; key: string | null;
};
type SelectedKeyRecord = {
  owner: VoiceCryptoOwner; key: JoinKeyRecord; native: VoiceSelectedKeyNative;
  executor: string; permit: string | null; join: string | null;
  attempt: Signal<VoiceSelectedKeyOutcome> | null; retry: Signal<void> | null;
  outcome: VoiceSelectedKeyOutcome | null; handle: VoiceSelectedKeyHandle;
  wake: object;
  started: boolean; drained: boolean; retired: boolean; released: boolean;
  channelUsers: number; jobReturned: boolean; closed: Signal<void>;
  unsubscribe: () => void;
};
// Application-owned generation state, key and fixed public-metadata projections. This is not
// a bound on opaque SDK/GC heaps or the stock framework's request/response copies.
const SELECTED_KEY_BYTES = 4096;
// The native setup reserves the matching SDK work and browser control bytes before Ready.
// These browser-side charges cover application-owned copies, not opaque SDK/GC heaps.
const IPC_SETUP_BYTES = 8 * 1024;
const IPC_CONTROL_BYTES = 8 * 1024;
// Serial public metadata read/parse plus the live parent's/current child's fixed handles,
// OR the deferred parent/current cleanup descriptor and proof. End joins the live driver
// and clears its graphs before cleanup uses this SAME payment. Reserve before setup ACK
// can enable native maintenance, including when End precedes first Input.
// This bounds application-owned graphs, not the framework/SDK/GC heaps.
const IPC_WORK_BYTES = 16 * 1024;
// Only exact, already-started cleanup is retried automatically. A completed refusal starts
// one new cadence interval; missed ticks cannot create catch-up work or overlap native RPCs.
const IPC_CLEANUP_RETRY_MS = 1_000;

export type VoiceIpcSetupReply =
  | Readonly<{ owner: string; executor: string; kind: "ready"; port: number; transcript: string }>
  | Readonly<{ owner: string; executor: string; kind: "pending" }>
  | Readonly<{ owner: string; executor: string; kind: "closed"; nativeSetupDrained: boolean }>;
/** Fixed public metadata through trusted main-window IPC only. settleSetup must close first,
 * recover after lock/End, and await the native worker. It never accepts a browser drain flag. */
export interface VoiceIpcNative {
  setup(owner: string, executor: string): Promise<VoiceIpcSetupReply>;
  settleSetup(owner: string, executor: string): Promise<void>;
  /** Exact final closure AFTER every real SDK/IO/cancel operation and local buffer disposal.
   * Native detaches its Session only after fencing new routing; live native replies retain
   * their original capacity until their actual last owner drains. Reply loss is retryable. */
  collectChannel(owner: string, executor: string): Promise<void>;
  /** Original-channel public metadata remains readable after End/lock for cleanup. */
  readWork(owner: string, executor: string, fromSlot: number): Promise<unknown>;
}
export type VoiceIpcStatus = Readonly<{ kind: "ready" | "closed"; nativeSetupDrained: boolean }> |
  Readonly<{ kind: "uncertain" | "retired" | "failed" }>;
/** Historical metadata only. Even localDrained/retired receipts grant no call, factory,
 * Workflow-finalization or unlink authority. Native history refusals do not close a Session. */
export type VoiceIpcHistoryOutcome = VoiceIpcActionReply |
  Readonly<{ kind: "uncertain" | "retired" | "failed" | "busy" }>;
type IpcHistoryRequest = {
  purpose: VoiceIpcPurpose; expectedRow: string | null;
  body: Uint8Array<ArrayBuffer>; result: Signal<VoiceIpcHistoryOutcome>;
};
export type VoiceIpcChannel = Readonly<{
  /** Coalesces the current physical flight. Later calls spend a fresh original-session TX. */
  status(): Promise<VoiceIpcStatus>;
  /** Resolves only after actual SDK/read/cancellation and exact native setup settlement. */
  close(): Promise<void>;
}>;
/** Browser monotonic IO seam. Expiry aborts real IO; it never settles/refunds a provider. */
export type VoiceIpcClock = Readonly<{
  now(): number; schedule(work: () => void, milliseconds: number): unknown; cancel(handle: unknown): void;
}>;
// A compacted peer keeps an inert plan/cell plus its registry scalars and completion
// wrappers. Transfer2KiB from original history; no fresh charge or cap increase. Inventory:
// cell128 + handle closures256 + completed dispatch128 + return observation128 + record256
// + cleanup promise128 + opaque generation64 + fee-return closures256 + map entry64 + slack640.
const COMPACT_PLAN_BYTES = 2 * 1024;
const MAX_RETAINED_MEDIA = 16; // Metadata ceiling; Root's32-Action join budget may refuse earlier.
const compactedHistory = Promise.resolve(true);
const voiceNoop = () => {};
type PreparedBrowserHistory = { commit(): number; abort(): void };
type ChainHandleOps = { close(): void; current(): boolean; retired(): void; timer(): void };
type ActionHandleOps = { attempt(): Promise<ActionOutcome>; inspect(): VoiceConfirmationStatus;
  retire(): void; pair(): VoiceCryptoPair | null };
type DerivationIntakeOps = { attempt(): Promise<VoiceChannelDerivationOutcome>; retire(): void; timer(): void };
type ConfirmationIntakeOps = { attempt(): Promise<VoiceChannelConfirmationOutcome>; row(): string | null };
const retiredAction = Promise.resolve(Object.freeze({ kind: "retired" } as const));
const retiredDerivationIntake = Promise.resolve(Object.freeze({ kind: "retired" } as const));
const retiredConfirmationIntake = Promise.resolve(Object.freeze({ kind: "retired" } as const));

// These factories MUST stay outside constructors/registration scopes. Merely reading
// cell.target in a closure created beside old input/record locals can retain that scope.
function chainHandle(cell: VoiceHandleCell<ChainHandleOps>): VoiceCryptoChain {
  return Object.freeze({ close: () => cell.dispatch(ops => ops.close(), undefined),
    isCurrent: () => cell.dispatch(ops => ops.current(), false) });
}
function chainRetirement(cell: VoiceHandleCell<ChainHandleOps>): () => void {
  return () => cell.dispatch(ops => ops.retired(), undefined);
}
function chainTimer(cell: VoiceHandleCell<ChainHandleOps>): () => void {
  return () => cell.dispatch(ops => ops.timer(), undefined);
}
function confirmationHandle(cell: VoiceHandleCell<ActionHandleOps>): VoiceConfirmationHandle {
  return Object.freeze({ attempt: () => cell.dispatch(ops => ops.attempt() as Promise<VoiceConfirmationOutcome>, retiredAction),
    inspect: () => cell.dispatch(ops => ops.inspect(), "retired" as const),
    retire: () => cell.dispatch(ops => ops.retire(), undefined) });
}
function derivationHandle(cell: VoiceHandleCell<ActionHandleOps>): VoiceDerivationHandle {
  return Object.freeze({ attempt: () => cell.dispatch(ops => ops.attempt() as Promise<VoiceDerivationOutcome>, retiredAction),
    inspect: () => cell.dispatch(ops => ops.inspect(), "retired" as const),
    retire: () => cell.dispatch(ops => ops.retire(), undefined), pair: () => cell.dispatch(ops => ops.pair(), null) });
}
function derivationIntakeHandle(cell: VoiceHandleCell<DerivationIntakeOps>): VoiceChannelDerivationHandle {
  return Object.freeze({ attempt: () => cell.dispatch(ops => ops.attempt(), retiredDerivationIntake) });
}
function derivationIntakeRetirement(cell: VoiceHandleCell<DerivationIntakeOps>): () => void {
  return () => cell.dispatch(ops => ops.retire(), undefined);
}
function derivationIntakeTimer(cell: VoiceHandleCell<DerivationIntakeOps>): () => void {
  return () => cell.dispatch(ops => ops.timer(), undefined);
}
function confirmationIntakeHandle(cell: VoiceHandleCell<ConfirmationIntakeOps>): VoiceChannelConfirmationHandle {
  return Object.freeze({ attempt: () => cell.dispatch(ops => ops.attempt(), retiredConfirmationIntake),
    row: () => cell.dispatch(ops => ops.row(), cell.row) });
}

type BrowserChain = {
  owner: VoiceCryptoOwner; key: JoinKeyRecord; binding: VoiceCryptoChainBinding;
  cell: VoiceHandleCell<ChainHandleOps>;
  expires: number; closed: boolean; historyDisposing: boolean; rowIssued: boolean; reservedRows: number;
  setup: VoiceDerivationSetup | null;
  running: boolean; execution: boolean; variable: boolean; fixedBytes: number;
  record: ActionRecord | null; unsubscribe: () => void; handle: VoiceCryptoChain;
  timer: ReturnType<typeof setTimeout> | null;
  records: Map<number, ActionRecord>; queue: ActionRecord[];
  retainedBytes: number; enginePeak: number;
  // Intake is physical work even before it becomes an ActionRecord. End must not refund the
  // original envelope while fetch/decrypt/parser/read cancellation or Return still uses it.
  continuation: ConfirmationIntake | null; confirmations: Map<string, ConfirmationIntake>;
  // A channel-linked chain shares the codec reservation and borrows the independent SDK
  // position only for actual admission/provider work; logical row serialization
  // remains separate so admission/result/settlement RPCs can use that SAME channel.
  channel: RetainedIpcChannel | null; origin: ChainIntake | null; drained: Signal<void>;
  // Same keys/payment; no effect, admission, SDP or RTC authority lives in this metadata.
  custody: VoiceKeyCustody | null;
  sdp: VoiceSdpSdkCustody | null;
  sdpRows: number;
  mediaIndexed: boolean;
};
type ConfirmationIntake = {
  cell: VoiceHandleCell<ConfirmationIntakeOps>;
  chain: BrowserChain; purpose: VoiceIpcConfirmationPurpose; row: string | null; identity: VoiceIpcActionIdentity | null;
  native: VoiceProtectedConfirmationNative | null; record: ActionRecord | null;
  handle: VoiceChannelConfirmationHandle; pending: Signal<VoiceChannelConfirmationOutcome> | null;
  running: boolean; returned: boolean; outcome: VoiceChannelConfirmationOutcome | null;
  // Fixed history is claimed at publication, before Input. Until an ActionRecord adopts
  // it, the original global preclaimed slot remains in broker.reservedRows too.
  rowClaim: boolean; rowAuthenticated: boolean;
};
/** The SAME prepaid claim becomes BrowserChain. No refund/recharge, new deadline, or second
 * group/execution position exists between admitting the Data read and publishing the chain. */
type ChainIntake = {
  cell: VoiceHandleCell<DerivationIntakeOps>;
  owner: VoiceCryptoOwner; key: JoinKeyRecord; channel: RetainedIpcChannel; generation: object; handleKey: VoiceJoinKeyHandle;
  purpose: VoiceIpcPurpose; row: string; identity: VoiceIpcActionIdentity; expires: number;
  transferred: boolean; retired: boolean; released: boolean; disposing: boolean; running: boolean;
  fixedBytes: number; variable: boolean; rows: number; unsubscribe: () => void; timer: unknown;
  pending: Promise<VoiceChannelDerivationOutcome> | null; outcome: VoiceChannelDerivationOutcome | null;
  handle: VoiceChannelDerivationHandle; drained: Signal<void>; retire(): void;
};
const CHAIN_ROWS = 12;
const CHAIN_HISTORY = 24 * 1024;
const CHAIN_METADATA = 8 * 1024;
const CHAIN_VARIABLE = 96 * 1024;
const EMPTY_RESULT_TAG = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
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

type ActionRecord = {
  cell: VoiceHandleCell<ActionHandleOps>;
  owner: VoiceCryptoOwner; pair: VoiceCryptoPair | null;
  identity: VoiceConfirmationIdentity; input: VoiceConfirmationInput | null; bytes: number;
  derivation: VoiceDerivationInput | null; chain: BrowserChain | null;
  protectedInput: VoiceProtectedConfirmationInput | null;
  status: VoiceConfirmationStatus; outcome: ActionOutcome | null;
  attempt: Signal<ActionOutcome> | null; retry: Signal<void> | null;
  job: Job | null; running: boolean; retired: boolean; drained: boolean; released: boolean; disposing: boolean;
  // Acceptance precedes settleAction's finally. Only the actual job/queued cancellation
  // resolves this one-shot fence; neither inspect() nor an accepted receipt supplies it.
  physicalDrained: Signal<void>;
  unsubscribe: () => void; handle: ActionHandle;
} & (
  | { operation: "confirmation"; native: VoiceConfirmationNative }
  | { operation: "derive"; native: VoiceDerivationNative }
  | { operation: "protected"; native: VoiceProtectedConfirmationNative }
);
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
function sameProtectedConfirmation(a: VoiceProtectedConfirmationInput, b: VoiceProtectedConfirmationInput): boolean {
  return a.owner === b.owner && a.action === b.action && a.version === b.version && a.inputTag === b.inputTag &&
    sameBinding(a.binding, b.binding) && a.row === b.row && a.operation === b.operation && a.kind === b.kind &&
    a.sequence === b.sequence && a.engineBytes === b.engineBytes && sameBytes(a.aad, b.aad) && sameBytes(a.body, b.body);
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
const BINDING_FIELDS = ["chain", "epoch", "localDevice", "localJoin", "localKey", "peerDevice", "peerJoin", "peerKey", "handshake", "generation", "nativeDeadline", "localIsLow"];
function validBinding(binding: VoiceCryptoChainBinding): boolean {
  if (!exactKeys(binding, BINDING_FIELDS) || typeof binding.localIsLow !== "boolean") return false;
  for (const field of ["localDevice", "localJoin", "localKey", "peerDevice", "peerJoin", "peerKey", "handshake"] as const) {
    if (typeof binding[field] !== "string" || !/^[0-9a-f]{64}$/.test(binding[field])) return false;
  }
  try {
    for (const field of ["chain", "generation", "nativeDeadline"] as const) u64(binding[field], field);
    // Epoch zero is valid; action IDs and generations start at one.
    if (typeof binding.epoch !== "string" || !/^(0|[1-9][0-9]{0,19})$/.test(binding.epoch) || BigInt(binding.epoch) > 0xffff_ffff_ffff_ffffn) return false;
  } catch { return false; }
  return true;
}
function sameBinding(a: VoiceCryptoChainBinding, b: VoiceCryptoChainBinding): boolean {
  return BINDING_FIELDS.every(field => a[field as keyof typeof a] === b[field as keyof typeof b]);
}
function sameDerivation(a: VoiceDerivationInput, b: VoiceDerivationInput): boolean {
  return a.owner === b.owner && a.action === b.action && a.version === b.version && a.inputTag === b.inputTag &&
    a.engineBytes === b.engineBytes && sameBinding(a.binding, b.binding) && sameBytes(a.localPublicKey, b.localPublicKey) &&
    sameBytes(a.peerPublicKey, b.peerPublicKey) && sameBytes(a.transcript, b.transcript);
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
  readonly version = 46;
  readonly subtle: SubtleCrypto;
  private readonly native: VoiceCryptoAdmission;
  private active = 0;
  private bytes = 0;
  private readonly groupBytes = new Map<string, number>();
  private readonly queue: Job[] = [];
  private readonly sequences = new Map<string, { owner: VoiceCryptoOwner; highest: bigint; used: Set<bigint> }>();
  // This map and its combined pending/terminal cap survive HMR with this exact broker.
  private readonly actions = new Map<string, ActionRecord>();
  private readonly joinKeys = new WeakMap<VoiceJoinKeyHandle, JoinKeyRecord>();
  private readonly keys = new Set<JoinKeyRecord>();
  private readonly selectedKeys = new Map<string, SelectedKeyRecord>();
  private appSession: { native: VoiceAppNative; selected: VoiceSelectedKeyNative; ipc: VoiceIpcNative; transport: typeof fetch;
    configurationSource: VoiceRtcDeviceSettingsSource | null; handle: VoiceAppCalls } | null = null;
  // Actual App-created native owner allocation only. Equal token/group labels cannot
  // install settings on a manual channel. Values contain no owner/Call/N backedge.
  private readonly appConfigurations = new WeakMap<VoiceCryptoOwner, VoiceRtcConfigurationOwner>();
  private readonly ownerRetirements = new WeakMap<VoiceCryptoOwner, Promise<void>>();
  private selectedExecutor: string | null = null;
  private readonly ipcChannels = new Map<string, RetainedIpcChannel>();
  private ipcPositions = 0;
  private ipcActive = 0;
  private ipcSdkActive = 0;
  // Sessions retain independent keys/counters but share one physical codec. The other
  // existing position can run SDK work without blocking Current on an established peer.
  private readonly ipcExecution = new VoiceSharedExecution({ begin() {}, end() {} });
  private readonly sdkPositions = new VoiceSdkPositions(() => this.acquireSdkPosition());
  private readonly chains = new WeakMap<VoiceCryptoChain, BrowserChain>();
  private readonly chainStamps = new Map<VoiceCryptoOwner, Map<string, BrowserChain>>();
  private readonly progress = new Map<string, BrowserChain | ChainIntake | VoiceSdpSdkCustody>();
  private readonly intakes = new Map<VoiceCryptoOwner, Map<string, ChainIntake>>();
  private reservedRows = 0;
  private protectedPositions = 0;
  private protectedActive = 0;
  private readonly now: () => number;
  private readonly ipcClock: VoiceIpcClock;

  constructor(native: VoiceCryptoAdmission, subtle: SubtleCrypto = globalThis.crypto.subtle, now: () => number = () => performance.now(),
    ipcClock: VoiceIpcClock = { now, schedule: (work, ms) => setTimeout(work, ms), cancel: handle => clearTimeout(handle as ReturnType<typeof setTimeout>) }) {
    this.native = native; this.subtle = subtle; this.now = now;
    this.ipcClock = ipcClock;
  }

  /** Reservations cover codec and SDK as a union, never three providers. With two
   * channels, SDK borrows their second reserved position; with one, it consumes the
   * spare position. Non-channel protected work keeps its original separate position. */
  private executionPositions(sdk = this.ipcSdkActive): number {
    const ordinary = this.active - this.protectedActive - this.ipcActive - this.ipcSdkActive;
    return ordinary + this.protectedPositions + Math.max(this.ipcPositions, this.ipcPositions ? 1 + sdk : 0);
  }
  private acquireSdkPosition(): (() => void) | null {
    if (!this.ipcPositions || this.ipcSdkActive || this.executionPositions(1) > MAX_ACTIVE) return null;
    // No callbacks between capacity comparison and ownership publication.
    this.active++; this.ipcSdkActive++;
    return () => { this.active--; this.ipcSdkActive--; this.pump(); };
  }

  private charge(owner: VoiceCryptoOwner, bytes: number): void {
    const group = this.groupBytes.get(owner.group) ?? 0;
    if (this.bytes + bytes > MAX_BYTES || group + bytes > MAX_GROUP_BYTES) throw new Error("voice crypto capacity reached");
    this.bytes += bytes; this.groupBytes.set(owner.group, group + bytes);
  }
  private refund(owner: VoiceCryptoOwner, bytes: number): void {
    this.bytes -= bytes;
    const remaining = (this.groupBytes.get(owner.group) ?? 0) - bytes;
    if (remaining) this.groupBytes.set(owner.group, remaining); else this.groupBytes.delete(owner.group);
  }

  /** Select one key for the original accepted native join. Repeated calls share the retained
   * job, including uncertain admission/publication/settlement. Compatibility fixture APIs below
   * deliberately still permit independent keys; upgraded callers must use this selection. */
  selectJoinKey(owner: VoiceCryptoOwner, native: VoiceSelectedKeyNative): VoiceSelectedKeyHandle {
    current(owner);
    const prior = this.selectedKeys.get(owner.token);
    if (prior) {
      if (prior.owner !== owner || prior.native !== native) throw new Error("voice key selection changed");
      return prior.handle;
    }
    if (this.keys.size >= 128 || [...this.keys].filter(key => key.owner === owner).length >= 16) throw new Error("voice join key capacity reached");
    if (!this.selectedExecutor) {
      // This identity lives with the retained broker, not a component. A new WebView cannot
      // adopt an earlier uncertain generation merely by repeating the native owner string.
      this.selectedExecutor = [...globalThis.crypto.getRandomValues(new Uint8Array(32))].map(n => n.toString(16).padStart(2, "0")).join("");
    }
    this.charge(owner, SELECTED_KEY_BYTES);
    const key: JoinKeyRecord = { owner, privateKey: null, point: new Uint8Array(), join: null, key: null };
    const record: SelectedKeyRecord = {
      owner, key, native, executor: this.selectedExecutor, permit: null, join: null,
      attempt: null, retry: null, outcome: null, handle: null!, started: false, drained: true,
      retired: false, released: false, unsubscribe: () => {}, wake: {},
      channelUsers: 0, jobReturned: true, closed: signal<void>(),
    };
    record.handle = Object.freeze({ attempt: () => this.attemptSelectedKey(record) });
    this.selectedKeys.set(owner.token, record); this.keys.add(key);
    try {
      record.unsubscribe = owner.onRetired(() => {
        record.retired = true;
        this.wakeSelectedKey(record);
        record.attempt?.resolve(Object.freeze({ kind: "retired" })); record.attempt = null;
        if (record.drained) this.releaseSelectedKey(record);
      });
    } catch (error) { this.releaseSelectedKey(record); throw error; }
    return record.handle;
  }

  private releaseSelectedKey(record: SelectedKeyRecord): void {
    // End cannot erase/refund the original selected key while an actual local-channel SDK
    // promise still borrows it. Keep its exact lookup/payment through actual queue-job
    // return too: a first teardown request after map removal must not miss an old worker.
    // The channel releases its separate borrow after real drainage.
    if (record.released || record.channelUsers || !record.jobReturned) return;
    record.released = true; record.unsubscribe();
    record.key.privateKey = null; record.key.point.fill(0);
    this.keys.delete(record.key); this.selectedKeys.delete(record.owner.token);
    this.refund(record.owner, SELECTED_KEY_BYTES);
    if (record.jobReturned) record.closed.resolve();
  }

  private attemptSelectedKey(record: SelectedKeyRecord): Promise<VoiceSelectedKeyOutcome> {
    // Old handles can wake their sole cleanup owner even after End. They never restart crypto.
    this.wakeSelectedKey(record);
    if (record.retired || !record.owner.isCurrent()) return Promise.resolve(Object.freeze({ kind: "retired" }));
    if (record.outcome) return Promise.resolve(record.outcome);
    if (record.attempt) return record.attempt.promise;
    record.attempt = signal<VoiceSelectedKeyOutcome>();
    const attempt = record.attempt.promise;
    if (record.started) return attempt;
    if (this.queue.length >= MAX_QUEUED) {
      record.attempt.resolve(Object.freeze({ kind: "busy" })); record.attempt = null; return attempt;
    }
    try { this.charge(record.owner, 128); }
    catch { record.attempt.resolve(Object.freeze({ kind: "busy" })); record.attempt = null; return attempt; }
    record.started = true; record.drained = false; record.jobReturned = false;
    const finish = () => {
      record.drained = true;
      if (record.retired || !record.owner.isCurrent()) this.releaseSelectedKey(record);
    };
    this.queue.push({ owner: record.owner, bytes: 128,
      refuse: () => { record.retired = true; record.attempt?.resolve(Object.freeze({ kind: "retired" })); record.attempt = null; finish(); },
      start: async () => { try { await this.driveSelectedKey(record); } finally { finish(); } },
      returned: () => {
        record.jobReturned = true; this.refund(record.owner, 128);
        if (record.retired || !record.owner.isCurrent()) this.releaseSelectedKey(record);
        if (record.released) record.closed.resolve();
      },
    });
    this.pump(); return attempt;
  }

  private wakeSelectedKey(record: SelectedKeyRecord): void {
    record.wake = {}; record.retry?.resolve();
  }

  /** One retained application session fa?ade. Its fixed8KiB metadata graph is paid
   * BEFORE construction/invoke and stays process-paid for this broker/window lifetime.
   * A call binds the SAME reservation to its native group before key admission; only
   * actual outer/native/broker cleanup can remove that group charge. No other fee or
   * execution allowance is enlarged. Optional device ICE settings have their separate
   *64KiB configuration inventory, not spare bytes borrowed from this8KiB graph. */
  nativeAppCalls(native: VoiceAppNative, selected: VoiceSelectedKeyNative, ipc: VoiceIpcNative,
    transport: typeof fetch = globalThis.fetch): VoiceAppCalls {
    return this.appCalls(native, selected, ipc, transport, null);
  }
  /** Explicit local-device configuration factory. An older retained broker cannot
   * silently ignore an extra argument and construct direct peers for relay-only intent. */
  nativeConfiguredAppCalls(native: VoiceAppNative, selected: VoiceSelectedKeyNative, ipc: VoiceIpcNative,
    configurationSource: VoiceRtcDeviceSettingsSource, transport: typeof fetch = globalThis.fetch): VoiceAppCalls {
    if (typeof configurationSource !== "function") throw new Error("invalid voice RTC settings source");
    return this.appCalls(native, selected, ipc, transport, configurationSource);
  }
  private appCalls(native: VoiceAppNative, selected: VoiceSelectedKeyNative, ipc: VoiceIpcNative,
    transport: typeof fetch, configurationSource: VoiceRtcDeviceSettingsSource | null): VoiceAppCalls {
    const previous = this.appSession;
    if (previous) {
      if (previous.native !== native || previous.selected !== selected || previous.ipc !== ipc || previous.transport !== transport ||
          previous.configurationSource !== configurationSource)
        throw new Error("voice application transport changed; reload required");
      return previous.handle;
    }
    if (configurationSource !== null && typeof configurationSource !== "function") throw new Error("invalid voice RTC settings source");
    const metadataBytes = VOICE_APP_SESSION_BYTES + (configurationSource ? VOICE_RTC_CONFIGURATION_BYTES : 0);
    if (this.bytes + metadataBytes > MAX_BYTES) throw new Error("voice application capacity reached");
    this.bytes += metadataBytes; // Both separate inventories paid BEFORE either constructor/callback.
    const configuration = configurationSource ? new VoiceRtcConfigurationOwner() : null;
    let configuredOwner: VoiceCryptoOwner | null = null;
    let configurationRetirement: Promise<void> | null = null;
    let group: string | null = null;
    const session = new VoiceAppSession(native, {
      clock: this.ipcClock,
      bindGroup: value => {
        if (group !== null || !/^[0-9a-f]{64}$/.test(value)) throw new Error("voice application group changed");
        const bytes = this.groupBytes.get(value) ?? 0;
        if (bytes + metadataBytes > MAX_GROUP_BYTES) throw new Error("voice application group capacity reached");
        this.groupBytes.set(value, bytes + metadataBytes); group = value;
      },
      unbindGroup: () => {
        if (group === null) return;
        // All actual broker/RTC/media joins precede this tail. A busy snapshot keeps
        // its original group fee; never refund while a failed SDK close still owns it.
        configuration?.reset();
        if (configuredOwner) this.appConfigurations.delete(configuredOwner);
        configuredOwner = null;
        configurationRetirement = null;
        const bytes = this.groupBytes.get(group)! - metadataBytes;
        if (bytes) this.groupBytes.set(group, bytes); else this.groupBytes.delete(group);
        group = null;
      },
      owner: createVoiceCryptoOwner,
      select: owner => {
        if (configuration && configurationSource) {
          if (group === null || owner.group !== group || configuredOwner !== null)
            throw new Error("original voice RTC settings owner required");
          configuration.capture(configurationSource, () => owner.isCurrent());
          configuredOwner = owner;
          this.appConfigurations.set(owner, configuration);
        }
        return this.selectJoinKey(owner, selected);
      },
      channel: (owner, key) => this.nativeChannel(owner, key, ipc, transport),
      observe: (owner, channel, key) => this.observeNativeLifecycle(owner, channel, key),
      retire: owner => {
        if (!configuration) return this.retireNativeOwner(owner);
        configuration.close(); // Before native/SDK callbacks, including capture reentry.
        if (configurationRetirement) return configurationRetirement;
        let resolve!: () => void, reject!: (reason: unknown) => void;
        const completion = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
        configurationRetirement = completion; // BEFORE owner retirement can reenter.
        try { void this.retireNativeOwner(owner).then(() => configuration.join()).then(resolve, reject); }
        catch (error) { reject(error); }
        return completion;
      },
      resume: () => this.retrySelectedJoinKeys(),
    });
    this.appSession = { native, selected, ipc, transport, configurationSource, handle: session.handle };
    return session.handle;
  }

  /** Trusted call-session teardown. Observer outcomes (including `retired`) are NOT
   * actual key/SDK/native settlement completion. Find exact internally published owners,
   * including a channel whose handle never reached its caller, then join outside the
   * shared codec. Publish completion before retirement callbacks can synchronously reenter.
   * Availability wakes retry only original cleanup; callers must keep their own fixed
   * session graph paid until this promise and their ordinary native End have returned. */
  retireNativeOwner(owner: VoiceCryptoOwner): Promise<void> {
    const previous = this.ownerRetirements.get(owner); if (previous) return previous;
    const selected = this.selectedKeys.get(owner.token), channel = this.ipcChannels.get(owner.token);
    if ((selected && selected.owner !== owner) || (channel && !channel.owns(owner)))
      throw new Error("original native voice owner required");
    const work = Promise.resolve().then(async () => {
      // Channel collection releases its original selected-key borrow. Neither join is
      // entered while owning SharedIpcExecution, and actual job return remains separate
      // from its earlier finally/outcome publication.
      if (channel) await channel.handle.close();
      if (selected) await selected.closed.promise;
    });
    this.ownerRetirements.set(owner, work);
    owner.retire();
    this.retrySelectedJoinKeys();
    return work;
  }

  /** One bounded availability wake for existing flights, including orphan cleanup after End.
   * Remount/native availability can recover them without retaining every component handle.
   * This never queues a fresh generation, repeats crypto, or starts a retry timer. */
  retrySelectedJoinKeys(): void {
    for (const record of this.selectedKeys.values()) {
      if (record.started && !record.drained) this.wakeSelectedKey(record);
    }
    for (const channel of this.ipcChannels.values()) channel.wake();
  }

  /** The only browser Session constructor consumes this broker's EXACT accepted selected
   * key/executor. Generic bound keys, supplied seeds/points or equal owner strings cannot
   * reconstruct it. The original channel and its counters survive component/HMR remounts.
   * Setup status and historical reads grant no Root, remote-member, action, SDP or RTC authority. */
  nativeChannel(owner: VoiceCryptoOwner, handle: VoiceJoinKeyHandle, native: VoiceIpcNative,
    transport: typeof fetch = globalThis.fetch): VoiceIpcChannel {
    current(owner);
    const selected = this.selectedKeys.get(owner.token);
    const key = this.joinKeys.get(handle);
    if (!selected || selected.owner !== owner || selected.key !== key || !key.privateKey ||
        selected.outcome?.kind !== "accepted" || selected.outcome.key !== handle ||
        !key.join || !key.key) throw new Error("selected voice key required");
    const previous = this.ipcChannels.get(owner.token);
    if (previous) {
      if (!previous.belongsTo(selected, native, transport)) throw new Error("voice channel identity changed");
      return previous.handle;
    }
    // Reserve from the existing two execution positions BEFORE the first native setup call.
    // Ordinary queue admission here would deadlock behind protected chains awaiting control.
    if (this.ipcChannels.size >= 32 ||
        this.executionPositions() >= MAX_ACTIVE) {
      throw new Error("voice channel capacity reached");
    }
    this.charge(owner, IPC_SETUP_BYTES + IPC_CONTROL_BYTES + IPC_WORK_BYTES);
    this.ipcPositions++; selected.channelUsers++;
    let setupPaid = true, position = true, historyPaid = true, compactPaid = 0, lifecyclePaid = false;
    const channel = new RetainedIpcChannel(selected, native, transport, this.subtle, this.ipcClock, {
      rtcConfiguration: () => this.appConfigurations.get(owner)?.borrow() ?? null,
      execute: work => this.ipcExecution.run(work),
      reserveSdk: () => this.sdkPositions.request(),
      lifecyclePrepared: () => {
        if (!lifecyclePaid) {
          this.charge(owner, VOICE_SESSION_LIFECYCLE_BYTES); lifecyclePaid = true;
        }
      },
      begin: () => { this.active++; this.ipcActive++; },
      end: () => { this.active--; this.ipcActive--; this.pump(); },
      setupDrained: () => { if (setupPaid) { setupPaid = false; this.refund(owner, IPC_SETUP_BYTES); } },
      closed: () => {
        if (position) { position = false; this.ipcPositions--; selected.channelUsers--; this.pump(); }
        if (selected.retired && selected.drained) this.releaseSelectedKey(selected);
      },
      collected: () => {
        // Observer stop does not refund its Session graph; collection also waits for
        // the actual coordinator/timer/IO/SDK return through channel activity.
        if (lifecyclePaid) { lifecyclePaid = false; this.refund(owner, VOICE_SESSION_LIFECYCLE_BYTES); }
        if (compactPaid) { this.refund(owner, compactPaid * COMPACT_PLAN_BYTES); compactPaid = 0; }
        if (historyPaid) { historyPaid = false; this.refund(owner, IPC_CONTROL_BYTES + IPC_WORK_BYTES); }
        this.ipcChannels.delete(owner.token);
      },
      prepareCompaction: async chain => {
        if (compactPaid >= MAX_RETAINED_MEDIA) throw new Error("voice peer history capacity reached");
        const prepared = await this.prepareChainCompaction(chain);
        return { abort: () => prepared.abort(), commit: () => {
          // Ownership transfers directly from the old fixed reservation. There is no
          // refund/recharge gap in which another admission can spend these same bytes.
          const refund = prepared.commit(); compactPaid++; return refund;
        } };
      },
      compactionReturned: bytes => this.refund(owner, bytes),
      failedHistoryRetained: chain => {
        // Native Plan-final/ACK and the peer worker have really returned. Keep every
        // fixed history fee, but sever adapters that capture the old complete N. Old
        // retired public handles cannot start effects, and End disposal needs no adapter.
        for (const record of chain.records.values()) {
          if (!record.drained || record.running || record.bytes !== 0) throw new Error("failed action still running");
        }
        for (const child of chain.confirmations.values()) {
          if (child.running || (child.record && !child.record.drained)) throw new Error("failed child still running");
        }
        for (const record of chain.records.values()) {
          // A completed Job template also retains its native adapter closures. Actual
          // physicalDrained/worker return is proved above; this is inert history only.
          record.native = null!; record.job = null;
        }
        for (const child of chain.confirmations.values()) {
          // Exact parent Plan-final/ACK supersedes an absent child or a lost ordinary
          // Return observation, AFTER its actual local work returned. This is fixed
          // negative history, never a new child Begin/admission or effect permission.
          child.returned = true; child.outcome = Object.freeze({ kind: "retired" }); child.native = null;
        }
      },
      unlinked: chain => {
        // Actual native final ACK and channel collection have removed the last channel
        // reference. Owner End alone cannot release these still-retained fixed objects.
        if (!chain.owner.isCurrent()) this.releaseChainHistory(chain);
      },
      sdpUnlinked: (chain, phase) => {
        // Runs AFTER the execution lane's actual finally. Neither End nor native storage
        // state3 refunds an outstanding RPC/WebCrypto/RTC promise or its retained graph.
        if (chain.sdp !== phase || !phase.nativeReleased) throw new Error("voice SDK custody remains unresolved");
        chain.sdp = null;
        this.reservedRows -= chain.sdpRows; chain.sdpRows = 0;
        if (this.progress.get(owner.group) === phase) this.progress.delete(owner.group);
        this.refund(owner, VOICE_SDP_SDK_BYTES);
        if (!chain.owner.isCurrent()) this.releaseChainHistory(chain);
      },
      finishInput: chain => this.retainFinishCustody(chain),
      transitionUpdated: chain => { this.drainChain(chain); if (this.chainCurrent(chain)) this.armChain(chain); },
      mediaPrepared: () => {
        this.charge(owner, VOICE_NATIVE_MEDIA_BYTES);
        return originalMediaFee(this.refund.bind(this, owner, VOICE_NATIVE_MEDIA_BYTES));
      },
    });
    this.ipcChannels.set(owner.token, channel);
    try { channel.observe(); }
    catch (error) { channel.abandon(); throw error; }
    return channel.handle;
  }

  /** Recover existing action history on this broker's EXACT retained channel. Establish it
   * with status() first: this method never constructs/reconstructs keys or starts setup.
   * One paid Control flight coalesces identical requests; another purpose returns busy.
   * Null expectedRow is only for the first Begin reply, which authenticates the unknown ID. */
  recoverChannelHistory(owner: VoiceCryptoOwner, handle: VoiceIpcChannel,
    purpose: VoiceIpcPurpose, expectedRow: string | null = null): Promise<VoiceIpcHistoryOutcome> {
    const channel = this.ipcChannels.get(owner.token);
    // Compare actual original allocations BEFORE any owner predicate/callback can run.
    if (!channel || channel.handle !== handle || !channel.owns(owner)) throw new Error("voice channel identity changed");
    return channel.history(purpose, expectedRow);
  }

  /** Release only the completed Derive phase so native maintenance can select the first
   * confirmation work. Preserve the original chain, keys, counters and deadline. The
   * retained channel coalesces this actual settlement/Return flight; a caller disappearing
   * or owner End cannot turn an outstanding SDK/RPC into drainage. */
  returnNativeDerivation(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<void> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !chain || chain.owner !== owner ||
        chain.channel !== channel || !chain.record || chain.record.operation !== "derive") {
      throw new Error("original native derivation required");
    }
    return channel.finishDerivation(chain);
  }

  /** Prepay one browser SDK/storage subtree for the exact established native chain.
   * This creates release obligations only: no PC, SDP computation or RTC effect is exposed.
   * Promise<void> joins an attempted/coalesced flight, not funded storage or RTC readiness;
   * a busy channel may defer the attempt. Exact subsequent calls recover the same intent.
   * Browser old-envelope drainage does NOT prove native core224KiB/Workflow drainage;
   * the production round scheduler must establish that separately before using this leaf. */
  prepareNativeSdpHandoff(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<void> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !chain || chain.owner !== owner || chain.channel !== channel) {
      throw new Error("original native SDP chain required");
    }
    if (chain.sdp) return channel.startSdp(chain);
    current(owner);
    if (!this.chainCurrent(chain) || chain.variable || chain.running || chain.continuation?.running ||
        chain.custody?.keyReceipt?.state !== "current" || this.progress.has(owner.group)) throw new Error("established drained voice chain required");
    if (this.actions.size + this.reservedRows + 2 > MAX_ACTIONS) throw new Error("voice action history capacity reached");
    this.charge(owner, VOICE_SDP_SDK_BYTES);
    // No callback/await between payment and publication. Unknown replies retain this SAME
    // intent; remounts cannot allocate a second subtree or another admission flight.
    const phase = new VoiceSdpSdkCustody(); chain.sdp = phase; chain.sdpRows = 2;
    this.reservedRows += 2; this.progress.set(owner.group, phase);
    return channel.startSdp(chain);
  }

  /** Private first-round SDK vertical. A nonempty description is LOW's original Offer;
   * null discovers HIGH's incoming Offer. This returns only joined progress, never plaintext,
   * a receipt-as-permission, or RTC authority. Exact repeated calls join the original owner;
   * use null for retry after its first description was synchronously consumed. Production
   * native SDP remains disabled until delivery/RTC and complete composition are integrated. */
  prepareNativeSdpCrypto(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain, description: string | null): Promise<void> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !chain || chain.owner !== owner ||
        chain.channel !== channel || !chain.sdp || chain.sdpRows !== 2) throw new Error("original native SDP custody required");
    return channel.startSdpCrypto(chain, description);
  }

  /** Private concrete first-Offer RTC path. Returns only a joined void attempt. The
   * original retained channel owns one actual SDK peer; App V2 activation is still closed.
   * LOW prepares its local gathered description after fresh14; HIGH applies Open after
   * fresh26. No public description promise or reusable receipt permission is exposed. */
  prepareNativeSdpRtc(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<void> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !chain || chain.owner !== owner ||
        chain.channel !== channel || !chain.sdp || chain.sdpRows !== 2) throw new Error("original native RTC custody required");
    return channel.startRtc(chain);
  }

  /** Compose one original Offer/Answer round with serial action-local compaction. A void
   * result is joined progress, not connected media: repeat the SAME call after a retryable
   * refusal. No new PC, Ready window, key, row allowance or public RTC handle is created. */
  completeNativeSdpRtcRound(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<void> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !chain || chain.owner !== owner ||
        chain.channel !== channel || !chain.sdp || chain.sdpRows !== 2) throw new Error("original native RTC custody required");
    return channel.startRtc(chain, true);
  }

  /** Complete the original one-peer native media handoff. A void result joins an
   * acquisition attempt; it exposes no peer/receipt permission. The independent media
   * driver stays paid through actual cleanup. Old graph compaction requires its own
   * exact native-history and browser-return gates; multiparty admission remains closed. */
  completeNativeMediaHandoff(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<void> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner)) throw new Error("original native media custody required");
    if (channel.compacted(plan)) return channel.compactedMedia(plan);
    if (!chain || chain.owner !== owner || chain.channel !== channel) throw new Error("original native media custody required");
    return channel.startMedia(chain);
  }

  /** Retire original negotiation after authenticated independent custody and actual
   * Source/Lower retirement. The media peer and shared physical Session stay live.
   * Fixed old history remains paid until explicit joined browser compaction or End.
   * This does not admit another negotiation on the shared physical Session. */
  retireNativeNegotiation(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<void> {
    const channel=this.ipcChannels.get(owner.token), chain=this.chains.get(plan);
    if (!channel || channel.handle!==handle || !channel.owns(owner)) throw new Error("original native negotiation required");
    if (channel.compacted(plan)) return Promise.resolve();
    if (!chain || chain.owner!==owner || chain.channel!==channel) throw new Error("original native negotiation required");
    return channel.retireNegotiation(chain);
  }

  /** Negative cleanup of one attached original plan, before any per-N media record. No history
   * refund or new admission follows a void result; exact final/ACK and actual worker return
   * are separately required. Unknown native publication is deliberately not covered. */
  retireFailedNativeNegotiation(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<void> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !chain || chain.owner !== owner || chain.channel !== channel)
      throw new Error("original attached native plan required");
    return channel.retireFailedNegotiation(chain);
  }

  /** Recover exact native terminal-history collection. True is historical evidence
   * for this original claim only; it is not currentness or a browser-history refund. */
  collectNativeNegotiationHistory(owner: VoiceCryptoOwner, handle: VoiceIpcChannel,
    plan: VoiceCryptoChain): Promise<boolean> {
    const channel=this.ipcChannels.get(owner.token), chain=this.chains.get(plan);
    if (!channel || channel.handle!==handle || !channel.owns(owner)) throw new Error("original native history required");
    if (channel.compacted(plan)) return compactedHistory;
    if (!chain || chain.owner!==owner || chain.channel!==channel) throw new Error("original native history required");
    return channel.collectNegotiationHistory(chain);
  }

  /** Reclaim the old browser graph only after exact native history collection and actual
   * negotiation drainage. Keeps the SAME media/Session and one paid inert plan marker.
   * True is historical collection, never another peer admission or fresh RTC authority. */
  compactNativeNegotiation(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<boolean> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner)) throw new Error("original browser history required");
    if (channel.compacted(plan)) return compactedHistory;
    if (!chain || chain.owner !== owner || chain.channel !== channel) throw new Error("original browser history required");
    return channel.compactNegotiation(chain);
  }

  /** Attempt the original native PC intent only after separate SDK/storage custody.
   * Returns no effect capability or RTC object. Like prepareNativeSdpHandoff, this joins
   * an attempted/coalesced flight; a busy channel defers it. Unknown replies remain on
   * the same paid subtree and must be closed before SDK/storage/key release. */
  prepareNativePcIntent(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): Promise<void> {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !chain || chain.owner !== owner ||
        chain.channel !== channel || !chain.sdp) throw new Error("original native PC custody required");
    return channel.startPc(chain);
  }

  /** Start one retained serial consumer after the original channel's authenticated Ready.
   * Remounts share it; closing it or ending the channel is irreversible. Public metadata
   * cannot mint a factory, reconstruct keys, select another parent or authorize RTC. */
  observeNativeWork(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, key: VoiceJoinKeyHandle): VoiceNativeWorkObserver {
    const channel = this.ipcChannels.get(owner.token), selected = this.selectedKeys.get(owner.token);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !selected ||
        selected.owner !== owner || selected.outcome?.kind !== "accepted" || selected.outcome.key !== key ||
        this.joinKeys.get(key) !== selected.key) throw new Error("original selected voice channel required");
    return channel.observeWork(this, key);
  }

  /** One separately prepaid Session coordinator. Remounts coalesce. close stops
   * scheduling, while channel End still owns pending/native/media obligations.
   * Scheduling observations are never native permission. App activation stays OFF. */
  observeNativeLifecycle(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, key: VoiceJoinKeyHandle): VoiceSessionLifecycleObserver {
    const channel = this.ipcChannels.get(owner.token), selected = this.selectedKeys.get(owner.token);
    if (!channel || channel.handle !== handle || !channel.owns(owner) || !selected ||
        selected.owner !== owner || selected.outcome?.kind !== "accepted" || selected.outcome.key !== key ||
        this.joinKeys.get(key) !== selected.key) throw new Error("original selected voice channel required");
    return channel.observeLifecycle(this, key);
  }

  /** Start a fresh browser negotiation after the actual previous compaction observer
   * returned. This grants no native action/RTC authority and never resets Session nonces. */
  advanceNativeNegotiation(owner: VoiceCryptoOwner, handle: VoiceIpcChannel, plan: VoiceCryptoChain): boolean {
    const channel = this.ipcChannels.get(owner.token);
    if (!channel || channel.handle !== handle || !channel.owns(owner)) throw new Error("original voice channel required");
    return channel.advanceNegotiation(plan);
  }

  /** Reserve the original group/row/variable position BEFORE Input fetch or materialization.
   * Exact retries keep this handle, its start deadline and its affine claim. A historical
   * Derived response is observation only and never reconstructs keys in a fresh browser. */
  nativeDerivationFromChannel(owner: VoiceCryptoOwner, channelHandle: VoiceIpcChannel, handle: VoiceJoinKeyHandle,
    purpose: VoiceIpcPurpose, row: string, identity: VoiceIpcActionIdentity, expectedGeneration?: object): VoiceChannelDerivationHandle {
    const channel = this.ipcChannels.get(owner.token), key = this.joinKeys.get(handle);
    if (!channel || channel.handle !== channelHandle || !channel.owns(owner) || !key || key.owner !== owner || !isVoiceIpcActionRow(row)) throw new Error("original voice intake required");
    const generation = expectedGeneration ?? channel.generation();
    const selected = Object.freeze({ peer: purpose.peer, join: purpose.join, keyRef: purpose.keyRef, epoch: purpose.epoch });
    const original = Object.freeze({ action: identity.action, version: identity.version, inputTag: identity.inputTag });
    const validation = encodeVoiceIpcAction(selected, { kind: "admit", identity: original, engineBytes: 0 }); validation.body.fill(0);
    current(owner);
    const prior = () => {
      if (!channel.matchesGeneration(generation)) throw new Error("old voice intake generation");
      const record = this.intakes.get(owner)?.get(row);
      if (record && (record.key !== key || record.channel !== channel || record.purpose.peer !== selected.peer || record.purpose.join !== selected.join ||
          record.purpose.keyRef !== selected.keyRef || record.purpose.epoch !== selected.epoch || record.identity.action !== original.action ||
          record.identity.version !== original.version || record.identity.inputTag !== original.inputTag)) throw new Error("voice intake identity changed");
      return record;
    };
    let previous = prior(); if (previous) return previous.handle;
    const expires = this.now() + 15_000;
    if (!Number.isFinite(expires) || !channel.canLend(owner, key, generation) || !channel.canLink(null, generation)) throw new Error("ready voice channel required");
    // Predicates/Clock above may reenter. Repeat ownership/capacity after their last callback.
    current(owner); previous = prior(); if (previous) return previous.handle;
    if (!channel.canLink(null, generation)) throw new Error("ready voice channel required");
    if ((this.intakes.get(owner)?.size ?? 0) >= 16 || this.progress.has(owner.group) ||
        this.actions.size + this.reservedRows + CHAIN_ROWS > MAX_ACTIONS) throw new Error("voice intake capacity reached");
    const fixed = CHAIN_HISTORY + CHAIN_METADATA;
    this.charge(owner, fixed + CHAIN_VARIABLE);
    const intake: ChainIntake = { cell: null!, owner, key, channel, generation, handleKey: handle, purpose: selected, row, identity: original, expires,
      transferred: false, retired: false, released: false, disposing: false, running: false, fixedBytes: fixed, variable: true, rows: CHAIN_ROWS,
      unsubscribe: () => {}, timer: null, pending: null, outcome: null, handle: null!, drained: signal<void>(),
      retire: null! };
    intake.cell = new VoiceHandleCell({ attempt: () => this.attemptIntake(intake),
      retire: () => { intake.retired = true; channel.cancelIntake(intake); this.releaseIntake(intake); },
      timer: () => this.expireIntake(intake) });
    intake.retire = derivationIntakeRetirement(intake.cell);
    intake.handle = derivationIntakeHandle(intake.cell);
    this.reservedRows += CHAIN_ROWS; this.progress.set(owner.group, intake);
    const records = this.intakes.get(owner) ?? new Map<string, ChainIntake>(); records.set(row, intake); this.intakes.set(owner, records);
    // Publish before any subscription/scheduler callback can run. End never loses a claim.
    try {
      channel.reserveIntake(intake);
      intake.unsubscribe = owner.onRetired(intake.retire);
      if (!intake.retired) intake.timer = this.ipcClock.schedule(derivationIntakeTimer(intake.cell), 15_000);
    } catch (error) { intake.retired = true; this.releaseIntake(intake); throw error; }
    return intake.handle;
  }

  private expireIntake(intake: ChainIntake): void {
    if (intake.released || intake.transferred) return;
    const remaining = intake.expires - this.now();
    if (remaining > 0) intake.timer = this.ipcClock.schedule(derivationIntakeTimer(intake.cell), remaining);
    else intake.retire();
  }

  private attemptIntake(intake: ChainIntake): Promise<VoiceChannelDerivationOutcome> {
    if (intake.pending) return intake.pending;
    if (intake.outcome) return Promise.resolve(intake.outcome);
    if (!intake.owner.isCurrent() || this.now() >= intake.expires) intake.retired = true;
    if (intake.retired) { this.releaseIntake(intake); return Promise.resolve(Object.freeze({ kind: "retired" })); }
    intake.running = true;
    // Defer execution until pending is published; an injected SDK/fetch cannot start a twin.
    intake.pending = Promise.resolve().then(() => this.runIntake(intake));
    return intake.pending;
  }
  private async runIntake(intake: ChainIntake): Promise<VoiceChannelDerivationOutcome> {
    let input: VoiceDerivationInput | null = null;
    let plan: VoiceCryptoChain | null = null;
    try {
      const answer = await intake.channel.pullDerivation(intake);
      if ("kind" in answer) {
        if (answer.kind === "refused" && (answer.code === "capacity" || answer.code === "stopped")) return Object.freeze({ kind: "uncertain" });
        intake.outcome = answer; intake.retired = true; return answer;
      }
      input = answer;
      if (intake.retired || !intake.owner.isCurrent() || this.now() >= intake.expires) return Object.freeze({ kind: "retired" });
      const chain = this.prepareChainInner(intake.owner, intake.handleKey, input.binding, intake.expires,
        { localPublicKey: input.localPublicKey, peerPublicKey: input.peerPublicKey, transcript: input.transcript }, intake.channel, intake);
      plan = chain;
      const derivation = this.registerProtectedDerivation(intake.owner, chain, intake.channel.derivationNative(intake), input);
      intake.outcome = Object.freeze({ kind: "ready", chain, derivation });
      return intake.outcome;
    } catch {
      if (plan) { plan.close(); intake.retired = true; }
      return Object.freeze({ kind: intake.retired || !intake.owner.isCurrent() ? "retired" : "uncertain" });
    }
    finally {
      input?.localPublicKey.fill(0); input?.peerPublicKey.fill(0); input?.transcript.fill(0);
      intake.running = false; intake.pending = null;
      if (intake.transferred || intake.retired || !intake.owner.isCurrent() || this.now() >= intake.expires) {
        intake.retired ||= !intake.transferred; this.releaseIntake(intake);
      }
    }
  }
  private releaseIntake(intake: ChainIntake): void {
    if (intake.running) return; // cancellation/deadline is never actual SDK/IO drainage
    if (!intake.released) {
      intake.released = true;
      if (intake.timer !== null) this.ipcClock.cancel(intake.timer);
      intake.timer = null;
      if (!intake.transferred) {
        this.reservedRows -= intake.rows;
        const rows = intake.rows * (CHAIN_HISTORY / CHAIN_ROWS); intake.rows = 0;
        intake.fixedBytes -= rows; this.refund(intake.owner, rows);
        if (intake.variable) { intake.variable = false; this.refund(intake.owner, CHAIN_VARIABLE); }
        if (this.progress.get(intake.owner.group) === intake) this.progress.delete(intake.owner.group);
      }
      intake.channel.releaseIntake(intake); intake.drained.resolve();
    }
    if (!intake.transferred && !intake.owner.isCurrent() && !intake.cell.detached) {
      if (!intake.disposing) {
        intake.disposing = true;
        // End does not refund a caller-entered dispatch. Publish the negative cell now,
        // then join its actual synchronous frame before clearing the retained target.
        const returned = intake.cell.detach();
        if (!intake.cell.detached) {
          void returned.then(() => { intake.disposing = false; this.releaseIntake(intake); });
          return;
        }
        intake.disposing = false;
      } else return;
    }
    if (intake.transferred || !intake.owner.isCurrent()) {
      intake.unsubscribe();
      if (!intake.transferred && intake.fixedBytes) { this.refund(intake.owner, intake.fixedBytes); intake.fixedBytes = 0; }
      if (!intake.owner.isCurrent()) {
        const records = this.intakes.get(intake.owner); records?.delete(intake.row); if (!records?.size) this.intakes.delete(intake.owner);
      }
    }
  }

  private async waitSelectedKeyRetry(record: SelectedKeyRecord, observed: object): Promise<void> {
    // End/availability may arrive while the RPC is still pending. Consume that one wake even
    // if it predates retry installation. The next RPC samples the new identity, so a permanently
    // failed retired operation cannot turn this into an immediate infinite retry loop.
    if (record.wake !== observed) return;
    record.retry = signal<void>();
    record.attempt?.resolve(Object.freeze({ kind: "uncertain" })); record.attempt = null;
    await record.retry.promise; record.retry = null;
  }

  private async driveSelectedKey(record: SelectedKeyRecord): Promise<void> {
    const live = () => !record.retired && record.owner.isCurrent();
    let attempted = false, historical = false;
    let outcome: VoiceSelectedKeyOutcome = Object.freeze({ kind: "failed" });
    try {
      while (live() && !record.permit) {
        attempted = true;
        const observed = record.wake;
        let reply: unknown;
        try { reply = await record.native.admit(record.owner.token, record.executor); } catch { /* Recover the same native selection. */ }
        if (exactKeys(reply, ["owner", "executor", "permit", "join", "kind"]) && reply.owner === record.owner.token && reply.executor === record.executor &&
            reply.kind === "refused" && reply.permit === "" && reply.join === "") {
          // Native proves this executor was never admitted. It owes no settlement and must
          // never settle/adopt the older executor's still-running generation. The call driver
          // may replace its native join; this broker cannot reconstruct the missing private key.
          historical = true; outcome = Object.freeze({ kind: "unavailable" }); return;
        }
        // A terminal no-computation response remains useful after End: ignoring it would make
        // this executor try to settle work native proved belongs to somebody else.
        if (exactKeys(reply, ["owner", "executor", "permit", "join", "kind"]) && reply.owner === record.owner.token && reply.executor === record.executor &&
            reply.kind === "closed" && boundText(reply.permit) && typeof reply.join === "string" && /^[0-9a-f]{64}$/.test(reply.join)) {
          historical = true; outcome = Object.freeze({ kind: "unavailable" }); return;
        }
        if (!live()) return;
        if (exactKeys(reply, ["owner", "executor", "permit", "join", "kind"]) && reply.owner === record.owner.token && reply.executor === record.executor &&
            boundText(reply.permit) && typeof reply.join === "string" && /^[0-9a-f]{64}$/.test(reply.join) && (reply.kind === "admitted" || reply.kind === "closed")) {
          record.permit = reply.permit; record.join = reply.join;
        } else await this.waitSelectedKeyRetry(record, observed);
      }
      if (!live()) return;
      // Share only provider computation, never ordinary run()/fresh-token admission. The whole
      // native/engine/publication/settlement flight holds this broker's one queue position.
      const keys = await this.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, false, ["deriveBits"]);
      if (!live()) return;
      record.key.privateKey = keys.privateKey;
      record.key.point = new Uint8Array(await this.subtle.exportKey("raw", keys.publicKey));
      if (!live()) return;
      while (live()) {
        const observed = record.wake;
        let reply: unknown;
        try { reply = await record.native.publish(record.owner.token, record.executor, record.permit!, record.key.point.slice()); }
        catch { /* Temporary Root capacity and lost replies retain the original key/point. */ }
        if (!live()) return;
        if (exactKeys(reply, ["owner", "executor", "permit", "join", "keyRef"]) && reply.owner === record.owner.token && reply.executor === record.executor &&
            reply.permit === record.permit && reply.join === record.join && typeof reply.keyRef === "string" && /^[0-9a-f]{64}$/.test(reply.keyRef)) {
          record.key.join = record.join; record.key.key = reply.keyRef;
          const handle = Object.freeze({ publicKey: () => { current(record.owner); return record.key.point.slice(); } });
          this.joinKeys.set(handle, record.key);
          outcome = Object.freeze({ kind: "accepted", key: handle }); return;
        }
        await this.waitSelectedKeyRetry(record, observed);
      }
    } catch { /* A failed provider operation is terminal; never compute again for this join. */ }
    finally {
      // An uncertain admit may have succeeded without returning its permit. Settlement names
      // the immutable owner/executor, fences delayed admission and awaits actual native drain.
      if (attempted && !historical) {
        for (;;) {
          const observed = record.wake;
          try { await record.native.settle(record.owner.token, record.executor); break; }
          catch { await this.waitSelectedKeyRetry(record, observed); }
        }
      }
      if (!live()) outcome = Object.freeze({ kind: "retired" });
      record.outcome = outcome;
      record.attempt?.resolve(outcome); record.attempt = null;
    }
  }

  /** Generate on this retained broker. There is deliberately no private-key adoption API. */
  async createJoinKeyHandle(owner: VoiceCryptoOwner): Promise<VoiceJoinKeyHandle> {
    current(owner);
    if (this.keys.size >= 128 || [...this.keys].filter(key => key.owner === owner).length >= 16) throw new Error("voice join key capacity reached");
    this.charge(owner, 1024);
    const key: JoinKeyRecord = { owner, privateKey: null, point: new Uint8Array(), join: null, key: null };
    this.keys.add(key);
    let unsubscribe = () => {};
    const release = () => {
      if (!this.keys.delete(key)) return;
      key.privateKey = null; key.point.fill(0); unsubscribe(); this.refund(owner, 1024);
    };
    try {
      unsubscribe = owner.onRetired(release);
      const generated = await createVoiceJoinKey(this, owner);
      current(owner);
      key.privateKey = generated.privateKey; key.point = generated.publicKey;
      const handle = Object.freeze({ publicKey: () => { current(owner); return key.point.slice(); } });
      this.joinKeys.set(handle, key); return handle;
    } catch (error) { release(); throw error; }
  }

  /** Capture the actual accepted native join projection once. This is browser identity
   * bookkeeping; native must independently resolve and authorize the original key/join. */
  bindJoinKey(owner: VoiceCryptoOwner, handle: VoiceJoinKeyHandle, join: string, keyRef: string, point: Uint8Array): void {
    current(owner);
    const key = this.joinKeys.get(handle);
    if (!key || key.owner !== owner || !key.privateKey || typeof join !== "string" || !/^[0-9a-f]{64}$/.test(join) ||
        typeof keyRef !== "string" || !/^[0-9a-f]{64}$/.test(keyRef) || !(point instanceof Uint8Array) || !sameBytes(key.point, point) ||
        (key.join !== null && (key.join !== join || key.key !== keyRef))) throw new Error("voice join key binding changed");
    key.join = join; key.key = keyRef;
  }

  /** Protected derivation and confirmation reservation. Twelve immutable row claims and one
   * serial execution position remain prepaid through close; SDP requires a later phase owner.
   * expires is computed from the ORIGINAL browser request start, never native monotonic time. */
  prepareChain(owner: VoiceCryptoOwner, handle: VoiceJoinKeyHandle, binding: VoiceCryptoChainBinding, expires: number, setup: VoiceDerivationSetup): VoiceCryptoChain {
    return this.prepareChainInner(owner, handle, binding, expires, setup, null);
  }

  /** Native-channel adapter only: use this broker's EXACT retained Ready channel.
   * A copied handle, generic key, same-label foreign owner or an unready/closed Session cannot
   * adopt its payment. A linked chain reserves no permanent extra execution position;
   * actual provider work obtains an affine SDK loan from the existing two-position union. */
  prepareChainFromChannel(owner: VoiceCryptoOwner, channelHandle: VoiceIpcChannel, handle: VoiceJoinKeyHandle,
    binding: VoiceCryptoChainBinding, expires: number, setup: VoiceDerivationSetup): VoiceCryptoChain {
    current(owner);
    const channel = this.ipcChannels.get(owner.token), key = this.joinKeys.get(handle);
    if (!channel || channel.handle !== channelHandle || !key || !channel.canLend(owner, key)) throw new Error("original ready voice channel required");
    return this.prepareChainInner(owner, handle, binding, expires, setup, channel);
  }

  private prepareChainInner(owner: VoiceCryptoOwner, handle: VoiceJoinKeyHandle, binding: VoiceCryptoChainBinding,
    expires: number, setup: VoiceDerivationSetup, channel: RetainedIpcChannel | null, intake: ChainIntake | null = null): VoiceCryptoChain {
    current(owner); this.collectChains();
    const key = this.joinKeys.get(handle);
    if (!key || key.owner !== owner || !key.privateKey || !validBinding(binding) ||
        key.join !== binding.localJoin || key.key !== binding.localKey ||
        !exactKeys(setup, ["localPublicKey", "peerPublicKey", "transcript"]) || !(setup.localPublicKey instanceof Uint8Array) ||
        !(setup.peerPublicKey instanceof Uint8Array) || !(setup.transcript instanceof Uint8Array) ||
        setup.transcript.length > MAX_DERIVATION_CONTEXT || setup.localPublicKey.length !== 65 || setup.peerPublicKey.length !== 65 ||
        setup.peerPublicKey[0] !== 4 || !sameBytes(key.point, setup.localPublicKey) ||
        !Number.isFinite(expires) || expires <= this.now() || expires - this.now() > 15_000) throw new Error("invalid voice chain binding");
    const stamps = this.chainStamps.get(owner);
    const previous = stamps?.get(binding.handshake);
    if (previous) {
      if (previous.key !== key || previous.channel !== channel || !sameBinding(previous.binding, binding) || previous.expires !== expires || previous.closed ||
          !previous.setup || !sameBytes(previous.setup.localPublicKey, setup.localPublicKey) ||
          !sameBytes(previous.setup.peerPublicKey, setup.peerPublicKey) || !sameBytes(previous.setup.transcript, setup.transcript)) throw new Error("voice chain already retired or changed");
      return previous.handle;
    }
    if (intake && (intake.transferred || intake.retired || intake.released || intake.owner !== owner || intake.key !== key ||
        intake.channel !== channel || intake.expires !== expires || this.progress.get(owner.group) !== intake)) throw new Error("original voice intake required");
    if ((stamps?.size ?? 0) >= 16 || (!intake && this.progress.has(owner.group)) ||
        (!intake && this.actions.size + this.reservedRows + CHAIN_ROWS > MAX_ACTIONS) ||
        (!channel && this.executionPositions() >= MAX_ACTIVE) ||
        (channel && !channel.canLink(intake))) throw new Error("voice chain capacity reached");
    const fixed = CHAIN_HISTORY + CHAIN_METADATA;
    if (!intake) this.charge(owner, fixed + CHAIN_VARIABLE);
    const chain: BrowserChain = { cell: null!,
      owner, key, binding: Object.freeze({ ...binding }), expires, closed: false, historyDisposing: false,
      setup: Object.freeze({ localPublicKey: new Uint8Array(setup.localPublicKey), peerPublicKey: new Uint8Array(setup.peerPublicKey), transcript: new Uint8Array(setup.transcript) }),
      rowIssued: false, reservedRows: CHAIN_ROWS, running: false, execution: true,
      variable: true, fixedBytes: fixed, record: null, unsubscribe: () => {}, handle: null!, timer: null,
      records: new Map(), queue: [], retainedBytes: 130 + setup.transcript.length, enginePeak: 0,
      continuation: null, confirmations: new Map(),
      channel, origin: intake ?? null, drained: signal<void>(), custody: null, sdp: null, sdpRows: 0, mediaIndexed: false,
    };
    chain.cell = new VoiceHandleCell({ close: () => this.closeChain(chain), current: () => this.chainCurrent(chain),
      retired: () => { this.closeChain(chain); this.releaseChainHistory(chain); },
      timer: () => { chain.timer = null; if (this.chainCurrent(chain)) this.armChain(chain); } });
    chain.handle = chainHandle(chain.cell);
    if (intake) { intake.transferred = true; intake.rows = 0; intake.fixedBytes = 0; intake.variable = false; }
    else this.reservedRows += CHAIN_ROWS;
    if (channel) channel.link(chain, intake); else this.protectedPositions++;
    this.progress.set(owner.group, chain); this.chains.set(chain.handle, chain);
    // Publish the stamp before subscribing: a reentrant owner retirement closes this exact
    // plan and cannot leave a successful, untracked reservation behind.
    const owned = stamps ?? new Map<string, BrowserChain>();
    owned.set(binding.handshake, chain); this.chainStamps.set(owner, owned);
    try { chain.unsubscribe = owner.onRetired(chainRetirement(chain.cell)); current(owner); }
    catch (error) { this.closeChain(chain); this.releaseChainHistory(chain); throw error; }
    this.armChain(chain);
    return chain.handle;
  }
  private armChain(chain: BrowserChain): void {
    if (chain.closed) return;
    if (chain.timer !== null) { clearTimeout(chain.timer); chain.timer = null; }
    const deadline = chain.custody ? chain.custody.retentionDeadline : chain.expires;
    // A pending/current key adoption retains only the SAME paid keys/history. It is not
    // another confirmation timer or a grant to use those keys. End still closes first.
    if (deadline === null) return;
    chain.timer = setTimeout(chainTimer(chain.cell), Math.max(1, Math.ceil(deadline - this.now())));
  }
  private chainCurrent(chain: BrowserChain): boolean {
    // Indexed media is a different owner. Old key/plan cleanup must not turn its
    // released negotiation snapshot into whole-channel End. Old actions remain unusable.
    if (chain.mediaIndexed && chain.owner.isCurrent()) return false;
    if (!chain.closed && (!chain.owner.isCurrent() || (chain.custody
      ? !chain.custody.current(this.now()) : this.now() >= chain.expires))) this.closeChain(chain);
    return !chain.closed;
  }
  /** Custody can survive the old deadline; it never admits another old confirmation. */
  private confirmationCurrent(chain: BrowserChain): boolean {
    return this.chainCurrent(chain) && this.now() < chain.expires && !chain.custody?.receipt;
  }
  private retainFinishCustody(chain: BrowserChain): void {
    if (chain.custody) return;
    const now = this.now();
    if (!chain.channel || !this.chainCurrent(chain) || now >= chain.expires || chain.closed) throw new Error("voice Finish expired");
    chain.custody = new VoiceKeyCustody(chain.expires, now);
    this.armChain(chain);
  }
  /** Collection is bounded by retained keys/stamps. It starts no native RPC and never treats
   * deadline/End as settlement of an actual WebCrypto or IPC promise. */
  collectChains(): void {
    for (const stamps of [...this.chainStamps.values()]) for (const chain of [...stamps.values()]) this.chainCurrent(chain);
  }
  private closeChain(chain: BrowserChain): void {
    if (chain.closed) return;
    chain.closed = true; chain.custody?.closeKeys(); chain.sdp?.close();
    if (chain.timer !== null) { clearTimeout(chain.timer); chain.timer = null; }
    const rows = chain.reservedRows;
    this.reservedRows -= rows; chain.reservedRows = 0;
    chain.fixedBytes -= rows * (CHAIN_HISTORY / CHAIN_ROWS); this.refund(chain.owner, rows * (CHAIN_HISTORY / CHAIN_ROWS));
    chain.record?.pair?.retire();
    for (const record of [...chain.records.values()]) this.retireAction(record);
    // A known native child can exist before its Input was read. Publish cleanup ownership
    // synchronously, before drainChain can refund the envelope or channel cleanup can run.
    if (chain.continuation && !chain.continuation.returned) this.startConfirmation(chain.continuation);
    // Local expiry must still release any possibly accepted native Claim through the
    // original channel. Its actual SDK/RPC owners keep their existing payments meanwhile.
    chain.channel?.closeLinked(chain);
    if (!chain.running) this.drainChain(chain);
  }
  private drainChain(chain: BrowserChain): void {
    const quiet = !chain.running && !chain.continuation?.running;
    const retired = chain.closed || chain.custody?.confirmationRetired === true;
    if (retired && quiet && !chain.closed && chain.reservedRows) {
      // Claim already froze native child adoption. Return only unused old descriptor
      // positions; real action history and the original pair/counters stay paid below.
      const rows = chain.reservedRows; chain.reservedRows = 0; this.reservedRows -= rows;
      chain.fixedBytes -= rows * (CHAIN_HISTORY / CHAIN_ROWS);
      this.refund(chain.owner, rows * (CHAIN_HISTORY / CHAIN_ROWS));
    }
    if (retired && chain.execution && quiet) {
      chain.execution = false; if (!chain.channel) this.protectedPositions--;
    }
    if (retired && chain.variable && quiet) {
      if (chain.setup) {
        chain.setup.localPublicKey.fill(0); chain.setup.peerPublicKey.fill(0); chain.setup.transcript.fill(0); chain.setup = null;
      }
      if (chain.record?.derivation) {
        chain.record.derivation.localPublicKey.fill(0); chain.record.derivation.peerPublicKey.fill(0); chain.record.derivation.transcript.fill(0);
        chain.record.derivation = null;
      }
      for (const record of chain.records.values()) {
        record.protectedInput?.aad.fill(0); record.protectedInput?.body.fill(0);
        record.protectedInput = null;
      }
      chain.variable = false; this.refund(chain.owner, CHAIN_VARIABLE);
      if (this.progress.get(chain.owner.group) === chain) this.progress.delete(chain.owner.group);
      chain.drained.resolve();
    }
    if (!chain.owner.isCurrent()) this.releaseChainHistory(chain);
    this.pump();
  }
  private releaseChainHistory(chain: BrowserChain): void {
    if (chain.running || chain.continuation?.running || chain.channel?.retainsHistory(chain) || chain.sdp !== null || chain.fixedBytes === 0) return;
    if (chain.historyDisposing) return;
    // Transfer publishes this actual association BEFORE the ready outcome or any owner
    // callback. End may remove the intake from its lookup map before ready is published;
    // the original chain must still own/detach that paid public dispatch target.
    const transferred = chain.origin ? [chain.origin] : [];
    const cells = [chain.cell, ...[...chain.records.values()].map(record => record.cell),
      ...[...chain.confirmations.values()].map(intake => intake.cell), ...transferred.map(intake => intake.cell)];
    if (cells.some(cell => !cell.detached)) {
      chain.historyDisposing = true; // Before a synchronous dispatch can reenter disposal.
      const returned = cells.map(cell => cell.detach());
      if (cells.some(cell => !cell.detached)) {
        // Original fixed history remains charged until every entered frame actually returns.
        void Promise.all(returned).then(() => {
          chain.historyDisposing = false; this.releaseChainHistory(chain);
        });
        return;
      }
      chain.historyDisposing = false;
    }
    const fixed = chain.fixedBytes; chain.fixedBytes = 0; chain.unsubscribe();
    chain.unsubscribe = () => {}; this.chains.delete(chain.handle);
    for (const intake of chain.confirmations.values()) {
      if (intake.rowClaim) { intake.rowClaim = false; this.reservedRows--; }
    }
    this.refund(chain.owner, fixed);
    const stamps = this.chainStamps.get(chain.owner);
    stamps?.delete(chain.binding.handshake);
    if (!stamps?.size) this.chainStamps.delete(chain.owner);
    const intakes = this.intakes.get(chain.owner);
    if (intakes) {
      for (const [row, intake] of intakes) if (intake.transferred && intake.outcome?.kind === "ready" && intake.outcome.chain === chain.handle) intakes.delete(row);
      if (!intakes.size) this.intakes.delete(chain.owner);
    }
    // Either owner retirement or native unlink can happen last. Protected records have
    // no individual retirement subscription, so collect their SAME global slots here in
    // both orderings, after fixed custody and actual local/channel work have ended.
    for (const record of chain.records.values()) this.releaseAction(record);
    // Retained public handles/timer thunks contain only detached cells. Erase the broker's
    // old heavyweight edges too, so terminal metadata cannot keep Phase/SDK/native adapters.
    for (const intake of chain.confirmations.values()) {
      intake.cell.row = null; intake.native = null; intake.record = null; intake.outcome = null;
    }
    for (const intake of transferred) { intake.outcome = null; intake.pending = null; }
    chain.records.clear(); chain.confirmations.clear(); chain.queue.length = 0;
    chain.record = null; chain.continuation = null; chain.custody = null; chain.sdp = null; chain.channel = null; chain.origin = null;
  }

  /** Exclusive live disposal preparation. Callback-bearing cancellation, subscriptions,
   * SDK/action joins and cell return happen BEFORE the channel's final commit tail. */
  private async prepareChainCompaction(chain: BrowserChain): Promise<PreparedBrowserHistory> {
    const rows = [...chain.records.values()], children = [...chain.confirmations.values()], origin = chain.origin;
    if (chain.historyDisposing || !chain.mediaIndexed || chain.variable || chain.execution || chain.sdp || chain.sdpRows ||
        chain.running || chain.continuation?.running || chain.reservedRows || chain.queue.length ||
        rows.some(row => row.running || !row.drained || row.bytes !== 0) || children.some(child => child.running) ||
        origin?.running || chain.fixedBytes < COMPACT_PLAN_BYTES) throw new Error("actual browser history still owned");
    chain.historyDisposing = true; chain.closed = true; // Negative publication BEFORE callbacks.
    const cells = [chain.cell, ...rows.map(row => row.cell), ...children.map(child => child.cell), ...(origin ? [origin.cell] : [])];
    const ids = rows.map(row => row.identity.owner + "/" + row.identity.action);
    const stamps = this.chainStamps.get(chain.owner), intakes = this.intakes.get(chain.owner);
    const fixed = chain.fixedBytes, owner = chain.owner, handle = chain.handle, handshake = chain.binding.handshake;
    let finished = false;
    try {
      const timer = chain.timer; chain.timer = null;
      if (timer !== null) clearTimeout(timer);
      chain.unsubscribe(); chain.unsubscribe = voiceNoop;
      origin?.unsubscribe(); if (origin) origin.unsubscribe = voiceNoop;
      for (const row of rows) { row.unsubscribe(); row.unsubscribe = voiceNoop; }
      await Promise.all([...cells.map(cell => cell.detach()), ...rows.map(row => row.physicalDrained.promise),
        ...(origin?.pending ? [origin.pending] : [])]);
      // These are actual owned arrays/adapters and no producer is running. Clear their
      // variable contents before the callback-free metadata/accounting transition.
      for (const row of rows) {
        row.input?.aad.fill(0); row.input?.ciphertext.fill(0); row.input = null;
        row.protectedInput?.aad.fill(0); row.protectedInput?.body.fill(0); row.protectedInput = null;
        if (row.derivation) {
          row.derivation.localPublicKey.fill(0); row.derivation.peerPublicKey.fill(0); row.derivation.transcript.fill(0);
          row.derivation = null;
        }
      }
      return {
        abort: () => { if (!finished) { finished = true; chain.historyDisposing = false; } },
        commit: () => {
          // The channel has validated original identities and unlinked itself in the SAME
          // synchronous tail. Everything below is private owned metadata; no provider,
          // owner predicate, fresh admission, await or callback-bearing destructor belongs here.
          if (finished) return 0;
          finished = true; chain.fixedBytes = 0;
          this.chains.delete(handle); stamps?.delete(handshake);
          if (stamps?.size === 0) this.chainStamps.delete(owner);
          if (origin) intakes?.delete(origin.row);
          if (intakes?.size === 0) this.intakes.delete(owner);
          for (let i = 0; i < rows.length; i++) {
            const row = rows[i]; this.actions.delete(ids[i]);
            row.released = true; row.retired = true; row.chain = null; row.native = null!;
            row.pair = null; row.job = null; row.outcome = null; row.attempt = null; row.retry = null;
          }
          for (let i = 0; i < children.length; i++) {
            const child = children[i];
            if (child.rowClaim) { child.rowClaim = false; this.reservedRows--; }
            child.cell.row = null; child.native = null; child.record = null; child.outcome = null;
          }
          if (origin) { origin.outcome = null; origin.pending = null; }
          chain.records.clear(); chain.confirmations.clear(); chain.queue.length = 0;
          chain.origin = null; chain.record = null; chain.continuation = null; chain.custody = null; chain.channel = null;
          chain.historyDisposing = false;
          // Keep ALL original bytes charged through this frame and the heavy worker's
          // actual return. A separate small completion scope performs the refund later.
          return fixed - COMPACT_PLAN_BYTES;
        },
      };
    } catch (error) { chain.historyDisposing = false; throw error; }
  }

  registerProtectedDerivation(owner: VoiceCryptoOwner, plan: VoiceCryptoChain, native: VoiceDerivationNative, input: VoiceDerivationInput): VoiceDerivationHandle {
    current(owner);
    const chain = this.chains.get(plan);
    if (!chain || chain.owner !== owner || !this.confirmationCurrent(chain) ||
        !exactKeys(input, ["owner", "action", "version", "inputTag", "binding", "localPublicKey", "peerPublicKey", "transcript", "engineBytes"]) ||
        input.owner !== owner.token || !validBinding(input.binding) || !sameBinding(chain.binding, input.binding) ||
        !(input.localPublicKey instanceof Uint8Array) || !(input.peerPublicKey instanceof Uint8Array) || !(input.transcript instanceof Uint8Array) ||
        input.transcript.length > MAX_DERIVATION_CONTEXT || input.localPublicKey.length !== 65 || input.peerPublicKey.length !== 65 ||
        input.peerPublicKey[0] !== 4 || !chain.setup || !sameBytes(chain.setup.localPublicKey, input.localPublicKey) ||
        !sameBytes(chain.setup.peerPublicKey, input.peerPublicKey) || !sameBytes(chain.setup.transcript, input.transcript) ||
        typeof input.inputTag !== "string" || !/^[0-9a-f]{64}$/.test(input.inputTag) || input.engineBytes !== 3 * input.transcript.length + 4096 + 256) throw new Error("invalid native derivation input");
    u64(input.action, "action"); u64(input.version, "action version");
    const id = owner.token + "/" + input.action, previous = this.actions.get(id);
    if (previous) {
      if (previous.owner !== owner || previous.operation !== "derive" || previous.native !== native || previous.chain !== chain ||
          !previous.derivation || !sameDerivation(previous.derivation, input)) throw new Error("native derivation identity changed");
      return previous.handle as VoiceDerivationHandle;
    }
    if (chain.rowIssued) throw new Error("voice derivation row already issued");
    const retained = chain.retainedBytes + 130 + input.transcript.length;
    const peak = Math.max(chain.enginePeak, input.engineBytes);
    if (retained + peak > CHAIN_VARIABLE) throw new Error("voice chain capacity reached");
    const identity = Object.freeze({ owner: owner.token, action: input.action, version: input.version, inputTag: input.inputTag });
    const owned = Object.freeze({ ...identity, binding: chain.binding, engineBytes: input.engineBytes,
      localPublicKey: new Uint8Array(input.localPublicKey), peerPublicKey: new Uint8Array(input.peerPublicKey), transcript: new Uint8Array(input.transcript) });
    const record: ActionRecord = { cell: null!,
      operation: "derive", native, owner, pair: null, identity, input: null, derivation: owned, protectedInput: null, chain, bytes: 0,
      status: "queued", outcome: null, attempt: null, retry: null, job: null, running: false,
      retired: false, drained: false, released: false, disposing: false, physicalDrained: signal<void>(), unsubscribe: () => {}, handle: null!,
    };
    record.cell = new VoiceHandleCell({ attempt: () => this.attemptAction(record), inspect: () => record.status,
      pair: () => this.chainCurrent(chain) && record.outcome?.kind === "accepted" ? record.pair : null,
      retire: () => this.retireAction(record) });
    const action = derivationHandle(record.cell);
    record.handle = action;
    chain.rowIssued = true; chain.reservedRows--; this.reservedRows--;
    chain.record = record; chain.records.set(0, record); chain.retainedBytes = retained; chain.enginePeak = peak;
    this.actions.set(id, record);
    return action;
  }

  /** Consume one native-projected protected row. Repulls coalesce the original immutable
   * input; a different action can never reuse a spent row, including after engine failure.
   * All buffers/peak are covered BEFORE copying by the existing chain's variable reservation. */
  registerProtectedConfirmation(owner: VoiceCryptoOwner, plan: VoiceCryptoChain, native: VoiceProtectedConfirmationNative,
    input: VoiceProtectedConfirmationInput): VoiceConfirmationHandle {
    return this.registerProtectedConfirmationInner(owner, plan, native, input, null);
  }
  private registerProtectedConfirmationInner(owner: VoiceCryptoOwner, plan: VoiceCryptoChain, native: VoiceProtectedConfirmationNative,
    input: VoiceProtectedConfirmationInput, intake: ConfirmationIntake | null): VoiceConfirmationHandle {
    current(owner);
    const chain = this.chains.get(plan);
    if (!chain || chain.owner !== owner || !this.confirmationCurrent(chain) ||
        !exactKeys(input, ["owner", "action", "version", "inputTag", "binding", "row", "operation", "kind", "sequence", "aad", "body", "engineBytes"]) ||
        input.owner !== owner.token || !validBinding(input.binding) || !sameBinding(chain.binding, input.binding) ||
        typeof input.inputTag !== "string" || !/^[0-9a-f]{64}$/.test(input.inputTag) ||
        !Number.isInteger(input.row) || input.row < 1 || input.row > 9 ||
        !(input.aad instanceof Uint8Array) || !(input.body instanceof Uint8Array) || input.aad.length > MAX_CONTEXT) {
      throw new Error("invalid native protected confirmation input");
    }
    u64(input.action, "action"); u64(input.version, "action version"); u64(input.sequence, "sequence");
    const step = (input.row - 1) % 3;
    const kind = (["hello", "proof", "finish"] as const)[step];
    const seal = chain.binding.localIsLow ? step !== 1 : step === 1;
    const ciphertextBytes = seal ? input.body.length + 16 : input.body.length;
    if (input.kind !== kind || input.operation !== (seal ? "seal" : "open") ||
        ciphertextBytes < 16 || ciphertextBytes > 528 ||
        input.engineBytes !== 4 * (input.aad.length + ciphertextBytes) + 3 * 512 + 2048) {
      throw new Error("invalid native protected confirmation input");
    }
    const id = owner.token + "/" + input.action, previous = this.actions.get(id);
    if (previous) {
      if (previous.owner !== owner || previous.chain !== chain || previous.operation !== "protected" || previous.native !== native ||
          !previous.protectedInput || !sameProtectedConfirmation(previous.protectedInput, input)) {
        throw new Error("native protected confirmation identity changed");
      }
      return previous.handle as VoiceConfirmationHandle;
    }
    const pair = chain.record?.outcome?.kind === "accepted" ? chain.record.pair : null;
    if (!pair || !pair.isProtected() || chain.records.has(input.row)) throw new Error("voice protected row unavailable");
    if (intake ? intake.chain !== chain || chain.continuation !== intake || !intake.rowClaim || intake.native !== native : chain.reservedRows === 0) {
      throw new Error("original protected row claim required");
    }
    const retained = chain.retainedBytes + input.aad.length + input.body.length;
    const peak = Math.max(chain.enginePeak, input.engineBytes);
    if (retained + peak > CHAIN_VARIABLE) throw new Error("voice chain capacity reached");
    const identity = Object.freeze({ owner: owner.token, action: input.action, version: input.version, inputTag: input.inputTag });
    // Uint8Array subclasses may override slice; Buffer.slice is a shared view. Construct real
    // owned arrays so caller mutation cannot change input after its native identity is fixed.
    const owned = Object.freeze({ ...input, binding: chain.binding, aad: new Uint8Array(input.aad), body: new Uint8Array(input.body) });
    const record: ActionRecord = { cell: null!,
      operation: "protected", native, owner, pair, identity, input: null, derivation: null, protectedInput: owned, chain, bytes: 0,
      status: "queued", outcome: null, attempt: null, retry: null, job: null, running: false,
      retired: false, drained: false, released: false, disposing: false, physicalDrained: signal<void>(), unsubscribe: () => {}, handle: null!,
    };
    record.cell = new VoiceHandleCell({ attempt: () => this.attemptAction(record), inspect: () => record.status,
      retire: () => this.retireAction(record), pair: () => null });
    record.handle = confirmationHandle(record.cell);
    chain.records.set(input.row, record); chain.retainedBytes = retained; chain.enginePeak = peak;
    // A channel descriptor already owns this fixed row. Transfer its SAME global slot
    // into actions; ordinary direct registration consumes a previously unused row instead.
    if (intake) intake.rowClaim = false; else chain.reservedRows--;
    this.reservedRows--; this.actions.set(id, record);
    return record.handle as VoiceConfirmationHandle;
  }

  /** Continue the original purpose on the SAME channel, keys, deadline and prepaid rows.
   * Omit BOTH row/identity for first Begin or lost-Begin recovery. Supplied known identities
   * are only hints until native resolves them again; they cannot replace an unknown intent. */
  nativeConfirmationFromChannel(owner: VoiceCryptoOwner, channelHandle: VoiceIpcChannel, plan: VoiceCryptoChain,
    purpose: VoiceIpcConfirmationPurpose, row?: string, identity?: VoiceIpcActionIdentity, expectedGeneration?: object): VoiceChannelConfirmationHandle {
    const channel = this.ipcChannels.get(owner.token), chain = this.chains.get(plan);
    if (!channel || channel.handle !== channelHandle || !channel.owns(owner) || !chain || chain.owner !== owner || chain.channel !== channel) {
      throw new Error("original voice continuation required");
    }
    const generation = expectedGeneration ?? channel.generation();
    // Retain only closed, immutable metadata. The native adapter has an independent check
    // of the actual indexed parent/child allocations; a renderer role is never authority.
    let request: VoiceIpcConfirmationPurpose["request"];
    switch (purpose.request.kind) {
      case "start": request = Object.freeze({ kind: "start", predecessor: purpose.request.predecessor }); break;
      case "respond": request = Object.freeze({ kind: "respond", triggerRow: purpose.request.triggerRow }); break;
      case "incoming": request = Object.freeze({ kind: "incoming", role: purpose.request.role }); break;
      default: throw new Error("invalid voice confirmation purpose");
    }
    const hint: VoiceIpcConfirmationPurpose = Object.freeze({ parent: Object.freeze({ peer: purpose.parent.peer, join: purpose.parent.join,
      keyRef: purpose.parent.keyRef, epoch: purpose.parent.epoch }), parentRow: purpose.parentRow, request });
    if ((row === undefined) !== (identity === undefined)) throw new Error("complete voice continuation identity required");
    const original = identity ? Object.freeze({ action: identity.action, version: identity.version, inputTag: identity.inputTag }) : null;
    const validation = encodeVoiceIpcConfirmationAction(hint, original ? { kind: "admit", identity: original, engineBytes: 0 } : { kind: "begin", retry: false }); validation.body.fill(0);
    if ((row !== undefined && (!isVoiceIpcActionRow(row) || row === hint.parentRow)) || !channel.matchesParent(chain, hint)) throw new Error("voice continuation parent changed");
    // Every retained child shares the one already-frozen original selector. Do not retain
    // nine independent parent graphs in the fixed per-row history payment.
    const selected = channel.retainChildPurpose(chain, hint);
    const key = request.kind === "start" ? "start/" + request.predecessor : request.kind === "respond" ? "respond/" + request.triggerRow : "incoming/" + request.role;
    const prior = () => {
      if (!channel.matchesGeneration(generation)) throw new Error("old voice continuation generation");
      const previous = chain.confirmations.get(key);
      if (previous && original && (previous.row !== row || previous.identity?.action !== original.action || previous.identity.version !== original.version ||
          previous.identity.inputTag !== original.inputTag)) throw new Error("voice continuation identity changed");
      return previous;
    };
    current(owner); let previous = prior(); if (previous) return previous.handle;
    if (!this.confirmationCurrent(chain) || !chain.record?.pair || chain.record.outcome?.kind !== "accepted") throw new Error("original derived pair required");
    // Recheck after caller predicates/Clock. One descriptor can hold read/cancel/cleanup
    // ownership. Completed descriptors retain their fixed original identity, not an intake.
    current(owner); previous = prior(); if (previous) return previous.handle;
    if (chain.closed || !chain.variable || chain.reservedRows === 0 || (chain.continuation && !chain.continuation.returned) || chain.confirmations.size >= 9) throw new Error("voice continuation capacity reached");
    // Four maximum frame/input copies plus a conservative16KiB engine ceiling overlap
    // retained chain inputs. They are a slice of the original96KiB, never a fresh charge.
    if (chain.retainedBytes + Math.max(chain.enginePeak, 16 * 1024) + 4 * 2064 > CHAIN_VARIABLE) throw new Error("voice continuation intake capacity reached");
    const intake: ConfirmationIntake = { cell: null!, chain, purpose: selected, row: row ?? null, identity: original,
      native: original ? channel.confirmationNative(selected, row!, original) : null, record: null, handle: null!, pending: null,
      running: false, returned: false, outcome: null, rowClaim: true, rowAuthenticated: false };
    intake.cell = new VoiceHandleCell({ attempt: () => this.attemptConfirmationIntake(intake),
      row: () => intake.rowAuthenticated ? intake.row : null });
    intake.handle = confirmationIntakeHandle(intake.cell);
    // Publish before any transport, subscription, engine or scheduler callback. End sees
    // this purpose even if its Begin was never attempted or its row reply was lost.
    chain.reservedRows--; // claimed history cannot be refunded as unused by End
    // Global reservedRows stays charged until SAME-claim ActionRecord transfer or actual
    // history collection. A historical/no-Input descriptor still occupies one logical slot.
    chain.confirmations.set(key, intake); chain.continuation = intake;
    return intake.handle;
  }

  private attemptConfirmationIntake(intake: ConfirmationIntake): Promise<VoiceChannelConfirmationOutcome> {
    if (intake.outcome) return Promise.resolve(intake.outcome);
    if (!intake.pending) intake.pending = signal<VoiceChannelConfirmationOutcome>();
    const answer = intake.pending.promise;
    intake.record?.retry?.resolve(); intake.chain.channel!.wake(); this.startConfirmation(intake);
    return answer;
  }

  private startConfirmation(intake: ConfirmationIntake): void {
    if (intake.running || intake.returned) return;
    intake.running = true;
    void Promise.resolve().then(() => this.runConfirmation(intake));
  }
  private observeConfirmation(intake: ConfirmationIntake, outcome: VoiceChannelConfirmationOutcome): void {
    intake.pending?.resolve(outcome); intake.pending = null;
  }
  private async runConfirmation(intake: ConfirmationIntake): Promise<void> {
    const chain = intake.chain, channel = chain.channel!;
    let input: VoiceProtectedConfirmationInput | null = null;
    let outcome: VoiceChannelConfirmationOutcome = Object.freeze({ kind: "uncertain" });
    let cleanup = chain.closed;
    try {
      // A receipt resolves before settleAction's finally. Do not lend the original Phase
      // until that actual work/RPC has drained AND native confirms ordinary Return.
      await chain.record!.physicalDrained.promise;
      await channel.returnDerivation(chain);
      if (!chain.closed) {
        if (!intake.identity) {
          outcome = await channel.beginConfirmation(intake);
          // Pending has a row but no action. Never invent an identity for engine/settlement.
          // A later exact attempt may adopt that SAME row after native attaches its action.
          if (!intake.identity || chain.closed) return;
          outcome = Object.freeze({ kind: "uncertain" }); // do not retain a second decoded history graph across Input/SDK
        }
        const answer = await channel.pullConfirmation(intake);
        if (!("binding" in answer)) { outcome = answer; cleanup = answer.kind === "status"; }
        else {
          input = answer;
          if (this.confirmationCurrent(chain)) {
            const handle = this.registerProtectedConfirmationInner(chain.owner, chain.handle, intake.native!, input, intake);
            intake.record = this.actions.get(chain.owner.token + "/" + input.action)!;
            // The decoder's temporary owned arrays end BEFORE computation; registration
            // copied them under the same prechecked envelope, retaining immutable input.
            input.aad.fill(0); input.body.fill(0); input = null;
            const first = await handle.attempt();
            if (first.kind === "uncertain") this.observeConfirmation(intake, first);
            await intake.record.physicalDrained.promise;
            outcome = intake.record.outcome as VoiceConfirmationOutcome ?? Object.freeze({ kind: "retired" });
          }
          cleanup = true;
        }
      }
    } catch { outcome = Object.freeze({ kind: chain.closed ? "retired" : "uncertain" }); }
    finally {
      input?.aad.fill(0); input?.body.fill(0);
      // End after any ambiguous read must close the ORIGINAL known child. A running
      // read/cancel keeps this owner until its real finally, never just until abort().
      cleanup ||= chain.closed;
      if (cleanup && intake.identity) {
        for (;;) {
          const observed = channel.wakeToken();
          try {
            await chain.record!.physicalDrained.promise;
            await channel.returnDerivation(chain);
            const returned = await channel.returnConfirmation(intake);
            if (outcome.kind === "status") {
              // Retained historical replies share the original immutable row/action strings
              // with their descriptor. Do not retain duplicate decoded identity/backing
              // strings in each of nine fixed-history rows. Full Sealed metadata remains.
              outcome = returned.kind === "status" ? Object.freeze({ ...returned, row: intake.row!,
                identity: returned.identity ? intake.identity : null,
                receipt: returned.receipt ? Object.freeze({ ...returned.receipt,
                  action: intake.identity.action, inputTag: intake.identity.inputTag }) : null }) : returned;
            }
            intake.returned = true; break;
          } catch {
            this.observeConfirmation(intake, Object.freeze({ kind: chain.closed ? "retired" : "uncertain" }));
            await channel.waitActionCleanup(observed);
          }
        }
        intake.outcome = chain.closed ? Object.freeze({ kind: "retired" }) : outcome;
      }
      // An unknown/Pending Begin transfers only its native cleanup obligation to the
      // already-paid channel driver. It must not await parent proof while that driver is
      // waiting for this actual SDK/I/O drainage. Fixed metadata/row claims remain retained.
      if (chain.closed && !intake.identity) intake.outcome = Object.freeze({ kind: "retired" });
      intake.running = false;
      // Native may have accepted Finish before its delayed reply crossed the old deadline.
      // Actual Return, rather than a browser-local accepted outcome, permits Claim recovery.
      if (intake.returned && chain.custody && !chain.closed) void channel.startTransition(chain);
      this.observeConfirmation(intake, intake.outcome ?? outcome);
      this.drainChain(chain);
    }
  }

  /** Pair wrappers come from the retained broker's module. A fresh HMR derive may finish on
   * this older broker; using its factory preserves the private computation seam's identity. */
  createPair(owner: VoiceCryptoOwner, tx: CryptoKey, rx: CryptoKey, domain: string, isLive: () => boolean = () => true, protectedPair = false): VoiceCryptoPair {
    current(owner);
    return new VoiceCryptoPair(this, owner, tx, rx, domain, isLive, protectedPair);
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
    this.sdkPositions.pump();
    while (this.executionPositions() < MAX_ACTIVE && this.queue.length) {
      const job = this.queue.shift()!;
      // Reserve BEFORE the callback-bearing owner predicate. Its reentrant work must
      // see this position already owned, whether it requests SDK or another channel.
      this.active++;
      let live = false;
      try { live = job.owner.isCurrent(); } catch { /* Invalid owner is a refusal. */ }
      if (!live) {
        try { job.refuse(); }
        finally { this.active--; job.returned?.(); this.sdkPositions.pump(); }
        continue;
      }
      void job.start().finally(() => { this.active--; job.returned?.(); this.pump(); });
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
   * Each row retains its exact native adapter across HMR; other channels cannot reroute its
   * identity, output or settlement through the first adapter registered in this broker. */
  registerConfirmation(
    owner: VoiceCryptoOwner, pair: VoiceCryptoPair, native: VoiceConfirmationNative,
    input: VoiceConfirmationInput,
  ): VoiceConfirmationHandle {
    current(owner);
    if (!pair.belongsTo(this, owner)) throw new Error("voice pair belongs to another owner");
    if (pair.isProtected()) throw new Error("protected native confirmation action required");
    if (!exactKeys(input, ["action", "version", "inputTag", "handshake", "deliveryToken", "kind", "sequence", "aad", "ciphertext"]) ||
        !(input.aad instanceof Uint8Array) || !(input.ciphertext instanceof Uint8Array)) {
      throw new Error("invalid native confirmation input");
    }
    u64(input.action, "action"); u64(input.version, "action version");
    u64(input.deliveryToken, "delivery token"); u64(input.sequence, "sequence");
    if (typeof input.inputTag !== "string" || typeof input.handshake !== "string" ||
        !/^[0-9a-f]{64}$/.test(input.inputTag) || !/^[0-9a-f]{64}$/.test(input.handshake) ||
        !["hello", "proof", "finish"].includes(input.kind) ||
        input.aad.length > MAX_CONTEXT || input.ciphertext.length < 16 || input.ciphertext.length > 528) {
      throw new Error("invalid native confirmation input");
    }
    const key = owner.token + "/" + input.action;
    const previous = this.actions.get(key);
    if (previous) {
      if (previous.owner !== owner || previous.native !== native || previous.pair !== pair || !previous.input || !sameConfirmation(previous.input, input)) {
        throw new Error("native confirmation identity changed");
      }
      return previous.handle as VoiceConfirmationHandle;
    }
    // Cover frozen input, real engine result, digest and independent IPC serialization copies.
    // Conservative peak stays charged through terminal receipt retention; there is no refund
    // gap between a computing record and its completed result/tombstone.
    const bytes = 4 * (input.aad.length + input.ciphertext.length) + 3 * 512 + 2048;
    const group = this.groupBytes.get(owner.group) ?? 0;
    if (this.actions.size + this.reservedRows >= MAX_ACTIONS || this.bytes + bytes > MAX_BYTES || group + bytes > MAX_GROUP_BYTES) {
      throw new Error("voice confirmation capacity reached");
    }
    this.bytes += bytes; this.groupBytes.set(owner.group, group + bytes);
    const identity = Object.freeze({ owner: owner.token, action: input.action, version: input.version, inputTag: input.inputTag });
    const owned = Object.freeze({
      action: input.action, version: input.version, inputTag: input.inputTag, handshake: input.handshake,
      deliveryToken: input.deliveryToken, kind: input.kind, sequence: input.sequence,
      aad: new Uint8Array(input.aad), ciphertext: new Uint8Array(input.ciphertext),
    });
    const record: ActionRecord = { cell: null!,
      operation: "confirmation", native, derivation: null, protectedInput: null, chain: null,
      owner, pair, identity, input: owned, bytes, status: "queued", outcome: null,
      attempt: null, retry: null, job: null, running: false, retired: false,
      drained: false, released: false, disposing: false, physicalDrained: signal<void>(), unsubscribe: () => {}, handle: null!,
    };
    record.cell = new VoiceHandleCell({ attempt: () => this.attemptAction(record), inspect: () => record.status,
      retire: () => this.retireAction(record), pair: () => null });
    record.handle = confirmationHandle(record.cell);
    this.actions.set(key, record);
    try { record.unsubscribe = owner.onRetired(() => this.retireAction(record)); }
    catch (error) {
      // Subscription failed before any job, native admission or SDK handoff exists.
      // Mark only this proven-unstarted registration drained; active work elsewhere
      // still requires its actual finally before releaseAction can dispose it.
      record.drained = true; record.physicalDrained.resolve();
      this.releaseAction(record); throw error;
    }
    return record.handle as VoiceConfirmationHandle;
  }

  private attemptAction(record: ActionRecord): Promise<ActionOutcome> {
    if (!record.owner.isCurrent() || record.retired || (record.chain && !this.chainCurrent(record.chain))) {
      this.retireAction(record); return Promise.resolve(Object.freeze({ kind: "retired" }));
    }
    if (record.outcome) return Promise.resolve(record.outcome);
    if (record.chain && !this.confirmationCurrent(record.chain)) {
      this.retireAction(record); return Promise.resolve(Object.freeze({ kind: "retired" }));
    }
    // Publish before queue/retry signalling. Every concurrent observer shares this one flight;
    // abandoning a Promise affects no underlying admission, engine or commit lifetime.
    if (record.attempt) return record.attempt.promise;
    record.attempt = signal<ActionOutcome>();
    const attempt = record.attempt.promise;
    if (record.retry) { record.retry.resolve(); return attempt; }
    if (record.job) return attempt;
    if (!record.chain && this.queue.length >= MAX_QUEUED) {
      // Nothing was admitted or computed. Temporary unrelated work cannot permanently burn a
      // registered native action; a later exact attempt may use returned queue capacity.
      record.status = "busy"; record.drained = true;
      record.attempt.resolve(Object.freeze({ kind: "busy" })); record.attempt = null;
      return attempt;
    }
    record.drained = false; record.status = "queued";
    const job: Job = {
      owner: record.owner, bytes: 0,
      refuse: () => { record.drained = true; this.retireAction(record); },
      start: async () => {
        record.running = true;
        try { await this.driveAction(record); }
        catch {
          // A local bookkeeping failure must not strand observers or become an unhandled
          // worker rejection. Its identity remains terminal; never reconstruct its engine work.
          if (!record.retired && record.owner.isCurrent()) this.finishAction(record, Object.freeze({ kind: "failed" }));
        }
        finally {
          record.drained = true; record.running = false;
          record.physicalDrained.resolve();
          if (!record.owner.isCurrent()) this.releaseAction(record);
        }
      },
    };
    record.job = job;
    if (record.chain) {
      // The bounded row queue shares one prepaid position. Follow-up input may arrive before
      // the previous result RPC settles; it cannot start a second engine in the same lane.
      record.chain.queue.push(record); this.pumpProtected(record.chain);
    } else { this.queue.push(job); this.pump(); }
    return attempt;
  }

  private pumpProtected(chain: BrowserChain): void {
    if (chain.running || !this.confirmationCurrent(chain)) return;
    const record = chain.queue.shift();
    if (!record) return;
    if (record.retired || !record.job) { this.pumpProtected(chain); return; }
    chain.running = true;
    if (!chain.channel) { this.active++; this.protectedActive++; }
    void record.job.start().finally(() => {
      if (!chain.channel) { this.active--; this.protectedActive--; }
      chain.running = false;
      this.drainChain(chain); this.pumpProtected(chain);
    });
  }

  private finishAction(record: ActionRecord, outcome: ActionOutcome): void {
    record.outcome = outcome; record.status = outcome.kind;
    record.attempt?.resolve(outcome); record.attempt = null;
  }

  private async uncertainAction(record: ActionRecord): Promise<void> {
    if (record.retired || !record.owner.isCurrent()) return;
    if (record.chain && !this.confirmationCurrent(record.chain)) { this.retireAction(record); return; }
    record.status = "uncertain";
    record.retry = signal<void>();
    record.attempt?.resolve(Object.freeze({ kind: "uncertain" })); record.attempt = null;
    if (record.chain?.channel) await record.chain.channel.waitActionRetry(record, record.retry);
    else await record.retry.promise;
    record.retry = null;
  }

  private async driveAction(record: ActionRecord): Promise<void> {
    const native = record.native;
    let attemptedAdmission = false, admitted = false, historicalOnly = false;
    let enginePosition: (() => void) | null = null;
    // A shared owner records provider output before any later check/await can throw. Its
    // finally clears that actual buffer even when computation never reaches the commit loop.
    const output: { plaintext: Uint8Array<ArrayBuffer> | null; resultTag: string | null } = { plaintext: null, resultTag: null };
    const live = () => !record.retired && record.owner.isCurrent() && (!record.chain || this.confirmationCurrent(record.chain));
    try {
      while (live() && !admitted) {
        if (record.chain?.channel) {
          enginePosition = await record.chain.channel.reserveEngine(record.chain);
          if (!live()) break;
          if (!enginePosition) { await this.uncertainAction(record); continue; }
        }
        record.status = "admitting"; attemptedAdmission = true;
        let answer: AdmissionReply | DerivationCommitReply | CommitReply;
        try { answer = await native.admitAction(record.identity, record.derivation?.engineBytes ?? record.protectedInput?.engineBytes ?? record.bytes); }
        catch { enginePosition?.(); enginePosition = null; await this.uncertainAction(record); continue; }
        if (!live()) break;
        if ((record.operation === "derive" && this.finishDerivationReply(record, answer)) ||
            (record.operation === "protected" && this.finishConfirmationReply(record, answer))) {
          // A past receipt is no authority to settle an engine owned by an earlier browser
          // incarnation. This broker did not compute it and cannot attest its retirement.
          historicalOnly = true; return;
        }
        if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "permit"]) &&
            answer.kind === "admitted" && boundIdentity(answer, record.identity) && boundText(answer.permit)) {
          admitted = true;
        } else if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "receipt", "code"]) &&
                   answer.kind === "rejected" && boundIdentity(answer, record.identity) &&
                   boundText(answer.receipt) && rejection(answer.code)) {
          // A protected terminal admission report describes past native state. This window
          // did not compute it and cannot attest settlement of an earlier browser's engine.
          if (record.operation === "protected") historicalOnly = true;
          this.finishAction(record, Object.freeze({ kind: "rejected", receipt: answer.receipt, code: answer.code }));
          return;
        } else { enginePosition?.(); enginePosition = null; await this.uncertainAction(record); }
      }
      if (!live()) return;
      // Engine failure is permanent for this identity. It is not retryable IPC uncertainty.
      record.status = "computing";
      try {
        const compute = async () => {
          // This check occurs AFTER acquiring a lent position, inside its try/finally. End may
          // have won while provider work waited behind a channel read/cancellation flight.
          if (!live()) throw new Error("voice action retired");
          if (record.operation === "derive") {
            const input = record.derivation!, chain = record.chain!;
            record.pair = await computeVoicePair(this, record.owner, chain.key.privateKey!,
              input.peerPublicKey, input.transcript, input.binding.localIsLow, () => this.chainCurrent(chain), true);
            // Native derivation commits an empty closed marker, never a key export.
            output.plaintext = new Uint8Array();
          } else if (record.operation === "protected") {
            output.plaintext = await record.pair!.computeProtectedConfirmation(CONFIRM_COMPUTE, record.protectedInput!);
          } else output.plaintext = await record.pair!.computeConfirmation(CONFIRM_COMPUTE, record.input!);
          if (!live()) return;
          if (output.plaintext.length > (record.protectedInput?.operation === "seal" ? 528 : 512)) throw new Error("invalid confirmation engine result");
          const digest = new Uint8Array(await this.subtle.digest("SHA-256", output.plaintext));
          if (digest.length !== 32) throw new Error("invalid confirmation digest");
          output.resultTag = Array.from(digest, value => value.toString(16).padStart(2, "0")).join("");
        };
        await compute();
        if (!live()) return;
        if (output.plaintext === null || output.resultTag === null) throw new Error("voice computation produced no result");
      } catch {
        if (live()) this.finishAction(record, Object.freeze({ kind: "failed" }));
        return;
      } finally { enginePosition?.(); enginePosition = null; }
      while (live()) {
        record.status = "committing";
        const copy = output.plaintext.slice();
        let answer: CommitReply | DerivationCommitReply | null = null;
        try {
          if (record.operation === "derive") answer = await record.native.commitDerivation(record.identity, output.resultTag);
          else if (record.operation === "protected") answer = await record.native.commitProtectedConfirmation(record.identity, copy, output.resultTag);
          else {
            const input = record.input!;
            const packet = Object.freeze({ ...input, aad: input.aad.slice(), ciphertext: input.ciphertext.slice() });
            answer = await record.native.commitConfirmation(record.identity, packet, copy, output.resultTag);
          }
        }
        catch { /* A lost/malformed reply cannot authorize another engine operation. */ }
        finally { copy.fill(0); }
        if (!live()) return;
        if (record.operation === "derive") {
          if (this.finishDerivationReply(record, answer)) return;
          await this.uncertainAction(record); continue;
        }
        if (this.finishConfirmationReply(record, answer, output.resultTag)) return;
        await this.uncertainAction(record);
      }
    } finally {
      enginePosition?.(); enginePosition = null;
      output.plaintext?.fill(0);
      // Only actual engine and last RPC settlement reaches this fence. Native must latch it,
      // serialized with admission, so delayed admission cannot recreate a retired charge.
      if (attemptedAdmission && !historicalOnly) {
        if (record.chain?.channel) {
          // Keep the ORIGINAL cleanup transport and logical owner through lost requests or
          // replies, even after End. One concrete availability wake retries this exact identity;
          // neither a rejection nor terminal browser metadata proves native engine drainage.
          const channel = record.chain.channel;
          for (;;) {
            const observed = channel.wakeToken();
            try { await native.settleAction(record.identity); break; }
            catch { await channel.waitActionCleanup(observed); }
          }
        } else {
          try { await native.settleAction(record.identity); } catch { /* Native uncertainty stays charged. */ }
        }
      }
      // Crossing the old admission deadline must resolve its observer without closing
      // provisional pair custody. Real SDK/result/settlement drainage still runs first.
      if (record.retired || !record.owner.isCurrent() ||
          (record.chain && !record.outcome && !this.confirmationCurrent(record.chain))) this.retireAction(record);
    }
  }

  private finishConfirmationReply(record: ActionRecord, answer: unknown, resultTag?: string): boolean {
    const tag = (value: unknown) => typeof value === "string" && /^[0-9a-f]{64}$/.test(value) &&
      (resultTag === undefined || value === resultTag);
    if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "resultTag", "receipt", "confirmation"]) &&
        answer.kind === "accepted" && boundIdentity(answer, record.identity) && tag(answer.resultTag) &&
        boundText(answer.receipt) && (answer.confirmation === "continue" || answer.confirmation === "confirmed")) {
      this.finishAction(record, Object.freeze({ kind: "accepted", receipt: Object.freeze({ ...answer }) as VoiceConfirmationReceipt }));
      return true;
    }
    if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "resultTag", "receipt", "code"]) &&
        answer.kind === "rejected" && boundIdentity(answer, record.identity) && tag(answer.resultTag) &&
        boundText(answer.receipt) && rejection(answer.code)) {
      this.finishAction(record, Object.freeze({ kind: "rejected", receipt: answer.receipt, code: answer.code }));
      return true;
    }
    return false;
  }

  private finishDerivationReply(record: ActionRecord, answer: unknown): boolean {
    if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "resultTag", "receipt", "derived"]) &&
        answer.kind === "accepted" && boundIdentity(answer, record.identity) && answer.resultTag === EMPTY_RESULT_TAG &&
        boundText(answer.receipt) && answer.derived === true) {
      this.finishAction(record, Object.freeze({ kind: "accepted", receipt: Object.freeze({ ...answer }) as VoiceDerivationReceipt }));
      return true;
    }
    if (exactKeys(answer, ["kind", "owner", "action", "version", "inputTag", "resultTag", "receipt", "code"]) &&
        answer.kind === "rejected" && boundIdentity(answer, record.identity) && answer.resultTag === EMPTY_RESULT_TAG &&
        boundText(answer.receipt) && rejection(answer.code)) {
      record.pair?.retire(); record.pair = null;
      this.finishAction(record, Object.freeze({ kind: "rejected", receipt: answer.receipt, code: answer.code })); return true;
    }
    return false;
  }

  private retireAction(record: ActionRecord): void {
    record.retired = true;
    if (record.operation === "derive") { record.pair?.retire(); record.pair = null; }
    this.finishAction(record, Object.freeze({ kind: "retired" }));
    record.retry?.resolve();
    if (!record.running) {
      const queued = record.job ? this.queue.indexOf(record.job) : -1;
      if (queued >= 0) this.queue.splice(queued, 1);
      if (record.chain) {
        const protectedQueued = record.chain.queue.indexOf(record);
        if (protectedQueued >= 0) record.chain.queue.splice(protectedQueued, 1);
      }
      record.drained = true;
      record.physicalDrained.resolve();
    }
    // Action-only retirement keeps the terminal identity/slot while its owner is current.
    // Owner retirement may remove it only after actual work has drained.
    if (record.drained && !record.owner.isCurrent()) this.releaseAction(record);
  }

  private releaseAction(record: ActionRecord): void {
    if (record.released) return;
    // A channel retains the same fixed record after local SDK/I/O drainage, while native
    // Plan-final may still be Pending. Preserve its actual global row position as well as
    // the chain's byte payment until cleanup unlinks that metadata. Variable input is
    // zeroed separately by drainChain; this guard does not keep the physical lane busy.
    if (record.chain?.channel?.retainsHistory(record.chain) || !record.drained) return;
    if (!record.cell.detached) {
      if (!record.disposing) {
        record.disposing = true;
        const returned = record.cell.detach();
        if (!record.cell.detached) {
          void returned.then(() => { record.disposing = false; this.releaseAction(record); });
          return;
        }
        record.disposing = false;
      } else return;
    }
    record.released = true; record.unsubscribe(); record.unsubscribe = () => {};
    record.input?.aad.fill(0); record.input?.ciphertext.fill(0);
    record.input = null; record.pair = null;
    record.protectedInput?.aad.fill(0); record.protectedInput?.body.fill(0); record.protectedInput = null;
    if (record.derivation) {
      record.derivation.localPublicKey.fill(0); record.derivation.peerPublicKey.fill(0); record.derivation.transcript.fill(0);
      record.derivation = null;
    }
    this.actions.delete(record.identity.owner + "/" + record.identity.action);
    // No computation/settlement is running and the public target has physically detached.
    // This adapter may close over the complete old channel, so it cannot survive disposal.
    record.chain = null; record.native = null!; record.job = null; record.outcome = null;
    record.attempt = null; record.retry = null;
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

type IpcKeys = Readonly<{ requestMac: CryptoKey; requestBody: CryptoKey; responseMac: CryptoKey; responseBody: CryptoKey }>;
function originalMediaFee(refund: () => void): () => void {
  let paid = true;
  return () => { if (paid) { paid = false; refund(); } };
}
type RetainedNativePeer = {
  readonly plan: VoiceCryptoChain; readonly generation: object; readonly refund: () => void;
  media: VoiceNativeMedia | null; compacted: boolean; compactionReturned: boolean;
  advanced: boolean; closing: boolean; refundReady: boolean; cleanup: Promise<void> | null;
};
function compactionReturned(work: Promise<boolean>, peer: RetainedNativePeer): Promise<boolean> {
  return work.then(result => { if (result) peer.compactionReturned = true; return result; });
}
type IpcCustody = Readonly<{
  rtcConfiguration(): VoiceRtcConfigurationLease | null;
  execute<T>(work: () => Promise<T>): Promise<T>;
  reserveSdk(): VoiceSdkPositionRequest;
  lifecyclePrepared(): void;
  begin(): void; end(): void; setupDrained(): void; closed(): void; collected(): void;
  prepareCompaction(chain: BrowserChain): Promise<PreparedBrowserHistory>;
  compactionReturned(bytes: number): void;
  failedHistoryRetained(chain: BrowserChain): void;
  unlinked(chain: BrowserChain): void;
  finishInput(chain: BrowserChain): void;
  transitionUpdated(chain: BrowserChain): void;
  sdpUnlinked(chain: BrowserChain, phase: VoiceSdpSdkCustody): void;
  mediaPrepared(): () => void;
}>;
/** Inert source construction ends its configuration argument frame before any SDP
 * await. No SDK exists on a constructor refusal, so the original loan returns here. */
function preparedRtcOwner(low: boolean, current: () => boolean, clock: VoiceIpcClock,
  configuration: VoiceRtcConfigurationLease | null): VoiceSdpRtcOwner {
  try { return new VoiceSdpRtcOwner(low, current, clock, configuration); }
  catch (error) {
    if (configuration) releaseVoiceRtcConfiguration(configuration);
    throw error;
  }
}
// Separate factory: returning a provider position must not retain SDK inputs/results.
function enginePositionReturn(channel: RetainedIpcChannel, pairState: RetainedNativeNegotiation,
  request: VoiceSdkPositionRequest, release: () => void): () => void {
  let held = true;
  return () => { if (held) { held = false; channel.engineReturned(pairState, request, release); } };
}
function ipcBytes(hex: string): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(hex.length / 2);
  for (let index = 0; index < bytes.length; index++) bytes[index] = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
  return bytes;
}

/** Original per-pair lifecycle. Async methods capture THIS actual owner once; old
 * callbacks/finally blocks cannot write a later negotiation through channel-global fields.
 * The selected physical Session, keys/counters, execution position and End remain shared.
 * Existing original negotiation/IPC metadata pays this object; no new effect is admitted. */
class RetainedNativeNegotiation {
  readonly generation = {};
  workPump: NativeWorkPump | null = null;
  linked: BrowserChain | null = null;
  intake: ChainIntake | null = null;
  // Original Derive parent only. This descriptor is never replaced by a sibling; native's
  // separate Plan-final intent still names that same parent after ordinary Return. Child
  // key/phase transfer remains separate. Neither a renderer flag nor localDrained can
  // manufacture final proof or authorize closing the native channel.
  terminalAction: Readonly<Pick<ChainIntake, "purpose" | "row" | "identity">> | null = null;
  derivationReturn: Promise<void> | null = null;
  transitionFlight: Promise<void> | null = null;
  sdpFlight: Promise<void> | null = null;
  rtcFlight: Promise<void> | null = null;
  rtcCreating = false;
  sdpRtc: VoiceSdpRtcOwner | null = null;
  media: VoiceNativeMedia | null = null;
  negotiationFlight: Promise<void> | null = null;
  negotiationRetired = false;
  failed = false;
  failedFlight: Promise<void> | null = null;
  failedReturned = false;
  failedPlan: VoiceCryptoChain | null = null;
  compactFlight: Promise<boolean> | null = null;
  sdpRoundStage: "offer" | "answer" | "done" = "offer";
  sdpAction: VoiceSdpActionOwner | null = null;
  sdpCreating = false;
  derivationReturned = false;
  // One current metadata cleanup intent survives lost history/final replies, plus at most
  // one waiting parent below. Never retain a32-row queue or reconstruct browser keys.
  cleanupWork: { work: VoiceWorkDescriptor; proof: string | null } | null = null;
  // A parent may be waiting for a late native child. Retain that one original intent while
  // scanning/closing one child at a time; retrying the parent first would starve its cleanup.
  cleanupParent: { work: VoiceWorkDescriptor; proof: string | null } | null = null;
  // Immutable after the one definite Phase-refusal transition. Unknown replies never
  // select a different intent, and an issued proof can only be ACKed in its original mode.
  finalMode: "retire" | "retire-plan" = "retire";
  finalIntentStarted = false;
  finalProof: string | null = null;
  finalAcknowledged = false;
  actionBusy = false;
  sdkPosition: VoiceSdkPositionRequest | null = null;
}

/** Private to this retained module. There is deliberately no seed/private-key constructor
 * export. Only VoiceCryptoBroker can provide its actual SelectedKeyRecord and prepaid lane.
 * No ordinary native.admit or generic queue is used: terminal control must remain executable
 * when a protected chain holds the other existing position while waiting for this channel. */
class RetainedIpcChannel {
  private negotiation = new RetainedNativeNegotiation();
  private readyAcknowledged = false;
  readonly handle: VoiceIpcChannel;
  private readonly selected: SelectedKeyRecord;
  private readonly native: VoiceIpcNative;
  private readonly transport: typeof fetch;
  private readonly subtle: SubtleCrypto;
  private readonly clock: VoiceIpcClock;
  private readonly custody: IpcCustody;
  private readonly execution: Pick<VoiceSharedExecution, "run">;
  private readonly peers = new Map<VoiceCryptoChain, RetainedNativePeer>();
  private cleanupStarted = false;
  private lifecycle: VoiceSessionLifecycle | null = null;
  private lifecycleCompletion: Promise<void> | null = null;
  // Opaque marker only. A BrowserChain/Phase/full N here would retain refunded graphs.
  // This one fixed slot and the task/timer/observer graph belong to the2KiB fee.
  private lifecycleAdvance: { generation: object; plan: VoiceCryptoChain } | null = null;
  // Drivers and SDK borrowers keep channel activity independently of the provider pool.
  // Keep that lifecycle activity through the last finally; a returned SDK position is
  // not whole-channel drainage or permission to clear keys/refund retained metadata.
  private activity = 0;
  private request: Signal<VoiceIpcStatus> | null = null;
  // At most one original paid read flight, not a request map or independent queue. Its
  // immutable purpose remains available for coalescing after SDK plaintext is zeroed.
  private historyRequest: IpcHistoryRequest | null = null;
  private driving = false;
  private retired = false;
  private drained = false;
  private collected = false;
  private attempted = false;
  private terminal: VoiceIpcStatus | null = null;
  private unsubscribe: () => void = () => {};
  private retry: Signal<void> | null = null;
  private cleanupWakeFlight: Promise<void> | null = null;
  private wakeIdentity: object = {};
  private readonly completion = signal<void>();
  // Installed before the first engine await. Failure is terminal; the same original selected
  // key/transcript can never get a second constructor, directional key set or nonce allocator.
  private construction: Promise<IpcKeys> | null = null;
  private keys: IpcKeys | null = null;
  private bootstrap: VoiceIpcBootstrap | null = null;
  private hash: Uint8Array<ArrayBuffer> | null = null;
  private port = 0;
  private tx = 0n;
  private rx = 0n;
  private controller: AbortController | null = null;
  // The physical controller may belong to indexed media while a new intake waits for
  // the codec. Only that exact Input owner may cancel it; actionBusy is merely queueing.
  private controllerIntake: ChainIntake | null = null;

  constructor(selected: SelectedKeyRecord, native: VoiceIpcNative,
    transport: typeof fetch, subtle: SubtleCrypto, clock: VoiceIpcClock, custody: IpcCustody) {
    this.selected = selected; this.native = native; this.transport = transport;
    this.subtle = subtle; this.clock = clock; this.custody = custody;
    this.execution = { run: work => custody.execute(async () => {
      this.beginActivity();
      try { return await work(); } finally { this.endActivity(); }
    }) };
    this.handle = Object.freeze({ status: () => this.status(), close: () => {
      this.retire(); this.wake(); return this.completion.promise;
    } });
  }
  belongsTo(selected: SelectedKeyRecord, native: VoiceIpcNative, transport: typeof fetch): boolean {
    return this.selected === selected && this.native === native && this.transport === transport;
  }
  owns(owner: VoiceCryptoOwner): boolean { return this.selected.owner === owner; }
  private retainedLifecycle(): VoiceSessionLifecycle | null { return this.lifecycle; }
  observeLifecycle(broker: VoiceCryptoBroker, key: VoiceJoinKeyHandle): VoiceSessionLifecycleObserver {
    const prior = this.retainedLifecycle(); if (prior) return prior.handle;
    if (!this.live() || this.retired || this.drained || !this.readyAcknowledged || !this.keys || !this.bootstrap)
      throw new Error("authenticated ready voice channel required");
    // Owner callbacks may reenter: repeat callback-free publication gates last.
    const reentrant = this.retainedLifecycle(); if (reentrant) return reentrant.handle;
    if (this.retired || this.drained || !this.readyAcknowledged || !this.keys || !this.bootstrap)
      throw new Error("authenticated ready voice channel required");
    this.custody.lifecyclePrepared(); this.beginActivity();
    const driver = new VoiceSessionLifecycle({
      live: () => this.live() && !this.drained,
      observe: () => this.lifecycleObservation(),
      step: observation => this.lifecycleStep(broker, key, observation),
    }, this.clock);
    this.lifecycle = driver; driver.start();
    // Only this small observer survives actual loop/finally return. The heavy loop
    // clears actions/clock first; its fee remains until final Session collection.
    this.lifecycleCompletion = driver.join().then(() => { this.endActivity(); });
    return driver.handle;
  }
  private lifecycleObservation(): VoiceLifecycleObservation {
    const state = this.negotiation, chain = state.linked;
    let stage: VoiceLifecycleStage = "waiting", plan = chain?.handle ?? null;
    const next = this.lifecycleAdvance;
    if (next && next.generation !== state.generation) this.lifecycleAdvance = null;
    if (this.retired || this.drained) stage = "stopped";
    else if (next?.generation === state.generation) { stage = "advance"; plan = next.plan; }
    else if (state.failedReturned) { stage = "advance"; plan = state.failedPlan; }
    else if (state.failed || chain?.closed || chain?.sdp?.closed) stage = chain ? "failed" : "waiting";
    else if (!chain) stage = state.workPump ? "waiting" : "confirmation";
    else if (state.negotiationRetired) stage = "compact";
    else if (state.media?.adopted) stage = state.media.closed ? "stopped" : "retire";
    else if (chain.custody?.keyReceipt?.state !== "current" || chain.variable || chain.running || chain.continuation?.running)
      stage = state.workPump ? "waiting" : "confirmation";
    else if (!chain.sdp || chain.sdp.storage?.state !== "funded" || chain.sdp.sdk?.state !== "held") stage = "handoff";
    else if (state.sdpRoundStage === "done") stage = "media";
    else if (chain.sdp.pc.schedulingState === "closed") stage = "failed";
    else stage = chain.sdp.pc.schedulingState === "admitted" ? "rtc" : "pc";
    return Object.freeze({ generation: state.generation, plan, stage });
  }
  private lifecycleMatches(observation: VoiceLifecycleObservation, compacted = false): boolean {
    const state = this.negotiation;
    if (state.generation !== observation.generation || this.retired || this.drained || !this.live()) return false;
    // Repeat original generation/negative state AFTER the last owner callback. An old
    // public marker cannot make a delayed step write into a replacement negotiation.
    if (this.negotiation !== state || this.retired || this.drained || state.generation !== observation.generation) return false;
    if (observation.plan === null) return state.linked === null;
    if (compacted) {
      if (state.failedReturned && state.failedPlan === observation.plan) return true;
      const peer = this.peers.get(observation.plan);
      return peer?.generation === observation.generation && peer.compacted && peer.compactionReturned;
    }
    return state.linked?.handle === observation.plan;
  }
  private stopLifecyclePump(generation: object): Promise<void> {
    const state = this.negotiation, pump = state.workPump;
    if (state.generation !== generation || !pump) return Promise.resolve();
    // Pump's own finally clears its old graph. Completion retains only that emptied
    // observer, Session and inert generation, never BrowserChain/Phase/full N.
    return pump.close().then(() => {
      if (this.negotiation.generation === generation && this.negotiation.workPump === pump)
        this.negotiation.workPump = null;
    });
  }
  private lifecycleStep(broker: VoiceCryptoBroker, key: VoiceJoinKeyHandle, observation: VoiceLifecycleObservation): Promise<void> {
    const owner = this.selected.owner, plan = observation.plan;
    if (!this.lifecycleMatches(observation, observation.stage === "advance")) return Promise.resolve();
    // Not async: no captured BrowserChain/Phase/N survives an await. Existing leaves
    // resolve their exact originals and obtain their own fresh native effect grants.
    switch (observation.stage) {
      case "confirmation": this.observeWork(broker, key); return Promise.resolve();
      case "handoff": return this.stopLifecyclePump(observation.generation).then(() => {
        if (plan && this.lifecycleMatches(observation)) return broker.prepareNativeSdpHandoff(owner, this.handle, plan);
      });
      case "pc": return broker.prepareNativePcIntent(owner, this.handle, plan!);
      case "rtc": return broker.completeNativeSdpRtcRound(owner, this.handle, plan!);
      case "media": return broker.completeNativeMediaHandoff(owner, this.handle, plan!);
      case "retire": return broker.retireNativeNegotiation(owner, this.handle, plan!);
      case "failed": return broker.retireFailedNativeNegotiation(owner, this.handle, plan!);
      case "compact": return broker.compactNativeNegotiation(owner, this.handle, plan!).then(collected => {
        if (collected && this.lifecycleMatches(observation, true))
          this.lifecycleAdvance = { generation: observation.generation, plan: plan! };
      });
      case "advance":
        if (broker.advanceNativeNegotiation(owner, this.handle, plan!)) this.lifecycleAdvance = null;
        return Promise.resolve();
      default: return Promise.resolve();
    }
  }
  private retainedWorkPump(): NativeWorkPump | null {
    const pairState = this.negotiation; return pairState.workPump; }
  observeWork(broker: VoiceCryptoBroker, key: VoiceJoinKeyHandle): VoiceNativeWorkObserver {
    const pairState = this.negotiation;
    const reentrant = this.retainedWorkPump();
    if (reentrant) return reentrant.handle;
    if (!this.negotiationCurrent(pairState) || !this.readyAcknowledged || !this.keys || !this.bootstrap) throw new Error("authenticated ready voice channel required");
    if (!this.negotiationCurrent(pairState)) throw new Error("authenticated ready voice channel required");
    // Owner predicates can synchronously reenter observation or close this channel.
    // Coalesce the reentrant publication, then recheck callback-free flags after the LAST
    // predicate. No callback occurs between here and installing the original pump.
    if (pairState.workPump) return pairState.workPump.handle;
    if (this.negotiation !== pairState || this.retired || this.drained || !this.readyAcknowledged || !this.keys || !this.bootstrap) throw new Error("authenticated ready voice channel required");
    const owner = this.selected.owner;
    const pump = new NativeWorkPump({ owner: owner.token, executor: this.selected.executor,
      live: () => this.negotiationCurrent(pairState),
      read: fromSlot => {
        if (!this.negotiationCurrent(pairState)) return Promise.reject(new Error("old voice observer"));
        return this.native.readWork(owner.token, this.selected.executor, fromSlot);
      },
      derive: work => {
        if (!this.negotiationCurrent(pairState)) throw new Error("old voice observer");
        if (work.purpose.kind !== "derive") throw new Error("native derivation required");
        return broker.nativeDerivationFromChannel(owner, this.handle, key, work.purpose, work.row, work.identity, pairState.generation);
      },
      returnDerivation: chain => {
        if (this.negotiation !== pairState) return Promise.reject(new Error("old voice observer"));
        return broker.returnNativeDerivation(owner, this.handle, chain);
      },
      confirmation: (chain, work) => {
        if (!this.negotiationCurrent(pairState)) throw new Error("old voice observer");
        const p = work.purpose;
        if (p.kind !== "confirmation") throw new Error("native confirmation required");
        const request: VoiceIpcConfirmationPurpose["request"] = p.request === "start" ? { kind: "start", predecessor: p.predecessor }
          : p.request === "respond" ? { kind: "respond", triggerRow: p.trigger } : { kind: "incoming", role: p.role };
        return broker.nativeConfirmationFromChannel(owner, this.handle, chain,
          { parent: p.parentPurpose, parentRow: p.parent, request }, work.row, work.identity, pairState.generation);
      },
      returned: (chain, handle) => {
        // A remounted/manual caller may adopt B after A Returned but before the pump
        // observes A. The latest continuation pointer is not A's completion witness.
        if (pairState.linked?.handle !== chain) return false;
        for (const intake of pairState.linked.confirmations.values()) {
          if (intake.handle === handle) return intake.returned && !intake.running;
        }
        return false;
      },
      transition: async chain => {
        if (!this.negotiationCurrent(pairState) || pairState.linked?.handle !== chain || !pairState.linked.custody) return false;
        await this.startTransition(pairState.linked); return true;
      },
    }, this.clock);
    pairState.workPump = pump; pump.start(); return pump.handle;
  }
  history(purpose: VoiceIpcPurpose, expectedRow: string | null): Promise<VoiceIpcHistoryOutcome> {
    const pairState = this.negotiation;
    if (!this.live()) return Promise.resolve(Object.freeze({ kind: "retired" }));
    if (expectedRow !== null && !isVoiceIpcActionRow(expectedRow)) throw new Error("invalid voice history row");
    // Copy exactly four immutable strings, never extra caller properties or a mutable
    // purpose object. Encode validates canonical hashes/u64 before publishing any request.
    const selected = Object.freeze({ peer: purpose.peer, join: purpose.join, keyRef: purpose.keyRef, epoch: purpose.epoch });
    const encoded = encodeVoiceIpcAction(selected, { kind: "begin", retry: false });
    if (!this.live()) { encoded.body.fill(0); return Promise.resolve(Object.freeze({ kind: "retired" })); }
    const previous = this.historyRequest;
    if (previous) {
      const same = previous.expectedRow === expectedRow && previous.purpose.peer === selected.peer &&
        previous.purpose.join === selected.join && previous.purpose.keyRef === selected.keyRef && previous.purpose.epoch === selected.epoch;
      encoded.body.fill(0);
      return same ? previous.result.promise : Promise.resolve(Object.freeze({ kind: "busy" }));
    }
    // History has no constructor path. Preserve the SAME keys/bootstrap and any real
    // SDK/IO owner already occupying this original driver, including its final cleanup.
    if (!this.keys || !this.bootstrap || this.request || this.driving || pairState.actionBusy) {
      encoded.body.fill(0); return Promise.resolve(Object.freeze({ kind: "busy" }));
    }
    const query: IpcHistoryRequest = { purpose: selected, expectedRow, body: encoded.body, result: signal<VoiceIpcHistoryOutcome>() };
    this.historyRequest = query;
    this.start();
    return query.result.promise;
  }
  private beginActivity(): void { if (++this.activity === 1) this.custody.begin(); }
  private endActivity(): void { if (--this.activity === 0) this.custody.end(); }
  /** Local generation identities fence callbacks; native still enforces every grant. */
  generation(): object { return this.negotiation.generation; }
  matchesGeneration(generation: object): boolean { return this.negotiation.generation === generation; }
  canLend(owner: VoiceCryptoOwner, key: JoinKeyRecord, generation = this.negotiation.generation): boolean {
    const pairState = this.negotiation;
    return pairState.generation === generation && this.negotiationCurrent(pairState) && !this.retired && !this.drained && this.keys !== null &&
      this.selected.owner === owner && this.selected.key === key;
  }
  canLink(intake: ChainIntake | null = null, generation = intake?.generation ?? this.negotiation.generation): boolean {
    const pairState = this.negotiation;
    if (pairState.generation !== generation) return false;
    // This pilot owns ONE original terminal plan. Historical Input may release its variable
    // claim without linking a chain, but cannot let another row overwrite its final cleanup.
    const sameTerminal = pairState.terminalAction === null || (intake !== null && pairState.terminalAction.row === intake.row &&
      pairState.terminalAction.purpose === intake.purpose && pairState.terminalAction.identity === intake.identity);
    return sameTerminal && !this.driving && !this.request && !this.historyRequest && !this.retry && !pairState.sdkPosition && !pairState.compactFlight && !pairState.negotiationRetired && this.peers.size < MAX_RETAINED_MEDIA && !this.retired && !this.drained && this.keys !== null && pairState.linked === null && (pairState.intake === null || pairState.intake === intake);
  }
  link(chain: BrowserChain, intake: ChainIntake | null = null): void {
    const pairState = this.negotiation;
    if (!this.canLink(intake) || chain.channel !== this || chain.owner !== this.selected.owner || chain.key !== this.selected.key) {
      throw new Error("voice channel chain binding changed");
    }
    pairState.linked = chain;
  }
  /** Fixed history custody is independent of actual SDK/I/O drainage. Pending native
   * finalization must not release either its byte payment or its global row claims. */
  retainsHistory(chain: BrowserChain): boolean {
    const pairState = this.negotiation; return pairState.linked === chain; }
  reserveIntake(intake: ChainIntake): void {
    const pairState = this.negotiation;
    if (!this.canLink(null, intake.generation) || pairState.intake || intake.channel !== this || intake.owner !== this.selected.owner || intake.key !== this.selected.key) throw new Error("voice intake changed");
    pairState.intake = intake;
    pairState.terminalAction = Object.freeze({ purpose: intake.purpose, row: intake.row, identity: intake.identity });
  }
  cancelIntake(intake: ChainIntake): void {
    if (this.controllerIntake === intake) this.controller?.abort();
  }
  releaseIntake(intake: ChainIntake): void {
    const pairState = this.negotiation; if (pairState.intake === intake) pairState.intake = null; }
  matchesParent(chain: BrowserChain, purpose: VoiceIpcConfirmationPurpose): boolean {
    const pairState = this.negotiation;
    const parent = pairState.terminalAction;
    return pairState.linked === chain && parent !== null && parent.row === purpose.parentRow &&
      parent.purpose.peer === purpose.parent.peer && parent.purpose.join === purpose.parent.join &&
      parent.purpose.keyRef === purpose.parent.keyRef && parent.purpose.epoch === purpose.parent.epoch;
  }
  retainChildPurpose(chain: BrowserChain, purpose: VoiceIpcConfirmationPurpose): VoiceIpcConfirmationPurpose {
    const pairState = this.negotiation;
    if (!this.matchesParent(chain, purpose)) throw new Error("original confirmation parent required");
    return Object.freeze({ parent: pairState.terminalAction!.purpose, parentRow: pairState.terminalAction!.row, request: purpose.request });
  }
  /** Ordinary Return is a Phase fence, not End. Preserve the original pair, plan, keys,
   * deadline and monotonic allocators for subsequent native-authorized confirmation rows. */
  async returnDerivation(chain: BrowserChain, pairState = this.negotiation): Promise<void> {
    if (pairState.linked !== chain) throw new Error("original derivation owner required");
    if (pairState.derivationReturned) return;
    const original = pairState.terminalAction;
    if (!original) throw new Error("original derivation row required");
    await this.returnAction(original.row, original.identity, request => this.actionRpc(original.purpose, original.row,
      request, "status", true, opened => decodeVoiceIpcActionReply(opened, original.row)));
    pairState.derivationReturned = true;
  }
  finishDerivation(chain: BrowserChain): Promise<void> {
    const pairState = this.negotiation;
    if (pairState.linked !== chain || !chain.record || chain.record.operation !== "derive") {
      throw new Error("original linked derivation required");
    }
    if (pairState.derivationReturn) return pairState.derivationReturn;
    // Coalescing an already-admitted flight is observation only. A new flight must not
    // queue behind final cleanup after retirement; repeat flags after the owner predicate,
    // whose callback can synchronously retire this same channel.
    if (!this.live() || this.retired || this.drained || chain.closed) throw new Error("voice derivation channel closed");
    const record = chain.record;
    // Reserve actual channel activity before scheduling. The existing fixed channel/chain
    // metadata pays one coalesced promise and its fixed original references, no byte copy.
    this.beginActivity();
    const pending = Promise.resolve().then(async () => {
      await record.physicalDrained.promise;
      if (record.outcome?.kind !== "accepted") throw new Error("accepted derivation required");
      await this.returnDerivation(chain, pairState);
    }).finally(() => {
      pairState.derivationReturn = null;
      this.endActivity(); this.collect();
    });
    pairState.derivationReturn = pending;
    return pending;
  }
  /** Only authenticated metadata from the SAME framed Session can resolve an unknown
   * purpose. Pending and attached identities are separate states; neither may change row. */
  private observeConfirmationBegin(intake: ConfirmationIntake, reply: VoiceIpcActionReply): VoiceIpcActionReply {
    if (reply.kind !== "status") return reply; // generic refusal is never absence proof
    if (reply.row === intake.purpose.parentRow || (intake.row !== null && intake.row !== reply.row)) throw new Error("voice Begin row changed");
    if (intake.identity && (!reply.identity || intake.identity.action !== reply.identity.action ||
        intake.identity.version !== reply.identity.version || intake.identity.inputTag !== reply.identity.inputTag)) throw new Error("voice Begin identity changed");
    if (!intake.identity && reply.identity?.action === intake.chain.record?.identity.action) throw new Error("voice Begin reused parent action");
    intake.row = reply.row;
    intake.rowAuthenticated = true;
    if (!intake.identity && reply.identity) {
      intake.identity = reply.identity;
      intake.native = this.confirmationNative(intake.purpose, reply.row, reply.identity);
    }
    return reply;
  }
  beginConfirmation(intake: ConfirmationIntake): Promise<VoiceIpcActionReply> {
    const pairState = this.negotiation;
    if (pairState.linked !== intake.chain || intake.chain.continuation !== intake || !intake.running || intake.chain.closed ||
        !this.matchesParent(intake.chain, intake.purpose)) throw new Error("original Begin intent required");
    return this.actionRpc(intake.purpose.parent, "0".repeat(32), { kind: "begin", retry: true }, "status", false,
      opened => this.observeConfirmationBegin(intake, decodeVoiceIpcActionReply(opened, intake.row)), intake.purpose);
  }
  pullConfirmation(intake: ConfirmationIntake): Promise<VoiceProtectedConfirmationInput | VoiceIpcActionReply> {
    const pairState = this.negotiation;
    if (pairState.linked !== intake.chain || intake.chain.continuation !== intake || !intake.running ||
        !intake.chain.variable || intake.chain.closed || !pairState.derivationReturned || !this.matchesParent(intake.chain, intake.purpose)) {
      throw new Error("original prepaid confirmation intake required");
    }
    if (!intake.row || !intake.identity || !intake.native) throw new Error("attached confirmation required");
    const row = intake.row, identity = intake.identity;
    return this.actionRpc(intake.purpose.parent, row, { kind: "input" }, "input", false, (opened, kind) => {
      const reply = kind === 0 ? decodeVoiceIpcActionReply(opened, row)
        : decodeVoiceIpcConfirmationInput(opened, { row, parent: intake.purpose.parentRow,
          identity, bootstrap: this.bootstrap!, binding: intake.chain.binding });
      // Publish only after the original authenticated response has passed its exact row
      // and input/identity checks. Generic refusal does not authenticate a supplied hint.
      if ("binding" in reply || reply.kind === "status") intake.rowAuthenticated = true;
      // Only fully authenticated native Finish Input arms provisional key custody.
      // Publish before any subsequent SDK/cancellation callback; direct hints cannot do it.
      if ("binding" in reply && reply.kind === "finish") this.custody.finishInput(intake.chain);
      return reply;
    }, intake.purpose);
  }
  returnConfirmation(intake: ConfirmationIntake): Promise<VoiceIpcActionReply> {
    const pairState = this.negotiation;
    if (pairState.linked !== intake.chain || !intake.row || !intake.identity || !this.matchesParent(intake.chain, intake.purpose)) throw new Error("original child cleanup required");
    const row = intake.row, identity = intake.identity;
    return this.returnAction(row, identity, request => this.actionRpc(intake.purpose.parent, row,
      request, "status", true, opened => decodeVoiceIpcActionReply(opened, row), intake.purpose));
  }
  private async returnAction(row: string, identity: VoiceIpcActionIdentity,
    rpc: (request: VoiceIpcConfirmationRequest) => Promise<VoiceIpcActionReply>): Promise<VoiceIpcActionReply> {
    const exact = (reply: VoiceIpcActionReply) => {
      if (reply.kind !== "status" || reply.row !== row) throw new Error("native Return unresolved");
      if (reply.pending !== "not-pending") {
        if (reply.pending !== "closed-handed" && reply.pending !== "closed-unhanded") throw new Error("native Pending closure unresolved");
        return reply;
      }
      if (!reply.identity || reply.identity.action !== identity.action || reply.identity.version !== identity.version ||
          reply.identity.inputTag !== identity.inputTag) throw new Error("native Return identity changed");
      return reply;
    };
    // Pending has no engine/receipt to settle. Its actual close_pending fence is distinct
    // from attached localDrained; the decoder enforces false settlement/ACK/drain flags.
    const settled = await rpc({ kind: "settle", identity });
    if (settled.kind === "refused") {
      // A Pending row has no matching attached action for Settle. Only an actual
      // closed-Pending Return can discharge that case; refusal is never absence proof.
      const pending = exact(await rpc({ kind: "close", mode: "return" }));
      if (pending.pending === "not-pending") throw new Error("native Return settlement unresolved");
      return pending;
    }
    let reply = exact(settled);
    if (reply.pending !== "not-pending") throw new Error("native Return settlement unresolved");
    if (!reply.settled) throw new Error("native Return settlement unresolved");
    if (reply.receipt && !reply.acknowledged) {
      const digest = reply.receipt.digest;
      reply = exact(await rpc({ kind: "ack", identity, digest }));
      if (!reply.acknowledged || reply.receipt?.digest !== digest) throw new Error("native Return acknowledgement unresolved");
    }
    reply = exact(await rpc({ kind: "close", mode: "return" }));
    if (!reply.localDrained) throw new Error("native Return drainage unresolved");
    return reply;
  }
  /** Cache this adapter on the original descriptor: broker registration compares its actual
   * object identity as well as the immutable input. No label-only retry may replace it. */
  confirmationNative(purpose: VoiceIpcConfirmationPurpose, row: string, original: VoiceIpcActionIdentity): VoiceProtectedConfirmationNative {
    const exact = (identity: VoiceConfirmationIdentity) => {
      if (identity.owner !== this.selected.owner.token || identity.action !== original.action ||
          identity.version !== original.version || identity.inputTag !== original.inputTag) throw new Error("voice confirmation identity changed");
    };
    const status = (opened: Uint8Array) => decodeVoiceIpcActionReply(opened, row);
    const rpc = <T>(request: VoiceIpcConfirmationRequest, response: "status" | "permit", afterEnd: boolean,
      decode: (opened: Uint8Array<ArrayBuffer>) => T) => this.actionRpc(purpose.parent, row, request, response, afterEnd, decode, purpose);
    const terminal = (reply: VoiceIpcActionReply): CommitReply => {
      if (reply.kind !== "status" || !reply.identity || !reply.receipt) throw new Error("native confirmation unresolved");
      const identity = { owner: this.selected.owner.token, ...reply.identity }; exact(identity);
      const receipt = reply.receipt;
      if (receipt.outcome.kind === "rejected") return Object.freeze({ ...identity, kind: "rejected", resultTag: receipt.outputTag, receipt: receipt.digest,
        code: receipt.outcome.code === "capacity" ? "capacity" : receipt.outcome.code === "unauthorized" || receipt.outcome.code === "freshness" ? "retired" : "invalid" });
      if (receipt.outcome.kind !== "sealed" && receipt.outcome.kind !== "opened") throw new Error("native confirmation outcome changed");
      // A historical crypto receipt is not current pair/RTC readiness. The production call
      // adapter must obtain that independently from the authenticated native call state.
      return Object.freeze({ ...identity, kind: "accepted", resultTag: receipt.outputTag, receipt: receipt.digest, confirmation: "continue" });
    };
    const ack = async (reply: VoiceIpcActionReply) => {
      if (reply.kind !== "status" || !reply.receipt || reply.acknowledged) return;
      const digest = reply.receipt.digest, answer = await rpc({ kind: "ack", identity: original, digest }, "status", true, status);
      if (answer.kind !== "status" || !answer.acknowledged || answer.receipt?.digest !== digest) throw new Error("native confirmation ACK unresolved");
    };
    return Object.freeze({
      admitAction: async (identity: VoiceConfirmationIdentity, bytes: number) => {
        exact(identity);
        return rpc({ kind: "admit", identity: original, engineBytes: bytes }, "permit", false, opened => opened.length === 78
          ? Object.freeze({ ...identity, kind: "admitted" as const, permit: decodeVoiceIpcPermit(opened, row, original, bytes) }) : terminal(status(opened)));
      },
      commitProtectedConfirmation: async (identity: VoiceConfirmationIdentity, output: Uint8Array, resultTag: string) => {
        exact(identity);
        const reply = await rpc({ kind: "result", identity: original, tag: resultTag, output }, "status", false, status);
        const result = terminal(reply); await ack(reply); return result;
      },
      settleAction: async (identity: VoiceConfirmationIdentity) => {
        exact(identity);
        const reply = await rpc({ kind: "settle", identity: original }, "status", true, status);
        if (reply.kind !== "status" || !reply.settled || !reply.identity) throw new Error("native confirmation settlement unresolved");
        exact({ owner: this.selected.owner.token, ...reply.identity }); await ack(reply);
      },
    });
  }
  async pullDerivation(intake: ChainIntake): Promise<VoiceDerivationInput | VoiceIpcActionReply> {
    const pairState = this.negotiation;
    if (pairState.generation !== intake.generation || pairState.intake !== intake || intake.retired || intake.released || intake.transferred) throw new Error("original prepaid intake required");
    return this.actionRpc(intake.purpose, intake.row, { kind: "input" }, "input", false, (opened, kind) => {
      if (kind === 0) return decodeVoiceIpcActionReply(opened, intake.row);
      return decodeVoiceIpcDerivationInput(opened, { row: intake.row, identity: intake.identity, purpose: intake.purpose,
        bootstrap: this.bootstrap!, localPoint: this.selected.key.point, localKey: this.selected.key.key! });
    }, null, intake);
  }
  /** These adapters retain the exact native purpose/row. No stock IPC carries Input/results,
   * and post-End requests can only clean original history, never admit another engine. */
  derivationNative(intake: ChainIntake): VoiceDerivationNative {
    const exact = (identity: VoiceConfirmationIdentity) => {
      if (identity.owner !== this.selected.owner.token || identity.action !== intake.identity.action ||
          identity.version !== intake.identity.version || identity.inputTag !== intake.identity.inputTag) throw new Error("voice action identity changed");
    };
    const status = (opened: Uint8Array) => decodeVoiceIpcActionReply(opened, intake.row);
    const terminal = (reply: VoiceIpcActionReply): DerivationCommitReply => {
      if (reply.kind !== "status" || !reply.identity || !reply.receipt) throw new Error("native voice action unresolved");
      exact({ owner: this.selected.owner.token, ...reply.identity });
      const receipt = reply.receipt, identity = { owner: this.selected.owner.token, ...reply.identity };
      if (receipt.outcome.kind === "derived") return Object.freeze({ ...identity, kind: "accepted", resultTag: receipt.outputTag, receipt: receipt.digest, derived: true });
      if (receipt.outcome.kind === "rejected") return Object.freeze({ ...identity, kind: "rejected", resultTag: receipt.outputTag, receipt: receipt.digest,
        code: receipt.outcome.code === "capacity" ? "capacity" : receipt.outcome.code === "unauthorized" || receipt.outcome.code === "freshness" ? "retired" : "invalid" });
      throw new Error("native derivation outcome changed");
    };
    const ack = async (reply: VoiceIpcActionReply) => {
      if (reply.kind !== "status" || !reply.receipt || reply.acknowledged) return;
      const answer = await this.actionRpc(intake.purpose, intake.row,
        { kind: "ack", identity: intake.identity, digest: reply.receipt.digest }, "status", true, status);
      if (answer.kind !== "status" || !answer.acknowledged || answer.receipt?.digest !== reply.receipt.digest) throw new Error("native receipt acknowledgement unresolved");
    };
    return Object.freeze({
      admitAction: async (identity: VoiceConfirmationIdentity, bytes: number) => {
        exact(identity);
        return this.actionRpc(intake.purpose, intake.row, { kind: "admit", identity: intake.identity, engineBytes: bytes }, "permit", false, opened => {
          if (opened.length === 78) return Object.freeze({ ...identity, kind: "admitted" as const, permit: decodeVoiceIpcPermit(opened, intake.row, intake.identity, bytes) });
          return terminal(status(opened));
        });
      },
      commitDerivation: async (identity: VoiceConfirmationIdentity, resultTag: string) => {
        exact(identity);
        const reply = await this.actionRpc(intake.purpose, intake.row,
          { kind: "result", identity: intake.identity, tag: resultTag, output: new Uint8Array() }, "status", false, status);
        const result = terminal(reply); await ack(reply); return result;
      },
      settleAction: async (identity: VoiceConfirmationIdentity) => {
        exact(identity);
        const reply = await this.actionRpc(intake.purpose, intake.row,
          { kind: "settle", identity: intake.identity }, "status", true, status);
        if (reply.kind !== "status" || !reply.settled || !reply.identity) throw new Error("native engine settlement unresolved");
        exact({ owner: this.selected.owner.token, ...reply.identity });
        await ack(reply); // a lost terminal reply is recovered from its ACTUAL native receipt
      },
    });
  }
  private async actionRpc<T>(purpose: VoiceIpcPurpose, row: string, action: VoiceIpcActionRequest,
    response: "input" | "permit" | "status", afterEnd: boolean,
    decode: (opened: Uint8Array<ArrayBuffer>, kind: VoiceIpcHeader["kind"]) => T,
    child: VoiceIpcConfirmationPurpose | null = null, inputIntake: ChainIntake | null = null): Promise<T> {
    const pairState = this.negotiation;
    const unknownBegin = child !== null && action.kind === "begin" && row === "0".repeat(32);
    if (pairState.actionBusy || this.request || this.historyRequest || (!unknownBegin && !isVoiceIpcActionRow(row)) ||
        (afterEnd && action.kind !== "ack" && action.kind !== "settle" && action.kind !== "close" && action.kind !== "close-plan" && action.kind !== "final")) throw new Error("voice channel busy");
    pairState.actionBusy = true;
    let encoded: ReturnType<typeof encodeVoiceIpcAction> | null = null;
    try {
      // Closed selection. A v2 child refusal never falls back to a v1 parent request.
      encoded = child ? encodeVoiceIpcConfirmationAction(child, action as VoiceIpcConfirmationRequest) : encodeVoiceIpcAction(purpose, action);
      const owned = encoded;
      return await this.execution.run(() => this.framed(owned.operation, owned.kind, row, owned.body, afterEnd,
        (kind, length) => (kind === 0 && (length === 19 || length === 258 || (response === "permit" && length === 94))) ||
          (response === "input" && kind === 1 && length >= (child ? 446 : 557) && length <= 2064), decode, undefined, inputIntake ?? undefined));
    } finally { encoded?.body.fill(0); pairState.actionBusy = false; }
  }
  /** Borrow before native admission. End cancels queued work; a granted loan remains
   * owned through actual SDK/digest return. This never refunds native authority/bytes. */
  async reserveEngine(chain: BrowserChain | null = null, pairState = this.negotiation): Promise<(() => void) | null> {
    if (this.negotiation !== pairState || this.retired || this.drained || pairState.negotiationRetired ||
        pairState.sdkPosition || (chain && (pairState.linked !== chain || chain.closed))) return null;
    this.beginActivity();
    let request: VoiceSdkPositionRequest;
    try { request = this.custody.reserveSdk(); }
    catch (error) { this.endActivity(); this.collect(); throw error; }
    pairState.sdkPosition = request;
    const release = await request.promise;
    if (!release) {
      pairState.sdkPosition = null; this.endActivity(); this.collect(); return null;
    }
    const returned = enginePositionReturn(this, pairState, request, release);
    // Grant can win the race with End/replacement before this await resumes. Return the
    // exact unstarted loan; no old observer may select or write the new negotiation.
    if (this.negotiation !== pairState || this.retired || this.drained || pairState.negotiationRetired ||
        (chain && (pairState.linked !== chain || chain.closed))) { returned(); return null; }
    return returned;
  }
  engineReturned(pairState: RetainedNativeNegotiation, request: VoiceSdkPositionRequest, release: () => void): void {
    if (pairState.sdkPosition === request) pairState.sdkPosition = null;
    release(); this.endActivity(); this.collect();
  }
  closeLinked(chain: BrowserChain): void {
    const pairState = this.negotiation;
    if (pairState.linked !== chain) return;
    if (!this.live() || this.negotiation !== pairState || this.retired || this.drained || !this.failedMediaMatches(pairState, chain)) {
      this.retire(); return;
    }
    // Fence ONLY this negotiation before SDK/owner callbacks. Independent indexed media
    // keeps its original Current owner and shared Session. Never abort its controller.
    pairState.failed = true;
    const peer = this.peers.get(chain.handle);
    if (pairState.media && peer) {
      peer.closing = true;
      try { pairState.media.close(); } catch { /* Keep actual driver/SDK debt for joined cleanup. */ }
    }
    pairState.sdkPosition?.cancel();
    try { pairState.sdpRtc?.close(); } catch { /* Actual close must join during cleanup. */ }
    pairState.sdpAction?.close();
    void pairState.workPump?.close();
    this.wake();
  }

  /** Same original un-compacted media allocation, or a plan with no media yet. Pure
   * lookup only: labels from another generation cannot select a peer for cleanup. */
  private failedMediaMatches(state: RetainedNativeNegotiation, chain: BrowserChain): boolean {
    if (!state.media) return true;
    const peer = this.peers.get(chain.handle);
    return !!peer && peer.plan === chain.handle && peer.generation === state.generation &&
      peer.media === state.media && !peer.compacted && !peer.advanced;
  }
  /** One attached failed plan. Recover uncertain
   * admission only AFTER its native irreversible Plan fence has physically returned;
   * an ordinary live Admit/Claim is otherwise capable of creating a new obligation. */
  retireFailedNegotiation(chain: BrowserChain): Promise<void> {
    const state = this.negotiation;
    if (state.linked !== chain && !(state.failedReturned && state.failedPlan === chain.handle))
      throw new Error("original failed attached plan required");
    if (state.failedFlight) return state.failedFlight;
    if (state.failedReturned && state.failedPlan === chain.handle) return Promise.resolve();
    if (state.linked !== chain || !this.failedMediaMatches(state, chain) || !state.terminalAction || !this.live() ||
        this.negotiation !== state || this.retired || this.drained)
      throw new Error("original failed attached plan required");
    // live() calls the owner predicate. It can reenter and publish this exact worker.
    if (state.failedFlight) return state.failedFlight;
    if (state.linked !== chain || !this.failedMediaMatches(state, chain) || !state.terminalAction)
      throw new Error("original failed attached plan required");
    if (state.finalIntentStarted && state.finalMode !== "retire-plan" && !state.media)
      throw new Error("original terminal intent requires exact recovery");
    // Select immutable Plan meaning BEFORE the first terminal transport opportunity.
    // No lost/Pending Phase intent may be relabelled, even during negative cleanup.
    // An in-flight ordinary-retirement worker may have lost a definite Phase refusal.
    // Preserve its exact intent; after joins, the existing authenticated fallback alone
    // may select Plan. Never relabel a Pending/issued Phase proof.
    if (!state.finalIntentStarted) state.finalMode = "retire-plan";
    if (!chain.closed) chain.handle.close();
    // Closing the actual RTC/dispatch cell can also reenter. Re-coalesce after the
    // LAST callback, then repeat callback-free publication fences before paying work.
    if (state.failedFlight) return state.failedFlight;
    if (this.negotiation !== state || this.retired || this.drained) return Promise.resolve();
    if (state.linked !== chain || !this.failedMediaMatches(state, chain) || !state.terminalAction)
      throw new Error("original failed attached plan required");
    state.failed = true; this.beginActivity();
    const work = Promise.resolve().then(() => this.cleanupFailedPlan(state));
    // Separate scope: publication observes actual heavy worker return, not its last
    // successful native call. The same fixed work envelope remains until replacement.
    const flight = this.failedPlanCompletion(work, state, chain.handle);
    state.failedFlight = flight; return flight;
  }
  private async cleanupFailedPlan(state: RetainedNativeNegotiation): Promise<boolean> {
    const chain = state.linked!;
    state.sdkPosition?.cancel();
    if (state.workPump) { await state.workPump.close(); state.workPump = null; }
    state.intake?.retire();
    if (state.intake) await state.intake.drained.promise;
    await chain.drained.promise;
    if (state.derivationReturn) { try { await state.derivationReturn; } catch { /* exact cleanup below */ } }
    if (state.transitionFlight) await state.transitionFlight;
    if (state.rtcFlight) await state.rtcFlight;
    if (state.sdpFlight) await state.sdpFlight;
    // Both old owners may already have committed native detach/history collection.
    // Join their actual frames/finally outside the codec, with no reverse wait from
    // compaction to this failed worker. Synchronous media.close preceded these joins.
    if (state.negotiationFlight) await state.negotiationFlight;
    if (state.compactFlight) await state.compactFlight;
    const peer = this.peers.get(chain.handle);
    if (peer?.compacted) { await this.cleanupPeer(peer); return false; }
    // Negative media cleanup is valid after owner End too. Its real Current/history/
    // SDK owners must join before this worker returns; End waits for this same worker
    // before clearing framing keys or starting remaining old-phase cleanup.
    if (peer) await this.cleanupPeer(peer);
    while (this.live() && this.negotiation === state && !this.drained) {
      const observed = this.wakeIdentity;
      try {
        if (state.sdpRtc) await state.sdpRtc.drain();
        if (state.sdpAction && !state.sdpAction.returned) await state.sdpAction.cleanup();
        if (!this.live() || this.negotiation !== state || this.drained) return false;
        const custody = chain.custody, phase = chain.sdp;
        await this.cleanupConfirmationIntents(true, state);
        if (!state.derivationReturned) await this.returnDerivation(chain, state);
        if (custody?.needsClaim || custody?.needsKeyRecovery || phase?.needsStorageRecovery || phase?.needsSdkRecovery) {
          // Native Pending means the SAME Plan intent has closed first admission, not
          // that its obligations drained. Await the COMPLETE authenticated frame/finally
          // before using admission-shaped history recovery on this still-live Session.
          // Lost/refused fence replies throw and repeat this same intent on the next turn.
          await this.retireOriginalAction(true, state, true);
          if (custody?.needsClaim) await this.transitionRpc(chain, { kind: "claim" }, true, state);
          if (custody?.needsKeyRecovery) await this.keySessionRpc(chain, { kind: "adopt" }, true, state);
        }
        await this.cleanupSdp(chain, true, state);
        if (custody?.needsKeyRelease)
          await this.keySessionRpc(chain, { kind: "release", receipt: custody.keyReceipt!.receipt }, true, state);
        if (custody?.needsRelease)
          await this.transitionRpc(chain, { kind: "release", receipt: custody.receipt!.receipt }, true, state);
        if (!state.finalAcknowledged) await this.retireOriginalAction(true, state);
        if (state.finalMode !== "retire-plan") throw new Error("original Plan final remains unresolved");
        if (!this.live() || this.negotiation !== state || this.drained) return false;
        state.sdpAction = null; state.sdpRtc = null;
        chain.record?.pair?.retire();
        for (const record of chain.records.values()) record.pair = null;
        return true;
      } catch {
        if (this.live() && this.negotiation === state) await this.waitCleanupWake(observed);
      }
    }
    return false;
  }
  private failedPlanCompletion(work: Promise<boolean>, state: RetainedNativeNegotiation, plan: VoiceCryptoChain): Promise<void> {
    return work.then(done => {
      if (!done || this.negotiation !== state || this.retired || this.drained) return;
      const chain = state.linked;
      if (!chain || chain.handle !== plan || !state.finalAcknowledged) return;
      if (chain.sdp) this.custody.sdpUnlinked(chain, chain.sdp);
      // That refund's final owner check is callback-bearing. If End wins, leave its
      // original link intact so global cleanup issues unlinked AFTER actual joins.
      if (!this.live() || this.retired || this.drained || this.negotiation !== state) {
        this.retire(); return;
      }
      this.custody.failedHistoryRetained(chain); // No callback or refund in this leaf.
      state.linked = null; state.intake = null;
      state.terminalAction = null; state.finalProof = null;
      state.failedPlan = plan; state.negotiationRetired = true; state.failedReturned = true;
      // Do not call unlinked/compaction or refund chain/action history. Native retains
      // the exact certified rows; broker fixed history stays paid until actual owner End.
    }).finally(() => { state.failedFlight = null; this.endActivity(); this.collect(); });
  }
  /** One bounded observer flight. End joins actual IO and transfers any ambiguous
   * receipt to the existing cleanup driver; no task waits here on parent finalization. */
  startTransition(chain: BrowserChain): Promise<void> {
    const pairState = this.negotiation;
    if (pairState.transitionFlight) return pairState.transitionFlight;
    if (pairState.linked !== chain || !chain.custody || !(chain.custody.needsClaim || chain.custody.needsKeyRecovery) || chain.closed || !this.negotiationCurrent(pairState) ||
        pairState.actionBusy || this.request || this.historyRequest || this.driving) return Promise.resolve();
    pairState.actionBusy = true; this.beginActivity();
    const pending = Promise.resolve().then(async () => {
      // A returning request can consume almost its whole physical bound. Give indexed
      // media its queued turn between FULL encode/read/body-clear RPCs; one coalesced
      // flight/actionBusy/payment still owns this complete original transition.
      await this.execution.run(async () => {
        if (!this.negotiationCurrent(pairState) || pairState.linked !== chain || chain.closed) return;
        if (chain.custody!.needsClaim) await this.transitionRpc(chain, { kind: "claim" }, false, pairState);
      });
      await this.execution.run(async () => {
        // This fresh admission may have waited behind another peer's Current or End.
        // Repeat original/negative state INSIDE the acquired turn, after owner callbacks.
        if (!this.negotiationCurrent(pairState) || pairState.linked !== chain || chain.closed) return;
        if (chain.custody!.needsKeyRecovery) await this.keySessionRpc(chain, { kind: "adopt" }, false, pairState);
      });
    })
      .catch(() => { /* Keep the original unknown Claim for exact recovery. */ })
      .finally(() => { pairState.transitionFlight = null; pairState.actionBusy = false; this.endActivity(); this.collect(); });
    pairState.transitionFlight = pending; return pending;
  }
  /** Caller already owns the physical lane. Claim also recovers history after End;
   * native independently enforces first-Claim admission against its actual UI/plan. */
  private async transitionRpc(chain: BrowserChain, request: VoiceTransitionRequest["request"], separate = false, pairState: RetainedNativeNegotiation): Promise<void> {
    // Live retirement yields BETWEEN complete physical RPCs, including body-clear finally.
    // The default is already lane-owned (fresh phase preparation); never reacquire recursively.
    if (separate) return this.execution.run(() => this.transitionRpc(chain, request, false, pairState));
    const original = pairState.terminalAction, custody = chain.custody;
    if (pairState.linked !== chain || !original || !custody || !this.keys || !this.bootstrap) throw new Error("original voice custody required");
    const expected = { parentRow: original.row, purpose: original.purpose, identity: original.identity, handshake: chain.binding.handshake };
    const encoded = encodeVoiceTransition({ ...expected, request });
    const started = this.clock.now();
    try {
      await this.framed(encoded.operation, encoded.kind, encoded.logical, encoded.body, true,
        (kind, length) => kind === 0 && (length === 19 || length === 230), opened => {
          if (opened.length === 3) {
            const refusal = decodeVoiceIpcActionReply(opened, original.row);
            if (refusal.kind !== "refused") throw new Error("native custody unresolved");
            if (request.kind === "release" || refusal.code === "capacity" || refusal.code === "stopped") throw new Error("native custody unresolved");
            custody.refuse(); // parent-final still independently proves cleanup
          } else {
            const receipt = decodeVoiceTransition(opened, expected);
            if (request.kind === "release" && (receipt.state !== "released" || receipt.receipt !== request.receipt)) throw new Error("native Release unresolved");
            // Fixed chain metadata pays the retained receipt. Reuse original identities
            // rather than retaining a second decoded selector/action/handshake graph;
            // the raw decoded reply ends in this Control flight's synchronous callback.
            custody.observe(Object.freeze({ ...receipt, parentRow: original.row,
              identity: original.identity, handshake: chain.binding.handshake, epoch: chain.binding.epoch }), started);
            if (receipt.state === "borrowed" && !chain.closed && chain.owner.isCurrent()) {
              // Install quarantine BEFORE the first adoption RPC. A lost reply after old
              // expiry can then recover the SAME original keys, never derive replacements.
              custody.beginKeyAdoption(this.clock.now());
            }
          }
          this.custody.transitionUpdated(chain);
        });
    } finally { encoded.body.fill(0); }
  }
  /** Caller owns the SAME paid Control/physical lane. Adopt is recovery-only after End;
   * first intent was installed while the original keys were alive. No new phase permission
   * is inferred from Current; a later SDK operation must obtain its own native authority. */
  private async keySessionRpc(chain: BrowserChain, request: VoiceKeySessionRequest["request"], separate = false, pairState: RetainedNativeNegotiation): Promise<void> {
    // Live retirement yields BETWEEN complete physical RPCs, including body-clear finally.
    // The default is already lane-owned (fresh phase preparation); never reacquire recursively.
    if (separate) return this.execution.run(() => this.keySessionRpc(chain, request, false, pairState));
    const original = pairState.terminalAction, custody = chain.custody, claim = custody?.receipt;
    if (pairState.linked !== chain || !original || !custody || !claim || !this.keys || !this.bootstrap ||
        (request.kind === "adopt" && !custody.needsKeyRecovery)) throw new Error("original key adoption required");
    const expected = { parentRow: original.row, purpose: original.purpose, identity: original.identity,
      handshake: chain.binding.handshake, claim: claim.receipt, finishMessage: claim.finishMessage, finishHash: claim.finishHash };
    const encoded = encodeVoiceKeySession({ ...expected, request });
    try {
      await this.framed(encoded.operation, encoded.kind, encoded.logical, encoded.body, true,
        (kind, length) => kind === 0 && (length === 19 || length === 250), opened => {
          if (opened.length === 3) {
            const refusal = decodeVoiceIpcActionReply(opened, original.row);
            if (refusal.kind !== "refused" || request.kind === "release" || refusal.code === "capacity" || refusal.code === "stopped") {
              throw new Error("native key adoption unresolved");
            }
            custody.refuseKeys();
          } else {
            const receipt = decodeVoiceKeySession(opened, expected);
            if (request.kind === "release" && (receipt.state !== "released" || receipt.receipt !== request.receipt)) throw new Error("native key release unresolved");
            // Reuse original parent/Claim strings; retained metadata pays only new proof
            // and fixed state. End may have won during IO: observe for cleanup, not revival.
            custody.observeKeys(Object.freeze({ ...receipt, parentRow: original.row, identity: original.identity,
              handshake: chain.binding.handshake, epoch: chain.binding.epoch, finishMessage: claim.finishMessage,
              finishHash: claim.finishHash, claim: claim.receipt }));
          }
          this.custody.transitionUpdated(chain);
        });
    } finally { encoded.body.fill(0); }
  }
  /** A single retained admission flight; it returns void and retains no SDK data result.
   * Never run a future RTC promise while owning SharedIpcExecution: its native result and
   * release calls need that SAME lane. Only these bounded Control RPCs run inside it. */
  startSdp(chain: BrowserChain): Promise<void> {
    const pairState = this.negotiation;
    if (pairState.sdpFlight) return pairState.sdpFlight;
    const phase = chain.sdp;
    if (pairState.linked !== chain || !phase || phase.closed || chain.closed || !this.negotiationCurrent(pairState) ||
        pairState.actionBusy || this.request || this.historyRequest || this.driving) return Promise.resolve();
    pairState.actionBusy = true; this.beginActivity();
    const pending = Promise.resolve().then(async () => {
      await this.execution.run(async () => {
        if (!this.negotiationCurrent(pairState) || pairState.linked !== chain || chain.sdp !== phase || phase.closed || chain.closed) return;
        if (phase.needsStorageRecovery || phase.storage?.state === "preparing") await this.sdpRpc(chain, { kind: "prepare" }, false, pairState);
      });
      // Keep SDK intent/payment and this SAME flight across the yield. A busy queue is
      // not a new attempt; genuinely held individual RPCs retain normal expiry behavior.
      await this.execution.run(async () => {
        if (!this.negotiationCurrent(pairState) || pairState.linked !== chain || chain.sdp !== phase ||
            phase.closed || chain.closed || phase.storage?.state !== "funded") return;
        phase.beginSdk();
        if (phase.needsSdkRecovery) await this.sdkRpc(chain, { kind: "admit" }, false, pairState);
      });
    }).catch(() => { /* Keep the exact paid intent for recovery; no effect was granted. */ })
      .finally(() => { pairState.sdpFlight = null; pairState.actionBusy = false; this.endActivity(); this.collect(); });
    pairState.sdpFlight = pending; return pending;
  }
  /** The existing separately prepaid SDK subtree pays this fixed intent and IO graph.
   * It uses the SAME physical execution position and original channel counters. No RTC
   * promise/provider can enter here or hold that lane while waiting for native effects. */
  startPc(chain: BrowserChain): Promise<void> {
    const pairState = this.negotiation;
    if (pairState.sdpFlight) return pairState.sdpFlight;
    const phase = chain.sdp;
    if (pairState.linked !== chain || !phase || phase.closed || chain.closed || !this.negotiationCurrent(pairState) ||
        phase.storage?.state !== "funded" || phase.sdk?.state !== "held" ||
        pairState.actionBusy || this.request || this.historyRequest || this.driving) return Promise.resolve();
    pairState.actionBusy = true; this.beginActivity();
    const pending = Promise.resolve().then(() => this.execution.run(async () => {
      if (phase.closed || chain.closed || !this.negotiationCurrent(pairState)) return;
      phase.beginPc(); // BEFORE any encoding/clock/crypto/fetch callback can reenter or fail.
      await this.pcRpc(chain, "admit", false, pairState);
    })).catch(() => { /* Ambiguous/refused admission still owes exact Close, never a new PC. */ })
      .finally(() => { pairState.sdpFlight = null; pairState.actionBusy = false; this.endActivity(); this.collect(); });
    pairState.sdpFlight = pending; return pending;
  }
  /** The composite flight/activity is published BEFORE Clock or RTC/provider callbacks.
   * The SDK owner itself is inert until its private native admission joins. Never retrofit
   * an existing crypto-only job with a different RTC object or native PC generation. */
  startRtc(chain: BrowserChain, completeRound = false): Promise<void> {
    const pairState = this.negotiation;
    if (pairState.rtcFlight) return pairState.rtcFlight;
    if (pairState.sdpRoundStage === "done") return Promise.resolve();
    if (!completeRound && pairState.sdpRoundStage !== "offer") throw new Error("original RTC round composition required");
    const phase = chain.sdp;
    if (pairState.rtcCreating || pairState.linked !== chain || !phase || phase.closed || chain.closed ||
        this.retired || this.drained || pairState.actionBusy || pairState.sdpFlight ||
        phase.storage?.state !== "funded" || phase.sdk?.state !== "held" ||
        (pairState.sdpAction && !pairState.sdpRtc)) throw new Error("original RTC custody unavailable");
    pairState.rtcCreating = true; pairState.actionBusy = true; this.beginActivity();
    const flight = Promise.resolve().then(async () => {
      const pc = phase.pc.cryptoBinding(this.clock.now());
      let rtc = pairState.sdpRtc;
      const current: () => boolean = () => {
        if (!this.negotiationCurrent(pairState)) return false;
        // This synchronous window exists ONLY after actual op28 Current has physically
        // joined for this exact peer. It permits media adoption after old Ready expiry;
        // it never authorizes another crypto action or retires the original Source.
        const mediaCurrent = rtc ? pairState.media?.transferCurrent(rtc) : null;
        if (mediaCurrent !== null && mediaCurrent !== undefined) return mediaCurrent;
        try {
          const next = phase.pc.cryptoBinding(this.clock.now());
          return this.negotiation === pairState && !this.retired && !this.drained && pairState.linked === chain && chain.sdp === phase &&
            !phase.closed && !chain.closed && next.proof === pc.proof && next.ready === pc.ready &&
            next.generation === pc.generation && next.localIsLow === pc.localIsLow;
        } catch { return false; }
      };
      if (!current()) throw new Error("original RTC custody unavailable");
      if (!rtc) rtc = pairState.sdpRtc = preparedRtcOwner(pc.localIsLow, current, this.clock, this.custody.rtcConfiguration());
      await this.runRtcAction(chain, rtc, current, pairState);
      if (completeRound && pairState.sdpRoundStage === "offer" && await this.finishRtcAction(pairState)) {
        if (!current()) return;
        pairState.sdpRoundStage = "answer";
        // finishRtcAction's Action/history/adapter frame has ended. Only fixed success
        // metadata crosses into the opposite-purpose SDK/crypto stage.
        await this.runRtcAction(chain, rtc, current, pairState);
      }
      if (completeRound && pairState.sdpRoundStage === "answer" && await this.finishRtcAction(pairState)) {
        if (current()) pairState.sdpRoundStage = "done";
      }
    }).catch(() => {
      // Keep exact native uncertainty for retry. A failed actual SDK peer cannot restart.
      if (pairState.sdpRtc?.closed) phase.close();
    }).finally(() => {
      pairState.rtcFlight = null; pairState.rtcCreating = false; pairState.actionBusy = false;
      this.endActivity(); this.collect();
    });
    pairState.rtcFlight = flight; return flight;
  }
  /** One active Action/adapter graph at a time. First Offer and opposite-purpose Answer
   * share the actual original PC, but each owns distinct native terminal/Return history. */
  private async runRtcAction(chain: BrowserChain, rtc: VoiceSdpRtcOwner, current: () => boolean, pairState: RetainedNativeNegotiation): Promise<void> {
    const phase = chain.sdp!;
    const localSeal = chain.binding.localIsLow === (pairState.sdpRoundStage === "offer");
    if (localSeal && !pairState.sdpAction) {
      await rtc.prepareLocal(async () => {
        // New actual RPC; the original conservative deadline is never renewed.
        await this.execution.run(() => this.pcRpc(chain, "admit", false, pairState));
        const now = this.clock.now();
        if (!current()) throw new Error("original RTC admission changed");
        return phase.pc.localPreparationDeadline(now);
      }, pairState.sdpRoundStage === "offer" ? "offer" : "answer");
      if (!current()) throw new Error("original RTC preparation retired");
    }
    await this.runSdpCrypto(chain, null, rtc, pairState);
  }
  private async finishRtcAction(pairState: RetainedNativeNegotiation): Promise<boolean> {
    let action: VoiceSdpActionOwner | null = pairState.sdpAction;
    try {
      if (!action?.complete) return false;
      await action.finish();
      if (!action.complete || !action.returned) return false;
      pairState.sdpAction = null;
      return true;
    } finally { action = null; } // No first Action graph across the next SDK/Begin await.
  }

  startMedia(chain: BrowserChain): Promise<void> {
    const pairState = this.negotiation;
    if (pairState.linked !== chain) throw new Error("original native media chain required");
    if (pairState.media) return pairState.media.start();
    if (pairState.linked !== chain || !chain.sdp || chain.sdpRows!==2 || !pairState.sdpRtc || pairState.sdpRoundStage !== "done" ||
        pairState.rtcFlight || pairState.sdpFlight || pairState.actionBusy || this.retired || this.drained) throw new Error("completed original RTC round required");
    if (this.peers.size >= MAX_RETAINED_MEDIA || this.peers.has(chain.handle)) throw new Error("voice media capacity reached");
    const refund = this.custody.mediaPrepared(); // BEFORE constructor/Clock/SDK/native callbacks.
    const peer: RetainedNativePeer = { plan: chain.handle, generation: pairState.generation, refund,
      media: null, compacted: false, compactionReturned: false, advanced: false, closing: false, refundReady: false, cleanup: null };
    try {
      const phase = chain.sdp, storage = phase.storage, sdk = phase.sdk;
      if (!storage || !sdk) throw new Error("original media storage unavailable");
      const original = { ...this.sdpIdentity(chain, pairState), storage: storage.proof,
        sdkGeneration: sdk.generation, sdkProof: sdk.proof, pcProof: phase.pc.originalProof() };
      const media = new VoiceNativeMedia(original, pairState.sdpRtc, this.mediaContext(peer));
      peer.media = media; this.peers.set(peer.plan, peer);
      pairState.media = media; // Registry and original owner both exist before callbacks.
      return media.start();
    } catch (error) { if (!peer.media) refund(); throw error; }
  }

  /** Scope contains only shared Session infrastructure, never an old chain/Phase/SDK. */
  private mediaContext(peer: RetainedNativePeer): VoiceNativeMediaContext {
    return { clock: this.clock, live: () => this.live(),
      begin: () => this.beginActivity(), end: () => this.endActivity(),
      stopped: () => this.mediaStopped(peer), refund: () => { peer.refundReady = true; },
      request: (encoded, cleanup) => this.execution.run(() => this.framed(
        encoded.operation, encoded.kind, encoded.logical, encoded.body, cleanup,
        (kind, length) => kind === 0 && (length === 18 || length === 20 || length === 55), decodeVoiceMedia)) };
  }
  private mediaStopped(peer: RetainedNativePeer): void {
    peer.closing = true;
    if (this.retired) return; // Global End owns the coalesced joins after closing ALL peers.
    if (!peer.compacted) {
      const state = this.negotiation, chain = state.linked;
      if (!chain || peer.plan !== chain.handle || peer.generation !== state.generation ||
          peer.media !== state.media || this.peers.get(peer.plan) !== peer) return;
      state.failed = true;
      try { chain.handle.close(); } finally { this.wake(); }
      return;
    }
    void this.cleanupPeer(peer);
  }
  private cleanupPeer(peer: RetainedNativePeer): Promise<void> {
    if (peer.cleanup) return peer.cleanup;
    if (!peer.media) return Promise.resolve();
    const work = Promise.resolve().then(async () => {
      for (;;) {
        const observed = this.wakeIdentity;
        try { await peer.media!.cleanup(); break; }
        catch { await this.waitCleanupWake(observed); }
      }
      if (!peer.refundReady) throw new Error("actual media cleanup has not returned");
      const state = this.negotiation;
      if (state.generation === peer.generation && state.media === peer.media) state.media = null;
      peer.media = null;
      // Failed pre-compaction records have no transferred2KiB marker payment. Their
      // old chain/native history remains independently paid; remove this registry owner
      // before the post-worker driver refund rather than retaining an unpaid marker.
      if (!peer.compacted && this.peers.get(peer.plan) === peer) this.peers.delete(peer.plan);
    });
    // Separate completion scope follows actual driver cleanup and heavy-worker return.
    peer.cleanup = this.peerCleanupReturned(work, peer); return peer.cleanup;
  }
  private peerCleanupReturned(work: Promise<void>, peer: RetainedNativePeer): Promise<void> {
    return work.then(() => { peer.refund(); }).finally(() => { peer.cleanup = null; });
  }
  compacted(plan: VoiceCryptoChain): boolean { return this.peers.get(plan)?.compacted === true; }
  compactedMedia(plan: VoiceCryptoChain): Promise<void> {
    const peer = this.peers.get(plan);
    if (!peer?.compacted || !peer.media) throw new Error("original compacted media required");
    return peer.media.start();
  }
  /** Pure final publication barrier, shared by both kinds of retired negotiation. */
  private negotiationQuiet(state: RetainedNativeNegotiation): boolean {
    // Both successful compaction and failed-plan retirement share this whole barrier.
    // A queued history driver still captures the old N for its End joins even when its
    // current request appears harmless. Never replace N before that driver's finally.
    return !this.driving && !this.request && !this.historyRequest && !this.retry &&
      !state.compactFlight && !state.failedFlight && !state.sdkPosition && !state.workPump && !state.linked && !state.intake && !state.media &&
      !state.transitionFlight && !state.derivationReturn && !state.sdpFlight && !state.rtcFlight && !state.negotiationFlight &&
      !state.actionBusy && !state.rtcCreating && !state.sdpCreating && !state.sdpRtc && !state.sdpAction && state.negotiationRetired;
  }
  /** Publication is explicit after the public retirement/compaction observer returns,
   * never inside old accounting callbacks. An old marker cannot replace a newer owner. */
  advanceNegotiation(plan: VoiceCryptoChain): boolean {
    const peer = this.peers.get(plan), pairState = this.negotiation;
    if (pairState.failedReturned && pairState.failedPlan === plan) {
      if (!this.live() || this.retired || this.drained || this.negotiation !== pairState || !this.keys || !this.negotiationQuiet(pairState))
        return false;
      // The same fixed Session work envelope pays the replacement only AFTER the actual
      // failed worker returned. Old chain/action history stays paid in the broker/native
      // ledgers until End. This does not remove history or reset any Root action budget.
      this.negotiation = new RetainedNativeNegotiation(); return true;
    }
    if (this.retired || this.drained || !peer?.compacted || !peer.compactionReturned) return false;
    if (peer.advanced) return true;
    if (!this.negotiationCurrent(pairState)) return false;
    if (this.negotiation !== pairState || this.retired || this.drained || !this.keys || peer.generation !== pairState.generation ||
        !this.negotiationQuiet(pairState)) return false;
    const next = new RetainedNativeNegotiation(); // Inert, no callback between checks/publication.
    peer.advanced = true; this.negotiation = next; return true;
  }
  compactNegotiation(chain: BrowserChain): Promise<boolean> {
    const pairState = this.negotiation;
    if (pairState.compactFlight) return pairState.compactFlight;
    if (this.compacted(chain.handle)) return compactedHistory;
    if (pairState.linked !== chain || !pairState.negotiationRetired || !pairState.media?.detached || !pairState.finalAcknowledged)
      throw new Error("retired original browser history required");
    const media = pairState.media, plan = chain.handle;
    this.beginActivity();
    const work = Promise.resolve().then(async () => {
      let prepared: PreparedBrowserHistory | null = null;
      try {
        if (pairState.negotiationFlight) await pairState.negotiationFlight;
        let collected = false;
        try { collected = await media.collectNativeHistory(); } catch { return -1; }
        if (!collected || !this.live()) return -1;
        if (this.retired || this.drained || pairState.linked !== chain || pairState.media !== media || media.closed) return -1;
        try { prepared = await this.custody.prepareCompaction(chain); }
        catch { return -1; } // Same old history remains paid; no new native admission.
        // Last callback precedes ALL private final checks and the synchronous tail.
        if (!this.live() || this.retired || this.drained || pairState.linked !== chain || pairState.media !== media || media.closed) return -1;
        const peer = this.peers.get(plan);
        if (!peer || peer.media !== media || peer.compacted) return -1;
        const refund = prepared.commit(); prepared = null;
        peer.compacted = true; // Same driver remains continuously owned by this paid record.
        pairState.linked = null; pairState.media = null;
        // Native mode8 already proves exact parent-final/ACK and actual destruction.
        // Drop old scalar history; explicit post-return publication opens the next owner.
        pairState.terminalAction = null; pairState.finalProof = null; pairState.finalAcknowledged = false;
        pairState.cleanupWork = null; pairState.cleanupParent = null;
        return refund;
      } finally { prepared?.abort(); }
    });
    // The public/coalesced completion is created in a DIFFERENT method scope. It cannot
    // retain this function's chain/rows/prepared closures after the heavy worker returns.
    const peer = this.peers.get(plan)!;
    const flight = compactionReturned(this.compactionCompletion(work), peer);
    pairState.compactFlight = flight; return flight;
  }
  private compactionCompletion(work: Promise<number>): Promise<boolean> {
    const pairState = this.negotiation;
    // This scope retains only shared paid infrastructure and the numeric work result.
    // Fulfilment proves the heavy preparation/commit/finally scopes actually returned;
    // original fixed history pays them until this point. No16KiB overlap assumption.
    return work.then(refund => {
      if (refund < 0) return false;
      this.custody.compactionReturned(refund);
      return true;
    }).finally(() => { pairState.compactFlight = null; this.endActivity(); this.collect(); });
  }

  collectNegotiationHistory(chain: BrowserChain): Promise<boolean> {
    const pairState = this.negotiation;
    if (pairState.linked!==chain || !pairState.negotiationRetired || !pairState.media)
      throw new Error("retired original negotiation required");
    return pairState.media.collectNativeHistory();
  }

  retireNegotiation(chain: BrowserChain): Promise<void> {
    const pairState = this.negotiation;
    if (pairState.linked!==chain || !pairState.media?.adopted) throw new Error("adopted original media required");
    if (pairState.negotiationFlight) return pairState.negotiationFlight;
    if (pairState.negotiationRetired) return Promise.resolve();
    if (!this.live() || this.retired || this.drained) throw new Error("original media ended");
    // Owner predicates can reenter. Repeat publication/negative gates after the LAST
    // callback, before reserving the one finite retirement flight and its old phase fee.
    if (pairState.negotiationFlight) return pairState.negotiationFlight;
    if (pairState.negotiationRetired) return Promise.resolve();
    if (pairState.linked!==chain || this.retired || this.drained || !pairState.media?.adopted || pairState.media.closed) throw new Error("original media ended");
    this.beginActivity();
    const flight=Promise.resolve().then(async () => {
      while (this.live() && !pairState.negotiationRetired) {
        const observed=this.wakeIdentity;
        try {
          const media=pairState.media;
          if (!media || media.closed) return;
          await media.requestDetach();
          if (!media.detached) throw new Error("original source still retained");
          // Positive authenticated Detach has physically joined. Fence old negotiation
          // immediately, before any held metadata/SDK join. This grants NO media effect;
          // its independent Current still controls the peer and all old fees remain paid.
          chain.mediaIndexed=true;
          // Stop old metadata adoption before releasing its keys. This joins native IPC
          // outside the shared lane; the independently paid media monitor keeps running.
          if (pairState.workPump) { await pairState.workPump.close(); pairState.workPump=null; }
          if (pairState.rtcFlight) await pairState.rtcFlight;
          if (pairState.sdpFlight) await pairState.sdpFlight;
          if (pairState.sdpRtc) await pairState.sdpRtc.drain(); // source has no peer after transfer
          if (!this.live()) return;
          // Retain this original negotiation flight/fee, but not the physical lane
          // across the batch. Each FULL RPC (encode through body-clear finally) yields
          // to an already queued media Current before the next cleanup request starts.
          // End joins negotiationFlight before its separate cleanup driver.
          await this.cleanupConfirmationIntents(true, pairState);
          await this.cleanupSdp(chain,true, pairState);
          const custody=chain.custody!;
          if (custody.needsKeyRelease) await this.keySessionRpc(chain,{kind:"release",receipt:custody.keyReceipt!.receipt},true, pairState);
          if (custody.needsRelease) await this.transitionRpc(chain,{kind:"release",receipt:custody.receipt!.receipt},true, pairState);
          if (!pairState.finalAcknowledged) await this.retireOriginalAction(true, pairState);
          // Actual old IO finally and native Plan-final/ACK precede the64KiB refund.
          // Retain fixed history, shared keys/counters and media custody until actual End.
          const phase=chain.sdp;
          pairState.sdpAction=null; pairState.sdpRtc=null;
          chain.record?.pair?.retire();
          for (const record of chain.records.values()) record.pair=null;
          if (phase) this.custody.sdpUnlinked(chain,phase);
          pairState.negotiationRetired=true;
        } catch {
          if (this.live()) await this.waitCleanupWake(observed);
        }
      }
    }).finally(() => { pairState.negotiationFlight=null; this.endActivity(); this.collect(); });
    pairState.negotiationFlight=flight; return flight;
  }

  /** One separately paid crypto owner; no awaited SDK holds the native-RPC lane. Only
   * the original private Pair can compute, and original native18 admits that one engine.
   * The crypto-only leaf remains first-Offer; the RTC composer selects Answer separately. */
  startSdpCrypto(chain: BrowserChain, description: string | null): Promise<void> {
    const pairState = this.negotiation;
    if (pairState.sdpRoundStage !== "offer") throw new Error("original RTC round composition required");
    return this.runSdpCrypto(chain, description, null, pairState);
  }
  /** The composite RTC flight keeps its exclusion through affine adoption. In particular,
   * owner/Clock callbacks cannot enqueue status/history in the gap before Action publication.
   * The SDK slot is taken only after fallible validation, immediately before the inert Action
   * constructor. A retryable refusal therefore leaves the SAME original bytes with RTC. */
  private runSdpCrypto(chain: BrowserChain, description: string | null, rtcOwner: VoiceSdpRtcOwner | null, pairState: RetainedNativeNegotiation): Promise<void> {
    const rtcTransfer = rtcOwner !== null;
    if (rtcTransfer && (rtcOwner !== pairState.sdpRtc || !pairState.rtcCreating || !pairState.rtcFlight || !pairState.actionBusy)) {
      throw new Error("original RTC transfer unavailable");
    }
    if (pairState.sdpCreating) throw new Error("voice SDP creation is already in progress");
    const phase = chain.sdp;
    if (pairState.linked !== chain || !phase || phase.closed || chain.closed || !this.negotiationCurrent(pairState) ||
        phase.storage?.state !== "funded" || phase.sdk?.state !== "held" ||
        this.request || this.historyRequest || this.driving) throw new Error("original SDP crypto custody unavailable");
    if (!pairState.sdpAction) {
      if (pairState.sdpFlight || (pairState.actionBusy && !rtcTransfer)) throw new Error("voice channel is busy");
      // Clock/owner seams can synchronously reenter this method. Publish the creation
      // fence before either callback; no second owner/backing may replace the first.
      pairState.sdpCreating = true;
      try {
        const pc = phase.pc.cryptoBinding(this.clock.now()), base = this.sdpIdentity(chain, pairState);
        const ownerCurrent = this.negotiationCurrent(pairState);
        if (!ownerCurrent || this.negotiation !== pairState || this.retired || this.drained || pairState.linked !== chain || chain.sdp !== phase || phase.closed || chain.closed ||
            pairState.sdpAction || pairState.sdpFlight || (pairState.actionBusy && !rtcTransfer)) throw new Error("original SDP crypto custody unavailable");
        const localSeal = pc.localIsLow === (pairState.sdpRoundStage === "offer");
        if (localSeal !== (description !== null || (rtcTransfer && localSeal))) throw new Error("first SDP Offer role required");
        if (pairState.sdpRtc && description !== null) throw new Error("RTC description must come from the original SDK owner");
        const original = Object.freeze({ ...base, storage: phase.storage.proof, sdkGeneration: phase.sdk.generation,
          sdkProof: phase.sdk.proof, pcProof: pc.proof });
        const purpose = localSeal ? "seal" as const : "open" as const;
        const live = () => {
          // Key custody intentionally outlives this PC round. Never use Pair currentness
          // as the PC deadline, including a permit reply held across the original expiry.
          if (!this.negotiationCurrent(pairState)) return false;
          try {
            const current = phase.pc.cryptoBinding(this.clock.now());
            // Clock and owner predicates can reenter End. All final checks are callback-free.
            return this.negotiation === pairState && !this.retired && !this.drained && pairState.linked === chain && chain.sdp === phase &&
              !phase.closed && !chain.closed && current.proof === pc.proof && current.ready === pc.ready &&
              current.generation === pc.generation && current.localIsLow === pc.localIsLow;
          } catch { return false; }
        };
        const expected = (row: string, identity: VoiceIpcActionIdentity): VoiceSdpInputExpectation => ({
          row, parentRow: base.parentRow, identity, purpose, generation: pc.generation, ready: pc.ready,
          key: base.key, sdkGeneration: "1", sdkProof: original.sdkProof, storage: original.storage,
          pcProof: pc.proof, localIsLow: pc.localIsLow,
        });
        const control = (encoded: ReturnType<typeof encodeVoiceSdpBegin>, row: string | null, close: boolean | null = null) =>
          this.execution.run(async () => {
            try { return await this.framed(encoded.operation, encoded.kind, encoded.logical, encoded.body, true,
              (kind, length) => kind === 0 && (length === 20 || length === 36 || length === 236),
              bytes => decodeVoiceSdpReply(bytes, { parentRow: base.parentRow, row, purpose, resolveClose: close })); }
            finally { encoded.body.fill(0); }
          });
        const action = (row: string, request: Parameters<typeof encodeVoiceSdpAction>[1]) =>
          control(encodeVoiceSdpAction({ parentRow: base.parentRow, row, purpose }, request), row);
        // Consume the string synchronously. No async closure or completed observer retains
        // it; the one8KiB UTF8 backing releases only after native attaches the Action.
        let plaintext = rtcTransfer && localSeal ? rtcOwner.takeLocal()
          : (description === null ? null : voiceSdpUtf8(description));
        description = null; // Do not depend on a VM's closure-environment liveness analysis.
        const rtc = pairState.sdpRtc;
        pairState.sdpAction = new VoiceSdpActionOwner({ live,
          begin: bytes => control(encodeVoiceSdpBegin(original, bytes), null),
          resolve: close => control(encodeVoiceSdpResolve(original, purpose, close), null, close),
          input: (row, identity, loan) => this.execution.run(async () => {
            const encoded = encodeVoiceSdpAction({ parentRow: base.parentRow, row, purpose }, { kind: "input", identity });
            try { return await this.framed(encoded.operation, encoded.kind, encoded.logical, encoded.body, false,
              (kind, length) => (kind === 2 && length <= 9232) || (kind === 0 && (length === 20 || length === 236)),
              bytes => decodeVoiceSdpReply(bytes, { parentRow: base.parentRow, row, purpose, resolveClose: null }),
              { loan, expected: expected(row, identity), current: () => {
                if (!live()) throw new Error("voice SDP Input retired");
                // Read/cancel/timer finally can reenter End. Recheck the original PC's
                // conservative deadline immediately before moving this exact backing.
                const current = phase.pc.cryptoBinding(this.clock.now());
                if (!live() || current.proof !== pc.proof || current.ready !== pc.ready) throw new Error("voice SDP Input retired");
              } }); }
            finally { encoded.body.fill(0); }
          }),
          admit: (row, identity, engineBytes) => this.execution.run(async () => {
            const encoded = encodeVoiceSdpAction({ parentRow: base.parentRow, row, purpose }, { kind: "admit", identity, engineBytes });
            try { return await this.framed(encoded.operation, encoded.kind, encoded.logical, encoded.body, false,
              (kind, length) => kind === 0 && (length === 20 || length === 94 || length === 236),
              bytes => decodeVoiceSdpEngineReply(bytes, { parentRow: base.parentRow, row, purpose, identity, engineBytes })); }
            finally { encoded.body.fill(0); }
          }),
          reserveEngine: () => this.reserveEngine(chain, pairState),
          compute: async input => {
            const current = phase.pc.cryptoBinding(this.clock.now());
            if (!live()) throw new Error("voice SDP engine retired");
            if (current.proof !== pc.proof || current.ready !== pc.ready) throw new Error("original voice SDP PC changed");
            const pair = chain.record?.pair;
            if (!pair) throw new Error("original voice Pair unavailable");
            return pair.computeSdp(SDP_COMPUTE, input, live);
          },
          digest: async output => {
            const digest = new Uint8Array(await this.subtle.digest("SHA-256", output));
            try { return Array.from(digest, value => value.toString(16).padStart(2, "0")).join(""); }
            finally { digest.fill(0); }
          },
          result: (row, identity, output, tag) => action(row, output === null ? { kind: "authentication-failed", identity }
            : { kind: purpose === "seal" ? "sealed" : "opened", identity, bytes: output, tag: tag! }),
          ack: (row, identity, digest) => action(row, { kind: "ack", identity, digest }),
          settle: (row, identity) => action(row, { kind: "settle", identity }),
          returned: async row => {
            // Successful live compaction closes only this Action. End must still join/close
            // the actual PC before Return and every shared SDK/storage release.
            if (!live()) await rtc?.drain();
            return action(row, { kind: "return" });
          },
          ...(rtc ? {
            delivery: (row: string, identity: VoiceIpcActionIdentity, applied: boolean) => action(row, { kind: "delivery", identity, applied }),
            applyRemote: (bytes: Uint8Array<ArrayBuffer>) => rtc.applyRemote(bytes, pc.localIsLow ? "answer" : "offer"),
          } : {}),
        }, plaintext);
        plaintext = null; // The job is the sole owner, including if this closure frame survives.
      } finally { pairState.sdpCreating = false; }
    } else if (description !== null) throw new Error("original SDP description already consumed; retry without it");
    if (pairState.sdpFlight) return pairState.sdpFlight;
    if (pairState.actionBusy && !rtcTransfer) throw new Error("voice channel is busy");
    pairState.actionBusy = true; this.beginActivity();
    const pending = pairState.sdpAction.attempt().catch(() => { /* Retain exact paid work/output for retry or End. */ })
      .finally(() => { pairState.sdpFlight = null; if (!rtcTransfer) pairState.actionBusy = false; this.endActivity(); this.collect(); });
    pairState.sdpFlight = pending; return pending;
  }
  private sdpIdentity(chain: BrowserChain, pairState: RetainedNativeNegotiation) {
    const original = pairState.terminalAction, key = chain.custody?.keyReceipt;
    if (pairState.linked !== chain || !chain.sdp || !original || !key || !this.keys || !this.bootstrap) throw new Error("original SDP custody required");
    return { parentRow: original.row, identity: original.identity, handshake: chain.binding.handshake, key: key.receipt, sequence: "1" };
  }
  /** Caller holds the original physical lane. After End, Prepare is exact recovery only;
   * native independently refuses first admission. No retry manufactures another identity. */
  private async sdpRpc(chain: BrowserChain, request: VoiceSdpPhaseRequest["request"], separate = false, pairState: RetainedNativeNegotiation): Promise<void> {
    // Live retirement yields BETWEEN complete physical RPCs, including body-clear finally.
    // The default is already lane-owned (fresh phase preparation); never reacquire recursively.
    if (separate) return this.execution.run(() => this.sdpRpc(chain, request, false, pairState));
    const phase = chain.sdp!, expected = this.sdpIdentity(chain, pairState), encoded = encodeVoiceSdpPhase({ ...expected, request });
    try {
      await this.framed(encoded.operation, encoded.kind, encoded.logical, encoded.body, true,
        (kind, length) => kind === 0 && (length === 19 || length === 186), opened => {
          if (opened.length === 3) {
            const refusal = decodeVoiceIpcActionReply(opened, expected.parentRow);
            if (refusal.kind !== "refused" || request.kind === "release" || refusal.code === "capacity" || refusal.code === "stopped" || refusal.code === "closed") throw new Error("native SDP storage unresolved");
            phase.refuseStorage(); // Independent original Final still gates cleanup.
          } else {
            const receipt = decodeVoiceSdpPhase(opened, expected);
            if (request.kind === "release" && (receipt.state !== "drained" || receipt.proof !== request.proof)) throw new Error("native SDP release unresolved");
            phase.observeStorage(receipt);
          }
        });
    } finally { encoded.body.fill(0); }
  }
  private async sdkRpc(chain: BrowserChain, request: VoiceSdkRequest["request"], separate = false, pairState: RetainedNativeNegotiation): Promise<void> {
    // Live retirement yields BETWEEN complete physical RPCs, including body-clear finally.
    // The default is already lane-owned (fresh phase preparation); never reacquire recursively.
    if (separate) return this.execution.run(() => this.sdkRpc(chain, request, false, pairState));
    const phase = chain.sdp!, storage = phase.storage;
    if (!storage) throw new Error("native SDP storage required");
    const expected = { ...this.sdpIdentity(chain, pairState), storage: storage.proof }, encoded = encodeVoiceSdkCustody({ ...expected, request });
    try {
      await this.framed(encoded.operation, encoded.kind, encoded.logical, encoded.body, true,
        (kind, length) => kind === 0 && (length === 19 || length === 226), opened => {
          if (opened.length === 3) {
            const refusal = decodeVoiceIpcActionReply(opened, expected.parentRow);
            if (refusal.kind !== "refused" || request.kind === "release" || refusal.code === "capacity" || refusal.code === "stopped" || refusal.code === "closed") throw new Error("native SDK custody unresolved");
            phase.refuseSdk();
          } else {
            const receipt = decodeVoiceSdkCustody(opened, expected);
            if (request.kind === "release" && (receipt.state !== "released" || receipt.proof !== request.proof || receipt.generation !== request.generation)) throw new Error("native SDK release unresolved");
            phase.observeSdk(receipt);
          }
        });
    } finally { encoded.body.fill(0); }
  }
  private async pcRpc(chain: BrowserChain, request: "admit" | "close", separate = false, pairState: RetainedNativeNegotiation): Promise<void> {
    // Live retirement yields BETWEEN complete physical RPCs, including body-clear finally.
    // The default is already lane-owned (fresh phase preparation); never reacquire recursively.
    if (separate) return this.execution.run(() => this.pcRpc(chain, request, false, pairState));
    const phase = chain.sdp!, storage = phase.storage, sdk = phase.sdk;
    if (!storage || !sdk) throw new Error("original native PC custody required");
    const requestStarted = this.clock.now();
    // Clock and owner predicates are callback seams. End here leaves the latched cleanup
    // duty, but must not submit a fresh admission. Close remains available after End.
    if (request === "admit" && (!this.negotiationCurrent(pairState) || phase.closed || chain.closed)) throw new Error("voice PC intent is closed");
    const encoded = encodeVoicePcIntent({ ...this.sdpIdentity(chain, pairState), storage: storage.proof,
      sdkGeneration: sdk.generation, sdkProof: sdk.proof, request });
    try {
      await this.framed(encoded.operation, encoded.kind, encoded.logical, encoded.body, request === "close",
        (kind, length) => kind === 0 && (length === 19 || length === 59 || length === 101), opened => {
          // A refusal may follow actual native intent/Actor publication. Only version7
          // Closed proves this effect fence; never infer absence from Unauthorized.
          const receipt = decodeVoicePcIntent(opened);
          if (receipt.state === "admitted" && receipt.localIsLow !== chain.binding.localIsLow) throw new Error("native PC role changed");
          if (request === "close" && receipt.state !== "closed") throw new Error("native PC close unresolved");
          phase.pc.observe(receipt, requestStarted, this.clock.now());
        });
    } finally { encoded.body.fill(0); }
  }
  /** Only after actual local admission/SDK/IO finally. Callers select individual turns;
   * live retirement instead gives each complete RPC its own physical turn. */
  private async cleanupSdp(chain: BrowserChain, separate = false, pairState: RetainedNativeNegotiation): Promise<void> {
    const phase = chain.sdp; if (!phase) return;
    phase.close();
    if (phase.needsStorageRecovery) await this.sdpRpc(chain, { kind: "prepare" }, separate, pairState);
    if (phase.needsSdkRecovery) await this.sdkRpc(chain, { kind: "admit" }, separate, pairState);
    if (phase.pc.needsClose) await this.pcRpc(chain, "close", separate, pairState);
    if (phase.needsSdkRelease) await this.sdkRpc(chain, { kind: "release", generation: phase.sdk!.generation, proof: phase.sdk!.proof }, separate, pairState);
    if (phase.needsStorageRelease) await this.sdpRpc(chain, { kind: "release", proof: phase.storage!.proof }, separate, pairState);
    if (!phase.nativeReleased) throw new Error("native SDK cleanup unresolved");
  }
  wakeToken(): object { return this.wakeIdentity; }
  async waitWake(observed: object): Promise<void> {
    if (this.wakeIdentity !== observed) return;
    if (this.cleanupWakeFlight) return this.cleanupWakeFlight;
    const waiting = this.retry ??= signal<void>();
    try { await waiting.promise; }
    finally { if (this.retry === waiting) this.retry = null; }
  }
  /** One Clock/availability wake for the ORIGINAL action, including lost replies with no
   * network event to wake the app. Retries repeat native identity/output, never computation. */
  async waitActionRetry(record: ActionRecord, waiting: Signal<void>): Promise<void> {
    const deadline = this.clock.now() + IPC_CLEANUP_RETRY_MS;
    let active = true, timer: unknown = null;
    const expire = () => {
      if (!active || record.retry !== waiting) return;
      const remaining = deadline - this.clock.now();
      if (remaining > 0) timer = this.clock.schedule(expire, remaining); else waiting.resolve();
    };
    try { timer = this.clock.schedule(expire, IPC_CLEANUP_RETRY_MS); await waiting.promise; }
    finally { active = false; if (timer !== null) this.clock.cancel(timer); }
  }
  waitActionCleanup(observed: object): Promise<void> { return this.waitCleanupWake(observed); }
  /** Root's final-retirement snapshot has no notification into the browser. Keep one
   * original paid cleanup driver alive across temporary native refusals without depending
   * on remount/availability. Timer and availability resolve the SAME Signal: racing two
   * asynchronous waiters would leak a waitWake continuation on every expired interval.
   * This is cleanup only, after actual SDK/IO/linked drainage; it grants no action authority. */
  private waitCleanupWake(observed: object): Promise<void> {
    if (this.wakeIdentity !== observed) return Promise.resolve();
    if (this.cleanupWakeFlight) return this.cleanupWakeFlight;
    // Publish the shared flight BEFORE Clock/scheduler callbacks. Joining peers do not
    // acquire timer cleanup ownership: a joining fault cannot detach another waiter.
    const work = Promise.resolve().then(() => this.driveCleanupWake(observed));
    const flight = work.finally(() => { if (this.cleanupWakeFlight === flight) this.cleanupWakeFlight = null; });
    this.cleanupWakeFlight = flight; return flight;
  }
  private async driveCleanupWake(observed: object): Promise<void> {
    if (this.wakeIdentity !== observed) return;
    const waiting = this.retry ??= signal<void>();
    const deadline = this.clock.now() + IPC_CLEANUP_RETRY_MS;
    let active = true, timer: unknown = null;
    const expire = () => {
      // Cancellation can race a queued callback. An old wait must never wake its successor.
      if (!active || this.retry !== waiting) return;
      const remaining = deadline - this.clock.now();
      if (remaining > 0) timer = this.clock.schedule(expire, Math.max(1, remaining));
      else waiting.resolve();
    };
    try {
      // Publish the Signal first, then arm ONE timer. The identity recheck also covers a
      // synchronous availability callback from an injected scheduler during installation.
      timer = this.clock.schedule(expire, IPC_CLEANUP_RETRY_MS);
      if (this.wakeIdentity !== observed) waiting.resolve();
      await waiting.promise;
    } finally {
      active = false;
      if (timer !== null) this.clock.cancel(timer);
      if (this.retry === waiting) this.retry = null;
    }
  }
  observe(): void { this.unsubscribe = this.selected.owner.onRetired(() => this.retire()); }
  /** Constructor subscription refusal precedes every RPC/SDK call and may roll back locally. */
  abandon(): void {
    if (this.driving || this.attempted) throw new Error("voice channel already started");
    this.retired = true; this.drained = true; this.collected = true;
    this.custody.setupDrained(); this.custody.closed(); this.custody.collected();
    this.completion.resolve();
  }
  wake(): void {
    const pairState = this.negotiation;
    this.wakeIdentity = {}; this.retry?.resolve();
    pairState.workPump?.wake();
    // Availability is one bounded wake for retained rows, never a new computation or timer.
    if (pairState.linked) for (const record of pairState.linked.records.values()) record.retry?.resolve();
  }
  private live(): boolean { return !this.retired && this.selected.owner.isCurrent() && !this.retired; }
  /** Fresh per-pair effects must survive both sides of the owner callback. Exact old
   * cleanup and independently indexed media use their separate original bindings. */
  private negotiationCurrent(state: RetainedNativeNegotiation): boolean {
    return this.negotiation === state && !state.failed && !this.retired && !this.drained && this.live() &&
      this.negotiation === state && !state.failed && !this.retired && !this.drained;
  }
  private check(): void { if (!this.live()) throw new Error("voice channel closed"); }
  private retire(): void {
    const pairState = this.negotiation;
    if (!this.retired) { this.retired = true; this.controller?.abort(); }
    pairState.sdkPosition?.cancel();
    for (const peer of this.peers.values()) {
      peer.closing = true;
      try { peer.media?.close(); } catch { /* Continue closing every actual peer; cleanup retries custody. */ }
    }
    pairState.sdpRtc?.close();
    pairState.sdpAction?.close();
    void this.lifecycle?.close();
    void pairState.workPump?.close();
    this.wake();
    // Retire the original prepaid intake even while its owner remains current. The broker
    // retains running SDK/read custody until its finally; an idle claim needs no expiry wait.
    pairState.intake?.retire();
    pairState.linked?.handle.close();
    this.request?.resolve(Object.freeze({ kind: "retired" })); this.request = null;
    // Early observation is not physical completion. Keep the SAME query/body in State and
    // the existing driver until actual SDK/read/cancellation and native cleanup finish.
    this.historyRequest?.result.resolve(Object.freeze({ kind: "retired" }));
    if (this.drained) this.collect(); else this.start();
  }
  private collect(): void {
    if (this.collected || !this.drained || this.activity !== 0 || this.selected.owner.isCurrent()) return;
    this.collected = true; this.unsubscribe(); this.peers.clear(); this.custody.collected();
  }
  private status(): Promise<VoiceIpcStatus> {
    const pairState = this.negotiation;
    if (this.historyRequest || pairState.actionBusy) return Promise.resolve(Object.freeze({ kind: this.live() ? "uncertain" : "retired" }));
    this.wake();
    if (!this.live()) return Promise.resolve(Object.freeze({ kind: "retired" }));
    if (this.request) return this.request.promise;
    this.request = signal<VoiceIpcStatus>();
    const promise = this.request.promise; this.start(); return promise;
  }
  private start(): void {
    if (this.driving || this.drained) return;
    // Publish ownership before scheduling a microtask; reentrant End cannot create a second
    // driver. Counts include the actual flight, while its prepaid position also covers idle.
    this.driving = true; this.beginActivity();
    void Promise.resolve().then(() => this.drive());
  }
  private async drive(): Promise<void> {
    const pairState = this.negotiation;
    let outcome: VoiceIpcStatus = Object.freeze({ kind: "uncertain" });
    const history = this.historyRequest;
    let historical: VoiceIpcHistoryOutcome = Object.freeze({ kind: "uncertain" });
    try {
      if (history) {
        await this.execution.run(async () => {
          if (this.live() && this.keys && this.bootstrap) historical = await this.exchange(history);
        });
      } else if (this.live() && await this.setup()) {
        await this.execution.run(async () => { if (this.live()) outcome = await this.exchange(null); });
      }
    } catch {
      // Engine/setup validation failure cannot restart construction. Transport/body failures
      // after keys exist remain retryable using fresh physical counters on this SAME Session.
      if (!this.keys) { this.retired = true; outcome = Object.freeze({ kind: "failed" }); historical = Object.freeze({ kind: "failed" }); }
    } finally {
      history?.body.fill(0); // the shared execution promise includes real SDK/read cancellation
      if (!this.live()) {
        if (history) historical = Object.freeze({ kind: "retired" });
        // The captured history driver still owns its payment/query until this finally
        // completes. Its actual execution/read/cancel has drained and released the lane,
        // so release ONLY the matching admission gate before waiting for linked cleanup.
        // Otherwise child settle/Return refuses actionRpc while this driver waits on that
        // very child's drainage. End already excludes any new live history request.
        if (history && this.historyRequest === history) { this.historyRequest = null; this.wake(); }
        // The metadata lane is shared with cleanup. Join its real outstanding read/attempt
        // OUTSIDE execution: original handles may still need the same physical lane.
        if (this.lifecycleCompletion) await this.lifecycleCompletion;
        if (pairState.workPump) await pairState.workPump.close();
        pairState.linked?.handle.close();
        // Release the physical lane BEFORE waiting for linked SDK/settlement. A provider
        // may already be queued behind our aborting read and still need this SAME position.
        // Framing keys and the selected-key/control payment remain until cleanup is last.
        if (pairState.intake) await pairState.intake.drained.promise;
        if (pairState.linked) await pairState.linked.drained.promise;
        // Ordinary Return comprises several serialized RPCs. Acquiring the lane between
        // them could clear keys/unlink/refund history while its next RPC is still queued.
        // No new public Return is admitted after retirement; drain the existing owner
        // OUTSIDE execution.run, including its rejection/cancellation finally.
        if (pairState.derivationReturn) {
          try { await pairState.derivationReturn; } catch { /* exact final cleanup resolves uncertainty */ }
        }
        if (pairState.transitionFlight) await pairState.transitionFlight;
        // Independent of old96KiB drainage. Join the actual SDK/IO admission finally
        // OUTSIDE the lane that its exact recovery/release calls require.
        if (pairState.rtcFlight) await pairState.rtcFlight;
        if (pairState.sdpFlight) await pairState.sdpFlight;
        if (pairState.negotiationFlight) await pairState.negotiationFlight;
        if (pairState.failedFlight) await pairState.failedFlight;
        // Media debt blocks native parent Final. Finish it BEFORE unseen-parent/old SDK
        // cleanup, and never own SharedIpcExecution while joining SDK or current work.
        if (pairState.compactFlight) await pairState.compactFlight;
        for (const peer of this.peers.values()) await this.cleanupPeer(peer);
        pairState.media = null;
        // A failed SDK close is not actual drainage. Keep the original paid peer and
        // retry cleanup; never release native SDK custody merely because End was seen.
        while (pairState.sdpRtc) {
          const observed = this.wakeIdentity;
          try { await pairState.sdpRtc.drain(); break; }
          catch { await this.waitCleanupWake(observed); }
        }
        // The original job's23/24 RPCs require this same physical lane. Join its SDK,
        // Result and close-first unknown Begin outside execution.run and before release.
        while (pairState.sdpAction && !pairState.sdpAction.returned) {
          const observed = this.wakeIdentity;
          try { await pairState.sdpAction.cleanup(); }
          catch { await this.waitCleanupWake(observed); }
        }
        pairState.sdpAction = null; pairState.sdpRtc = null;
        const sdkChain = pairState.linked, sdkPhase = sdkChain?.sdp;
        await this.cleanup();
        if (sdkChain && sdkPhase) this.custody.sdpUnlinked(sdkChain, sdkPhase);
      }
      if (this.terminal) outcome = this.terminal;
      this.request?.resolve(outcome); this.request = null;
      history?.result.resolve(historical);
      if (this.historyRequest === history) this.historyRequest = null;
      this.driving = false; this.endActivity(); this.collect();
      // close() includes the original lane's finally and independent SDK-subtree return.
      // Resolving from cleanup() itself exposed a microtask gap before those owners drained.
      if (this.drained) this.completion.resolve();
    }
  }
  private async setup(): Promise<boolean> {
    if (this.keys) return true;
    this.attempted = true;
    let reply: unknown;
    try { reply = await this.native.setup(this.selected.owner.token, this.selected.executor); }
    catch { return false; } // Lost reply: recover the original native identity, without crypto.
    if (!this.live()) return false;
    const identity = (value: Record<string, unknown>) => value.owner === this.selected.owner.token && value.executor === this.selected.executor;
    if (exactKeys(reply, ["owner", "executor", "kind"]) && identity(reply) && reply.kind === "pending") return false;
    if (exactKeys(reply, ["owner", "executor", "kind", "nativeSetupDrained"]) && identity(reply) &&
        reply.kind === "closed" && typeof reply.nativeSetupDrained === "boolean") {
      this.terminal = Object.freeze({ kind: "closed", nativeSetupDrained: reply.nativeSetupDrained });
      this.retired = true; return false;
    }
    if (!exactKeys(reply, ["owner", "executor", "kind", "port", "transcript"]) || !identity(reply) || reply.kind !== "ready" ||
        !Number.isInteger(reply.port) || (reply.port as number) < 1 || (reply.port as number) > 65535 || typeof reply.transcript !== "string") {
      throw new Error("voice channel setup refused");
    }
    this.bootstrap = parseVoiceIpcBootstrap(reply.transcript, {
      owner: this.selected.owner.token, executor: this.selected.executor,
      join: this.selected.key.join!, group: this.selected.owner.group, point: this.selected.key.point,
    });
    this.port = reply.port as number;
    // Never wait for an SDK position while holding the global codec: another Session's
    // Current/cleanup must progress even if bootstrap's ECDH/HKDF never returns.
    const release = await this.reserveEngine();
    if (!release) return false;
    try {
      this.check();
      this.construction ??= this.derive(this.bootstrap);
      this.keys = await this.construction;
    } finally { release(); }
    this.check(); return true;
  }
  private async step<T>(promise: Promise<T>): Promise<T> {
    const value = await promise; this.check(); return value;
  }
  private async derive(bootstrap: VoiceIpcBootstrap): Promise<IpcKeys> {
    let shared: Uint8Array<ArrayBuffer> | null = null, seed: Uint8Array<ArrayBuffer> | null = null;
    try {
      this.check();
      this.hash = new Uint8Array(await this.step(this.subtle.digest("SHA-256", bootstrap.transcript)));
      const peer = await this.step(this.subtle.importKey("raw", bootstrap.nativePoint, { name: "ECDH", namedCurve: "P-256" }, false, []));
      // Capture the original nonextractable private key. Its SelectedKeyRecord stays paid and
      // borrowed until every SDK promise below (including rejected/retired work) has settled.
      const privateKey = this.selected.key.privateKey;
      if (!privateKey) throw new Error("selected voice key closed");
      shared = new Uint8Array(await this.subtle.deriveBits({ name: "ECDH", public: peer }, privateKey, 256));
      this.check();
      if (shared.length !== 32) throw new Error("voice ECDH result refused");
      const base = await this.step(this.subtle.importKey("raw", shared, "HKDF", false, ["deriveBits"]));
      shared.fill(0); shared = null;
      seed = new Uint8Array(await this.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: this.hash,
        info: encoder.encode("Mewtual native voice bootstrap seed v1") }, base, 256));
      this.check();
      if (seed.length !== 32) throw new Error("voice bootstrap seed refused");
      const material = await this.step(this.subtle.importKey("raw", seed, "HKDF", false, ["deriveKey"]));
      seed.fill(0); seed = null;
      const parameters = (label: string) => ({ name: "HKDF", hash: "SHA-256", salt: encoder.encode("Mewtual native voice handoff v1"), info: encoder.encode(label) });
      const requestMac = await this.step(this.subtle.deriveKey(parameters("v1 c2n header HMAC-SHA256"), material, { name: "HMAC", hash: "SHA-256", length: 256 }, false, ["sign"]));
      const requestBody = await this.step(this.subtle.deriveKey(parameters("v1 c2n body AES-256-GCM"), material, { name: "AES-GCM", length: 256 }, false, ["encrypt"]));
      const responseMac = await this.step(this.subtle.deriveKey(parameters("v1 n2c header HMAC-SHA256"), material, { name: "HMAC", hash: "SHA-256", length: 256 }, false, ["verify"]));
      const responseBody = await this.step(this.subtle.deriveKey(parameters("v1 n2c body AES-256-GCM"), material, { name: "AES-GCM", length: 256 }, false, ["decrypt"]));
      return Object.freeze({ requestMac, requestBody, responseMac, responseBody });
    } finally { shared?.fill(0); seed?.fill(0); }
  }
  private exchange(query: null): Promise<VoiceIpcStatus>;
  private exchange(query: IpcHistoryRequest): Promise<VoiceIpcActionReply>;
  private exchange(query: IpcHistoryRequest | null): Promise<VoiceIpcStatus | VoiceIpcActionReply> {
    const bootstrap = this.bootstrap!;
    const body = query ? query.body : new Uint8Array(49);
    if (!query) { body[0] = 1; body.set(ipcBytes(bootstrap.session), 1); body.set(this.hash!, 17); }
    return this.framed(query ? 8 : 9, 0, query ? "0".repeat(32) : bootstrap.session, body, false,
      (kind, length) => kind === 0 && (length === 19 || (!!query && length === 258)),
      opened => {
        if (query) return decodeVoiceIpcActionReply(opened, query.expectedRow);
        if (opened.length !== 3 || opened[0] !== 1 || (opened[1] !== 1 && opened[1] !== 2) || opened[2] > 1) throw new Error("voice status refused");
        this.custody.setupDrained();
        const outcome = Object.freeze({ kind: opened[1] === 1 ? "ready" as const : "closed" as const, nativeSetupDrained: opened[2] === 1 });
        if (outcome.kind === "ready") this.readyAcknowledged = true;
        if (outcome.kind === "closed") { this.terminal = outcome; this.retired = true; }
        return outcome;
      });
  }
  /** One original physical flight. The caller prepays Data intake before entering here and
   * retains that same claim through decoding/SDK/read cancellation. Cleanup may use only the
   * original keys/counters after End; no constructor or fresh-work owner check is weakened. */
  private framed<T>(operation: VoiceIpcHeader["operation"], kind: VoiceIpcHeader["kind"],
    logical: string, plaintext: Uint8Array<ArrayBuffer>, afterEnd: boolean,
    allowed: (kind: VoiceIpcHeader["kind"], length: number) => boolean,
    decode: (opened: Uint8Array<ArrayBuffer>, kind: VoiceIpcHeader["kind"]) => T, input?: undefined, inputIntake?: ChainIntake): Promise<T>;
  private framed<T>(operation: VoiceIpcHeader["operation"], kind: VoiceIpcHeader["kind"],
    logical: string, plaintext: Uint8Array<ArrayBuffer>, afterEnd: boolean,
    allowed: (kind: VoiceIpcHeader["kind"], length: number) => boolean,
    decode: (opened: Uint8Array<ArrayBuffer>, kind: VoiceIpcHeader["kind"]) => T,
    input: Readonly<{ loan: VoiceSdpInputLoan; expected: VoiceSdpInputExpectation; current(): void }>): Promise<T | null>;
  private async framed<T>(operation: VoiceIpcHeader["operation"], kind: VoiceIpcHeader["kind"],
    logical: string, plaintext: Uint8Array<ArrayBuffer>, afterEnd: boolean,
    allowed: (kind: VoiceIpcHeader["kind"], length: number) => boolean,
    decode: (opened: Uint8Array<ArrayBuffer>, kind: VoiceIpcHeader["kind"]) => T,
    input?: Readonly<{ loan: VoiceSdpInputLoan; expected: VoiceSdpInputExpectation; current(): void }>, inputIntake?: ChainIntake): Promise<T | null> {
    const keys = this.keys, bootstrap = this.bootstrap;
    const check = () => {
      if (!keys || !bootstrap || this.keys !== keys || this.bootstrap !== bootstrap || this.drained) throw new Error("original voice channel unavailable");
      if (!afterEnd) this.check();
      // Repeat after the owner callback and every physical await. Cancellation while
      // queued must not submit a fresh Input when this same codec turn eventually starts.
      if (inputIntake && (inputIntake.retired || inputIntake.released || inputIntake.transferred ||
          this.negotiation.intake !== inputIntake || this.negotiation.generation !== inputIntake.generation)) {
        throw new Error("original prepaid intake retired");
      }
    };
    const step = async <V>(promise: Promise<V>): Promise<V> => { const value = await promise; check(); return value; };
    check();
    if (input && (operation !== 17 || kind !== 0 || plaintext.byteLength !== 66)) throw new Error("original SDP Input request required");
    if (this.tx === 0xffff_ffff_ffff_ffffn) { this.retired = true; throw new Error("voice channel exhausted"); }
    // Debit before every SDK/fetch callback. No local refusal/lost reply refunds this TX.
    const request: VoiceIpcHeader = Object.freeze({ direction: 0, operation, kind,
      slot: bootstrap!.slot, sequence: ++this.tx, logical, length: plaintext.byteLength + 16 });
    const header = encodeVoiceIpcHeader(request);
    let sealed: Uint8Array<ArrayBuffer> | null = null, opened: Uint8Array<ArrayBuffer> | null = null;
    let body: Uint8Array<ArrayBuffer> | null = null, response: Response | null = null;
    let result: T | null = null, transfer = false;
    const controller = new AbortController(); this.controller = controller; this.controllerIntake = inputIntake ?? null;
    let timer: unknown = null;
    try { try {
      sealed = new Uint8Array(await this.subtle.encrypt({ name: "AES-GCM", iv: voiceIpcNonce(request), additionalData: header }, keys!.requestBody, plaintext));
      check(); plaintext.fill(0);
      const mac = new Uint8Array(await step(this.subtle.sign("HMAC", keys!.requestMac, header)));
      const admission = encodeVoiceIpcAdmission(request, mac); mac.fill(0);
      const deadline = this.clock.now() + 5_000;
      const expire = () => {
        const remaining = deadline - this.clock.now();
        if (remaining <= 0) controller.abort();
        else timer = this.clock.schedule(expire, remaining);
      };
      timer = this.clock.schedule(expire, 5_000);
      check(); // Clock callbacks may retire the intake; AbortSignal alone is not admission.
      response = await this.transport("http://127.0.0.1:" + this.port + "/voice/v2", {
        method: "POST", mode: "cors", credentials: "omit", redirect: "error", cache: "no-store",
        referrerPolicy: "no-referrer", keepalive: false, signal: controller.signal,
        headers: { "content-type": "application/octet-stream", "x-voice-admission": admission }, body: sealed,
      });
      check();
      let incoming: ReturnType<typeof decodeVoiceIpcAdmission> | null = null;
      const authenticate = async (admission: string) => {
        const decoded = decodeVoiceIpcAdmission(admission);
        if (decoded.header.operation !== request.operation || !allowed(decoded.header.kind, decoded.header.length)) throw new Error("voice reply refused");
        const aad = voiceIpcResponseAad(decoded.header, request);
        if (!await step(this.subtle.verify("HMAC", keys!.responseMac, decoded.tag, aad))) throw new Error("voice response refused");
        if (decoded.header.sequence <= this.rx) throw new Error("voice response replayed");
        this.rx = decoded.header.sequence; incoming = decoded;
        return decoded.header;
      };
      body = input ? await readVoiceIpcSdpInputResponse(response, controller.signal, authenticate)
        : await readVoiceIpcResponse(response, controller.signal, async admission => (await authenticate(admission)).length);
      check();
      const received = incoming as ReturnType<typeof decodeVoiceIpcAdmission> | null;
      if (!received) throw new Error("voice response refused");
      opened = new Uint8Array(await this.subtle.decrypt({ name: "AES-GCM", iv: voiceIpcNonce(received.header), additionalData: voiceIpcResponseAad(received.header, request) }, keys!.responseBody, body));
      check();
      if (input && received.header.kind === 2) transfer = true;
      else result = decode(opened, received.header.kind);
    } finally {
      try { if (response?.body && !response.body.locked) await response.body.cancel(); }
      catch { /* Keep the original error; actual cancellation has settled. */ }
      finally {
        if (timer !== null) this.clock.cancel(timer);
        plaintext.fill(0); sealed?.fill(0); body?.fill(0);
        if (this.controller === controller) { this.controller = null; this.controllerIntake = null; }
      }
    }
      if (transfer) {
        check(); input!.current(); // AFTER actual read/cancel/timer finally.
        input!.loan.accept(opened!, input!.expected);
        opened = null; // Exactly one clearer now belongs to the private crypto owner.
      }
      return result;
    } finally { opened?.fill(0); }
  }
  private async cleanup(): Promise<void> {
    const pairState = this.negotiation;
    // setup()/exchange() have returned: every real SDK promise has settled. The close-first
    // IPC path also works when setup replies were lost or no usable channel keys ever existed.
    // A never-started browser channel cannot have created native work. After End its unknown
    // owner correctly refuses native settlement, so do not strand local capacity retrying it.
    // attempted is latched immediately before the actual setup RPC; any ambiguous handoff
    // still requires the exact close-first fence, even when no reply/keys were received.
    while (this.attempted) {
      const observed = this.wakeIdentity;
      try {
        // Close-first native admission irrevocably fences maintenance even when it refuses
        // Capacity for unseen rows. Control routing/keys remain until actual final closure.
        let firstRefused = false;
        if (!this.cleanupStarted) {
          this.cleanupStarted = true;
          try { await this.native.settleSetup(this.selected.owner.token, this.selected.executor); break; }
          catch { firstRefused = true; }
        }
        const discovered = this.keys && this.bootstrap ? await this.cleanupUnseenWork() : false;
        await this.cleanupConfirmationIntents(true, pairState);
        if (pairState.linked?.custody) {
          const chain = pairState.linked, custody = chain.custody!;
          if (custody.needsClaim) await this.transitionRpc(chain, { kind: "claim" }, true, pairState);
          if (custody.needsKeyRecovery) await this.keySessionRpc(chain, { kind: "adopt" }, true, pairState);
          await this.cleanupSdp(chain, true, pairState);
          if (custody.needsKeyRelease) await this.keySessionRpc(chain, { kind: "release", receipt: custody.keyReceipt!.receipt }, true, pairState);
          if (custody.needsRelease) await this.transitionRpc(chain, { kind: "release", receipt: custody.receipt!.receipt }, true, pairState);
        }
        if (pairState.terminalAction && !pairState.finalAcknowledged) await this.retireOriginalAction(true, pairState);
        // With no actual cleanup work, a lost close reply waits for the existing cadence
        // rather than immediately issuing a second close in the same driver turn.
        if (firstRefused && !discovered && !pairState.terminalAction) throw new Error("native close unresolved");
        await this.native.settleSetup(this.selected.owner.token, this.selected.executor); break;
      }
      catch {
        this.request?.resolve(Object.freeze({ kind: "uncertain" })); this.request = null;
        await this.waitCleanupWake(observed);
      }
    }
    this.keys = null; this.construction = null;
    this.hash?.fill(0); this.hash = null;
    this.bootstrap?.transcript.fill(0); this.bootstrap?.nativePoint.fill(0); this.bootstrap = null;
    this.port = 0;
    while (this.attempted) {
      const observed = this.wakeIdentity;
      try { await this.native.collectChannel(this.selected.owner.token, this.selected.executor); break; }
      catch {
        this.request?.resolve(Object.freeze({ kind: "uncertain" })); this.request = null;
        await this.waitCleanupWake(observed);
      }
    }
    this.drained = true;
    const linked = pairState.linked;
    pairState.linked = null;
    pairState.terminalAction = null;
    if (linked) this.custody.unlinked(linked);
    this.custody.setupDrained(); this.custody.closed();
  }

  /** Runs after all locally known SDK/IO has drained. Each complete physical RPC
   * acquires its own global codec turn; native waits/retry cadence never hold that lane.
   * Children close before parent finalization. A producer admitted before the close-first
   * fence may still publish: retry/rescan until native's independent final gate accepts.
   * Empty metadata never establishes absence; no Input/admission/Begin(true) is used here. */
  private async cleanupUnseenWork(): Promise<boolean> {
    const pairState = this.negotiation;
    if (pairState.cleanupWork?.work.purpose.kind === "derive") {
      pairState.cleanupParent = pairState.cleanupWork; pairState.cleanupWork = null;
    }
    let discovered = pairState.cleanupWork !== null || pairState.cleanupParent !== null;
    let completedParent: string | null = null;
    if (pairState.cleanupWork) await this.cleanupDiscoveredWork();
    for (const children of [true, false]) {
      if (!children && pairState.cleanupParent) {
        completedParent = pairState.cleanupParent.work.row;
        pairState.cleanupWork = pairState.cleanupParent; pairState.cleanupParent = null;
        await this.cleanupDiscoveredWork();
      }
      let fromSlot = 0;
      while (fromSlot < 32) {
        const snapshot = decodeVoiceWorkSnapshot(await this.native.readWork(
          this.selected.owner.token, this.selected.executor, fromSlot),
          this.selected.owner.token, this.selected.executor, fromSlot);
        fromSlot = snapshot.nextSlot;
        const work = snapshot.work;
        if (!work || (work.purpose.kind === "confirmation") !== children ||
            work.row === pairState.terminalAction?.row || (!children && work.row === completedParent)) continue;
        pairState.cleanupWork = { work, proof: null };
        discovered = true;
        await this.cleanupDiscoveredWork();
      }
    }
    return discovered;
  }

  private async cleanupDiscoveredWork(): Promise<void> {
    const pairState = this.negotiation;
    const intent = pairState.cleanupWork!;
    const { work } = intent;
    const p = work.purpose;
    const parent = p.kind === "derive" ? p : p.parentPurpose;
    const purpose: VoiceIpcPurpose = { peer: parent.peer, join: parent.join, keyRef: parent.keyRef, epoch: parent.epoch };
    const child: VoiceIpcConfirmationPurpose | null = p.kind === "derive" ? null : {
      parent: purpose, parentRow: p.parent,
      request: p.request === "start" ? { kind: "start", predecessor: p.predecessor }
        : p.request === "respond" ? { kind: "respond", triggerRow: p.trigger } : { kind: "incoming", role: p.role },
    };
    const rpc = async (request: VoiceIpcConfirmationRequest) => {
      const encoded = child ? encodeVoiceIpcConfirmationAction(child, request) : encodeVoiceIpcAction(purpose, request);
      try { return await this.execution.run(() => this.framed(encoded.operation, encoded.kind, request.kind === "begin" ? "0".repeat(32) : work.row,
        encoded.body, true, (kind, length) => kind === 0 && (length === 19 || length === 258),
        opened => decodeVoiceIpcActionReply(opened, work.row))); }
      finally { encoded.body.fill(0); }
    };
    const history = await rpc({ kind: "begin", retry: false });
    if (history.kind !== "status") throw new Error("native unseen work history unresolved");
    if (history.pending !== "not-pending") {
      const closed = await rpc({ kind: "close", mode: "return" });
      if (closed.kind !== "status" || (closed.pending !== "closed-handed" && closed.pending !== "closed-unhanded")) {
        throw new Error("native unseen Pending closure unresolved");
      }
    } else {
      const identity = history.identity!;
      if (work.identity && (identity.action !== work.identity.action || identity.version !== work.identity.version ||
          identity.inputTag !== work.identity.inputTag)) throw new Error("native unseen work identity changed");
      // A prior exact final intent can make another Return refuse. Its authenticated
      // localDrained status already covers the immutable ordinary closure.
      if (!history.localDrained) await this.returnAction(work.row, identity, rpc);
      if (!child) {
        const exchange = async (request: Extract<VoiceIpcActionRequest, { kind: "close-plan" | "final" }>) => {
          const encoded = encodeVoiceIpcAction(purpose, request);
          try { return await this.execution.run(() => this.framed(encoded.operation, encoded.kind, work.row, encoded.body, true,
            (kind, length) => kind === 0 && (length === 19 || length === 116),
            opened => decodeVoiceIpcFinalReply(opened, work.row, identity,
              request.kind === "final" ? intent.proof : null, "retire-plan"))); }
          finally { encoded.body.fill(0); }
        };
        let acknowledged = false;
        if (intent.proof === null) {
          const final = await exchange({ kind: "close-plan" });
          if (final.kind !== "final" || final.stage === "pending") throw new Error("native unseen plan retirement unresolved");
          intent.proof = final.proof; acknowledged = final.stage === "acknowledged";
        }
        if (!acknowledged) {
          const ack = await exchange({ kind: "final", proof: intent.proof! });
          if (ack.kind !== "final" || ack.stage !== "acknowledged") throw new Error("native unseen plan ACK unresolved");
        }
      }
    }
    pairState.cleanupWork = null;
  }

  /** After actual local SDK/I/O drained. End and live retirement both schedule each
   * complete physical RPC separately so another Session can refresh during retries.
   * History-only Begin(false) advances the SAME Session request floor, fencing a delayed
   * old Begin. Missing/refused history stays unresolved: parent Plan-final must still check
   * every actual child and freeze new adoption before its exact final ACK covers absence.
   * Never recursively acquire the lane-owned default or wait here for parent final proof. */
  private async cleanupConfirmationIntents(separate = false, pairState: RetainedNativeNegotiation): Promise<void> {
    const chain = pairState.linked;
    if (!chain || pairState.finalAcknowledged) return;
    for (const intake of chain.confirmations.values()) {
      if (intake.returned) continue;
      if (intake.running || (intake.record && !intake.record.drained)) throw new Error("actual child work still running");
      const rpc = async (request: VoiceIpcConfirmationRequest, row: string): Promise<VoiceIpcActionReply> => {
        const physical = async () => {
          const encoded = encodeVoiceIpcConfirmationAction(intake.purpose, request);
          try { return await this.framed(encoded.operation, encoded.kind, row, encoded.body, true,
            (kind, length) => kind === 0 && (length === 19 || length === 258),
            opened => decodeVoiceIpcActionReply(opened, intake.row)); }
          finally { encoded.body.fill(0); }
        };
        return separate ? this.execution.run(physical) : physical();
      };
      try {
        if (!intake.identity) this.observeConfirmationBegin(intake, await rpc({ kind: "begin", retry: false }, "0".repeat(32)));
        if (!intake.row) continue; // not proof of absence; still require actual parent-final
        if (intake.identity) {
          await this.returnAction(intake.row, intake.identity, request => rpc(request, intake.row!));
        } else {
          // Pending has no engine identity. Its exact Return is the only child fence here.
          const reply = await rpc({ kind: "close", mode: "return" }, intake.row);
          if (reply.kind !== "status" || (reply.pending !== "closed-handed" && reply.pending !== "closed-unhanded")) continue;
        }
        intake.returned = true;
      } catch {
        // Lost/refused history/Return keeps this SAME fixed intent and row claim. Native
        // Plan-final either proves all children closed or refuses for the next bounded tick.
      }
    }
  }

  /** After intake/SDK/result settlement drainage. End and live retirement give each
   * physical RPC a separate turn so another peer/Session's Current can interleave.
   * Preserve original keys/counters and never recursively acquire the lane-owned default. */
  private async retireOriginalAction(separate = false, pairState: RetainedNativeNegotiation, fenceOnly = false): Promise<void> {
    if (fenceOnly && pairState.finalMode !== "retire-plan") throw new Error("original Plan closing fence required");
    const original = pairState.terminalAction!;
    const exchange = async <T>(request: VoiceIpcActionRequest, length: number,
      decode: (opened: Uint8Array<ArrayBuffer>) => T): Promise<T> => {
      const physical = async () => {
        const encoded = encodeVoiceIpcAction(original.purpose, request);
        try { return await this.framed(encoded.operation, encoded.kind, original.row, encoded.body, true,
          (kind, actual) => kind === 0 && (actual === 19 || actual === length + 16), decode); }
        finally { encoded.body.fill(0); }
      };
      return separate ? this.execution.run(physical) : physical();
    };
    const status = (opened: Uint8Array<ArrayBuffer>) => {
      const reply = decodeVoiceIpcActionReply(opened, original.row);
      if (reply.kind !== "status" || !reply.identity || reply.identity.action !== original.identity.action ||
          reply.identity.version !== original.identity.version || reply.identity.inputTag !== original.identity.inputTag || !reply.settled) {
        throw new Error("native action cleanup unresolved");
      }
      return reply;
    };
    let ordinaryDrained: boolean;
    {
      // Do not retain the decoded action/receipt graph while the separate final proof is
      // read. Their payload owners occupy the same prepaid serial Control envelope.
      const settled = await exchange({ kind: "settle", identity: original.identity }, 242, status);
      if (settled.receipt && !settled.acknowledged) {
        const reply = await exchange({ kind: "ack", identity: original.identity, digest: settled.receipt.digest }, 242, status);
        if (!reply.acknowledged || reply.receipt?.digest !== settled.receipt.digest) throw new Error("native receipt cleanup unresolved");
      }
      ordinaryDrained = settled.localDrained;
    }
    if (pairState.finalProof === null) {
      pairState.finalIntentStarted = true; // BEFORE transport; an ambiguous intent is immutable.
      let reply = await exchange(pairState.finalMode === "retire" ? { kind: "close", mode: "retire" } : { kind: "close-plan" }, 100,
        opened => decodeVoiceIpcFinalReply(opened, original.row, original.identity, null, pairState.finalMode));
      if (reply.kind === "refused" && reply.code === "unauthorized" && ordinaryDrained && pairState.finalMode === "retire") {
        // Only an authenticated definite refusal permits this separate intent. A lost or
        // Pending Phase reply may already own mode1 and MUST retry it. Native mode2 checks
        // the actual returned original parent and retires its SAME plan; it cannot relabel
        // Return or turn this observation into drainage. Latch before the first mode2 send.
        pairState.finalMode = "retire-plan";
        reply = await exchange({ kind: "close-plan" }, 100,
          opened => decodeVoiceIpcFinalReply(opened, original.row, original.identity, null, pairState.finalMode));
      }
      if (reply.kind === "refused") {
        throw new Error("native terminal cleanup unresolved");
      }
      if (reply.stage === "pending") {
        // This return is an authenticated negative fence observation AFTER complete IO
        // and body-clearing finally. It is never final/ACK, drainage or advance evidence.
        if (fenceOnly) return;
        throw new Error("native terminal cleanup pending");
      }
      pairState.finalProof = reply.proof;
      pairState.finalAcknowledged = reply.stage === "acknowledged";
    }
    if (!pairState.finalAcknowledged) {
      const proof = pairState.finalProof!;
      const reply = await exchange({ kind: "final", proof }, 100,
        opened => decodeVoiceIpcFinalReply(opened, original.row, original.identity, proof, pairState.finalMode));
      if (reply.kind !== "final" || reply.stage !== "acknowledged") throw new Error("native final acknowledgement unresolved");
      pairState.finalAcknowledged = true;
    }
  }
}

// Keep the historical slot name so a retained older broker is detected and refused, rather than
// creating a second pool while its old engine/native-commit promises still own capacity.
const BROKER = Symbol.for("mewtual.voice.crypto-broker.v2");

/** Keep the broker across component/HMR remounts. A version mismatch refuses rather than
 * creating a second pool while the previous pool may still own unresolved engine operations. */
export function windowVoiceCryptoBroker(
  native: VoiceCryptoAdmission, window: object = globalThis, subtle: SubtleCrypto = globalThis.crypto.subtle,
  expected?: VoiceCryptoBroker,
): VoiceCryptoBroker {
  const host = window as Record<symbol, unknown>;
  const previous = host[BROKER] as VoiceCryptoBroker | undefined;
  // A retained outer owner must validate lookup BEFORE any constructing path. Missing,
  // null or swapped slots cannot publish a second pool beside its unresolved original.
  if (expected !== undefined && previous !== expected) throw new Error("voice crypto broker changed; reload required");
  if (previous) {
    if (previous.version !== 46 || typeof previous.nativeConfiguredAppCalls !== "function" || typeof previous.nativeAppCalls !== "function" || typeof previous.retireNativeOwner !== "function" || typeof previous.retireFailedNativeNegotiation !== "function" || typeof previous.observeNativeLifecycle !== "function" || typeof previous.advanceNativeNegotiation !== "function" || typeof previous.compactNativeNegotiation !== "function" || typeof previous.collectNativeNegotiationHistory !== "function" || typeof previous.retireNativeNegotiation !== "function" || typeof previous.completeNativeMediaHandoff !== "function" || typeof previous.completeNativeSdpRtcRound !== "function" || typeof previous.prepareNativeSdpRtc !== "function" || typeof previous.prepareNativeSdpCrypto !== "function" || typeof previous.prepareNativePcIntent !== "function" || typeof previous.prepareNativeSdpHandoff !== "function" || typeof previous.observeNativeWork !== "function" || typeof previous.returnNativeDerivation !== "function" || typeof previous.nativeConfirmationFromChannel !== "function" || typeof previous.nativeDerivationFromChannel !== "function" || typeof previous.recoverChannelHistory !== "function" || typeof previous.prepareChainFromChannel !== "function" || typeof previous.nativeChannel !== "function" || typeof previous.run !== "function" || typeof previous.selectJoinKey !== "function" || typeof previous.retrySelectedJoinKeys !== "function" ||
        typeof previous.registerProtectedConfirmation !== "function" ||
        typeof previous.registerProtectedDerivation !== "function" || typeof previous.registerConfirmation !== "function" || typeof previous.createPair !== "function") {
      throw new Error("voice crypto broker version changed; reload required");
    }
    previous.retrySelectedJoinKeys();
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
  #tx: CryptoKey | null;
  #rx: CryptoKey | null;
  #broker: VoiceCryptoBroker;
  #owner: VoiceCryptoOwner;
  #domain: string;
  #isLive: () => boolean;
  #protected: boolean;

  constructor(broker: VoiceCryptoBroker, owner: VoiceCryptoOwner, tx: CryptoKey, rx: CryptoKey, domain: string, isLive: () => boolean = () => true, protectedPair = false) {
    this.#broker = broker; this.#owner = owner; this.#tx = tx; this.#rx = rx;
    this.#domain = domain;
    this.#isLive = isLive;
    this.#protected = protectedPair;
  }
  private assertCurrent(): void {
    current(this.#owner);
    if (!this.#isLive() || !this.#tx || !this.#rx) { this.retire(); throw new Error("voice pair retired"); }
  }
  /** Clears this wrapper's key references. Actual engine calls keep their already-captured
   * CryptoKey and paid execution custody until their own promise settles. */
  retire(): void { this.#tx = null; this.#rx = null; this.#isLive = () => false; }
  isProtected(): boolean { return this.#protected; }
  private assertOrdinary(): void {
    this.assertCurrent();
    if (this.#protected) throw new Error("protected native crypto action required");
  }

  belongsTo(broker: VoiceCryptoBroker, owner: VoiceCryptoOwner): boolean { return this.#broker === broker && this.#owner === owner; }

  /** Internal engine seam. Only this module's broker can supply the private computation mark;
   * native admission and action/result custody have already been reserved before reaching it. */
  async computeConfirmation(mark: symbol, input: VoiceConfirmationInput): Promise<Uint8Array<ArrayBuffer>> {
    if (mark !== CONFIRM_COMPUTE) throw new Error("native confirmation computation required");
    this.assertOrdinary();
    const opened = new Uint8Array(await this.#broker.subtle.decrypt({
      name: "AES-GCM", iv: voicePacketIv(u64(input.sequence, "sequence")),
      additionalData: input.aad as Uint8Array<ArrayBuffer>, tagLength: 128,
    }, this.#rx!, input.ciphertext as Uint8Array<ArrayBuffer>));
    try { this.assertOrdinary(); return opened; } catch (error) { opened.fill(0); throw error; }
  }

  /** Only a registered protected chain job can reach this private-mark seam. Ordinary APIs
   * continue to reject these keys. Native owns the immutable row/counter/AAD/body; this window
   * additionally burns encryption counters before entering WebCrypto, even if it rejects. */
  async computeProtectedConfirmation(mark: symbol, input: VoiceProtectedConfirmationInput): Promise<Uint8Array<ArrayBuffer>> {
    if (mark !== CONFIRM_COMPUTE || !this.#protected) throw new Error("protected native computation required");
    this.assertCurrent();
    const sequence = u64(input.sequence, "sequence");
    const algorithm = { name: "AES-GCM", iv: voicePacketIv(sequence), additionalData: input.aad as Uint8Array<ArrayBuffer>, tagLength: 128 };
    let result: Uint8Array<ArrayBuffer>;
    if (input.operation === "seal") {
      this.#broker.burnSequence(this.#owner, this.#domain, sequence);
      result = new Uint8Array(await this.#broker.subtle.encrypt(algorithm, this.#tx!, input.body as Uint8Array<ArrayBuffer>));
    } else result = new Uint8Array(await this.#broker.subtle.decrypt(algorithm, this.#rx!, input.body as Uint8Array<ArrayBuffer>));
    try { this.assertCurrent(); return result; } catch (error) { result.fill(0); throw error; }
  }

  /** Original SDP-only SDK seam. The retained native row owner latches one execution
   * before entry; old confirmation APIs never admit larger input or these operations.
   * Only actual AEAD OperationError is the closed authentication-failed result. End,
   * provider/programming errors and bad lengths must settle without invented plaintext. */
  async computeSdp(mark: symbol, input: VoiceSdpInput, isCurrent: () => boolean): Promise<Uint8Array<ArrayBuffer> | null> {
    if (mark !== SDP_COMPUTE || !this.#protected) throw new Error("original native SDP computation required");
    this.assertCurrent();
    const sequence = u64(input.sequence, "sequence"), seal = input.purpose === "seal";
    const algorithm = { name: "AES-GCM", iv: voicePacketIv(sequence), additionalData: input.aad, tagLength: 128 };
    let result: Uint8Array<ArrayBuffer>;
    try {
      if (seal) {
        this.#broker.burnSequence(this.#owner, this.#domain, sequence);
        if (!isCurrent()) throw new Error("voice SDP engine retired");
        result = new Uint8Array(await this.#broker.subtle.encrypt(algorithm, this.#tx!, input.body));
      } else {
        if (!isCurrent()) throw new Error("voice SDP engine retired");
        result = new Uint8Array(await this.#broker.subtle.decrypt(algorithm, this.#rx!, input.body));
      }
    } catch (error) {
      this.assertCurrent();
      if (!seal && error instanceof DOMException && error.name === "OperationError") return null;
      throw error;
    }
    try { this.assertCurrent(); return result; } catch (error) { result.fill(0); throw error; }
  }

  encrypt(aad: Uint8Array, sequence: bigint, plaintext: Uint8Array): Promise<Uint8Array<ArrayBuffer>> {
    this.assertOrdinary();
    const header = boundedBytes(aad, MAX_CONTEXT, "header"), body = boundedBytes(plaintext, MAX_PLAINTEXT, "plaintext");
    const iv = voicePacketIv(sequence);
    return this.#broker.run(this.#owner, "encrypt", header.length + body.length, async () => {
      this.assertCurrent();
      this.#broker.burnSequence(this.#owner, this.#domain, sequence);
      const result = new Uint8Array(await this.#broker.subtle.encrypt({ name: "AES-GCM", iv, additionalData: header, tagLength: 128 }, this.#tx!, body));
      this.assertCurrent(); return result;
    }).then(result => { this.assertOrdinary(); return result; });
  }

  async open(aad: Uint8Array, sequence: bigint, ciphertext: Uint8Array, deliveryToken: string): Promise<Uint8Array<ArrayBuffer>> {
    this.assertOrdinary();
    const header = boundedBytes(aad, MAX_CONTEXT, "header"), body = boundedBytes(ciphertext, MAX_PLAINTEXT + 16, "ciphertext");
    const iv = voicePacketIv(sequence);
    // Keep the bounded window/native charge through final authorization as well as engine work.
    // Releasing it before this await would allow unbounded decrypted results awaiting native.
    const owned: { plaintext?: Uint8Array<ArrayBuffer> } = {};
    try {
      const opened = await this.#broker.run(this.#owner, "decrypt", header.length + body.length, async () => {
        this.assertCurrent();
        const opened = new Uint8Array(await this.#broker.subtle.decrypt({ name: "AES-GCM", iv, additionalData: header, tagLength: 128 }, this.#rx!, body));
        owned.plaintext = opened;
        this.assertCurrent();
        await this.#broker.commitInbound(this.#owner, deliveryToken, { aad: header, sequence, ciphertext: body }, opened);
        this.assertCurrent();
        return opened;
      });
      this.assertOrdinary(); return opened;
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
  const point = boundedBytes(peerPublicKey, 65, "public key"), transcript = boundedBytes(nativeTranscript, MAX_DERIVATION_CONTEXT, "transcript");
  if (point.length !== 65 || point[0] !== 4 || privateKey.type !== "private" || privateKey.extractable ||
      privateKey.algorithm.name !== "ECDH" || (privateKey.algorithm as EcKeyAlgorithm).namedCurve !== "P-256") {
    throw new Error("invalid voice join key");
  }
  return broker.run(owner, "derive", point.length + transcript.length + 128,
    () => computeVoicePair(broker, owner, privateKey, point, transcript, localIsLow));
}

/** Closed computation shared by ordinary fixtures and protected native action row0. No caller
 * callback is executed under the protected position; every await repeats its original lease. */
async function computeVoicePair(
  broker: VoiceCryptoBroker, owner: VoiceCryptoOwner, privateKey: CryptoKey,
  point: Uint8Array, transcript: Uint8Array, localIsLow: boolean, isLive: () => boolean = () => true,
  protectedPair = false,
): Promise<VoiceCryptoPair> {
    const assertCurrent = () => { current(owner); if (!isLive()) throw new Error("voice derivation retired"); };
    assertCurrent();
    const publicKey = await broker.subtle.importKey("raw", point as Uint8Array<ArrayBuffer>, { name: "ECDH", namedCurve: "P-256" }, false, []);
    assertCurrent();
    const secret = new Uint8Array(await broker.subtle.deriveBits({ name: "ECDH", public: publicKey }, privateKey, 256));
    let base: CryptoKey;
    try { assertCurrent(); base = await broker.subtle.importKey("raw", secret, "HKDF", false, ["deriveKey"]); }
    finally { secret.fill(0); }
    assertCurrent();
    const salt = new Uint8Array(await broker.subtle.digest("SHA-256", context("mewtual/voice/v2/transcript", transcript)));
    const direction = async (label: string, usage: KeyUsage) => {
      assertCurrent();
      return broker.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt, info: context(`mewtual/voice/v2/aes-gcm/${label}`, transcript) }, base, { name: "AES-GCM", length: 256 }, false, [usage]);
    };
    const tx = await direction(localIsLow ? "low-to-high" : "high-to-low", "encrypt");
    const rx = await direction(localIsLow ? "high-to-low" : "low-to-high", "decrypt");
    assertCurrent();
    const domain = Array.from(salt, byte => byte.toString(16).padStart(2, "0")).join("") + (localIsLow ? "/low" : "/high");
    return broker.createPair(owner, tx, rx, domain, isLive, protectedPair);
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
