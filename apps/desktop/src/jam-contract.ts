// jam-contract.ts: the jam:v2 wire contract, shared by the engine, the UI, and the tests.
// This module is the single source of truth for every limit cited in docs/INTERFACES.md section
// 12; it holds constants and types ONLY. Validators, allocators, and renderers live in the
// engine modules and import from here, so a number can never drift between doc, guard, and test.

export const JAM_WIRE_VERSION = "jam:v2";
export const JAM_PATCH_VERSION = "jam-patch:v1";
export const JAM_RENDERER_VERSION = "mewtual-synth:v1";
export const JAM_KIT_VERSION = "jam-kit:v1";
export const JAM_TAKE_VERSION = "jam-take:v1";
export const JAM_DRUM_SEED_DOMAIN = "catcoms-jam-drum:v1";
export const JAM_TAKE_COMMITMENT_DOMAIN = "catcoms-jam-take-commitment:v1";

// --- Frame admission -------------------------------------------------------------------------
// Charged before JSON.parse, for every frame, valid or not. The pre-parse byte cap admits the
// one large type (t:"p"); every other type must still fit the small cap after parse.
export const JAM_FRAME_MAX_BYTES = 1024;
export const JAM_SMALL_FRAME_MAX_BYTES = 200;
export const JAM_FRAME_BUCKET_RATE = 80; // tokens per second
export const JAM_FRAME_BUCKET_BURST = 160;
// Once musical abuse has muted a peer, a separately bounded control lane keeps only their exact
// coarse call-state heartbeat alive. Without it, consent withdrawal and video/mute state would
// freeze for the rest of the call. This lane is still charged before JSON.parse.
export const JAM_MUTED_STATE_BUCKET_RATE = 2;
export const JAM_MUTED_STATE_BUCKET_BURST = 4;
// A not-yet-open outbound edge retains only this many locally generated events. Overflow drops
// that unopened transient as a unit so note-ons cannot survive without their later note-offs.
export const JAM_OUTBOUND_PENDING_MAX = 256;
// Async SHA-256 extends receive processing past data-channel delivery. Bound the causal backlog
// independently so a peer cannot turn one delayed digest into unbounded retained closures.
export const JAM_INBOUND_PENDING_MAX = 256;
// Sustained-abuse auto-mute: bucket exhausted this long (cumulative, rolling window) mutes the
// peer's instrument channel receive-side for the rest of the call.
export const JAM_ABUSE_EXHAUSTED_MS = 10_000;
export const JAM_ABUSE_WINDOW_MS = 60_000;
// The musical bucket: note-ons only; note-offs are never charged (no stuck drones).
export const JAM_NOTEON_BUCKET_RATE = 30;
export const JAM_NOTEON_BUCKET_BURST = 60;
export const JAM_SEQUENCE_MAX = 0xffff_ffff;

// --- Voices ----------------------------------------------------------------------------------
// A voice is one sounding note; every slot is sized for the worst permitted patch, so there is
// no cost model. Held voices are never stolen; only releasing tails are (own oldest first, then
// global oldest), else the new note is rejected.
export const JAM_HELD_PER_PEER = 16;
export const JAM_VOICES_GLOBAL = 64;
export const JAM_RELEASE_CAP_MS = 8_000; // receiver-enforced ceiling, above any patch value
export const JAM_REMOTE_HOLD_MAX_MS = 30_000; // watchdog: a lost note-off may never drone

// Receiver-owned output and fixed control mappings. Patch fields can select only within these
// ceilings; none can bypass the per-source gates or master limiter.
export const JAM_VOICE_PEAK_GAIN = 0.11;
export const JAM_MASTER_GAIN = 0.72;
export const JAM_FILTER_Q_MIN = 0.1;
export const JAM_FILTER_Q_MAX = 18;
export const JAM_FILTER_ENV_MAX_OCTAVES = 6;
export const JAM_LFO_CUTOFF_MAX_OCTAVES = 4;
export const JAM_EFFECT_SEND_MAX_GAIN = 0.5;
export const JAM_FILTER_NYQUIST_RATIO = 0.45;
export const JAM_LIMITER_THRESHOLD_DB = -12;
export const JAM_LIMITER_KNEE_DB = 6;
export const JAM_LIMITER_RATIO = 12;
export const JAM_LIMITER_ATTACK_SECONDS = 0.003;
export const JAM_LIMITER_RELEASE_SECONDS = 0.25;

// --- Patch announces -------------------------------------------------------------------------
export const JAM_PATCH_ANNOUNCE_MIN_INTERVAL_MS = 2_000;
export const JAM_PATCH_ANNOUNCE_BURST = 3;
export const JAM_PATCH_CACHE_PER_PEER = 4; // validated patches remembered per peer (LRU)
// Full SHA-256: a 64-bit truncation admits generic collisions after roughly 2^32 work, while the
// full id still leaves every non-patch frame comfortably under JAM_SMALL_FRAME_MAX_BYTES.
export const JAM_PATCH_ID_HEX_CHARS = 64;
export const JAM_SESSION_NONCE_HEX_CHARS = 16;
// Exact pre-v2 note frames have no sender nonce. The decoder marks them explicitly and the
// integration binds this reserved receive-only nonce to that authenticated channel generation.
export const JAM_LEGACY_SESSION_NONCE = "0000000000000000";

