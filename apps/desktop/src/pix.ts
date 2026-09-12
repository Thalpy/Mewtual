// pix:v1, the byte-exact indexed raster every drawing, doodle, stamp and flipnote frame is
// (design-creative-suite.md 2.1). Decoded by this code and only this code: bytes never reach an
// <img>. The encoder emits maximal runs, so the same pixels under the same palette always give
// the same bytes; the decoder rejects everything the design says it must, before allocating
// anything it has not bounded.
//
//   magic   4 bytes "PIX1"
//   w1 h1   u8 each, side = value + 1 (1..256)
//   n1      u8, palette entries = n1 + 1 (4..16)
//   palette N x (role u8, r u8, g u8, b u8)
//   runs    (len1 u8, idx u8) pairs until exactly W*H pixels

import {
  PIX_MAGIC,
  PIX_MAX_BYTES,
  PIX_MAX_PALETTE,
  PIX_MAX_PIXELS,
  PIX_MAX_RUN,
  PIX_MIN_PALETTE,
  PIX_ROLE_MAX,
  type PixImage,
  type PixPaletteEntry,
  type PixRole,
} from "./studio-contract.ts";

export class PixError extends Error {
  readonly reason: string;
  constructor(reason: string) {
    super(`pix:v1: ${reason}`);
    this.reason = reason;
  }
}

const MAGIC = new TextEncoder().encode(PIX_MAGIC);
const HEADER_BYTES = 4 + 3;

/// Validate a palette the way the decoder would, so an editor cannot build an image the
/// encoder will happily emit and every receiver will reject.
export function checkPalette(palette: readonly PixPaletteEntry[]): void {
  if (palette.length < PIX_MIN_PALETTE || palette.length > PIX_MAX_PALETTE) throw new PixError("palette size");
  const seen = new Set<string>();
  for (const e of palette) {
    if (!Number.isInteger(e.role) || e.role < 0 || e.role > PIX_ROLE_MAX) throw new PixError("palette role");
    for (const c of [e.r, e.g, e.b]) if (!Number.isInteger(c) || c < 0 || c > 255) throw new PixError("palette colour");
    const key = `${e.role}:${e.r}:${e.g}:${e.b}`;
    if (seen.has(key)) throw new PixError("duplicate palette entry");
    seen.add(key);
  }
}

/// Encode. Runs are maximal by construction: a run ends only at the palette index changing, the
/// 256-pixel run cap, or the end of the raster.
export function encodePix(img: PixImage): Uint8Array {
  const { w, h, palette, pixels } = img;
  if (!Number.isInteger(w) || !Number.isInteger(h) || w < 1 || h < 1 || w > 256 || h > 256) throw new PixError("size");
  if (w * h > PIX_MAX_PIXELS) throw new PixError("too many pixels");
  if (pixels.length !== w * h) throw new PixError("pixel count");
  checkPalette(palette);
  const n = palette.length;
  const runs: number[] = [];
  let i = 0;
  while (i < pixels.length) {
    const idx = pixels[i];
    if (idx >= n) throw new PixError("index out of palette");
    let len = 1;
    while (len < PIX_MAX_RUN && i + len < pixels.length && pixels[i + len] === idx) len++;
    runs.push(len - 1, idx);
    i += len;
  }
  const out = new Uint8Array(HEADER_BYTES + n * 4 + runs.length);
  out.set(MAGIC, 0);
  out[4] = w - 1;
  out[5] = h - 1;
  out[6] = n - 1;
  let p = HEADER_BYTES;
  for (const e of palette) {
    out[p++] = e.role;
    out[p++] = e.r;
    out[p++] = e.g;
    out[p++] = e.b;
  }
  out.set(runs, p);
  if (out.length > PIX_MAX_BYTES) throw new PixError("over 64 KiB");
  return out;
}

