import {
  JAM_CLOCK_RTT_MAX_MS,
  JAM_CLOCK_SAMPLE_MAX,
  JAM_CLOCK_PROBE_BURST,
  JAM_CLOCK_PROBE_RATE,
  JAM_MET_CLICKS_PER_PASS_MAX,
  JAM_MET_LOOKAHEAD_MS,
  JAM_MET_OFFSET_JUMP_MAX_MS,
  JAM_MET_REV_MIN_INTERVAL_MS,
  JAM_SEQUENCE_MAX,
  type JamClockProbe,
  type JamClockReply,
  type JamMetronome,
} from "./jam-contract.ts";
import { JamTokenBucket } from "./jam-budget.ts";
import { JamSourceChannelRegistry, type JamSourceChannel } from "./jam-channel.ts";

export type JamClockSample = Readonly<{
  /** Probe transmit and reply receive in this receiver's monotonic clock. */
  localSentMs: number;
  localReceivedMs: number;
  /** Probe receive and reply transmit in the anchor's monotonic clock. */
  remoteReceivedMs: number;
  remoteSentMs: number;
}>;

export type JamClockEstimate = Readonly<{ offsetMs: number; rttMs: number }>;

function finiteTime(value: number): boolean {
  return Number.isFinite(value) && value >= 0 && value <= Number.MAX_SAFE_INTEGER;
}

/** NTP four-timestamp estimate; offset is `remote clock - local clock`. */
export function estimateJamClock(sample: JamClockSample): JamClockEstimate | null {
  const { localSentMs: t0, remoteReceivedMs: t1, remoteSentMs: t2, localReceivedMs: t3 } = sample;
  if (![t0, t1, t2, t3].every(finiteTime) || t3 < t0 || t2 < t1) return null;
  const rttMs = (t3 - t0) - (t2 - t1);
  if (rttMs < 0 || rttMs > JAM_CLOCK_RTT_MAX_MS) return null;
  const offsetMs = ((t1 - t0) + (t2 - t3)) / 2;
  return Number.isFinite(offsetMs) ? { offsetMs, rttMs } : null;
}

/**
 * Bounded best-RTT clock estimator. Arbitrary initial origins are valid; an implausible jump away
 * from an established estimate marks sync unsafe instead of slewing scheduled audio wildly.
 */
export class JamClockSync {
  private samples: JamClockEstimate[] = [];
  private estimate: JamClockEstimate | null = null;
  private synced = false;

  add(sample: JamClockSample): JamClockEstimate | null {
    const candidate = estimateJamClock(sample);
    if (!candidate) return null;
    if (this.estimate && Math.abs(candidate.offsetMs - this.estimate.offsetMs) > JAM_MET_OFFSET_JUMP_MAX_MS) {
      this.synced = false;
      return null;
    }
    this.samples.push(candidate);
    // Age samples by receipt order. Keeping the all-time lowest RTT forever would freeze a stale
    // offset and prevent ordinary clock drift from ever replacing it.
    this.samples = this.samples.slice(-JAM_CLOCK_SAMPLE_MAX);
    // Lowest RTT has the smallest unknown one-way-delay error and is the normal NTP clock-filter
    // choice. Keeping the other bounded samples lets a later lower-RTT exchange replace it.
    this.estimate = [...this.samples].sort((a, b) => a.rttMs - b.rttMs || a.offsetMs - b.offsetMs)[0] ?? null;
    this.synced = this.estimate !== null;
    return this.estimate;
  }

  isSynced(): boolean {
    return this.synced;
  }

  offsetMs(): number | null {
    return this.synced ? this.estimate?.offsetMs ?? null : null;
  }

  remoteToLocal(remoteMs: number): number | null {
    const offset = this.offsetMs();
    return offset === null || !finiteTime(remoteMs) ? null : remoteMs - offset;
  }

  reset(): void {
    this.samples = [];
    this.estimate = null;
    this.synced = false;
  }
}

type OutstandingProbe = { tx: number };

/** Rate-limited probe issuance plus exact reply correlation; unsolicited replies do no work. */
export class JamClockProbeTracker {
  private readonly bucket = new JamTokenBucket(JAM_CLOCK_PROBE_BURST, JAM_CLOCK_PROBE_RATE);
  private readonly outstanding = new Map<number, OutstandingProbe>();
  private nextSequence = 0;

  issue(nowMs: number): JamClockProbe | null {
    if (!finiteTime(nowMs)) return null;
    this.prune(nowMs);
    if (this.outstanding.size >= JAM_CLOCK_PROBE_BURST || !this.bucket.charge(nowMs)) return null;
    let q = this.nextSequence;
    while (this.outstanding.has(q)) q = q === JAM_SEQUENCE_MAX ? 0 : q + 1;
    this.nextSequence = q === JAM_SEQUENCE_MAX ? 0 : q + 1;
    this.outstanding.set(q, { tx: nowMs });
    return { t: "c", q, tx: nowMs };
  }

