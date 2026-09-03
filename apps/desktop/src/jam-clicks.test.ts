import assert from "node:assert/strict";
import test from "node:test";
import { JamCallCuePlayer, JamClickPlayer } from "./jam-clicks.ts";
import { JAM_CALL_CUE_PENDING_MAX } from "./jam-contract.ts";

class FakeParam {
  value = 0;
  setValueAtTime(value: number) { this.value = value; }
  exponentialRampToValueAtTime(value: number) { this.value = value; }
}
class FakeNode {
  disconnected = false;
  connections: unknown[] = [];
  connect<T>(destination: T): T { this.connections.push(destination); return destination; }
  disconnect() { this.disconnected = true; }
}
class FakeOscillator extends FakeNode {
  type: OscillatorType = "sine";
  frequency = new FakeParam();
  onended: (() => void) | null = null;
  starts: number[] = [];
  stops: number[] = [];
  start(at: number) { this.starts.push(at); }
  stop(at: number) { this.stops.push(at); }
}
class FakeGain extends FakeNode { gain = new FakeParam(); }
class FakeCompressor extends FakeNode {
  threshold = new FakeParam();
  knee = new FakeParam();
  ratio = new FakeParam();
  attack = new FakeParam();
  release = new FakeParam();
}
class FakeContext {
  state: AudioContextState = "running";
  currentTime = 4;
  destination = new FakeNode();
  oscillators: FakeOscillator[] = [];
  gains: FakeGain[] = [];
  compressors: FakeCompressor[] = [];
  resumeCalls = 0;
  resume() { this.resumeCalls += 1; this.state = "running"; return Promise.resolve(); }
  createOscillator() { const node = new FakeOscillator(); this.oscillators.push(node); return node; }
  createGain() { const node = new FakeGain(); this.gains.push(node); return node; }
  createDynamicsCompressor() { const node = new FakeCompressor(); this.compressors.push(node); return node; }
}

test("a suspended context creates no metronome nodes", () => {
  const context = new FakeContext();
  context.state = "suspended";
  const player = new JamClickPlayer();
  assert.equal(player.play(context as unknown as AudioContext, { audioTime: 4, beat: 0, accent: true }), false);
  assert.equal(context.oscillators.length, 0);
  assert.equal(context.gains.length, 0);
  assert.equal(player.pending(), 0);
});

test("stop or Deafen cancellation tears down every scheduled lookahead click", () => {
  const context = new FakeContext();
  const player = new JamClickPlayer();
  player.play(context as unknown as AudioContext, { audioTime: 4.1, beat: 0, accent: true });
  player.play(context as unknown as AudioContext, { audioTime: 4.2, beat: 1, accent: false });
  assert.equal(player.pending(), 2);
  player.cancel(context as unknown as AudioContext);
  assert.equal(player.pending(), 0);
  assert.ok(context.oscillators.every((node) => node.stops.includes(context.currentTime) && node.disconnected));
  assert.ok(context.gains.every((node) => node.disconnected));
});

test("remote-call cues never resume or create nodes in a suspended context", () => {
  const context = new FakeContext();
  context.state = "suspended";
  const player = new JamCallCuePlayer();
  assert.equal(player.play(context as unknown as AudioContext, 79), false);
  assert.equal(context.resumeCalls, 0);
  assert.equal(context.oscillators.length, 0);
  assert.equal(context.gains.length, 0);
});

test("Deafen cancellation tears down an already-playing call cue", () => {
  const context = new FakeContext();
  const player = new JamCallCuePlayer();
  assert.equal(player.play(context as unknown as AudioContext, 60), true);
  assert.equal(player.pending(), 1);
  player.cancel(context as unknown as AudioContext);
  assert.equal(player.pending(), 0);
  assert.ok(context.oscillators[0].stops.includes(context.currentTime));
  assert.ok(context.oscillators[0].disconnected);
  assert.ok(context.gains.every((gain) => gain.disconnected));
  assert.ok(context.compressors[0].disconnected);
});

test("call cues have a global overlap cap and one receiver-owned limited output", () => {
  const context = new FakeContext();
  const player = new JamCallCuePlayer();
  for (let index = 0; index < JAM_CALL_CUE_PENDING_MAX; index += 1) {
    assert.equal(player.play(context as unknown as AudioContext, 60 + index), true);
  }
  assert.equal(player.play(context as unknown as AudioContext, 80), false);
  assert.equal(player.pending(), JAM_CALL_CUE_PENDING_MAX);
  assert.equal(context.oscillators.length, JAM_CALL_CUE_PENDING_MAX);
  assert.equal(context.compressors.length, 1);
  const [input, ...cueGains] = context.gains;
  assert.ok(cueGains.every((gain) => gain.connections[0] === input));
  assert.ok(cueGains.every((gain) => !gain.connections.includes(context.destination)));
  assert.equal(input.connections[0], context.compressors[0]);
  assert.equal(context.compressors[0].connections[0], context.destination);
});
