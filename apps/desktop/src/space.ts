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
export type ScreenPoint = { x: number; y: number };
export type SpaceCluster = { id: string; name: string; color: string };
export type SpaceState = {
  backdrop: string; // "den" | "ridge" | "void" | "garden" | "custom"
  custom: string; // data: URL of a user equirect panorama ("" = none)
  shape: "circle" | "square"; // local viewport aperture
  serverSize: number; // icon diameter in CSS px, before perspective scaling
  zoomOnOpen: boolean;
  entrySound: boolean;
  showMinimap: boolean;
  ambience: number; // 0..100, particles + atmospheric drift
  links: number; // 0..100, constellation visibility
  glow: number; // 0..100, idle ring + halo intensity
  hoverShake: boolean;
  backdropBlur: number; // 0..12 CSS px
  clusters: SpaceCluster[];
  serverClusters: Record<number, string>;
  placements: Record<number, Placement>;
};

export const SERVER_SIZE_MIN = 32;
export const SERVER_SIZE_MAX = 88;
export const SERVER_SIZE_DEFAULT = 46;

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

function pointOnSegment(p: ScreenPoint, a: ScreenPoint, b: ScreenPoint): boolean {
  const cross = (p.y - a.y) * (b.x - a.x) - (p.x - a.x) * (b.y - a.y);
  if (Math.abs(cross) > 0.001) return false;
  const dot = (p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y);
  if (dot < 0) return false;
  const len2 = (b.x - a.x) ** 2 + (b.y - a.y) ** 2;
  return dot <= len2;
}

// Even/odd polygon fill with an explicit edge check: an icon whose centre lands
// exactly on the user's stroke counts as caught rather than flickering in/out.
function pointInPolygon(p: ScreenPoint, path: ScreenPoint[]): boolean {
  let inside = false;
  for (let i = 0, j = path.length - 1; i < path.length; j = i++) {
    const a = path[j], b = path[i];
    if (pointOnSegment(p, a, b)) return true;
    if ((a.y > p.y) !== (b.y > p.y)) {
      const crossX = ((b.x - a.x) * (p.y - a.y)) / (b.y - a.y) + a.x;
      if (p.x < crossX) inside = !inside;
    }
  }
  return inside;
}

