import assert from "node:assert/strict";
import test from "node:test";
import { JAM_DRUM_DIGESTS_GLOBAL, JAM_DRUM_DIGESTS_PER_LANE, JAM_DRUM_PENDING_GLOBAL, JAM_DRUM_PENDING_PER_SOURCE, JAM_HELD_PER_PEER, JAM_LEGACY_SESSION_NONCE, JAM_REMOTE_HOLD_MAX_MS, JAM_SESSION_NONCE_HEX_CHARS, type JamPatch } from "./jam-contract.ts";
import { drumSeed, JamEngine, type JamDrumSeedInput, type JamPlaybackPatchSet } from "./jam-engine.ts";
import { jamPatchId } from "./jam-patch.ts";
import { JamLatestTaskQueue } from "./jam-publication.ts";
import { JamPeerBudget } from "./jam-budget.ts";
import { JamFrameDecoder } from "./jam-wire.ts";

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

test("four reconnect generations establish the same verified patch under one persistent budget", async () => {
  const fake = new FakeContext();
  const verifiedId = "c".repeat(64);
  let hashes = 0;
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    undefined,
    async () => { hashes += 1; return verifiedId; },
  );
  const budget = new JamPeerBudget();
  const rawPatch = JSON.stringify({ t: "p", v: 1, id: verifiedId, sn, d: patch });

  for (let generation = 0; generation < 4; generation += 1) {
    const channel = engine.openSource("alice");
    const decoder = new JamFrameDecoder(budget);
    const decoded = decoder.decode(rawPatch, 0);
    assert.equal(decoded.ok, true);
    if (!decoded.ok || decoded.kind !== "jam" || decoded.message.t !== "p") return;
    const installed = decoded.verifiedReannounce
      ? engine.installVerifiedReannounce(channel, decoded.message.sn, decoded.message.id, decoded.message.d)
      : await engine.installPatch(channel, decoded.message.sn, decoded.message.id, decoded.message.d);
    assert.ok(installed === "installed" || installed === "cached");
    assert.equal(decoder.confirmInstalledPatch(decoded.message), true);

    const note = decoder.decode(JSON.stringify({
      t: "n", on: 1, n: 60 + generation, w: "triangle", p: verifiedId, q: 1,
    }), 0);
    assert.equal(note.ok, true);
    if (!note.ok || note.kind !== "jam" || note.message.t !== "n" || note.message.on !== 1) return;
    assert.equal(engine.noteOn({
      channel,
      sequence: note.message.q,
      note: note.message.n,
      wave: note.message.w,
      patchId: note.message.p,
    }).ok, true);
    engine.removeSource("alice"); // the real disconnect path forgets the per-source patch cache
  }
  assert.equal(hashes, 1, "exact reconnect recovery reuses prior verification, not fresh hash work");
});

test("recorder recipe lookup is the renderer's exact touched live LRU", async () => {
  const { engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  const patches = Array.from({ length: 5 }, (_, index): JamPatch => ({
    ...patch,
    o: patch.o.map((osc) => ({ ...osc })),
    e: { ...patch.e },
    f: { ...patch.f, c: 1_000 + index },
    l: { ...patch.l },
    x: { ...patch.x },
  }));
  const ids = await Promise.all(patches.map(jamPatchId));
  for (let index = 0; index < 4; index += 1) {
    assert.equal(await engine.installPatch(alice, sn, ids[index], patches[index]), "installed");
  }
  const first = engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "sine", patchId: ids[0] });
  assert.equal(first.usedPatch, ids[0]);
  assert.deepEqual(engine.patchForRecording(alice, ids[0]), patches[0]);
  engine.noteOff({ channel: alice, sequence: 2, note: 60 });

  assert.equal(await engine.installPatch(alice, sn, ids[4], patches[4]), "installed");
  assert.equal(engine.patchForRecording(alice, ids[1]), undefined, "B is the renderer's true eviction");
  assert.deepEqual(engine.patchForRecording(alice, ids[0]), patches[0], "playing A made it newest");
  const fallback = engine.noteOn({ channel: alice, sequence: 3, note: 61, wave: "sine", patchId: ids[1] });
  assert.equal(fallback.ok, true);
  assert.equal(fallback.usedPatch, undefined);
});

