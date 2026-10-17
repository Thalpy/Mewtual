import assert from "node:assert/strict";
import test from "node:test";
import { PixError, decodePix, encodePix, localCid, pixToRgba } from "./pix.ts";
import { PIX_MAX_BYTES, type PixImage, type PixPaletteEntry } from "./studio-contract.ts";

const pal4: PixPaletteEntry[] = [
  { role: 1, r: 0x13, g: 0x12, b: 0x18 },
  { role: 2, r: 0xe8, g: 0xe6, b: 0xf0 },
  { role: 3, r: 0x97, g: 0x7d, b: 0xf2 },
  { role: 0, r: 0xe0, g: 0x7a, b: 0xb8 },
];

function img(w: number, h: number, fill: (x: number, y: number) => number, palette = pal4): PixImage {
  const pixels = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) pixels[y * w + x] = fill(x, y);
  return { w, h, palette, pixels };
}

test("golden vector: a 4x2 image with three runs encodes to exactly these bytes", () => {
  // Row 0: 0 0 1 1, row 1: 2 2 2 3 -> runs (2,0) (2,1) (3,2) (1,3), each len stored as len-1.
  const im = img(4, 2, (x, y) => (y === 0 ? (x < 2 ? 0 : 1) : x < 3 ? 2 : 3));
  const bytes = encodePix(im);
  const expected = [
    0x50, 0x49, 0x58, 0x31, // PIX1
    3, 1, 3, // w-1, h-1, n-1
    1, 0x13, 0x12, 0x18, 2, 0xe8, 0xe6, 0xf0, 3, 0x97, 0x7d, 0xf2, 0, 0xe0, 0x7a, 0xb8,
    1, 0, 1, 1, 2, 2, 0, 3,
  ];
  assert.deepEqual(Array.from(bytes), expected);
  const back = decodePix(bytes);
  assert.equal(back.w, 4);
  assert.equal(back.h, 2);
  assert.deepEqual(back.palette, pal4);
  assert.deepEqual(Array.from(back.pixels), Array.from(im.pixels));
});

test("canonical: the same pixels always give the same bytes, and a full 192x144 frame round-trips", () => {
  const a = img(192, 144, (x, y) => ((x >> 4) + (y >> 4)) & 3);
  const b = img(192, 144, (x, y) => ((x >> 4) + (y >> 4)) & 3);
  assert.deepEqual(encodePix(a), encodePix(b));
  const back = decodePix(encodePix(a));
  assert.deepEqual(Array.from(back.pixels), Array.from(a.pixels));
});

test("a run longer than 256 pixels is split at the cap and decodes back as one colour", () => {
  const im = img(256, 3, () => 1); // 768 pixels of index 1 -> three maximal runs of 256
  const bytes = encodePix(im);
  assert.equal(bytes.length, 7 + 16 + 6);
  assert.deepEqual(Array.from(decodePix(bytes).pixels), Array.from(im.pixels));
});

function rejects(bytes: number[], reason: string) {
  assert.throws(() => decodePix(new Uint8Array(bytes)), (e: unknown) => e instanceof PixError && e.reason === reason, reason);
}

test("decode rejects every rule the design names", () => {
  const head = [0x50, 0x49, 0x58, 0x31, 1, 0, 3]; // 2x1, 4 entries
  const pal = [1, 0, 0, 0, 2, 1, 1, 1, 3, 2, 2, 2, 0, 3, 3, 3];
  rejects([0x50, 0x49, 0x58, 0x30, 1, 0, 3, ...pal, 1, 0], "bad magic");
  rejects([...head, ...pal, 0, 0, 0, 0], "non-maximal run"); // (1,0)(1,0) should have been (2,0)
  rejects([...head, ...pal, 0, 0, 0, 7], "index out of palette");
  rejects([...head, ...pal, 2, 0], "overshoot"); // 3 pixels into a 2-pixel raster
  rejects([...head, ...pal, 0, 0], "undershoot"); // one pixel, then nothing
  rejects([...head, ...pal, 1, 0, 9], "trailing bytes");
  rejects([0x50, 0x49, 0x58, 0x31, 1, 0, 2, ...pal.slice(0, 12), 1, 0], "palette size"); // 3 entries
  rejects([...head, 9, 0, 0, 0, ...pal.slice(4), 1, 0], "palette role");
  rejects([...head, 1, 0, 0, 0, 1, 0, 0, 0, 3, 2, 2, 2, 0, 3, 3, 3, 1, 0], "duplicate palette entry");
  rejects([0x50, 0x49, 0x58], "truncated header");
  rejects([...head, ...pal.slice(0, 10)], "truncated palette");
  // 256x256 = 65536 pixels is the cap and legal; the cap check happens before allocation, so a
  // header that claims it with no runs fails on undershoot, not on size.
  rejects([0x50, 0x49, 0x58, 0x31, 255, 255, 3, ...pal], "undershoot");
  const oversize = new Uint8Array(PIX_MAX_BYTES + 1);
  assert.throws(() => decodePix(oversize), (e: unknown) => e instanceof PixError && e.reason === "over 64 KiB");
});

test("encode refuses what decode would refuse", () => {
  assert.throws(() => encodePix(img(2, 1, () => 4)), /index out of palette/);
  assert.throws(() => encodePix(img(2, 1, () => 0, pal4.slice(0, 3))), /palette size/);
  assert.throws(() => encodePix({ w: 2, h: 1, palette: pal4, pixels: new Uint8Array(3) }), /pixel count/);
  assert.throws(() => encodePix(img(2, 1, () => 0, [...pal4.slice(0, 3), pal4[0]])), /duplicate palette entry/);
});

test("rgba painting goes through the resolver, so a theme can recolour role entries", () => {
  const im = img(2, 1, (x) => x);
  const plain = pixToRgba(im);
  assert.deepEqual(Array.from(plain), [0x13, 0x12, 0x18, 255, 0xe8, 0xe6, 0xf0, 255]);
  const adapted = pixToRgba(im, (e) => (e.role === 1 ? [1, 2, 3] : [e.r, e.g, e.b]));
  assert.deepEqual(Array.from(adapted.slice(0, 4)), [1, 2, 3, 255]);
});

test("the placeholder cid is 64 hex, deterministic, and moves with the bytes", () => {
  const a = localCid(encodePix(img(4, 4, (x) => x & 1)));
  assert.match(a, /^[0-9a-f]{64}$/);
  assert.equal(a, localCid(encodePix(img(4, 4, (x) => x & 1))));
  assert.notEqual(a, localCid(encodePix(img(4, 4, (x) => (x + 1) & 1))));
});
