// The pixel editor's controller (design-creative-suite.md 2.2): a palette-locked raster with
// three local layers, the tools (pen, eraser, fill, shapes, stamp, text stamp, mirror) and a
// bounded undo stack. Pure data and functions; the Svelte shell only translates pointer events
// into these calls and paints the result. Every function is exact-integer so the same gestures
// give the same pixels on every machine, which is what makes a frame's bytes canonical.
//
// Layers are an editing convenience: `composite()` flattens them into the one indexed raster
// that pix:v1 carries. Upper layers use CLEAR (255) for "nothing here"; the base layer never
// does, because pix:v1 has no transparent pixel.

import { encodePix } from "./pix.ts";
import {
  BRUSH_MAX,
  BRUSH_MIN,
  LAYER_COUNT,
  UNDO_DEPTH,
  type PixImage,
  type PixPaletteEntry,
} from "./studio-contract.ts";

export const CLEAR = 255;

export type Tool = "pen" | "eraser" | "fill" | "shape" | "stamp" | "text";
export type Shape = "line" | "rect" | "ellipse" | "triangle";
export type Mirror = { h: boolean; v: boolean };
export type Cell = readonly [number, number];
export type Bitmap = { w: number; h: number; pixels: Uint8Array }; // CLEAR = leave the raster alone

export class PixRaster {
  readonly w: number;
  readonly h: number;
  palette: PixPaletteEntry[];
  readonly layers: Uint8Array[];

  constructor(w: number, h: number, palette: PixPaletteEntry[]) {
    this.w = w;
    this.h = h;
    this.palette = palette;
    this.layers = [];
    for (let i = 0; i < LAYER_COUNT; i++) {
      const l = new Uint8Array(w * h);
      if (i > 0) l.fill(CLEAR);
      else l.fill(this.bgIndex());
      this.layers.push(l);
    }
  }

  /// The paper: entry 0 by convention, the base layer's fill and the eraser's colour. A palette
  /// whose paper is the theme's bg role puts that role at 0; the default palette keeps a literal
  /// cream there so a drawing reads as a drawing under every theme.
  bgIndex(): number {
    return 0;
  }

  inside(x: number, y: number): boolean {
    return x >= 0 && y >= 0 && x < this.w && y < this.h;
  }

  /// Flatten: base layer first, then each upper layer where it is not CLEAR.
  composite(out: Uint8Array = new Uint8Array(this.w * this.h)): Uint8Array {
    out.set(this.layers[0]);
    for (let l = 1; l < this.layers.length; l++) {
      const src = this.layers[l];
      for (let i = 0; i < src.length; i++) if (src[i] !== CLEAR) out[i] = src[i];
    }
    return out;
  }

  toPixImage(): PixImage {
    return { w: this.w, h: this.h, palette: this.palette.map((e) => ({ ...e })), pixels: this.composite() };
  }

  /// The canonical bytes of the flattened frame, as the frame record would carry them.
  encode(): Uint8Array {
    return encodePix(this.toPixImage());
  }

  /// Replace every layer from a flat raster: base takes the pixels, upper layers clear. Used
  /// when opening a frame whose bytes came from elsewhere (no layer split survives the wire).
  loadFlat(pixels: Uint8Array): void {
    if (pixels.length !== this.w * this.h) throw new Error("raster size");
    this.layers[0].set(pixels);
    for (let l = 1; l < this.layers.length; l++) this.layers[l].fill(CLEAR);
  }

  snapshot(): Uint8Array[] {
    return this.layers.map((l) => l.slice());
  }

  restore(snap: Uint8Array[]): void {
    snap.forEach((l, i) => this.layers[i].set(l));
  }
}

// --- Brush geometry ---------------------------------------------------------------------------

export function clampBrush(size: number): number {
  return Math.max(BRUSH_MIN, Math.min(BRUSH_MAX, Math.round(size) || BRUSH_MIN));
}

