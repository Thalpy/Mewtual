import { PATCH_OSC_WAVES, type JamPatch } from "./jam-contract.ts";
import { canonicalJson } from "./studio-contract.ts";
import type { SfxView, StudioSoundModel } from "./studio-sound.ts";

/** Names are a presentation hint; the saved descriptor/id remains the playback source. */
export function instrumentName(patch: JamPatch | undefined, library: readonly { name: string; patch: JamPatch }[]): string {
  if (!patch) return "Unavailable sound";
  const encoded = canonicalJson(patch);
  const named = library.find(item => canonicalJson(item.patch) === encoded);
  if (named) return named.name;
  const wave = PATCH_OSC_WAVES[patch.o[0]?.w] ?? "custom";
  return `${wave.charAt(0).toUpperCase()}${wave.slice(1)} sound`;
}

export function noteName(note: number): string {
  return `${["C", "C♯", "D", "D♯", "E", "F", "F♯", "G", "G♯", "A", "A♯", "B"][note % 12]}${Math.floor(note / 12) - 1}`;
}

/** Seconds are only the UI unit. Saves retain an exact, bounded integer millisecond hold. */
export function soundLengthMs(seconds: number): number | null {
  if (!Number.isFinite(seconds) || seconds < .001 || seconds > 600) return null;
  const ms = Math.round(seconds * 1000);
  return ms >= 1 && ms <= 600000 ? ms : null;
}

// ---------------------------------------------------------------------------------------------
// Pure helpers behind the SOUND inspector tab (StudioSound.svelte). Everything below is plain
// data in, plain data out so the layout rules can be pinned by unit tests without a DOM.
// ---------------------------------------------------------------------------------------------

/**
 * Wave glyph paths in a 26x12 viewBox, indexed like `PATCH_OSC_WAVES` (sine, triangle, square,
 * sawtooth). They are the same strokes the call rack draws (`INST_TILES` in App.svelte) so a
 * recipe looks the same wherever it appears.
 */
export const WAVE_GLYPHS: readonly string[] = [
  "M1 6q3-5 6 0t6 0 6 0 6 0", // sine
  "M1 10 4.5 2 8.5 10 12.5 2 16.5 10 20.5 2 24.5 10", // triangle
  "M1 10h3V2h6v8h6V2h6v8h3", // square
  "M1 10 7 2v8l6-8v8l6-8v8l5-6.7", // sawtooth
];
/** A dashed flat line: there is no recipe to draw (removed slot, missing descriptor). */
export const MISSING_GLYPH = "M2 6h3m3 0h3m3 0h3m3 0h3";

/** The glyph of a recipe's first oscillator, or the dashed "nothing here" line. */
export function waveGlyph(patch: JamPatch | null | undefined): string {
  const w = patch?.o[0]?.w;
  return w !== undefined && WAVE_GLYPHS[w] !== undefined ? WAVE_GLYPHS[w] : MISSING_GLYPH;
}

/** Rack tiles are three to a 236px row; twelve uppercase mono characters is what one can show. */
export function rackLabel(name: string, limit = 12): string {
  return name.trim().slice(0, limit).toUpperCase();
}

/** The display name an imported file gets on its rack tile: the stem, trimmed like a share's. */
export function importName(fileName: string | undefined, limit = 12): string {
  const base = (fileName ?? "").trim().replace(/^.*[\\/]/, "");
  const stem = base.replace(/\.(json|jampatch)$/i, "");
  return rackLabel(stem, limit) || "IMPORT";
}

/** Recipe equality by canonical bytes: library arrays may be rebuilt between renders. */
export function samePatch(a: JamPatch, b: JamPatch): boolean {
  return a === b || canonicalJson(a) === canonicalJson(b);
}

// --- pitch -------------------------------------------------------------------------------------

export const NOTE_MIN = 0;
export const NOTE_MAX = 127;

export function clampNote(note: number, lo = NOTE_MIN, hi = NOTE_MAX): number {
  const low = Math.max(NOTE_MIN, lo), high = Math.min(NOTE_MAX, hi);
  return Math.min(high, Math.max(low, Math.round(note)));
}

/** The octave number `noteName` prints: C4 is 60, so register = floor(note / 12) - 1. */
export function registerOf(note: number): number {
  return Math.floor(note / 12) - 1;
}

export type PianoKey = { note: number; label: string; /** percent from the left; black keys only */ left: number };

/**
 * One octave of keys starting at C{register}. Seven white keys share the width evenly; the five
 * black keys sit over the C/D, D/E, F/G, G/A and A/B boundaries, each 7.2% wide. A key whose note
 * exceeds 127 (register 9 ends at G9) is still returned so the layout stays an octave; the caller
 * disables it.
 */
