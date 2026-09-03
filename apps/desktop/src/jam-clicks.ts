import type { JamClick } from "./jam-clock.ts";
import {
  JAM_CALL_CUE_PENDING_MAX,
  JAM_LIMITER_ATTACK_SECONDS,
  JAM_LIMITER_KNEE_DB,
  JAM_LIMITER_RATIO,
  JAM_LIMITER_RELEASE_SECONDS,
  JAM_LIMITER_THRESHOLD_DB,
} from "./jam-contract.ts";

type ScheduledClick = { oscillator: OscillatorNode; gain: GainNode };

/** Receiver-owned, cancellable metronome click pool outside the musical voice allocator. */
export class JamClickPlayer {
  private readonly scheduled = new Set<ScheduledClick>();

  play(context: AudioContext, click: JamClick): boolean {
    // A remote grid is not a user gesture. Never create a graph that waits inside a suspended
    // context and later emerges as a surprise burst.
    if (context.state !== "running") return false;
    let oscillator: OscillatorNode | null = null;
    let gain: GainNode | null = null;
    try {
      oscillator = context.createOscillator();
      gain = context.createGain();
      const entry: ScheduledClick = { oscillator, gain };
      oscillator.type = "square";
      oscillator.frequency.value = click.accent ? 1760 : 1175;
      gain.gain.setValueAtTime(0.0001, click.audioTime);
      gain.gain.exponentialRampToValueAtTime(click.accent ? 0.1 : 0.06, click.audioTime + 0.004);
      gain.gain.exponentialRampToValueAtTime(0.0001, click.audioTime + 0.05);
      oscillator.connect(gain).connect(context.destination);
      this.scheduled.add(entry);
      oscillator.onended = () => this.release(entry);
      oscillator.start(click.audioTime);
      oscillator.stop(click.audioTime + 0.06);
      return true;
    } catch {
      if (oscillator && gain) this.release({ oscillator, gain });
      else {
        try { oscillator?.disconnect(); } catch { /* partial graph */ }
        try { gain?.disconnect(); } catch { /* partial graph */ }
      }
      return false;
    }
  }

  cancel(context: AudioContext): void {
    for (const entry of [...this.scheduled]) {
      try { entry.oscillator.stop(context.currentTime); } catch { /* already ended */ }
      this.release(entry);
    }
  }

  pending(): number {
    return this.scheduled.size;
  }

  private release(entry: ScheduledClick): void {
    this.scheduled.delete(entry);
    try { entry.oscillator.disconnect(); } catch { /* already disconnected */ }
    try { entry.gain.disconnect(); } catch { /* already disconnected */ }
  }
}

/** Receiver-owned pool for short call UI cues; it never resumes or creates an AudioContext. */
export class JamCallCuePlayer {
  private readonly scheduled = new Set<ScheduledClick>();
  private output: { context: AudioContext; input: GainNode; limiter: DynamicsCompressorNode } | null = null;

  play(context: AudioContext, note: number): boolean {
    // Remote signalling can invoke this path. Only an already-running context admits nodes, so an
    // arrival message can neither manufacture a context nor queue audio behind browser suspension.
    if (
      context.state !== "running" || !Number.isInteger(note) || note < 0 || note > 127 ||
      this.scheduled.size >= JAM_CALL_CUE_PENDING_MAX
    ) return false;
    let oscillator: OscillatorNode | null = null;
    let gain: GainNode | null = null;
    try {
      const output = this.ensureOutput(context);
      oscillator = context.createOscillator();
      gain = context.createGain();
      const entry = { oscillator, gain };
      oscillator.type = "sine";
      oscillator.frequency.value = 440 * Math.pow(2, (note - 69) / 12);
      const at = context.currentTime;
      gain.gain.setValueAtTime(0.0001, at);
      gain.gain.exponentialRampToValueAtTime(0.1, at + 0.008);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + 0.16);
      // Every cue shares one receiver-owned limiter. A signalling burst can neither allocate past
      // the global overlap cap nor sum raw oscillators directly at the hardware destination.
      oscillator.connect(gain).connect(output);
      this.scheduled.add(entry);
      oscillator.onended = () => this.release(entry);
      oscillator.start(at);
      oscillator.stop(at + 0.18);
      return true;
    } catch {
      if (oscillator && gain) this.release({ oscillator, gain });
      else {
        try { oscillator?.disconnect(); } catch { /* partial graph */ }
        try { gain?.disconnect(); } catch { /* partial graph */ }
      }
      return false;
    }
  }

  cancel(context: AudioContext): void {
    for (const entry of [...this.scheduled]) {
      try { entry.oscillator.stop(context.currentTime); } catch { /* already ended */ }
      this.release(entry);
    }
    if (this.output?.context === context) {
      try { this.output.input.disconnect(); } catch { /* already disconnected */ }
      try { this.output.limiter.disconnect(); } catch { /* already disconnected */ }
      this.output = null;
    }
  }

  pending(): number {
    return this.scheduled.size;
  }

  private release(entry: ScheduledClick): void {
    this.scheduled.delete(entry);
    try { entry.oscillator.disconnect(); } catch { /* already disconnected */ }
    try { entry.gain.disconnect(); } catch { /* already disconnected */ }
  }

  private ensureOutput(context: AudioContext): GainNode {
    if (this.output?.context === context) return this.output.input;
    if (this.output) {
      try { this.output.input.disconnect(); } catch { /* replaced context */ }
      try { this.output.limiter.disconnect(); } catch { /* replaced context */ }
      this.output = null;
    }
    let input: GainNode | null = null;
    let limiter: DynamicsCompressorNode | null = null;
    try {
      input = context.createGain();
      input.gain.value = 0.7;
      limiter = context.createDynamicsCompressor();
      limiter.threshold.value = JAM_LIMITER_THRESHOLD_DB;
      limiter.knee.value = JAM_LIMITER_KNEE_DB;
      limiter.ratio.value = JAM_LIMITER_RATIO;
      limiter.attack.value = JAM_LIMITER_ATTACK_SECONDS;
      limiter.release.value = JAM_LIMITER_RELEASE_SECONDS;
      input.connect(limiter).connect(context.destination);
      this.output = { context, input, limiter };
      return input;
    } catch (error) {
      try { input?.disconnect(); } catch { /* partial graph */ }
      try { limiter?.disconnect(); } catch { /* partial graph */ }
      throw error;
    }
  }
}
