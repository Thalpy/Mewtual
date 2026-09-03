import assert from "node:assert/strict";
import test from "node:test";
import {
  JAM_FRAME_BUCKET_BURST,
  JAM_FRAME_BUCKET_RATE,
  JAM_FRAME_MAX_BYTES,
  JAM_NOTEON_BUCKET_BURST,
  JAM_PATCH_ANNOUNCE_BURST,
} from "./jam-contract.ts";
import { JamPeerBudget, JamTokenBucket } from "./jam-budget.ts";

test("token buckets refill monotonically and a backward clock step refills nothing", () => {
  const bucket = new JamTokenBucket(2, 1);
  assert.equal(bucket.charge(1_000), true);
  assert.equal(bucket.charge(1_000), true);
  assert.equal(bucket.charge(1_000), false);
  assert.equal(bucket.charge(500), false);
  assert.equal(bucket.charge(2_000), true);
});

test("every delivery pays the frame bucket before parsing, including wrong and oversize types", () => {
  const budget = new JamPeerBudget();
  for (let index = 0; index < JAM_FRAME_BUCKET_BURST - 2; index += 1) {
    assert.equal(budget.admitFrame("not-json", 0).ok, true);
  }
  assert.deepEqual(budget.admitFrame(new Uint8Array([1]), 0), { ok: false, reason: "not-string" });
  assert.deepEqual(budget.admitFrame("x".repeat(JAM_FRAME_MAX_BYTES + 1), 0), { ok: false, reason: "too-large" });
  assert.deepEqual(budget.admitFrame("{}", 0), { ok: false, reason: "frame-rate" });
});

test("UTF-8 byte length closes the non-ASCII frame-size bypass", () => {
  const budget = new JamPeerBudget();
  const multibyte = "é".repeat(Math.floor(JAM_FRAME_MAX_BYTES / 2) + 1);
  assert.ok(multibyte.length < JAM_FRAME_MAX_BYTES);
  assert.deepEqual(budget.admitFrame(multibyte, 0), { ok: false, reason: "too-large" });
});

test("note-on and patch work have independent tighter buckets", () => {
  const budget = new JamPeerBudget();
  for (let index = 0; index < JAM_NOTEON_BUCKET_BURST; index += 1) assert.equal(budget.admitNoteOn(0), true);
  assert.equal(budget.admitNoteOn(0), false);
  for (let index = 0; index < JAM_PATCH_ANNOUNCE_BURST; index += 1) assert.equal(budget.admitPatch(0), true);
  assert.equal(budget.admitPatch(0), false);
  assert.equal(budget.admitPatch(2_000), true);
});

test("sustained pre-parse flooding auto-mutes, while a short burst recovers", () => {
  const short = new JamPeerBudget();
  for (let index = 0; index < JAM_FRAME_BUCKET_BURST; index += 1) short.admitFrame("{}", 0);
  assert.equal(short.admitFrame("{}", 0).ok, false);
  assert.equal(short.admitFrame("{}", 1_000 / JAM_FRAME_BUCKET_RATE + 1).ok, true);
  assert.equal(short.isAbuseMuted(), false);

  const abusive = new JamPeerBudget();
  for (let index = 0; index < JAM_FRAME_BUCKET_BURST; index += 1) abusive.admitFrame("{}", 0);
  let last: ReturnType<JamPeerBudget["admitFrame"]> = { ok: true, raw: "" };
  // 1 ms traffic keeps the bucket continually pressured; accepted frames refill only a fraction
  // of the stream and do not erase the rolling exhausted intervals.
  for (let ms = 1; ms <= 20_000 && !abusive.isAbuseMuted(); ms += 1) last = abusive.admitFrame("{}", ms);
  assert.equal(abusive.isAbuseMuted(), true);
  assert.deepEqual(last, { ok: false, reason: "abuse-muted" });
  abusive.clearAbuseMute();
  assert.equal(abusive.isAbuseMuted(), false);
});

