import { JamVoiceAllocator, type JamAllocation, type JamVoiceEndReason } from "./jam-allocator.ts";
import { JamSourceChannelRegistry, type JamSourceChannel } from "./jam-channel.ts";
import {
  JAM_DRUM_SEED_DOMAIN,
  JAM_DRUM_NOISE_PERIOD_SAMPLES,
  JAM_DRUM_TAIL_MAX_MS,
  JAM_EFFECT_SEND_MAX_GAIN,
  JAM_FILTER_ENV_MAX_OCTAVES,
  JAM_FILTER_NYQUIST_RATIO,
  JAM_FILTER_Q_MAX,
  JAM_FILTER_Q_MIN,
  JAM_HELD_PER_PEER,
  JAM_KIT,
  JAM_LEGACY_SESSION_NONCE,
  JAM_LFO_CUTOFF_MAX_OCTAVES,
  JAM_LIMITER_ATTACK_SECONDS,
  JAM_LIMITER_KNEE_DB,
  JAM_LIMITER_RATIO,
  JAM_LIMITER_RELEASE_SECONDS,
  JAM_LIMITER_THRESHOLD_DB,
  JAM_MASTER_GAIN,
  JAM_PATCH_CACHE_PER_PEER,
  JAM_PATCH_ID_HEX_CHARS,
  JAM_RELEASE_CAP_MS,
  JAM_REMOTE_HOLD_MAX_MS,
  JAM_SEQUENCE_MAX,
  JAM_SESSION_NONCE_HEX_CHARS,
  JAM_VOICE_PEAK_GAIN,
  PATCH_FILTER_MODES,
  PATCH_LFO_DESTS,
  PATCH_LFO_PITCH_DEPTH_CENTS,
  PATCH_OSC_WAVES,
  type JamPatch,
  type LegacyWave,
} from "./jam-contract.ts";
import { jamPatchId, legacyJamPatch, validateJamPatch } from "./jam-patch.ts";

export type JamSequenceResult = "first" | "next" | "gap" | "duplicate" | "no-session";
export type JamPatchInstallResult = "installed" | "cached" | "invalid-session" | "invalid-patch" |
  "hash-mismatch" | "stale-channel" | "disposed";

export type JamNoteInput = Readonly<{
  channel: JamSourceChannel;
  sequence: number;
  note: number;
  wave: LegacyWave;
  patchId?: string;
  remote?: boolean;
}>;

export type JamNoteOffInput = Readonly<{
  channel: JamSourceChannel;
  sequence: number;
  note: number;
}>;

export type JamDrumInput = Readonly<{
  callId: string;
  channel: JamSourceChannel;
  sessionNonce: string;
  sequence: number;
  pad: number;
  remote?: boolean;
}>;

export type JamDrumSeedInput = Readonly<{
  callId: string;
  source: string;
  sessionNonce: string;
  sequence: number;
  pad: number;
}>;

export type JamPlayResult = Readonly<{
  ok: boolean;
  reason?: "invalid" | "channel" | "sequence" | "muted" | "duplicate" | "source-held" | "room-held" | "audio";
  sequence?: JamSequenceResult;
  usedPatch?: string;
  stolen?: readonly string[];
}>;

type SourceState = {
  sessionNonce: string | null;
  lastSequence: number | null;
  patches: Map<string, JamPatch>;
  muted: boolean;
  /** Receiver-owned linear gain applied equally to dry and every room send. */
  level: number;
};

type SourceBus = {
  dry: GainNode;
  chorus: GainNode;
  delay: GainNode;
  reverb: GainNode;
};

type VoiceRuntime = {
  id: string;
  source: string;
  remote: boolean;
  customPatch: boolean;
  kind: "note" | "drum";
  note: number;
  pad: number | null;
  output: GainNode;
  nodes: AudioNode[];
  sources: AudioScheduledSourceNode[];
  params: AudioParam[];
  released: boolean;
  finished: boolean;
  release: (seconds?: number) => void;
  teardown: (reason: JamVoiceEndReason) => void;
};

type RoomGraph = {
  dry: GainNode;
  chorus: GainNode;
  delay: GainNode;
  reverb: GainNode;
  master: GainNode;
  limiter: DynamicsCompressorNode;
  nodes: AudioNode[];
  sources: AudioScheduledSourceNode[];
};

/** MIDI note to cycles/second; transposition happens before this conversion. */
function noteHz(note: number): number {
  return 440 * Math.pow(2, (note - 69) / 12);
}

function validSequence(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= JAM_SEQUENCE_MAX;
}

function validNonce(value: string): boolean {
  return new RegExp(`^[0-9a-f]{${JAM_SESSION_NONCE_HEX_CHARS}}$`).test(value);
}

function validWireNonce(value: string): boolean {
  return value !== JAM_LEGACY_SESSION_NONCE && validNonce(value);
}

function setGain(node: GainNode, value: number, at: number): void {
  try {
    node.gain.cancelScheduledValues(at);
    node.gain.setValueAtTime(value, at);
  } catch {
    node.gain.value = value;
  }
}

