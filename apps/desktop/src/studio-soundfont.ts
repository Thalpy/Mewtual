// studio-soundfont.ts: the soundfont editor's pure pieces.
//
// `StudioSoundfont.svelte` is the call drawer's patch editor lifted into a modal, and the drawer
// keeps its geometry, its key map and its import rules inline where nothing can assert them. The
// pieces that do not need a DOM live here instead: knob arc geometry and stepping, the oscillator
// summary line, the piano layout and home-row map, the register arithmetic, and the one import
// decoder that turns a chosen file into a validated `jam-patch:v1` or a reason it was refused.
//
// Nothing here mutates its input and nothing here touches the patch state transitions: those stay
// in `jam-editor.ts`, and every recipe still passes through `validateJamPatch` before it is used.

import { EMOJI_SOUND_MAX_BYTES, parseEmojiSoundJson } from "./emoji-sound.ts";
import {
  JAM_PATCH_FILE_MAX_BYTES,
  JAM_PATCH_NAME_MAX_CHARS,
  PATCH_OSC_WAVES,
  type JamOsc,
  type JamPatch,
} from "./jam-contract.ts";
import { JAM_ENV_ON, JAM_FILTER_OFF, JAM_SENDS_OFF } from "./jam-editor.ts";
import { validateJamPatch } from "./jam-patch.ts";

// --- Recipes ----------------------------------------------------------------------------------

/** A JSON clone: the only copy the editor hands anyone, so no caller ever holds its live state. */
export function clonePatch(patch: JamPatch): JamPatch {
  return JSON.parse(JSON.stringify(patch)) as JamPatch;
}

/**
 * The "blank" start tile: one plain sine, a gentle default envelope, the filter wide open, no
 * wobble and nothing sent to the room. Deliberately the neutral setting of every stage the editor
 * can bypass, so the stage lamps read OFF/WIDE on a fresh sheet and the first knob turned is the
 * first thing heard. Validated in its test; the component never assumes it.
 */
export function blankSoundfontPatch(): JamPatch {
  return {
    v: 1,
    o: [{ w: 0, t: 0, c: 0, l: 80 }],
    e: { ...JAM_ENV_ON },
    f: { ...JAM_FILTER_OFF },
    l: { r: 100, d: 0, t: 0 },
    x: { ...JAM_SENDS_OFF },
  };
}

/**
 * The canonical bytes of a recipe, or null when it would not validate. Two recipes are "the same
 * sound" exactly when their keys match: this is what the share and the wire hash, so the editor's
 * "edited" chip and its start-from highlight can never disagree with the id a save would mint.
 */
export function recipeKey(patch: JamPatch): string | null {
  const checked = validateJamPatch(clonePatch(patch));
  return checked.ok ? checked.canonical : null;
}

// --- Oscillator tiles -------------------------------------------------------------------------

export type WaveTile = {
  wave: (typeof PATCH_OSC_WAVES)[number];
  /** The wire value (`JamOsc.w`): the index into `PATCH_OSC_WAVES`. */
  index: number;
  label: string;
  /** A waveform glyph in a 26 x 12 box, the same strokes the call drawer's tiles draw. */
  d: string;
};

/** The four timbres in the call drawer's tile order, so both editors read the same way. */
export const WAVE_TILES: readonly WaveTile[] = [
  { wave: "triangle", index: PATCH_OSC_WAVES.indexOf("triangle"), label: "TRI", d: "M1 10 4.5 2 8.5 10 12.5 2 16.5 10 20.5 2 24.5 10" },
  { wave: "sine", index: PATCH_OSC_WAVES.indexOf("sine"), label: "SIN", d: "M1 6q3-5 6 0t6 0 6 0 6 0" },
  { wave: "square", index: PATCH_OSC_WAVES.indexOf("square"), label: "SQR", d: "M1 10h3V2h6v8h6V2h6v8h3" },
  { wave: "sawtooth", index: PATCH_OSC_WAVES.indexOf("sawtooth"), label: "SAW", d: "M1 10 7 2v8l6-8v8l6-8v8l5-6.7" },
];

/** The tile for a wire wave value; an out-of-range value draws as a sine rather than nothing. */
export function waveTile(w: number): WaveTile {
  return WAVE_TILES.find((tile) => tile.index === w) ?? WAVE_TILES[1];
}

