import type { JamPatch } from "./jam-contract.ts";
import { validateJamPatch } from "./jam-patch.ts";

export const EMOJI_SOUND_MAX_BYTES = 4096;
export type EmojiSound = Readonly<{ v: 1; img: string; patch: JamPatch; lo: number; hi: number; len: number }>;

/**
 * Local file-source decoder only. This does not authenticate a shared emoji sidecar, bind it to
 * a picker image, or grant chat/call authority: those require the separate verified catalog path.
 * JSON parsing gives us owned data without executing caller accessors; every recipe is rebuilt
 * by the existing validator. The authored hold is copied exactly, never replaced by a default.
 */
export function parseEmojiSoundJson(json: string): EmojiSound {
  if (typeof json !== "string" || json.length > EMOJI_SOUND_MAX_BYTES || new TextEncoder().encode(json).length > EMOJI_SOUND_MAX_BYTES) throw new Error("emoji sound exceeds 4KiB");
  let value: unknown;
  try { value = JSON.parse(json); } catch { throw new Error("emoji sound is not valid JSON"); }
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("emoji sound is not an object");
  const r = value as Record<string, unknown>;
  if (Object.keys(r).sort().join(",") !== "hi,img,len,lo,patch,v" || r.v !== 1 || typeof r.img !== "string" || !/^[0-9a-f]{64}$/.test(r.img)) throw new Error("emoji sound version, image or fields are invalid");
  const integer = (v: unknown, min: number, max: number): v is number => typeof v === "number" && Number.isInteger(v) && v >= min && v <= max;
  if (!integer(r.lo, 0, 127) || !integer(r.hi, r.lo, 127) || !integer(r.len, 20, 1000)) throw new Error("emoji sound note range or authored length is invalid");
  const patch = validateJamPatch(r.patch);
  if (!patch.ok) throw new Error(patch.error);
  return { v: 1, img: r.img, patch: patch.patch, lo: r.lo, hi: r.hi, len: r.len };
}
