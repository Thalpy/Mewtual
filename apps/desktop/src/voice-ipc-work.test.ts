import assert from "node:assert/strict";
import test from "node:test";
import { decodeVoiceWorkSnapshot } from "./voice-ipc-work.ts";

const owner = "voice-0000000000000001", executor = "09".repeat(32);
const row = "00000000000000000000000000000001";
const purpose = { kind: "derive", peer: "11".repeat(32), join: "22".repeat(32), keyRef: "33".repeat(32), epoch: "18446744073709551615" };
function sample(state = "pending", identity: unknown = null, actorHandoff: unknown = null) {
  return { kind: "snapshot", owner, executor, nextSlot: 1, work: { row, purpose, state, actorHandoff, identity } };
}

test("one original slot can change Pending to attached without becoming an event cursor", () => {
  const pending = decodeVoiceWorkSnapshot(sample(), owner, executor, 0);
  assert.equal(pending.work?.state, "pending");
  assert.equal(pending.work?.identity, null);
  const action = { action: "18446744073709551615", version: "1", inputTag: "44".repeat(32) };
  const attached = decodeVoiceWorkSnapshot(sample("attached", action), owner, executor, 0);
  assert.equal(attached.work?.row, pending.work?.row);
  assert.equal(attached.work?.state, "attached");
  assert.deepEqual(attached.work?.identity, action);
  assert.ok(Object.isFrozen(attached) && Object.isFrozen(attached.work) && Object.isFrozen(attached.work?.purpose));
  assert.equal(pending.work?.state, "pending");
});

test("empty, Closing and ClosedPending remain distinct from an attached action or drainage proof", () => {
  const empty = decodeVoiceWorkSnapshot({ kind: "snapshot", owner, executor, nextSlot: 32, work: null }, owner, executor, 0);
  assert.equal(empty.work, null);
  assert.equal(decodeVoiceWorkSnapshot(sample("closing"), owner, executor, 0).work?.state, "closing");
  for (const handedOff of [false, true]) {
    const closed = decodeVoiceWorkSnapshot(sample("closedPending", null, handedOff), owner, executor, 0);
    assert.equal(closed.work?.actorHandoff, handedOff);
    assert.equal(closed.work?.identity, null);
  }
  const addedClaim = { ...sample(), drained: true };
  assert.throws(() => decodeVoiceWorkSnapshot(addedClaim, owner, executor, 0));
});

test("child metadata includes the exact original parent purpose and request", () => {
  const childRow = "00000000000000030000000000000004", trigger = "00000000000000020000000000000003";
  for (const request of [{ request: "start", predecessor: null }, { request: "start", predecessor: "55".repeat(32) },
    { request: "respond", trigger }, { request: "incoming", role: 9 }]) {
    const input = sample();
    const child = { kind: "confirmation", parent: row, parentPurpose: purpose, ...request };
    const message = { ...input, nextSlot: 4, work: { ...input.work, row: childRow, purpose: child } };
    const result = decodeVoiceWorkSnapshot(message, owner, executor, 0);
    assert.deepEqual(result.work?.purpose, child);
    const missing = { ...child } as Record<string, unknown>;
    delete missing.parentPurpose;
    assert.throws(() => decodeVoiceWorkSnapshot({ ...message, work: { ...message.work, purpose: missing } }, owner, executor, 0));
    assert.throws(() => decodeVoiceWorkSnapshot({ ...message, work: { ...message.work, purpose: { ...child, parent: childRow } } }, owner, executor, 0));
    if (request.request === "respond") {
      for (const trigger of [row, childRow]) assert.throws(() => decodeVoiceWorkSnapshot({ ...message, work: { ...message.work, purpose: { ...child, trigger } } }, owner, executor, 0));
    }
  }
});

test("wrong selection, slot movement, shape, u64s and contradictory states are refused", () => {
  for (const change of [{ owner: "voice-0000000000000002" }, { executor: "08".repeat(32) }, { nextSlot: 0 }, { nextSlot: 33 },
    { nextSlot: 1.5 }, { work: null }, { kind: "paired" }, { extra: true }]) {
    assert.throws(() => decodeVoiceWorkSnapshot({ ...sample(), ...change }, owner, executor, 0));
  }
  assert.throws(() => decodeVoiceWorkSnapshot(sample(), owner, executor, 1));
  for (const input of [sample("attached"), sample("pending", { action: "1" }), sample("pending", null, false), sample("closedPending"),
    sample("established"), sample("attached", { action: "0", version: "1", inputTag: "44".repeat(32) }),
    sample("attached", { action: "1", version: "01", inputTag: "44".repeat(32) })]) {
    assert.throws(() => decodeVoiceWorkSnapshot(input, owner, executor, 0));
  }
  for (const epoch of ["18446744073709551616", "01", 1, "1e3"]) {
    const input = sample();
    assert.throws(() => decodeVoiceWorkSnapshot({ ...input, work: { ...input.work, purpose: { ...purpose, epoch } } }, owner, executor, 0));
  }
});