/** A collapsed layer's one-line summary: wave, then only the offsets that are not zero. */
export function oscSummary(osc: JamOsc): string {
  const parts = [waveTile(osc.w).label];
  if (osc.t !== 0) parts.push(`${osc.t > 0 ? "+" : ""}${osc.t} st`);
  if (osc.c !== 0) parts.push(`${osc.c > 0 ? "+" : ""}${osc.c} ct`);
  return parts.join(" · ");
}

// --- Knobs ------------------------------------------------------------------------------------
// A knob sweeps 270 degrees, from -135 (seven o'clock) to +135 (five o'clock). Dragging up or
// right raises it; a full sweep is KNOB_SWEEP_PX of travel, or KNOB_FINE_SWEEP_PX with Shift held,
// which is the fine grip. Arrow keys and the wheel step by a sixtieth of the range (Shift: by one).
// These numbers are the call drawer's, so a knob feels the same in both editors.

export const KNOB_SWEEP_PX = 170;
export const KNOB_FINE_SWEEP_PX = 1200;
const KNOB_MIN_DEG = -135;
const KNOB_MAX_DEG = 135;

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** Where the tick points for a value, in degrees from twelve o'clock. */
export function knobAngle(value: number, min: number, max: number): number {
  const ratio = (value - min) / (max - min || 1);
  return KNOB_MIN_DEG + (KNOB_MAX_DEG - KNOB_MIN_DEG) * clamp(ratio, 0, 1);
}

/**
 * The tick angle and the lit arc for one knob, on a 32 x 32 face with the ring at radius 13.
 *
 * The arc runs from the knob's rest position to the tick: the left stop for a plain range, twelve
 * o'clock for a bipolar one, so a detuned layer or a negative filter sweep shows WHICH way it leans
 * and not only how far. At rest the arc is a zero-length path, which SVG draws as nothing.
 */
export function knobArc(value: number, min: number, max: number, bipolar: boolean): { deg: number; d: string } {
  const deg = knobAngle(value, min, max);
  const from = bipolar ? 0 : KNOB_MIN_DEG;
  const a = (deg * Math.PI) / 180;
  const b = (from * Math.PI) / 180;
  const large = Math.abs(deg - from) > 180 ? 1 : 0;
  const sweep = deg > from ? 1 : 0;
  const d = `M${(16 + 13 * Math.sin(b)).toFixed(2)} ${(16 - 13 * Math.cos(b)).toFixed(2)} A13 13 0 ${large} ${sweep} ${(16 + 13 * Math.sin(a)).toFixed(2)} ${(16 - 13 * Math.cos(a)).toFixed(2)}`;
  return { deg, d };
}

/** One arrow-key or wheel notch: a sixtieth of the range (never under one), or one with Shift. */
export function knobStep(value: number, min: number, max: number, direction: number, fine: boolean): number {
  const step = fine ? 1 : Math.max(1, Math.round((max - min) / 60));
  return clamp(value + direction * step, min, max);
}

/** The value under a drag that has travelled `px` (up and right positive) since it started. */
export function knobDragValue(startValue: number, min: number, max: number, px: number, fine: boolean): number {
  const sweep = fine ? KNOB_FINE_SWEEP_PX : KNOB_SWEEP_PX;
  const raw = startValue + (px / sweep) * (max - min);
  return clamp(Math.round(raw), min, max);
}

// Knob readouts. The patch stores integers in its own units; these are the labels under the dial.
export function secondsLabel(ms: number): string {
  return `${(ms / 1000).toFixed(1)}s`;
}
export function cutoffLabel(hz: number): string {
  return hz >= 1000 ? `${(hz / 1000).toFixed(1)}k` : String(hz);
}
export function lfoRateLabel(centiHz: number): string {
  return `${(centiHz / 100).toFixed(2)}hz`;
}

// --- Audition keyboard ------------------------------------------------------------------------
// Two octaves from C{register}. MIDI 60 is C4, so the base note of register r is (r + 1) * 12: the
// same arithmetic the call drawer's octave stepper uses, so "C4" means the same pitch in both.

export const SOUNDFONT_REGISTER_MIN = 1;
export const SOUNDFONT_REGISTER_MAX = 7;
export const SOUNDFONT_REGISTER_DEFAULT = 4;

export function clampRegister(register: number): number {
  return clamp(Math.round(register), SOUNDFONT_REGISTER_MIN, SOUNDFONT_REGISTER_MAX);
}

/** The MIDI note of C in this register. Register 7 plus the top white key (23) is 119, under 127. */
export function registerBase(register: number): number {
  return (clampRegister(register) + 1) * 12;
}

