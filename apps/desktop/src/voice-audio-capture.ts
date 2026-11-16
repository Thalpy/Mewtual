/** One process-wide microphone permission request, independent of old Call/N/Phase.
 * This private SDK leaf is not a native capability or an application handle. Its
 * caller must prepay the fixed pool inventory before construction, and bind that
 * inventory to the accepted group before creating the one active session.
 *
 * A separate original-group capsule is paid BEFORE getUserMedia. End can detach
 * that pending capsule without waiting for an OS permission prompt: a replacement
 * session stays listen-only until the actual old SDK task returns and stops its
 * tracks. Neither a timeout nor cancellation refunds that outstanding task.
 * No existing App path constructs this leaf yet. */
import type { VoiceMediaStatus } from "./voice-media-status.ts";
import type { VoiceSdpRtcClock } from "./voice-sdp-rtc.ts";

// Logical owned-JS inventory (opaque browser/GC heaps excluded): one fixed pool,
// one active selector/ledger adapter, and one post-SDK completion/refund frame with
// an emptied capsule/passive status. Reserve4KiB for this fixed graph, independently
// of the8KiB capsule's bounded device/constraints strings, SDK await and master.
// In particular the completion reaction is NOT unpaid just because SDK IO returned.
export const VOICE_AUDIO_CAPTURE_POOL_BYTES = 4 * 1024;
export const VOICE_AUDIO_CAPTURE_BYTES = 8 * 1024;
const MAX_DEVICE_UNITS = 1024;
const COPY_CHUNK_UNITS = 256;

/** Executable inventory of serial application-owned stages, not VM/SDK heap or RSS.
 * The caller's borrowed input may have a large backing; it never crosses the synchronous
 * copy frame. Numeric arguments end before join, and all copy scratch ends before SDK IO.
 * The fixed4KiB pool still separately pays its post-SDK completion/refund tail. */
export function voiceAudioCaptureInventory(): Readonly<{ copy: number; join: number; sdk: number }> {
  const fixed = 1024; // Capsule/current/payment/master refs and small synchronous wrappers.
  const pieces = Math.ceil(MAX_DEVICE_UNITS / COPY_CHUNK_UNITS) * (COPY_CHUNK_UNITS * 2 + 32);
  const index = 128, owned = MAX_DEVICE_UNITS * 2 + 32;
  return Object.freeze({
    copy: fixed + pieces + index + (COPY_CHUNK_UNITS * 2 + 64) + (COPY_CHUNK_UNITS * 8 + 128),
    join: fixed + pieces + index + owned + 1024,
    sdk: fixed + owned + 512 + 2048, // Constraints plus actual SDK await/completion frames.
  });
}

/** A short substring can retain an arbitrarily large caller backing. Create each bounded
 * UTF16 chunk from numeric code units, then join only those owned chunks. A UTF8 roundtrip
 * would alter unpaired surrogates in opaque device IDs; slice/join need not detach backing. */
function ownedDevice(value: string): string {
  const pieces: string[] = [];
  for (let from = 0; from < value.length; from += COPY_CHUNK_UNITS) {
    const length = Math.min(COPY_CHUNK_UNITS, value.length - from);
    const units = new Uint16Array(length);
    for (let at = 0; at < length; at++) units[at] = value.charCodeAt(from + at);
    pieces.push(String.fromCharCode(...units));
  }
  return pieces.join("");
}

/** Must capture only the prepaid broker/group ledger, never an old native owner,
 * Call, negotiation, Phase, or UI component. It returns the SAME capsule payment. */
export type VoiceAudioCaptureBudget = Readonly<{
  /** Atomic synchronous ledger admission: return its one-shot payment, or throw
   * without charging. No UI/native/negotiation object may hide in this adapter. */
  reserve(): Readonly<{ release(): void }>;
}>;
/** Trusted broker supplies the actual independently paid adopted media status and
 * shared clock. This is not a renderer boolean, callback, or historical Ready. */
export type VoiceAudioCaptureCurrent = Readonly<{ status: VoiceMediaStatus; clock: VoiceSdpRtcClock }>;
type Devices = Pick<MediaDevices, "getUserMedia">;
type Capsule = {
  target: VoiceAudioCaptureSession | null;
  current: VoiceAudioCaptureCurrent;
  payment: Readonly<{ release(): void }> | null;
  stream: MediaStream | null;
  task: Promise<void> | null;
  settled: boolean; stopping: boolean;
};
const unavailable = (): never => { throw new Error("voice microphone unavailable"); };