function holdAndCancel(param: AudioParam, at: number): void {
  try {
    if (typeof param.cancelAndHoldAtTime === "function") param.cancelAndHoldAtTime(at);
    else {
      const value = param.value;
      param.cancelScheduledValues(at);
      param.setValueAtTime(value, at);
    }
  } catch {
    // A node that already ended is about to be disconnected; no later automation can escape it.
  }
}

function stopSource(source: AudioScheduledSourceNode, at: number): void {
  try { source.stop(at); } catch { /* already stopped */ }
}

function disconnect(node: AudioNode): void {
  try { node.disconnect(); } catch { /* already disconnected */ }
}

function makeGain(ctx: AudioContext, value: number): GainNode {
  const gain = ctx.createGain();
  gain.gain.value = value;
  return gain;
}

function trackedGain(ctx: AudioContext, nodes: AudioNode[], value: number): GainNode {
  const gain = makeGain(ctx, value);
  nodes.push(gain);
  return gain;
}

/**
 * Receiver-owned Web Audio engine for the in-call instrument channel.
 *
 * App.svelte supplies authenticated source identities and already-bounded wire events. This class
 * owns every node after that seam: patch cache, voices, peer gates, room effects, master limiter,
 * watchdog releases and deterministic drum synthesis. No remote value can connect a new edge.
 */
export class JamEngine {
  readonly context: AudioContext;
  readonly sourceChannels: JamSourceChannelRegistry;
  private readonly allocator = new JamVoiceAllocator();
  private readonly room: RoomGraph;
  private readonly states = new Map<string, SourceState>();
  private readonly patchInstallQueues = new Map<string, Promise<void>>();
  private readonly buses = new Map<string, SourceBus>();
  private readonly voices = new Map<string, VoiceRuntime>();
  private legacyOnly = false;
  private deafened = false;
  private disposed = false;

  constructor(
    context: AudioContext,
    nowMs: () => number = () => performance.now(),
    destination: AudioNode = context.destination,
    sourceChannels = new JamSourceChannelRegistry(),
  ) {
    this.context = context;
    this.nowMs = nowMs;
    this.sourceChannels = sourceChannels;
    this.room = createRoomGraph(context, destination);
  }

  private readonly nowMs: () => number;

  /** Mint the capability an exact authenticated data-channel callback must close over. */
  openSource(source: string): JamSourceChannel {
    if (this.disposed) throw new Error("jam engine is disposed");
    if (this.sourceChannels.current(source)) {
      this.releaseSource(source);
      const state = this.states.get(source);
      if (state) state.lastSequence = null;
    }
    return this.sourceChannels.open(source);
  }

  /** A new sender nonce starts a new replay/sequence and patch-cache domain. */
  beginSourceSession(channel: JamSourceChannel, sessionNonce: string): boolean {
    if (!validWireNonce(sessionNonce)) return false;
    return this.activateSourceSession(channel, sessionNonce);
  }

  /** Explicit receive-only compatibility domain for an exact pre-v2 note channel. */
  beginLegacySourceSession(channel: JamSourceChannel): boolean {
    return this.activateSourceSession(channel, JAM_LEGACY_SESSION_NONCE);
  }

  private activateSourceSession(channel: JamSourceChannel, sessionNonce: string): boolean {
    if (this.disposed || !this.sourceChannels.isCurrent(channel) || !validNonce(sessionNonce)) return false;
    const source = channel.source;
    const state = this.state(source);
    if (state.sessionNonce === sessionNonce) return true;
    this.resetSource(source, true);
    state.sessionNonce = sessionNonce;
    return true;
  }

  installPatch(channel: JamSourceChannel, sessionNonce: string, id: string, candidate: unknown): Promise<JamPatchInstallResult> {
    if (this.disposed) return Promise.resolve("disposed");
    if (!this.sourceChannels.isCurrent(channel)) return Promise.resolve("stale-channel");
    const source = channel.source;
    // Hash announcements serially in ordered-channel receipt order. This avoids both completion
    // rollback and the opposite bug where a later rejected announce cancels an earlier valid one.
    const previous = this.patchInstallQueues.get(source) ?? Promise.resolve();
    const operation = previous.then(() => this.installPatchNow(channel, sessionNonce, id, candidate));
    const tail = operation.then(() => undefined, () => undefined);
    this.patchInstallQueues.set(source, tail);
    void tail.then(() => {
      if (this.patchInstallQueues.get(source) === tail) this.patchInstallQueues.delete(source);
    });
    return operation;
  }

  private async installPatchNow(
    channel: JamSourceChannel,
    sessionNonce: string,
    id: string,
    candidate: unknown,
  ): Promise<JamPatchInstallResult> {
    if (this.disposed) return "disposed";
    if (!this.sourceChannels.isCurrent(channel)) return "stale-channel";
    const source = channel.source;
    if (!validWireNonce(sessionNonce)) return "invalid-session";
    if (!new RegExp(`^[0-9a-f]{${JAM_PATCH_ID_HEX_CHARS}}$`).test(id)) return "hash-mismatch";
    const checked = validateJamPatch(candidate);
    if (!checked.ok) return "invalid-patch";
    let actualId: string;
    try { actualId = await jamPatchId(checked.patch); } catch { return "invalid-patch"; }

    if (this.disposed) return "disposed";
    if (!this.sourceChannels.isCurrent(channel)) return "stale-channel";
    const current = this.state(source);
    if (actualId !== id) {
      // With no prior sender state, the authenticated peer may still play the bounded legacy `w`
      // fallback. An invalid nonce transition must never reset an already-established session.
      if (current.sessionNonce === null) this.beginSourceSession(channel, sessionNonce);
      return "hash-mismatch";
    }
    if (!this.beginSourceSession(channel, sessionNonce)) return "invalid-session";

    const cache = this.state(source).patches;
    if (cache.has(id)) {
      // Map insertion order is the LRU order; touching a valid immutable entry moves it newest.
      const existing = cache.get(id)!;
      cache.delete(id);
      cache.set(id, existing);
      return "cached";
    }
    cache.set(id, checked.patch);
    while (cache.size > JAM_PATCH_CACHE_PER_PEER) cache.delete(cache.keys().next().value!);
    return "installed";
  }

