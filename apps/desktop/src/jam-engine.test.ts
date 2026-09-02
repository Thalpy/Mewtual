import assert from "node:assert/strict";
import test from "node:test";
import { JAM_LEGACY_SESSION_NONCE, JAM_REMOTE_HOLD_MAX_MS, JAM_SESSION_NONCE_HEX_CHARS, type JamPatch } from "./jam-contract.ts";
import { drumSeed, JamEngine } from "./jam-engine.ts";
import { jamPatchId } from "./jam-patch.ts";

class FakeParam {
  value = 0;
  events: [string, number, number][] = [];
  setValueAtTime(value: number, time: number) { this.value = value; this.events.push(["set", value, time]); }
  linearRampToValueAtTime(value: number, time: number) { this.value = value; this.events.push(["linear", value, time]); }
  exponentialRampToValueAtTime(value: number, time: number) { this.value = value; this.events.push(["exponential", value, time]); }
  cancelScheduledValues(time: number) { this.events.push(["cancel", this.value, time]); }
  cancelAndHoldAtTime(time: number) { this.events.push(["hold", this.value, time]); }
}

class FakeNode {
  connections: unknown[] = [];
  disconnects = 0;
  connect(destination: unknown) { this.connections.push(destination); return destination; }
  disconnect() { this.disconnects += 1; this.connections = []; }
}

class FakeSource extends FakeNode {
  onended: (() => void) | null = null;
  starts: number[] = [];
  stops: number[] = [];
  start(time = 0) { this.starts.push(time); }
  stop(time = 0) { this.stops.push(time); }
  end() { this.onended?.(); }
}

class FakeOscillator extends FakeSource {
  type: OscillatorType = "sine";
  frequency = new FakeParam();
  detune = new FakeParam();
}

class FakeGain extends FakeNode { gain = new FakeParam(); }
class FakeFilter extends FakeNode {
  type: BiquadFilterType = "lowpass";
  frequency = new FakeParam();
  Q = new FakeParam();
  detune = new FakeParam();
}
class FakeDelay extends FakeNode { delayTime = new FakeParam(); }
class FakeCompressor extends FakeNode {
  threshold = new FakeParam();
  knee = new FakeParam();
  ratio = new FakeParam();
  attack = new FakeParam();
  release = new FakeParam();
}
class FakeBufferSource extends FakeSource {
  buffer: unknown = null;
  loop = false;
}

class FakeContext {
  currentTime = 0;
  sampleRate = 48_000;
  state: AudioContextState = "running";
  destination = new FakeNode();
  nodes: FakeNode[] = [this.destination];
  sources: FakeSource[] = [];
  delays: FakeDelay[] = [];
  failOscillator = false;
  failGainAt = -1;
  failDelayAt = -1;
  gainCreations = 0;
  delayCreations = 0;

  resume() { this.state = "running"; return Promise.resolve(); }
  createGain() {
    this.gainCreations += 1;
    if (this.gainCreations === this.failGainAt) throw new Error("gain unavailable");
    return this.keep(new FakeGain());
  }
  createOscillator() {
    if (this.failOscillator) throw new Error("oscillator unavailable");
    return this.source(new FakeOscillator());
  }
  createBiquadFilter() { return this.keep(new FakeFilter()); }
  createDelay() {
    this.delayCreations += 1;
    if (this.delayCreations === this.failDelayAt) throw new Error("delay unavailable");
    const delay = this.keep(new FakeDelay()); this.delays.push(delay); return delay;
  }
  createDynamicsCompressor() { return this.keep(new FakeCompressor()); }
  createBufferSource() { return this.source(new FakeBufferSource()); }
  createBuffer(_channels: number, length: number) {
    const data = new Float32Array(length);
    return { getChannelData: () => data };
  }
  private keep<T extends FakeNode>(node: T): T { this.nodes.push(node); return node; }
  private source<T extends FakeSource>(node: T): T { this.nodes.push(node); this.sources.push(node); return node; }
}