test("patch hash mismatch fails before anything can render", async () => {
  const { engine } = contextAndEngine();
  const alice = engine.openSource("alice");
  assert.equal(await engine.installPatch(alice, sn, "0".repeat(64), patch), "hash-mismatch");
  const fallback = engine.noteOn({ channel: alice, sequence: 1, note: 60, wave: "sine", patchId: "0".repeat(64) });
  assert.equal(fallback.ok, false);
  assert.equal(fallback.sequence, "no-session");
  assert.equal(fallback.usedPatch, undefined);
});

test("archival playback keeps all 64 validated patches without lane-LRU eviction", async () => {
  const { engine } = contextAndEngine();
  const patches = Array.from({ length: 64 }, (_, index): JamPatch => ({
    ...patch,
    o: patch.o.map((osc) => ({ ...osc })),
    e: { ...patch.e },
    f: { ...patch.f, c: 1_000 + index },
    l: { ...patch.l },
    x: { ...patch.x },
  }));
  const expectedIds = await Promise.all(patches.map(jamPatchId));
  const prepared = await engine.preparePlaybackPatches(patches);
  assert.ok(prepared);
  const lane = engine.openSource("take-lane");
  engine.beginSourceSession(lane, sn);
  let sequence = 1;
  for (let index = 0; index < patches.length; index += 1) {
    const result = engine.noteOn(
      { channel: lane, sequence, note: 48 + (index % 12), wave: "triangle", remote: false },
      { patches: prepared, index },
    );
    assert.equal(result.usedPatch, expectedIds[index], `patch ${index} must survive exact replay`);
    sequence += 1;
    engine.noteOff({ channel: lane, sequence, note: 48 + (index % 12) });
    sequence += 1;
  }
});

