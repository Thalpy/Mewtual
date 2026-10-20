import assert from "node:assert/strict";
import test from "node:test";
import { planJamArchivalPlayback } from "./jam-archival-plan.ts";
import type { JamTake } from "./jam-contract.ts";
import {
  RENDER_QUANTUM, renderSteps, renderTakeOffline, runningView, suspendTime, TakeRenderError, takeLengthMs,
} from "./jam-offline.ts";

// --- A fake offline context that keeps Chromium's suspend rules ---------------------------------
// The real JamEngine runs against it, so these tests exercise the actual dispatch path. The suspend
// rules are the platform's, enforced: one stop per render quantum, never at or before the quantum
// being rendered, never at or past the end. A fake that accepted those would hide exactly the
// rounding bugs `suspendTime` exists to avoid.

class Param {
  value = 0;
  setValueAtTime(value: number) { this.value = value; }
  linearRampToValueAtTime(value: number) { this.value = value; }
  exponentialRampToValueAtTime(value: number) { this.value = value; }
  setTargetAtTime() {}
  cancelScheduledValues() {}
  cancelAndHoldAtTime() {}
}
class Node {
  connect(to: unknown) { return to; }
  disconnect() {}
}
class Source extends Node {
  onended: (() => void) | null = null;
  starts: number[] = [];
  private readonly log: number[];
  constructor(log: number[]) { super(); this.log = log; }
  start(time = 0) { this.starts.push(time); this.log.push(time); }
  stop() {}
}
class Osc extends Source { type = "sine"; frequency = new Param(); detune = new Param(); }
class BufferSource extends Source { buffer: unknown = null; loop = false; playbackRate = new Param(); }

class FakeOffline {
  readonly sampleRate: number;
  readonly length: number;
  readonly destination = new Node();
  state: "suspended" | "running" | "closed" = "suspended";
  currentFrame = 0;
  started = false;
  rendered = false;
  /** Every source start, in render seconds. */
  readonly starts: number[] = [];
  /** Render seconds at which each stop was reached. */
  readonly stops: number[] = [];
  /** "always": every oscillator fails; "render": only those made once rendering has started. */
  failOscillator: false | "always" | "render" = false;
  private readonly pending = new Map<number, () => void>();
  private onResume: (() => void) | null = null;

  constructor(length: number, sampleRate: number) { this.length = length; this.sampleRate = sampleRate; }
  get currentTime() { return this.currentFrame / this.sampleRate; }

  createGain() { return Object.assign(new Node(), { gain: new Param() }); }
  createOscillator() {
    if (this.failOscillator === "always" || (this.failOscillator === "render" && this.started)) throw new Error("oscillator unavailable");
    return new Osc(this.starts);
  }
  createBiquadFilter() { return Object.assign(new Node(), { type: "lowpass", frequency: new Param(), Q: new Param(), detune: new Param(), gain: new Param() }); }
  createDelay() { return Object.assign(new Node(), { delayTime: new Param() }); }
  createDynamicsCompressor() {
    return Object.assign(new Node(), { threshold: new Param(), knee: new Param(), ratio: new Param(), attack: new Param(), release: new Param() });
  }
  createWaveShaper() { return Object.assign(new Node(), { curve: null, oversample: "none" }); }
  createStereoPanner() { return Object.assign(new Node(), { pan: new Param() }); }
  createConstantSource() { return Object.assign(new Source(this.starts), { offset: new Param() }); }
  createBufferSource() { return new BufferSource(this.starts); }
  createBuffer(_channels: number, length: number) {
    const data = new Float32Array(length);
    return { length, getChannelData: () => data };
  }