/** What the stepper shows: the two octaves the keyboard spans, e.g. "C4–C5". */
export function registerLabel(register: number): string {
  const r = clampRegister(register);
  return `C${r}–C${r + 1}`;
}

/**
 * Home row to semitones above the register's C, laid out like a piano: the letter row is the
 * white keys and the row above holds the black ones in their gaps. `;` is the E of the second
 * octave, seventeen keys in all, so one hand reaches past the octave without shifting register.
 */
export const HOME_ROW_SEMITONES: Readonly<Record<string, number>> = {
  a: 0, w: 1, s: 2, e: 3, d: 4, f: 5, t: 6, g: 7, y: 8, h: 9, u: 10, j: 11, k: 12, o: 13, l: 14, p: 15, ";": 16,
};

/** Semitone offsets of the fourteen white keys across two octaves. */
export const PIANO_WHITE_KEYS: readonly number[] = [0, 2, 4, 5, 7, 9, 11, 12, 14, 16, 17, 19, 21, 23];

/** A black key's width as a share of the keyboard, so its `left` can centre it on a white seam. */
export const PIANO_BLACK_WIDTH_PCT = 3.7;

export type PianoBlackKey = { semitone: number; /** CSS left, percent of the keyboard width. */ left: number };

/**
 * The ten black keys, each centred on the seam after the white key it follows. Positions are
 * derived from the white-key count rather than typed in, so adding an octave moves every key.
 */
export const PIANO_BLACK_KEYS: readonly PianoBlackKey[] = (() => {
  const whiteWidth = 100 / PIANO_WHITE_KEYS.length;
  const keys: PianoBlackKey[] = [];
  for (let octave = 0; octave < 2; octave += 1) {
    for (const [semitone, afterWhite] of [[1, 0], [3, 1], [6, 3], [8, 4], [10, 5]] as const) {
      const whiteIndex = afterWhite + octave * 7;
      keys.push({
        semitone: semitone + octave * 12,
        left: Number(((whiteIndex + 1) * whiteWidth - PIANO_BLACK_WIDTH_PCT / 2).toFixed(2)),
      });
    }
  }
  return keys;
})();

// --- Import -----------------------------------------------------------------------------------

/** Both admitted formats cap at the same 4 KiB; a larger file is refused before it is read. */
export const SOUNDFONT_IMPORT_MAX_BYTES = Math.max(JAM_PATCH_FILE_MAX_BYTES, EMOJI_SOUND_MAX_BYTES);

export type SoundfontImport =
  | Readonly<{ ok: true; patch: JamPatch; /** Something the recipe left behind that the user should hear about. */ notice: string | null }>
  | Readonly<{ ok: false; error: string }>;

/**
 * Decode a chosen file's text into a recipe.
 *
 * Two shapes are admitted: a bare `jam-patch:v1` (what a `.jampatch` holds) and an emoji sound
 * sidecar, whose recipe is lifted out and whose pitch range and authored length are NOT kept, since
 * a soundfont has neither. The shape decides, not the extension: a sidecar renamed `.jampatch`
 * still has `patch` inside it and still imports, and a bare recipe saved as `.json` is still a
 * recipe. Whichever shape is picked, its own decoder reports the error, so a broken sidecar says
 * what is wrong with the sidecar rather than "patch must contain exactly the fields".
 */
export function importSoundfontText(text: string): SoundfontImport {
  if (new TextEncoder().encode(text).length > SOUNDFONT_IMPORT_MAX_BYTES) {
    return { ok: false, error: `file is larger than ${SOUNDFONT_IMPORT_MAX_BYTES} bytes` };
  }
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return { ok: false, error: "file is not valid JSON" };
  }
  const isRecord = typeof value === "object" && value !== null && !Array.isArray(value);
  if (isRecord && "patch" in (value as Record<string, unknown>)) {
    try {
      return {
        ok: true,
        patch: parseEmojiSoundJson(text).patch,
        notice: "Imported the emoji sound's recipe; its pitch range and length were not kept.",
      };
    } catch (e) {
      return { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
  }
  const checked = validateJamPatch(value);
  return checked.ok ? { ok: true, patch: checked.patch, notice: null } : { ok: false, error: checked.error };
}

/** The name a file suggests for what it held: its stem, trimmed to what a name may be. */
export function importName(fileName: string, limit = JAM_PATCH_NAME_MAX_CHARS): string {
  const base = fileName.trim().replace(/^.*[\\/]/, "");
  const stem = base.replace(/\.[^.]*$/, "");
  return stem.trim().slice(0, limit);
}
