import { JAM_HELD_PER_PEER, JAM_REMOTE_HOLD_MAX_MS, JAM_VOICES_GLOBAL } from "./jam-contract.ts";

export type JamVoicePhase = "held" | "tail";
export type JamVoiceEndReason = "stolen" | "source-reset" | "engine-dispose";

export type JamVoiceRequest = Readonly<{
  id: string;
  /** Lifecycle/render lane. Reconnect lanes for one performer remain independently removable. */
  source: string;
  /** Fairness identity. Archival reconnect lanes for one performer must share this owner. */
  owner?: string;
  phase: JamVoicePhase;
  startedAtMs: number;
  /** Must synchronously disconnect every node owned by the voice. */
  teardown: (reason: JamVoiceEndReason) => void;
}>;

type AllocatedVoice = {
  id: string;
  source: string;
  owner: string;
  phase: JamVoicePhase;
  startedAtMs: number;
  releasedAtMs: number | null;
  teardown: (reason: JamVoiceEndReason) => void;
};

export type JamAllocationDenial = "duplicate" | "source-held" | "room-held";
export type JamAllocation =
  | Readonly<{ ok: true; stolen: readonly string[] }>
  | Readonly<{ ok: false; reason: JamAllocationDenial }>;

/**
 * Fair, renderer-independent voice ownership.
 *
 * A new voice may evict a release tail, never a held note. It consumes its own source's oldest
 * tail first; only a room-wide ceiling may take another source's oldest tail. Removal happens
 * before the teardown callback, so a callback cannot observe or recursively remove a stale slot.
 */
export class JamVoiceAllocator {
  private readonly voices = new Map<string, AllocatedVoice>();

  allocate(request: JamVoiceRequest): JamAllocation {
    if (this.voices.has(request.id)) return { ok: false, reason: "duplicate" };
    const owner = request.owner ?? request.source;
    if (!request.id || !request.source || !owner || !Number.isFinite(request.startedAtMs)) {
      throw new TypeError("voice allocation needs a stable id, source, owner and monotonic start time");
    }
    if (request.phase !== "held" && request.phase !== "tail") throw new TypeError("invalid voice phase");
    if (request.phase === "held" && this.countOwner(owner, "held") >= JAM_HELD_PER_PEER) {
      return { ok: false, reason: "source-held" };
    }

    const stolen: string[] = [];
    if (this.voices.size >= JAM_VOICES_GLOBAL) {
      // Prefer the requester's own remaining tail. Rapid releases cannot make another performer
      // pay while the offender still owns something safe to reclaim.
      const roomTail = this.oldestTail(owner) ?? this.oldestTail();
      if (!roomTail) return { ok: false, reason: "room-held" };
      stolen.push(this.steal(roomTail));
    }

    this.voices.set(request.id, {
      ...request,
      owner,
      releasedAtMs: request.phase === "tail" ? request.startedAtMs : null,
    });
    return { ok: true, stolen };
  }

  /** Transition a held voice into a stealable release tail. */
  release(id: string, nowMs: number): boolean {
    const voice = this.voices.get(id);
    if (!voice || voice.phase === "tail" || !Number.isFinite(nowMs)) return false;
    voice.phase = "tail";
    voice.releasedAtMs = nowMs;
    return true;
  }

  /** Audio `ended` owns normal removal; no teardown callback is needed after nodes ended naturally. */
  finish(id: string): boolean {
    return this.voices.delete(id);
  }

  /** Held voices whose remote note-off never arrived. The engine schedules their bounded release. */
  expiredHeld(nowMs: number): string[] {
    if (!Number.isFinite(nowMs)) return [];
    return [...this.voices.values()]
      .filter((voice) => voice.phase === "held" && nowMs - voice.startedAtMs >= JAM_REMOTE_HOLD_MAX_MS)
      .sort(compareAge)
      .map((voice) => voice.id);
  }

  releaseSource(source: string): string[] {
    return this.removeMatching((voice) => voice.source === source, "source-reset");
  }

  dispose(): string[] {
    return this.removeMatching(() => true, "engine-dispose");
  }

  has(id: string): boolean {
    return this.voices.has(id);
  }

  snapshot(): readonly Readonly<Pick<AllocatedVoice, "id" | "source" | "owner" | "phase" | "startedAtMs" | "releasedAtMs">>[] {
    return [...this.voices.values()].map(({ teardown: _teardown, ...voice }) => ({ ...voice }));
  }

  private countOwner(owner: string, phase?: JamVoicePhase): number {
    let count = 0;
    for (const voice of this.voices.values()) {
      if (voice.owner === owner && (!phase || voice.phase === phase)) count += 1;
    }
    return count;
  }

  private oldestTail(owner?: string): AllocatedVoice | null {
    return [...this.voices.values()]
      .filter((voice) => voice.phase === "tail" && (!owner || voice.owner === owner))
      .sort(compareAge)[0] ?? null;
  }

  private steal(voice: AllocatedVoice): string {
    this.voices.delete(voice.id);
    try { voice.teardown("stolen"); } catch { /* ownership is already gone; keep reclaiming */ }
    return voice.id;
  }

  private removeMatching(predicate: (voice: AllocatedVoice) => boolean, reason: JamVoiceEndReason): string[] {
    const removed: string[] = [];
    for (const voice of [...this.voices.values()].sort(compareAge)) {
      if (!predicate(voice)) continue;
      this.voices.delete(voice.id);
      try { voice.teardown(reason); } catch { /* one faulty graph must not strand its siblings */ }
      removed.push(voice.id);
    }
    return removed;
  }
}

function compareAge(a: AllocatedVoice, b: AllocatedVoice): number {
  const aAge = a.releasedAtMs ?? a.startedAtMs;
  const bAge = b.releasedAtMs ?? b.startedAtMs;
  return aAge - bAge || a.id.localeCompare(b.id);
}