/// Decode, in the design's order: cap on the encoded length first, fixed header, pixel-count
/// bound, one allocation, bounds-checked runs, then reject overshoot, undershoot, trailing bytes.
export function decodePix(bytes: Uint8Array): PixImage {
  if (bytes.length > PIX_MAX_BYTES) throw new PixError("over 64 KiB");
  if (bytes.length < HEADER_BYTES) throw new PixError("truncated header");
  for (let i = 0; i < 4; i++) if (bytes[i] !== MAGIC[i]) throw new PixError("bad magic");
  const w = bytes[4] + 1;
  const h = bytes[5] + 1;
  const n = bytes[6] + 1;
  if (n < PIX_MIN_PALETTE || n > PIX_MAX_PALETTE) throw new PixError("palette size");
  if (w * h > PIX_MAX_PIXELS) throw new PixError("too many pixels");
  let p = HEADER_BYTES;
  if (bytes.length < p + n * 4) throw new PixError("truncated palette");
  const palette: PixPaletteEntry[] = [];
  for (let i = 0; i < n; i++) {
    const role = bytes[p++];
    if (role > PIX_ROLE_MAX) throw new PixError("palette role");
    palette.push({ role: role as PixRole, r: bytes[p++], g: bytes[p++], b: bytes[p++] });
  }
  checkPalette(palette);
  const total = w * h;
  const pixels = new Uint8Array(total);
  let filled = 0;
  let lastIdx = -1;
  while (filled < total) {
    if (p + 2 > bytes.length) throw new PixError("undershoot");
    const len = bytes[p] + 1;
    const idx = bytes[p + 1];
    p += 2;
    if (idx >= n) throw new PixError("index out of palette");
    if (idx === lastIdx) throw new PixError("non-maximal run");
    if (filled + len > total) throw new PixError("overshoot");
    pixels.fill(idx, filled, filled + len);
    filled += len;
    // Two consecutive runs of one index are legal only when the first was cut by the run cap.
    lastIdx = len === PIX_MAX_RUN ? -1 : idx;
  }
  if (p !== bytes.length) throw new PixError("trailing bytes");
  return { w, h, palette, pixels };
}

/// Paint decoded pixels into RGBA, through a resolver that may swap role colours for the
/// viewer's own theme ("adapt drawings to my theme"). The resolver gets the palette entry and
/// returns [r, g, b]; the default returns the authored fallback.
export function pixToRgba(
  img: PixImage,
  resolve: (entry: PixPaletteEntry, index: number) => readonly [number, number, number] = (e) => [e.r, e.g, e.b],
  out: Uint8ClampedArray<ArrayBuffer> = new Uint8ClampedArray(img.w * img.h * 4),
): Uint8ClampedArray<ArrayBuffer> {
  const lut = img.palette.map((e, i) => resolve(e, i));
  const { pixels } = img;
  for (let i = 0, o = 0; i < pixels.length; i++, o += 4) {
    const c = lut[pixels[i]] ?? [255, 0, 255];
    out[o] = c[0];
    out[o + 1] = c[1];
    out[o + 2] = c[2];
    out[o + 3] = 255;
  }
  return out;
}

/// A placeholder content address for in-memory records: FNV-1a folded to 64 hex so it has the
/// shape the fileshare uses. NOT the sealed blob store's cid; replaced when `publish_pix` lands.
export function localCid(bytes: Uint8Array): string {
  let a = 0x811c9dc5 >>> 0;
  let b = 0x01000193 >>> 0;
  for (let i = 0; i < bytes.length; i++) {
    a = Math.imul(a ^ bytes[i], 0x01000193) >>> 0;
    b = Math.imul(b ^ ((bytes[i] + i) & 0xff), 0x9e3779b1) >>> 0;
  }
  let s = "";
  for (let k = 0; k < 8; k++) {
    a = Math.imul(a ^ (a >>> 15), 0x2c1b3c6d) >>> 0;
    b = Math.imul(b ^ (b >>> 13), 0x297a2d39) >>> 0;
    s += (a >>> 0).toString(16).padStart(8, "0");
    s += (b >>> 0).toString(16).padStart(8, "0");
    if (s.length >= 64) break;
  }
  return s.slice(0, 64);
}
