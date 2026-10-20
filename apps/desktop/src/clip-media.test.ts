import assert from "node:assert/strict";
import test from "node:test";
import { decodedBytes, decodeSize, fileRefusal, fileRole, imageSize, MEDIA_LIMITS, MediaBin, mediaName, readTakeFile, soundDecodeSeconds } from "./clip-media.ts";

test("files are sorted by what they are, by type first and extension second; SVG never counts as a picture", () => {
  assert.equal(fileRole("cat.png", "image/png"), "image");
  assert.equal(fileRole("cat.PNG", ""), "image", "a lost type falls back to the extension");
  assert.equal(fileRole("clip.mov", "video/quicktime"), "video");
  assert.equal(fileRole("song.flac", ""), "audio");
  assert.equal(fileRole("tune.mp3", "audio/mpeg"), "audio");
  assert.equal(fileRole("score.jamtake", "application/json"), "take", "the extension decides a take, whatever the type says");
  assert.equal(fileRole("logo.svg", "image/svg+xml"), null);
  assert.equal(fileRole("logo.svg", ""), null);
  assert.equal(fileRole("sneaky.png", "image/svg+xml"), null, "a declared SVG is refused even with a picture's name");
  assert.equal(fileRole("notes.txt", "text/plain"), null);
});

test("oversized files are refused before anything decodes them", () => {
  assert.equal(fileRefusal("image", MEDIA_LIMITS.imageBytes), null);
  assert.match(fileRefusal("image", MEDIA_LIMITS.imageBytes + 1)!, /too big/);
  assert.match(fileRefusal("audio", MEDIA_LIMITS.soundBytes + 1)!, /too big/);
  assert.match(fileRefusal("take", MEDIA_LIMITS.takeBytes + 1)!, /too big/);
  assert.equal(fileRefusal("video", 10 * 1024 ** 3), null, "videos stream through the demuxer; the export cap bounds them");
  assert.match(fileRefusal("image", Number.NaN)!, /could not be read/);
});

test("pictures decode inside the edge limit, keeping their shape", () => {
  assert.deepEqual(decodeSize(800, 600), { width: 800, height: 600 });
  assert.deepEqual(decodeSize(8000, 4000), { width: MEDIA_LIMITS.imageEdge, height: MEDIA_LIMITS.imageEdge / 2 });
  assert.deepEqual(decodeSize(0, 10), { width: 1, height: 1 });
});

test("names drop the extension and folders, and long ones are shortened", () => {
  assert.equal(mediaName("C:\\pics\\my cat.png"), "my cat");
  assert.equal(mediaName(".hidden"), ".hidden");
  assert.equal(mediaName("   "), "untitled");
  assert.equal(mediaName("a".repeat(40) + ".mp3", 10), "aaaaaaaaa…");
});

// --- Sizes read before decoding --------------------------------------------------------------------
// Found in review: a 2 MB PNG declaring 20000 x 20000 decoded to about 1.6 GB before it was scaled
// down. Its size is now read from the header and refused first.

function png(w: number, h: number): Uint8Array {
  const b = new Uint8Array(33);
  b.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52]);
  new DataView(b.buffer).setUint32(16, w); new DataView(b.buffer).setUint32(20, h);
  return b;
}
function jpeg(w: number, h: number): Uint8Array {
  // SOI, an APP0 segment to skip, then SOF0 with height and width.
  return Uint8Array.from([0xff, 0xd8, 0xff, 0xe0, 0, 6, 1, 2, 3, 4, 0xff, 0xc0, 0, 11, 8, h >> 8, h & 255, w >> 8, w & 255, 3, 0, 0, 0]);
}

test("picture sizes are read from PNG, JPEG, GIF, WebP and BMP headers without decoding", () => {
  assert.deepEqual(imageSize(png(20000, 20000)), { width: 20000, height: 20000 });
  assert.deepEqual(imageSize(jpeg(640, 480)), { width: 640, height: 480 });
  const bmp = new Uint8Array(26); bmp.set([0x42, 0x4d]); new DataView(bmp.buffer).setInt32(18, 100, true); new DataView(bmp.buffer).setInt32(22, -50, true);
  assert.deepEqual(imageSize(bmp), { width: 100, height: 50 }, "a top-down BMP's negative height is still a height");
  const webp = (fourcc: string, fill: (b: Uint8Array) => void) => {
    const b = new Uint8Array(30); b.set([..."RIFF"].map((c) => c.charCodeAt(0))); b.set([..."WEBP"].map((c) => c.charCodeAt(0)), 8);
    b.set([...fourcc].map((c) => c.charCodeAt(0)), 12); fill(b); return b;
  };
  assert.deepEqual(imageSize(webp("VP8 ", (b) => { b[26] = 0x80; b[27] = 0x02; b[28] = 0xe0; b[29] = 0x01; })), { width: 640, height: 480 });
  assert.deepEqual(imageSize(webp("VP8X", (b) => { b.set([0x7f, 0x02, 0x00], 24); b.set([0xdf, 0x01, 0x00], 27); })), { width: 640, height: 480 });
  // VP8L packs 14-bit sizes minus one: width-1 = 0x27f, height-1 = 0x1df.
  assert.deepEqual(imageSize(webp("VP8L", (b) => { b.set([0x7f, 0xc2, 0x77, 0x00], 21); })), { width: 640, height: 480 });
  assert.equal(imageSize(Uint8Array.from([1, 2, 3, 4, 5])), null, "not a picture we can size");
});

