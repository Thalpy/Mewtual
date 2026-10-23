// voice-v2-call.ts: the V2 call driver (docs/communication-recovery/VOICE-V2-CALL-DRIVER.md).
//
// One instance owns one V2 call for one room and connects the native-authorized V2 signalling
// path to the browser's RTCPeerConnections. It is the only renderer code that may turn V2 SDP/ICE
// into RTC effects, and it never reads the legacy `call-signal` path: App.svelte excludes every
// legacy signal for a V2 room before its legacy handler runs (voice-signaling.ts).
//
// Authority stays native. This module receives only items the native path has already
// authenticated, freshness-checked and authorized for the exact current join, and it submits
// plaintext SDP/ICE to the native path, which reserves, encrypts, signs and routes it. The driver's
// own job is the renderer half of the contract:
// - fence every RTC continuation by call generation, exact peer record and exact OfferId, so a
//   delayed bye, stale ICE, a lost reply or a replaced join can never touch newer state;
// - keep signalling progress, native submission and media connectivity as separate facts;
// - never fall back to the legacy protocol, whatever fails.
//
// ACTIVATION IS INCOMPLETE. No native command implements VoiceV2Native yet: the handoff channel
// is status-only and native SDP phase factories do not exist. `voiceV2Activation()` therefore
// reports unavailable and App.svelte keeps every call on legacy signalling. The contract tests in
// voice-v2-call.test.ts pin the behaviour a native adapter must support before that changes.

import {
  isCallIdentity,
  isVoiceInitiatorTowards,
  isVoiceNonce,
  isVoiceRoomId,
  parseVoiceOfferId,
  sameVoiceOffer,
  voiceOfferKey,
  type VoiceOfferId,
} from "./call-identity.ts";

/** Native plaintext ceilings (VOICE-SIGNALLING-V2.md "Initial defensive ceilings"). */
export const VOICE_V2_MAX_SDP = 8 * 1024;
export const VOICE_V2_MAX_ICE = 2 * 1024;
/** At most 16 participants/pairs per local call; further fresh peers are refused, not evicted. */
export const VOICE_V2_MAX_PEERS = 16;
/** Early remote ICE kept for one offer before its description applies: 8 candidates or 16 KiB. */
export const VOICE_V2_EARLY_ICE = 8;
export const VOICE_V2_EARLY_ICE_BYTES = 16 * 1024;
/** Local ICE gathered before this end's SDP submission resolved. */
export const VOICE_V2_PENDING_LOCAL_ICE = 16;
/** Applied-receipt memory per call generation. The native cursor is the primary duplicate guard. */
export const VOICE_V2_RECEIPTS = 512;
/** Items accepted from one native pull; the native queue itself rotates at most a few per pull. */
export const VOICE_V2_BATCH = 64;
/** Backoff after an uncertain native reply (a rejected promise). The last delay then repeats. */
export const VOICE_V2_RETRY_MS: readonly number[] = [250, 1000, 4000];
/** Exact-retry attempts for one start/end, or one ICE submit, before giving up as uncertain. */
export const VOICE_V2_RETRIES = VOICE_V2_RETRY_MS.length;
/**
 * How long an offer/answer submit keeps retrying its exact action after lost replies: native
 * retains an exact completed result through the packet's 15 s signed expiry plus 5 s skew, so a
 * retry inside this window recovers the original OfferId instead of stranding the negotiation.
 */
export const VOICE_V2_RECEIPT_MS = 20_000;
/** Minimum pause after an empty pull, so a native adapter that answers at once cannot spin us. */
export const VOICE_V2_IDLE_MS = 250;
/** Consecutive uncertain pulls before the call reports "events-uncertain" (it keeps polling). */
export const VOICE_V2_PULL_UNCERTAIN = 8;
/** Automatic fresh joins after native epoch/policy replacement, within one rolling minute. */
export const VOICE_V2_REJOINS = 3;
export const VOICE_V2_REJOIN_WINDOW_MS = 60_000;
/** How long a fresh peer may wait for native key confirmation before we report it unavailable. */
export const VOICE_V2_PAIR_WAIT_MS = 20_000;

// ---------------------------------------------------------------------------------------------
// Native contract. Every promise REJECTION means "uncertain": the reply was lost or IPC failed,
// and the request may or may not have taken effect. The driver retries the exact same request
// (same attempt/action id and bytes). Every RESOLVED value is authoritative.

/** Start one fresh V2 join. `attempt` is the exact-retry identity of this start request. */
export type VoiceV2StartRequest = Readonly<{ server: number; room: string; attempt: string }>;

/** A native call lease. `lease` is opaque; `join` is the native nonce of this exact join. */
export type VoiceV2Lease = Readonly<{ lease: string; group: string; room: string; self: string; join: string }>;

export type VoiceV2StartResult =
  | Readonly<{ kind: "started"; lease: VoiceV2Lease }>
  | Readonly<{ kind: "refused"; reason: string }>;

/** An ICE candidate in the browser's own JSON form, bounded by VOICE_V2_MAX_ICE. */
export type VoiceV2Candidate = Readonly<{
  candidate: string;
  sdpMid: string | null;
  sdpMLineIndex: number | null;
  usernameFragment: string | null;
}>;

/**
 * Outbound work. `action` is the exact-retry identity: native must return the cached result for
 * a repeat of the same id with the same content, and refuse the same id with different content.
 */
export type VoiceV2Action =
  | Readonly<{ kind: "offer"; action: string; peer: string; peerJoin: string; sdp: string }>
  | Readonly<{ kind: "answer"; action: string; peer: string; peerJoin: string; offer: VoiceOfferId; sdp: string }>
  | Readonly<{ kind: "ice"; action: string; peer: string; peerJoin: string; offer: VoiceOfferId; candidate: VoiceV2Candidate | null }>;

