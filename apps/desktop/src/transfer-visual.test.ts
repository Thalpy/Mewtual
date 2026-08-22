import test from "node:test";
import assert from "node:assert/strict";
import {
  formatBytes, formatRate, sampleRate, transferPieces, uploadContract,
} from "./transfer-visual.ts";

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

test("a well-formed upload ticket passes through unchanged", () => {
  assert.deepEqual(uploadContract({ token: "abc", chunkTotal: 3, sliceBytes: 1024 * 1024 }), {
    token: "abc",
    chunkTotal: 3,
    sliceBytes: 1024 * 1024,
  });
  // Extra keys are the native side being ahead of this file, which is not a reason to refuse a
  // ticket that states everything the loop needs.
  assert.deepEqual(uploadContract({ token: "a", chunkTotal: 1, sliceBytes: 8, extra: true }), {
    token: "a",
    chunkTotal: 1,
    sliceBytes: 8,
  });
});

test("a snake_case upload ticket is refused instead of slicing by undefined", () => {
  // The shipped bug: the native struct serialized token/chunk_total/slice_bytes while this side
  // reads chunkTotal/sliceBytes. Neither language could see it. The upload loop stepped by
  // undefined, Blob.slice turned the resulting NaN into an empty slice, and the native side
  // rejected the first slice with "only the last slice of an upload may be short": a message
  // about this side's own behaviour that named nothing anyone could act on.
  assert.throws(
    () => uploadContract({ token: "abc", chunk_total: 3, slice_bytes: 1024 * 1024 }),
    /chunk count/,
  );
});

test("an upload ticket missing any part of the contract is refused", () => {
  const ok = { token: "abc", chunkTotal: 3, sliceBytes: 1024 * 1024 };
  assert.throws(() => uploadContract(undefined), /invalid upload ticket/);
  assert.throws(() => uploadContract("abc"), /invalid upload ticket/);
  assert.throws(() => uploadContract({ ...ok, token: "" }), /no token/);
  assert.throws(() => uploadContract({ ...ok, token: 7 }), /no token/);
  // Zero and a fraction are the same fault as a missing key: a count the loop divides by and a
  // size it steps by cannot be either and still produce chunk boundaries a reader can find.
  assert.throws(() => uploadContract({ ...ok, chunkTotal: 0 }), /chunk count/);
  assert.throws(() => uploadContract({ ...ok, sliceBytes: 0 }), /slice size/);
  assert.throws(() => uploadContract({ ...ok, sliceBytes: 1.5 }), /slice size/);
  assert.throws(() => uploadContract({ ...ok, sliceBytes: Number.NaN }), /slice size/);
  assert.throws(() => uploadContract({ ...ok, sliceBytes: "1048576" }), /slice size/);
});
