import assert from "node:assert/strict";
import test from "node:test";
import { decodePix } from "./pix.ts";
import {
  CLEAR,
  PixRaster,
  UndoStack,
  brushCells,
  eraseValue,
  floodFill,
  lineCells,
  mirrored,
  paintDot,
  paintShape,
  paintStroke,
  pressureSize,
  shapeFilled,
  shapeOutline,
  stampBitmap,
  textBitmap,
} from "./pix-canvas.ts";
import type { PixPaletteEntry } from "./studio-contract.ts";

const pal: PixPaletteEntry[] = [
  { role: 1, r: 0x13, g: 0x12, b: 0x18 },
  { role: 2, r: 0xe8, g: 0xe6, b: 0xf0 },
  { role: 3, r: 0x97, g: 0x7d, b: 0xf2 },
  { role: 0, r: 0xe0, g: 0x7a, b: 0xb8 },
];
const NO_MIRROR = { h: false, v: false };

test("a fresh raster is the bg role on the base layer and clear above, and composites flat", () => {
  const r = new PixRaster(8, 4, pal);
  assert.equal(r.bgIndex(), 0);
  assert.ok(r.layers[0].every((v) => v === 0));
  assert.ok(r.layers[1].every((v) => v === CLEAR));
  r.layers[2][5] = 3;
  r.layers[1][5] = 2; // fg layer wins over mid
  r.layers[1][6] = 2;
  const flat = r.composite();
  assert.equal(flat[5], 3);
  assert.equal(flat[6], 2);
  assert.equal(flat[0], 0);
  // and the flattened frame is a valid pix:v1
  assert.deepEqual(Array.from(decodePix(r.encode()).pixels), Array.from(flat));
});

test("brush cells centre odd sizes and hang even sizes down-right, and the stroke is a line", () => {
  assert.deepEqual(brushCells(5, 5, 1), [[5, 5]]);
  assert.deepEqual(brushCells(5, 5, 3).length, 9);
  assert.deepEqual(brushCells(5, 5, 3)[0], [4, 4]);
  assert.deepEqual(brushCells(5, 5, 2)[0], [5, 5]);
  assert.deepEqual(brushCells(5, 5, 2)[3], [6, 6]);
  assert.deepEqual(lineCells(0, 0, 3, 1), [[0, 0], [1, 0], [2, 1], [3, 1]]);
  const r = new PixRaster(8, 8, pal);
  assert.equal(paintStroke(r, 0, 0, 0, 7, 7, 1, 1, NO_MIRROR), 8);
  assert.equal(r.layers[0][7 * 8 + 7], 1);
});

test("painting counts only cells that changed, so a no-op stroke produces no operation", () => {
  const r = new PixRaster(8, 8, pal);
  assert.equal(paintDot(r, 2, 2, 2, 1, 1, NO_MIRROR), 1);
  assert.equal(paintDot(r, 2, 2, 2, 1, 1, NO_MIRROR), 0);
  // out of bounds is silently clipped rather than thrown: a brush overhanging the edge is normal
  assert.equal(paintDot(r, 2, -5, -5, 1, 1, NO_MIRROR), 0);
});

test("mirror doubles the mark across the axes that are on", () => {
  assert.deepEqual(mirrored([[1, 2]], 8, 8, { h: true, v: false }), [[1, 2], [6, 2]]);
  assert.deepEqual(mirrored([[1, 2]], 8, 8, { h: true, v: true }), [[1, 2], [6, 2], [1, 5], [6, 5]]);
  const r = new PixRaster(8, 8, pal);
  paintDot(r, 2, 0, 0, 1, 1, { h: true, v: true });
  assert.equal(r.layers[2][0], 1);
  assert.equal(r.layers[2][7], 1);
  assert.equal(r.layers[2][7 * 8], 1);
  assert.equal(r.layers[2][7 * 8 + 7], 1);
});

test("pressure scales the brush for a pen only, floors at a quarter, and can be switched off", () => {
  assert.equal(pressureSize(8, 1, "pen", true), 8);
  assert.equal(pressureSize(8, 0.5, "pen", true), 4);
  assert.equal(pressureSize(8, 0.1, "pen", true), 2, "a quarter of 8, not 1");
  assert.equal(pressureSize(8, 0, "pen", true), 2);
  assert.equal(pressureSize(1, 0, "pen", true), 1, "never below one cell");
  assert.equal(pressureSize(8, 0.5, "mouse", true), 8, "a mouse's constant 0.5 is not pressure");
  assert.equal(pressureSize(8, 0.5, "touch", true), 8);
  assert.equal(pressureSize(8, 0.1, "pen", false), 8, "switched off paints the chosen size");
  assert.equal(pressureSize(8, Number.NaN, "pen", true), 8, "a missing reading paints full");
});