/**
 * Native's authoritative answer to one action. "submitted" means native admitted, signed and
 * queued the packet for routing; it is NOT remote receipt, decryption or media. An offer's result
 * carries the OfferId native minted for it (issuer = this device).
 */
export type VoiceV2SubmitResult =
  | Readonly<{ status: "submitted"; action: string; offer: VoiceOfferId | null }>
  | Readonly<{ status: "refused"; action: string; reason: string }>;

/** One non-dequeueing pull: items after the caller's cursor, and the cursor after them. */
export type VoiceV2Batch = Readonly<{ revision: string; items: readonly unknown[] }>;

export interface VoiceV2Native {
  start(request: VoiceV2StartRequest): Promise<VoiceV2StartResult>;
  /** Long-poll or immediate: may resolve with no items. Repeating `after` repeats the items. */
  events(lease: string, after: string): Promise<VoiceV2Batch>;
  submit(lease: string, action: VoiceV2Action): Promise<VoiceV2SubmitResult>;
  /** Idempotent. Native sends its own V2 bye naming this exact join and revokes the lease. */
  end(lease: string): Promise<void>;
}

/** Why native ended our join. Only epoch/policy replacement is repaired by a fresh join. */
export type VoiceV2EndReason = "epoch" | "policy" | "removed" | "room" | "locked" | "replaced" | "capacity";
const END_REASONS: ReadonlySet<string> = new Set(["epoch", "policy", "removed", "room", "locked", "replaced", "capacity"]);
const REJOINABLE: ReadonlySet<string> = new Set(["epoch", "policy"]);

/**
 * Native-authorized inbound items, already authenticated and bound to this exact lease.
 * - `peer`: a native watcher challenge/proof established this exact join as fresh now.
 * - `peer-gone`: native retired that exact join (removed member, expiry without renewal, tombstone).
 * - `paired`: native key confirmation finished for this exact join; SDP may flow.
 * - `offer`/`answer`/`ice`: decrypted and committed by native for the exact pair and OfferId.
 * - `bye`: the peer's V2 bye naming its exact join.
 * - `ended`: native ended OUR join.
 */
type Inbound =
  | Readonly<{ kind: "peer"; receipt: string; identity: string; join: string }>
  | Readonly<{ kind: "peer-gone"; receipt: string; identity: string; join: string; reason: "removed" | "expired" | "retired" }>
  | Readonly<{ kind: "paired"; receipt: string; identity: string; join: string }>
  | Readonly<{ kind: "offer" | "answer"; receipt: string; identity: string; join: string; offer: VoiceOfferId; sdp: string }>
  | Readonly<{ kind: "ice"; receipt: string; identity: string; join: string; offer: VoiceOfferId; candidate: VoiceV2Candidate | null }>
  | Readonly<{ kind: "bye"; receipt: string; identity: string; join: string }>
  | Readonly<{ kind: "ended"; receipt: string; reason: VoiceV2EndReason }>;

// ---------------------------------------------------------------------------------------------
// RTC seam. App.svelte supplies the real RTCPeerConnection with its tracks, data channels and
// `ontrack` wiring; the driver owns only negotiation. `isCurrent` lets App fence its own
// callbacks (audio attach, data channels) by this exact peer record and call generation.

export interface VoiceV2PeerConnection {
  readonly connectionState: string;
  readonly signalingState: string;
  readonly localDescription: { readonly type: string; readonly sdp: string } | null;
  setLocalDescription(description?: { type: "offer" | "answer"; sdp: string }): Promise<void>;
  setRemoteDescription(description: { type: "offer" | "answer"; sdp: string }): Promise<void>;
  addIceCandidate(candidate?: VoiceV2Candidate): Promise<void>;
  onicecandidate: ((event: { candidate: { toJSON(): unknown } | null }) => void) | null;
  onconnectionstatechange: (() => void) | null;
  close(): void;
}

export interface VoiceV2Rtc {
  create(identity: string, isCurrent: () => boolean): VoiceV2PeerConnection;
  /** Called once when the driver retires a peer object; App detaches audio and closes it. */
  retire(identity: string, pc: VoiceV2PeerConnection): void;
}

export type VoiceV2Clock = Readonly<{ now(): number; sleep(ms: number): Promise<void> }>;

export type VoiceV2Diagnostic = Readonly<{ code: string; fields?: Readonly<Record<string, string | number>> }>;

/**
 * "unavailable" is terminal for this join (refused, budget spent, native end). "uncertain" is not:
 * native replies are being lost, the driver keeps polling and returns to "joined" on success.
 */
export type VoiceV2Phase = "starting" | "joined" | "rejoining" | "uncertain" | "ended" | "unavailable";
export type VoiceV2Signalling =
  | "waiting-pair" | "pair-unavailable" | "paired" | "offering" | "answering"
  | "offer-submitted" | "answer-submitted" | "negotiated" | "refused" | "uncertain" | "failed";
export type VoiceV2Media = "none" | "connecting" | "connected" | "failed";

export type VoiceV2PeerSnapshot = Readonly<{
  identity: string;
  join: string;
  presence: "fresh" | "stale";
  signalling: VoiceV2Signalling;
  media: VoiceV2Media;
}>;

export type VoiceV2Snapshot = Readonly<{
  mode: "v2";
  phase: VoiceV2Phase;
  reason: string;
  generation: number;
  join: string;
  peers: readonly VoiceV2PeerSnapshot[];
}>;