/// The cells a square brush of `size` covers when its hot cell is (x, y). Odd sizes centre on
/// the cursor; even sizes hang one cell down-right, so growing the brush never shifts the mark.
export function brushCells(x: number, y: number, size: number): Cell[] {
  const s = clampBrush(size);
  const off = Math.floor((s - 1) / 2);
  const out: Cell[] = [];
  for (let dy = 0; dy < s; dy++) for (let dx = 0; dx < s; dx++) out.push([x - off + dx, y - off + dy]);
  return out;
}

/// Pressure floor: a pen reports near zero before it settles, and a stroke that starts as one
/// cell then fattens reads as a mistake, so the lightest touch still paints a quarter brush.
export const PRESSURE_FLOOR = 0.25;

/// The brush size one pointer sample paints at. Only a pen carries pressure (a mouse reports a
/// constant 0.5 that means nothing), so anything else, and pressure switched off, paints the
/// chosen size. The chosen size is the maximum: full pressure paints it, lighter paints less.
export function pressureSize(base: number, pressure: number, pointerType: string, enabled: boolean): number {
  const b = clampBrush(base);
  if (!enabled || pointerType !== "pen") return b;
  const p = Number.isFinite(pressure) ? Math.max(PRESSURE_FLOOR, Math.min(1, pressure)) : 1;
  return clampBrush(Math.max(1, Math.round(b * p)));
}

/// Mirror a set of cells across the raster's axes. Duplicates are fine: painting is idempotent.
export function mirrored(cells: readonly Cell[], w: number, h: number, m: Mirror): Cell[] {
  if (!m.h && !m.v) return cells.slice();
  const out: Cell[] = [];
  for (const [x, y] of cells) {
    out.push([x, y]);
    if (m.h) out.push([w - 1 - x, y]);
    if (m.v) out.push([x, h - 1 - y]);
    if (m.h && m.v) out.push([w - 1 - x, h - 1 - y]);
  }
  return out;
}

/// Bresenham's line, inclusive of both ends, one cell per step.
export function lineCells(x0: number, y0: number, x1: number, y1: number): Cell[] {
  const out: Cell[] = [];
  let dx = Math.abs(x1 - x0);
  let dy = -Math.abs(y1 - y0);
  const sx = x0 < x1 ? 1 : -1;
  const sy = y0 < y1 ? 1 : -1;
  let err = dx + dy;
  let x = x0;
  let y = y0;
  for (;;) {
    out.push([x, y]);
    if (x === x1 && y === y1) break;
    const e2 = 2 * err;
    if (e2 >= dy) { err += dy; x += sx; }
    if (e2 <= dx) { err += dx; y += sy; }
  }
  return out;
}

/// The outline of a shape spanning the box (x0,y0)..(x1,y1) inclusive, as cells.
export function shapeOutline(shape: Shape, x0: number, y0: number, x1: number, y1: number): Cell[] {
  if (shape === "line") return lineCells(x0, y0, x1, y1);
  const l = Math.min(x0, x1), r = Math.max(x0, x1), t = Math.min(y0, y1), b = Math.max(y0, y1);
  if (shape === "rect") {
    const out: Cell[] = [];
    for (let x = l; x <= r; x++) { out.push([x, t]); if (b !== t) out.push([x, b]); }
    for (let y = t + 1; y < b; y++) { out.push([l, y]); if (r !== l) out.push([r, y]); }
    return out;
  }
  if (shape === "triangle") {
    // Apex at the top centre, base along the bottom edge.
    const ax = Math.floor((l + r) / 2);
    return dedupe([...lineCells(ax, t, l, b), ...lineCells(ax, t, r, b), ...lineCells(l, b, r, b)]);
  }
  // Ellipse: midpoint algorithm on the box's half-axes, in doubled coordinates so a box with an
  // even side still lands on whole cells. Degenerate boxes fall back to a line.
  const W = r - l, H = b - t;
  if (W === 0 || H === 0) return lineCells(l, t, r, b);
  const out: Cell[] = [];
  const cx2 = l + r, cy2 = t + b; // centre times two
  const a = W, bb = H; // diameters
  const plot = (px: number, py: number) => out.push([px, py]);
  // Standard "ellipse in a rectangle" rasterizer (Alois Zingl's variant), exact in integers.
  let xx0 = l, yy0 = t, xx1 = r, yy1 = b;
  let bq = bb & 1;
  let dxq = 4 * (1 - a) * bb * bb, dyq = 4 * (bq + 1) * a * a;
  let err = dxq + dyq + bq * a * a;
  yy0 += (bb + 1) >> 1;
  yy1 = yy0 - bq;
  const a8 = 8 * a * a, b8 = 8 * bb * bb;
  do {
    plot(xx1, yy0); plot(xx0, yy0); plot(xx0, yy1); plot(xx1, yy1);
    const e2 = 2 * err;
    if (e2 <= dyq) { yy0++; yy1--; dyq += a8; err += dyq; }
    if (e2 >= dxq || 2 * err > dyq) { xx0++; xx1--; dxq += b8; err += dxq; }
  } while (xx0 <= xx1);
  while (yy0 - yy1 < bb) {
    plot(xx0 - 1, yy0); plot(xx1 + 1, yy0++);
    plot(xx0 - 1, yy1); plot(xx1 + 1, yy1--);
  }
  void cx2; void cy2;
  return dedupe(out);
}