test("the eraser is clear on an upper layer and the bg role on the base layer", () => {
  const r = new PixRaster(4, 4, pal);
  assert.equal(eraseValue(r, 0), 0);
  assert.equal(eraseValue(r, 1), CLEAR);
});

test("flood fill is four-connected and stops at a diagonal gap", () => {
  const r = new PixRaster(5, 5, pal);
  // A diagonal wall of index 1 from top-right to bottom-left; the two sides must not connect.
  for (let i = 0; i < 5; i++) r.layers[0][i * 5 + (4 - i)] = 1;
  const changed = floodFill(r, 0, 0, 0, 2);
  assert.equal(changed, 10); // the triangle above the diagonal
  assert.equal(r.layers[0][4 * 5 + 4], 0); // bottom-right untouched
  assert.equal(floodFill(r, 0, 0, 0, 2), 0); // already that value
});

test("shapes: rect outline and fill, ellipse symmetry, triangle apex, line passthrough", () => {
  assert.equal(shapeOutline("rect", 2, 2, 5, 4).length, 10); // 4 wide x 3 tall perimeter
  assert.equal(shapeFilled("rect", 2, 2, 5, 4).length, 12);
  const el = shapeOutline("ellipse", 0, 0, 10, 6);
  assert.ok(el.length > 0);
  for (const [x, y] of el) {
    assert.ok(x >= 0 && x <= 10 && y >= 0 && y <= 6, `ellipse cell inside box: ${x},${y}`);
    assert.ok(el.some(([mx, my]) => mx === 10 - x && my === y), "horizontally symmetric");
    assert.ok(el.some(([mx, my]) => mx === x && my === 6 - y), "vertically symmetric");
  }
  assert.ok(shapeFilled("ellipse", 0, 0, 10, 6).length > el.length);
  const tri = shapeOutline("triangle", 0, 0, 8, 6);
  assert.ok(tri.some(([x, y]) => x === 4 && y === 0), "apex at the top centre");
  assert.equal(tri.filter(([, y]) => y === 6).length, 9, "full base row");
  assert.deepEqual(shapeOutline("line", 0, 0, 2, 0), [[0, 0], [1, 0], [2, 0]]);
  const r = new PixRaster(12, 12, pal);
  assert.equal(paintShape(r, 0, "rect", 1, 1, 4, 4, 1, 1, true, NO_MIRROR), 16);
});

test("a stamp leaves the raster alone where it is clear", () => {
  const r = new PixRaster(6, 6, pal);
  r.layers[1][0] = 3;
  const bm = { w: 2, h: 2, pixels: new Uint8Array([CLEAR, 1, 1, CLEAR]) };
  assert.equal(stampBitmap(r, 1, 0, 0, bm, NO_MIRROR), 2);
  assert.equal(r.layers[1][0], 3); // untouched under the clear cell
  assert.equal(r.layers[1][1], 1);
  assert.equal(r.layers[1][6], 1);
});

test("the text face is 5x7 with one-cell gaps, folds case, and boxes what it lacks", () => {
  const a = textBitmap("a", 1);
  assert.equal(a.w, 5);
  assert.equal(a.h, 7);
  assert.equal(a.pixels[0], CLEAR); // A's top-left corner is empty
  assert.equal(a.pixels[1], 1); // the apex row is .###.
  const two = textBitmap("ab", 1);
  assert.equal(two.w, 11);
  assert.ok([0, 1, 2, 3, 4, 5, 6].every((y) => two.pixels[y * 11 + 5] === CLEAR), "gap column is clear");
  const box = textBitmap("é", 1);
  assert.ok([0, 1, 2, 3, 4].every((x) => box.pixels[x] === 1), "unknown glyph draws a box");
  assert.equal(textBitmap("", 1).w, 0);
});

test("undo returns to the state before the edit, redo goes forward, and the depth is bounded", () => {
  const r = new PixRaster(4, 4, pal);
  const u = new UndoStack(3);
  for (let i = 0; i < 5; i++) {
    u.push(r);
    paintDot(r, 0, i % 4, 0, 1, 1, NO_MIRROR);
  }
  assert.ok(u.canUndo);
  assert.ok(u.undo(r));
  assert.ok(u.undo(r));
  assert.ok(u.undo(r));
  assert.equal(u.undo(r), false, "only three kept");
  assert.ok(u.canRedo);
  assert.ok(u.redo(r));
  assert.equal(r.layers[0][2], 1);
  u.push(r); // a new edit clears redo
  assert.equal(u.canRedo, false);
});

test("loading a flat frame puts the pixels on the base layer and clears the rest", () => {
  const r = new PixRaster(3, 1, pal);
  r.layers[2][0] = 2;
  r.loadFlat(new Uint8Array([1, 1, 3]));
  assert.deepEqual(Array.from(r.layers[0]), [1, 1, 3]);
  assert.equal(r.layers[2][0], CLEAR);
  assert.throws(() => r.loadFlat(new Uint8Array(2)));
});