export type VoiceV2Deps = Readonly<{
  native: VoiceV2Native;
  rtc: VoiceV2Rtc;
  clock: VoiceV2Clock;
  /** Unique, unguessable ids for start attempts and actions (crypto.randomUUID in production). */
  newId(): string;
  server: number;
  room: string;
  onChange?(snapshot: VoiceV2Snapshot): void;
  diagnose?(event: VoiceV2Diagnostic): void;
}>;

/** What App needs to decide a join's mode. V2 is never chosen while this is unavailable. */
export type VoiceV2Activation =
  | Readonly<{ available: false; missing: readonly string[] }>
  | Readonly<{ available: true; native: VoiceV2Native }>;

const UNAVAILABLE: VoiceV2Activation = Object.freeze({
  available: false,
  missing: Object.freeze([
    "native start/events/submit/end commands implementing VoiceV2Native",
    "handoff delivery of SDP/ICE actions and post-decrypt native authorization",
    "native SDP phase factories on a confirmed LocalJoin",
    "App RTC wiring for V2 peers (tracks, data channels, ontrack fencing)",
  ]),
});

/**
 * Whether this build may start V2 calls. It may not: see UNAVAILABLE.missing. Flipping this is
 * the activation step, and it must come with the native adapter, not before it.
 */
export function voiceV2Activation(): VoiceV2Activation {
  return UNAVAILABLE;
}

// ---------------------------------------------------------------------------------------------
// Native projection parsing. Native is trusted main-window IPC, but a closed shape check is cheap
// and turns an adapter bug into a skipped item plus a diagnostic instead of an RTC effect.

const U64_MAX = (1n << 64n) - 1n;
function isSerial(value: unknown): value is string {
  return typeof value === "string" && /^(0|[1-9][0-9]{0,19})$/.test(value) && BigInt(value) <= U64_MAX;
}

function parseCandidate(value: unknown): VoiceV2Candidate | null | undefined {
  if (value === null) return null; // end-of-candidates
  if (!value || typeof value !== "object") return undefined;
  const c = value as Record<string, unknown>;
  const candidate = c.candidate, mid = c.sdpMid ?? null, index = c.sdpMLineIndex ?? null, fragment = c.usernameFragment ?? null;
  if (typeof candidate !== "string" || candidate.length > VOICE_V2_MAX_ICE) return undefined;
  if (mid !== null && (typeof mid !== "string" || mid.length > 64)) return undefined;
  if (index !== null && (!Number.isInteger(index) || (index as number) < 0 || (index as number) > 255)) return undefined;
  if (fragment !== null && (typeof fragment !== "string" || fragment.length > 256)) return undefined;
  return Object.freeze({ candidate, sdpMid: mid as string | null, sdpMLineIndex: index as number | null, usernameFragment: fragment as string | null });
}

function candidateBytes(candidate: VoiceV2Candidate | null): number {
  if (!candidate) return 16;
  return candidate.candidate.length + (candidate.sdpMid?.length ?? 0) + (candidate.usernameFragment?.length ?? 0) + 16;
}

function parseInbound(value: unknown, self: string): Inbound | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (!isSerial(v.receipt) || v.receipt === "0") return null;
  const receipt = v.receipt;
  if (v.kind === "ended") {
    return typeof v.reason === "string" && END_REASONS.has(v.reason)
      ? Object.freeze({ kind: "ended", receipt, reason: v.reason as VoiceV2EndReason })
      : null;
  }
  // Every peer item names a full counterpart identity (never self) and its exact join.
  if (!isCallIdentity(v.identity) || v.identity === self || !isVoiceNonce(v.join)) return null;
  const identity = v.identity, join = v.join;
  switch (v.kind) {
    case "peer": case "paired": case "bye":
      return Object.freeze({ kind: v.kind, receipt, identity, join });
    case "peer-gone":
      return v.reason === "removed" || v.reason === "expired" || v.reason === "retired"
        ? Object.freeze({ kind: "peer-gone", receipt, identity, join, reason: v.reason })
        : null;
    case "offer": case "answer": {
      const offer = parseVoiceOfferId(v.offer);
      if (!offer || typeof v.sdp !== "string" || v.sdp.length === 0 || v.sdp.length > VOICE_V2_MAX_SDP) return null;
      // An offer is issued by its sender; an answer echoes an offer this device issued.
      if (v.kind === "offer" ? offer.issuer !== identity : offer.issuer !== self) return null;
      return Object.freeze({ kind: v.kind, receipt, identity, join, offer, sdp: v.sdp });
    }
    case "ice": {
      const offer = parseVoiceOfferId(v.offer);
      const candidate = parseCandidate(v.candidate);
      if (!offer || candidate === undefined || (offer.issuer !== identity && offer.issuer !== self)) return null;
      return Object.freeze({ kind: "ice", receipt, identity, join, offer, candidate });
    }
    default:
      return null;
  }
}

function parseLease(value: unknown, room: string): VoiceV2Lease | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (typeof v.lease !== "string" || v.lease.length === 0 || v.lease.length > 128) return null;
  if (!isVoiceNonce(v.group) || v.room !== room || !isCallIdentity(v.self) || !isVoiceNonce(v.join)) return null;
  return Object.freeze({ lease: v.lease, group: v.group, room: v.room, self: v.self, join: v.join });
}

// ---------------------------------------------------------------------------------------------