/// Fill a convex outline: every cell between the leftmost and rightmost outline cell per row.
export function shapeFilled(shape: Shape, x0: number, y0: number, x1: number, y1: number): Cell[] {
  const outline = shapeOutline(shape, x0, y0, x1, y1);
  if (shape === "line") return outline;
  const rows = new Map<number, [number, number]>();
  for (const [x, y] of outline) {
    const r = rows.get(y);
    if (!r) rows.set(y, [x, x]);
    else { if (x < r[0]) r[0] = x; if (x > r[1]) r[1] = x; }
  }
  const out: Cell[] = [];
  for (const [y, [a, b]] of rows) for (let x = a; x <= b; x++) out.push([x, y]);
  return out;
}

function dedupe(cells: Cell[]): Cell[] {
  const seen = new Set<number>();
  const out: Cell[] = [];
  for (const c of cells) {
    const k = c[1] * 65536 + c[0] + 32768 * 65536;
    if (!seen.has(k)) { seen.add(k); out.push(c); }
  }
  return out;
}

// --- Painting ---------------------------------------------------------------------------------

/// Write `value` into the layer at every in-bounds cell. Returns how many cells changed, which
/// is what the editor coalesces edits on (an op is produced only when a value changed, 2.9).
export function paintCells(r: PixRaster, layer: number, cells: readonly Cell[], value: number): number {
  const l = r.layers[layer];
  let changed = 0;
  for (const [x, y] of cells) {
    if (!r.inside(x, y)) continue;
    const i = y * r.w + x;
    if (l[i] !== value) { l[i] = value; changed++; }
  }
  return changed;
}

export function paintDot(r: PixRaster, layer: number, x: number, y: number, value: number, size: number, m: Mirror): number {
  return paintCells(r, layer, mirrored(brushCells(x, y, size), r.w, r.h, m), value);
}

export function paintStroke(r: PixRaster, layer: number, x0: number, y0: number, x1: number, y1: number, value: number, size: number, m: Mirror): number {
  const cells: Cell[] = [];
  for (const [x, y] of lineCells(x0, y0, x1, y1)) cells.push(...brushCells(x, y, size));
  return paintCells(r, layer, mirrored(cells, r.w, r.h, m), value);
}

export function paintShape(r: PixRaster, layer: number, shape: Shape, x0: number, y0: number, x1: number, y1: number, value: number, size: number, filled: boolean, m: Mirror): number {
  const base = filled ? shapeFilled(shape, x0, y0, x1, y1) : shapeOutline(shape, x0, y0, x1, y1);
  const cells: Cell[] = [];
  if (filled || size <= 1) cells.push(...base);
  else for (const [x, y] of base) cells.push(...brushCells(x, y, size));
  return paintCells(r, layer, mirrored(cells, r.w, r.h, m), value);
}

