/**
 * Pure helpers for the global flipnote viewer (`PixaViewer.svelte`): frame/elapsed arithmetic,
 * the visual-only playback clock, and the small text formatters the head row and transport use.
 *
 * Everything here is deterministic and DOM-free so the viewer's timing rules can be pinned by
 * unit tests without mounting the component. The viewer itself owns the two playback sources
 * (host audio clock vs. this module's visual clock); this module only answers "where is the
 * picture at time t" and "what should the labels say".
 */
import type { Pixa } from "./pixa.ts";
import { FLIPNOTE_H, FLIPNOTE_W } from "./studio-contract.ts";

/**
 * Upper bound on the frame-thumbnail strip. Each thumbnail decodes one frame and retains a
 * 64×48 canvas backing store, so the strip is the only place the viewer holds more than one
 * expanded raster at a time (the main canvas expands only the frame on screen, see
 * `PixaCanvas.svelte`). 48 tiles is roughly 600 KiB of RGBA plus canvas overhead; a 999-frame
 * package would otherwise decode and retain every frame up front. Above the cap the strip is
 * omitted entirely rather than truncated: a strip showing 48 of 999 frames with no indication of
 * the rest would misrepresent the animation's length.
 */
export const PIXA_VIEWER_THUMB_LIMIT = 48;

/**
 * Cadence of the visual-only loop. 40 ms is finer than the fastest authored frame (24 fps is
 * ~42 ms) so no frame is skipped, and matches the inline chat previews (`pixa-preview.ts`) so
 * both silent renderers drift identically. A `setTimeout` chain, not `requestAnimationFrame`:
 * background or throttled WebViews may suspend rAF entirely, and the timer is what the tests
 * drive with an injected clock.
 */
export const PIXA_VIEWER_TICK_MS = 40;

/** Keep a frame index inside `[0, count - 1]`; an empty package maps everything to 0. */
export function clampFrame(frame: number, count: number): number {
  if (!Number.isFinite(frame) || count <= 0) return 0;
  return Math.min(count - 1, Math.max(0, Math.trunc(frame)));
}

/**
 * The authored start time of a frame. This is what the audio host is asked to start from, so
 * sound markers on the held frame still fire when playback resumes there.
 */
export function frameOffsetMs(frame: number, fps: number): number {
  return frame * 1000 / fps;
}

/**
 * A time safely inside a frame, for driving a canvas that derives its frame from elapsed time
 * (`pixaFrameAt` floors `elapsedMs * fps / 1000`). Feeding it the exact frame start is a float
 * hazard: `1 * 1000 / 3 * 3 / 1000` can round to `0.999…` and show the previous frame. The
 * midpoint is immune to that for every legal fps and is never mistaken for the next frame.
 */
export function frameProbeMs(frame: number, fps: number): number {
  return (frame + 0.5) * 1000 / fps;
}

/** Length of the picture track; audio may run longer through sound tails, which the host owns. */
export function packageDurationMs(pixa: Pick<Pixa, "frames" | "fps">): number {
  return pixa.frames.length * 1000 / pixa.fps;
}

/** A visual-only run: where it started on the picture track and on the injected clock. */
export type VisualRun = Readonly<{ baseMs: number; originMs: number; loop: boolean }>;

/**
 * Position of a visual-only run at `nowMs`. With `loop` the track wraps forever; without it the
 * run reports `ended` once the last frame's time has elapsed, and the caller holds that frame.
 * A clock that steps backwards (an injected test clock, or a paused tab resuming) is treated as
 * no time passing rather than negative progress.
 */
export function visualElapsedAt(run: VisualRun, nowMs: number, durationMs: number): { elapsedMs: number; ended: boolean } {
  if (!(durationMs > 0)) return { elapsedMs: 0, ended: true };
  const raw = run.baseMs + Math.max(0, nowMs - run.originMs);
  if (run.loop) return { elapsedMs: raw % durationMs, ended: false };
  if (raw >= durationMs) return { elapsedMs: durationMs, ended: true };
  return { elapsedMs: raw, ended: false };
}

/** Sound markers per frame index, for the "♪ n" chip and the thumbnail pills. */
export function sfxCountByFrame(pixa: Pick<Pixa, "sfx"> | null): ReadonlyMap<number, number> {
  const counts = new Map<number, number>();
  if (pixa) for (const entry of pixa.sfx) counts.set(entry.frame, (counts.get(entry.frame) ?? 0) + 1);
  return counts;
}

/**
 * A package with nothing to hear. Starting the audio host for it would still resume the audio
 * context and mint an engine capability for silence, and a refused start would then block the
 * only Play button; the viewer drives such packages from its visual clock instead.
 */
export function isSilentPixa(pixa: Pick<Pixa, "sfx" | "take">): boolean {
  return pixa.sfx.length === 0 && pixa.take.events.length === 0;
}

/** Transport readout seconds, two decimals, never negative or NaN. */
export function formatSeconds(ms: number): string {
  return (Math.max(0, Number.isFinite(ms) ? ms : 0) / 1000).toFixed(2);
}

/** Lowercase mono size label matching the Studio's own ("4.1 kib", "1.2 mib"). */
export function formatKib(bytes: number): string {
  return bytes >= 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} mib` : `${(bytes / 1024).toFixed(1)} kib`;
}

/**
 * The head row's mono facts: "flipnote · 192×144 · 3 frames · 8 fps · 4.1 kib". Dimensions are
 * the validated constants because `decodePixa` rejects any other size, so a decoded package can
 * state them without re-reading a frame. Unknown parts (no package yet, no size) are left out
 * rather than shown as zero.
 */
export function viewerFacts(pixa: Pick<Pixa, "frames" | "fps"> | null, bytes: number): string {
  const parts = ["flipnote"];
  if (pixa) {
    const count = pixa.frames.length;
    parts.push(`${FLIPNOTE_W}×${FLIPNOTE_H}`, `${count} frame${count === 1 ? "" : "s"}`, `${pixa.fps} fps`);
  }
  if (Number.isFinite(bytes) && bytes > 0) parts.push(formatKib(bytes));
  return parts.join(" · ");
}

/** The right-aligned "by rowan · in #general · expires 30d" line; empty when nothing is known. */
export function viewerProvenance(author: string, place: string, expiry: string): string {
  const parts: string[] = [];
  if (author) parts.push(`by ${author}`);
  if (place) parts.push(`in ${place}`);
  if (expiry) parts.push(expiry);
  return parts.join(" · ");
}
