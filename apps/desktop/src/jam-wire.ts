import { JamPeerBudget, type JamBudgetDenial } from "./jam-budget.ts";
import {
  JAM_FRAME_MAX_BYTES,
  JAM_LEGACY_SESSION_NONCE,
  JAM_MET_BPB_MAX,
  JAM_MET_BPB_MIN,
  JAM_MET_BPM_MAX,
  JAM_MET_BPM_MIN,
  JAM_PATCH_ID_HEX_CHARS,
  JAM_SEQUENCE_MAX,
  JAM_SESSION_NONCE_HEX_CHARS,
  JAM_SMALL_FRAME_MAX_BYTES,
  type JamClockProbe,
  type JamClockReply,
  type JamDrumHit,
  type JamMetronome,
  type JamNoteOff,
  type JamNoteOn,
  type JamPatchAnnounce,
  type LegacyWave,
} from "./jam-contract.ts";
import { validateJamPatch } from "./jam-patch.ts";

export type JamWireMessage = JamNoteOn | JamNoteOff | JamDrumHit | JamPatchAnnounce |
  JamMetronome | JamClockProbe | JamClockReply;
export type JamFrameDecode =
  | Readonly<{ ok: true; kind: "jam"; message: JamWireMessage }>
  | Readonly<{
    ok: true;
    kind: "legacy-note";
    sessionNonce: typeof JAM_LEGACY_SESSION_NONCE;
    message: JamNoteOn | JamNoteOff;
  }>
  | Readonly<{ ok: true; kind: "other"; value: Record<string, unknown> }>
  | Readonly<{ ok: false; reason: JamBudgetDenial | "json" | "shape" | "small-frame" }>;

const encoder = new TextEncoder();

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function exact(value: Record<string, unknown>, required: readonly string[], optional: readonly string[] = []): boolean {
  const allowed = new Set([...required, ...optional]);
  const keys = Object.keys(value);
  return required.every((key) => Object.hasOwn(value, key)) && keys.every((key) => allowed.has(key));
}

function uint32(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value <= JAM_SEQUENCE_MAX;
}

function boundedInt(value: unknown, min: number, max: number): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= min && value <= max;
}

function timestamp(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= Number.MAX_SAFE_INTEGER;
}

function wave(value: unknown): value is LegacyWave {
  return value === "sine" || value === "triangle" || value === "square" || value === "sawtooth";
}

function nonce(value: unknown): value is string {
  return typeof value === "string" && value !== JAM_LEGACY_SESSION_NONCE &&
    new RegExp(`^[0-9a-f]{${JAM_SESSION_NONCE_HEX_CHARS}}$`).test(value);
}

function patchId(value: unknown): value is string {
  return typeof value === "string" && new RegExp(`^[0-9a-f]{${JAM_PATCH_ID_HEX_CHARS}}$`).test(value);
}

/**
 * Decode one authenticated peer's instrument-channel frame.
 *
 * The all-frame gate runs before JSON.parse. Returning `other` preserves the existing `t:"s"`
 * state path without teaching this module its unrelated fields; unknown messages are still paid
 * for and bounded, so extension negotiation cannot become a parse-rate bypass.
 */
export class JamFrameDecoder {
  readonly budget: JamPeerBudget;
  private legacySequence = 0;

  constructor(budget = new JamPeerBudget()) {
    this.budget = budget;
  }

  decode(raw: unknown, nowMs: number): JamFrameDecode {
    const admitted = this.budget.admitFrame(raw, nowMs);
    if (!admitted.ok) return admitted;
    let value: unknown;
    try { value = JSON.parse(admitted.raw); } catch { return { ok: false, reason: "json" }; }
    const message = record(value);
    if (!message || typeof message.t !== "string") return { ok: false, reason: "shape" };
    if (message.t !== "p" && encoder.encode(admitted.raw).byteLength > JAM_SMALL_FRAME_MAX_BYTES) {
      return { ok: false, reason: "small-frame" };
    }

    if (message.t === "n") return this.note(message, nowMs);
    if (message.t === "d") return this.drum(message, nowMs);
    if (message.t === "p") return this.patch(message, nowMs);
    if (message.t === "m") return this.metronome(message);
    if (message.t === "c") return this.clock(message, nowMs);
    return { ok: true, kind: "other", value: message };
  }