const sn = "a".repeat(JAM_SESSION_NONCE_HEX_CHARS);
const otherSn = "b".repeat(JAM_SESSION_NONCE_HEX_CHARS);
const patch: JamPatch = {
  v: 1,
  o: [
    { w: 3, t: 0, c: -12, l: 34 },
    { w: 3, t: 0, c: 0, l: 33 },
    { w: 3, t: 0, c: 12, l: 33 },
  ],
  e: { a: 850, d: 1_200, s: 72, r: 6_000 },
  f: { m: 0, c: 1_800, q: 28, e: 45 },
  l: { r: 18, d: 12, t: 1 },
  x: { c: 26, d: 14, r: 38 },
};

function contextAndEngine(now = { value: 0 }) {
  const fake = new FakeContext();
  const engine = new JamEngine(fake as unknown as AudioContext, () => now.value);
  return { fake, engine, now };
}

test("validated immutable patches render through one receiver-owned room effect graph", async () => {
  const { fake, engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  const roomDelayCount = fake.delays.length;
  const id = await jamPatchId(patch);
  assert.equal(await engine.installPatch(alice, sn, id, patch), "installed");
  const voiceSourcesBefore = fake.sources.length;
  const played = engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "triangle", patchId: id });
  assert.equal(played.ok, true);
  assert.equal(played.usedPatch, id);
  assert.equal(fake.delays.length, roomDelayCount, "a custom patch must not allocate its own effects");
  assert.equal(engine.snapshot().voices.length, 1);

  const voiceSources = fake.sources.slice(voiceSourcesBefore);
  assert.ok(voiceSources.length >= 3);
  assert.ok(voiceSources.every((source) => source.stops.some((time) => time >= 36 && time < 36.1)),
    "the audio-clock watchdog must stop even in a throttled webview");
  assert.equal(engine.noteOff({ channel: alice, sequence: 2, note: 60 }).ok, true);
  assert.equal(engine.snapshot().voices[0]?.phase, "tail");
  for (const source of voiceSources) source.end();
  assert.equal(engine.snapshot().voices.length, 0);
  assert.ok(voiceSources.every((source) => source.disconnects > 0));
});

test("patch hash mismatch fails before anything can render", async () => {
  const { engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  assert.equal(await engine.installPatch(alice, sn, "0".repeat(64), patch), "hash-mismatch");
  const fallback = engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "sine", patchId: "0".repeat(64) });
  assert.equal(fallback.ok, true);
  assert.equal(fallback.usedPatch, undefined);
});

test("patch hashing follows channel receipt order and cannot revive a disposed engine", async () => {
  const { engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  const id = await jamPatchId(patch);
  const older = engine.installPatch(alice, sn, id, patch);
  const newer = engine.installPatch(alice, otherSn, id, patch);
  assert.equal(await older, "installed");
  assert.equal(await newer, "installed");
  assert.equal(engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "sine", patchId: id }).usedPatch, id);

  const pending = engine.installPatch(alice, sn, id, patch);
  engine.dispose();
  assert.equal(await pending, "disposed");
  assert.equal(engine.snapshot().voices.length, 0);
});

