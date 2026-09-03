import {
  JAM_MET_BPB_MAX,
  JAM_MET_BPB_MIN,
  JAM_MET_BPM_MAX,
  JAM_MET_BPM_MIN,
  JAM_SEQUENCE_MAX,
  JAM_SESSION_NONCE_HEX_CHARS,
  JAM_TAKE_COMMITMENT_DOMAIN,
  TAKE_MAX_BYTES,
  TAKE_MAX_DURATION_MS,
  TAKE_MAX_EVENTS,
  TAKE_ID_MAX_BYTES,
  TAKE_MAX_LANES,
  TAKE_MAX_PARTICIPANTS,
  TAKE_MAX_PATCHES,
  type JamPatch,
  type JamTake,
  type JamTakeEvent,
  type JamTakeLane,
  type LegacyWave,
} from "./jam-contract.ts";
import { validateJamPatch } from "./jam-patch.ts";

export type JamRecorderState = "arming" | "recording" | "paused-membership" | "stopped";
export type JamRecordResult =
  | Readonly<{ ok: true; gap: Readonly<{ from: number; to: number }> | null; event: JamTakeEvent }>
  | Readonly<{
    ok: false;
    reason: "not-recording" | "unknown-source" | "invalid" | "duplicate" | "backward-time" |
      "lane-limit" | "patch-limit" | "event-limit" | "byte-limit";
  }>;

export type JamRecorderConfig = Readonly<{
  groupId: string;
  callId: string;
  bpm: number;
  beatsPerBar: number;
  participants: readonly string[];
}>;

type LaneState = { index: number; lastSequence: number | null; lastMs: number };
const encoder = new TextEncoder();

function boundedTakeIdentity(value: unknown): value is string {
  return typeof value === "string" && value.length > 0 && encoder.encode(value).byteLength <= TAKE_ID_MAX_BYTES;
}

function bytes(value: unknown): number {
  return encoder.encode(JSON.stringify(value)).byteLength;
}

function plainRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  try {
    const proto = Object.getPrototypeOf(value);
    if (proto !== Object.prototype && proto !== null) return null;
    const descriptors = Object.values(Object.getOwnPropertyDescriptors(value));
    return descriptors.every((descriptor) => !descriptor.get && !descriptor.set)
      ? value as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function validSequence(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= JAM_SEQUENCE_MAX;
}

function validMs(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value <= TAKE_MAX_DURATION_MS;
}

function validNonce(value: string): boolean {
  return new RegExp(`^[0-9a-f]{${JAM_SESSION_NONCE_HEX_CHARS}}$`).test(value);
}

function validWave(value: string): value is LegacyWave {
  return value === "sine" || value === "triangle" || value === "square" || value === "sawtooth";
}

/**
 * Bounded ephemeral take log. Authenticated source identity is a method argument, never an event
 * property accepted from the sender. Consent remains honest-client coordination and is surfaced
 * as explicit state rather than being mistaken for an endpoint-enforceable privacy boundary.
 */
export class JamTakeRecorder {
  readonly config: JamRecorderConfig;
  private readonly parts: string[];
  private readonly partIndex = new Map<string, number>();
  private readonly consents = new Set<string>();
  private readonly lanes: JamTakeLane[] = [];
  private readonly laneState = new Map<string, LaneState>();
  private readonly patches: JamPatch[] = [];
  private readonly patchIndex = new Map<string, number>();
  private readonly events: JamTakeEvent[] = [];
  private stateValue: JamRecorderState = "arming";
  /** Invalidates receipt-time leases whenever consent/membership leaves recording. */
  private recordingGeneration = 0;
  private membershipMatches = true;
  private resumeAfterMembershipPause = false;
  private byteEstimate: number;

  constructor(config: JamRecorderConfig) {
    if (
      !boundedTakeIdentity(config.groupId) || !boundedTakeIdentity(config.callId) ||
      !Number.isInteger(config.bpm) || config.bpm < JAM_MET_BPM_MIN || config.bpm > JAM_MET_BPM_MAX ||
      !Number.isInteger(config.beatsPerBar) || config.beatsPerBar < JAM_MET_BPB_MIN || config.beatsPerBar > JAM_MET_BPB_MAX ||
      config.participants.length < 1 || config.participants.length > TAKE_MAX_PARTICIPANTS ||
      new Set(config.participants).size !== config.participants.length ||
      config.participants.some((part) => !boundedTakeIdentity(part))
    ) throw new TypeError("invalid jam recorder configuration");
    // Keep the validated header independent of its caller. TypeScript's `readonly` is erased at
    // runtime; retaining `config` would let a later array/property mutation bypass byte accounting
    // and change the take that is eventually signed.
    const participants = Object.freeze([...config.participants]);
    this.config = Object.freeze({
      groupId: config.groupId,
      callId: config.callId,
      bpm: config.bpm,
      beatsPerBar: config.beatsPerBar,
      participants,
    });
    this.parts = [...participants];
    this.parts.forEach((part, index) => this.partIndex.set(part, index));
    this.byteEstimate = bytes(this.emptyTake());
    if (this.byteEstimate > TAKE_MAX_BYTES) throw new TypeError("jam recorder header exceeds the take byte limit");
  }

  state(): JamRecorderState {
    return this.stateValue;
  }

  /** Opaque monotonic value captured with an admitted event before any async queue/digest. */
  leaseGeneration(): number {
    return this.recordingGeneration;
  }

  /** A lease is append-authoritative only inside the uninterrupted recording interval that minted it. */
  acceptsLease(generation: number): boolean {
    return this.stateValue === "recording" && generation === this.recordingGeneration;
  }

  setConsent(source: string, consent: boolean): boolean {
    if (!this.partIndex.has(source) || this.stateValue === "stopped") return false;
    if (consent) this.consents.add(source);
    else {
      this.consents.delete(source);
      if (this.stateValue === "recording") {
        this.recordingGeneration += 1;
        this.stateValue = "arming";
      }
    }
    return true;
  }

  ready(): boolean {
    return this.parts.every((source) => this.consents.has(source));
  }

  start(): boolean {
    if (this.stateValue === "stopped" || !this.membershipMatches || !this.ready()) return false;
    this.stateValue = "recording";
    return true;
  }

  /** Any membership-set change pauses this take; the UI may stop it or restore the exact set. */
  membershipChanged(current: readonly string[]): void {
    if (this.stateValue === "stopped") return;
    const same = current.length === this.parts.length && new Set(current).size === current.length &&
      current.every((source) => this.partIndex.has(source));
    this.membershipMatches = same;
    if (!same) {
      if (this.stateValue === "recording") {
        this.recordingGeneration += 1;
        this.resumeAfterMembershipPause = true;
      }
      this.stateValue = "paused-membership";
    } else if (this.stateValue === "paused-membership") {
      this.stateValue = this.resumeAfterMembershipPause && this.ready() ? "recording" : "arming";
      this.resumeAfterMembershipPause = false;
    }
  }

  recordNoteOn(input: {
    source: string;
    sessionNonce: string;
    ms: number;
    sequence: number;
    note: number;
    wave: LegacyWave;
    patch?: unknown;
  }): JamRecordResult {
    if (!Number.isInteger(input.note) || input.note < 0 || input.note > 127 || !validWave(input.wave)) {
      return { ok: false, reason: "invalid" };
    }
    let patch: number | undefined;
    let extraBytes = 0;
    let commitPatch: (() => void) | undefined;
    if (input.patch !== undefined) {
      const validated = validateJamPatch(input.patch);
      if (!validated.ok) return { ok: false, reason: "invalid" };
      const existing = this.patchIndex.get(validated.canonical);
      if (existing !== undefined) patch = existing;
      else {
        if (this.patches.length >= TAKE_MAX_PATCHES) return { ok: false, reason: "patch-limit" };
        patch = this.patches.length;
        extraBytes = encoder.encode(validated.canonical).byteLength + 1;
        commitPatch = () => {
          this.patches.push(validated.patch);
          this.patchIndex.set(validated.canonical, patch!);
        };
      }
    }
    return this.append(input, (lane) => ({
      ms: input.ms,
      lane,
      n: input.note,
      on: 1,
      w: input.wave,
      ...(patch === undefined ? {} : { p: patch }),
      q: input.sequence,
    }), { extraBytes, commit: commitPatch });
  }

  recordNoteOff(input: {
    source: string;
    sessionNonce: string;
    ms: number;
    sequence: number;
    note: number;
  }): JamRecordResult {
    if (!Number.isInteger(input.note) || input.note < 0 || input.note > 127) return { ok: false, reason: "invalid" };
    return this.append(input, (lane) => ({ ms: input.ms, lane, n: input.note, on: 0, q: input.sequence }));
  }

  recordDrum(input: {
    source: string;
    sessionNonce: string;
    ms: number;
    sequence: number;
    pad: number;
  }): JamRecordResult {
    if (!Number.isInteger(input.pad) || input.pad < 0 || input.pad > 9) return { ok: false, reason: "invalid" };
    return this.append(input, (lane) => ({ ms: input.ms, lane, n: input.pad, d: 1, q: input.sequence }));
  }

  stop(): JamTake {
    if (this.stateValue === "recording") this.recordingGeneration += 1;
    this.stateValue = "stopped";
    return this.snapshot();
  }

  snapshot(): JamTake {
    return {
      v: 1,
      group: this.config.groupId,
      call: this.config.callId,
      met: { bpm: this.config.bpm, bpb: this.config.beatsPerBar },
      parts: [...this.parts],
      lanes: this.lanes.map((lane) => ({ ...lane })),
      patches: this.patches.map((patch) => structuredClone(patch)),
      events: this.events
        .map((event) => ({ ...event }))
        .sort((a, b) => a.ms - b.ms || a.lane - b.lane || a.q - b.q),
    };
  }

  private emptyTake(): JamTake {
    return {
      v: 1,
      group: this.config.groupId,
      call: this.config.callId,
      met: { bpm: this.config.bpm, bpb: this.config.beatsPerBar },
      parts: [...this.parts],
      lanes: [],
      patches: [],
      events: [],
    };
  }

  private append(
    input: { source: string; sessionNonce: string; ms: number; sequence: number },
    build: (lane: number) => JamTakeEvent,
    pending: { extraBytes: number; commit?: () => void } = { extraBytes: 0 },
  ): JamRecordResult {
    if (this.stateValue !== "recording") return { ok: false, reason: "not-recording" };
    if (!this.partIndex.has(input.source)) return { ok: false, reason: "unknown-source" };
    if (!validNonce(input.sessionNonce) || !validMs(input.ms) || !validSequence(input.sequence)) {
      return { ok: false, reason: "invalid" };
    }
    const laneKey = `${input.source}\u0000${input.sessionNonce}`;
    let lane = this.laneState.get(laneKey);
    let pendingLane: JamTakeLane | null = null;
    let pendingLaneBytes = 0;
    if (!lane) {
      if (this.lanes.length >= TAKE_MAX_LANES) return { ok: false, reason: "lane-limit" };
      pendingLane = { src: this.partIndex.get(input.source)!, sn: input.sessionNonce };
      pendingLaneBytes = bytes(pendingLane) + 1;
      lane = { index: this.lanes.length, lastSequence: null, lastMs: 0 };
    }
    if (input.ms < lane.lastMs) return { ok: false, reason: "backward-time" };
    if (lane.lastSequence !== null && input.sequence <= lane.lastSequence) return { ok: false, reason: "duplicate" };
    if (this.events.length >= TAKE_MAX_EVENTS) return { ok: false, reason: "event-limit" };

    const event = build(lane.index);
    const eventBytes = bytes(event) + 1;
    const addedBytes = eventBytes + pendingLaneBytes + pending.extraBytes;
    if (this.byteEstimate + addedBytes > TAKE_MAX_BYTES) return { ok: false, reason: "byte-limit" };
    const gap = lane.lastSequence !== null && input.sequence > lane.lastSequence + 1
      ? { from: lane.lastSequence + 1, to: input.sequence - 1 }
      : null;
    if (pendingLane) {
      this.lanes.push(pendingLane);
      this.laneState.set(laneKey, lane);
    }
    pending.commit?.();
    lane.lastSequence = input.sequence;
    lane.lastMs = input.ms;
    this.events.push(event);
    this.byteEstimate += addedBytes;
    return { ok: true, gap, event };
  }
}

/** Shared UI seam for local consent changes; local and remote withdrawal hit the same state gate. */
export function applyJamRecorderConsent(
  recorder: JamTakeRecorder | null,
  source: string,
  consent: boolean,
): boolean {
  return !!recorder && !!source && recorder.setConsent(source, consent);
}

export type JamRecorderLease = Readonly<{ recorder: JamTakeRecorder; generation: number; ms: number }>;

export type JamRecorderTimeline = Readonly<{ startMs: number | null }>;

/** Set a recorder's monotonic origin exactly once; consent pause/resume preserves it. */
export function startJamRecorderTimeline(
  timeline: JamRecorderTimeline,
  nowMs: number,
): JamRecorderTimeline {
  if (timeline.startMs !== null || !Number.isFinite(nowMs)) return timeline;
  return { startMs: Math.max(0, nowMs) };
}

/** Timestamp against the retained origin; callers may use the same value for UI duration. */
export function jamRecorderTimelineMs(timeline: JamRecorderTimeline, nowMs: number): number {
  if (timeline.startMs === null || !Number.isFinite(nowMs)) return 0;
  return Math.max(0, Math.round(nowMs - timeline.startMs));
}

/**
 * Capture recorder identity and event time before an asynchronous render/digest boundary.
 *
 * Admission is decided at receipt, not after the await. An event received while consent is still
 * arming must never become recordable merely because the same recorder starts before hashing ends.
 */
export function captureJamRecorderLease(
  recorder: JamTakeRecorder | null,
  ms: number,
): JamRecorderLease | null {
  return recorder?.state() === "recording" && Number.isFinite(ms)
    ? { recorder, generation: recorder.leaseGeneration(), ms: Math.max(0, Math.round(ms)) }
    : null;
}

function jamRecorderLeaseCurrent(current: JamTakeRecorder | null, lease: JamRecorderLease | null): lease is JamRecorderLease {
  return !!lease && current === lease.recorder && lease.recorder.acceptsLease(lease.generation);
}

/** Append a note-on only inside the uninterrupted recording interval that admitted it. */
export function recordLeasedJamNoteOn(
  current: JamTakeRecorder | null,
  lease: JamRecorderLease | null,
  input: Omit<Parameters<JamTakeRecorder["recordNoteOn"]>[0], "ms">,
): JamRecordResult | null {
  if (!jamRecorderLeaseCurrent(current, lease)) return null;
  return lease.recorder.recordNoteOn({ ...input, ms: lease.ms });
}

/** Append a note-off only inside the uninterrupted recording interval that admitted it. */
export function recordLeasedJamNoteOff(
  current: JamTakeRecorder | null,
  lease: JamRecorderLease | null,
  input: Omit<Parameters<JamTakeRecorder["recordNoteOff"]>[0], "ms">,
): JamRecordResult | null {
  if (!jamRecorderLeaseCurrent(current, lease)) return null;
  return lease.recorder.recordNoteOff({ ...input, ms: lease.ms });
}

/** Append a drum only inside the uninterrupted recording interval that admitted it. */
export function recordLeasedJamDrum(
  current: JamTakeRecorder | null,
  lease: JamRecorderLease | null,
  input: { source: string; sessionNonce: string; sequence: number; pad: number },
): JamRecordResult | null {
  if (!jamRecorderLeaseCurrent(current, lease)) return null;
  return lease.recorder.recordDrum({ ...input, ms: lease.ms });
}

export type TakeValidation =
  | Readonly<{ ok: true; take: JamTake }>
  | Readonly<{ ok: false; error: string }>;

/** Validate a saved/imported take before any event reaches Web Audio. */
export function validateJamTake(value: unknown): TakeValidation {
  const root = plainRecord(value);
  if (!root) return { ok: false, error: "take is not a plain object" };
  const raw = root as Partial<JamTake>;
  const met = plainRecord(raw.met);
  if (
    Object.keys(root).sort().join(",") !== "call,events,group,lanes,met,parts,patches,v" ||
    raw.v !== 1 || !boundedTakeIdentity(raw.group) || !boundedTakeIdentity(raw.call) ||
    !met || Object.keys(met).sort().join(",") !== "bpb,bpm" ||
    !Number.isInteger(met.bpm) || (met.bpm as number) < JAM_MET_BPM_MIN || (met.bpm as number) > JAM_MET_BPM_MAX ||
    !Number.isInteger(met.bpb) || (met.bpb as number) < JAM_MET_BPB_MIN || (met.bpb as number) > JAM_MET_BPB_MAX ||
    !Array.isArray(raw.parts) || raw.parts.length < 1 || raw.parts.length > TAKE_MAX_PARTICIPANTS ||
    raw.parts.some((part) => !boundedTakeIdentity(part)) || new Set(raw.parts).size !== raw.parts.length ||
    !Array.isArray(raw.lanes) || raw.lanes.length > TAKE_MAX_LANES ||
    !Array.isArray(raw.patches) || raw.patches.length > TAKE_MAX_PATCHES ||
    !Array.isArray(raw.events) || raw.events.length > TAKE_MAX_EVENTS
  ) return { ok: false, error: "take header or collection bound is invalid" };

  const lanes: JamTakeLane[] = [];
  const seenLanes = new Set<string>();
  for (const candidate of raw.lanes) {
    const laneRecord = plainRecord(candidate);
    if (!laneRecord || Object.keys(laneRecord).sort().join(",") !== "sn,src") {
      return { ok: false, error: "take lane is invalid" };
    }
    const lane = laneRecord as unknown as JamTakeLane;
    if (!Number.isInteger(lane.src) || lane.src < 0 || lane.src >= raw.parts.length || !validNonce(lane.sn)) {
      return { ok: false, error: "take lane is outside its bound" };
    }
    const key = `${lane.src}\u0000${lane.sn}`;
    if (seenLanes.has(key)) return { ok: false, error: "take lane is duplicated" };
    seenLanes.add(key);
    lanes.push({ src: lane.src, sn: lane.sn });
  }

  const patches: JamPatch[] = [];
  const canonicalPatches = new Set<string>();
  for (const candidate of raw.patches) {
    const validated = validateJamPatch(candidate);
    if (!validated.ok) return { ok: false, error: `take patch is invalid: ${validated.error}` };
    if (canonicalPatches.has(validated.canonical)) return { ok: false, error: "take patch is duplicated" };
    canonicalPatches.add(validated.canonical);
    patches.push(validated.patch);
  }

  const lastSequence = new Map<number, number>();
  const lastMs = new Map<number, number>();
  const events: JamTakeEvent[] = [];
  for (const candidate of raw.events) {
    const checked = validateTakeEvent(candidate, lanes.length, patches.length);
    if (!checked) return { ok: false, error: "take event is invalid" };
    const previousSequence = lastSequence.get(checked.lane);
    const previousMs = lastMs.get(checked.lane);
    if ((previousSequence !== undefined && checked.q <= previousSequence) || (previousMs !== undefined && checked.ms < previousMs)) {
      return { ok: false, error: "take lane order is invalid" };
    }
    lastSequence.set(checked.lane, checked.q);
    lastMs.set(checked.lane, checked.ms);
    events.push(checked);
  }

  events.sort((a, b) => a.ms - b.ms || a.lane - b.lane || a.q - b.q);

  const take: JamTake = {
    v: 1,
    group: raw.group,
    call: raw.call,
    met: { bpm: met.bpm as number, bpb: met.bpb as number },
    parts: [...raw.parts],
    lanes,
    patches,
    events,
  };
  if (bytes(take) > TAKE_MAX_BYTES) return { ok: false, error: "take exceeds its byte limit" };
  return { ok: true, take };
}

function validateTakeEvent(candidate: unknown, lanes: number, patches: number): JamTakeEvent | null {
  const event = plainRecord(candidate);
  if (!event) return null;
  if (!validMs(event.ms as number) || !Number.isInteger(event.lane) || (event.lane as number) < 0 ||
    (event.lane as number) >= lanes || !validSequence(event.q as number) ||
    !Number.isInteger(event.n)) return null;
  if (event.d === 1) {
    if (Object.keys(event).sort().join(",") !== "d,lane,ms,n,q" || (event.n as number) < 0 || (event.n as number) > 9) return null;
    return { ms: event.ms as number, lane: event.lane as number, n: event.n as number, d: 1, q: event.q as number };
  }
  if (event.on === 0) {
    if (Object.keys(event).sort().join(",") !== "lane,ms,n,on,q" || (event.n as number) < 0 || (event.n as number) > 127) return null;
    return { ms: event.ms as number, lane: event.lane as number, n: event.n as number, on: 0, q: event.q as number };
  }
  const keys = Object.keys(event).sort().join(",");
  if (
    event.on !== 1 || (keys !== "lane,ms,n,on,q,w" && keys !== "lane,ms,n,on,p,q,w") ||
    (event.n as number) < 0 || (event.n as number) > 127 || !validWave(event.w as string) ||
    (event.p !== undefined && (!Number.isInteger(event.p) || (event.p as number) < 0 || (event.p as number) >= patches))
  ) return null;
  return {
    ms: event.ms as number,
    lane: event.lane as number,
    n: event.n as number,
    on: 1,
    w: event.w as LegacyWave,
    ...(event.p === undefined ? {} : { p: event.p as number }),
    q: event.q as number,
  };
}

function toHex(buffer: ArrayBuffer): string {
  return [...new Uint8Array(buffer)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function sha256(value: unknown): Promise<string> {
  return toHex(await crypto.subtle.digest("SHA-256", encoder.encode(JSON.stringify(value))));
}

export async function jamTakeId(take: JamTake): Promise<string> {
  const validated = validateJamTake(take);
  if (!validated.ok) throw new TypeError(validated.error);
  return sha256(validated.take);
}

/** Byte-cap an untrusted import before JSON.parse allocates its object graph. */
export function parseJamTakeJson(raw: unknown): TakeValidation {
  if (typeof raw !== "string") return { ok: false, error: "take import is not text" };
  if (raw.length > TAKE_MAX_BYTES || encoder.encode(raw).byteLength > TAKE_MAX_BYTES) {
    return { ok: false, error: "take exceeds its byte limit" };
  }
  try {
    return validateJamTake(JSON.parse(raw));
  } catch {
    return { ok: false, error: "take is not valid JSON" };
  }
}

/** One future signature covers all reconnect lanes owned by one participant. */
export async function jamParticipantCommitment(takeId: string, take: JamTake, participantIndex: number): Promise<Readonly<{
  domain: typeof JAM_TAKE_COMMITMENT_DOMAIN;
  takeId: string;
  groupId: string;
  callId: string;
  device: string;
  laneEventLogHash: string;
}>> {
  const validated = validateJamTake(take);
  if (!validated.ok) throw new TypeError(validated.error);
  if (takeId !== await sha256(validated.take)) throw new TypeError("take id does not match the validated take");
  const device = validated.take.parts[participantIndex];
  if (!device) throw new RangeError("unknown take participant");
  const lanes = validated.take.lanes.flatMap((lane, laneIndex) => lane.src === participantIndex
    ? [{
      sessionNonce: lane.sn,
      events: validated.take.events.filter((event) => event.lane === laneIndex),
    }]
    : []);
  return {
    domain: JAM_TAKE_COMMITMENT_DOMAIN,
    takeId,
    groupId: validated.take.group,
    callId: validated.take.call,
    device,
    laneEventLogHash: await sha256({ v: 1, domain: JAM_TAKE_COMMITMENT_DOMAIN, device, lanes }),
  };
}