test("playback preparation is one-running-plus-latest and cancels superseded hash chains", async () => {
  const fake = new FakeContext();
  const id = await jamPatchId(patch);
  let releaseHash!: (value: string) => void;
  const stalled = new Promise<string>((resolve) => { releaseHash = resolve; });
  let hashes = 0;
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    undefined,
    async () => (++hashes === 1 ? stalled : id),
  );
  let generation = 0;
  const queue = new JamLatestTaskQueue<JamPlaybackPatchSet | null>();
  const firstGeneration = generation;
  const first = queue.submit(() => engine.preparePlaybackPatches(
    [patch, patch],
    () => generation === firstGeneration,
  ));
  await Promise.resolve();
  assert.equal(hashes, 1);

  const superseded: Array<Promise<JamPlaybackPatchSet | null>> = [];
  for (let request = 1; request < 100; request += 1) {
    generation += 1;
    const requestGeneration = generation;
    superseded.push(queue.submit(() => engine.preparePlaybackPatches(
      [patch],
      () => generation === requestGeneration,
    )));
  }
  assert.deepEqual(await Promise.all(superseded.slice(0, -1)), new Array(98).fill(null));
  releaseHash(id);
  assert.equal(await first, null);
  assert.ok(await superseded.at(-1));
  assert.equal(hashes, 2, "superseded 64-hash chains never begin");
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

test("a stalled old-channel patch digest cannot block a replacement channel", async () => {
  const fake = new FakeContext();
  const id = await jamPatchId(patch);
  let resolveOld!: (value: string) => void;
  const oldDigest = new Promise<string>((resolve) => { resolveOld = resolve; });
  let hashes = 0;
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    undefined,
    async () => (++hashes === 1 ? oldDigest : id),
  );
  const oldChannel = engine.openSource("alice");
  const oldInstall = engine.installPatch(oldChannel, sn, id, patch);
  await Promise.resolve();
  assert.equal(hashes, 1);

  const replacement = engine.openSource("alice");
  assert.equal(await engine.installPatch(replacement, otherSn, id, patch), "installed");
  assert.equal(hashes, 2, "replacement hashing has its own channel-generation lane");

  resolveOld(id);
  assert.equal(await oldInstall, "stale-channel");
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

test("Deafen is a hard room gate and tears down remote and local-preview voices", () => {
  const { fake, engine } = contextAndEngine();
  const oldRoomNodes = fake.nodes.slice(1);
  const oldRoomSources = [...fake.sources];
  const me = engine.openSource("me");
  const alice = engine.openSource("alice");
  engine.beginSourceSession(me, sn);
  engine.beginSourceSession(alice, otherSn);
  engine.noteOn({ channel: me, sequence: 1, note: 60, wave: "triangle", remote: false });
  engine.noteOn({ channel: alice, sequence: 1, note: 64, wave: "triangle" });
  assert.equal(engine.snapshot().voices.length, 2);
  const sourceBuses = fake.nodes.filter((node) =>
    !oldRoomNodes.includes(node) && node instanceof FakeGain &&
    node.connections.some((destination) => oldRoomNodes.includes(destination as FakeNode))
  );
  assert.equal(sourceBuses.length, 8);
  engine.setDeafened(true);
  assert.equal(engine.snapshot().deafened, true);
  assert.deepEqual(engine.snapshot().voices.map((voice) => voice.source), []);
  assert.ok(oldRoomNodes.every((node) => node.disconnects > 0), "the old wet graph has no live edge");
  assert.ok(oldRoomSources.every((source) => source.stops.length > 0), "room modulation sources stop");
  assert.ok(sourceBuses.every((bus) => bus.connections.length === 0), "source sends leave the old room");
  assert.deepEqual(engine.noteOn({ channel: alice, sequence: 2, note: 65, wave: "triangle" }), {
    ok: false,
    reason: "muted",
    sequence: "next",
  });
  assert.deepEqual(engine.noteOn({ channel: me, sequence: 2, note: 61, wave: "triangle", remote: false }), {
    ok: false,
    reason: "muted",
    sequence: "next",
  });
  const beforeReplacement = fake.nodes.length;
  engine.setDeafened(false);
  const replacementNodes = fake.nodes.slice(beforeReplacement);
  assert.ok(replacementNodes.length > 0);
  assert.ok(sourceBuses.every((bus) =>
    bus.connections.length === 1 &&
    replacementNodes.includes(bus.connections[0] as FakeNode) &&
    !oldRoomNodes.includes(bus.connections[0] as FakeNode)
  ), "existing source gates connect only to a fresh effect graph");
  assert.equal(engine.noteOn({ channel: alice, sequence: 3, note: 65, wave: "triangle" }).ok, true);
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

test("App gate suppression advances sequencing without hashing or constructing audio", async () => {
  const fake = new FakeContext();
  let seedCalls = 0;
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    async () => { seedCalls += 1; return 1; },
  );
  const alice = engine.openSource("alice");
  engine.beginSourceSession(alice, sn);
  const before = fake.sources.length;
  assert.deepEqual(engine.noteOn({
    channel: alice, sequence: 1, note: 60, wave: "triangle", remote: true, render: false,
  }), { ok: false, reason: "muted", sequence: "first" });
  assert.deepEqual(await engine.drum({
    callId: "general", channel: alice, sessionNonce: sn, sequence: 2, pad: 0, remote: true, render: false,
  }), { ok: false, reason: "muted", sequence: "next" });
  assert.equal(seedCalls, 0);
  assert.equal(fake.sources.length, before);
  assert.equal(engine.noteOn({
    channel: alice, sequence: 3, note: 61, wave: "triangle", remote: true,
  }).ok, true, "the next post-gate event continues the accepted q lane");
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

test("live and take playback use identical stable drum seed provenance", async () => {
  const fake = new FakeContext();
  const seeds: JamDrumSeedInput[] = [];
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    async (input) => { seeds.push(input); return 123; },
  );
  // The App deliberately keeps the local sequencing alias as "me"; the engine must bind seed
  // provenance to the authenticated fingerprint rather than leaking that alias into recordings.
  const live = engine.openLocalSource("alice");
  assert.equal((await engine.drum({
    callId: "stable-channel", channel: live, sessionNonce: sn, sequence: 7, pad: 4,
  })).ok, true);

  const patches = await engine.preparePlaybackPatches([], () => true, {
    callId: "stable-channel",
    sources: ["alice"],
  });
  assert.ok(patches);
  const playback = engine.openPlaybackSource("take\u00000", patches!, 0);
  assert.ok(playback);
  assert.equal((await engine.drum(
    { callId: "ignored-live-field", channel: playback!, sessionNonce: sn, sequence: 7, pad: 4 },
    { patches: patches!, part: 0 },
  )).ok, true);
  assert.deepEqual(seeds, [seeds[0], seeds[0]], "synthetic allocator ids never enter seed material");
});

test("take reconnect lanes share performer fairness and source-scoped hat choke", async () => {
  const { fake, engine } = contextAndEngine();
  const patches = await engine.preparePlaybackPatches([], () => true, {
    callId: "stable-channel",
    sources: ["alice", "bob"],
  });
  assert.ok(patches);

  for (let index = 0; index < JAM_HELD_PER_PEER; index += 1) {
    const lane = engine.openPlaybackSource(`alice-${index}`, patches!, 0);
    assert.ok(lane);
    engine.beginSourceSession(lane!, sn);
    assert.equal(engine.noteOn({
      channel: lane!, sequence: 1, note: 40 + index, wave: "triangle", remote: false,
    }).ok, true);
  }
  const overflowLane = engine.openPlaybackSource("alice-overflow", patches!, 0);
  assert.ok(overflowLane);
  engine.beginSourceSession(overflowLane!, sn);
  assert.deepEqual(engine.noteOn({
    channel: overflowLane!, sequence: 1, note: 90, wave: "triangle", remote: false,
  }), { ok: false, reason: "source-held", sequence: "first" });

  // Take identities are recorder-attested until phase 6. Claiming a live fingerprint must not
  // turn playback into a deputy that consumes that live performer's fairness/choke domain.
  const liveAlice = engine.openSource("alice");
  engine.beginSourceSession(liveAlice, otherSn);
  assert.equal(engine.noteOn({
    channel: liveAlice, sequence: 1, note: 91, wave: "triangle", remote: false,
  }).ok, true, "a forged take participant label cannot consume the live source's allowance");

  const bobLane = engine.openPlaybackSource("bob-one", patches!, 1);
  assert.ok(bobLane);
  engine.beginSourceSession(bobLane!, otherSn);
  assert.equal(engine.noteOn({
    channel: bobLane!, sequence: 1, note: 90, wave: "triangle", remote: false,
  }).ok, true, "another participant keeps an independent held-note allowance");

  const aliceOpenHat = engine.openPlaybackSource("alice-hat-open", patches!, 0);
  const aliceClosedHat = engine.openPlaybackSource("alice-hat-closed", patches!, 0);
  const bobOpenHat = engine.openPlaybackSource("bob-hat-open", patches!, 1);
  assert.ok(aliceOpenHat && aliceClosedHat && bobOpenHat);
  const beforeHats = fake.sources.length;
  assert.equal((await engine.drum(
    { callId: "ignored", channel: aliceOpenHat!, sessionNonce: sn, sequence: 1, pad: 5, remote: false },
    { patches: patches!, part: 0 },
  )).ok, true);
  const aliceHatSources = fake.sources.slice(beforeHats);
  assert.equal((await engine.drum(
    { callId: "ignored", channel: bobOpenHat!, sessionNonce: otherSn, sequence: 1, pad: 5, remote: false },
    { patches: patches!, part: 1 },
  )).ok, true);
  const bobHatSources = fake.sources.slice(beforeHats + aliceHatSources.length);
  assert.equal((await engine.drum(
    { callId: "ignored", channel: aliceClosedHat!, sessionNonce: otherSn, sequence: 1, pad: 4, remote: false },
    { patches: patches!, part: 0 },
  )).ok, true);
  assert.ok(aliceHatSources.every((source) => source.stops.length > 1), "Alice's reconnect lane chokes Alice's open hat");
  assert.ok(bobHatSources.every((source) => source.stops.length === 1), "Alice never chokes Bob's open hat");
});

test("take completion releases an unmatched held note before bounded channel teardown", async () => {
  const { fake, engine } = contextAndEngine();
  const patches = await engine.preparePlaybackPatches([patch], () => true, {
    callId: "stable-channel",
    sources: ["alice"],
  });
  assert.ok(patches);
  const channel = engine.openPlaybackSource("take-unmatched", patches!, 0);
  assert.ok(channel);
  engine.beginSourceSession(channel!, sn);
  const before = fake.sources.length;
  assert.equal(engine.noteOn(
    { channel: channel!, sequence: 1, note: 60, wave: "triangle", remote: false },
    { patches: patches!, index: 0 },
  ).ok, true);
  assert.equal(engine.snapshot().voices.at(-1)?.phase, "held");

  assert.equal(engine.releaseChannelHeld(channel!).length, 1);
  assert.equal(engine.snapshot().voices.at(-1)?.phase, "tail");
  assert.equal(engine.releaseChannelHeld(channel!).length, 0, "completion release is idempotent");
  const voiceSources = fake.sources.slice(before);
  assert.ok(voiceSources.every((source) => Math.min(...source.stops) <= 6.01),
    "the release envelope ends before the take's six-second retention horizon");

  const stale = channel!;
  const replacement = engine.openPlaybackSource("take-unmatched", patches!, 0);
  assert.ok(replacement);
  engine.beginSourceSession(replacement!, sn);
  assert.equal(engine.noteOn(
    { channel: replacement!, sequence: 1, note: 62, wave: "triangle", remote: false },
    { patches: patches!, index: 0 },
  ).ok, true);
  assert.deepEqual(engine.releaseChannelHeld(stale), [], "stale completion cannot release a replacement lane");
  assert.equal(engine.snapshot().voices.at(-1)?.phase, "held");
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

test("a stalled old-channel drum cannot head-of-line block its replacement", async () => {
  const fake = new FakeContext();
  const resolvers: ((seed: number) => void)[] = [];
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    () => new Promise<number>((resolve) => { resolvers.push(resolve); }),
  );
  const oldChannel = engine.openSource("alice");
  const old = engine.drum({ callId: "general", channel: oldChannel, sessionNonce: sn, sequence: 1, pad: 5 });
  await Promise.resolve();
  assert.equal(resolvers.length, 1, "the old generation owns one genuinely active digest");

  const replacement = engine.openSource("alice");
  const current = engine.drum({ callId: "general", channel: replacement, sessionNonce: sn, sequence: 1, pad: 4 });
  await Promise.resolve();
  assert.equal(resolvers.length, 2, "opaque generation ordering lets replacement work start");
  resolvers[1](2);
  assert.equal((await current).ok, true);
  resolvers[0](1);
  assert.deepEqual(await old, { ok: false, reason: "channel", sequence: "first" });
  assert.equal(engine.snapshot().voices.length, 1);
});

test("a pending remote drum rechecks Deafen after its digest await", async () => {
  const fake = new FakeContext();
  let resolveSeed!: (seed: number) => void;
  const seed = new Promise<number>((resolve) => { resolveSeed = resolve; });
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    () => seed,
  );
  const alice = engine.openSource("alice");
  const pending = engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: 1, pad: 0 });
  engine.setDeafened(true);
  resolveSeed(123);
  assert.deepEqual(await pending, { ok: false, reason: "muted", sequence: "first" });
  assert.equal(engine.snapshot().voices.length, 0);
});

test("Deafen and source mute generations prevent pre-gate drums emerging after reopen", async () => {
  const fake = new FakeContext();
  const resolvers: ((seed: number) => void)[] = [];
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    () => new Promise<number>((resolve) => { resolvers.push(resolve); }),
  );
  const alice = engine.openSource("alice");

  const beforeDeafen = engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: 1, pad: 0 });
  await Promise.resolve();
  const queuedBeforeDeafen = engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: 2, pad: 1 });
  engine.setDeafened(true);
  assert.deepEqual(await queuedBeforeDeafen, { ok: false, reason: "muted", sequence: "next" });
  engine.setDeafened(false);
  const afterDeafen = engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: 3, pad: 2 });
  await Promise.resolve();
  assert.equal(resolvers.length, 2, "fresh room-generation work does not wait behind obsolete work");
  resolvers[1](2);
  assert.equal((await afterDeafen).ok, true);
  resolvers[0](1);
  assert.deepEqual(await beforeDeafen, { ok: false, reason: "muted", sequence: "first" });

  const beforeMute = engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: 4, pad: 3 });
  await Promise.resolve();
  const queuedBeforeMute = engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: 5, pad: 4 });
  engine.setSourceMuted("alice", true);
  assert.deepEqual(await queuedBeforeMute, { ok: false, reason: "muted", sequence: "next" });
  engine.setSourceMuted("alice", false);
  const afterMute = engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: 6, pad: 5 });
  await Promise.resolve();
  assert.equal(resolvers.length, 4, "fresh source-generation work does not wait behind obsolete work");
  resolvers[3](4);
  assert.equal((await afterMute).ok, true);
  resolvers[2](3);
  assert.deepEqual(await beforeMute, { ok: false, reason: "muted", sequence: "next" });
});

