// The 360 server space: spherical placement math + per-device persistence.
//
// Servers live as billboards on a sphere around a fixed camera that only rotates
// (yaw around the vertical axis, pitch clamped so the poles stay out of reach).
// The view itself is rendered by App.svelte; everything here is pure math so it
// can be unit-tested: angles in DEGREES at the boundary, radians only inside.
//
// Conventions (must match the CSS cube in App.svelte):
//   yaw 0 = the backdrop's "front" wall, yaw grows to the RIGHT, wrapped to [0, 360)
//   pitch grows UP, clamped to [-PITCH_MAX, PITCH_MAX]
//   focal length f is in px: a point d radians off-center lands ~ f*tan(d) px away

export const PITCH_MAX = 60;

export type Placement = { yaw: number; pitch: number };
export type SpaceState = {
  backdrop: string; // "den" | "ridge" | "void" | "custom"
  custom: string; // data: URL of a user equirect panorama ("" = none)
  placements: Record<number, Placement>;
};

const DEG = Math.PI / 180;

export function clampPitch(p: number): number {
  return Math.max(-PITCH_MAX, Math.min(PITCH_MAX, p));
}

export function wrapYaw(y: number): number {
  const w = y % 360;
  return w < 0 ? w + 360 : w;
}

// Unit direction vector for a (yaw, pitch) pair: x right, y up, z forward (yaw 0).
function dir(yaw: number, pitch: number): [number, number, number] {
  const cy = Math.cos(yaw * DEG), sy = Math.sin(yaw * DEG);
  const cp = Math.cos(pitch * DEG), sp = Math.sin(pitch * DEG);
  return [cp * sy, sp, cp * cy];
}

export type Projected = { x: number; y: number; scale: number; visible: boolean };

// Where a placement lands on screen, in px offsets from the viewport center.
// `visible` is false for points behind the camera plane (or grazing it); scale
// shrinks gently off-center so far icons read as far without real depth.
export function project(cam: Placement, p: Placement, f: number): Projected {
  const [x, y, z] = dir(p.yaw, p.pitch);
  // world -> camera: undo yaw around Y, then undo pitch around X
  const cy = Math.cos(-cam.yaw * DEG), sy = Math.sin(-cam.yaw * DEG);
  const x1 = cy * x + sy * z;
  const z1 = -sy * x + cy * z;
  const cp = Math.cos(cam.pitch * DEG), sp = Math.sin(cam.pitch * DEG);
  const y2 = cp * y - sp * z1;
  const z2 = sp * y + cp * z1;
  if (z2 < 0.15) return { x: 0, y: 0, scale: 0, visible: false };
  // subtle edge falloff (z2 is the cosine of the off-center angle): 1 at center, ~0.87 at 60 deg
  return { x: (x1 / z2) * f, y: (-y2 / z2) * f, scale: 0.75 + 0.25 * z2, visible: true };
}

// The (yaw, pitch) a screen point (px offsets from center) is aiming at.
export function unproject(cam: Placement, sx: number, sy: number, f: number): Placement {
  // ray in camera space
  let x = sx / f, y = -sy / f, z = 1;
  const n = Math.hypot(x, y, z);
  x /= n; y /= n; z /= n;
  // camera -> world: pitch around X, then yaw around Y (the exact inverse of project)
  const cp = Math.cos(cam.pitch * DEG), sp = Math.sin(cam.pitch * DEG);
  const y1 = cp * y + sp * z;
  const z1 = -sp * y + cp * z;
  const cy = Math.cos(cam.yaw * DEG), sy2 = Math.sin(cam.yaw * DEG);
  const x2 = cy * x + sy2 * z1;
  const z2 = -sy2 * x + cy * z1;
  return { yaw: wrapYaw(Math.atan2(x2, z2) / DEG), pitch: clampPitch(Math.asin(Math.max(-1, Math.min(1, y1))) / DEG) };
}

// Shortest signed yaw distance a -> b in degrees, in (-180, 180].
export function yawDelta(a: number, b: number): number {
  let d = wrapYaw(b) - wrapYaw(a);
  if (d > 180) d -= 360;
  if (d <= -180) d += 360;
  return d;
}

// Which placed servers a lasso (screen center + radius, px) captures.
export function lassoCapture(
  placements: Record<number, Placement>,
  cam: Placement,
  cx: number,
  cy: number,
  r: number,
  f: number,
): number[] {
  const out: number[] = [];
  for (const [id, p] of Object.entries(placements)) {
    const pr = project(cam, p, f);
    if (pr.visible && Math.hypot(pr.x - cx, pr.y - cy) <= r) out.push(Number(id));
  }
  return out;
}

// Carrying a group keeps its internal arrangement: each captured server is stored
// as an angular offset from the grab point, re-applied around the drop point.
export function angularOffsets(
  ids: number[],
  placements: Record<number, Placement>,
  center: Placement,
): Record<number, Placement> {
  const out: Record<number, Placement> = {};
  for (const id of ids) {
    const p = placements[id];
    if (p) out[id] = { yaw: yawDelta(center.yaw, p.yaw), pitch: p.pitch - center.pitch };
  }
  return out;
}

export function applyOffsets(
  offsets: Record<number, Placement>,
  center: Placement,
): Record<number, Placement> {
  const out: Record<number, Placement> = {};
  for (const [id, o] of Object.entries(offsets)) {
    out[Number(id)] = { yaw: wrapYaw(center.yaw + o.yaw), pitch: clampPitch(center.pitch + o.pitch) };
  }
  return out;
}

// -------- persistence (per-device, like desktop icon positions) --------

export const SPACE_BACKDROPS = ["den", "ridge", "void", "custom"] as const;

export function defaultSpace(): SpaceState {
  return { backdrop: "den", custom: "", placements: {} };
}

// Parse a stored blob defensively: unknown fields drop, bad placements drop,
// angles re-wrapped/re-clamped so a hand-edited file can't wedge the view.
export function parseSpace(raw: string | null): SpaceState {
  const out = defaultSpace();
  if (!raw) return out;
  try {
    const j = JSON.parse(raw);
    if (typeof j.backdrop === "string" && (SPACE_BACKDROPS as readonly string[]).includes(j.backdrop)) out.backdrop = j.backdrop;
    if (typeof j.custom === "string" && j.custom.startsWith("data:image/")) out.custom = j.custom;
    if (j.placements && typeof j.placements === "object") {
      for (const [k, v] of Object.entries(j.placements as Record<string, unknown>)) {
        const id = Number(k);
        const p = v as Placement;
        if (Number.isInteger(id) && p && typeof p.yaw === "number" && typeof p.pitch === "number" && Number.isFinite(p.yaw) && Number.isFinite(p.pitch)) {
          out.placements[id] = { yaw: wrapYaw(p.yaw), pitch: clampPitch(p.pitch) };
        }
      }
    }
  } catch {
    /* corrupt store reads as factory-fresh */
  }
  return out;
}
