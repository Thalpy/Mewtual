// rt-admission.ts: the per-peer admission model every realtime call data channel shares
// (design-creative-suite.md 2.3, slice C0b).
//
// The jam `inst` channel (id 7) established the model and the Flipnote `draw` channel (id 8)
// reuses it with its own limits:
//
// - an opaque per-channel capability, minted for one exact authenticated channel generation, that
//   every callback must present (authority is never recovered from a frame field or a string);
// - a deterministic token bucket whose callers supply monotonic time, so tests need no clock;
// - a pre-parse gate that charges every delivery against an all-frame bucket BEFORE any
//   content-dependent work, then bounds the frame in UTF-8 bytes; and
// - a sustained-exhaustion auto-mute: a peer whose bucket stays empty for a cumulative interval
//   inside a rolling window is receive-muted for the rest of the call.
//
// This module holds only the shared mechanism. Each channel keeps its own limits, its own
// semantic buckets and its own parser; nothing here knows a message vocabulary.

const CHANNEL_BRAND = Symbol("mewtual-rt-source-channel");

/**
 * Opaque capability for one exact authenticated data-channel generation of one channel kind.
 *
 * Event callbacks close over this object. Identity, not a caller-supplied generation number,
 * prevents an already queued callback from an older/removed channel being mistaken for current.
 * The `kind` field keeps a draw capability from type-checking where a jam capability is required
 * (and is a debugging aid); registries never consult it for authority.
 */
export type RtSourceChannel<K extends string = string> = Readonly<{
  kind: K;
  source: string;
  serial: number;
  [CHANNEL_BRAND]: true;
}>;

/**
 * The current capability per authenticated source, for one channel kind. `open` replaces any
 * previous capability for that source, so a queued callback holding the old one fails `isCurrent`.
 */
export class RtSourceChannelRegistry<K extends string = string> {
  readonly kind: K;
  private readonly currentBySource = new Map<string, RtSourceChannel<K>>();
  private nextSerial = 1;

  constructor(kind: K) {
    this.kind = kind;
  }

  open(source: string): RtSourceChannel<K> {
    if (!source) throw new TypeError(`${this.kind} source channel needs an authenticated source identity`);
    const token = Object.freeze({
      kind: this.kind,
      source,
      serial: this.nextSerial,
      [CHANNEL_BRAND]: true as const,
    });
    this.nextSerial = this.nextSerial >= Number.MAX_SAFE_INTEGER ? 1 : this.nextSerial + 1;
    this.currentBySource.set(source, token);
    return token;
  }

  isCurrent(channel: RtSourceChannel<K>): boolean {
    return !!channel && this.currentBySource.get(channel.source) === channel;
  }

  current(source: string): RtSourceChannel<K> | null {
    return this.currentBySource.get(source) ?? null;
  }

  close(source: string): boolean {
    return this.currentBySource.delete(source);
  }

  clear(): void {
    this.currentBySource.clear();
  }
}

/** Deterministic token bucket: callers provide monotonic time, so tests need no ambient clock. */
export class RtTokenBucket {
  readonly burst: number;
  readonly refillPerSecond: number;
  private tokens: number;
  private lastMs: number | null = null;

  constructor(burst: number, refillPerSecond: number) {
    if (!(burst > 0) || !(refillPerSecond >= 0)) throw new RangeError("invalid token-bucket bounds");
    this.burst = burst;
    this.refillPerSecond = refillPerSecond;
    this.tokens = burst;
  }

  charge(nowMs: number, cost = 1): boolean {
    if (!Number.isFinite(nowMs) || !Number.isFinite(cost) || cost <= 0) return false;
    if (this.lastMs === null) this.lastMs = nowMs;
    else if (nowMs > this.lastMs) {
      this.tokens = Math.min(this.burst, this.tokens + ((nowMs - this.lastMs) / 1_000) * this.refillPerSecond);
      this.lastMs = nowMs;
    }
    // A backward clock step refills nothing and cannot move the refill origin backwards.
    if (this.tokens < cost) return false;
    this.tokens -= cost;
    return true;
  }

  reset(): void {
    this.tokens = this.burst;
    this.lastMs = null;
  }
}

const encoder = new TextEncoder();

/**
 * True when `raw` fits `maxBytes` once UTF-8 encoded. The allocation-free UTF-16 length check
 * runs first (a UTF-16 code unit never encodes to fewer than one byte), so only a bounded string
 * ever reaches TextEncoder.
 */
export function fitsUtf8Bytes(raw: string, maxBytes: number): boolean {
  return raw.length <= maxBytes && encoder.encode(raw).byteLength <= maxBytes;
}

