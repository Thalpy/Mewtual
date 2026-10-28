import { JAM_KIT, JAM_NOTE_VOLUME_MAX, PATCH_OSC_WAVES, TAKE_MAX_BYTES, TAKE_MAX_DURATION_MS, TAKE_MAX_EVENTS, type JamTake } from "./jam-contract.ts";
import { validateJamTake } from "./jam-recorder.ts";
import { legacyJamPatch } from "./jam-patch.ts";
import { FLIPNOTE_FPS_MAX, FLIPNOTE_MAX_FRAMES, FLIPNOTE_MAX_SFX } from "./studio-contract.ts";
import type { PixaSfx } from "./pixa.ts";

/** Audio timing only. PIX validation and scoped publication remain the package/native owners. */
export type JamArchivalAnimation = Readonly<{ fps: number; frameCount: number; sfx: readonly PixaSfx[] }>;
export type JamArchivalEvent = Readonly<{
  ms: number;
  /** Take lane, or a unique synthetic lane for this individual SFX (never a pitch slot). */
  lane: number;
  kind: "on" | "off" | "drum" | "cleanup";
  sequence: number;
  note: number;
  wave?: "sine" | "triangle" | "square" | "sawtooth";
  patch?: number;
  /** Exact authored hold, not the live lost-key-up watchdog. */
  holdMs?: number;
  /** An attack's absolute closing time, used to reject expired delayed attacks. */
  offMs?: number;
  /** The note's authored level as a 0..1 multiplier (from the take event's `v`); absent is 1. */
  level?: number;
  /** Original take event or SFX index, retained as a stable diagnostic key. */
  index: number;
}>;
export type JamArchivalPlan = Readonly<{
  take: JamTake;
  animation?: JamArchivalAnimation;
  events: readonly JamArchivalEvent[];
  /** End of authored content, before the renderer-owned release tail. */
  endMs: number;
}>;

function integer(value: unknown, min: number, max: number): value is number {
  return Number.isSafeInteger(value) && (value as number) >= min && (value as number) <= max;
}

/**
 * A bounded data-only copy before any hash can yield. Accessors, subclasses and toJSON hooks are
 * not content: rejecting them also prevents validation and later serialization reading different
 * scalar values. Actual imports already arrive through byte-capped JSON/package decoders.
 */
function snapshot(value: unknown): unknown {
  let remaining = TAKE_MAX_EVENTS * 10 + 20_000;
  let textUnits = 0;
  const copy = (item: unknown, depth: number): unknown => {
    if (--remaining < 0 || depth > 10) throw new TypeError("archival input structure exceeds its bound");
    if (item === null || typeof item === "boolean" || typeof item === "number") return item;
    if (typeof item === "string") {
      textUnits += item.length;
      if (textUnits > TAKE_MAX_BYTES) throw new TypeError("archival input text exceeds its bound");
      return item;
    }
    if (!item || typeof item !== "object") throw new TypeError("archival input is not JSON data");
    const array = Array.isArray(item);
    const prototype = Object.getPrototypeOf(item);
    if (prototype !== (array ? Array.prototype : Object.prototype) && !(prototype === null && !array)) {
      throw new TypeError("archival input must contain only plain data");
    }
    const keys = Reflect.ownKeys(item);
    const length = array ? Object.getOwnPropertyDescriptor(item, "length")?.value : 0;
    if (keys.length > remaining || (array && (!integer(length, 0, TAKE_MAX_EVENTS) || keys.length !== length + 1))) {
      throw new TypeError("archival input collection exceeds its bound");
    }
    const result: Record<string, unknown> | unknown[] = array ? [] : Object.create(null);
    for (const key of keys) {
      if (array && key === "length") continue;
      if (typeof key !== "string") throw new TypeError("archival input has a symbol field");
      textUnits += key.length;
      if (textUnits > TAKE_MAX_BYTES) throw new TypeError("archival input text exceeds its bound");
      if (array && (!/^(0|[1-9][0-9]*)$/.test(key) || Number(key) >= length)) throw new TypeError("archival input array has an extra field");
      const descriptor = Object.getOwnPropertyDescriptor(item, key)!;
      if (!descriptor.enumerable || !("value" in descriptor)) throw new TypeError("archival input has an accessor or hidden field");
      Object.defineProperty(result, key, { value: copy(descriptor.value, depth + 1), enumerable: true, writable: true, configurable: true });
    }
    return result;
  };
  return copy(value, 0);
}

function freezeDeep<T>(value: T): T {
  if (value && typeof value === "object") {
    for (const child of Object.values(value)) freezeDeep(child);
    Object.freeze(value);
  }
  return value;
}

function noteTailMs(take: JamTake, patch: number | undefined, wave: "sine" | "triangle" | "square" | "sawtooth"): number {
  // Legacy Sounds Only is receiver policy and can change after preparation. Reserve either
  // rendering's release, not just the archived recipe's, so a zero-release custom patch cannot
  // have its 120-ms fallback cut off by an earlier source cleanup or transport completion.
  return Math.max(8, legacyJamPatch(wave).e.r, patch === undefined ? 0 : take.patches[patch].e.r);
}

