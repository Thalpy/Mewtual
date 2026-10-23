import assert from "node:assert/strict";
import test from "node:test";
import { JAM_DOCUMENT_HELD_MAX, JAM_HELD_PER_PEER, JAM_REMOTE_HOLD_MAX_MS, JAM_VOICES_GLOBAL } from "./jam-contract.ts";
import { JamVoiceAllocator, type JamVoiceEndReason } from "./jam-allocator.ts";

function add(
  allocator: JamVoiceAllocator,
  id: string,
  source: string,
  phase: "held" | "tail",
  at: number,
  ended: [string, JamVoiceEndReason][],
  owner?: string,
) {
  return allocator.allocate({
    id,
    source,
    owner,
    phase,
    startedAtMs: at,
    teardown: (reason) => ended.push([id, reason]),
  });
}

test("archival reconnect lanes share one performer's held-note limit", () => {
  const allocator = new JamVoiceAllocator();
  const ended: [string, JamVoiceEndReason][] = [];
  for (let index = 0; index < JAM_HELD_PER_PEER; index += 1) {
    assert.equal(add(allocator, `alice:${index}`, `take-lane-${index}`, "held", index, ended, "alice").ok, true);
  }
  assert.deepEqual(
    add(allocator, "alice:overflow", "take-lane-overflow", "held", 99, ended, "alice"),
    { ok: false, reason: "source-held" },
  );
  assert.equal(add(allocator, "bob:one", "take-lane-bob", "held", 100, ended, "bob").ok, true);
});

// A layered score plays several sounds' chords at once. The Studio's own document playback may hold
// up to JAM_DOCUMENT_HELD_MAX, which still leaves a live performer its full 16 under the ceiling.
test("a document playback may hold up to 48 notes and a live performer still gets its 16", () => {
  const allocator = new JamVoiceAllocator();
  const ended: [string, JamVoiceEndReason][] = [];
  const doc = (id: string) => allocator.allocate({ id, source: `doc-${id}`, owner: "doc", phase: "held", startedAtMs: 0, heldCap: JAM_DOCUMENT_HELD_MAX, teardown: () => {} });
  for (let index = 0; index < JAM_DOCUMENT_HELD_MAX; index += 1) assert.equal(doc(`n${index}`).ok, true, `note ${index}`);
  assert.deepEqual(doc("over"), { ok: false, reason: "source-held" });
  for (let index = 0; index < JAM_HELD_PER_PEER; index += 1) {
    assert.equal(add(allocator, `live:${index}`, "live", "held", 1, ended, "live").ok, true, "the live performer keeps its full allowance");
  }
  assert.equal(JAM_DOCUMENT_HELD_MAX + JAM_HELD_PER_PEER, JAM_VOICES_GLOBAL);
});

test("a held-note cap outside 16..48 is a programming error", () => {
  const allocator = new JamVoiceAllocator();
  for (const heldCap of [0, JAM_HELD_PER_PEER - 1, JAM_DOCUMENT_HELD_MAX + 1, 1.5, Number.NaN]) {
    assert.throws(() => allocator.allocate({ id: `x${heldCap}`, source: "s", phase: "held", startedAtMs: 0, heldCap, teardown: () => {} }), /held-note cap/);
  }
});

test("a source cannot hold more than the musical polyphony limit", () => {
  const allocator = new JamVoiceAllocator();
  const ended: [string, JamVoiceEndReason][] = [];
  for (let index = 0; index < JAM_HELD_PER_PEER; index += 1) {
    assert.equal(add(allocator, `a${index}`, "alice", "held", index, ended).ok, true);
  }
  assert.deepEqual(add(allocator, "too-many", "alice", "held", 99, ended), { ok: false, reason: "source-held" });
  assert.deepEqual(ended, []);
});

test("a spamming peer can never steal another performer's held chord", () => {
  const allocator = new JamVoiceAllocator();
  const ended: [string, JamVoiceEndReason][] = [];
  // Four full held chords occupy all 64 room slots. Mallory may retry forever; no held teardown
  // callback is allowed to run. Removing this fairness rule makes this regression fail loudly.
  for (let peer = 0; peer < JAM_VOICES_GLOBAL / JAM_HELD_PER_PEER; peer += 1) {
    for (let note = 0; note < JAM_HELD_PER_PEER; note += 1) {
      assert.equal(add(allocator, `peer${peer}:${note}`, `peer${peer}`, "held", peer * 100 + note, ended).ok, true);
    }
  }
  for (let attempt = 0; attempt < 100; attempt += 1) {
    assert.deepEqual(add(allocator, `mallory:${attempt}`, "mallory", "held", 1_000 + attempt, ended), {
      ok: false,
      reason: "room-held",
    });
  }
  assert.equal(allocator.snapshot().length, JAM_VOICES_GLOBAL);
  assert.deepEqual(ended, []);
});

test("the allocator steals the requester's own oldest tail before anyone else's", () => {
  const allocator = new JamVoiceAllocator();
  const ended: [string, JamVoiceEndReason][] = [];
  assert.equal(add(allocator, "other-tail", "bob", "tail", 0, ended).ok, true);
  assert.equal(add(allocator, "own-old", "alice", "tail", 1, ended).ok, true);
  assert.equal(add(allocator, "own-new", "alice", "tail", 2, ended).ok, true);
  for (let index = 3; index < JAM_VOICES_GLOBAL; index += 1) {
    assert.equal(add(allocator, `fill${index}`, `source${index}`, "tail", index, ended).ok, true);
  }
  const accepted = add(allocator, "incoming", "alice", "held", 500, ended);
  assert.deepEqual(accepted, { ok: true, stolen: ["own-old"] });
  assert.deepEqual(ended, [["own-old", "stolen"]]);
  assert.equal(allocator.has("own-old"), false);
  assert.equal(allocator.has("other-tail"), true);
});

test("steal removes ownership before invoking full teardown", () => {
  const allocator = new JamVoiceAllocator();
  let absentInsideCallback = false;
  allocator.allocate({
    id: "old",
    source: "alice",
    phase: "tail",
    startedAtMs: 0,
    teardown: () => { absentInsideCallback = !allocator.has("old"); },
  });
  for (let index = 1; index < JAM_VOICES_GLOBAL; index += 1) {
    add(allocator, `fill${index}`, "bob", "tail", index, []);
  }
  allocator.allocate({ id: "new", source: "alice", phase: "held", startedAtMs: 99, teardown: () => {} });
  assert.equal(absentInsideCallback, true);
});

test("watchdog reports only held voices at or beyond 30 seconds", () => {
  const allocator = new JamVoiceAllocator();
  const ended: [string, JamVoiceEndReason][] = [];
  add(allocator, "old-held", "alice", "held", 0, ended);
  add(allocator, "new-held", "alice", "held", 1, ended);
  add(allocator, "tail", "alice", "tail", 0, ended);
  assert.deepEqual(allocator.expiredHeld(JAM_REMOTE_HOLD_MAX_MS - 1), []);
  assert.deepEqual(allocator.expiredHeld(JAM_REMOTE_HOLD_MAX_MS), ["old-held"]);
});

test("a throwing teardown cannot strand the rest of a source", () => {
  const allocator = new JamVoiceAllocator();
  allocator.allocate({ id: "bad", source: "alice", phase: "held", startedAtMs: 0, teardown: () => { throw new Error("browser edge"); } });
  allocator.allocate({ id: "good", source: "alice", phase: "held", startedAtMs: 1, teardown: () => {} });
  assert.deepEqual(allocator.releaseSource("alice"), ["bad", "good"]);
  assert.equal(allocator.snapshot().length, 0);
});