  suspend(time: number): Promise<void> {
    const frame = Math.floor(time * this.sampleRate);
    const quantum = frame - frame % RENDER_QUANTUM;
    if (quantum >= this.length) return Promise.reject(new Error("suspend past the end"));
    if (this.started && quantum <= this.currentFrame) return Promise.reject(new Error("suspend in the past"));
    if (this.pending.has(quantum)) return Promise.reject(new Error("two suspends in one quantum"));
    return new Promise((resolve) => { this.pending.set(quantum, resolve); });
  }
  resume(): Promise<void> {
    const go = this.onResume;
    this.onResume = null;
    go?.();
    return Promise.resolve();
  }
  async startRendering() {
    this.started = true;
    this.state = "running";
    for (;;) {
      const next = Math.min(...this.pending.keys());
      if (!Number.isFinite(next)) break;
      const release = this.pending.get(next)!;
      this.pending.delete(next);
      this.currentFrame = next;
      this.stops.push(this.currentTime);
      this.state = "suspended";
      const resumed = new Promise<void>((resolve) => { this.onResume = resolve; });
      release();
      await resumed;
      this.state = "running";
    }
    this.currentFrame = this.length;
    this.rendered = true;
    return { length: this.length, sampleRate: this.sampleRate, numberOfChannels: 2 } as unknown as AudioBuffer;
  }
}

const RATE = 48_000;
function noteTake(times: readonly number[], holdMs = 200): JamTake {
  const timed: { ms: number; n: number; on: 0 | 1 }[] = [];
  times.forEach((ms, index) => {
    timed.push({ ms, n: 60 + index, on: 1 }, { ms: ms + holdMs, n: 60 + index, on: 0 });
  });
  // A take is a log: ascending time, one sequence number per event in that order.
  timed.sort((a, b) => a.ms - b.ms);
  const events: JamTake["events"] = timed.map((e, q) => e.on
    ? { ms: e.ms, lane: 0, n: e.n, on: 1, w: "sine", q }
    : { ms: e.ms, lane: 0, n: e.n, on: 0, q });
  return { v: 1, group: "group", call: "call", met: { bpm: 120, bpb: 4 }, parts: ["author"], lanes: [{ src: 0, sn: "a".repeat(16) }], patches: [], events };
}
function fakeFactory() {
  const made: FakeOffline[] = [];
  return {
    made,
    createContext: (_channels: number, frames: number, sampleRate: number) => {
      const context = new FakeOffline(frames, sampleRate);
      made.push(context);
      return context as unknown as OfflineAudioContext;
    },
  };
}

test("events are grouped per render quantum, rounded down, and none past the end", () => {
  const quantumMs = RENDER_QUANTUM / RATE * 1000;
  const steps = renderSteps(
    [{ ms: 0 }, { ms: 1 }, { ms: quantumMs }, { ms: quantumMs * 2.9 }, { ms: 500 }, { ms: 999 }, { ms: 1000 }],
    RATE,
    RATE,
  );
  assert.deepEqual(steps.map((s) => s.frame), [0, RENDER_QUANTUM, RENDER_QUANTUM * 2, 23_936, 47_872]);
  assert.deepEqual(steps.map((s) => s.events), [[0, 1], [2], [3], [4], [5]], "the event at exactly the end is left out");
  for (const step of steps) assert.equal(step.frame % RENDER_QUANTUM, 0);
});

test("a stop's suspend time floors back to its own frame at the common rates", () => {
  for (const rate of [44_100, 48_000, 96_000]) {
    for (let frame = 0; frame < 2_000_000; frame += RENDER_QUANTUM * 37) {
      assert.equal(Math.floor(suspendTime(frame, rate) * rate), frame, `${frame} at ${rate}`);
    }
  }
});

