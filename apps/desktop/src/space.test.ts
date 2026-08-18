// Unit tests for the 360 server space math (`space.ts`).
//
// Run with `npm test`. What matters here: projection and unprojection agree
// (a drop lands where the cursor pointed), yaw wraps but never jumps, pitch
// clamps at the poles, and the lasso captures exactly what sits inside it.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  PITCH_MAX,
  angularOffsets,
  applyOffsets,
  clampPitch,
  defaultSpace,
  lassoCapture,
  parseSpace,
  project,
  unproject,
  wrapYaw,
  yawDelta,
} from "./space.ts";

const F = 760;

test("yaw wraps into [0, 360)", () => {
  assert.equal(wrapYaw(0), 0);
  assert.equal(wrapYaw(360), 0);
  assert.equal(wrapYaw(-30), 330);
  assert.equal(wrapYaw(725), 5);
});

test("pitch clamps at the poles", () => {
  assert.equal(clampPitch(90), PITCH_MAX);
  assert.equal(clampPitch(-90), -PITCH_MAX);
  assert.equal(clampPitch(12), 12);
});

test("looking straight at a placement centers it", () => {
  const p = project({ yaw: 214, pitch: -6 }, { yaw: 214, pitch: -6 }, F);
  assert.ok(p.visible);
  assert.ok(Math.abs(p.x) < 1e-9 && Math.abs(p.y) < 1e-9);
  assert.ok(Math.abs(p.scale - 1) < 1e-9);
});

test("a placement to the right of the camera projects right of center", () => {
  const p = project({ yaw: 0, pitch: 0 }, { yaw: 10, pitch: 0 }, F);
  assert.ok(p.visible && p.x > 0 && Math.abs(p.y) < 1e-9);
});

test("a placement above the camera projects up (negative screen y)", () => {
  const p = project({ yaw: 0, pitch: 0 }, { yaw: 0, pitch: 10 }, F);
  assert.ok(p.visible && p.y < 0 && Math.abs(p.x) < 1e-9);
});

test("a placement behind the camera is invisible", () => {
  const p = project({ yaw: 0, pitch: 0 }, { yaw: 180, pitch: 0 }, F);
  assert.ok(!p.visible);
});

test("project and unproject round-trip through the yaw wrap", () => {
  for (const cam of [{ yaw: 0, pitch: 0 }, { yaw: 214, pitch: -6 }, { yaw: 351, pitch: 40 }]) {
    for (const target of [{ yaw: 8, pitch: -20 }, { yaw: 340, pitch: 15 }, { yaw: 214, pitch: 0 }]) {
      const pr = project(cam, target, F);
      if (!pr.visible) continue;
      const back = unproject(cam, pr.x, pr.y, F);
      assert.ok(Math.abs(yawDelta(back.yaw, target.yaw)) < 1e-6, `yaw ${back.yaw} vs ${target.yaw}`);
      assert.ok(Math.abs(back.pitch - target.pitch) < 1e-6, `pitch ${back.pitch} vs ${target.pitch}`);
    }
  }
});

test("unproject clamps pitch even when aiming past the pole", () => {
  const p = unproject({ yaw: 0, pitch: 55 }, 0, -F, F); // aiming ~45 deg above an already-high camera
  assert.ok(p.pitch <= PITCH_MAX);
});

test("yawDelta takes the short way around", () => {
  assert.equal(yawDelta(350, 10), 20);
  assert.equal(yawDelta(10, 350), -20);
  assert.equal(yawDelta(0, 180), 180);
});

test("lasso captures inside, not outside, not behind", () => {
  const placements = {
    1: { yaw: 0, pitch: 0 }, // dead center
    2: { yaw: 4, pitch: 2 }, // near
    3: { yaw: 40, pitch: 0 }, // far right, outside a small lasso
    4: { yaw: 180, pitch: 0 }, // behind the camera
  };
  const caught = lassoCapture(placements, { yaw: 0, pitch: 0 }, 0, 0, 90, F);
  assert.deepEqual(caught.sort(), [1, 2]);
});

test("carrying a group keeps its arrangement across the wrap", () => {
  const placements = { 1: { yaw: 355, pitch: 10 }, 2: { yaw: 5, pitch: -10 } };
  const grab = { yaw: 0, pitch: 0 };
  const offs = angularOffsets([1, 2], placements, grab);
  const dropped = applyOffsets(offs, { yaw: 90, pitch: 20 });
  assert.ok(Math.abs(yawDelta(dropped[1].yaw, 85)) < 1e-9);
  assert.ok(Math.abs(yawDelta(dropped[2].yaw, 95)) < 1e-9);
  assert.equal(dropped[1].pitch, 30);
  assert.equal(dropped[2].pitch, 10);
});

test("applyOffsets clamps pitch on drop", () => {
  const dropped = applyOffsets({ 1: { yaw: 0, pitch: 30 } }, { yaw: 0, pitch: 50 });
  assert.equal(dropped[1].pitch, PITCH_MAX);
});

test("parseSpace survives garbage and hand-edits", () => {
  assert.deepEqual(parseSpace(null), defaultSpace());
  assert.deepEqual(parseSpace("not json"), defaultSpace());
  const s = parseSpace(JSON.stringify({
    backdrop: "void",
    custom: "javascript:alert(1)",
    placements: { 7: { yaw: -30, pitch: 99 }, bad: { yaw: 1, pitch: 1 }, 8: { yaw: "x", pitch: 0 }, 9: null },
  }));
  assert.equal(s.backdrop, "void");
  assert.equal(s.custom, ""); // only data:image/ URLs may come back out of the store
  assert.deepEqual(s.placements, { 7: { yaw: 330, pitch: PITCH_MAX } });
});

test("parseSpace rejects unknown backdrops but keeps placements", () => {
  const s = parseSpace(JSON.stringify({ backdrop: "matrix", placements: { 1: { yaw: 10, pitch: 5 } } }));
  assert.equal(s.backdrop, "den");
  assert.deepEqual(s.placements, { 1: { yaw: 10, pitch: 5 } });
});
