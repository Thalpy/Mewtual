import {
  JAM_ABUSE_EXHAUSTED_MS,
  JAM_ABUSE_WINDOW_MS,
  JAM_FRAME_BUCKET_BURST,
  JAM_FRAME_BUCKET_RATE,
  JAM_FRAME_MAX_BYTES,
  JAM_CLOCK_PROBE_BURST,
  JAM_CLOCK_PROBE_RATE,
  JAM_NOTEON_BUCKET_BURST,
  JAM_NOTEON_BUCKET_RATE,
  JAM_PATCH_ANNOUNCE_BURST,
  JAM_PATCH_ANNOUNCE_MIN_INTERVAL_MS,
} from "./jam-contract.ts";

export type JamBudgetDenial = "not-string" | "too-large" | "frame-rate" | "note-rate" | "patch-rate" |
  "clock-rate" | "abuse-muted";

/** Deterministic token bucket: callers provide monotonic time, so tests need no ambient clock. */
export class JamTokenBucket {
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
type Exhaustion = { start: number; end: number };

/**
 * Per-authenticated-peer ingress budgets. Frame size and the all-frame bucket run before parsing;
 * the note/patch buckets run only after a bounded object has identified its subtype.
 */
export class JamPeerBudget {
  private readonly frames = new JamTokenBucket(JAM_FRAME_BUCKET_BURST, JAM_FRAME_BUCKET_RATE);
  private readonly noteOns = new JamTokenBucket(JAM_NOTEON_BUCKET_BURST, JAM_NOTEON_BUCKET_RATE);
  private readonly patches = new JamTokenBucket(
    JAM_PATCH_ANNOUNCE_BURST,
    1_000 / JAM_PATCH_ANNOUNCE_MIN_INTERVAL_MS,
  );
  private readonly clockProbes = new JamTokenBucket(JAM_CLOCK_PROBE_BURST, JAM_CLOCK_PROBE_RATE);
  private exhaustions: Exhaustion[] = [];
  private currentExhaustion: Exhaustion | null = null;
  private lastDeniedMs: number | null = null;
  private abuseMuted = false;
  // Exact raw frame last confirmed by the engine after descriptor hash verification. This is
  // bounded by admitFrame and permits only reconnect recovery, never a fresh descriptor.
  private verifiedPatchFrame: string | null = null;

  admitFrame(raw: unknown, nowMs: number): Readonly<{ ok: true; raw: string }> | Readonly<{ ok: false; reason: JamBudgetDenial }> {
    if (this.abuseMuted) return { ok: false, reason: "abuse-muted" };
    // Every data-channel delivery pays before any content-dependent work, including wrong binary
    // types and oversize strings. An attacker cannot evade the rate budget by choosing a frame the
    // parser will reject anyway.
    if (!this.frames.charge(nowMs)) {
      this.recordExhaustion(nowMs);
      if (this.exhaustedMs(nowMs) >= JAM_ABUSE_EXHAUSTED_MS) {
        this.abuseMuted = true;
        return { ok: false, reason: "abuse-muted" };
      }
      return { ok: false, reason: "frame-rate" };
    }
    this.closeExhaustion(nowMs);
    if (typeof raw !== "string") return { ok: false, reason: "not-string" };
    // The allocation-free UTF-16 check makes sure only a bounded string reaches TextEncoder.
    if (raw.length > JAM_FRAME_MAX_BYTES || encoder.encode(raw).byteLength > JAM_FRAME_MAX_BYTES) {
      return { ok: false, reason: "too-large" };
    }
    return { ok: true, raw };
  }

  admitNoteOn(nowMs: number): boolean {
    return !this.abuseMuted && this.noteOns.charge(nowMs);
  }

  admitPatch(nowMs: number): boolean {
    return !this.abuseMuted && this.patches.charge(nowMs);
  }

  isVerifiedPatchFrame(raw: string): boolean {
    return !this.abuseMuted && this.verifiedPatchFrame === raw;
  }

  confirmVerifiedPatchFrame(raw: string): void {
    // The caller is JamFrameDecoder after JamEngine reported installed/cached. Preserve the raw
    // canonical sender frame so a fresh decoder can recognize exactly this recipe without hash
    // work; no sender-controlled key or claimed id is sufficient on its own.
    if (encoder.encode(raw).byteLength <= JAM_FRAME_MAX_BYTES) this.verifiedPatchFrame = raw;
  }

  admitClockProbe(nowMs: number): boolean {
    return !this.abuseMuted && this.clockProbes.charge(nowMs);
  }

  isAbuseMuted(): boolean {
    return this.abuseMuted;
  }

  /** Manual unmute starts fresh; peers are never told about this receiver-side policy. */
  clearAbuseMute(): void {
    this.abuseMuted = false;
    this.exhaustions = [];
    this.currentExhaustion = null;
    this.lastDeniedMs = null;
    this.frames.reset();
    this.noteOns.reset();
    this.patches.reset();
    this.clockProbes.reset();
  }

  private recordExhaustion(nowMs: number): void {
    if (!Number.isFinite(nowMs)) return;
    // A frame-rate bucket can regain one token after this interval. A longer silence therefore
    // closes the old exhausted episode instead of falsely counting idle time as continuing abuse.
    const grace = 1_000 / JAM_FRAME_BUCKET_RATE;
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
    this.currentExhaustion.end = Math.min(nowMs, (this.lastDeniedMs ?? nowMs) + (1_000 / JAM_FRAME_BUCKET_RATE));
    this.exhaustions.push(this.currentExhaustion);
    this.currentExhaustion = null;
    this.lastDeniedMs = null;
    this.prune(nowMs);
  }

  private prune(nowMs: number): void {
    const floor = nowMs - JAM_ABUSE_WINDOW_MS;
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