type PeerRecord = {
  readonly identity: string;
  readonly join: string;
  /** Distinguishes two records for one identity/join, so a retired object never passes a fence. */
  readonly serial: number;
  readonly initiator: boolean;
  readonly seenAt: number;
  presence: "fresh" | "stale";
  signalling: VoiceV2Signalling;
  media: VoiceV2Media;
  pc: VoiceV2PeerConnection | null;
  /** The pair's single negotiation. Set once; a second, different offer is refused. */
  offer: VoiceOfferId | null;
  remoteDescribed: boolean;
  /** This end's SDP submission resolved "submitted", so local ICE may follow it. */
  localSubmitted: boolean;
  pendingLocalIce: (VoiceV2Candidate | null)[];
  earlyIce: Map<string, { items: (VoiceV2Candidate | null)[]; bytes: number }>;
  earlyIceBytes: number;
  /** A newer fresh join of the same identity, waiting for native key confirmation. */
  pendingJoin: string | null;
  /** Serializes this record's SDP/ICE application; control items never wait on it. */
  chain: Promise<void>;
};

export class VoiceV2Call {
  readonly #deps: VoiceV2Deps;
  #generation = 0;
  #phase: VoiceV2Phase = "starting";
  #reason = "";
  #ended = false;
  #lease: VoiceV2Lease | null = null;
  #peers = new Map<string, PeerRecord>();
  #receipts = new Set<string>();
  #rejoins: number[] = [];
  #serial = 0;

  constructor(deps: VoiceV2Deps) {
    if (!Number.isInteger(deps.server) || !isVoiceRoomId(deps.room)) throw new Error("voice V2 room refused");
    this.#deps = deps;
  }

  /** Start the first join. Resolves once native has answered (or definitively refused). */
  start(): Promise<void> {
    return this.#join(this.#generation);
  }

  /**
   * End this call for good: leave, lock, unmount, server removal. Synchronously retires every
   * peer object and fences all queued work before anything awaits; the native end follows.
   */
  end(reason = "leave"): void {
    if (this.#ended) return;
    this.#ended = true;
    this.#generation++;
    this.#retireAll();
    this.#phase = "ended";
    this.#reason = reason;
    const lease = this.#lease;
    this.#lease = null;
    if (lease) void this.#endNative(lease.lease);
    this.#changed();
  }

  snapshot(): VoiceV2Snapshot {
    const now = this.#deps.clock.now();
    const peers = [...this.#peers.values()]
      .map((p) => Object.freeze({
        identity: p.identity,
        join: p.join,
        presence: p.presence,
        signalling: p.signalling === "waiting-pair" && now - p.seenAt > VOICE_V2_PAIR_WAIT_MS ? "pair-unavailable" : p.signalling,
        media: p.media,
      }))
      .sort((a, b) => (a.identity < b.identity ? -1 : 1));
    return Object.freeze({
      mode: "v2",
      phase: this.#phase,
      reason: this.#reason,
      generation: this.#generation,
      join: this.#lease?.join ?? "",
      peers: Object.freeze(peers),
    });
  }

  // -- lifecycle ------------------------------------------------------------------------------

  #liveCall(generation: number): boolean {
    return !this.#ended && this.#generation === generation;
  }

  /** The fence for every RTC continuation: same call generation, same peer object, same offer. */
  #live(generation: number, record: PeerRecord, offer?: VoiceOfferId): boolean {
    return (
      this.#liveCall(generation) &&
      this.#peers.get(record.identity) === record &&
      record.pc !== null &&
      (offer === undefined || sameVoiceOffer(record.offer, offer))
    );
  }

  async #join(generation: number): Promise<void> {
    const attempt = this.#deps.newId();
    // True once a start request may have committed natively without our learning its lease.
    let uncertain = false;
    for (let tries = 0; ; tries++) {
      // Nothing can have committed before the first request, so an ended call stops here. After
      // an uncertain reply it keeps retrying the SAME attempt even though the call has ended:
      // only that recovers the lease, so the join can be ended instead of left as a phantom
      // participant on the peers' side.
      if (!this.#liveCall(generation) && !uncertain) return;
      let result: VoiceV2StartResult;
      try {
        result = await this.#deps.native.start({ server: this.#deps.server, room: this.#deps.room, attempt });
      } catch {
        // Uncertain: native may have started a join whose reply we lost. Retry the SAME attempt
        // so native returns that same lease instead of minting a second join.
        uncertain = true;
        if (tries >= VOICE_V2_RETRIES) return this.#unavailable(generation, "start-uncertain");
        await this.#deps.clock.sleep(VOICE_V2_RETRY_MS[Math.min(tries, VOICE_V2_RETRY_MS.length - 1)]);
        continue;
      }
      if (result?.kind === "refused") {
        return this.#unavailable(generation, typeof result.reason === "string" ? `start-refused:${result.reason}` : "start-refused");
      }
      const lease = result?.kind === "started" ? parseLease(result.lease, this.#deps.room) : null;
      if (!lease) return this.#unavailable(generation, "start-malformed");
      if (!this.#liveCall(generation)) {
        // The call ended (leave, lock, unmount, a newer join) while native was starting this one.
        // Nothing may adopt it; end it so its native owner and bye do not linger.
        void this.#endNative(lease.lease);
        return;
      }
      this.#lease = lease;
      this.#receipts = new Set();
      this.#phase = "joined";
      this.#reason = "";
      this.#changed();
      void this.#pump(generation, lease);
      return;
    }
  }

  #unavailable(generation: number, reason: string): void {
    if (!this.#liveCall(generation)) return;
    // Never a legacy fallback: the join stays V2 and reports why it cannot proceed.
    this.#phase = "unavailable";
    this.#reason = reason;
    this.#diag("VOICE.V2.UNAVAILABLE", { reason });
    this.#changed();
  }

  async #endNative(lease: string): Promise<void> {
    for (let tries = 0; tries <= VOICE_V2_RETRIES; tries++) {
      try {
        await this.#deps.native.end(lease);
        return;
      } catch {
        if (tries < VOICE_V2_RETRIES) await this.#deps.clock.sleep(VOICE_V2_RETRY_MS[Math.min(tries, VOICE_V2_RETRY_MS.length - 1)]);
      }
    }
    // Native also revokes the lease on lock, unmount and owner replacement; this is diagnostic.
    this.#diag("VOICE.V2.END_UNCERTAIN");
  }