/**
 * Resolve existing validated take events and authored frame-SFX into one bounded schedule.
 * This is content, not a playback permission: only the engine can mint the archival capability.
 */
export function planJamArchivalPlayback(takeInput: unknown, animationInput?: JamArchivalAnimation): JamArchivalPlan {
  const checked = validateJamTake(snapshot(takeInput));
  if (!checked.ok) throw new TypeError(checked.error);
  const take = checked.take;
  let animation: JamArchivalAnimation | undefined;
  if (animationInput !== undefined) {
    const data = snapshot(animationInput) as JamArchivalAnimation;
    if (!data || Object.keys(data).sort().join(",") !== "fps,frameCount,sfx" ||
      !integer(data.fps, 1, FLIPNOTE_FPS_MAX) || !integer(data.frameCount, 1, FLIPNOTE_MAX_FRAMES) ||
      !Array.isArray(data.sfx) || data.sfx.length > FLIPNOTE_MAX_SFX) throw new TypeError("invalid archival animation timing");
    for (const sfx of data.sfx) {
      if (!sfx || Object.keys(sfx).sort().join(",") !== "durationMs,frame,note,patch" ||
        !integer(sfx.frame, 0, data.frameCount - 1) || !integer(sfx.patch, 0, take.patches.length - 1) ||
        !integer(sfx.note, 0, 127) || !integer(sfx.durationMs, 1, TAKE_MAX_DURATION_MS)) throw new TypeError("invalid archival SFX");
    }
    animation = data;
  }
  const takeEnd = take.events.at(-1)?.ms ?? 0;
  const events: JamArchivalEvent[] = [];
  const held = new Map<string, number>();
  for (let index = 0; index < take.events.length; index++) {
    const event = take.events[index];
    const key = `${event.lane}:${event.n}`;
    if ("d" in event) {
      events.push({ ms: event.ms, lane: event.lane, kind: "drum", sequence: event.q, note: event.n,
        offMs: event.ms + JAM_KIT[event.n].tailMs, index });
    } else if (event.on === 1) {
      // A duplicate held attack is rejected by the renderer. It does not replace the first on
      // here either, so the next key-up closes exactly the voice the engine actually admitted.
      if (held.has(key)) continue;
      held.set(key, events.length);
      events.push({ ms: event.ms, lane: event.lane, kind: "on", sequence: event.q, note: event.n, wave: event.w,
        ...(event.p === undefined ? {} : { patch: event.p }), ...(event.v === undefined ? {} : { level: event.v / JAM_NOTE_VOLUME_MAX }),
        holdMs: takeEnd - event.ms, offMs: takeEnd, index });
    } else {
      const on = held.get(key);
      if (on !== undefined) {
        events[on] = { ...events[on], holdMs: event.ms - events[on].ms, offMs: event.ms };
        held.delete(key);
      }
      events.push({ ms: event.ms, lane: event.lane, kind: "off", sequence: event.q, note: event.n, index });
    }
  }
  let endMs = Math.max(takeEnd, animation ? Math.ceil(animation.frameCount * 1_000 / animation.fps) : 0);
  animation?.sfx.forEach((sfx, index) => {
    const ms = Math.floor(sfx.frame * 1_000 / animation!.fps);
    const offMs = ms + sfx.durationMs;
    const lane = take.lanes.length + index;
    const wave = PATCH_OSC_WAVES[take.patches[sfx.patch].o[0].w];
    events.push({ ms, lane, kind: "on", sequence: 0, note: sfx.note, patch: sfx.patch, wave, holdMs: sfx.durationMs, offMs, index });
    events.push({ ms: offMs, lane, kind: "off", sequence: 1, note: sfx.note, index });
    // A cleanup is separately bounded scheduler work, never an unbounded timer per frame sound.
    events.push({ ms: offMs + noteTailMs(take, sfx.patch, wave) + 5, lane, kind: "cleanup", sequence: 2, note: sfx.note, index });
    endMs = Math.max(endMs, offMs);
  });
  events.sort((a, b) => a.ms - b.ms || a.lane - b.lane || a.sequence - b.sequence);
  return freezeDeep({ take, ...(animation ? { animation } : {}), events, endMs });
}

/** Seek work is linear in the already bounded event log; drums and completed releases do not reattack. */
export function planJamArchivalSeek(plan: JamArchivalPlan, offsetMs: number): Readonly<{ next: number; sounding: readonly number[] }> {
  const offset = Number.isFinite(offsetMs) ? Math.max(0, offsetMs) : 0;
  const sounding: number[] = [];
  let next = plan.events.length;
  for (let index = 0; index < plan.events.length; index++) {
    const event = plan.events[index];
    if (event.ms >= offset) { next = index; break; }
    if (event.kind === "on" && event.offMs! > offset) sounding.push(index);
  }
  return { next, sounding };
}

/** Conservative bounded tail; the engine retains ownership of release semantics. */
export function archivalReleaseTailMs(plan: JamArchivalPlan): number {
  let tail = 0;
  for (const event of plan.events) {
    if (event.kind === "drum") tail = Math.max(tail, JAM_KIT[event.note].tailMs);
    else if (event.kind === "on") tail = Math.max(tail, noteTailMs(plan.take, event.patch, event.wave!));
  }
  return tail;
}