test("a rejected announce cannot reset an established sender session", async () => {
  const { engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  const id = await jamPatchId(patch);
  assert.equal(await engine.installPatch(alice, sn, id, patch), "installed");
  assert.equal(engine.noteOn({ channel: alice, sequence: 10, note: 60, wave: "sine", patchId: id }).ok, true);
  assert.equal(await engine.installPatch(alice, otherSn, "0".repeat(64), patch), "hash-mismatch");
  const next = engine.noteOn({ channel: alice, sequence: 11, note: 62, wave: "sine", patchId: id });
  assert.equal(next.sequence, "next");
  assert.equal(next.usedPatch, id);
});

test("reopen clears held state while retaining the sender session for re-announcement", async () => {
  const { engine } = contextAndEngine();
  const id = await jamPatchId(patch);
  const oldChannel = engine.openSource("alice");
  await engine.installPatch(oldChannel, sn, id, patch);
  engine.noteOn({ channel: oldChannel, sequence: 1, note: 60, wave: "sine", patchId: id });
  assert.equal(engine.snapshot().voices.length, 1);
  const newChannel = engine.openSource("alice");
  assert.equal(engine.snapshot().voices.length, 0);
  assert.equal(engine.noteOn({ channel: oldChannel, sequence: 2, note: 61, wave: "sine" }).reason, "channel");
  assert.equal(engine.noteOn({ channel: newChannel, sequence: 9, note: 62, wave: "sine", patchId: id }).usedPatch, id);
});

test("stale and removed channel capabilities cannot mutate or recreate source state", async () => {
  const { engine } = contextAndEngine();
  const id = await jamPatchId(patch);
  const oldChannel = engine.openSource("alice");
  await engine.installPatch(oldChannel, sn, id, patch);
  const current = engine.openSource("alice");
  assert.equal(await engine.installPatch(oldChannel, sn, id, patch), "stale-channel");
  assert.equal(engine.noteOn({ channel: oldChannel, sequence: 99, note: 60, wave: "sine" }).reason, "channel");
  assert.equal(engine.noteOn({ channel: current, sequence: 1, note: 60, wave: "sine" }).sequence, "first");

  engine.removeSource("alice");
  assert.equal(engine.noteOn({ channel: current, sequence: 2, note: 61, wave: "sine" }).reason, "channel");
  assert.equal(await engine.installPatch(current, sn, id, patch), "stale-channel");
  assert.equal(engine.snapshot().voices.length, 0);

  const replacement = engine.openSource("alice");
  engine.beginSourceSession(replacement, otherSn);
  assert.equal(engine.noteOn({ channel: replacement, sequence: 1, note: 62, wave: "sine" }).sequence, "first");
});

test("repeated note-on cannot stack an already-held pitch", () => {
  const { engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  engine.beginSourceSession(alice, sn);
  assert.equal(engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "sine" }).ok, true);
  assert.equal(engine.noteOn({ channel: alice, sequence: 2, note: 60, wave: "sine" }).reason, "duplicate");
  assert.deepEqual(engine.snapshot().voices.map((voice) => voice.phase), ["held"]);
  engine.noteOff({ channel: alice, sequence: 3, note: 60 });
  assert.deepEqual(engine.snapshot().voices.map((voice) => voice.phase), ["tail"]);
});

test("Deafen is a hard remote gate but does not tear down the local player's instrument", () => {
  const { engine } = contextAndEngine();
  const me = engine.openSource("me");
  const alice = engine.openSource("alice");
  engine.beginSourceSession(me, sn);
  engine.beginSourceSession(alice, otherSn);
  engine.noteOn({ channel: me, sequence: 1, note: 60, wave: "triangle", remote: false });
  engine.noteOn({ channel: alice, sequence: 1, note: 64, wave: "triangle" });
  assert.equal(engine.snapshot().voices.length, 2);
  engine.setDeafened(true);
  assert.equal(engine.snapshot().deafened, true);
  assert.deepEqual(engine.snapshot().voices.map((voice) => voice.source), ["me"]);
  assert.deepEqual(engine.noteOn({ channel: alice, sequence: 2, note: 65, wave: "triangle" }), {
    ok: false,
    reason: "muted",
    sequence: "next",
  });
});

test("receiver-owned source level scales every send and survives mute without exceeding unity", () => {
  const { fake, engine } = contextAndEngine();
  const live = engine.openSource("live-performer");
  engine.beginSourceSession(live, otherSn);
  const liveBefore = fake.nodes.length;
  assert.equal(engine.noteOn({ channel: live, sequence: 1, note: 64, wave: "triangle" }).ok, true);
  const liveGates = fake.nodes.slice(liveBefore, liveBefore + 4) as FakeGain[];
  assert.ok(liveGates.every((gate) => gate instanceof FakeGain && gate.gain.value === 1));

  const takeLane = engine.openSource("take-lane");
  engine.beginSourceSession(takeLane, sn);
  assert.equal(engine.setSourceLevel("take-lane", 0.25), true);
  const before = fake.nodes.length;
  assert.equal(engine.noteOn({ channel: takeLane, sequence: 1, note: 60, wave: "triangle", remote: false }).ok, true);

  // A source bus is always dry/chorus/delay/reverb, created before its first voice nodes.
  const gates = fake.nodes.slice(before, before + 4) as FakeGain[];
  assert.equal(gates.length, 4);
  assert.ok(gates.every((gate) => gate instanceof FakeGain && gate.gain.value === 0.25));

  engine.setSourceMuted("take-lane", true);
  assert.ok(gates.every((gate) => gate.gain.value === 0));
  engine.setSourceMuted("take-lane", false);
  assert.ok(gates.every((gate) => gate.gain.value === 0.25), "unmute restores the receiver's level");

  assert.equal(engine.setSourceLevel("take-lane", 9), true);
  assert.ok(gates.every((gate) => gate.gain.value === 1), "gain clamps at unity");
  assert.ok(liveGates.every((gate) => gate.gain.value === 1), "take volume never changes a live source");
  assert.equal(engine.setSourceLevel("take-lane", Number.NaN), false);
  assert.ok(gates.every((gate) => gate.gain.value === 1), "invalid input preserves the safe prior level");

  assert.equal(engine.removeChannel(takeLane), true);
  const reopened = engine.openSource("take-lane");
  engine.beginSourceSession(reopened, sn);
  const reopenedBefore = fake.nodes.length;
  assert.equal(engine.noteOn({ channel: reopened, sequence: 1, note: 67, wave: "triangle", remote: false }).ok, true);
  const reopenedGates = fake.nodes.slice(reopenedBefore, reopenedBefore + 4) as FakeGain[];
  assert.ok(reopenedGates.every((gate) => gate.gain.value === 1), "removing a source forgets its local level");
});