  noteOn(input: JamNoteInput): JamPlayResult {
    if (this.disposed || !isMidiNote(input.note) || !isLegacyWave(input.wave) || !validSequence(input.sequence)) {
      return { ok: false, reason: "invalid" };
    }
    if (!this.sourceChannels.isCurrent(input.channel)) return { ok: false, reason: "channel" };
    const source = input.channel.source;
    const sequence = this.acceptSequence(source, input.sequence);
    if (sequence === "duplicate" || sequence === "no-session") return { ok: false, reason: "sequence", sequence };
    if (this.state(source).muted || (this.deafened && input.remote !== false)) {
      return { ok: false, reason: "muted", sequence };
    }
    // Remote input is not a user gesture and must never queue a surprise sound in a suspended
    // context. The UI resumes audio from a local gesture before admitting remote rendering.
    if (input.remote !== false && this.context.state !== "running") {
      return { ok: false, reason: "audio", sequence };
    }
    if (this.latestHeldNote(source, input.note)) return { ok: false, reason: "duplicate", sequence };

    const custom = !this.legacyOnly && input.patchId
      ? this.touchPatch(source, input.patchId)
      : null;
    const patch = custom ?? legacyJamPatch(input.wave);
    const id = noteVoiceId(source, input.note, input.sequence);
    let runtime: VoiceRuntime | null = null;
    const allocation = this.allocator.allocate({
      id,
      source,
      phase: "held",
      startedAtMs: this.nowMs(),
      teardown: (reason) => runtime?.teardown(reason),
    });
    if (!allocation.ok) return { ok: false, reason: allocation.reason, sequence };

    try {
      runtime = this.createPatchVoice(id, input, patch, !!custom);
      this.voices.set(id, runtime);
      return { ok: true, sequence, usedPatch: custom ? input.patchId : undefined, stolen: allocation.stolen };
    } catch {
      this.allocator.finish(id);
      runtime?.teardown("stolen");
      return { ok: false, reason: "audio", sequence, stolen: allocation.stolen };
    }
  }

  noteOff(input: JamNoteOffInput): JamPlayResult {
    if (this.disposed || !isMidiNote(input.note) || !validSequence(input.sequence)) return { ok: false, reason: "invalid" };
    if (!this.sourceChannels.isCurrent(input.channel)) return { ok: false, reason: "channel" };
    const source = input.channel.source;
    const sequence = this.acceptSequence(source, input.sequence);
    if (sequence === "duplicate" || sequence === "no-session") return { ok: false, reason: "sequence", sequence };
    const voice = this.latestHeldNote(source, input.note);
    const id = voice?.id;
    if (!voice || !id || !this.allocator.release(id, this.nowMs())) return { ok: true, sequence };
    voice.release();
    return { ok: true, sequence };
  }

  async drum(input: JamDrumInput): Promise<JamPlayResult> {
    if (
      this.disposed || !Number.isInteger(input.pad) || input.pad < 0 || input.pad >= JAM_KIT.length ||
      !validSequence(input.sequence) || !validWireNonce(input.sessionNonce)
    ) return { ok: false, reason: "invalid" };
    if (!this.sourceChannels.isCurrent(input.channel)) return { ok: false, reason: "channel" };
    const source = input.channel.source;
    if (!this.beginSourceSession(input.channel, input.sessionNonce)) return { ok: false, reason: "invalid" };
    const sequence = this.acceptSequence(source, input.sequence);
    if (sequence === "duplicate" || sequence === "no-session") return { ok: false, reason: "sequence", sequence };
    if (this.state(source).muted || (this.deafened && input.remote !== false)) {
      return { ok: false, reason: "muted", sequence };
    }
    if (input.remote !== false && this.context.state !== "running") {
      return { ok: false, reason: "audio", sequence };
    }

    const pad = JAM_KIT[input.pad];
    for (const voice of [...this.voices.values()]) {
      if (voice.source === source && voice.pad !== null && pad.chokes.includes(voice.pad)) voice.release(0.02);
    }
    // Include the opaque channel generation in the slot identity. A reconnect may legitimately
    // reuse both the sender nonce and sequence, and the seed digest below yields to the event loop.
    // Generation-scoped IDs prevent stale cleanup from touching the replacement channel's slot.
    const id = drumVoiceId(input.channel, input.sessionNonce, input.sequence);
    let runtime: VoiceRuntime | null = null;
    const allocation = this.allocator.allocate({
      id,
      source,
      phase: "tail",
      startedAtMs: this.nowMs(),
      teardown: (reason) => runtime?.teardown(reason),
    });
    if (!allocation.ok) return { ok: false, reason: allocation.reason, sequence };

    try {
      const seed = await drumSeed({ ...input, source });
      // The await gives leave/reconnect or a session transition a chance to invalidate this event.
      // Recheck every provenance boundary before creating nodes: allocator presence alone is not a
      // capability and an older channel could otherwise resurrect sound after being replaced.
      if (this.disposed) return { ok: false, reason: "muted", sequence };
      if (!this.sourceChannels.isCurrent(input.channel)) {
        this.allocator.finish(id);
        return { ok: false, reason: "channel", sequence };
      }
      if (this.state(source).sessionNonce !== input.sessionNonce) {
        this.allocator.finish(id);
        return { ok: false, reason: "sequence", sequence };
      }
      if (!this.allocator.has(id)) return { ok: false, reason: "muted", sequence };
      runtime = this.createDrumVoice(id, input, seed);
      this.voices.set(id, runtime);
      return { ok: true, sequence, stolen: allocation.stolen };
    } catch {
      this.allocator.finish(id);
      runtime?.teardown("stolen");
      return { ok: false, reason: "audio", sequence, stolen: allocation.stolen };
    }
  }