  /** Native ended our join. Repair epoch/policy replacement with a FRESH join; never revive. */
  #nativeEnded(generation: number, reason: VoiceV2EndReason): void {
    if (!this.#liveCall(generation)) return;
    this.#generation++;
    this.#retireAll();
    const lease = this.#lease;
    this.#lease = null;
    if (lease) void this.#endNative(lease.lease);
    const now = this.#deps.clock.now();
    this.#rejoins = this.#rejoins.filter((at) => now - at < VOICE_V2_REJOIN_WINDOW_MS);
    if (REJOINABLE.has(reason) && this.#rejoins.length < VOICE_V2_REJOINS) {
      this.#rejoins.push(now);
      this.#phase = "rejoining";
      this.#reason = reason;
      this.#changed();
      void this.#join(this.#generation);
      return;
    }
    this.#phase = "unavailable";
    this.#reason = REJOINABLE.has(reason) ? `${reason}:rejoin-budget` : reason;
    this.#diag("VOICE.V2.ENDED", { reason });
    this.#changed();
  }

  async #pump(generation: number, lease: VoiceV2Lease): Promise<void> {
    let after = "0";
    let failures = 0;
    while (this.#liveCall(generation)) {
      let batch: VoiceV2Batch;
      try {
        batch = await this.#deps.native.events(lease.lease, after);
        failures = 0;
      } catch {
        // Uncertain pull: repeat the same cursor. Non-dequeueing pulls make this exact. A revoked
        // lease must be answered authoritatively by native (an `ended` item), so endless failure
        // means the native side is unreachable: say so, keep polling, and never fall back.
        failures++;
        if (failures === VOICE_V2_PULL_UNCERTAIN && this.#liveCall(generation)) {
          this.#phase = "uncertain";
          this.#reason = "events-uncertain";
          this.#diag("VOICE.V2.UNCERTAIN", { reason: "events-uncertain" });
          this.#changed();
        }
        await this.#deps.clock.sleep(VOICE_V2_RETRY_MS[Math.min(failures - 1, VOICE_V2_RETRY_MS.length - 1)]);
        continue;
      }
      if (!this.#liveCall(generation)) return;
      if (this.#phase === "uncertain") {
        this.#phase = "joined";
        this.#reason = "";
        this.#changed();
      }
      const items = Array.isArray(batch?.items) ? batch.items : null;
      if (!isSerial(batch?.revision) || !items || items.length > VOICE_V2_BATCH || BigInt(batch.revision) < BigInt(after)) {
        this.#diag("VOICE.V2.BATCH_REFUSED");
        await this.#deps.clock.sleep(VOICE_V2_RETRY_MS[VOICE_V2_RETRY_MS.length - 1]);
        continue;
      }
      after = batch.revision;
      for (const raw of items) {
        if (!this.#liveCall(generation)) return;
        const item = parseInbound(raw, lease.self);
        if (!item) {
          this.#diag("VOICE.V2.ITEM_REFUSED");
          continue;
        }
        if (this.#receipts.has(item.receipt)) continue; // duplicate delivery of one native result
        this.#receipts.add(item.receipt);
        if (this.#receipts.size > VOICE_V2_RECEIPTS) this.#receipts.delete(this.#receipts.values().next().value as string);
        this.#apply(generation, lease, item);
      }
      if (items.length === 0) await this.#deps.clock.sleep(VOICE_V2_IDLE_MS);
    }
  }

  // -- inbound --------------------------------------------------------------------------------

  #apply(generation: number, lease: VoiceV2Lease, item: Inbound): void {
    switch (item.kind) {
      case "ended": return this.#nativeEnded(generation, item.reason);
      case "peer": return this.#peerFresh(lease, item.identity, item.join);
      case "paired": return this.#paired(generation, lease, item.identity, item.join);
      case "bye": return this.#peerLeft(lease, item.identity, item.join, "bye", true);
      case "peer-gone": return this.#peerLeft(lease, item.identity, item.join, item.reason, item.reason !== "expired");
      case "offer": case "answer": case "ice": {
        const record = this.#peers.get(item.identity);
        if (!record || record.join !== item.join) {
          // A join this driver does not hold now: an older join's late traffic, or one replaced.
          this.#diag("VOICE.V2.STALE_JOIN", { kind: item.kind });
          return;
        }
        record.chain = record.chain.then(() => this.#applyRtc(generation, record, item)).catch(() => {});
        return;
      }
    }
  }

  #peerFresh(lease: VoiceV2Lease, identity: string, join: string): void {
    const record = this.#peers.get(identity);
    if (record && record.join === join) {
      record.presence = "fresh";
      this.#changed();
      return;
    }
    if (record) {
      if (record.pc === null || record.signalling === "waiting-pair" || record.signalling === "pair-unavailable") {
        // Nothing working to preserve: a newer fresh join simply replaces the unpaired candidate.
        this.#retire(record);
      } else {
        // A working pair stays until the newer join's key confirmation succeeds (native promotion).
        record.pendingJoin = join;
        this.#changed();
        return;
      }
    }
    if (this.#peers.size >= VOICE_V2_MAX_PEERS) {
      this.#diag("VOICE.V2.PEER_CAPACITY");
      return;
    }
    this.#peers.set(identity, this.#record(identity, join, lease.self));
    this.#changed();
  }

  #record(identity: string, join: string, self: string): PeerRecord {
    return {
      identity, join, serial: ++this.#serial,
      initiator: isVoiceInitiatorTowards(self, identity),
      seenAt: this.#deps.clock.now(),
      presence: "fresh", signalling: "waiting-pair", media: "none", pc: null,
      offer: null, remoteDescribed: false, localSubmitted: false,
      pendingLocalIce: [], earlyIce: new Map(), earlyIceBytes: 0, pendingJoin: null,
      chain: Promise.resolve(),
    };
  }

  #paired(generation: number, lease: VoiceV2Lease, identity: string, join: string): void {
    let record = this.#peers.get(identity);
    if (record && record.join !== join) {
      if (record.pendingJoin !== join) {
        this.#diag("VOICE.V2.STALE_JOIN", { kind: "paired" });
        return;
      }
      // Promotion: the newer join is confirmed, so the older pair and all its work retire now.
      this.#retire(record);
      record = this.#record(identity, join, lease.self);
      this.#peers.set(identity, record);
    }
    if (!record || record.pc !== null) return; // unknown join, or a duplicate confirmation
    const current = record;
    current.signalling = "paired";
    current.pc = this.#deps.rtc.create(identity, () => this.#live(generation, current));
    this.#wire(generation, current);
    this.#changed();
    // On the record's chain, so an answer or ICE that outruns our own offer's submit reply waits
    // for the OfferId it echoes instead of being judged stale and losing its receipt.
    if (current.initiator) current.chain = current.chain.then(() => this.#offer(generation, current)).catch(() => {});
  }

  #peerLeft(lease: VoiceV2Lease, identity: string, join: string, reason: string, tearDown: boolean): void {
    const record = this.#peers.get(identity);
    if (!record) return;
    if (record.pendingJoin === join) {
      record.pendingJoin = null;
      this.#changed();
      return;
    }
    // Exact join only. A delayed bye from the peer's previous join can never dismiss its newer join.
    if (record.join !== join) {
      this.#diag("VOICE.V2.STALE_JOIN", { kind: reason });
      return;
    }
    if (!tearDown && record.pc !== null) {
      // Observation expiry is not pair retirement. An established pair may be mid-reconnect
      // (RTC "disconnected" reads as connecting), and native keeps the pair across discovery
      // expiry without a new confirmation, so a destroyed record could never be re-paired for this
      // join. Keep the connection and its OfferId, report stale presence, and let renewal of the
      // same join restore it. Only bye, removal, retirement or a confirmed replacement end it.
      record.presence = "stale";
      this.#changed();
      return;
    }
    const pending = record.pendingJoin;
    this.#retire(record);
    // Native may retire the old join before the newer one's confirmation arrives. The newer
    // join is already native-fresh, so it becomes the waiting candidate instead of being lost.
    if (pending) this.#peers.set(identity, this.#record(identity, pending, lease.self));
    this.#changed();
  }