/** Limits for one channel kind's pre-parse gate. */
export type RtFrameLimits = Readonly<{
  /** Largest delivered frame, in UTF-8 bytes, that may reach a parser. */
  maxBytes: number;
  /** All-frame bucket: charged for every delivery, valid or not. */
  bucketBurst: number;
  bucketRatePerSecond: number;
  /** Cumulative exhausted time inside the rolling window that auto-mutes the peer. */
  abuseExhaustedMs: number;
  abuseWindowMs: number;
}>;

export type RtAdmissionDenial = "not-string" | "too-large" | "frame-rate" | "abuse-muted";
export type RtAdmission =
  | Readonly<{ ok: true; raw: string }>
  | Readonly<{ ok: false; reason: RtAdmissionDenial }>;

type Exhaustion = { start: number; end: number };

/**
 * One authenticated peer's pre-parse gate for one channel kind.
 *
 * Owners keep this for the whole call epoch, keyed by the authenticated identity, not per
 * reconnectable channel: reconnecting must not refill a burst or clear an auto-mute. Only the
 * receiver's explicit `clearAbuseMute` resets it, and leaving the call discards it.
 */
export class RtPeerAdmission {
  readonly limits: RtFrameLimits;
  private readonly frames: RtTokenBucket;
  private exhaustions: Exhaustion[] = [];
  private currentExhaustion: Exhaustion | null = null;
  private lastDeniedMs: number | null = null;
  private abuseMuted = false;

  constructor(limits: RtFrameLimits) {
    if (!(limits.maxBytes > 0) || !(limits.bucketRatePerSecond > 0)) throw new RangeError("invalid admission limits");
    this.limits = limits;
    this.frames = new RtTokenBucket(limits.bucketBurst, limits.bucketRatePerSecond);
  }

  admitFrame(raw: unknown, nowMs: number): RtAdmission {
    if (this.abuseMuted) return { ok: false, reason: "abuse-muted" };
    // Every data-channel delivery pays before any content-dependent work, including wrong binary
    // types and oversize strings. An attacker cannot evade the rate budget by choosing a frame the
    // parser will reject anyway.
    if (!this.frames.charge(nowMs)) {
      this.recordExhaustion(nowMs);
      if (this.exhaustedMs(nowMs) >= this.limits.abuseExhaustedMs) {
        this.abuseMuted = true;
        return { ok: false, reason: "abuse-muted" };
      }
      return { ok: false, reason: "frame-rate" };
    }
    this.closeExhaustion(nowMs);
    if (typeof raw !== "string") return { ok: false, reason: "not-string" };
    if (!fitsUtf8Bytes(raw, this.limits.maxBytes)) return { ok: false, reason: "too-large" };
    return { ok: true, raw };
  }

  isAbuseMuted(): boolean {
    return this.abuseMuted;
  }

  /**
   * Manual unmute starts this gate fresh; peers are never told about this receiver-side policy.
   * Owners that layer semantic buckets on top reset those themselves.
   */
  clearAbuseMute(): void {
    this.abuseMuted = false;
    this.exhaustions = [];
    this.currentExhaustion = null;
    this.lastDeniedMs = null;
    this.frames.reset();
  }

  private recordExhaustion(nowMs: number): void {
    if (!Number.isFinite(nowMs)) return;
    // A frame-rate bucket can regain one token after this interval. A longer silence therefore
    // closes the old exhausted episode instead of falsely counting idle time as continuing abuse.
    const grace = 1_000 / this.limits.bucketRatePerSecond;
    if (!this.currentExhaustion || this.lastDeniedMs === null || nowMs - this.lastDeniedMs > grace * 2) {
      this.closeExhaustion(this.lastDeniedMs === null ? nowMs : this.lastDeniedMs + grace);
      this.currentExhaustion = { start: nowMs, end: nowMs };
    } else {
      this.currentExhaustion.end = nowMs;
    }
    this.lastDeniedMs = nowMs;
    this.prune(nowMs);
  }

  private closeExhaustion(nowMs: number): void {
    if (!this.currentExhaustion) return;
    this.currentExhaustion.end = Math.min(nowMs, (this.lastDeniedMs ?? nowMs) + (1_000 / this.limits.bucketRatePerSecond));
    this.exhaustions.push(this.currentExhaustion);
    this.currentExhaustion = null;
    this.lastDeniedMs = null;
    this.prune(nowMs);
  }

  private prune(nowMs: number): void {
    const floor = nowMs - this.limits.abuseWindowMs;
    this.exhaustions = this.exhaustions
      .filter((episode) => episode.end > floor)
      .map((episode) => ({ start: Math.max(floor, episode.start), end: episode.end }));
  }

  private exhaustedMs(nowMs: number): number {
    this.prune(nowMs);
    let total = this.exhaustions.reduce((sum, episode) => sum + Math.max(0, episode.end - episode.start), 0);
    if (this.currentExhaustion) total += Math.max(0, nowMs - this.currentExhaustion.start);
    return total;
  }
}