// --- jam-patch:v1 bounds ---------------------------------------------------------------------
// All values are integers; unknown fields REJECT. Canonical serialization is JSON with exactly
// the JamPatch keys in declaration order and no whitespace.
export const PATCH_OSC_MIN = 1;
export const PATCH_OSC_MAX = 3;
export const PATCH_OSC_WAVES = ["sine", "triangle", "square", "sawtooth"] as const; // index = wire value
export const PATCH_TRANSPOSE_SEMITONES = 24; // +/-
export const PATCH_DETUNE_CENTS = 50; // +/-
export const PATCH_LEVEL_MAX = 100; // levels, sustain, resonance, depths, sends: 0..100
export const PATCH_ENV_ATTACK_MAX_MS = 5_000;
export const PATCH_ENV_DECAY_MAX_MS = 5_000;
export const PATCH_ENV_RELEASE_MAX_MS = 8_000; // never above JAM_RELEASE_CAP_MS
export const PATCH_FILTER_MODES = ["lowpass", "highpass", "bandpass"] as const; // index = wire value
export const PATCH_CUTOFF_MIN_HZ = 20;
export const PATCH_CUTOFF_MAX_HZ = 18_000;
export const PATCH_FILTER_ENV_RANGE = 100; // +/-
export const PATCH_LFO_RATE_MIN_CHZ = 1; // centi-Hz: 0.01 Hz
export const PATCH_LFO_RATE_MAX_CHZ = 1_200; // 12 Hz
export const PATCH_LFO_DESTS = ["off", "cutoff", "pitch"] as const; // index = wire value
export const PATCH_LFO_PITCH_DEPTH_CENTS = 25; // +/- at depth 100

export interface JamOsc {
  w: number; // 0..3, index into PATCH_OSC_WAVES
  t: number; // transpose, semitones
  c: number; // detune, cents
  l: number; // level 0..100
}
export interface JamEnv {
  a: number; // attack ms
  d: number; // decay ms
  s: number; // sustain 0..100
  r: number; // release ms
}
export interface JamFilter {
  m: number; // 0..2, index into PATCH_FILTER_MODES
  c: number; // cutoff Hz
  q: number; // resonance 0..100
  e: number; // envelope amount -100..100
}
export interface JamLfo {
  r: number; // rate, centi-Hz
  d: number; // depth 0..100
  t: number; // 0..2, index into PATCH_LFO_DESTS
}
export interface JamSends {
  c: number; // chorus send 0..100
  d: number; // delay send 0..100
  r: number; // reverb send 0..100
}
export interface JamPatch {
  v: 1;
  o: JamOsc[]; // 1..3
  e: JamEnv;
  f: JamFilter;
  l: JamLfo;
  x: JamSends;
}

// --- Wire frames -----------------------------------------------------------------------------
export type LegacyWave = "sine" | "triangle" | "square" | "sawtooth";

export interface JamNoteOn {
  t: "n";
  on: 1;
  n: number; // MIDI 0..127
  w: LegacyWave; // always present; what an old build renders
  p?: string; // patch id; honoured only after a validated announce
  q: number; // per-sender monotonic uint32 across note+drum events, per sn
}
export interface JamNoteOff {
  t: "n";
  on: 0;
  n: number;
  q: number;
}
// Drums use their own type so an old build ignores them. Encoding a pad as t:"n" would make an
// old receiver hold MIDI notes 0..9 forever (there is deliberately no drum note-off) and exhaust
// that sender's 16-note piano allowance.
export interface JamDrumHit {
  t: "d";
  n: number; // pad 0..9
  q: number;
}
export interface JamPatchAnnounce {
  t: "p";
  v: 1;
  id: string; // 64 lowercase hex; must equal the full SHA-256 hash of d
  sn: string; // 16 hex sender-session nonce; a change resets that sender's q domain
  d: JamPatch;
}
// Phase 4 (planned): metronome + clock probes.
export interface JamMetronome {
  t: "m";
  v: 1;
  sn: string; // sender-session nonce; rev is scoped to the authenticated (sender, sn) pair
  on: 0 | 1;
  rev: number; // uint32 monotonic only within this sender session
  bpm: number;
  bpb: number;
  org: number; // beat-0 in the anchor's clock domain
}
export interface JamClockProbe {
  t: "c";
  q: number;
  tx: number;
}
export interface JamClockReply {
  t: "c";
  r: number; // echoes the probe's q
  tx: number;
  rx: number;
}