/** A GIF: a `screen`-sized logical screen with a global colour table, then the given frames. */
function gif(screen: [number, number], frames: [number, number, number, number][]): Uint8Array {
  const out: number[] = [...[..."GIF89a"].map((c) => c.charCodeAt(0))];
  const le = (n: number) => [n & 255, n >> 8];
  out.push(...le(screen[0]), ...le(screen[1]), 0x80 /* 2-entry global table */, 0, 0, 0, 0, 0, 255, 255, 255);
  out.push(0x21, 0xf9, 4, 0, 0, 0, 0, 0); // a graphic control extension, to skip
  for (const [left, top, w, h] of frames) {
    out.push(0x2c, ...le(left), ...le(top), ...le(w), ...le(h), 0, 2, 2, 0x4c, 0x01, 0); // descriptor, LZW size, one data block
  }
  out.push(0x3b);
  return Uint8Array.from(out);
}

test("a GIF is sized by its frames, not only by the screen it claims", () => {
  // Found in review: a GIF declaring a 1 x 1 screen with a 20000 x 20000 frame passed the pixel
  // check, and Chromium decodes it at the frame's size.
  assert.deepEqual(imageSize(gif([1, 1], [[0, 0, 20000, 20000]])), { width: 20000, height: 20000 });
  assert.deepEqual(imageSize(gif([100, 50], [[0, 0, 100, 50], [90, 40, 30, 30]])), { width: 120, height: 70 }, "a frame poking out of the screen grows it");
  assert.deepEqual(imageSize(gif([320, 32], [[0, 0, 10, 10]])), { width: 320, height: 32 });
  const classic = Uint8Array.from([0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 1, 0, 1, 0, 0x80, 0, 0, 0xff, 0xff, 0xff, 0, 0, 0, 0x21, 0xf9, 4, 1, 0, 0, 0, 0, 0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 1, 0, 0x3b]);
  assert.deepEqual(imageSize(classic), { width: 1, height: 1 }, "the classic 1 x 1 GIF (decoded in Edge too)");
  assert.equal(imageSize(gif([10, 10], [])), null, "no frame at all is malformed");
  assert.equal(imageSize(gif([10, 10], [[0, 0, 5, 5]]).subarray(0, 30)), null, "a file cut off mid-frame is malformed");
  assert.equal(imageSize(png(0, 10)), null, "a zero size is malformed");
  assert.equal(imageSize(Uint8Array.from([0xff, 0xd8, 0xff, 0xe0, 0, 1, 0, 0, 0, 0, 0, 0])), null, "a broken segment length");
});

test("sounds decode only as far as the clip can use, within the sample cap", () => {
  assert.equal(soundDecodeSeconds(40, 3600, 48_000), 40, "an hour-long file decodes 40 s");
  assert.equal(soundDecodeSeconds(40, 12, 48_000), 12);
  assert.equal(soundDecodeSeconds(9999, 9999, 192_000), MEDIA_LIMITS.soundFrames / 192_000, "a high rate decodes fewer seconds");
  assert.equal(soundDecodeSeconds(40, Number.NaN, 48_000), 0);
});

test("a music file that is not JSON is refused with a sentence, never the parser's quote of it", async () => {
  await assert.rejects(readTakeFile(new Blob(["{ secret: oops"])), (e: Error) => e.message === "That music file could not be read.");
});

const base = { name: "clip", file: new Blob([]), duration: 10, width: 640, height: 360, hasAudio: true };

test("the bin refuses what would take it past its decoded-bytes budget", () => {
  const bin = new MediaBin(base, 1000);
  const sound = (frames: number) => ({ kind: "sound" as const, name: "s", buffer: { length: frames, numberOfChannels: 2 } as AudioBuffer, origin: "file" as const });
  assert.equal(decodedBytes(sound(100)), 800);
  assert.ok(bin.add(sound(100)));
  assert.equal(bin.bytes, 800);
  assert.equal(bin.add(sound(100)), null, "800 more would pass 1000");
  assert.ok(bin.add({ kind: "image", name: "p", image: {} as CanvasImageSource, width: 5, height: 10, origin: "file" }), "200 more fits");
  assert.equal(bin.bytes, 1000);
});

test("the bin hands out fresh ids, finds things by kind and key, and stops at its limit", () => {
  const bin = new MediaBin(base);
  assert.equal(bin.video("base")?.duration, 10);
  const closed: string[] = [];
  const image = { close: () => closed.push("pic") } as unknown as CanvasImageSource;
  const a = bin.add({ kind: "image", name: "🐱", image, width: 256, height: 256, origin: "emoji", key: "emoji:🐱" })!;
  const b = bin.add({ kind: "sound", name: "Pow", buffer: {} as AudioBuffer, origin: "builtin", key: "sfx:pow" })!;
  assert.deepEqual([a, b], ["m1", "m2"]);
  assert.equal(bin.image(a)?.name, "🐱");
  assert.equal(bin.sound(a), null, "a picture is not a sound");
  assert.equal(bin.byKey("sfx:pow")?.id, b);
  assert.equal(bin.byKey("sfx:zap"), null);
  assert.deepEqual(bin.list("sound").map((s) => s.id), [b]);
  while (!bin.full) bin.add({ kind: "sound", name: "x", buffer: {} as AudioBuffer, origin: "file" });
  assert.equal(bin.size, MEDIA_LIMITS.items);
  assert.equal(bin.add({ kind: "sound", name: "y", buffer: {} as AudioBuffer, origin: "file" }), null);
  bin.dispose();
  assert.deepEqual(closed, ["pic"], "decoded pictures are released");
  assert.equal(bin.get(a), null);
});
