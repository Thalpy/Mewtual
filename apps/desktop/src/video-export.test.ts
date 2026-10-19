import assert from "node:assert/strict";
import test from "node:test";
import {
  CappedBytes, ExportCanceledError, OutputTooLargeError, containerMime, editableContainer, editedFileName,
  exportEditedVideo, filmstrip, probeVideo, webCodecsAvailable,
} from "./video-export.ts";
import { initialEdit, setInPoint, setRemoved, splitAt } from "./video-edit.ts";

// --- Eligibility and naming ---------------------------------------------------------------------

test("only containers the demuxer can read are offered for editing, QuickTime as MP4", () => {
  assert.equal(editableContainer("video/mp4"), "mp4");
  assert.equal(editableContainer("VIDEO/MP4; codecs=\"avc1\""), "mp4");
  assert.equal(editableContainer("video/quicktime"), "mp4");
  assert.equal(editableContainer("video/webm"), "webm");
  for (const mime of ["video/ogg", "video/x-msvideo", "video/x-matroska", "audio/mp4", "image/gif", "", "text/html"]) {
    assert.equal(editableContainer(mime), null, mime);
  }
  assert.equal(containerMime("mp4"), "video/mp4");
  assert.equal(containerMime("webm"), "video/webm");
});

test("an edited upload keeps its stem and takes its container's extension", () => {
  assert.equal(editedFileName("holiday.mov", "mp4"), "holiday.mp4");
  assert.equal(editedFileName("clip.mp4", "mp4"), "clip.mp4");
  assert.equal(editedFileName("screen.recording.webm", "webm"), "screen.recording.webm");
  assert.equal(editedFileName("noext", "mp4"), "noext.mp4");
  assert.equal(editedFileName(".mp4", "mp4"), ".mp4.mp4", "a dotfile's name is all stem");
  assert.equal(editedFileName("   ", "webm"), "video.webm");
});

test("WebCodecs is detected from the video coder constructors", () => {
  assert.equal(webCodecsAvailable({}), false);
  assert.equal(webCodecsAvailable({ VideoDecoder: class {}, VideoEncoder: undefined }), false);
  assert.equal(webCodecsAvailable({ VideoDecoder: class {}, VideoEncoder: class {} }), true);
});

// --- Bounded output -----------------------------------------------------------------------------

test("CappedBytes reassembles positional writes across pages, including back-patches", async () => {
  const bytes = new CappedBytes(64, 8);
  bytes.write(0, new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8, 9, 10])); // crosses a page
  bytes.write(10, new Uint8Array([11, 12, 13]));
  bytes.write(2, new Uint8Array([99, 98])); // a size patch behind the write head
  assert.equal(bytes.length, 13);
  const out = new Uint8Array(await bytes.toBlob("video/mp4").arrayBuffer());
  assert.deepEqual([...out], [1, 2, 99, 98, 5, 6, 7, 8, 9, 10, 11, 12, 13]);
});

test("CappedBytes reads an unwritten gap as zeros", async () => {
  const bytes = new CappedBytes(64, 4);
  bytes.write(10, new Uint8Array([7]));
  const out = new Uint8Array(await bytes.toBlob("video/webm").arrayBuffer());
  assert.equal(out.length, 11);
  assert.deepEqual([...out.subarray(0, 10)], new Array(10).fill(0));
  assert.equal(out[10], 7);
});

test("CappedBytes refuses a write past the cap and writes none of it", async () => {
  const bytes = new CappedBytes(10, 4);
  bytes.write(0, new Uint8Array(10).fill(1));
  assert.throws(() => bytes.write(8, new Uint8Array([2, 2, 2])), (error: unknown) =>
    error instanceof OutputTooLargeError && error.cap === 10);
  assert.equal(bytes.length, 10);
  const out = new Uint8Array(await bytes.toBlob("video/mp4").arrayBuffer());
  assert.deepEqual([...out], new Array(10).fill(1), "the refused write left nothing behind");
  // Exactly reaching the cap is fine.
  bytes.write(9, new Uint8Array([5]));
});

test("CappedBytes rejects nonsense configuration and positions", () => {
  assert.throws(() => new CappedBytes(0), RangeError);
  assert.throws(() => new CappedBytes(1.5), RangeError);
  assert.throws(() => new CappedBytes(10, 0), RangeError);
  const bytes = new CappedBytes(10, 4);
  assert.throws(() => bytes.write(-1, new Uint8Array([1])), RangeError);
  assert.throws(() => bytes.write(Number.NaN, new Uint8Array([1])), RangeError);
});

// --- Behaviour without WebCodecs (Node has none) ------------------------------------------------

test("without WebCodecs the probe says editing is unavailable instead of throwing", async () => {
  assert.equal(webCodecsAvailable(), false, "this suite assumes Node has no WebCodecs");
  const result = await probeVideo(new Blob([new Uint8Array(16)], { type: "video/mp4" }));
  assert.equal(result.ok, false);
  assert.ok(!result.ok && result.reason.length > 0);
});

test("without WebCodecs the filmstrip delivers nothing and still resolves", async () => {
  let tiles = 0;
  await filmstrip(new Blob([new Uint8Array(16)]), [0, 1, 2], 40, () => { tiles++; });
  assert.equal(tiles, 0);
});

test("export refuses a container it cannot write before doing any work", async () => {
  const file = new File([new Uint8Array(16)], "a.ogv", { type: "video/ogg" });
  await assert.rejects(exportEditedVideo(file, setInPoint(initialEdit(10), 1), { capBytes: 1024 }), /can't be edited/);
});

test("export refuses an edit that keeps nothing", async () => {
  const file = new File([new Uint8Array(16)], "a.mp4", { type: "video/mp4" });
  const nothing = setRemoved(setRemoved(splitAt(initialEdit(10), 5), 0, true), 1, true);
  await assert.rejects(exportEditedVideo(file, nothing, { capBytes: 1024 }), /Nothing is left/);
});

test("a library failure reaches the editor as a sentence, with the original kept as its cause", async () => {
  // Node can load Mediabunny; it cannot decode, and these bytes are not a video at all, so the
  // demuxer itself throws. That raw error must not become the message the editor shows.
  const file = new File([new Uint8Array(4096).fill(7)], "a.mp4", { type: "video/mp4" });
  await assert.rejects(
    exportEditedVideo(file, setInPoint(initialEdit(10), 1), { capBytes: 1 << 20 }),
    (error: unknown) => error instanceof Error
      && error.message === "The edited video could not be made in this window."
      && error.cause instanceof Error && error.cause.message !== error.message,
  );
});

test("export cancelled before it starts rejects as a cancellation", async () => {
  const file = new File([new Uint8Array(16)], "a.mp4", { type: "video/mp4" });
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(
    exportEditedVideo(file, setInPoint(initialEdit(10), 1), { capBytes: 1024, signal: controller.signal }),
    ExportCanceledError,
  );
});
