/** One prepaid original-peer media handoff on the retained authenticated IPC Session.
 * One-pair negotiation can retire into independent native custody; shared Session and
 * old browser history has a separate explicit compaction transaction. This driver stays
 * paid through actual peer/native/IO cleanup and does not admit another pair. The opaque destination and
 * passive status contain no driver/old-chain callbacks; only this paid driver does. */
import { encodeVoiceMediaOriginal, encodeVoiceMediaExisting, type VoiceMediaReceipt } from "./voice-ipc-media.ts";
import type { VoiceSdpOriginal } from "./voice-ipc-sdp-actions.ts";
import { VoiceMediaStatus, voiceMediaDeadline } from "./voice-media-status.ts";
import type { VoiceMediaRtcOwner, VoiceSdpRtcOwner, VoiceSdpRtcClock } from "./voice-sdp-rtc.ts";

/** Fixed strings, one peer wrapper, one flight/monitor/callback cell and overlap with
 * the existing physical Control codec. Opaque browser RTC heap is outside this bound. */
export const VOICE_NATIVE_MEDIA_BYTES = 8 * 1024;
type Request = ReturnType<typeof encodeVoiceMediaOriginal>;
export type VoiceNativeMediaContext = Readonly<{
  clock: VoiceSdpRtcClock;
  live(): boolean;
  request(request: Request, cleanup: boolean): Promise<VoiceMediaReceipt | null>;
  stopped(): void;
  begin(): void; end(): void; refund(): void;
}>;
type TimerCell = { work: ((generation: object) => void) | null };
const unavailable = (): never => { throw new Error("original native media unavailable"); };

