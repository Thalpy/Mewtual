/** Retained production bootstrap, separate from App activation and RTC track adoption.
 * Only bounded public metadata uses ordinary invoke. Key/Input/Result/SDP remain in
 * the original broker's authenticated private channel. No caller receives those owners.
 */
import type { VoiceCryptoOwner, VoiceIpcChannel, VoiceIpcClock, VoiceSelectedKeyHandle, VoiceJoinKeyHandle } from "./voice-crypto.ts";
import { VoiceHandleCell } from "./voice-handle-cell.ts";
import { VoiceSessionLifecycle, type VoiceSessionLifecycleObserver } from "./voice-session-lifecycle.ts";

// Complete conservative application-owned metadata inventory: controller/context1280,
// call/cell/handle1024, two candidate IDs/activation metadata1024, overlapping native
// RPC args/replies1024, startup/cleanup/wait promises1024, cadence2048, map/slack768.
// Framework IPC copies and opaque native/SDK heaps are outside this metadata payment.
export const VOICE_APP_SESSION_BYTES = 8 * 1024;
export type VoiceAppActivation = Readonly<{ owner: string; group: string | null; join: string | null }>;
export type VoiceAppNative = Readonly<{
  prepareController(): Promise<string>;
  prepareOwner(controller: string, server: number, room: string): Promise<string>;
  activate(candidate: string): Promise<VoiceAppActivation>;
  end(owner: string): Promise<void>;
}>;
export type VoiceAppCallPhase = "prepared" | "starting" | "uncertain" | "ready" | "closing" | "closed";
export type VoiceAppCall = Readonly<{
  start(): Promise<"ready" | "stopped" | "unavailable">;
  resume(): void;
  close(): Promise<void>;
  /** `ready` records authenticated bootstrap/lifecycle installation only. It is NOT
   * present-time native/RTC/media currency or a UI connected indicator. App must close
   * this original on lock/room/owner replacement; actual media keeps its own Current. */
  snapshot(): Readonly<{ phase: VoiceAppCallPhase }>;
}>;
export type VoiceAppCalls = Readonly<{
  /** Same live room returns the exact handle. A closing original must actually return
   * before a different call may be prepared; repeated clicks cannot queue call graphs. */
  prepare(server: number, room: string): VoiceAppCall;
  resume(): void;
  close(): Promise<void>;
}>;

/** Broker-only construction context. This is accounting/lifecycle, never native permission.
 * The broker reserves process payment BEFORE constructing this graph/first callback. */
export type VoiceAppSessionContext = Readonly<{
  clock: VoiceIpcClock;
  bindGroup(group: string): void;
  unbindGroup(): void;
  owner(token: string, group: string, current: () => boolean): VoiceCryptoOwner;
  select(owner: VoiceCryptoOwner): VoiceSelectedKeyHandle;
  channel(owner: VoiceCryptoOwner, key: VoiceJoinKeyHandle): VoiceIpcChannel;
  observe(owner: VoiceCryptoOwner, channel: VoiceIpcChannel, key: VoiceJoinKeyHandle): VoiceSessionLifecycleObserver;
  retire(owner: VoiceCryptoOwner): Promise<void>;
  resume(): void;
}>;
type StartResult = "ready" | "stopped" | "unavailable";
type Call = {
  server: number; room: string; phase: VoiceAppCallPhase; stopped: boolean; finalising: boolean;
  controller: string | null; candidate: string | null; owner: VoiceCryptoOwner | null;
  channel: VoiceIpcChannel | null; selected: VoiceSelectedKeyHandle | null;
  worker: Promise<StartResult> | null; closing: Promise<void> | null;
  cadence: VoiceSessionLifecycle | null; generation: object; wake: number; waiting: { promise: Promise<void>; resolve(): void } | null;
  cell: VoiceHandleCell<VoiceAppCall>; handle: VoiceAppCall;
};
const stopped = Promise.resolve<StartResult>("stopped"), returned = Promise.resolve();
const closed = Object.freeze({ phase: "closed" as const });
const id = (value: unknown): value is string => typeof value === "string" && /^voice-[0-9a-f]{16}$/.test(value);
const hash = (value: unknown): value is string => typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
function activation(value: unknown, candidate: string, controller: boolean): value is VoiceAppActivation {
  if (!value || typeof value !== "object" || Object.keys(value).sort().join(",") !== "group,join,owner") return false;
  const result = value as VoiceAppActivation;
  return result.owner === candidate && (controller ? result.group === null && result.join === null : hash(result.group) && hash(result.join));
}
// Public handles capture only the detachable cell, never the call/owner/native graph.
function callHandle(cell: VoiceHandleCell<VoiceAppCall>): VoiceAppCall {
  return Object.freeze({
    start: () => cell.dispatch(target => target.start(), stopped),
    resume: () => cell.dispatch(target => target.resume(), undefined),
    close: () => cell.dispatch(target => target.close(), returned),
    snapshot: () => cell.dispatch(target => target.snapshot(), closed),
  });
}