test("a pending remote drum rechecks audio suspension after its digest await", async () => {
  const fake = new FakeContext();
  let resolveSeed!: (seed: number) => void;
  const seed = new Promise<number>((resolve) => { resolveSeed = resolve; });
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    () => seed,
  );
  const alice = engine.openSource("alice");
  const pending = engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: 1, pad: 0 });
  fake.state = "suspended";
  resolveSeed(123);
  assert.deepEqual(await pending, { ok: false, reason: "audio", sequence: "first" });
  assert.equal(engine.snapshot().voices.length, 0);
});

test("same-owner drum hashes are backpressured and commit in event order", async () => {
  const fake = new FakeContext();
  const seeds: { input: JamDrumSeedInput; resolve: (seed: number) => void }[] = [];
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    (input) => new Promise<number>((resolve) => { seeds.push({ input, resolve }); }),
  );
  const alice = engine.openSource("alice");
  const pads = [5, 4, 0, 1, 2];
  const pending = pads.map((pad, index) => engine.drum({
    callId: "general", channel: alice, sessionNonce: sn, sequence: index + 1, pad,
  }));
  await Promise.resolve();
  assert.equal(JAM_DRUM_DIGESTS_PER_LANE, 1);
  assert.equal(seeds.length, 1, "later owner work waits instead of being rejected or racing");

  const beforeOpenHat = fake.sources.length;
  seeds[0].resolve(101);
  assert.equal((await pending[0]).ok, true);
  const openHatSources = fake.sources.slice(beforeOpenHat);
  await Promise.resolve();
  assert.equal(seeds.length, 2);
  seeds[1].resolve(102);
  assert.equal((await pending[1]).ok, true);
  assert.ok(openHatSources.every((source) => source.stops.length > 1),
    "the later closed hat commits after and chokes the earlier open hat");

  for (let index = 2; index < pending.length; index += 1) {
    await Promise.resolve();
    assert.equal(seeds.length, index + 1);
    seeds[index].resolve(100 + index);
    assert.equal((await pending[index]).ok, true);
  }
  assert.deepEqual(seeds.map(({ input }) => input.sequence), [1, 2, 3, 4, 5]);
});