  private note(message: Record<string, unknown>, nowMs: number): JamFrameDecode {
    if (message.on === 1) {
      if (!this.budget.admitNoteOn(nowMs)) return { ok: false, reason: "note-rate" };
      if (
        exact(message, ["t", "on", "n", "w"]) && boundedInt(message.n, 0, 127) && wave(message.w)
      ) {
        const q = this.nextLegacySequence();
        if (q === null) return { ok: false, reason: "shape" };
        return {
          ok: true,
          kind: "legacy-note",
          sessionNonce: JAM_LEGACY_SESSION_NONCE,
          message: { t: "n", on: 1, n: message.n, w: message.w, q },
        };
      }
      if (
        !exact(message, ["t", "on", "n", "w", "q"], ["p"]) ||
        !boundedInt(message.n, 0, 127) || !wave(message.w) || !uint32(message.q) ||
        (message.p !== undefined && !patchId(message.p))
      ) return { ok: false, reason: "shape" };
      return { ok: true, kind: "jam", message: message as unknown as JamNoteOn };
    }
    if (message.on === 0 && exact(message, ["t", "on", "n"]) && boundedInt(message.n, 0, 127)) {
      const q = this.nextLegacySequence();
      if (q === null) return { ok: false, reason: "shape" };
      return {
        ok: true,
        kind: "legacy-note",
        sessionNonce: JAM_LEGACY_SESSION_NONCE,
        message: { t: "n", on: 0, n: message.n, q },
      };
    }
    if (
      message.on !== 0 || !exact(message, ["t", "on", "n", "q"]) ||
      !boundedInt(message.n, 0, 127) || !uint32(message.q)
    ) return { ok: false, reason: "shape" };
    return { ok: true, kind: "jam", message: message as unknown as JamNoteOff };
  }

  private drum(message: Record<string, unknown>, nowMs: number): JamFrameDecode {
    if (!this.budget.admitNoteOn(nowMs)) return { ok: false, reason: "note-rate" };
    if (!exact(message, ["t", "n", "q"]) || !boundedInt(message.n, 0, 9) || !uint32(message.q)) {
      return { ok: false, reason: "shape" };
    }
    return { ok: true, kind: "jam", message: message as unknown as JamDrumHit };
  }

  private patch(message: Record<string, unknown>, nowMs: number): JamFrameDecode {
    // Charge the rare subtype before descriptor validation and hashing work.
    if (!this.budget.admitPatch(nowMs)) return { ok: false, reason: "patch-rate" };
    if (
      !exact(message, ["t", "v", "id", "sn", "d"]) || message.v !== 1 ||
      !patchId(message.id) || !nonce(message.sn)
    ) return { ok: false, reason: "shape" };
    const checked = validateJamPatch(message.d);
    if (!checked.ok) return { ok: false, reason: "shape" };
    return {
      ok: true,
      kind: "jam",
      message: { t: "p", v: 1, id: message.id, sn: message.sn, d: checked.patch },
    };
  }

  private metronome(message: Record<string, unknown>): JamFrameDecode {
    if (
      !exact(message, ["t", "v", "sn", "on", "rev", "bpm", "bpb", "org"]) || message.v !== 1 ||
      !nonce(message.sn) || (message.on !== 0 && message.on !== 1) || !uint32(message.rev) ||
      !boundedInt(message.bpm, JAM_MET_BPM_MIN, JAM_MET_BPM_MAX) ||
      !boundedInt(message.bpb, JAM_MET_BPB_MIN, JAM_MET_BPB_MAX) || !timestamp(message.org)
    ) return { ok: false, reason: "shape" };
    return { ok: true, kind: "jam", message: message as unknown as JamMetronome };
  }

  private clock(message: Record<string, unknown>, nowMs: number): JamFrameDecode {
    if (exact(message, ["t", "q", "tx"]) && uint32(message.q) && timestamp(message.tx)) {
      if (!this.budget.admitClockProbe(nowMs)) return { ok: false, reason: "clock-rate" };
      return { ok: true, kind: "jam", message: message as unknown as JamClockProbe };
    }
    if (
      exact(message, ["t", "r", "tx", "rx"]) && uint32(message.r) &&
      timestamp(message.tx) && timestamp(message.rx)
    ) return { ok: true, kind: "jam", message: message as unknown as JamClockReply };
    return { ok: false, reason: "shape" };
  }

  private nextLegacySequence(): number | null {
    if (this.legacySequence > JAM_SEQUENCE_MAX) return null;
    const sequence = this.legacySequence;
    this.legacySequence += 1;
    return sequence;
  }
}

// Compile-time assertion: the pre-parse gate must remain at least as large as the per-type cap.
if (JAM_FRAME_MAX_BYTES < JAM_SMALL_FRAME_MAX_BYTES) throw new Error("invalid jam frame limits");
