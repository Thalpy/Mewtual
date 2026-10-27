/** Concrete private RTC custody for one original native PC intent. There is no renderer
 * factory/description permit: the retained channel creates this owner before any SDK callback,
 * supplies fresh native admission and joins its void observers before releasing native custody.
 * Application text/copies are bounded here; opaque browser RTC/GC heap is outside that proof. */
import { voiceSdpUtf8 } from "./voice-sdp-action.ts";
import { voiceRtcConfigurationCurrent, voiceRtcConfiguredPeer, releaseVoiceRtcConfiguration,
  type VoiceRtcConfigurationLease } from "./voice-rtc-configuration.ts";

export type VoiceSdpRtcClock = Readonly<{
  now(): number;
  schedule(work: () => void, milliseconds: number): unknown;
  cancel(handle: unknown): void;
}>;
const unavailable = (): never => { throw new Error("original voice RTC peer unavailable"); };
const decoder = new TextDecoder("utf-8", { fatal: true });

type MediaState = {
  identity: object; peer: RTCPeerConnection | null; adopted: boolean;
  stopped: boolean; closing: boolean; closeComplete: boolean;
  configuration: VoiceRtcConfigurationLease | null;
};
// Runtime-private ownership, not a public setter or a renderer-supplied peer/capability.
// The identity is a fixed inert token, with no old owner/phase/clock/callback backedge.
const mediaOwners = new WeakMap<MediaRtcOwner, MediaState>();
class MediaRtcOwner {
  constructor(identity: object) {
    mediaOwners.set(this, { identity, peer: null, adopted: false, stopped: false, closing: false, closeComplete: false, configuration: null });
  }
  get adopted(): boolean { return mediaOwners.get(this)!.adopted; }
  get closed(): boolean { return mediaOwners.get(this)!.stopped; }
  get drained(): boolean {
    const state = mediaOwners.get(this)!;
    return state.stopped && state.closeComplete && state.peer === null && state.configuration === null;
  }
  /** Negative-only execution by the separately paid media/status owner. No current
   * callback, negotiation clock, native receipt or old SDK graph is retained here.
   * A throwing SDK close leaves the actual peer retained for an explicit cleanup retry. */
  close(): void {
    const state = mediaOwners.get(this)!;
    state.stopped = true;
    if (!state.peer) { state.closeComplete = true; return; }
    if (state.closeComplete || state.closing) return;
    state.closing = true; // Before a provider method getter can reenter close/drain.
    try {
      const close = state.peer.close;
      close.call(state.peer);
      state.closeComplete = true; // Actual synchronous SDK return, never a close intent.
    } finally { state.closing = false; }
  }
  /** No new SDK promise exists in this leaf. Future track/data-channel workers must
   * acquire their own payment and join here before native media Release is legal. */
  async drain(): Promise<void> {
    this.close();
    const state = mediaOwners.get(this)!;
    if (!state.closeComplete || state.closing) throw new Error("voice media close still running");
    state.peer = null;
    if (state.configuration) {
      releaseVoiceRtcConfiguration(state.configuration);
      state.configuration = null;
    }
  }
}
/** Opaque custody only: no actual PC, description or media/key handle is exposed. */
export type VoiceMediaRtcOwner = MediaRtcOwner;

export class VoiceSdpRtcOwner {
  #peer: RTCPeerConnection | null = null;
  #configuration: VoiceRtcConfigurationLease | null;
  #stopped = false;
  #peerClosed = false;
  #peerClosing = false;
  private localStarted = false;
  private localDone = false;
  private remoteStarted = false;
  private remoteDone = false;
  private localBytes: Uint8Array<ArrayBuffer> | null = null;
  private flight: Promise<void> | null = null;
  private gatherStop: (() => void) | null = null;
  private readonly localIsLow: boolean;
  #current: (() => boolean) | null;
  private readonly clock: VoiceSdpRtcClock;
  readonly #mediaIdentity = {};
  #mediaDestination: MediaRtcOwner | null = null;
  #transferChecking = false;
  constructor(localIsLow: boolean, current: () => boolean, clock: VoiceSdpRtcClock,
    configuration: VoiceRtcConfigurationLease | null = null) {
    this.#configuration = configuration;
    this.localIsLow = localIsLow; this.#current = current; this.clock = clock;
  } // Inert: no getter, Clock, constructor or SDK callback before publication.

