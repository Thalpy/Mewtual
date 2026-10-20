/**
 * Renders a jam take (a Studio score's playback, or a `.jamtake`) to an `AudioBuffer`, faster than
 * real time, so the clip maker can lay music made in Studio under a video.
 *
 * It is the SAME synthesis Studio plays: the real `JamEngine` and its archival transport, driven by
 * an `OfflineAudioContext` instead of the speakers. There is deliberately no second synth here; a
 * clip's music would otherwise drift from what its author heard while writing it.
 *
 * The engine plays every event "now" (`context.currentTime`), because live play has no future to
 * schedule into. Offline, "now" is moved by suspending the render at each event's time, dispatching
 * the events that fall in that render quantum, and resuming:
 *
 *   suspend(t0) -> play events at t0 -> resume -> ... suspend(t1) -> play events at t1 -> resume
 *
 * Details that matter, each enforced below:
 * - Events are grouped per 128-frame render quantum: a render can only stop on quantum boundaries
 *   and refuses two suspends in the same quantum. Grouping rounds DOWN, so a note can start up to
 *   one quantum (2.7 ms at 48 kHz) early; never late, and never out of order.
 * - Each suspend asks for the middle of its first sample, not its edge. The browser floors the
 *   requested time to a frame, and `frame / rate * rate` can land a hair under `frame` and fall into
 *   the previous quantum, which already has a suspend and would reject.
 * - Drum hits resolve asynchronously (their noise is seeded by a digest). Every result is awaited
 *   before resuming, so a hit cannot land a quantum late.
 * - The engine refuses remote events unless its context is "running"; a suspended offline context
 *   reports "suspended" during exactly the window events are dispatched in. `runningView` presents
 *   the offline context as running to the engine and nothing else. Nothing in the engine's trust
 *   model rests on that check (it only stops a muted window from queueing sound), and the take has
 *   passed the same validation (`planJamArchivalPlayback`) Studio applies.
 * - The engine's clock is the render clock (`currentTime`), so its watchdogs and drum deadlines
 *   count render time, not the wall time the render happens to take.
 *
 * What is repeatable: every event lands in the same render quantum every time, so two renders of
 * a take are the same music. They are not bit-identical: Chromium's audio processing differs by
 * float rounding between renders (at most about 1e-7, around -140 dB, measured in Edge), which
 * nothing downstream can hear or depends on.
 */
import { JamEngine } from "./jam-engine.ts";
import { planJamArchivalPlayback, type JamArchivalPlan } from "./jam-archival-plan.ts";
import { TAKE_MAX_DURATION_MS } from "./jam-contract.ts";
import { completionMs, DROPPED_EVENT } from "./studio-playback.ts";

/** Web Audio renders in blocks of this many frames; suspends land on block boundaries. */
export const RENDER_QUANTUM = 128;
/** The rate clip audio is mixed at. Music is rendered straight at it, so it is never resampled. */
export const CLIP_SAMPLE_RATE = 48_000;

/** One render stop: the quantum-aligned frame to stop at, and the plan events played there. */
export type RenderStep = { readonly frame: number; readonly events: readonly number[] };

/**
 * Group a plan's events (ascending by `ms`, as the planner emits them) into render stops. Events
 * at or after `lengthFrames` cannot be stopped for and are left out: the render ends before them.
 */
export function renderSteps(events: readonly { readonly ms: number }[], sampleRate: number, lengthFrames: number): RenderStep[] {
  const steps: RenderStep[] = [];
  let current: { frame: number; events: number[] } | null = null;
  for (let index = 0; index < events.length; index++) {
    const ms = events[index].ms;
    if (!Number.isFinite(ms) || ms < 0) continue;
    const frame = Math.floor(ms * sampleRate / 1000 / RENDER_QUANTUM) * RENDER_QUANTUM;
    if (frame >= lengthFrames) break;
    // The planner sorts by time, so a later event never belongs to an earlier stop. A stray
    // out-of-order event joins the current stop rather than asking the render to go back.
    if (!current || frame > current.frame) {
      current = { frame, events: [] };
      steps.push(current);
    }
    current.events.push(index);
  }
  return steps;
}

/** The time to pass to `suspend` for a stop at `frame`: the middle of its first sample. */
export function suspendTime(frame: number, sampleRate: number): number {
  return (frame + 0.5) / sampleRate;
}

/**
 * The offline context as the engine sees it: identical, except that it always reports "running".
 * Methods are bound to the real context (a native method called on a Proxy throws "Illegal
 * invocation"), and getters read the real context for the same reason.
 */