  /** Purely receiver-side; no state message is emitted or implied. */
  setLegacyOnly(enabled: boolean): void {
    if (this.legacyOnly === enabled) return;
    this.legacyOnly = enabled;
    if (!enabled) return;
    for (const voice of [...this.voices.values()]) {
      if (voice.remote && voice.customPatch) this.endImmediately(voice.id, "source-reset");
    }
  }

  setSourceMuted(source: string, muted: boolean): void {
    if (this.disposed) return;
    const state = this.state(source);
    state.muted = muted;
    const bus = this.buses.get(source);
    if (bus) for (const gate of Object.values(bus)) setGain(gate, muted ? 0 : state.level, this.context.currentTime);
    if (muted) this.releaseSource(source);
  }

  /**
   * Set a receiver-owned source level without changing sender state or the shared master.
   *
   * The jukebox uses this for synthetic take lanes: ordinary media and event-log playback then
   * obey the same local volume control without allowing a take to raise the global limiter input
   * above its validated 0..1 range. Invalid non-finite values fail closed and preserve the prior
   * level; muting remains authoritative over the stored level.
   */
  setSourceLevel(source: string, level: number): boolean {
    if (this.disposed || !Number.isFinite(level)) return false;
    const state = this.state(source);
    state.level = Math.min(1, Math.max(0, level));
    const bus = this.buses.get(source);
    if (bus) {
      const audibleLevel = state.muted ? 0 : state.level;
      for (const gate of Object.values(bus)) setGain(gate, audibleLevel, this.context.currentTime);
    }
    return true;
  }

  /** Deafen is a hard safety gate and also forgets every remote held state. */
  setDeafened(enabled: boolean): void {
    this.deafened = enabled;
    setGain(this.room.master, enabled ? 0 : JAM_MASTER_GAIN, this.context.currentTime);
    if (!enabled) return;
    const remoteSources = new Set([...this.voices.values()].filter((voice) => voice.remote).map((voice) => voice.source));
    for (const source of remoteSources) this.releaseSource(source);
  }

  releaseSource(source: string): void {
    this.allocator.releaseSource(source);
    // Allocator callbacks normally delete runtimes; this is a defensive sweep for a partially
    // constructed browser node graph whose callback never became reachable.
    for (const voice of [...this.voices.values()]) {
      if (voice.source === source) voice.teardown("source-reset");
    }
  }

  removeSource(source: string): void {
    this.sourceChannels.close(source);
    this.releaseSource(source);
    this.states.delete(source);
    const bus = this.buses.get(source);
    if (bus) for (const node of Object.values(bus)) disconnect(node);
    this.buses.delete(source);
  }

  /**
   * Remove a source only when the caller still owns its exact channel generation.
   *
   * Async take setup can finish after a replacement playback has opened the same synthetic
   * source name. Cleaning up by name in that case would tear down the replacement. The opaque
   * channel is therefore the authority for lifecycle cleanup, just as it is for note events.
   */
  removeChannel(channel: JamSourceChannel): boolean {
    if (this.disposed || !this.sourceChannels.isCurrent(channel)) return false;
    this.removeSource(channel.source);
    return true;
  }

  /** Timer-driven bookkeeping backup; audio-time watchdog automation remains authoritative. */
  sweepWatchdogs(nowMs = this.nowMs()): string[] {
    const expired = this.allocator.expiredHeld(nowMs);
    for (const id of expired) {
      if (this.allocator.release(id, nowMs)) this.voices.get(id)?.release();
    }
    return expired;
  }

