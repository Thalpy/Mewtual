import assert from "node:assert/strict";
import test from "node:test";
import { JAM_HELD_PER_PEER, JAM_REMOTE_HOLD_MAX_MS, JAM_VOICES_GLOBAL } from "./jam-contract.ts";
import { JamVoiceAllocator, type JamVoiceEndReason } from "./jam-allocator.ts";

function add(
  allocator: JamVoiceAllocator,
  id: string,
  source: string,
  phase: "held" | "tail",
  at: number,
  ended: [string, JamVoiceEndReason][],
) {
  return allocator.allocate({
    id,
    source,
    phase,
    startedAtMs: at,
    teardown: (reason) => ended.push([id, reason]),
  });
}

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