export function runningView<T extends object>(context: T): T {
  return new Proxy(context, {
    get(target, property) {
      if (property === "state") return "running";
      const value: unknown = Reflect.get(target, property, target);
      return typeof value === "function" ? (value as (...args: unknown[]) => unknown).bind(target) : value;
    },
  });
}

/** How long a take sounds, in ms: its last release, or its authored grid end if that is later. */
export function takeLengthMs(plan: JamArchivalPlan, gridMs = 0): number {
  return completionMs(plan, Number.isSafeInteger(gridMs) && gridMs > 0 ? gridMs : 0);
}

export type TakeRenderOptions = {
  /** A Score's authored grid end (`ScorePlayback.gridMs`); trailing rests count as length. */
  readonly gridMs?: number;
  /** Render at most this many ms (a clip never needs more music than the clip is long). */
  readonly maxMs?: number;
  readonly sampleRate?: number;
  /** Aborting stops dispatching; the render finishes silently and the call rejects. */
  readonly signal?: AbortSignal;
  /** Test seam: the context to render into. */
  readonly createContext?: (channels: number, frames: number, sampleRate: number) => OfflineAudioContext;
};

export class TakeRenderError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "TakeRenderError";
  }
}

/**
 * Render `take` from its start. Rejects with `TakeRenderError` when the take is invalid, when its
 * synthesis cannot be prepared, or when an event is refused for a reason that means the music is
 * not being rendered as written (the same refusals that stop Studio playback). A dropped voice
 * under the shared polyphony cap is not a failure, exactly as in Studio.
 */
export async function renderTakeOffline(take: unknown, options: TakeRenderOptions = {}): Promise<AudioBuffer> {
  let plan: JamArchivalPlan;
  try { plan = planJamArchivalPlayback(take); } catch (error) { throw new TakeRenderError("This music could not be read.", { cause: error }); }
  const rate = options.sampleRate ?? CLIP_SAMPLE_RATE;
  const cap = Math.min(TAKE_MAX_DURATION_MS + 10_000, options.maxMs ?? Number.POSITIVE_INFINITY);
  const lengthMs = Math.min(takeLengthMs(plan, options.gridMs), cap);
  const frames = Math.max(RENDER_QUANTUM, Math.ceil(lengthMs * rate / 1000));
  const make = options.createContext ?? ((channels, length, sampleRate) => new OfflineAudioContext({ numberOfChannels: channels, length, sampleRate }));
  const offline = make(2, frames, rate);
  let engine: JamEngine;
  try {
    // Builds the room graph (limiter, chorus, echo) in the offline context straight away.
    engine = new JamEngine(runningView(offline) as unknown as AudioContext, () => offline.currentTime * 1000);
  } catch (error) {
    throw new TakeRenderError("This music could not be rendered.", { cause: error });
  }
  try {
    const capability = await engine.prepareArchivalPlayback(plan.take, undefined);
    const admitted = capability ? engine.archivalPlaybackPlan(capability) : null;
    if (!capability || !admitted) throw new TakeRenderError("This music's instruments could not be prepared.");
    const steps = renderSteps(admitted.events, rate, frames);
    let failure: unknown = null;
    const stopAt = (index: number) => {
      const step = steps[index];
      offline.suspend(suspendTime(step.frame, rate)).then(async () => {
        try {
          if (options.signal?.aborted) failure ??= new DOMException("render aborted", "AbortError");
          for (const event of failure ? [] : step.events) {
            const result = await engine.playArchivalEvent(capability, event, 0);
            if (!result.ok && !DROPPED_EVENT.has(result.reason)) {
              failure = new TakeRenderError("This music could not be played as written.");
              break;
            }
          }
          engine.sweepWatchdogs();
        } catch (error) {
          failure ??= error;
        }
        // The next stop is registered while this one holds the render, which is the only time a
        // stop later than `currentTime` is sure to be accepted. After a failure none are added and
        // the rest renders silently: an offline render cannot be cancelled, only left to finish.
        if (!failure && index + 1 < steps.length) stopAt(index + 1);
        void offline.resume();
      }, (error: unknown) => { failure ??= error; });
    };
    if (steps.length) stopAt(0);
    const rendered = await offline.startRendering();
    if (failure) {
      if (failure instanceof DOMException && failure.name === "AbortError") throw failure;
      if (failure instanceof TakeRenderError) throw failure;
      throw new TakeRenderError("This music could not be rendered.", { cause: failure });
    }
    return rendered;
  } finally {
    engine.dispose();
  }
}
