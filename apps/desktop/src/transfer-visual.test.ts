import test from "node:test";
import assert from "node:assert/strict";
import {
  formatBytes, formatRate, sampleRate, transferPieces,
  TRANSFER_CHUNK_BYTES, TRANSFER_SLICE_BYTES,
} from "./transfer-visual.ts";

test("an upload slice divides a seal chunk exactly", () => {
  // The native side seals a chunk once its buffer reaches TRANSFER_CHUNK_BYTES. If a slice did
  // not divide a chunk, a buffer would overshoot and chunks would stop being uniformly sized,
  // which the media reader (offset / chunk size = chunk index) depends on.
  assert.ok(TRANSFER_SLICE_BYTES > 0);
  assert.ok(TRANSFER_SLICE_BYTES < TRANSFER_CHUNK_BYTES, "a slice is smaller than a chunk");
  assert.equal(TRANSFER_CHUNK_BYTES % TRANSFER_SLICE_BYTES, 0);
});

test("piece strips distinguish held, active and pending chunks", () => {
  assert.deepEqual(transferPieces(5, 2, true, true, false, false), [
    "held", "held", "active", "pending", "pending",
  ]);
});

test("missing chunks are red without a connection and failed after an error", () => {
  assert.deepEqual(transferPieces(3, 1, false, false, false, false), ["held", "offline", "offline"]);
  assert.deepEqual(transferPieces(3, 1, false, true, true, false), ["held", "failed", "failed"]);
});

test("completion makes every chunk held", () => {
  assert.deepEqual(transferPieces(3, 0, false, false, true, true), ["held", "held", "held"]);
});

test("byte and rate labels stay compact", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(8 * 1024 * 1024), "8.0 MiB");
  assert.equal(formatRate(2 * 1024 * 1024), "2.0 MiB/s");
});

test("rate samples use byte deltas and smooth later updates", () => {
  assert.equal(sampleRate(0, 0, 1_000, 1_000, 2_000), 1_000);
  assert.equal(sampleRate(1_000, 1_000, 2_000, 3_000, 3_000), 1_350);
  assert.equal(sampleRate(1_350, 3_000, 3_000, 3_000, 4_000), 1_350);
});