function current(capsule: Capsule): boolean {
  try { return capsule.current.status.currentAt(capsule.current.clock.now()); }
  catch { return false; }
}

/** Synchronous negative SDK tail. A throwing provider retains the actual stream
 * and original payment for retry. SDK getUserMedia(audio-only) promises one audio
 * track; nevertheless stop every returned track on an unexpected/late result.
 * These are local browser objects, never a peer-supplied track collection. */
function stop(capsule: Capsule): boolean {
  if (capsule.stopping) return false;
  if (!capsule.stream) return true;
  capsule.stopping = true;
  try {
    let failed = false;
    for (const track of capsule.stream.getTracks()) {
      try { track.enabled = false; } catch { failed = true; }
      try { track.stop(); } catch { failed = true; }
    }
    if (failed) return false;
    capsule.stream = null;
    return true;
  } catch { return false; }
  finally { capsule.stopping = false; }
}

/** No Session/Call/native/N lexical arguments survive the actual SDK await. The
 * capsule's detachable target is cleared synchronously on End. The fulfilled SDK
 * promise/stream never becomes the result of the retained void task. */
async function capture(pool: VoiceAudioCapturePool, capsule: Capsule, devices: Devices,
    constraints: MediaStreamConstraints): Promise<void> {
  if (!pool.mayStart(capsule)) return;
  const get = devices.getUserMedia;
  if (!pool.mayStart(capsule)) return; // A method getter can End/reenter.
  try {
    capsule.stream = await get.call(devices, constraints);
    pool.accept(capsule);
  } catch {
    // Permission/device/provider failure leaves the authenticated peer listen-only.
    // A late/invalid result or failed stop remains independently paid for cleanup.
  }
}

/** These two factories intentionally live outside begin's setup scope. No async
 * closure may retain its old Session/budget/authority wrapper, even on a JS engine
 * which conservatively keeps complete lexical environments. The heavy factory
 * retains only explicitly paid SDK inputs until its void worker actually returns. */
function startCapture(pool: VoiceAudioCapturePool, capsule: Capsule, devices: Devices,
    constraints: MediaStreamConstraints): Promise<void> {
  return Promise.resolve().then(() => capture(pool, capsule, devices, constraints));
}
/** The separately prepaid fixed pool covers this small post-worker finalizer.
 * The settled work's result is void; no SDK payload/device/constraints or old
 * Session/setup scope is captured by this reaction. */
function finishCapture(pool: VoiceAudioCapturePool, capsule: Capsule, work: Promise<void>): Promise<void> {
  return work.then(() => pool.returned(capsule), () => pool.returned(capsule));
}

/** Already paid window owner. At most one active session and one unsettled/orphan
 * capture exist. Active master custody uses the same8KiB capsule, not a new fee.
 * No selected key, media driver, native capability, old Call or Phase is retained. */