  snapshot(): Readonly<{
    voices: ReturnType<JamVoiceAllocator["snapshot"]>;
    deafened: boolean;
    legacyOnly: boolean;
    mutedSources: readonly string[];
  }> {
    return {
      voices: this.allocator.snapshot(),
      deafened: this.deafened,
      legacyOnly: this.legacyOnly,
      mutedSources: [...this.states].filter(([, state]) => state.muted).map(([source]) => source),
    };
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.allocator.dispose();
    for (const voice of [...this.voices.values()]) voice.teardown("engine-dispose");
    for (const bus of this.buses.values()) for (const node of Object.values(bus)) disconnect(node);
    for (const source of this.room.sources) stopSource(source, this.context.currentTime);
    for (const node of this.room.nodes) disconnect(node);
    this.voices.clear();
    this.buses.clear();
    this.states.clear();
    this.patchInstallQueues.clear();
    this.sourceChannels.clear();
  }

  private state(source: string): SourceState {
    let state = this.states.get(source);
    if (!state) {
      state = { sessionNonce: null, lastSequence: null, patches: new Map(), muted: false, level: 1 };
      this.states.set(source, state);
    }
    return state;
  }

  private resetSource(source: string, clearPatches: boolean): void {
    this.releaseSource(source);
    const state = this.state(source);
    state.lastSequence = null;
    if (clearPatches) state.patches.clear();
  }

  private acceptSequence(source: string, sequence: number): JamSequenceResult {
    const state = this.state(source);
    if (!state.sessionNonce) return "no-session";
    const previous = state.lastSequence;
    if (previous === null) {
      state.lastSequence = sequence;
      return "first";
    }
    if (sequence <= previous) return "duplicate";
    state.lastSequence = sequence;
    return sequence === previous + 1 ? "next" : "gap";
  }

  private touchPatch(source: string, id: string): JamPatch | null {
    const cache = this.state(source).patches;
    const patch = cache.get(id);
    if (!patch) return null;
    cache.delete(id);
    cache.set(id, patch);
    return patch;
  }

  private latestHeldNote(source: string, note: number): VoiceRuntime | null {
    let found: VoiceRuntime | null = null;
    // Map insertion order makes the last matching held runtime the most recent re-press. An old
    // watchdog/release tail for the same pitch must not block or consume the new key-up.
    for (const voice of this.voices.values()) {
      if (voice.source === source && voice.kind === "note" && voice.note === note && !voice.released) found = voice;
    }
    return found;
  }

  private sourceBus(source: string): SourceBus {
    let bus = this.buses.get(source);
    if (bus) return bus;
    const state = this.state(source);
    const nodes: AudioNode[] = [];
    try {
      bus = {
        dry: trackedGain(this.context, nodes, state.muted ? 0 : state.level),
        chorus: trackedGain(this.context, nodes, state.muted ? 0 : state.level),
        delay: trackedGain(this.context, nodes, state.muted ? 0 : state.level),
        reverb: trackedGain(this.context, nodes, state.muted ? 0 : state.level),
      };
      bus.dry.connect(this.room.dry);
      bus.chorus.connect(this.room.chorus);
      bus.delay.connect(this.room.delay);
      bus.reverb.connect(this.room.reverb);
      this.buses.set(source, bus);
      return bus;
    } catch (error) {
      for (const node of nodes) disconnect(node);
      throw error;
    }
  }