// Which placed servers have their projected centre inside a freehand, closed
// lasso path. The caller samples the pointer path; closing the final segment on
// release makes a quick rough loop work without demanding pixel-perfect closure.
export function lassoCapturePath(
  placements: Record<number, Placement>,
  cam: Placement,
  path: ScreenPoint[],
  f: number,
): number[] {
  const clean = path.filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y));
  if (clean.length < 3) return [];
  // Reject a held click or near-line scribble. Besides being less surprising,
  // this keeps the edge test above from treating a degenerate path as a lasso.
  let twiceArea = 0;
  for (let i = 0, j = clean.length - 1; i < clean.length; j = i++) {
    twiceArea += clean[j].x * clean[i].y - clean[i].x * clean[j].y;
  }
  if (Math.abs(twiceArea) < 24) return [];

  const out: number[] = [];
  for (const [id, placement] of Object.entries(placements)) {
    const projected = project(cam, placement, f);
    if (projected.visible && pointInPolygon({ x: projected.x, y: projected.y }, clean)) {
      out.push(Number(id));
    }
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

// Spherical centre for a set of placements. Averaging direction vectors, rather
// than yaw numbers, makes 359° + 1° centre on 0° instead of the opposite wall.
// Empty and perfectly-opposed sets use the caller's stable fallback.
export function placementCentre(
  placements: Record<number, Placement>,
  ids: number[],
  fallback: Placement,
): Placement {
  let x = 0, y = 0, z = 0, count = 0;
  for (const id of ids) {
    const placement = placements[id];
    if (!placement) continue;
    const vector = dir(placement.yaw, placement.pitch);
    x += vector[0];
    y += vector[1];
    z += vector[2];
    count += 1;
  }
  const length = Math.hypot(x, y, z);
  if (!count || length < 1e-6) return { yaw: wrapYaw(fallback.yaw), pitch: clampPitch(fallback.pitch) };
  return {
    yaw: wrapYaw(Math.atan2(x, z) / DEG),
    pitch: clampPitch(Math.asin(Math.max(-1, Math.min(1, y / length))) / DEG),
  };
}

function angularDistance(a: Placement, b: Placement): number {
  const av = dir(a.yaw, a.pitch), bv = dir(b.yaw, b.pitch);
  const dot = Math.max(-1, Math.min(1, av[0] * bv[0] + av[1] * bv[1] + av[2] * bv[2]));
  return Math.acos(dot) / DEG;
}

// Nudge newly moved servers into the nearest open angular position. Stationary
// placements are never disturbed; pass every id as `movedIds` after increasing
// icon size to gently untangle the whole saved layout. The golden-angle spiral
// avoids a visible row/grid bias and behaves deterministically for persistence.
export function separatePlacements(
  placements: Record<number, Placement>,
  movedIds: number[],
  minSeparationDeg: number,
): Record<number, Placement> {
  const minSep = Math.max(0.5, Math.min(30, minSeparationDeg));
  const moving = new Set(movedIds);
  const accepted: Placement[] = Object.entries(placements)
    .filter(([id]) => !moving.has(Number(id)))
    .map(([, p]) => p);
  const out = { ...placements };
  const open = (candidate: Placement) => accepted.every((p) => angularDistance(candidate, p) >= minSep);

  for (const id of movedIds) {
    const desired = placements[id];
    if (!desired) continue;
    let chosen = desired;
    if (!open(chosen)) {
      for (let i = 1; i <= 2048; i += 1) {
        const radius = Math.min(120, minSep * 0.58 * Math.sqrt(i));
        const angle = i * 137.507764 * DEG;
        const pitch = clampPitch(desired.pitch + Math.sin(angle) * radius);
        // At higher latitudes a yaw degree covers less actual sphere, so expand
        // the longitude delta to keep the search spiral approximately circular.
        const yawScale = Math.max(0.28, Math.cos(pitch * DEG));
        const candidate = {
          yaw: wrapYaw(desired.yaw + (Math.cos(angle) * radius) / yawScale),
          pitch,
        };
        if (open(candidate)) {
          chosen = candidate;
          break;
        }
      }
    }
    out[id] = chosen;
    accepted.push(chosen);
  }
  return out;
}

// Deterministic tidy-up for the whole sphere. Named neighbourhoods occupy their
// own longitude sectors and their members form compact, centred grids within
// each sector. The final separation pass handles large icon sizes and dense rows.
export function autoArrangePlacements(
  ids: number[],
  serverClusters: Record<number, string>,
  minSeparationDeg: number,
): Record<number, Placement> {
  const unique = [...new Set(ids.filter(Number.isInteger))].sort((a, b) => a - b);
  const grouped = new Map<string, number[]>();
  for (const id of unique) {
    const key = serverClusters[id] || "";
    const members = grouped.get(key) ?? [];
    members.push(id);
    grouped.set(key, members);
  }
  const groups = [...grouped.entries()].sort(([a], [b]) => a.localeCompare(b));
  const out: Record<number, Placement> = {};
  const step = Math.max(8, minSeparationDeg * 1.35);
  for (let gi = 0; gi < groups.length; gi += 1) {
    const members = groups[gi][1];
    const centerYaw = wrapYaw((gi * 360) / Math.max(1, groups.length));
    const cols = Math.max(1, Math.ceil(Math.sqrt(members.length)));
    const rows = Math.ceil(members.length / cols);
    for (let i = 0; i < members.length; i += 1) {
      const col = i % cols;
      const row = Math.floor(i / cols);
      out[members[i]] = {
        yaw: wrapYaw(centerYaw + (col - (cols - 1) / 2) * step),
        pitch: clampPitch((row - (rows - 1) / 2) * step),
      };
    }
  }
  return separatePlacements(out, unique, minSeparationDeg);
}

// -------- persistence (per-device, like desktop icon positions) --------

export const SPACE_BACKDROPS = ["den", "ridge", "void", "garden", "custom"] as const;

export function defaultSpace(): SpaceState {
  return {
    backdrop: "den",
    custom: "",
    shape: "square",
    serverSize: SERVER_SIZE_DEFAULT,
    zoomOnOpen: true,
    entrySound: true,
    showMinimap: true,
    ambience: 72,
    links: 55,
    glow: 84,
    hoverShake: true,
    backdropBlur: 0,
    clusters: [],
    serverClusters: {},
    placements: {},
  };
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
    if (j.shape === "circle" || j.shape === "square") out.shape = j.shape;
    if (typeof j.serverSize === "number" && Number.isFinite(j.serverSize)) {
      out.serverSize = Math.round(Math.max(SERVER_SIZE_MIN, Math.min(SERVER_SIZE_MAX, j.serverSize)));
    }
    if (typeof j.zoomOnOpen === "boolean") out.zoomOnOpen = j.zoomOnOpen;
    if (typeof j.entrySound === "boolean") out.entrySound = j.entrySound;
    if (typeof j.showMinimap === "boolean") out.showMinimap = j.showMinimap;
    for (const key of ["ambience", "links", "glow"] as const) {
      if (typeof j[key] === "number" && Number.isFinite(j[key])) {
        out[key] = Math.round(Math.max(0, Math.min(100, j[key])));
      }
    }
    if (typeof j.hoverShake === "boolean") out.hoverShake = j.hoverShake;
    if (typeof j.backdropBlur === "number" && Number.isFinite(j.backdropBlur)) {
      out.backdropBlur = Math.round(Math.max(0, Math.min(12, j.backdropBlur)));
    }
    if (Array.isArray(j.clusters)) {
      const seen = new Set<string>();
      for (const raw of j.clusters.slice(0, 24)) {
        if (!raw || typeof raw !== "object") continue;
        const id = typeof raw.id === "string" ? raw.id.replace(/[^a-zA-Z0-9_-]/g, "").slice(0, 32) : "";
        const name = typeof raw.name === "string" ? raw.name.trim().slice(0, 32) : "";
        const color = typeof raw.color === "string" && /^#[0-9a-fA-F]{6}$/.test(raw.color) ? raw.color : "#8d7cf5";
        if (!id || !name || seen.has(id)) continue;
        seen.add(id);
        out.clusters.push({ id, name, color });
      }
      if (j.serverClusters && typeof j.serverClusters === "object") {
        for (const [key, value] of Object.entries(j.serverClusters as Record<string, unknown>)) {
          const server = Number(key);
          if (Number.isInteger(server) && typeof value === "string" && seen.has(value)) out.serverClusters[server] = value;
        }
      }
    }
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