/** One window-retained controller façade; one active/closing call, one startup task,
 * one cleanup task and one separately paid cadence. Constructor is inert. */
export class VoiceAppSession {
  readonly handle: VoiceAppCalls;
  private current: Call | null = null;
  private readonly native: VoiceAppNative;
  private readonly context: VoiceAppSessionContext;
  constructor(native: VoiceAppNative, context: VoiceAppSessionContext) {
    this.native = native; this.context = context;
    this.handle = Object.freeze({ prepare: (server, room) => this.prepare(server, room),
      resume: () => { if (this.current) this.resume(this.current); },
      close: () => this.current ? this.close(this.current) : returned });
  }
  private prepare(server: number, room: string): VoiceAppCall {
    if (!Number.isSafeInteger(server) || server <= 0 || typeof room !== "string" || !/^(0|[1-9][0-9]{0,38})$/.test(room) || BigInt(room) >= (1n << 128n))
      throw new Error("invalid voice room");
    const previous = this.current;
    if (previous) {
      if (!previous.stopped && previous.server === server && previous.room === room) return previous.handle;
      throw new Error("previous voice call has not finished closing");
    }
    const call = { server, room, phase: "prepared", stopped: false, finalising: false,
      controller: null, candidate: null, owner: null, channel: null, selected: null,
      worker: null, closing: null, cadence: null, generation: {}, wake: 0, waiting: null, cell: null!, handle: null! } as Call;
    call.cell = new VoiceHandleCell(Object.freeze({ start: () => this.start(call), resume: () => this.resume(call),
      close: () => this.close(call), snapshot: () => Object.freeze({ phase: call.phase }) }));
    call.handle = callHandle(call.cell); this.current = call;
    return call.handle;
  }
  private resume(call: Call): void {
    if (this.current !== call || call.finalising) return;
    call.wake++; const waiting = call.waiting; call.waiting = null; waiting?.resolve(); this.context.resume();
  }
  private wait(call: Call, observed: number): Promise<void> {
    if (call.wake !== observed) return returned;
    if (!call.waiting) {
      let release!: () => void;
      const promise = new Promise<void>(resolve => { release = resolve; });
      call.waiting = { promise, resolve: release };
    }
    return call.waiting.promise;
  }
  private start(call: Call): Promise<StartResult> {
    if (call.stopped) return stopped;
    if (call.worker) return call.worker;
    call.phase = "starting";
    // Publish before any Clock/invoke/owner callback. The cadence only wakes SAME work;
    // its stage metadata grants no native operation or fresh allowance.
    call.worker = Promise.resolve().then(() => this.run(call));
    call.cadence = new VoiceSessionLifecycle({ live: () => !call.finalising,
      observe: () => ({ generation: call.generation, plan: null, stage: "confirmation" }),
      step: () => { this.resume(call); return returned; } }, this.context.clock);
    call.cadence.start();
    return call.worker;
  }
  private async activate(call: Call, candidate: string, controller: boolean): Promise<VoiceAppActivation | null> {
    while (!call.stopped) {
      const observed = call.wake;
      let reply: unknown;
      try { reply = await this.native.activate(candidate); } catch { /* Original candidate only. */ }
      if (call.stopped) return null;
      if (activation(reply, candidate, controller)) return reply;
      call.phase = "uncertain"; await this.wait(call, observed);
    }
    return null;
  }
  private async run(call: Call): Promise<StartResult> {
    try {
      if (call.stopped) return "stopped";
      const controller = await this.native.prepareController();
      if (!id(controller)) throw new Error("invalid native voice controller");
      call.controller = controller;
      if (call.stopped || !await this.activate(call, controller, true)) return "stopped";
      const candidate = await this.native.prepareOwner(controller, call.server, call.room);
      if (!id(candidate)) throw new Error("invalid native voice owner");
      call.candidate = candidate;
      if (call.stopped) return "stopped";
      const accepted = await this.activate(call, candidate, false);
      if (!accepted || call.stopped) return "stopped";
      this.context.bindGroup(accepted.group!); // SAME process fee, before any key admission.
      call.owner = this.context.owner(candidate, accepted.group!, () => !call.stopped && this.current === call);
      if (call.stopped) return "stopped";
      call.selected = this.context.select(call.owner);
      while (!call.stopped) {
        const observed = call.wake, result = await call.selected.attempt();
        if (call.stopped) return "stopped";
        if (result.kind === "accepted") {
          call.channel = this.context.channel(call.owner, result.key);
          while (!call.stopped) {
            const before = call.wake, status = await call.channel.status();
            if (call.stopped) return "stopped";
            if (status.kind === "ready") {
              this.context.observe(call.owner, call.channel, result.key);
              if (call.stopped) return "stopped";
              call.phase = "ready"; return "ready";
            }
            if (status.kind !== "uncertain") throw new Error("native voice channel stopped");
            call.phase = "uncertain"; await this.wait(call, before);
          }
        } else if (result.kind !== "busy" && result.kind !== "uncertain") throw new Error("native voice key stopped");
        call.phase = "uncertain"; await this.wait(call, observed);
      }
      return "stopped";
    } catch {
      // Prepare loss has no activation identity: never manufacture an automatic fresh
      // candidate. Any unknown native candidate remains inert under its bounded TTL.
      const wasStopped = call.stopped;
      void this.close(call); return wasStopped ? "stopped" : "unavailable";
    }
  }
  private close(call: Call): Promise<void> {
    if (call.closing) return call.closing;
    call.stopped = true; call.phase = "closing";
    call.closing = Promise.resolve().then(() => this.cleanup(call)); // Before owner callbacks.
    if (call.owner) void this.context.retire(call.owner);
    call.wake++; const waiting = call.waiting; call.waiting = null; waiting?.resolve(); this.context.resume();
    return call.closing;
  }
  private async endKnown(call: Call): Promise<void> {
    // Controller revocation also fences a still-unknown call-prepare result. Start this
    // BEFORE joining activation; repeat after its actual return to cover both orderings.
    for (const candidate of [call.controller, call.candidate]) {
      if (!candidate) continue;
      for (;;) {
        const observed = call.wake;
        try { await this.native.end(candidate); break; }
        catch { await this.wait(call, observed); }
      }
    }
  }
  private async cleanup(call: Call): Promise<void> {
    await this.endKnown(call);
    if (call.worker) await call.worker;
    await this.endKnown(call);
    // This join finds internally retained keys/channels even if no handle was returned.
    if (call.owner) await this.context.retire(call.owner);
    call.finalising = true;
    if (call.cadence) await call.cadence.close();
    await call.cell.detach();
    call.owner = null; call.channel = null; call.selected = null; call.cadence = null;
    call.controller = null; call.candidate = null; call.worker = null; call.waiting = null;
    this.context.unbindGroup();
    call.phase = "closed";
    if (this.current === call) this.current = null;
  }
}