  private createPatchVoice(id: string, input: JamNoteInput, patch: JamPatch, customPatch: boolean): VoiceRuntime {
    const ctx = this.context;
    if (ctx.state === "suspended") void ctx.resume().catch(() => {});
    const at = ctx.currentTime;
    const sourceName = input.channel.source;
    const bus = this.sourceBus(sourceName);
    const nodes: AudioNode[] = [];
    const sources: AudioScheduledSourceNode[] = [];
    const params: AudioParam[] = [];
    try {
    const output = trackedGain(ctx, nodes, 1);
    params.push(output.gain);
    const filter = ctx.createBiquadFilter();
    nodes.push(filter);
    params.push(filter.frequency, filter.Q, filter.detune);
    filter.type = PATCH_FILTER_MODES[patch.f.m];
    filter.frequency.setValueAtTime(Math.min(patch.f.c, ctx.sampleRate * JAM_FILTER_NYQUIST_RATIO), at);
    filter.Q.setValueAtTime(JAM_FILTER_Q_MIN + (patch.f.q / 100) * (JAM_FILTER_Q_MAX - JAM_FILTER_Q_MIN), at);

    const attack = patch.e.a / 1_000;
    const decay = patch.e.d / 1_000;
    const releaseSeconds = Math.min(patch.e.r, JAM_RELEASE_CAP_MS) / 1_000;
    const sustain = patch.e.s / 100;
    output.gain.setValueAtTime(0, at);
    output.gain.linearRampToValueAtTime(JAM_VOICE_PEAK_GAIN, at + attack);
    output.gain.linearRampToValueAtTime(JAM_VOICE_PEAK_GAIN * sustain, at + attack + decay);

    const baseCutoff = Math.min(patch.f.c, ctx.sampleRate * JAM_FILTER_NYQUIST_RATIO);
    let envelopeCutoff = baseCutoff;
    const envOctaves = (patch.f.e / 100) * JAM_FILTER_ENV_MAX_OCTAVES;
    if (envOctaves !== 0) {
      const peak = Math.min(ctx.sampleRate * JAM_FILTER_NYQUIST_RATIO, Math.max(20, patch.f.c * Math.pow(2, envOctaves)));
      envelopeCutoff = peak;
      filter.frequency.exponentialRampToValueAtTime(peak, at + Math.max(attack, 0.001));
      filter.frequency.exponentialRampToValueAtTime(
        Math.min(patch.f.c, ctx.sampleRate * JAM_FILTER_NYQUIST_RATIO),
        at + Math.max(attack + decay, 0.002),
      );
    }

    const levelSum = patch.o.reduce((sum, osc) => sum + osc.l, 0);
    for (const descriptor of patch.o) {
      const oscillator = ctx.createOscillator();
      nodes.push(oscillator);
      sources.push(oscillator);
      params.push(oscillator.frequency, oscillator.detune);
      const level = trackedGain(ctx, nodes, levelSum > 0 ? descriptor.l / levelSum : 0);
      params.push(level.gain);
      oscillator.type = PATCH_OSC_WAVES[descriptor.w];
      oscillator.frequency.setValueAtTime(noteHz(input.note + descriptor.t), at);
      oscillator.detune.setValueAtTime(descriptor.c, at);
      oscillator.connect(level).connect(filter);
    }
    filter.connect(output);

    if (PATCH_LFO_DESTS[patch.l.t] !== "off" && patch.l.d > 0) {
      const lfo = ctx.createOscillator();
      nodes.push(lfo);
      sources.push(lfo);
      params.push(lfo.frequency, lfo.detune);
      const depth = trackedGain(ctx, nodes, 1);
      params.push(depth.gain);
      lfo.frequency.setValueAtTime(patch.l.r / 100, at);
      if (PATCH_LFO_DESTS[patch.l.t] === "cutoff") {
        const desired = (patch.l.d / 100) * JAM_LFO_CUTOFF_MAX_OCTAVES * 1_200;
        const minCutoff = Math.min(baseCutoff, envelopeCutoff);
        const maxCutoff = Math.max(baseCutoff, envelopeCutoff);
        const highHeadroom = Math.max(0, 1_200 * Math.log2((ctx.sampleRate * JAM_FILTER_NYQUIST_RATIO) / maxCutoff));
        const lowHeadroom = Math.max(0, 1_200 * Math.log2(minCutoff / 20));
        depth.gain.setValueAtTime(Math.min(desired, highHeadroom, lowHeadroom), at);
        lfo.connect(depth).connect(filter.detune);
      } else {
        depth.gain.setValueAtTime((patch.l.d / 100) * PATCH_LFO_PITCH_DEPTH_CENTS, at);
        lfo.connect(depth);
        for (const source of sources) if (isOscillator(source) && source !== lfo) depth.connect(source.detune);
      }
    }

    connectVoiceSends(ctx, output, bus, patch.x, nodes);
    const runtime = this.runtime({
      id,
      source: sourceName,
      remote: input.remote !== false,
      customPatch,
      kind: "note",
      note: input.note,
      pad: null,
      output,
      nodes,
      sources,
      params,
      releaseSeconds,
      hardHoldSeconds: JAM_REMOTE_HOLD_MAX_MS / 1_000,
      watchdogLevel: JAM_VOICE_PEAK_GAIN * sustain,
    });
    for (const source of sources) source.start(at);
    return runtime;
    } catch (error) {
      for (const source of sources) stopSource(source, ctx.currentTime);
      for (const param of params) holdAndCancel(param, ctx.currentTime);
      for (const node of nodes) disconnect(node);
      throw error;
    }
  }

  private createDrumVoice(id: string, input: JamDrumInput, seed: number): VoiceRuntime {
    const ctx = this.context;
    if (ctx.state === "suspended") void ctx.resume().catch(() => {});
    const at = ctx.currentTime;
    const pad = JAM_KIT[input.pad];
    const tailSeconds = Math.min(pad.tailMs, JAM_DRUM_TAIL_MAX_MS) / 1_000;
    const sourceName = input.channel.source;
    const bus = this.sourceBus(sourceName);
    const nodes: AudioNode[] = [];
    const sources: AudioScheduledSourceNode[] = [];
    const params: AudioParam[] = [];
    try {
    const output = trackedGain(ctx, nodes, 1);
    params.push(output.gain);
    const filter = ctx.createBiquadFilter();
    nodes.push(filter);
    params.push(filter.frequency, filter.Q, filter.detune);

    const noise = ctx.createBufferSource();
    nodes.push(noise);
    sources.push(noise);
    const buffer = ctx.createBuffer(1, JAM_DRUM_NOISE_PERIOD_SAMPLES, ctx.sampleRate);
    fillNoise(buffer.getChannelData(0), seed);
    noise.buffer = buffer;
    noise.loop = true;
    noise.connect(filter);

    configureDrumFilter(filter, input.pad, at);
    if (input.pad === 0 || input.pad === 6 || input.pad === 7 || input.pad === 1) {
      const tone = ctx.createOscillator();
      nodes.push(tone);
      sources.push(tone);
      params.push(tone.frequency, tone.detune);
      const toneGain = trackedGain(ctx, nodes, input.pad === 1 ? 0.25 : 0.7);
      params.push(toneGain.gain);
      const base = input.pad === 0 ? 52 : input.pad === 6 ? 92 : input.pad === 7 ? 135 : 180;
      tone.frequency.setValueAtTime(base * 1.8, at);
      tone.frequency.exponentialRampToValueAtTime(base, at + Math.min(0.12, tailSeconds / 2));
      tone.connect(toneGain).connect(output);
    }
    filter.connect(output);
    output.gain.setValueAtTime(JAM_VOICE_PEAK_GAIN, at);
    output.gain.exponentialRampToValueAtTime(0.0001, at + Math.max(0.01, tailSeconds));
    connectVoiceSends(ctx, output, bus, drumSends(input.pad), nodes);

    const runtime = this.runtime({
      id,
      source: sourceName,
      remote: input.remote !== false,
      customPatch: false,
      kind: "drum",
      note: input.pad,
      pad: input.pad,
      output,
      nodes,
      sources,
      params,
      releaseSeconds: 0.03,
      hardHoldSeconds: 0,
      oneShotSeconds: tailSeconds,
    });
    for (const source of sources) source.start(at);
    return runtime;
    } catch (error) {
      for (const source of sources) stopSource(source, ctx.currentTime);
      for (const param of params) holdAndCancel(param, ctx.currentTime);
      for (const node of nodes) disconnect(node);
      throw error;
    }
  }