export class VoiceAudioCapturePool {
  #active: VoiceAudioCaptureSession | null = null;
  #pending: Capsule | null = null;
  #reserving = false;
  session(budget: VoiceAudioCaptureBudget): VoiceAudioCaptureSession {
    if (this.#active) return unavailable();
    const session = new VoiceAudioCaptureSession(this, budget);
    this.#active = session;
    return session;
  }
  get pending(): boolean { return this.#pending !== null; }
  /** Observation of the SAME actual task. Calling End need not await this. No
   * payload, master track, SDK promise result, or native authority is exposed. */
  settlement(): Promise<void> { return this.#pending?.task ?? Promise.resolve(); }

  begin(session: VoiceAudioCaptureSession, budget: VoiceAudioCaptureBudget,
      authority: VoiceAudioCaptureCurrent, device: string, devices: Devices): boolean {
    if (this.#active !== session || session.closed || this.#pending || this.#reserving) return false;
    if (typeof device !== "string" || device.length > MAX_DEVICE_UNITS) return false;
    // A ledger/Clock getter may End and create a replacement session. The fixed
    // pool's latch fences its nested reserve, including before we own a capsule.
    this.#reserving = true;
    try {
      const status = authority.status, clock = authority.clock;
      if (!status.currentAt(clock.now()) || this.#active !== session || session.closed || this.#pending) return false;
      const payment = budget.reserve(); // BEFORE any capsule/constraints/provider allocation.
      // Snapshot ONLY independent passive status/shared clock, never the caller's
      // wrapper, whose other fields could retain an old Call/N/Phase graph.
      const capsule: Capsule = { target: session, current: Object.freeze({ status, clock }), payment, stream: null,
        task: null, settled: false, stopping: false };
      // Publish before any remaining provider/Clock callbacks. reserve may have Ended.
      this.#pending = capsule;
      if (this.#active !== session || session.closed) capsule.target = null;
      let constraints: MediaStreamConstraints;
      try {
        // The original fee and discoverable capsule precede every copying callback. End
        // during copying detaches its target; the later SDK leaf will refuse to start.
        const owned = ownedDevice(device);
        constraints = { video: false, audio: owned
          ? { deviceId: { exact: owned }, echoCancellation: true, noiseSuppression: true, autoGainControl: true }
          : true };
      } catch {
        // Proven local refusal BEFORE constructing any SDK promise/registration. There
        // is no ambiguous handoff here. Return only this exact original capsule payment.
        capsule.target = null; capsule.settled = true; this.retry(); return false;
      }
      capsule.task = finishCapture(this, capsule, startCapture(this, capsule, devices, constraints));
      return true;
    } finally { this.#reserving = false; }
  }
  mayStart(capsule: Capsule): boolean {
    const target = capsule.target;
    return this.#pending === capsule && target !== null && this.#active === target && !target.closed &&
      current(capsule) && capsule.target === target && this.#active === target && !target.closed;
  }
  accept(capsule: Capsule): void {
    if (!this.mayStart(capsule) || !capsule.stream) return;
    const tracks = capsule.stream.getTracks();
    if (tracks.length !== 1) return;
    const track = tracks[0];
    if (track.kind !== "audio" || track.readyState !== "live") return;
    // A master never transmits directly. Sender clones must start disabled and be
    // governed independently; muting/stopping the master does NOT gate its clones.
    track.enabled = false;
    if (!this.mayStart(capsule)) return;
    capsule.target!.adopt(capsule);
  }
  returned(capsule: Capsule): void {
    capsule.settled = true; // Actual SDK/accept frame returned, not an abort observer.
    if (this.#pending !== capsule) return;
    const target = capsule.target;
    if (target && this.#active === target && !target.closed && target.owns(capsule)) {
      this.#pending = null; // Same paid capsule now owns the active master.
      return;
    }
    capsule.target = null;
    this.retry();
  }
  end(session: VoiceAudioCaptureSession, master: Capsule | null): void {
    if (this.#active !== session) return;
    if (this.#pending?.target === session) this.#pending.target = null;
    if (master) {
      // Publish quarantine BEFORE stop/getTracks setters can reenter and create a
      // replacement session. It may listen, but cannot start another OS request.
      if (this.#pending && this.#pending !== master) return unavailable();
      master.target = null; this.#pending = master;
    }
    this.#active = null;
    this.retry();
  }
  /** Negative cleanup only; no new request/admission/retry grant. Failed SDK stop
   * remains in the one quarantine and keeps the original-group payment. */
  retry(): void {
    const capsule = this.#pending;
    if (!capsule || !capsule.settled || capsule.target !== null || !stop(capsule)) return;
    this.#pending = null;
    const payment = capsule.payment; capsule.payment = null; capsule.task = null;
    // Actual SDK frame and stream custody are gone. The fixed paid pool covers
    // this remaining empty-capsule completion/refund reaction until its own return.
    payment?.release();
  }
}

/** One inert current-call capture selector under the fixed pool inventory. Its
 * budget is group-ledger-only. End clears all links from pending OS work back here. */
export class VoiceAudioCaptureSession {
  readonly #pool: VoiceAudioCapturePool;
  readonly #budget: VoiceAudioCaptureBudget;
  #closed = false;
  #master: Capsule | null = null;
  constructor(pool: VoiceAudioCapturePool, budget: VoiceAudioCaptureBudget) { this.#pool = pool; this.#budget = budget; }
  get closed(): boolean { return this.#closed; }
  get available(): boolean { return !this.#closed && this.#master?.settled === true; }
  request(authority: VoiceAudioCaptureCurrent, device: string, devices: Devices): boolean {
    if (this.#closed || this.#master) return false;
    try { return this.#pool.begin(this, this.#budget, authority, device, devices); }
    catch { return false; } // Local capacity/Clock refusal leaves the peer listen-only.
  }
  adopt(capsule: Capsule): void { if (!this.#closed && this.#master === null) this.#master = capsule; }
  owns(capsule: Capsule): boolean { return this.#master === capsule; }
  close(): void {
    if (this.#closed) { this.#pool.retry(); return; }
    this.#closed = true;
    const master = this.#master; this.#master = null;
    this.#pool.end(this, master);
  }
}
