/** Serial consumption of existing native work. Metadata selects a retained descriptor;
 * authenticated Input/admission/result/Return still decide every actual effect. This is
 * deliberately one original pair, not the multi-peer/SDP or replay-events adapter. */
import type {
  VoiceCryptoChain, VoiceChannelDerivationHandle, VoiceChannelDerivationOutcome,
  VoiceChannelConfirmationHandle, VoiceIpcClock,
} from "./voice-crypto.ts";
import { decodeVoiceWorkSnapshot, type VoiceWorkDescriptor } from "./voice-ipc-work.ts";

type Attached = Extract<VoiceWorkDescriptor, { state: "attached" }>;
type Ready = Extract<VoiceChannelDerivationOutcome, { kind: "ready" }>;
export type VoiceNativeWorkObserver = Readonly<{
  /** Irreversible. Resolves after the actual current metadata/attempt call returns;
   * channel End separately drains the underlying SDK/settlement owners. */
  close(): Promise<void>;
}>;
export type NativeWorkActions = Readonly<{
  owner: string; executor: string;
  live(): boolean;
  read(fromSlot: number): Promise<unknown>;
  derive(work: Attached): VoiceChannelDerivationHandle;
  returnDerivation(chain: VoiceCryptoChain): Promise<void>;
  confirmation(chain: VoiceCryptoChain, work: Attached): VoiceChannelConfirmationHandle;
  returned(chain: VoiceCryptoChain, child: VoiceChannelConfirmationHandle): boolean;
}>;

/** One prepaid metadata graph, parent and current child, one task and one wake/timer.
 * The retained channel funds these before setup ACK. Never hold SharedIpcExecution while
 * awaiting this driver: its original handles need that same lane for SDK/RPC drainage. */
export class NativeWorkPump {
  readonly handle: VoiceNativeWorkObserver;
  private actions: NativeWorkActions | null;
  private readonly clock: VoiceIpcClock;
  private stopped = false;
  private task: Promise<void> | null = null;
  private wakeWait: (() => void) | null = null;
  private wakeVersion = 0;
  private completed = 0;
  private parent: { work: Attached; intake: VoiceChannelDerivationHandle | null; ready: Ready | null; returned: boolean } | null = null;
  private child: { work: Attached; handle: VoiceChannelConfirmationHandle } | null = null;
  private terminal = false;

  constructor(actions: NativeWorkActions, clock: VoiceIpcClock) {
    this.actions = actions; this.clock = clock;
    this.handle = Object.freeze({ close: () => this.close() });
  }
  start(): void {
    // Install on the channel before this microtask can invoke any native/owner callbacks.
    this.task ??= Promise.resolve().then(() => this.drive());
  }
  wake(): void { this.wakeVersion++; this.wakeWait?.(); }
  close(): Promise<void> { this.stopped = true; this.wake(); return this.task ?? Promise.resolve(); }
  private live(): boolean {
    return !this.stopped && this.actions !== null && this.actions.live() && !this.stopped &&
      (!this.parent?.ready || this.parent.ready.chain.isCurrent()) && !this.stopped;
  }
  private async drive(): Promise<void> {
    try {
      while (this.live()) {
        const observed = this.wakeVersion;
        try { if (!this.terminal) await this.turn(); }
        catch { /* Keep the same descriptor/handle on capacity, malformed data or reply loss. */ }
        if (this.live()) await this.wait(observed);
      }
    } catch { /* A failed scheduler/current predicate stops adoption; End still owns cleanup. */ }
    finally {
      this.stopped = true; this.wakeWait = null;
      // Cleanup now owns the serial metadata payment. Do not retain a second decoded
      // parent/child graph or the broker callbacks behind a collected public observer.
      this.parent = null; this.child = null; this.actions = null;
    }
  }
  private async turn(): Promise<void> {
    const actions = this.actions!;
    if (this.parent && !this.parent.returned) {
      const parent = this.parent;
      parent.intake ??= actions.derive(parent.work);
      if (!parent.ready) {
        const outcome = await parent.intake.attempt();
        if (!this.live()) return;
        if (outcome.kind === "ready") parent.ready = outcome;
        else {
          // A past receipt cannot reconstruct browser keys. Only temporary pre-effect
          // refusal keeps the SAME intake eligible; all terminal outcomes pin this parent.
          this.terminal = !(outcome.kind === "uncertain" ||
            (outcome.kind === "refused" && (outcome.code === "capacity" || outcome.code === "stopped")));
          return;
        }
      }
      const outcome = await parent.ready.derivation.attempt();
      if (!this.live()) return;
      if (outcome.kind !== "accepted") {
        this.terminal = outcome.kind !== "uncertain" && outcome.kind !== "busy"; return;
      }
      // Acceptance resolves before actual settlement. Keep keys/counters/deadline and wait
      // for original physical drainage AND native ordinary Return before selecting children.
      await actions.returnDerivation(parent.ready.chain);
      if (!this.live()) return;
      parent.returned = true; this.complete(parent.work);
    }
    if (this.child) {
      const child = this.child;
      await child.handle.attempt();
      if (!this.live()) return;
      // An uncertain observation may precede an ongoing SDK/result/Return. Never replace
      // it or scan another child until the exact retained intake has actually Returned.
      if (!actions.returned(this.parent!.ready!.chain, child.handle)) return;
      this.complete(child.work); this.child = null;
    }
    let fromSlot = 0;
    while (fromSlot < 32 && this.live()) {
      const raw = await actions.read(fromSlot);
      const snapshot = decodeVoiceWorkSnapshot(raw, actions.owner, actions.executor, fromSlot);
      if (!this.live()) return; // callbacks/decoding never override End/current ownership
      fromSlot = snapshot.nextSlot;
      const work = snapshot.work;
      if (!work || work.state !== "attached" || (this.completed & (1 << (fromSlot - 1))) !== 0) continue;
      if (!this.parent) {
        if (work.purpose.kind !== "derive") continue;
        // Pin BEFORE a constructor or owner predicate can fail/reenter. A failed/historical
        // parent never causes fresh keys or another original intake on this channel.
        this.parent = { work, intake: null, ready: null, returned: false }; return;
      }
      if (work.purpose.kind !== "confirmation" || work.purpose.parent !== this.parent.work.row) continue;
      const handle = actions.confirmation(this.parent.ready!.chain, work);
      if (!this.live()) return;
      this.child = { work, handle }; return;
    }
  }
  private complete(work: Attached): void {
    // Only attached histories completed by their original handles are stable here.
    // Proven-unstarted Pending rows can be removed/reused and must NEVER enter this bitmap.
    this.completed |= 1 << Number(BigInt("0x" + work.row.slice(0, 16)));
  }
  private async wait(observed: number): Promise<void> {
    const began = this.clock.now(), earliest = began + 50, cadence = began + 250;
    let active = true, timer: unknown = null, resolve!: () => void;
    const waiting = new Promise<void>(yes => { resolve = yes; });
    const check = () => {
      if (!active) return;
      const deadline = this.stopped ? -Infinity : this.wakeVersion !== observed ? earliest : cadence;
      const remaining = deadline - this.clock.now();
      if (remaining <= 0) resolve();
      else {
        if (timer !== null) this.clock.cancel(timer);
        timer = this.clock.schedule(check, remaining);
      }
    };
    this.wakeWait = check;
    try { check(); await waiting; }
    finally {
      active = false; if (timer !== null) this.clock.cancel(timer);
      if (this.wakeWait === check) this.wakeWait = null;
    }
  }
}