  private runtime(input: {
    id: string;
    source: string;
    remote: boolean;
    customPatch: boolean;
    kind: "note" | "drum";
    note: number;
    pad: number | null;
    output: GainNode;
    nodes: AudioNode[];
    sources: AudioScheduledSourceNode[];
    params: AudioParam[];
    releaseSeconds: number;
    hardHoldSeconds: number;
    watchdogLevel?: number;
    oneShotSeconds?: number;
  }): VoiceRuntime {
    const at = this.context.currentTime;
    let ended = 0;
    let releaseTriggered = false;
    const stopAt = at + (input.oneShotSeconds ?? input.hardHoldSeconds + input.releaseSeconds);
    const releaseAt = input.oneShotSeconds === undefined ? at + input.hardHoldSeconds : at;
    if (input.oneShotSeconds === undefined) {
      input.output.gain.setValueAtTime(input.watchdogLevel ?? input.output.gain.value, releaseAt);
      input.output.gain.linearRampToValueAtTime(0, stopAt);
    }

    const runtime: VoiceRuntime = {
      ...input,
      released: input.kind === "drum",
      finished: false,
      release: (seconds = input.releaseSeconds) => {
        if (runtime.finished || releaseTriggered) return;
        releaseTriggered = true;
        runtime.released = true;
        const now = this.context.currentTime;
        // A late note-off may shorten the watchdog tail, never cancel it and extend the hard stop.
        const remainingHardWindow = Math.max(0, stopAt - now);
        const bounded = Math.max(0, Math.min(seconds, JAM_RELEASE_CAP_MS / 1_000, remainingHardWindow));
        holdAndCancel(input.output.gain, now);
        input.output.gain.linearRampToValueAtTime(0, now + bounded);
        for (const source of input.sources) stopSource(source, now + bounded + 0.005);
      },
      teardown: (_reason) => {
        if (runtime.finished) return;
        runtime.finished = true;
        const now = this.context.currentTime;
        for (const param of input.params) holdAndCancel(param, now);
        setGain(input.output, 0, now);
        for (const source of input.sources) stopSource(source, now);
        // Disconnection is deliberately synchronous: a stopped source remains connected until its
        // scheduled end otherwise, so stealing would not actually reclaim the graph immediately.
        for (const node of input.nodes) disconnect(node);
        this.voices.delete(input.id);
      },
    };

    for (const source of input.sources) {
      source.onended = () => {
        ended += 1;
        if (ended < input.sources.length || runtime.finished) return;
        runtime.finished = true;
        for (const node of input.nodes) disconnect(node);
        this.voices.delete(input.id);
        this.allocator.finish(input.id);
      };
      stopSource(source, stopAt + 0.005);
    }
    return runtime;
  }

  private endImmediately(id: string, reason: JamVoiceEndReason): void {
    const voice = this.voices.get(id);
    if (!voice) return;
    this.allocator.finish(id);
    voice.teardown(reason);
  }
}