test("the 33rd owner waits behind the global digest ceiling instead of losing its hit", async () => {
  const fake = new FakeContext();
  const resolvers: ((seed: number) => void)[] = [];
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    () => new Promise<number>((resolve) => { resolvers.push(resolve); }),
  );
  const pending = Array.from({ length: JAM_DRUM_DIGESTS_GLOBAL + 1 }, (_, index) => {
    const channel = engine.openSource(`owner-${index}`);
    return engine.drum({ callId: "general", channel, sessionNonce: sn, sequence: 1, pad: 0 });
  });
  await Promise.resolve();
  assert.equal(resolvers.length, JAM_DRUM_DIGESTS_GLOBAL);
  assert.equal(engine.snapshot().voices.length, 0, "queued hashes do not reserve audible voices");

  resolvers[0](1);
  assert.equal((await pending[0]).ok, true);
  await Promise.resolve();
  assert.equal(resolvers.length, JAM_DRUM_DIGESTS_GLOBAL + 1, "the queued owner eventually receives a slot");
  for (const resolve of resolvers.slice(1)) resolve(2);
  const settled = await Promise.all(pending);
  assert.ok(settled.every((result) => result.ok));
});

test("drum pending bounds reject excess work and dispose releases queued closures", async () => {
  const fake = new FakeContext();
  const resolvers: ((seed: number) => void)[] = [];
  const engine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    () => new Promise<number>((resolve) => { resolvers.push(resolve); }),
  );
  const alice = engine.openSource("alice");
  const pending = Array.from({ length: JAM_DRUM_PENDING_PER_SOURCE }, (_, index) =>
    engine.drum({ callId: "general", channel: alice, sessionNonce: sn, sequence: index + 1, pad: 0 }));
  assert.deepEqual(await engine.drum({
    callId: "general", channel: alice, sessionNonce: sn,
    sequence: JAM_DRUM_PENDING_PER_SOURCE + 1, pad: 0,
  }), { ok: false, reason: "source-held", sequence: "next" });
  await Promise.resolve();
  assert.equal(resolvers.length, 1);
  engine.dispose();
  const queued = await Promise.all(pending.slice(1));
  assert.ok(queued.every((result) => !result.ok), "dispose resolves every not-yet-started job inert");
  resolvers[0](1);
  assert.equal((await pending[0]).ok, false);

  const roomResolvers: ((seed: number) => void)[] = [];
  const roomEngine = new JamEngine(
    fake as unknown as AudioContext,
    () => 0,
    fake.destination as unknown as AudioNode,
    undefined,
    () => new Promise<number>((resolve) => { roomResolvers.push(resolve); }),
  );
  const roomPending = Array.from({ length: JAM_DRUM_PENDING_GLOBAL }, (_, index) => {
    const channel = roomEngine.openSource(`bounded-owner-${index}`);
    return roomEngine.drum({ callId: "general", channel, sessionNonce: sn, sequence: 1, pad: 0 });
  });
  const overflowChannel = roomEngine.openSource("bounded-overflow");
  assert.deepEqual(await roomEngine.drum({
    callId: "general", channel: overflowChannel, sessionNonce: sn, sequence: 1, pad: 0,
  }), { ok: false, reason: "room-held", sequence: "first" });
  await Promise.resolve();
  assert.equal(roomResolvers.length, JAM_DRUM_DIGESTS_GLOBAL);
  roomEngine.dispose();
  for (const resolve of roomResolvers) resolve(2);
  assert.ok((await Promise.all(roomPending)).every((result) => !result.ok));
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