export class VoiceNativeMedia {
  readonly #context: VoiceNativeMediaContext;
  readonly #original: VoiceSdpOriginal;
  #source: VoiceSdpRtcOwner | null;
  #peer: VoiceMediaRtcOwner | null = null;
  #status: VoiceMediaStatus | null = null;
  #proof: string | null = null;
  #attempted = false;
  #admitted = false;
  #nativeReleased = false;
  #detachRequested = false;
  #detached = false;
  #historyRequested = false;
  #historyCollected = false;
  #historyObservation: Promise<boolean> | null = null;
  #closed = false;
  #refunded = false;
  #transferring = false;
  #flight: Promise<void> | null = null;
  #cleaning: Promise<void> | null = null;
  #acquireUntil: number | null = null;
  #due = 0;
  #lastObserved = -Infinity;
  #timer: unknown = null;
  #timerSet = false;
  #arming = false;
  #installing = false;
  #cancelling = false;
  #synchronousFire = false;
  #callbackDepth = 0;
  readonly #cell: TimerCell = { work: null };
  #timerGeneration: object | null = null;
  #registrationUnknown = false;
  constructor(original: VoiceSdpOriginal, source: VoiceSdpRtcOwner, context: VoiceNativeMediaContext) {
    // The caller prepays BEFORE construction. Snapshot only fixed public bindings,
    // never a mutable caller graph, key, browser chain or fulfilled description promise.
    this.#original = Object.freeze({ parentRow: original.parentRow,
      identity: Object.freeze({ action: original.identity.action, version: original.identity.version, inputTag: original.identity.inputTag }),
      handshake: original.handshake, key: original.key, sequence: original.sequence, storage: original.storage,
      sdkGeneration: original.sdkGeneration, sdkProof: original.sdkProof, pcProof: original.pcProof });
    this.#source = source; this.#context = context;
    // Notifications retain only one detachable cell and their inert identity, never
    // the old graph directly. Identity also fences a late old callback from paying off
    // a newer schedule call that registered a timer but threw before returning its handle.
    this.#cell.work = generation => this.#onTimer(generation);
  } // Inert: no Clock, provider, SDK, native or owner callback before publication.
  get adopted(): boolean { return this.#peer?.adopted === true; }
  get closed(): boolean { return this.#closed; }
  get detached(): boolean { return this.#detached; }
  /** Ask for the same native successor. Source expiry alone is insufficient: only
   * positive authenticated mode7 after actual peer adoption publishes detached=true. */
  requestDetach(): Promise<void> {
    if (!this.adopted || this.#closed) return unavailable();
    this.#detachRequested = true; return this.start();
  }
  /** Historical native-row removal only. This neither closes the peer nor refunds
   * browser fixed history. One existing status flight/timer owns lost/pending replies;
   * exact repetition observes the SAME paid native tombstone without new admission. */
  collectNativeHistory(): Promise<boolean> {
    if (this.#historyObservation) return this.#historyObservation;
    if (this.#historyCollected) return Promise.resolve(true);
    if (!this.adopted || !this.#detached || this.#closed) return unavailable();
    this.#historyRequested = true;
    // Publish the ONE observer before start can run Clock/provider/liveness callbacks.
    // Repeated callers must not attach reactions to an abort-resistant shared flight.
    const observation = Promise.resolve().then(async () => {
      try { await this.start(); return this.#historyCollected; }
      finally { this.#historyObservation = null; }
    });
    this.#historyObservation = observation;
    return observation;
  }
  /** Used ONLY by the original RTC current closure during the synchronous transfer.
   * The flag is set after an actual same-record Current physical finally. It is never
   * renderer connected state, never Source completion, and is not retained by the peer. */
  transferCurrent(source: VoiceSdpRtcOwner): boolean | null {
    if (!this.#transferring) return null;
    // During transfer this result is authoritative, INCLUDING refusal. Falling back to
    // old Ready after a failed media check would bypass the new freshness boundary.
    if (this.#source !== source || this.#closed || !this.#status) return false;
    const live = this.#context.live(), now = this.#now();
    return this.#transferring && this.#source === source && !this.#closed && live && this.#status.currentAt(now);
  }
  #observe(receipt: VoiceMediaReceipt | null): void {
    if (!receipt) return;
    if (this.#proof !== null && this.#proof !== receipt.proof) { this.close(); return unavailable(); }
    this.#proof = receipt.proof;
    this.#admitted ||= receipt.admitted;
    this.#nativeReleased ||= receipt.released;
    if (receipt.closed || receipt.released) this.close();
  }
  async #rpc(request: Request, cleanup: boolean): Promise<VoiceMediaReceipt | null> {
    try { return await this.#context.request(request, cleanup); }
    finally { request.body.fill(0); }
  }
  /** Every observer, including a timer firing during hung IO, rejects invalid or
   * rolled-back time. Otherwise a consumed one-shot could leave the peer unmonitored. */
  #now(): number {
    const now = this.#context.clock.now();
    if (!Number.isFinite(now) || now < this.#lastObserved) { this.close(); return unavailable(); }
    this.#lastObserved = now; return now;
  }
  #deadline(now: number): number {
    const deadline = voiceMediaDeadline(now);
    if (deadline === null) return unavailable();
    return deadline;
  }
  /** One finite observer flight, not a promise that lives until the call ends. Both
   * acquisition and refresh publish ownership before provider/Clock callbacks run. */
  start(): Promise<void> {
    if (this.#flight) return this.#flight;
    if (this.#closed || this.#refunded) return Promise.resolve();
    this.#context.begin();
    const flight = Promise.resolve().then(() => this.#run()).catch(() => {
      // An unknown reply retains original obligations and may retry inside the SAME
      // acquisition/freshness window. Neither computation nor deadline is recreated.
    }).finally(() => {
      try { this.#context.end(); } finally { this.#flight = null; }
      if (!this.#closed) {
        try {
          const now = this.#now();
          const until = this.#status?.deadline ?? this.#acquireUntil;
          if (until === null || now >= until) this.close();
          else { this.#due = Math.min(until, now + 1000); this.#arm(); }
        } catch { this.close(); }
      }
    });
    this.#flight = flight; return flight;
  }
  async #run(): Promise<void> {
    if (this.#closed || !this.#context.live() || this.#closed) return;
    const now = this.#now();
    this.#acquireUntil ??= this.#deadline(now);
    const until = this.#status?.deadline ?? this.#acquireUntil;
    if (now >= until) { this.close(); return; }
    this.#due = until; this.#arm(); // Acquisition expiry also covers hanging first Admit.
    if (this.#closed) return;
    this.#peer ??= this.#source!.prepareMediaDestination();
    if (this.#closed || !this.#context.live() || this.#closed) return;
    if (!this.#admitted) {
      if (this.#attempted) {
        this.#observe(await this.#rpc(encodeVoiceMediaOriginal(this.#original, "resolve"), true));
      }
      if (this.#closed) return;
      if (!this.#admitted) {
        this.#attempted = true; // BEFORE encoding/crypto/transport can become uncertain.
        this.#observe(await this.#rpc(encodeVoiceMediaOriginal(this.#original, "admit"), false));
      }
      if (this.#closed || !this.#admitted || this.#proof === null) return;
    }
    this.#status ??= new VoiceMediaStatus(this.#proof!);
    const token = this.#status.begin(this.#now());
    let receipt: VoiceMediaReceipt | null = null;
    try {
      receipt = await this.#rpc(encodeVoiceMediaExisting(this.#original.parentRow, this.#original.identity, this.#proof!, "current"), false);
    } finally {
      let observed = NaN;
      try { observed = this.#now(); } catch { /* Fail closed and retire this exact token. */ }
      this.#status.finish(token, receipt, observed);
    }
    this.#observe(receipt);
    const observed = this.#now();
    if (!this.#peer!.adopted && observed >= this.#acquireUntil!) { this.close(); return; }
    const current = this.#status.currentAt(observed);
    if (this.#closed || this.#status.closed || !this.#context.live() || this.#closed) { this.close(); return; }
    if (!current) return; // Initial capacity refusal may retry within original acquisition.
    if (!this.#peer!.adopted) {
      this.#transferring = true;
      try { this.#source!.transferInto(this.#peer!); }
      finally { this.#transferring = false; }
      this.#source = null; // Actual peer and old current closure have moved/severed.
    }
    if (this.#detachRequested && !this.#detached && receipt?.retireInMs === 0) {
      const detachedToken = this.#status.begin(this.#now());
      let indexed: VoiceMediaReceipt | null = null;
      try { indexed = await this.#rpc(encodeVoiceMediaExisting(this.#original.parentRow, this.#original.identity, this.#proof!, "detach"), false); }
      finally {
        let at = NaN; try { at = this.#now(); } catch { /* Close already latched. */ }
        this.#status.finish(detachedToken, indexed, at);
      }
      this.#observe(indexed);
      if (!this.#closed && this.#status.currentAt(this.#now()) && indexed?.current && indexed.retireInMs === 0) this.#detached = true;
    }
    if (!this.#closed && this.#detached && this.#historyRequested && !this.#historyCollected) {
      // Historical cleanup cannot refresh freshness. The ordinary Current above and
      // its unchanged request-start deadline remain monitored while this RPC is held.
      const history = await this.#rpc(encodeVoiceMediaExisting(this.#original.parentRow,
        this.#original.identity, this.#proof!, "retirehistory"), true);
      this.#observe(history);
      if (history?.admitted && !history.current && history.retireInMs === null) this.#historyCollected = true;
    }
  }
  #cancel(): void {
    if (!this.#timerSet || this.#cancelling) return;
    // Keep the handle on failure so cleanup can retry actual cancellation. No new
    // registration may be installed while this original one remains uncertain.
    this.#cancelling = true;
    try {
      this.#context.clock.cancel(this.#timer);
      this.#timerSet = false; this.#timer = null;
    } finally { this.#cancelling = false; }
  }
  #arm(): void {
    if (this.#closed || this.#arming) return;
    // Fence the WHOLE replacement. cancel/now can synchronously deliver an OLD
    // notification; it must not install a second timer whose handle we overwrite.
    this.#arming = true; this.#synchronousFire = false;
    try {
      this.#cancel();
      if (this.#closed) return;
      const now = this.#now();
      if (this.#closed) return; // Clock callbacks can synchronously End this owner.
      let remaining = this.#due - now;
      if (!Number.isFinite(remaining)) { this.close(); return; }
      if (remaining <= 0) {
        // An early callback can cross the polling cadence during cancellation/Clock
        // entry. That cadence is not expiry. Defer ONE ordinary refresh wake without
        // changing due, the native observation or its original absolute deadline.
        const until = this.#status?.deadline ?? this.#acquireUntil;
        if (until === null || !Number.isFinite(until) || now >= until) { this.close(); return; }
        remaining = Math.min(1, until - now);
      }
      this.#installing = true;
      const generation = {}, cell = this.#cell;
      this.#timerGeneration = generation;
      this.#registrationUnknown = true; // BEFORE the scheduler can register then throw.
      try {
        const handle = this.#context.clock.schedule(() => cell.work?.(generation), remaining);
        this.#timer = handle; this.#timerSet = true; this.#registrationUnknown = false;
      } catch (error) {
        // There is no safe handle to cancel. Never arm again or refund while that
        // one-shot registration is uncertain; only its SAME actual callback can join it.
        this.close(); throw error;
      } finally { this.#installing = false; }
    } finally { this.#arming = false; }
    if (this.#closed || this.#synchronousFire) {
      // A NEW registration that fires inline cannot stand in for a future deadline.
      this.close();
    }
  }
  #onTimer(generation: object): void {
    if (generation !== this.#timerGeneration) return;
    if (this.#arming) {
      if (this.#installing) { this.#synchronousFire = true; this.#registrationUnknown = false; }
      return;
    }
    this.#callbackDepth++;
    try {
      this.#registrationUnknown = false; // SAME uncertain one-shot actually returned here.
      if (this.#closed) return;
      const now = this.#now();
      if (this.#closed) return;
      if (now < this.#due) {
        // Timers may fire slightly early. Keep the SAME absolute due time and replace
        // its consumed registration; an old queued notification is equally harmless.
        this.#arm(); return;
      }
      const until = this.#status?.deadline ?? this.#acquireUntil;
      if (until === null || now >= until || !this.#context.live()) { this.close(); return; }
      if (!this.#flight) void this.start();
      else {
        // A joined Current may have refreshed the status while a later detach RPC
        // in this SAME flight is pending. Replace the consumed old-deadline timer.
        this.#due = until; this.#arm();
      }
    } catch { this.close(); }
    finally { this.#callbackDepth--; }
  }
  /** Negative first: stop the real peer BEFORE awaiting abort-resistant IO, SDK or
   * timer retirement. Cancellation failure retains its handle and this driver's fee. */
  close(): void {
    const first = !this.#closed;
    this.#closed = true; this.#status?.close();
    try {
      // Publish the owning negotiation's negative state before SDK/cancel callbacks
      // can reenter it. The private closed/status fence already holds here. Even a
      // throwing owner callback must not prevent the real SDK/timer close attempts.
      if (first) this.#context.stopped();
    } finally {
      try { this.#source?.close(); }
      finally {
        try { this.#peer?.close(); } catch { /* Actual custody survives for cleanup retry. */ }
        try { this.#cancel(); } catch { /* Keep the original uncertain handle. */ }
      }
    }
  }
  /** Called OUTSIDE the shared physical lane, BEFORE unseen-parent/old SDK cleanup.
   * A lost Release recovers the exact released claim via original Close; no refill or
   * replacement claim is created. Refund follows actual callbacks/IO/SDK and native ACK. */
  cleanup(): Promise<void> {
    if (this.#cleaning) return this.#cleaning;
    const work = Promise.resolve().then(() => this.#cleanup()).finally(() => { this.#cleaning = null; });
    this.#cleaning = work; return work;
  }
  async #cleanup(): Promise<void> {
    this.close();
    const historyObservation = this.#historyObservation;
    if (this.#flight) {
      try { await this.#flight; } catch { /* Actual flight returned; recover original native uncertainty below. */ }
    }
    if (historyObservation) await historyObservation; // Actual observer finally precedes refund.
    this.#cancel();
    if (this.#source) await this.#source.drain();
    if (this.#peer) await this.#peer.drain();
    if (this.#attempted && !this.#nativeReleased) {
      const closed = await this.#rpc(encodeVoiceMediaOriginal(this.#original, "close"), true);
      this.#observe(closed);
      if (!closed?.closed) return unavailable();
      if (!this.#nativeReleased) {
        const released = await this.#rpc(encodeVoiceMediaOriginal(this.#original, "release"), true);
        this.#observe(released);
        if (!released?.released) return unavailable();
      }
    }
    // Join synchronous timer/cancel/provider/consumer finally frames, including a
    // reentrant cleanup launched inside their callbacks, before releasing payment.
    await Promise.resolve();
    if (this.#arming || this.#cancelling || this.#callbackDepth !== 0 || this.#flight || this.#historyObservation || this.#timerSet || this.#registrationUnknown) return unavailable();
    this.#cell.work = null;
    this.#source = null; this.#peer = null;
    if (!this.#refunded) { this.#refunded = true; this.#context.refund(); }
  }
}