  accept(reply: JamClockReply, localReceivedMs: number): JamClockSample | null {
    if (!finiteTime(localReceivedMs) || !Number.isInteger(reply.r) || !finiteTime(reply.tx) || !finiteTime(reply.rx)) {
      return null;
    }
    this.prune(localReceivedMs);
    const probe = this.outstanding.get(reply.r);
    if (!probe || reply.tx !== probe.tx || localReceivedMs - probe.tx > JAM_CLOCK_RTT_MAX_MS) return null;
    this.outstanding.delete(reply.r);
    // The compact reply records the remote receive timestamp. Reply construction is immediate, so
    // v1 intentionally uses it as both t1 and t2 rather than inventing an unsent timestamp.
    return {
      localSentMs: probe.tx,
      localReceivedMs,
      remoteReceivedMs: reply.rx,
      remoteSentMs: reply.rx,
    };
  }

  reset(): void {
    this.outstanding.clear();
  }

  private prune(nowMs: number): void {
    for (const [q, probe] of this.outstanding) {
      if (nowMs - probe.tx > JAM_CLOCK_RTT_MAX_MS) this.outstanding.delete(q);
    }
  }
}

export type JamClick = Readonly<{ audioTime: number; beat: number; accent: boolean }>;

type ActiveMetronome = {
  source: string;
  sessionNonce: string;
  revision: number;
  bpm: number;
  beatsPerBar: number;
  remoteOriginMs: number;
  localFallbackOriginMs: number;
  acceptedAtMs: number;
  nextBeat: number;
};

export type MetronomeUpdate = "started" | "updated" | "stopped" | "foreign" | "stale" |
  "stale-channel" | "too-fast";

/**
 * Anchor ownership and audio-clock scheduling, with no timer dependency.
 *
 * A coarse UI timer calls `plan()`; all returned timestamps are against AudioContext.currentTime.
 * The 150 ms horizon survives background timer jitter without leaving seconds of uncancellable
 * clicks in the graph.
 */
export class JamMetronomeClock {
  private active: ActiveMetronome | null = null;
  private readonly sourceChannels: JamSourceChannelRegistry;

  constructor(sourceChannels: JamSourceChannelRegistry) {
    this.sourceChannels = sourceChannels;
  }

  receive(channel: JamSourceChannel, message: JamMetronome, localReceivedMs: number): MetronomeUpdate {
    if (!this.sourceChannels.isCurrent(channel)) return "stale-channel";
    const source = channel.source;
    if (!source || !finiteTime(localReceivedMs)) return "stale";
    const current = this.active;
    if (!current) {
      if (message.on === 0) return "stale";
      this.active = fromMessage(source, message, localReceivedMs);
      return "started";
    }
    if (current.source !== source || current.sessionNonce !== message.sn) return "foreign";
    if (message.rev <= current.revision) return "stale";
    if (localReceivedMs - current.acceptedAtMs < JAM_MET_REV_MIN_INTERVAL_MS) return "too-fast";
    if (message.on === 0) {
      this.active = null;
      return "stopped";
    }
    this.active = fromMessage(source, message, localReceivedMs);
    return "updated";
  }

  anchorLeft(source: string): boolean {
    if (this.active?.source !== source) return false;
    this.active = null;
    return true;
  }

  /**
   * Translate the shared beat origin into the audio clock once per scheduling pass. Callers pass
   * both clocks from the same wake-up, keeping performance/audio mapping in exactly one place.
   */
  plan(sync: JamClockSync, localPerformanceMs: number, audioNow: number): JamClick[] {
    const active = this.active;
    if (!active || !finiteTime(localPerformanceMs) || !Number.isFinite(audioNow) || audioNow < 0) return [];
    const localOrigin = sync.remoteToLocal(active.remoteOriginMs) ?? active.localFallbackOriginMs;
    const originAudio = audioNow + (localOrigin - localPerformanceMs) / 1_000;
    const beatSeconds = 60 / active.bpm;
    const horizon = audioNow + JAM_MET_LOOKAHEAD_MS / 1_000;
    const firstAudible = Math.max(0, Math.ceil((audioNow - originAudio) / beatSeconds - 1e-9));
    let beat = Math.max(active.nextBeat, firstAudible);
    const clicks: JamClick[] = [];
    while (clicks.length < JAM_MET_CLICKS_PER_PASS_MAX) {
      const audioTime = originAudio + beat * beatSeconds;
      if (audioTime > horizon) break;
      if (audioTime >= audioNow - 0.001) {
        clicks.push({ audioTime: Math.max(audioNow, audioTime), beat, accent: beat % active.beatsPerBar === 0 });
      }
      beat += 1;
    }
    active.nextBeat = beat;
    return clicks;
  }

  snapshot(): Readonly<Omit<ActiveMetronome, "nextBeat">> | null {
    if (!this.active) return null;
    const { nextBeat: _nextBeat, ...state } = this.active;
    return { ...state };
  }
}

function fromMessage(source: string, message: JamMetronome, localReceivedMs: number): ActiveMetronome {
  return {
    source,
    sessionNonce: message.sn,
    revision: message.rev,
    bpm: message.bpm,
    beatsPerBar: message.bpb,
    remoteOriginMs: message.org,
    // If clock estimation is unavailable, clicking from receipt time is explicitly local-only;
    // it never pretends the anchor's arbitrary performance-clock origin belongs to this process.
    localFallbackOriginMs: localReceivedMs,
    acceptedAtMs: localReceivedMs,
    nextBeat: 0,
  };
}