// --- Metronome bounds (phase 4) --------------------------------------------------------------
export const JAM_MET_BPM_MIN = 40;
export const JAM_MET_BPM_MAX = 240;
export const JAM_MET_BPB_MIN = 1;
export const JAM_MET_BPB_MAX = 8;
export const JAM_MET_REV_MIN_INTERVAL_MS = 2_000;
// Different performance.now() clocks have arbitrary origins, so the first finite offset may be
// any size. Only a jump away from an established estimate is bounded.
export const JAM_MET_OFFSET_JUMP_MAX_MS = 2_000;
export const JAM_MET_LOOKAHEAD_MS = 150; // scheduling horizon against AudioContext.currentTime
export const JAM_CLOCK_PROBE_RATE = 1; // per second
export const JAM_CLOCK_PROBE_BURST = 4;
export const JAM_CLOCK_RTT_MAX_MS = 2_000;
export const JAM_CLOCK_SAMPLE_MAX = 8;
export const JAM_MET_CLICKS_PER_PASS_MAX = 8;

// --- jam-kit:v1 ------------------------------------------------------------------------------
// Fixed pad table; recipes are receiver-owned under the renderer contract. Choke is
// source-scoped: chokes lists the pads this pad silences FROM THE SAME SENDER only.
export interface JamPad {
  id: number;
  name: string;
  chokes: number[];
  tailMs: number;
}
export const JAM_KIT: readonly JamPad[] = [
  { id: 0, name: "kick", chokes: [], tailMs: 700 },
  { id: 1, name: "snare", chokes: [], tailMs: 700 },
  { id: 2, name: "rim", chokes: [], tailMs: 300 },
  { id: 3, name: "clap", chokes: [], tailMs: 700 },
  { id: 4, name: "hat", chokes: [5], tailMs: 300 },
  { id: 5, name: "open hat", chokes: [], tailMs: 1_500 },
  { id: 6, name: "lo tom", chokes: [], tailMs: 900 },
  { id: 7, name: "hi tom", chokes: [], tailMs: 900 },
  { id: 8, name: "ride", chokes: [], tailMs: 2_500 },
  { id: 9, name: "crash", chokes: [], tailMs: 3_000 },
] as const;
export const JAM_DRUM_TAIL_MAX_MS = 3_000;
export const JAM_DRUM_NOISE_PERIOD_SAMPLES = 4_096;
// Deterministic drum hashes commit in channel-generation order: one active digest per opaque lane
// prevents asynchronously resolved hits from reversing their event order, while a stale channel
// cannot head-of-line block its replacement. App playback serializes its validated cross-lane log.
// The pending owner bounds retain a finite fairness/backpressure lane.
export const JAM_DRUM_DIGESTS_PER_LANE = 1;
export const JAM_DRUM_DIGESTS_GLOBAL = 32;
export const JAM_DRUM_PENDING_PER_SOURCE = 256;
export const JAM_DRUM_PENDING_GLOBAL = 512;
/** Short receiver-local call cues may overlap only this many times across the whole room. */
export const JAM_CALL_CUE_PENDING_MAX = 4;

// --- jam-take:v1 (phase 5 live; durable signed attribution remains phase 6) ------------------
export const TAKE_MAX_DURATION_MS = 600_000;
export const TAKE_MAX_EVENTS = 20_000;
export const TAKE_MAX_BYTES = 524_288;
/** Validated jukebox takes retained per call (at most about 4 MiB of source JSON). */
export const JAM_TAKE_CACHE_MAX = 8;
export const TAKE_MAX_PARTICIPANTS = 16;
export const TAKE_MAX_LANES = 64;
export const TAKE_MAX_PATCHES = 64;
export const TAKE_ID_MAX_BYTES = 256;
// Dense/seeked takes are drained over bounded macrotask passes instead of monopolising one tick.
export const TAKE_PLAYBACK_EVENTS_PER_TICK = 128;

export interface JamTakeLane {
  src: number; // index into parts
  sn: string; // sender session needed for q scope and deterministic drum playback
}
export interface JamTakeNoteOn {
  ms: number; // grid time, stamped at the source
  lane: number; // index into lanes; src itself never comes from the event sender
  n: number;
  on: 1;
  w: LegacyWave;
  p?: number; // index into patches
  q: number; // gaps are surfaced, never silently smoothed
}
export interface JamTakeNoteOff {
  ms: number;
  lane: number;
  n: number;
  on: 0;
  q: number;
}
export interface JamTakeDrumHit {
  ms: number;
  lane: number;
  n: number; // pad 0..9
  d: 1;
  q: number;
}
export type JamTakeEvent = JamTakeNoteOn | JamTakeNoteOff | JamTakeDrumHit;
export interface JamTake {
  v: 1;
  group: string; // stable group/server id; signatures must not be replayable across groups
  call: string;
  met: { bpm: number; bpb: number };
  parts: string[]; // fingerprints
  lanes: JamTakeLane[];
  patches: JamPatch[]; // full descriptors inline; the player re-validates every one
  events: JamTakeEvent[];
}
