import { test } from "node:test";
import assert from "node:assert/strict";

import {
  MIN_VISIBLE_PX,
  clampToBounds,
  dragTo,
  parsePosition,
  startsDrag,
} from "./overlay-drag.ts";

const panel = { width: 400, height: 300 };
const bounds = { width: 1024, height: 720, insetTop: 30 };

test("a panel dropped inside the window is left where it was put", () => {
  assert.deepEqual(clampToBounds({ x: 200, y: 200 }, panel, bounds), { x: 200, y: 200 });
});

test("a panel can never be dragged somewhere it cannot be dragged back from", () => {
  // Off the left: a graspable strip of the header stays on screen.
  assert.deepEqual(
    clampToBounds({ x: -9999, y: 100 }, panel, bounds),
    { x: MIN_VISIBLE_PX - panel.width, y: 100 },
  );
  // Off the right: likewise.
  assert.deepEqual(
    clampToBounds({ x: 9999, y: 100 }, panel, bounds),
    { x: bounds.width - MIN_VISIBLE_PX, y: 100 },
  );
  // Off the bottom.
  assert.equal(clampToBounds({ x: 0, y: 9999 }, panel, bounds).y, bounds.height - MIN_VISIBLE_PX);
  // And never above the title bar, because the header IS the handle: a panel whose header is
  // under the app's own chrome could never be moved again.
  assert.equal(clampToBounds({ x: 0, y: -9999 }, panel, bounds).y, bounds.insetTop);
  assert.equal(clampToBounds({ x: 0, y: 0 }, panel, bounds).y, bounds.insetTop);
});

test("shrinking the window pulls a stranded panel back into view", () => {
  // The same class of bug as the off-screen collapse button: a panel placed against the right
  // edge of a wide window is entirely outside a narrow one until it is clamped again.
  const placed = { x: 900, y: 600 };
  const narrow = clampToBounds(placed, panel, { width: 720, height: 560, insetTop: 30 });
  assert.equal(narrow.x, 720 - MIN_VISIBLE_PX);
  assert.equal(narrow.y, 560 - MIN_VISIBLE_PX);
});

test("a panel wider than the window still leaves something to grab", () => {
  const wide = { width: 900, height: 200 };
  const tiny = { width: 300, height: 400, insetTop: 30 };
  const at = clampToBounds({ x: -9999, y: 0 }, wide, tiny);
  assert.ok(at.x + wide.width >= MIN_VISIBLE_PX, "a strip is still on screen");
  assert.ok(at.x <= tiny.width - MIN_VISIBLE_PX + 1);
});

test("a drag moves the panel by how far the pointer went, not to the pointer", () => {
  // Grabbing a header near its right edge must not teleport the panel's corner under the cursor.
  const origin = { x: 100, y: 100 };
  const moved = dragTo(origin, { x: 480, y: 120 }, { x: 500, y: 160 }, panel, bounds);
  assert.deepEqual(moved, { x: 120, y: 140 });
});

test("a drag is still bounded while it is happening, not only when it ends", () => {
  const moved = dragTo({ x: 100, y: 100 }, { x: 0, y: 0 }, { x: -9999, y: -9999 }, panel, bounds);
  assert.equal(moved.x, MIN_VISIBLE_PX - panel.width);
  assert.equal(moved.y, bounds.insetTop);
});

test("only a primary press on the header itself begins a drag", () => {
  assert.equal(startsDrag(0, false), true);
  // The header is also a row of buttons; pressing one is that button's press.
  assert.equal(startsDrag(0, true), false);
  assert.equal(startsDrag(2, false), false, "right button opens a menu, it does not drag");
  assert.equal(startsDrag(1, false), false);
});

test("a stored position is only restored when it is a real pair of numbers", () => {
  assert.deepEqual(parsePosition({ x: 10, y: 20 }), { x: 10, y: 20 });
  assert.equal(parsePosition(null), null);
  assert.equal(parsePosition({}), null);
  assert.equal(parsePosition({ x: 10 }), null);
  assert.equal(parsePosition({ x: "10", y: 20 }), null);
  assert.equal(parsePosition({ x: Number.NaN, y: 0 }), null, "NaN would place it nowhere");
  assert.equal(parsePosition({ x: Number.POSITIVE_INFINITY, y: 0 }), null);
});