/// The eraser paints "nothing": CLEAR on an upper layer, the bg role on the base layer.
export function eraseValue(r: PixRaster, layer: number): number {
  return layer === 0 ? r.bgIndex() : CLEAR;
}

/// Four-connected flood fill on one layer, by that layer's own values (CLEAR is a value too).
export function floodFill(r: PixRaster, layer: number, x: number, y: number, value: number): number {
  if (!r.inside(x, y)) return 0;
  const l = r.layers[layer];
  const from = l[y * r.w + x];
  if (from === value) return 0;
  const stack = [y * r.w + x];
  let changed = 0;
  while (stack.length) {
    const i = stack.pop()!;
    if (l[i] !== from) continue;
    l[i] = value;
    changed++;
    const cx = i % r.w;
    if (cx > 0) stack.push(i - 1);
    if (cx < r.w - 1) stack.push(i + 1);
    if (i >= r.w) stack.push(i - r.w);
    if (i + r.w < l.length) stack.push(i + r.w);
  }
  return changed;
}

/// Place a bitmap with its top-left at (x, y); CLEAR cells leave the raster alone. The bitmap's
/// non-clear values are palette indices already remapped by the caller.
export function stampBitmap(r: PixRaster, layer: number, x: number, y: number, bm: Bitmap, m: Mirror): number {
  const cells: Cell[] = [];
  const values: number[] = [];
  for (let by = 0; by < bm.h; by++) for (let bx = 0; bx < bm.w; bx++) {
    const v = bm.pixels[by * bm.w + bx];
    if (v === CLEAR) continue;
    cells.push([x + bx, y + by]);
    values.push(v);
  }
  let changed = 0;
  cells.forEach((c, i) => { changed += paintCells(r, layer, mirrored([c], r.w, r.h, m), values[i]); });
  return changed;
}

// --- Text stamp: a 5x7 bitmap face --------------------------------------------------------------

const GLYPH_W = 5, GLYPH_H = 7;
const FONT: Record<string, string> = {
  A: ".###.,#...#,#...#,#####,#...#,#...#,#...#",
  B: "####.,#...#,#...#,####.,#...#,#...#,####.",
  C: ".###.,#...#,#....,#....,#....,#...#,.###.",
  D: "####.,#...#,#...#,#...#,#...#,#...#,####.",
  E: "#####,#....,#....,####.,#....,#....,#####",
  F: "#####,#....,#....,####.,#....,#....,#....",
  G: ".###.,#...#,#....,#.###,#...#,#...#,.####",
  H: "#...#,#...#,#...#,#####,#...#,#...#,#...#",
  I: ".###.,..#..,..#..,..#..,..#..,..#..,.###.",
  J: "..###,...#.,...#.,...#.,...#.,#..#.,.##..",
  K: "#...#,#..#.,#.#..,##...,#.#..,#..#.,#...#",
  L: "#....,#....,#....,#....,#....,#....,#####",
  M: "#...#,##.##,#.#.#,#.#.#,#...#,#...#,#...#",
  N: "#...#,#...#,##..#,#.#.#,#..##,#...#,#...#",
  O: ".###.,#...#,#...#,#...#,#...#,#...#,.###.",
  P: "####.,#...#,#...#,####.,#....,#....,#....",
  Q: ".###.,#...#,#...#,#...#,#.#.#,#..#.,.##.#",
  R: "####.,#...#,#...#,####.,#.#..,#..#.,#...#",
  S: ".####,#....,#....,.###.,....#,....#,####.",
  T: "#####,..#..,..#..,..#..,..#..,..#..,..#..",
  U: "#...#,#...#,#...#,#...#,#...#,#...#,.###.",
  V: "#...#,#...#,#...#,#...#,#...#,.#.#.,..#..",
  W: "#...#,#...#,#...#,#.#.#,#.#.#,##.##,#...#",
  X: "#...#,#...#,.#.#.,..#..,.#.#.,#...#,#...#",
  Y: "#...#,#...#,.#.#.,..#..,..#..,..#..,..#..",
  Z: "#####,....#,...#.,..#..,.#...,#....,#####",
  "0": ".###.,#...#,#..##,#.#.#,##..#,#...#,.###.",
  "1": "..#..,.##..,..#..,..#..,..#..,..#..,.###.",
  "2": ".###.,#...#,....#,...#.,..#..,.#...,#####",
  "3": "#####,...#.,..#..,...#.,....#,#...#,.###.",
  "4": "...#.,..##.,.#.#.,#..#.,#####,...#.,...#.",
  "5": "#####,#....,####.,....#,....#,#...#,.###.",
  "6": "..##.,.#...,#....,####.,#...#,#...#,.###.",
  "7": "#####,....#,...#.,..#..,.#...,.#...,.#...",
  "8": ".###.,#...#,#...#,.###.,#...#,#...#,.###.",
  "9": ".###.,#...#,#...#,.####,....#,...#.,.##..",
  " ": ".....,.....,.....,.....,.....,.....,.....",
  "!": "..#..,..#..,..#..,..#..,..#..,.....,..#..",
  "?": ".###.,#...#,....#,...#.,..#..,.....,..#..",
  ".": ".....,.....,.....,.....,.....,.##..,.##..",
  ",": ".....,.....,.....,.....,.##..,..#..,.#...",
  "'": ".##..,..#..,.#...,.....,.....,.....,.....",
  "-": ".....,.....,.....,#####,.....,.....,.....",
  ":": ".....,.##..,.##..,.....,.##..,.##..,.....",
  "~": ".....,.....,.#...,#.#.#,...#.,.....,.....",
  "+": ".....,..#..,..#..,#####,..#..,..#..,.....",
  "<": "...#.,..#..,.#...,#....,.#...,..#..,...#.",
  ">": ".#...,..#..,...#.,....#,...#.,..#..,.#...",
  "/": "....#,...#.,...#.,..#..,.#...,.#...,#....",
};
const UNKNOWN = "#####,#...#,#...#,#...#,#...#,#...#,#####";