function createRoomGraph(ctx: AudioContext, destination: AudioNode): RoomGraph {
  const nodes: AudioNode[] = [];
  const sources: AudioScheduledSourceNode[] = [];
  try {
  const dry = trackedGain(ctx, nodes, 1);
  const chorus = trackedGain(ctx, nodes, 1);
  const delay = trackedGain(ctx, nodes, 1);
  const reverb = trackedGain(ctx, nodes, 1);
  const master = trackedGain(ctx, nodes, JAM_MASTER_GAIN);
  const limiter = ctx.createDynamicsCompressor();
  nodes.push(limiter);
  limiter.threshold.value = JAM_LIMITER_THRESHOLD_DB;
  limiter.knee.value = JAM_LIMITER_KNEE_DB;
  limiter.ratio.value = JAM_LIMITER_RATIO;
  limiter.attack.value = JAM_LIMITER_ATTACK_SECONDS;
  limiter.release.value = JAM_LIMITER_RELEASE_SECONDS;
  dry.connect(master);

  const chorusDelay = ctx.createDelay(0.05);
  nodes.push(chorusDelay);
  const chorusWet = trackedGain(ctx, nodes, 0.32);
  const chorusLfo = ctx.createOscillator();
  nodes.push(chorusLfo);
  sources.push(chorusLfo);
  const chorusDepth = trackedGain(ctx, nodes, 0.0035);
  chorusDelay.delayTime.value = 0.018;
  chorusLfo.frequency.value = 0.27;
  chorusLfo.connect(chorusDepth).connect(chorusDelay.delayTime);
  chorus.connect(chorusDelay).connect(chorusWet).connect(master);
  chorusLfo.start();

  const echo = ctx.createDelay(0.75);
  nodes.push(echo);
  const echoWet = trackedGain(ctx, nodes, 0.28);
  const echoFeedback = trackedGain(ctx, nodes, 0.22);
  echo.delayTime.value = 0.28;
  delay.connect(echo);
  echo.connect(echoWet).connect(master);
  echo.connect(echoFeedback).connect(echo);

  for (const [seconds, level] of [[0.071, 0.18], [0.113, 0.14], [0.173, 0.1]] as const) {
    const tap = ctx.createDelay(0.25);
    nodes.push(tap);
    const wet = trackedGain(ctx, nodes, level);
    tap.delayTime.value = seconds;
    reverb.connect(tap).connect(wet).connect(master);
  }
  master.connect(limiter).connect(destination);
  return { dry, chorus, delay, reverb, master, limiter, nodes, sources };
  } catch (error) {
    // Constructor failure must not strand a partially connected room graph that no JamEngine
    // instance exists to dispose.
    for (const source of sources) stopSource(source, ctx.currentTime);
    for (const node of nodes) disconnect(node);
    throw error;
  }
}

function connectVoiceSends(
  ctx: AudioContext,
  output: GainNode,
  bus: SourceBus,
  sends: { c: number; d: number; r: number },
  nodes: AudioNode[],
): void {
  const dry = trackedGain(ctx, nodes, 1);
  const chorus = trackedGain(ctx, nodes, (sends.c / 100) * JAM_EFFECT_SEND_MAX_GAIN);
  const delay = trackedGain(ctx, nodes, (sends.d / 100) * JAM_EFFECT_SEND_MAX_GAIN);
  const reverb = trackedGain(ctx, nodes, (sends.r / 100) * JAM_EFFECT_SEND_MAX_GAIN);
  output.connect(dry).connect(bus.dry);
  output.connect(chorus).connect(bus.chorus);
  output.connect(delay).connect(bus.delay);
  output.connect(reverb).connect(bus.reverb);
}

function configureDrumFilter(filter: BiquadFilterNode, pad: number, at: number): void {
  if (pad === 0 || pad === 6 || pad === 7) {
    filter.type = "lowpass";
    filter.frequency.setValueAtTime(pad === 0 ? 500 : 1_200, at);
    filter.Q.setValueAtTime(1, at);
  } else if (pad === 1 || pad === 2 || pad === 3) {
    filter.type = "bandpass";
    filter.frequency.setValueAtTime(pad === 2 ? 2_400 : 1_800, at);
    filter.Q.setValueAtTime(0.8, at);
  } else {
    filter.type = "highpass";
    filter.frequency.setValueAtTime(pad >= 8 ? 4_500 : 6_500, at);
    filter.Q.setValueAtTime(0.7, at);
  }
}

function drumSends(pad: number): { c: number; d: number; r: number } {
  if (pad === 0) return { c: 0, d: 2, r: 4 };
  if (pad === 1 || pad === 3) return { c: 4, d: 4, r: 16 };
  if (pad >= 8) return { c: 7, d: 3, r: 20 };
  return { c: 3, d: 2, r: 9 };
}

function fillNoise(out: Float32Array, seed: number): void {
  let state = seed || 0x6d2b79f5;
  for (let index = 0; index < out.length; index += 1) {
    state ^= state << 13;
    state ^= state >>> 17;
    state ^= state << 5;
    out[index] = ((state >>> 0) / 0x8000_0000) - 1;
  }
}

/** Domain-separated, injectively serialized seed material. This is reproducibility, not entropy. */
export async function drumSeed(input: JamDrumSeedInput): Promise<number> {
  const material = JSON.stringify([
    JAM_DRUM_SEED_DOMAIN,
    input.callId,
    input.source,
    input.sessionNonce,
    input.sequence,
    input.pad,
  ]);
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(material));
  return new DataView(digest).getUint32(0, false);
}

function noteVoiceId(source: string, note: number, sequence: number): string {
  return `${source}\u0000note:${note}:${sequence}`;
}

function drumVoiceId(channel: JamSourceChannel, nonce: string, sequence: number): string {
  return `${channel.source}\u0000drum:${channel.serial}:${nonce}:${sequence}`;
}

function isMidiNote(note: number): boolean {
  return Number.isInteger(note) && note >= 0 && note <= 127;
}

function isLegacyWave(value: string): value is LegacyWave {
  return value === "sine" || value === "triangle" || value === "square" || value === "sawtooth";
}

function isOscillator(source: AudioScheduledSourceNode): source is OscillatorNode {
  return "frequency" in source && "detune" in source;
}

// Compile-time pin: the allocator and engine must agree on the held-note boundary.
void JAM_HELD_PER_PEER;