  async #applyRtc(generation: number, record: PeerRecord, item: Extract<Inbound, { kind: "offer" | "answer" | "ice" }>): Promise<void> {
    if (!this.#live(generation, record)) return;
    const pc = record.pc!;
    if (item.kind === "ice") {
      if (record.offer && !sameVoiceOffer(record.offer, item.offer)) {
        this.#diag("VOICE.V2.STALE_ICE");
        return; // ICE for a retired or foreign negotiation is never reassigned to the current one
      }
      if (!record.offer || !record.remoteDescribed) return this.#bufferEarlyIce(record, item.offer, item.candidate);
      try {
        await pc.addIceCandidate(item.candidate ?? undefined);
      } catch {
        this.#diag("VOICE.V2.ICE_FAILED");
      }
      return;
    }
    if (item.kind === "offer") {
      if (record.initiator) {
        // This end owns the offer for this pair. A counter-offer is crossed; native refuses it
        // before positive admission and so does the renderer, without rolling anything back.
        this.#diag("VOICE.V2.CROSSED_OFFER");
        return;
      }
      if (record.offer) {
        if (!sameVoiceOffer(record.offer, item.offer)) this.#diag("VOICE.V2.SECOND_OFFER");
        return; // one exchange per pair (narrow native SDP contract); duplicates apply once
      }
      record.offer = item.offer;
      record.signalling = "answering";
      this.#changed();
      try {
        await pc.setRemoteDescription({ type: "offer", sdp: item.sdp });
      } catch {
        if (this.#live(generation, record, item.offer)) this.#refused(record, "remote-offer");
        return;
      }
      if (!this.#live(generation, record, item.offer)) return;
      record.remoteDescribed = true;
      await this.#flushEarlyIce(generation, record, item.offer);
      if (!this.#live(generation, record, item.offer)) return;
      try {
        await pc.setLocalDescription();
      } catch {
        if (this.#live(generation, record, item.offer)) this.#refused(record, "local-answer");
        return;
      }
      if (!this.#live(generation, record, item.offer)) return;
      const answer = pc.localDescription;
      if (!answer || answer.type !== "answer" || answer.sdp.length === 0 || answer.sdp.length > VOICE_V2_MAX_SDP) {
        return this.#refused(record, "local-answer-size");
      }
      const result = await this.#submit(generation, record, {
        kind: "answer", action: this.#deps.newId(), peer: record.identity, peerJoin: record.join, offer: item.offer, sdp: answer.sdp,
      });
      if (!result || !this.#live(generation, record, item.offer)) return;
      if (result.status === "refused") return this.#refused(record, `answer:${result.reason}`);
      record.signalling = "answer-submitted";
      record.localSubmitted = true;
      this.#changed();
      this.#flushLocalIce(generation, record);
      return;
    }
    // Answer to the offer this end made.
    if (!record.initiator || !sameVoiceOffer(record.offer, item.offer)) {
      this.#diag("VOICE.V2.STALE_ANSWER");
      return;
    }
    if (record.remoteDescribed || pc.signalingState !== "have-local-offer") return; // duplicate
    try {
      await pc.setRemoteDescription({ type: "answer", sdp: item.sdp });
    } catch {
      if (this.#live(generation, record, item.offer)) this.#refused(record, "remote-answer");
      return;
    }
    if (!this.#live(generation, record, item.offer)) return;
    record.remoteDescribed = true;
    record.signalling = "negotiated";
    this.#changed();
    await this.#flushEarlyIce(generation, record, item.offer);
  }

  #bufferEarlyIce(record: PeerRecord, offer: VoiceOfferId, candidate: VoiceV2Candidate | null): void {
    const key = voiceOfferKey(offer);
    const bytes = candidateBytes(candidate);
    const slot = record.earlyIce.get(key) ?? { items: [], bytes: 0 };
    // Bounded per pair: refuse rather than evict, so a flood of early ICE cannot push out the
    // candidates of the offer that will actually apply. Only one negotiation exists per pair.
    if (slot.items.length >= VOICE_V2_EARLY_ICE || record.earlyIceBytes + bytes > VOICE_V2_EARLY_ICE_BYTES || (!record.earlyIce.has(key) && record.earlyIce.size >= 2)) {
      this.#diag("VOICE.V2.EARLY_ICE_FULL");
      return;
    }
    slot.items.push(candidate);
    slot.bytes += bytes;
    record.earlyIceBytes += bytes;
    record.earlyIce.set(key, slot);
  }

  async #flushEarlyIce(generation: number, record: PeerRecord, offer: VoiceOfferId): Promise<void> {
    const slot = record.earlyIce.get(voiceOfferKey(offer));
    // Any other buffered negotiation can never apply now; release it.
    record.earlyIce.clear();
    record.earlyIceBytes = 0;
    for (const candidate of slot?.items ?? []) {
      if (!this.#live(generation, record, offer)) return;
      try {
        await record.pc!.addIceCandidate(candidate ?? undefined);
      } catch {
        this.#diag("VOICE.V2.ICE_FAILED");
      }
    }
  }

  // -- outbound -------------------------------------------------------------------------------

  #wire(generation: number, record: PeerRecord): void {
    const pc = record.pc!;
    pc.onicecandidate = (event) => {
      if (!this.#live(generation, record)) return;
      const candidate = event.candidate === null ? null : parseCandidate(event.candidate.toJSON());
      if (candidate === undefined) {
        this.#diag("VOICE.V2.LOCAL_ICE_REFUSED");
        return;
      }
      if (record.offer && record.localSubmitted) {
        void this.#sendIce(generation, record, record.offer, candidate);
      } else if (record.pendingLocalIce.length < VOICE_V2_PENDING_LOCAL_ICE) {
        record.pendingLocalIce.push(candidate);
      } else {
        this.#diag("VOICE.V2.LOCAL_ICE_FULL");
      }
    };
    pc.onconnectionstatechange = () => {
      if (!this.#live(generation, record)) return;
      // Media is only what the RTC transport reports. "disconnected" may recover by itself, so it
      // reads as connecting again; "new" and "closed" keep the last fact (closure is retirement).
      const state = pc.connectionState;
      if (state === "connected") record.media = "connected";
      else if (state === "failed") record.media = "failed";
      else if (state === "connecting" || state === "disconnected") record.media = "connecting";
      this.#changed();
    };
  }

  async #offer(generation: number, record: PeerRecord): Promise<void> {
    record.signalling = "offering";
    this.#changed();
    const pc = record.pc!;
    try {
      await pc.setLocalDescription();
    } catch {
      if (this.#live(generation, record)) this.#refused(record, "local-offer");
      return;
    }
    if (!this.#live(generation, record)) return;
    const offer = pc.localDescription;
    if (!offer || offer.type !== "offer" || offer.sdp.length === 0 || offer.sdp.length > VOICE_V2_MAX_SDP) {
      return this.#refused(record, "local-offer-size");
    }
    const result = await this.#submit(generation, record, {
      kind: "offer", action: this.#deps.newId(), peer: record.identity, peerJoin: record.join, sdp: offer.sdp,
    });
    if (!result || !this.#live(generation, record)) return;
    if (result.status === "refused") return this.#refused(record, `offer:${result.reason}`);
    const self = this.#lease?.self;
    if (!result.offer || result.offer.issuer !== self || record.offer) {
      // A submitted offer must name this device as issuer, exactly once per pair.
      if (!record.offer) this.#refused(record, "offer-id");
      return;
    }
    record.offer = result.offer;
    record.localSubmitted = true;
    record.signalling = "offer-submitted";
    this.#changed();
    // Remote ICE that arrived before we learnt our own OfferId may now be matched to it. It still
    // waits for the answer's remote description; only the matching negotiation is retained.
    for (const key of [...record.earlyIce.keys()]) {
      if (key !== voiceOfferKey(result.offer)) {
        record.earlyIceBytes -= record.earlyIce.get(key)!.bytes;
        record.earlyIce.delete(key);
      }
    }
    this.#flushLocalIce(generation, record);
  }

  #flushLocalIce(generation: number, record: PeerRecord): void {
    const pending = record.pendingLocalIce;
    record.pendingLocalIce = [];
    const offer = record.offer;
    if (!offer) return;
    for (const candidate of pending) void this.#sendIce(generation, record, offer, candidate);
  }

  async #sendIce(generation: number, record: PeerRecord, offer: VoiceOfferId, candidate: VoiceV2Candidate | null): Promise<void> {
    const result = await this.#submit(generation, record, {
      kind: "ice", action: this.#deps.newId(), peer: record.identity, peerJoin: record.join, offer, candidate,
    });
    if (result?.status === "refused" && this.#live(generation, record, offer)) this.#diag("VOICE.V2.ICE_REFUSED");
  }

  /**
   * Submit one action, retrying the EXACT same action after an uncertain reply. A resolved
   * result is authoritative, and a duplicate result for a retried action is the cached original.
   * Returns null when the call/peer was retired or every retry stayed uncertain.
   */
  async #submit(generation: number, record: PeerRecord, action: VoiceV2Action): Promise<VoiceV2SubmitResult | null> {
    const lease = this.#lease;
    if (!lease) return null;
    const started = this.#deps.clock.now();
    for (let tries = 0; ; tries++) {
      if (!this.#live(generation, record)) return null;
      try {
        const result = await this.#deps.native.submit(lease.lease, action);
        if (!result || result.action !== action.action || (result.status !== "submitted" && result.status !== "refused")) {
          this.#diag("VOICE.V2.RESULT_REFUSED", { kind: action.kind });
          return null;
        }
        if (result.status === "refused") {
          return Object.freeze({ status: "refused", action: action.action, reason: typeof result.reason === "string" ? result.reason : "" });
        }
        // Keep only the validated, frozen OfferId; never the adapter's own (mutable) object.
        const offer = result.offer === null || result.offer === undefined ? null : parseVoiceOfferId(result.offer);
        if (offer === null && result.offer !== null && result.offer !== undefined) {
          this.#diag("VOICE.V2.RESULT_REFUSED", { kind: action.kind });
          return null;
        }
        return Object.freeze({ status: "submitted", action: action.action, offer });
      } catch {
        if (action.kind === "ice") {
          // A lost candidate is cheap to lose and the negotiation does not wait on it.
          if (tries >= VOICE_V2_RETRIES) return null;
        } else {
          // Offer/answer: the native side may have committed it (and the peer may already have
          // answered), so keep retrying the EXACT action while native can still return its cached
          // result. "uncertain" therefore always means scheduled recovery work exists.
          if (tries + 1 === VOICE_V2_RETRIES && this.#live(generation, record)) {
            record.signalling = "uncertain";
            this.#changed();
          }
          if (this.#deps.clock.now() - started >= VOICE_V2_RECEIPT_MS) {
            // The cached result may be gone: further retries could neither recover the original
            // OfferId nor be distinguished from a new action. Stop and say so, actionably.
            if (this.#live(generation, record)) {
              record.signalling = "failed";
              this.#diag("VOICE.V2.SUBMIT_FAILED", { kind: action.kind });
              this.#changed();
            }
            return null;
          }
        }
        await this.#deps.clock.sleep(VOICE_V2_RETRY_MS[Math.min(tries, VOICE_V2_RETRY_MS.length - 1)]);
      }
    }
  }

  #refused(record: PeerRecord, reason: string): void {
    record.signalling = "refused";
    this.#diag("VOICE.V2.NEGOTIATION_REFUSED", { reason: reason.slice(0, 48) });
    this.#changed();
  }

  // -- teardown -------------------------------------------------------------------------------

  #retire(record: PeerRecord): void {
    if (this.#peers.get(record.identity) === record) this.#peers.delete(record.identity);
    const pc = record.pc;
    record.pc = null; // fences every queued continuation and callback of this object first
    record.earlyIce.clear();
    record.pendingLocalIce = [];
    if (pc) {
      pc.onicecandidate = null;
      pc.onconnectionstatechange = null;
      this.#deps.rtc.retire(record.identity, pc);
    }
  }

  #retireAll(): void {
    for (const record of [...this.#peers.values()]) this.#retire(record);
    this.#peers.clear();
  }

  #changed(): void {
    this.#deps.onChange?.(this.snapshot());
  }

  #diag(code: string, fields?: Record<string, string | number>): void {
    this.#deps.diagnose?.({ code, fields });
  }
}

/**
 * A short, honest status line for one V2 peer. Signalling, native submission and media are kept
 * apart: "Offer sent" is native admission, not the peer's receipt, and only a connected media
 * state says audio can flow.
 */
export function describeVoiceV2Peer(peer: VoiceV2PeerSnapshot): string {
  if (peer.media === "connected") return peer.presence === "stale" ? "Connected, presence not renewed" : "Connected";
  if (peer.signalling === "failed") return "Signalling failed, leave and rejoin to retry";
  if (peer.media === "failed") return "Signalling done, no media route";
  if (peer.presence === "stale") return "Presence not renewed, keeping the call open";
  switch (peer.signalling) {
    case "waiting-pair": return "In the room, confirming keys";
    case "pair-unavailable": return "In the room, key confirmation not reached";
    case "paired": case "offering": case "answering": return "Keys confirmed, negotiating";
    case "offer-submitted": return "Offer sent, waiting for answer";
    case "answer-submitted": return "Answer sent, waiting for media";
    case "negotiated": return "Signalling done, connecting media";
    case "refused": return "Negotiation refused";
    case "uncertain": return "Signalling uncertain, still trying";
  }
}