  private live(): boolean {
    const observe = this.#current;
    const current = !this.#stopped && observe !== null && observe();
    return current && !this.#stopped && this.#current === observe &&
      (this.#configuration === null || voiceRtcConfigurationCurrent(this.#configuration)); // injected code may End.
  }

  /** Inert one-shot destination for this EXACT SDK peer identity. The trusted retained
   * broker must prepay its independent record BEFORE calling this or native media Admit.
   * Construction itself grants no native/RTC authority and performs no provider callback.
   * This leaf does not implement that broker payment or native handoff/status driver. */
  prepareMediaDestination(): VoiceMediaRtcOwner {
    if (this.#stopped) return unavailable();
    return this.#mediaDestination ??= new MediaRtcOwner(this.#mediaIdentity);
  }
  /** Trusted broker only, AFTER a fresh joined native media admission/current reply.
   * Perform the final callback outside the transfer tail; repeat every pure ownership
   * condition afterward because that callback can synchronously End or reenter us.
   * Historical native admission alone must never be used to call this transition. */
  transferInto(destination: VoiceMediaRtcOwner): void {
    if (this.#transferChecking) return unavailable();
    this.#transferChecking = true;
    let finished = false;
    // Existing SDP bookkeeping uses ordinary properties. Finish ALL such reads before
    // checking either owner's runtime-private gates: a hostile getter can synchronously
    // Close the inert destination, including its close-complete latch, while it is empty.
    try {
      if (!this.live()) return unavailable();
      finished = this.localDone && this.remoteDone && !this.flight && !this.gatherStop && !this.localBytes;
    } finally { this.#transferChecking = false; }
    const target = mediaOwners.get(destination);
    if (!finished || !target || target.identity !== this.#mediaIdentity || this.#mediaDestination !== destination ||
        target.adopted || target.stopped || target.peer !== null || target.configuration !== null || target.closing || target.closeComplete ||
        this.#stopped || this.#peerClosed || this.#peerClosing || !this.#peer) return unavailable();

    // COMMIT: private state writes only. No getter, method/provider/Clock callback,
    // allocation, await or fallible operation after the first mutation. The SAME actual
    // peer moves once; old close/drain loses all authority over it. Clear the old current
    // closure so retaining this stopped source cannot pin its channel/chain/phase graph.
    target.peer = this.#peer;
    target.configuration = this.#configuration;
    target.adopted = true;
    this.#peer = null;
    this.#configuration = null;
    this.#peerClosed = true;
    this.#stopped = true;
    this.#current = null;
    this.#mediaDestination = null;
  }
  private originalPeer(): RTCPeerConnection {
    if (!this.live()) return unavailable();
    if (this.#peer) return this.#peer;
    const Constructor = globalThis.RTCPeerConnection;
    if (!this.live() || typeof Constructor !== "function") return unavailable();
    try {
      const peer = this.#configuration
        ? voiceRtcConfiguredPeer(this.#configuration, Constructor, () => this.live())
        : new Constructor({ bundlePolicy: "max-bundle" });
      this.#peer = peer; // Publish actual identity before another provider callback.
      if (!this.live()) { this.close(); return unavailable(); }
      return peer;
    } catch (error) { this.close(); throw error; }
  }
  private start(work: () => Promise<void>): Promise<void> {
    if (this.flight) return this.flight;
    if (this.#stopped) return Promise.reject(new Error("original voice RTC peer unavailable"));
    // No SDK/provider callback runs until this actual observer is published. The channel
    // separately publishes its enclosing activity before calling this method.
    const flight = Promise.resolve().then(() => {
      if (!this.live()) return unavailable();
      return work();
    }).finally(() => { this.flight = null; });
    this.flight = flight; return flight;
  }

  /** Fresh same-original14 admission must FULLY finish its physical lane before this
   * method invokes setLocalDescription. An old PcReceipt/cryptoBinding is insufficient.
   * The void preparation keeps its UTF8 slot private; takeLocal is legal only after it joins. */
  prepareLocal(admit: () => Promise<number>, kind: "offer" | "answer"): Promise<void> {
    if (this.localStarted) return this.flight ?? Promise.resolve();
    if ((kind === "offer") !== this.localIsLow || (kind === "answer" && !this.remoteDone)) return unavailable();
    return this.start(async () => {
      const deadline = await admit(); // Original absolute deadline, never renewed by SDK work.
      let remaining = deadline - this.clock.now();
      if (!Number.isFinite(deadline) || !Number.isFinite(remaining) || remaining <= 0 || remaining > 30_000 || !this.live()) return unavailable();
      const peer = this.originalPeer();
      this.localStarted = true; // Includes transceiver setup; synchronous uncertainty cannot duplicate it.
      try {
        if (kind === "offer") {
          const add = peer.addTransceiver;
          if (!this.live()) return unavailable();
          add.call(peer, "audio", { direction: "sendrecv" });
        }
        const prepare: (this: RTCPeerConnection) => Promise<void> = peer.setLocalDescription;
        if (!this.live()) return unavailable(); // Method getters/provider hooks may End.
        await prepare.call(peer);
        if (!this.live()) return unavailable();
        remaining = deadline - this.clock.now();
        if (!Number.isFinite(remaining) || remaining <= 0 || !this.live()) return unavailable();
        await this.gather(peer, deadline);
        if (!this.live()) return unavailable();
        this.captureLocal(peer, kind);
        this.localDone = true;
      } catch (error) { this.close(); throw error; }
    });
  }
  /** Ends this entire getter/text frame synchronously, before its caller may start Begin.
   * No fulfilled description promise or async continuation retains the UTF16 text. */
  private captureLocal(peer: RTCPeerConnection, kind: "offer" | "answer"): void {
    let description: RTCSessionDescription | null = null;
    let text: string | null = null;
    let bytes: Uint8Array<ArrayBuffer> | null = null;
    try {
      description = peer.localDescription;
      if (!description || description.type !== kind || !this.live()) return unavailable();
      text = description.sdp;
      if (!this.live()) return unavailable();
      bytes = voiceSdpUtf8(text);
      if (!this.live()) return unavailable();
      this.localBytes = bytes; bytes = null;
    } finally { bytes?.fill(0); text = null; description = null; }
  }
  /** Affine transfer after the void preparation's finally. This grants no native effect. */
  takeLocal(): Uint8Array<ArrayBuffer> {
    if (this.flight || !this.localDone || !this.live() || !this.localBytes) return unavailable();
    const bytes = this.localBytes; this.localBytes = null; return bytes;
  }

  /** The caller must have joined a FRESH false26 RPC and repeated original currentness.
   * There is one SDK-started latch, independent of uncertain true26 metadata replies. */
  applyRemote(bytes: Uint8Array<ArrayBuffer>, kind: "offer" | "answer"): Promise<void> {
    if (this.remoteStarted || this.flight || (kind === "answer") !== this.localIsLow ||
        (kind === "answer" && !this.localDone) || bytes.byteLength > 8192) return unavailable();
    this.remoteStarted = true; // Before decode/current/provider callbacks can reenter.
    return this.start(async () => {
      let description: RTCSessionDescriptionInit | null = null;
      let text: string | null = null;
      try {
        const peer = this.originalPeer();
        text = decoder.decode(bytes); // Native Open UTF8 also checks; malformed never becomes AEAD failure.
        if (text.length > 8192 || !this.live()) return unavailable();
        description = { type: kind, sdp: text };
        const apply: (this: RTCPeerConnection, description: RTCSessionDescriptionInit) => Promise<void> = peer.setRemoteDescription;
        if (!this.live()) return unavailable(); // Final callback-free flags follow the last getter.
        await apply.call(peer, description);
        this.remoteDone = true; // Actual success only, even if End occurred during that await.
      } catch (error) { this.close(); throw error; }
      finally {
        if (description) description.sdp = "";
        description = null; text = null;
      }
    });
  }
  get applied(): boolean { return this.remoteDone; }
  get closed(): boolean { return this.#stopped; }

  /** CompleteSdpV1 sends the gathered description, never an unbounded early ICE queue.
   * This is an observer-only await; Close detaches it. It cannot stand in for joining the
   * actual setLocalDescription promise, which the enclosing flight already awaited. */
  private gather(peer: RTCPeerConnection, deadline: number): Promise<void> {
    return new Promise<void>((resolve, reject) => {
      let timer: unknown = null;
      let finished = false;
      const stop = (expired: boolean) => {
        if (finished) return;
        finished = true; this.gatherStop = null;
        let failure: unknown = null;
        try { peer.removeEventListener("icegatheringstatechange", changed); } catch (error) { failure = error; }
        try { if (timer !== null) this.clock.cancel(timer); } catch (error) { failure = error; }
        timer = null;
        let live = false;
        try {
          live = this.live();
          const now = this.clock.now();
          if (!Number.isFinite(now) || now >= deadline) expired = true;
        } catch (error) { failure = error; }
        if (failure || expired || !live) reject(failure ?? new Error("original voice RTC gathering ended")); else resolve();
      };
      const changed = () => {
        try {
          if (!this.live() || this.clock.now() >= deadline) stop(true);
          else if (peer.iceGatheringState === "complete") stop(false);
        } catch { stop(true); }
      };
      this.gatherStop = () => stop(true); // Publish before any event/Clock provider callback.
      try {
        peer.addEventListener("icegatheringstatechange", changed);
        // Registration/currentness are provider callbacks. Derive the timer from the
        // original absolute deadline AFTER them, never extend it by their elapsed time.
        const live = !finished && this.live();
        const remaining = deadline - this.clock.now();
        if (!finished && (!live || !Number.isFinite(remaining) || remaining <= 0)) stop(true);
        if (!finished) {
          const handle = this.clock.schedule(() => { this.close(); }, remaining);
          if (finished) this.clock.cancel(handle); else timer = handle;
        }
        if (!finished) changed();
      } catch (error) {
        this.gatherStop = null;
        if (timer !== null) this.clock.cancel(timer);
        try { peer.removeEventListener("icegatheringstatechange", changed); }
        finally { reject(error); }
      }
    });
  }
  /** Fence/close synchronously, but retain original Peer/flight until its actual SDK await
   * returns. A rejected/hanging SDK call is not made drained by close or timer expiry. */
  close(): void {
    this.#stopped = true;
    this.localBytes?.fill(0); this.localBytes = null;
    const stop = this.gatherStop; this.gatherStop = null;
    try { stop?.(); } catch { /* Preserve Close even if an injected observer throws. */ }
    if (this.#peer && !this.#peerClosed && !this.#peerClosing) {
      this.#peerClosing = true;
      try {
        const close = this.#peer.close;
        close.call(this.#peer);
        this.#peerClosed = true; // Actual SDK return, not merely a close intent.
      } catch { /* Retain the actual peer and payment; drain retries instead of inventing completion. */ }
      finally { this.#peerClosing = false; }
    }
  }
  async drain(): Promise<void> {
    this.close();
    const flight = this.flight;
    if (flight) {
      try { await flight; } catch { /* Actual SDK await returned; no success is invented. */ }
    }
    this.close(); // A failed close while SDK work was pending is independently retryable.
    if (this.#peer && (!this.#peerClosed || this.#peerClosing)) throw new Error("voice RTC close still running");
    this.#peer = null;
    this.#current = null;
    if (this.#configuration) {
      releaseVoiceRtcConfiguration(this.#configuration);
      this.#configuration = null;
    }
  }
}
