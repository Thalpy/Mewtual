import {
  JAM_PATCH_ID_HEX_CHARS,
  JAM_PATCH_VERSION,
  JAM_RENDERER_VERSION,
  PATCH_CUTOFF_MAX_HZ,
  PATCH_CUTOFF_MIN_HZ,
  PATCH_DETUNE_CENTS,
  PATCH_ENV_ATTACK_MAX_MS,
  PATCH_ENV_DECAY_MAX_MS,
  PATCH_ENV_RELEASE_MAX_MS,
  PATCH_FILTER_ENV_RANGE,
  PATCH_FILTER_MODES,
  PATCH_LEVEL_MAX,
  PATCH_LFO_DESTS,
  PATCH_LFO_RATE_MAX_CHZ,
  PATCH_LFO_RATE_MIN_CHZ,
  PATCH_OSC_MAX,
  PATCH_OSC_MIN,
  PATCH_OSC_WAVES,
  PATCH_TRANSPOSE_SEMITONES,
  type JamPatch,
  type LegacyWave,
} from "./jam-contract.ts";

export type PatchValidation =
  | Readonly<{ ok: true; patch: JamPatch; canonical: string }>
  | Readonly<{ ok: false; error: string }>;

function plainRecord(value: unknown): Record<string, unknown> | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return null;
  try {
    const proto = Object.getPrototypeOf(value);
    if (proto !== Object.prototype && proto !== null) return null;
    const descriptors = Object.values(Object.getOwnPropertyDescriptors(value));
    return descriptors.every((descriptor) => !descriptor.get && !descriptor.set)
      ? value as Record<string, unknown>
      : null;
  } catch {
    return null;
  }
}

function hasExactKeys(value: Record<string, unknown>, expected: readonly string[]): boolean {
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  return actual.length === wanted.length && actual.every((key, index) => key === wanted[index]);
}

function integer(value: unknown, min: number, max: number): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= min && value <= max
    ? value
    : null;
}

function invalid(error: string): PatchValidation {
  return { ok: false, error };
}

/**
 * The one patch validator for wire announcements, local drafts/imports and take playback.
 *
 * Unknown properties and non-plain objects fail closed. Reconstructing every object in protocol
 * declaration order gives `jam-patch:v1` exactly one canonical byte representation; hashing a
 * caller's original key order would make equal recipes acquire different ids.
 */