export function pianoKeys(register: number): { white: PianoKey[]; black: PianoKey[] } {
  const base = (register + 1) * 12;
  const whiteOffsets = [0, 2, 4, 5, 7, 9, 11], blackOffsets = [1, 3, 6, 8, 10], blackAfterWhite = [1, 2, 4, 5, 6];
  const white = whiteOffsets.map((offset, i) => ({ note: base + offset, label: noteName(base + offset), left: i * (100 / 7) }));
  const black = blackOffsets.map((offset, i) => ({ note: base + offset, label: noteName(base + offset), left: blackAfterWhite[i] * (100 / 7) - 3.6 }));
  return { white, black };
}

// --- length ------------------------------------------------------------------------------------

/** The four one-tap holds. 250 ms is also a new marker's default. */
export const LENGTH_PRESETS: readonly { ms: number; label: string }[] = [
  { ms: 100, label: "0.1" }, { ms: 250, label: "0.25" }, { ms: 500, label: "0.5" }, { ms: 1000, label: "1 s" },
];
export const DEFAULT_LENGTH_MS = 250;

/** "0.25 s": the row/alternative unit. Exact milliseconds survive; this is only how they read. */
export function secondsLabel(ms: number): string {
  return `${ms / 1000} s`;
}

// --- shelves -----------------------------------------------------------------------------------

export type ShelfName = "preset" | "mine" | "share";
export type LibraryItem = { name: string; patch: JamPatch; shelf: ShelfName };
export const SHELF_LABELS: Readonly<Record<ShelfName, string>> = { preset: "presets", mine: "my library", share: "from the share" };
const SHELF_ORDER: readonly ShelfName[] = ["preset", "mine", "share"];

/** Library recipes grouped by shelf in presets/mine/share order; empty shelves are not returned. */
export function groupShelves(library: readonly LibraryItem[]): { shelf: ShelfName; label: string; items: LibraryItem[] }[] {
  return SHELF_ORDER
    .map((shelf) => ({ shelf, label: SHELF_LABELS[shelf], items: library.filter((item) => item.shelf === shelf) }))
    .filter((group) => group.items.length > 0);
}

// --- readiness ---------------------------------------------------------------------------------

/**
 * What the "needs attention" banner counts. `markers` is the number of distinct marker ids that
 * are orphaned (frame gone), dangling (recipe gone), over the 4096 cap, or carry a concurrent
 * alternative; `recipes` is the number of recipe ids over the 64 cap. Export overflow and
 * recipe-level conflicts leave the document not ready without adding to either count, which is
 * why the caller must not infer readiness from these numbers alone.
 */
export function soundAttention(model: StudioSoundModel): { markers: number; recipes: number } {
  const r = model.readiness;
  const markers = new Set<string>([...r.missingFrames, ...r.missingPatches, ...r.sfxOverflow]);
  for (const sfx of model.sfx) if (sfx.conflicts.length) markers.add(sfx.id);
  return { markers: markers.size, recipes: new Set(r.patchOverflow).size };
}

/** Live markers per recipe id, for "2 markers" / "unused" on the evidence rows. */
export function patchUsage(sfx: readonly SfxView[]): Map<string, number> {
  const usage = new Map<string, number>();
  for (const marker of sfx) usage.set(marker.patch, (usage.get(marker.patch) ?? 0) + 1);
  return usage;
}

// --- the "all sounds" list -----------------------------------------------------------------------

/** How many markers one page of the "all sounds" list renders; a 4096-marker document must not render 4096 rows. */
export const ALL_SOUNDS_PAGE = 24;
/** Page size for the evidence lists (recipes, cleanup, raw records) and alternatives per record. */
export const EVIDENCE_PAGE = 8;
export const ALTERNATIVES_PAGE = 3;

export type SfxRow = SfxView & { /** 1-based timeline number, 0 when the frame is gone */ frameNumber: number };

/** Timeline order (gone frames, numbered 0, sort first) and then id, so the order is stable. */
export function sortSfx(sfx: readonly SfxView[], frameIndexOf: (frameId: string) => number): SfxRow[] {
  return sfx
    .map((marker) => ({ ...marker, frameNumber: frameIndexOf(marker.frame) }))
    .sort((a, b) => a.frameNumber - b.frameNumber || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

/**
 * One page of a list. The page index is clamped, so a removal that empties the last page shows
 * the new last page instead of an empty one; `pages` is at least 1 so pagers never divide by 0.
 */
export function pageSlice<T>(items: readonly T[], page: number, size: number): { items: T[]; page: number; pages: number } {
  const pages = Math.max(1, Math.ceil(items.length / size));
  const current = Math.min(Math.max(0, page), pages - 1);
  return { items: items.slice(current * size, current * size + size), page: current, pages };
}