/// Render text into a bitmap: 5x7 glyphs with a one-cell gap, `value` for ink, CLEAR elsewhere.
/// Lowercase folds to uppercase; anything the face lacks draws as a box, so the author sees it.
export function textBitmap(text: string, value: number): Bitmap {
  const chars = Array.from(text.toUpperCase());
  const w = Math.max(0, chars.length * (GLYPH_W + 1) - 1);
  const pixels = new Uint8Array(w * GLYPH_H).fill(CLEAR);
  chars.forEach((ch, ci) => {
    const rows = (FONT[ch] ?? UNKNOWN).split(",");
    rows.forEach((row, y) => {
      for (let x = 0; x < GLYPH_W; x++) if (row[x] === "#") pixels[y * w + ci * (GLYPH_W + 1) + x] = value;
    });
  });
  return { w, h: GLYPH_H, pixels };
}

// --- Undo ---------------------------------------------------------------------------------------

/// Whole-layer snapshots, bounded. A 192x144 frame is 27 KiB per layer, so the cap keeps the
/// stack under 6 MiB at three layers; cheap enough, and exact.
export class UndoStack {
  private past: Uint8Array[][] = [];
  private future: Uint8Array[][] = [];
  private readonly depth: number;

  constructor(depth = UNDO_DEPTH) {
    this.depth = depth;
  }

  /// Call before an edit with the raster as it is; a later `undo` returns to it.
  push(r: PixRaster): void {
    this.past.push(r.snapshot());
    if (this.past.length > this.depth) this.past.shift();
    this.future = [];
  }

  undo(r: PixRaster): boolean {
    const s = this.past.pop();
    if (!s) return false;
    this.future.push(r.snapshot());
    r.restore(s);
    return true;
  }

  redo(r: PixRaster): boolean {
    const s = this.future.pop();
    if (!s) return false;
    this.past.push(r.snapshot());
    r.restore(s);
    return true;
  }

  get canUndo(): boolean { return this.past.length > 0; }
  get canRedo(): boolean { return this.future.length > 0; }
  clear(): void { this.past = []; this.future = []; }
}