test("the running view reports running and still calls the real object's methods and getters", () => {
  class Branded {
    #frames = 7;
    state = "suspended";
    get frames() { return this.#frames; }
    read() { return this.#frames; }
  }
  const real = new Branded();
  const view = runningView(real);
  assert.equal(view.state, "running");
  assert.equal(real.state, "suspended", "the real object is not changed");
  assert.equal(view.read(), 7, "a method that checks its receiver's brand still works");
  assert.equal(view.frames, 7, "a getter that checks its receiver's brand still works");
});

test("each note starts when the render clock reaches it, never late, and the render is as long as the take", async () => {
  const take = noteTake([0, 500, 1000, 1001]);
  const { made, createContext } = fakeFactory();
  await renderTakeOffline(take, { createContext });
  const context = made[0];
  assert.equal(context.rendered, true);
  const expectedMs = takeLengthMs(planJamArchivalPlayback(take));
  assert.equal(context.length, Math.ceil(expectedMs * RATE / 1000));
  // Each attack starts oscillators at the stop's time: within one quantum before the event.
  const quantum = RENDER_QUANTUM / RATE;
  const attacks = [...new Set(context.starts.map((s) => s.toFixed(6)))].map(Number);
  for (const ms of [0, 500, 1000]) {
    const at = ms / 1000;
    assert.ok(attacks.some((s) => s <= at + 1e-9 && s > at - quantum), `a note starts at ${ms} ms (starts: ${attacks})`);
  }
  assert.ok(context.starts.every((s) => s <= expectedMs / 1000), "nothing starts after the end");
  // 1000 and 1001 ms share a quantum: one stop, not two (two would have been refused).
  const stopsNear1s = context.stops.filter((s) => Math.abs(s - 1) < 0.01);
  assert.equal(stopsNear1s.length, 1);
});

test("a length limit cuts the render and plays nothing beyond it", async () => {
  const take = noteTake([0, 400, 2000]);
  const { made, createContext } = fakeFactory();
  await renderTakeOffline(take, { createContext, maxMs: 1000 });
  const context = made[0];
  assert.equal(context.length, RATE);
  assert.ok(context.starts.every((s) => s < 1), "the note at 2 s is never started");
  assert.ok(context.stops.every((s) => s < 1));
});

test("a Score's trailing rests count as length", async () => {
  const take = noteTake([0]);
  const { made, createContext } = fakeFactory();
  await renderTakeOffline(take, { createContext, gridMs: 4000 });
  assert.equal(made[0].length, 4 * RATE);
});

test("a take that fails validation is refused before any render starts", async () => {
  const { made, createContext } = fakeFactory();
  const bad = { ...noteTake([0]), v: 2 };
  await assert.rejects(renderTakeOffline(bad, { createContext }), TakeRenderError);
  assert.equal(made.length, 0);
});

test("aborting stops dispatch, lets the render finish silently, and rejects", async () => {
  const take = noteTake([0, 300, 600, 900]);
  const { made, createContext } = fakeFactory();
  const abort = new AbortController();
  const original = FakeOffline.prototype.resume;
  let resumes = 0;
  FakeOffline.prototype.resume = function (this: FakeOffline) {
    if (++resumes === 2) abort.abort();
    return original.call(this);
  };
  try {
    await assert.rejects(renderTakeOffline(take, { createContext, signal: abort.signal }), { name: "AbortError" });
  } finally {
    FakeOffline.prototype.resume = original;
  }
  const context = made[0];
  assert.equal(context.rendered, true, "the render ran to its end rather than hanging suspended");
  assert.ok(context.starts.every((s) => s < 0.6), "no note after the abort is started");
});

function failingFactory(mode: "always" | "render") {
  const { made, createContext } = fakeFactory();
  return {
    made,
    createContext: (channels: number, frames: number, rate: number) => {
      const context = createContext(channels, frames, rate) as unknown as FakeOffline;
      context.failOscillator = mode;
      return context as unknown as OfflineAudioContext;
    },
  };
}

test("a synthesis failure while playing rejects instead of returning quieter music", async () => {
  const { made, createContext } = failingFactory("render");
  await assert.rejects(renderTakeOffline(noteTake([0, 500]), { createContext }), TakeRenderError);
  assert.equal(made[0].rendered, true, "the render was let finish, not left suspended");
});

test("a context that cannot build the engine's mixer is a render error, not a raw one", async () => {
  const { createContext } = failingFactory("always");
  await assert.rejects(renderTakeOffline(noteTake([0]), { createContext }), TakeRenderError);
});