export function validateJamPatch(value: unknown): PatchValidation {
  const root = plainRecord(value);
  if (!root || !hasExactKeys(root, ["v", "o", "e", "f", "l", "x"]) || root.v !== 1) {
    return invalid(`patch must contain exactly the ${JAM_PATCH_VERSION} fields`);
  }

  if (!Array.isArray(root.o) || root.o.length < PATCH_OSC_MIN || root.o.length > PATCH_OSC_MAX) {
    return invalid(`o must contain ${PATCH_OSC_MIN}..${PATCH_OSC_MAX} oscillators`);
  }
  const oscillators: JamPatch["o"] = [];
  for (const candidate of root.o) {
    const osc = plainRecord(candidate);
    if (!osc || !hasExactKeys(osc, ["w", "t", "c", "l"])) return invalid("oscillator fields are invalid");
    const w = integer(osc.w, 0, PATCH_OSC_WAVES.length - 1);
    const t = integer(osc.t, -PATCH_TRANSPOSE_SEMITONES, PATCH_TRANSPOSE_SEMITONES);
    const c = integer(osc.c, -PATCH_DETUNE_CENTS, PATCH_DETUNE_CENTS);
    const l = integer(osc.l, 0, PATCH_LEVEL_MAX);
    if (w === null || t === null || c === null || l === null) return invalid("oscillator value is outside its bound");
    oscillators.push({ w, t, c, l });
  }

  const env = plainRecord(root.e);
  if (!env || !hasExactKeys(env, ["a", "d", "s", "r"])) return invalid("envelope fields are invalid");
  const attack = integer(env.a, 0, PATCH_ENV_ATTACK_MAX_MS);
  const decay = integer(env.d, 0, PATCH_ENV_DECAY_MAX_MS);
  const sustain = integer(env.s, 0, PATCH_LEVEL_MAX);
  const release = integer(env.r, 0, PATCH_ENV_RELEASE_MAX_MS);
  if (attack === null || decay === null || sustain === null || release === null) {
    return invalid("envelope value is outside its bound");
  }

  const filter = plainRecord(root.f);
  if (!filter || !hasExactKeys(filter, ["m", "c", "q", "e"])) return invalid("filter fields are invalid");
  const mode = integer(filter.m, 0, PATCH_FILTER_MODES.length - 1);
  const cutoff = integer(filter.c, PATCH_CUTOFF_MIN_HZ, PATCH_CUTOFF_MAX_HZ);
  const q = integer(filter.q, 0, PATCH_LEVEL_MAX);
  const envAmount = integer(filter.e, -PATCH_FILTER_ENV_RANGE, PATCH_FILTER_ENV_RANGE);
  if (mode === null || cutoff === null || q === null || envAmount === null) {
    return invalid("filter value is outside its bound");
  }

  const lfo = plainRecord(root.l);
  if (!lfo || !hasExactKeys(lfo, ["r", "d", "t"])) return invalid("LFO fields are invalid");
  const rate = integer(lfo.r, PATCH_LFO_RATE_MIN_CHZ, PATCH_LFO_RATE_MAX_CHZ);
  const depth = integer(lfo.d, 0, PATCH_LEVEL_MAX);
  const target = integer(lfo.t, 0, PATCH_LFO_DESTS.length - 1);
  if (rate === null || depth === null || target === null) return invalid("LFO value is outside its bound");

  const sends = plainRecord(root.x);
  if (!sends || !hasExactKeys(sends, ["c", "d", "r"])) return invalid("effect-send fields are invalid");
  const chorus = integer(sends.c, 0, PATCH_LEVEL_MAX);
  const delay = integer(sends.d, 0, PATCH_LEVEL_MAX);
  const reverb = integer(sends.r, 0, PATCH_LEVEL_MAX);
  if (chorus === null || delay === null || reverb === null) return invalid("effect send is outside its bound");

  const patch: JamPatch = {
    v: 1,
    o: oscillators,
    e: { a: attack, d: decay, s: sustain, r: release },
    f: { m: mode, c: cutoff, q, e: envAmount },
    l: { r: rate, d: depth, t: target },
    x: { c: chorus, d: delay, r: reverb },
  };
  return { ok: true, patch, canonical: JSON.stringify(patch) };
}

export function parseJamPatchJson(raw: string): PatchValidation {
  try {
    return validateJamPatch(JSON.parse(raw));
  } catch {
    return invalid("patch is not valid JSON");
  }
}

function toHex(bytes: ArrayBuffer): string {
  return [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/**
 * Hash identity promises the same normalized recipe, not bit-identical sound. Renderer/platform
 * differences remain explicitly outside the `mewtual-synth:v1` contract.
 */
export async function jamPatchId(patch: JamPatch): Promise<string> {
  const validated = validateJamPatch(patch);
  if (!validated.ok) throw new TypeError(validated.error);
  const bytes = new TextEncoder().encode(validated.canonical);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return toHex(digest).slice(0, JAM_PATCH_ID_HEX_CHARS);
}

export async function verifyJamPatchId(id: string, patch: JamPatch): Promise<boolean> {
  return new RegExp(`^[0-9a-f]{${JAM_PATCH_ID_HEX_CHARS}}$`).test(id) && id === await jamPatchId(patch);
}

/** Internal fallback recipe. `w` keeps its legacy wire meaning and is always safe to render. */
export function legacyJamPatch(wave: LegacyWave): JamPatch {
  const waveIndex = PATCH_OSC_WAVES.indexOf(wave);
  return {
    v: 1,
    o: [{ w: waveIndex >= 0 ? waveIndex : 1, t: 0, c: 0, l: 100 }],
    e: { a: 12, d: 380, s: 45, r: 120 },
    f: { m: 0, c: PATCH_CUTOFF_MAX_HZ, q: 0, e: 0 },
    l: { r: 100, d: 0, t: 0 },
    x: { c: 0, d: 0, r: 0 },
  };
}

export const JAM_RENDERER_CONTRACT = JAM_RENDERER_VERSION;
