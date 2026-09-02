import assert from "node:assert/strict";
import test from "node:test";
import {
  JAM_FRAME_BUCKET_BURST,
  JAM_FRAME_BUCKET_RATE,
  JAM_CLOCK_PROBE_BURST,
  JAM_LEGACY_SESSION_NONCE,
  JAM_FRAME_MAX_BYTES,
  JAM_MUTED_STATE_BUCKET_BURST,
  JAM_NOTEON_BUCKET_BURST,
  JAM_PATCH_ID_HEX_CHARS,
  JAM_SESSION_NONCE_HEX_CHARS,
  JAM_SMALL_FRAME_MAX_BYTES,
  type JamPatch,
} from "./jam-contract.ts";
import { JamFrameDecoder, toggleJamPeerMute } from "./jam-wire.ts";

const id = "a".repeat(JAM_PATCH_ID_HEX_CHARS);
const sn = "b".repeat(JAM_SESSION_NONCE_HEX_CHARS);
const TEST_PATCH: JamPatch = {
  v: 1,
  o: [{ w: 1, t: 0, c: 0, l: 100 }],
  e: { a: 10, d: 100, s: 60, r: 200 },
  f: { m: 0, c: 4_000, q: 10, e: 0 },
  l: { r: 100, d: 0, t: 0 },
  x: { c: 0, d: 0, r: 0 },
};

test("the decoder admits the v2 note, patch and old-client-safe drum shapes", () => {
  const decoder = new JamFrameDecoder();
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "n", on: 1, n: 60, w: "triangle", p: id, q: 1 }), 0),
    { ok: true, kind: "jam", message: { t: "n", on: 1, n: 60, w: "triangle", p: id, q: 1 } },
  );
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "n", on: 0, n: 60, q: 2 }), 0),
    { ok: true, kind: "jam", message: { t: "n", on: 0, n: 60, q: 2 } },
  );
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "d", n: 4, q: 3 }), 0),
    { ok: true, kind: "jam", message: { t: "d", n: 4, q: 3 } },
  );
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "p", v: 1, id, sn, d: TEST_PATCH }), 0),
    { ok: true, kind: "jam", message: { t: "p", v: 1, id, sn, d: TEST_PATCH } },
  );
});

test("the rejected t:n drum shape cannot strand notes on an old build", () => {
  const decoder = new JamFrameDecoder();
  const legacyHazard = decoder.decode(JSON.stringify({ t: "n", on: 1, d: 1, n: 4, q: 1 }), 0);
  assert.deepEqual(legacyHazard, { ok: false, reason: "shape" });
});

test("exact pre-v2 notes normalize on their own authenticated-channel sequence domain", () => {
  const decoder = new JamFrameDecoder();
  assert.deepEqual(decoder.decode(JSON.stringify({ t: "n", on: 1, n: 60, w: "triangle" }), 0), {
    ok: true,
    kind: "legacy-note",
    sessionNonce: JAM_LEGACY_SESSION_NONCE,
    message: { t: "n", on: 1, n: 60, w: "triangle", q: 0 },
  });
  assert.deepEqual(decoder.decode(JSON.stringify({ t: "n", on: 0, n: 60 }), 0), {
    ok: true,
    kind: "legacy-note",
    sessionNonce: JAM_LEGACY_SESSION_NONCE,
    message: { t: "n", on: 0, n: 60, q: 1 },
  });
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "n", on: 0, n: 60, extra: true }), 0),
    { ok: false, reason: "shape" },
  );
});

test("full patch ids still fit the 200-byte note-frame compatibility cap", () => {
  const raw = JSON.stringify({ t: "n", on: 1, n: 127, w: "sawtooth", p: "f".repeat(64), q: 0xffff_ffff });
  assert.ok(new TextEncoder().encode(raw).byteLength < JAM_SMALL_FRAME_MAX_BYTES);
  assert.equal(new JamFrameDecoder().decode(raw, 0).ok, true);
});

test("oversize, malformed and unknown-field messages fail before subtype work", () => {
  const decoder = new JamFrameDecoder();
  assert.deepEqual(decoder.decode("x".repeat(JAM_FRAME_MAX_BYTES + 1), 0), { ok: false, reason: "too-large" });
  assert.deepEqual(decoder.decode("{", 0), { ok: false, reason: "json" });
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "n", on: 1, n: 60, w: "triangle", q: 1, gain: 10_000 }), 0),
    { ok: false, reason: "shape" },
  );
  const padded = JSON.stringify({ t: "s", pad: "x".repeat(JAM_SMALL_FRAME_MAX_BYTES) });
  assert.deepEqual(decoder.decode(padded, 0), { ok: false, reason: "small-frame" });
});

test("garbage consumes the all-frame bucket before JSON.parse", () => {
  const decoder = new JamFrameDecoder();
  for (let index = 0; index < JAM_FRAME_BUCKET_BURST; index += 1) {
    assert.deepEqual(decoder.decode("not-json", 0), { ok: false, reason: "json" });
  }
  assert.deepEqual(decoder.decode("not-json", 0), { ok: false, reason: "frame-rate" });
});