test("capability-scoped cleanup cannot remove a replacement playback source", () => {
  const { engine } = contextAndEngine();
  const stale = engine.openSource("take-lane");
  engine.beginSourceSession(stale, sn);
  const current = engine.openSource("take-lane");
  engine.beginSourceSession(current, otherSn);
  assert.equal(engine.noteOn({ channel: current, sequence: 1, note: 60, wave: "triangle", remote: false }).ok, true);

  assert.equal(engine.removeChannel(stale), false);
  assert.deepEqual(engine.snapshot().voices.map((voice) => voice.source), ["take-lane"]);
  assert.equal(engine.removeChannel(current), true);
  assert.equal(engine.snapshot().voices.length, 0);
});

test("legacy-only is receive-side and immediately removes sounding custom remote voices", async () => {
  const { engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  const id = await jamPatchId(patch);
  await engine.installPatch(alice, sn, id, patch);
  engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "triangle", patchId: id });
  engine.setLegacyOnly(true);
  assert.equal(engine.snapshot().voices.length, 0);
  const fallback = engine.noteOn({ channel: alice, sequence: 2, note: 62, wave: "square", patchId: id });
  assert.equal(fallback.ok, true);
  assert.equal(fallback.usedPatch, undefined);
});

test("the bookkeeping watchdog moves a lost note-off into a bounded release tail", () => {
  const now = { value: 0 };
  const { engine } = contextAndEngine(now);
  const alice = engine.openSource("alice");
  engine.beginSourceSession(alice, sn);
  engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "triangle" });
  now.value = JAM_REMOTE_HOLD_MAX_MS - 1;
  assert.deepEqual(engine.sweepWatchdogs(), []);
  now.value += 1;
  assert.equal(engine.sweepWatchdogs().length, 1);
  assert.equal(engine.snapshot().voices[0]?.phase, "tail");
});

test("a legacy sender receives the same watchdog and can retrigger a releasing pitch", () => {
  const now = { value: 0 };
  const { engine } = contextAndEngine(now);
  const oldClient = engine.openSource("old-client");
  engine.beginLegacySourceSession(oldClient);
  assert.equal(engine.noteOn({ channel: oldClient, sequence: 0, note: 60, wave: "triangle" }).ok, true);
  assert.equal(engine.noteOff({ channel: oldClient, sequence: 1, note: 60 }).ok, true);
  assert.equal(engine.noteOn({ channel: oldClient, sequence: 2, note: 60, wave: "triangle" }).ok, true);
  assert.deepEqual(engine.snapshot().voices.map((voice) => voice.phase), ["tail", "held"]);
  assert.equal(engine.noteOff({ channel: oldClient, sequence: 3, note: 60 }).ok, true);
  assert.deepEqual(engine.snapshot().voices.map((voice) => voice.phase), ["tail", "tail"]);
});

test("a late note-off cannot extend the audio-clock hard-stop deadline", async () => {
  const { fake, engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  const id = await jamPatchId(patch);
  await engine.installPatch(alice, sn, id, patch);
  const before = fake.sources.length;
  engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "triangle", patchId: id });
  const voiceSources = fake.sources.slice(before);
  fake.currentTime = 35;
  engine.noteOff({ channel: alice, sequence: 2, note: 60 });
  assert.ok(voiceSources.every((source) => Math.max(...source.stops) <= 36.01));
});

