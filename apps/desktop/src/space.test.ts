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
  autoArrangePlacements,
  clampPitch,
  defaultSpace,
  lassoCapture,
  lassoCapturePath,
  parseSpace,
  placementCentre,
  project,
  separatePlacements,
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

test("a freehand lasso captures only projected servers inside its closed path", () => {
  const placements = {
    1: { yaw: 0, pitch: 0 },
    2: { yaw: 4, pitch: 2 },
    3: { yaw: 30, pitch: 0 },
    4: { yaw: 180, pitch: 0 },
  };
  const path = [
    { x: -70, y: -70 },
    { x: 100, y: -65 },
    { x: 105, y: 80 },
    { x: -80, y: 75 },
  ];
  assert.deepEqual(lassoCapturePath(placements, { yaw: 0, pitch: 0 }, path, F).sort(), [1, 2]);
});

test("a held click and a line are not treated as freehand lassos", () => {
  const placements = { 1: { yaw: 0, pitch: 0 } };
  assert.deepEqual(lassoCapturePath(placements, { yaw: 0, pitch: 0 }, [{ x: 0, y: 0 }], F), []);
  assert.deepEqual(lassoCapturePath(placements, { yaw: 0, pitch: 0 }, [
    { x: -20, y: 0 }, { x: 0, y: 0 }, { x: 20, y: 0 },
  ], F), []);
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

test("placementCentre averages across the yaw seam", () => {
  const centre = placementCentre({
    1: { yaw: 358, pitch: -4 },
    2: { yaw: 2, pitch: 4 },
  }, [1, 2], { yaw: 90, pitch: 20 });
  assert.ok(Math.abs(yawDelta(centre.yaw, 0)) < 1e-6);
  assert.ok(Math.abs(centre.pitch) < 1e-6);
});

test("placementCentre uses its fallback for an empty or balanced set", () => {
  const fallback = { yaw: -20, pitch: 99 };
  assert.deepEqual(placementCentre({}, [7], fallback), { yaw: 340, pitch: PITCH_MAX });
  assert.deepEqual(placementCentre({
    1: { yaw: 0, pitch: 0 },
    2: { yaw: 180, pitch: 0 },
  }, [1, 2], fallback), { yaw: 340, pitch: PITCH_MAX });
});

test("collision separation keeps fixed servers still and nudges moved servers apart", () => {
  const placements = {
    1: { yaw: 0, pitch: 0 },
    2: { yaw: 0, pitch: 0 },
    3: { yaw: 0, pitch: 0 },
  };
  const out = separatePlacements(placements, [2, 3], 6);
  assert.deepEqual(out[1], placements[1]);
  const a = project(out[1], out[2], F);
  const b = project(out[1], out[3], F);
  assert.ok(Math.hypot(a.x, a.y) > 70);
  assert.ok(Math.hypot(b.x, b.y) > 70);
  assert.ok(Math.hypot(a.x - b.x, a.y - b.y) > 70);
});

test("collision separation handles the yaw seam", () => {
  const out = separatePlacements({
    1: { yaw: 359, pitch: 0 },
    2: { yaw: 1, pitch: 0 },
  }, [2], 8);
  assert.notDeepEqual(out[2], { yaw: 1, pitch: 0 });
});

test("auto arrange is deterministic and keeps neighbourhoods together", () => {
  const groups = { 1: "friends", 2: "friends", 3: "work", 4: "work" };
  const a = autoArrangePlacements([4, 2, 1, 3], groups, 6);
  const b = autoArrangePlacements([1, 2, 3, 4], groups, 6);
  assert.deepEqual(a, b);
  assert.ok(Math.abs(yawDelta(a[1].yaw, a[2].yaw)) < 30);
  assert.ok(Math.abs(yawDelta(a[3].yaw, a[4].yaw)) < 30);
  assert.ok(Math.abs(yawDelta(a[1].yaw, a[3].yaw)) > 120);
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

test("parseSpace validates new interaction preferences and migrates old stores", () => {
  const old = parseSpace(JSON.stringify({ backdrop: "ridge", placements: {} }));
  assert.equal(old.shape, "square");
  assert.equal(old.serverSize, 46);
  assert.equal(old.zoomOnOpen, true);
  assert.equal(old.entrySound, true);
  assert.equal(old.showMinimap, true);
  assert.equal(old.glow, 84);

  const parsed = parseSpace(JSON.stringify({
    shape: "circle",
    serverSize: 999,
    zoomOnOpen: false,
    entrySound: false,
    placements: {},
  }));
  assert.equal(parsed.shape, "circle");
  assert.equal(parsed.serverSize, 88);
  assert.equal(parsed.zoomOnOpen, false);
  assert.equal(parsed.entrySound, false);
});

test("parseSpace validates effects, clusters, and the fourth backdrop", () => {
  const parsed = parseSpace(JSON.stringify({
    backdrop: "garden",
    ambience: 999,
    links: -4,
    glow: 61.2,
    hoverShake: false,
    backdropBlur: 30,
    clusters: [
      { id: "friends", name: " Friends ", color: "#12abEF" },
      { id: "friends", name: "duplicate", color: "#ffffff" },
      { id: "bad id!", name: "Work", color: "not-a-colour" },
    ],
    serverClusters: { 1: "friends", 2: "missing", 3: "badid" },
  }));
  assert.equal(parsed.backdrop, "garden");
  assert.equal(parsed.ambience, 100);
  assert.equal(parsed.links, 0);
  assert.equal(parsed.glow, 61);
  assert.equal(parsed.hoverShake, false);
  assert.equal(parsed.backdropBlur, 12);
  assert.deepEqual(parsed.clusters, [
    { id: "friends", name: "Friends", color: "#12abEF" },
    { id: "badid", name: "Work", color: "#8d7cf5" },
  ]);
  assert.deepEqual(parsed.serverClusters, { 1: "friends", 3: "badid" });
});
