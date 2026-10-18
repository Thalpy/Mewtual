import { PATCH_OSC_WAVES, type JamPatch } from "./jam-contract.ts";
import { canonicalJson } from "./studio-contract.ts";

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