test("remote notes never queue while audio is suspended", () => {
  const { fake, engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  engine.beginSourceSession(alice, sn);
  fake.state = "suspended";
  const before = fake.sources.length;
  assert.deepEqual(engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "triangle" }), {
    ok: false,
    reason: "audio",
    sequence: "first",
  });
  assert.equal(fake.sources.length, before);
});

test("drum seeds are reproducible and source-scoped hat choke never touches another player", async () => {
  const base = { callId: "general", source: "alice", sessionNonce: sn, sequence: 1, pad: 5 };
  assert.equal(await drumSeed(base), await drumSeed(base));
  assert.notEqual(await drumSeed(base), await drumSeed({ ...base, source: "bob" }));

  const { engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  const bob = engine.openSource("bob");
  assert.equal((await engine.drum({ ...base, channel: alice })).ok, true);
  assert.equal((await engine.drum({ ...base, channel: bob, sessionNonce: otherSn })).ok, true);
  assert.equal((await engine.drum({ ...base, channel: alice, sequence: 2, pad: 4 })).ok, true);
  const voices = engine.snapshot().voices;
  assert.equal(voices.filter((voice) => voice.source === "bob").length, 1);
  assert.equal(voices.filter((voice) => voice.source === "alice").length, 2);
});

test("an async drum from a replaced channel cannot adopt the replacement's reused slot", async () => {
  const { fake, engine } = contextAndEngine();
  const oldChannel = engine.openSource("alice");
  const input = { callId: "general", sessionNonce: sn, sequence: 1, pad: 0 };

  // drum() allocates synchronously and then yields at SHA-256. Reopen the authenticated channel
  // in that gap, deliberately reusing the sender nonce and sequence on the replacement channel.
  const staleDrum = engine.drum({ ...input, channel: oldChannel });
  const currentChannel = engine.openSource("alice");
  const sourceCount = fake.sources.length;
  const currentDrum = engine.drum({ ...input, channel: currentChannel });
  const [staleResult, currentResult] = await Promise.all([staleDrum, currentDrum]);

  assert.equal(staleResult.ok, false);
  assert.equal(staleResult.reason, "channel");
  assert.equal(currentResult.ok, true);
  assert.equal(engine.snapshot().voices.length, 1);
  const currentSources = fake.sources.slice(sourceCount);
  assert.ok(currentSources.length > 0);
  for (const source of currentSources) source.end();
  assert.equal(engine.snapshot().voices.length, 0, "the surviving graph must still own and release its slot");
});

test("a Web Audio construction failure disconnects the partial voice graph", () => {
  const { fake, engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  engine.beginSourceSession(alice, sn);
  fake.failOscillator = true;
  const before = fake.nodes.length;
  assert.deepEqual(engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "triangle" }), {
    ok: false,
    reason: "audio",
    sequence: "first",
    stolen: [],
  });
  const partial = fake.nodes.slice(before);
  // Four source buses intentionally survive for the peer; every later per-voice node must be gone.
  assert.ok(partial.slice(4).every((node) => node.disconnects > 0));
  assert.equal(engine.snapshot().voices.length, 0);
});

test("source-bus construction failure disconnects every partial node", () => {
  const { fake, engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  engine.beginSourceSession(alice, sn);
  const before = fake.nodes.length;
  fake.failGainAt = fake.gainCreations + 3;
  assert.equal(engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "triangle" }).reason, "audio");
  const partial = fake.nodes.slice(before);
  assert.equal(partial.length, 2);
  assert.ok(partial.every((node) => node.disconnects > 0));
});

test("room construction failure stops the already-started chorus source and disconnects the graph", () => {
  const fake = new FakeContext();
  fake.failDelayAt = 2;
  assert.throws(() => new JamEngine(fake as unknown as AudioContext), /delay unavailable/);
  assert.ok(fake.sources.some((source) => source.starts.length > 0 && source.stops.length > 0));
  assert.ok(fake.nodes.slice(1).every((node) => node.disconnects > 0));
});
