import assert from "node:assert/strict";
import test from "node:test";
import { RtPeerAdmission, RtSourceChannelRegistry, RtTokenBucket, fitsUtf8Bytes, type RtFrameLimits } from "./rt-admission.ts";
import { JamSourceChannelRegistry } from "./jam-channel.ts";
import { JAM_FRAME_LIMITS } from "./jam-budget.ts";

const LIMITS: RtFrameLimits = {
  maxBytes: 16,
  bucketBurst: 4,
  bucketRatePerSecond: 2,
  abuseExhaustedMs: 3_000,
  abuseWindowMs: 10_000,
};

test("a bucket never refills on a backward or non-finite clock step", () => {
  const bucket = new RtTokenBucket(1, 1);
  assert.equal(bucket.charge(5_000), true);
  assert.equal(bucket.charge(1_000), false, "backward step refills nothing");
  assert.equal(bucket.charge(Number.NaN), false);
  assert.equal(bucket.charge(5_999), false, "the refill origin did not move backwards");
  assert.equal(bucket.charge(6_000), true);
  assert.throws(() => new RtTokenBucket(0, 1), RangeError);
});

test("the gate charges before looking at the frame, then bounds UTF-8 bytes", () => {
  const gate = new RtPeerAdmission(LIMITS);
  assert.deepEqual(gate.admitFrame(42, 0), { ok: false, reason: "not-string" });
  assert.deepEqual(gate.admitFrame("x".repeat(17), 0), { ok: false, reason: "too-large" });
  // Nine two-byte characters are 9 UTF-16 units but 18 bytes.
  assert.deepEqual(gate.admitFrame("é".repeat(9), 0), { ok: false, reason: "too-large" });
  assert.deepEqual(gate.admitFrame("ok", 0), { ok: true, raw: "ok" });
  // All four burst tokens were spent by the four deliveries above, valid or not.
  assert.deepEqual(gate.admitFrame("ok", 0), { ok: false, reason: "frame-rate" });
});

test("sustained exhaustion mutes for the call, a manual unmute starts fresh", () => {
  const gate = new RtPeerAdmission(LIMITS);
  for (let i = 0; i < LIMITS.bucketBurst; i += 1) gate.admitFrame("x", 0);
  let ms = 0;
  while (!gate.isAbuseMuted() && ms < 20_000) {
    ms += 10;
    gate.admitFrame("x", ms);
  }
  assert.equal(gate.isAbuseMuted(), true);
  assert.ok(ms >= LIMITS.abuseExhaustedMs, "a short burst is not enough");
  assert.deepEqual(gate.admitFrame("x", ms + 60_000), { ok: false, reason: "abuse-muted" }, "time alone never unmutes");
  gate.clearAbuseMute();
  assert.deepEqual(gate.admitFrame("x", ms + 60_000), { ok: true, raw: "x" });
});

test("utf-8 fit checks bytes, not UTF-16 units", () => {
  assert.equal(fitsUtf8Bytes("abc", 3), true);
  assert.equal(fitsUtf8Bytes("abcd", 3), false);
  assert.equal(fitsUtf8Bytes("😀", 3), false, "one astral character is four bytes");
});

test("capabilities are per kind, per source, and only the newest is current", () => {
  const draw = new RtSourceChannelRegistry("draw");
  const jam = new JamSourceChannelRegistry();
  const a = "a".repeat(64);
  const first = draw.open(a);
  const second = draw.open(a);
  assert.equal(first.kind, "draw");
  assert.equal(draw.isCurrent(first), false);
  assert.equal(draw.isCurrent(second), true);
  // A different registry never recognises another registry's token, even for the same source.
  const jamToken = jam.open(a);
  assert.equal(jam.isCurrent(jamToken), true);
  assert.equal((draw as RtSourceChannelRegistry<string>).isCurrent(jamToken), false);
  assert.equal(draw.close(a), true);
  assert.equal(draw.isCurrent(second), false);
  assert.throws(() => draw.open(""), TypeError);
});

test("the jam gate keeps exactly the jam:v2 limits it had before the extraction", () => {
  assert.deepEqual({ ...JAM_FRAME_LIMITS }, {
    maxBytes: 1024,
    bucketBurst: 160,
    bucketRatePerSecond: 80,
    abuseExhaustedMs: 10_000,
    abuseWindowMs: 60_000,
  });
});