test("note-offs remain available after the musical note-on bucket is empty", () => {
  const decoder = new JamFrameDecoder();
  for (let index = 0; index < JAM_NOTEON_BUCKET_BURST; index += 1) {
    assert.equal(decoder.decode(JSON.stringify({ t: "n", on: 1, n: index % 128, w: "sine", q: index }), 0).ok, true);
  }
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "n", on: 1, n: 64, w: "sine", q: 61 }), 0),
    { ok: false, reason: "note-rate" },
  );
  assert.equal(decoder.decode(JSON.stringify({ t: "n", on: 0, n: 64, q: 62 }), 0).ok, true);
});

test("metronome revisions are session-scoped and clock probes have exact shapes", () => {
  const decoder = new JamFrameDecoder();
  assert.equal(decoder.decode(JSON.stringify({ t: "m", v: 1, sn, on: 1, rev: 0, bpm: 120, bpb: 4, org: 5 }), 0).ok, true);
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "m", v: 1, on: 1, rev: 0, bpm: 120, bpb: 4, org: 5 }), 0),
    { ok: false, reason: "shape" },
  );
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "m", v: 1, sn: JAM_LEGACY_SESSION_NONCE, on: 1, rev: 0, bpm: 120, bpb: 4, org: 5 }), 0),
    { ok: false, reason: "shape" },
  );
  assert.equal(decoder.decode(JSON.stringify({ t: "c", q: 7, tx: 12.5 }), 0).ok, true);
  assert.equal(decoder.decode(JSON.stringify({ t: "c", r: 7, tx: 13, rx: 12.5 }), 0).ok, true);
  assert.deepEqual(decoder.decode(JSON.stringify({ t: "c", r: 7, tx: 13 }), 0), { ok: false, reason: "shape" });
});

test("incoming clock probes have a dedicated anti-amplification bucket", () => {
  const decoder = new JamFrameDecoder();
  for (let q = 0; q < JAM_CLOCK_PROBE_BURST; q += 1) {
    assert.equal(decoder.decode(JSON.stringify({ t: "c", q, tx: q }), 0).ok, true);
  }
  assert.deepEqual(decoder.decode(JSON.stringify({ t: "c", q: 99, tx: 99 }), 0), {
    ok: false,
    reason: "clock-rate",
  });
  // Replies are correlation-checked by JamClockProbeTracker and must not spend the request bucket.
  assert.equal(decoder.decode(JSON.stringify({ t: "c", r: 0, tx: 0, rx: 1 }), 0).ok, true);
});

test("unrelated bounded state frames remain available to the existing call handler", () => {
  const state = { t: "s", mic: 1, inst: 0, vid: 2, rx: 1080 };
  assert.deepEqual(new JamFrameDecoder().decode(JSON.stringify(state), 0), { ok: true, kind: "other", value: state });
});

test("auto-muted music retains a tiny pre-parse lane for exact call state", () => {
  const decoder = new JamFrameDecoder();
  for (let index = 0; index < JAM_FRAME_BUCKET_BURST; index += 1) decoder.decode("{}", 0);
  for (let ms = 1; ms <= 20_000 && !decoder.budget.isAbuseMuted(); ms += 1) {
    decoder.decode("{}", ms);
  }
  assert.equal(decoder.budget.isAbuseMuted(), true);

  const state = { t: "s", mic: 1, inst: 0, vid: 2, rx: 1080, rec: 0, rc: 0 };
  assert.deepEqual(decoder.decode(JSON.stringify(state), 20_001), {
    ok: true,
    kind: "other",
    value: state,
  });
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "n", on: 1, n: 60, w: "sine", q: 1 }), 20_001),
    { ok: false, reason: "abuse-muted" },
  );
  assert.deepEqual(
    decoder.decode(JSON.stringify({ ...state, href: "https://attacker.invalid" }), 20_001),
    { ok: false, reason: "abuse-muted" },
  );
  // The test intentionally keeps pressure continuous; this import pins that the same production
  // refill rate is used rather than a test-only time shortcut.
  assert.ok(JAM_FRAME_BUCKET_RATE > 0);
});

test("wrong-type frames consume the post-mute state lane before inspection", () => {
  const decoder = new JamFrameDecoder();
  for (let index = 0; index < JAM_FRAME_BUCKET_BURST; index += 1) decoder.decode("{}", 0);
  for (let ms = 1; ms <= 20_000 && !decoder.budget.isAbuseMuted(); ms += 1) {
    decoder.decode("{}", ms);
  }
  assert.equal(decoder.budget.isAbuseMuted(), true);

  for (let index = 0; index < JAM_MUTED_STATE_BUCKET_BURST; index += 1) {
    assert.deepEqual(decoder.decode(new Uint8Array(), 20_001), { ok: false, reason: "abuse-muted" });
  }
  assert.deepEqual(
    decoder.decode(JSON.stringify({ t: "s", mic: 1 }), 20_001),
    { ok: false, reason: "abuse-muted" },
  );
});

test("one click on an effective flood mute forgives it instead of adding a manual mute", () => {
  assert.deepEqual(toggleJamPeerMute(false, true), { manuallyMuted: false, forgiveAbuse: true });
  assert.deepEqual(toggleJamPeerMute(true, true), { manuallyMuted: false, forgiveAbuse: true });
  assert.deepEqual(toggleJamPeerMute(false, false), { manuallyMuted: true, forgiveAbuse: false });
});
