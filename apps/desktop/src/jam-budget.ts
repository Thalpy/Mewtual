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
import { RtPeerAdmission, RtTokenBucket, fitsUtf8Bytes, type RtAdmission, type RtFrameLimits } from "./rt-admission.ts";

export type JamBudgetDenial = "not-string" | "too-large" | "frame-rate" | "note-rate" | "patch-rate" |
  "clock-rate" | "abuse-muted";

/** The shared deterministic token bucket (rt-admission.ts), under the name jam callers use. */
export { RtTokenBucket as JamTokenBucket };

/** The instrument channel's pre-parse gate, exactly the jam:v2 contract (INTERFACES.md 12). */
export const JAM_FRAME_LIMITS: RtFrameLimits = Object.freeze({
  maxBytes: JAM_FRAME_MAX_BYTES,
  bucketBurst: JAM_FRAME_BUCKET_BURST,
  bucketRatePerSecond: JAM_FRAME_BUCKET_RATE,
  abuseExhaustedMs: JAM_ABUSE_EXHAUSTED_MS,
  abuseWindowMs: JAM_ABUSE_WINDOW_MS,
});

/**
 * Per-authenticated-peer ingress budgets. Frame size and the all-frame bucket run before parsing
 * (the shared rt-admission gate); the note/patch/clock buckets run only after a bounded object has
 * identified its subtype. Every semantic lane is closed while the shared gate is auto-muted.
 */
export class JamPeerBudget {
  private readonly admission = new RtPeerAdmission(JAM_FRAME_LIMITS);
  private readonly noteOns = new RtTokenBucket(JAM_NOTEON_BUCKET_BURST, JAM_NOTEON_BUCKET_RATE);
  private readonly patches = new RtTokenBucket(
    JAM_PATCH_ANNOUNCE_BURST,
    1_000 / JAM_PATCH_ANNOUNCE_MIN_INTERVAL_MS,
  );
  private readonly clockProbes = new RtTokenBucket(JAM_CLOCK_PROBE_BURST, JAM_CLOCK_PROBE_RATE);
  // Exact raw frame last confirmed by the engine after descriptor hash verification. This is
  // bounded by admitFrame and permits only reconnect recovery, never a fresh descriptor.
  private verifiedPatchFrame: string | null = null;

  admitFrame(raw: unknown, nowMs: number): RtAdmission {
    return this.admission.admitFrame(raw, nowMs);
  }

  admitNoteOn(nowMs: number): boolean {
    return !this.admission.isAbuseMuted() && this.noteOns.charge(nowMs);
  }

  admitPatch(nowMs: number): boolean {
    return !this.admission.isAbuseMuted() && this.patches.charge(nowMs);
  }

  isVerifiedPatchFrame(raw: string): boolean {
    return !this.admission.isAbuseMuted() && this.verifiedPatchFrame === raw;
  }

  confirmVerifiedPatchFrame(raw: string): void {
    // The caller is JamFrameDecoder after JamEngine reported installed/cached. Preserve the raw
    // canonical sender frame so a fresh decoder can recognize exactly this recipe without hash
    // work; no sender-controlled key or claimed id is sufficient on its own.
    if (fitsUtf8Bytes(raw, JAM_FRAME_MAX_BYTES)) this.verifiedPatchFrame = raw;
  }

  admitClockProbe(nowMs: number): boolean {
    return !this.admission.isAbuseMuted() && this.clockProbes.charge(nowMs);
  }

  isAbuseMuted(): boolean {
    return this.admission.isAbuseMuted();
  }

  /** Manual unmute starts fresh; peers are never told about this receiver-side policy. */
  clearAbuseMute(): void {
    this.admission.clearAbuseMute();
    this.noteOns.reset();
    this.patches.reset();
    this.clockProbes.reset();
  }
}
