// Pure helpers for the right-click menu: where it opens, which reactions it surfaces first, and
// how typing moves focus inside it. No DOM here, so every rule is unit-tested away from Svelte.

export type MenuPlacement = { left: number; top: number; flipX: boolean; flipY: boolean };

/**
 * Where a menu of size w x h opens for a pointer at (x, y) inside a viewport of vw x vh.
 *
 * The menu hangs down and to the right of the pointer, its corner ON the click. When that would
 * run off an edge it mirrors to the other side of the pointer instead (so a flyout knows to mirror
 * too), and only if neither side fits is it clamped. Clamping is the last resort because it slides
 * the box over the pointer, which is what the old placement always did.
 */
export function placeMenu(x: number, y: number, w: number, h: number, vw: number, vh: number, margin = 4): MenuPlacement {
  const flipX = x + w + margin > vw && x - w >= margin;
  const flipY = y + h + margin > vh && y - h >= margin;
  const left = clamp(flipX ? x - w : x, margin, Math.max(margin, vw - w - margin));
  const top = clamp(flipY ? y - h : y, margin, Math.max(margin, vh - h - margin));
  return { left, top, flipX, flipY };
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

/** Reaction picks per emoji, as stored. Tolerant of anything that is not a plain object of counts. */
export function parseTally(raw: string | null): Record<string, number> {
  if (!raw) return {};
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const out: Record<string, number> = {};
    for (const [k, v] of Object.entries(parsed as Record<string, unknown>)) {
      if (typeof v === "number" && Number.isFinite(v) && v > 0 && k.length <= 64) out[k] = Math.floor(v);
    }
    return out;
  } catch {
    return {};
  }
}

/** One more pick of `emoji`. Returns a new object; the caller decides where it lives. */
export function tallyReaction(tally: Record<string, number>, emoji: string): Record<string, number> {
  return { ...tally, [emoji]: (tally[emoji] ?? 0) + 1 };
}

/**
 * The strip's emoji: the n most-picked, then the defaults in their own order until the strip is
 * full. Ties keep the defaults' order (a fresh install shows the defaults verbatim), and nothing
 * appears twice.
 */
export function topReactions(tally: Record<string, number>, defaults: string[], n: number): string[] {
  const rank = new Map(defaults.map((e, i) => [e, i]));
  const picked = Object.entries(tally)
    .filter(([, c]) => c > 0)
    .sort((a, b) => b[1] - a[1] || (rank.get(a[0]) ?? Infinity) - (rank.get(b[0]) ?? Infinity) || a[0].localeCompare(b[0]))
    .map(([e]) => e);
  const out: string[] = [];
  for (const e of [...picked, ...defaults]) {
    if (out.length >= n) break;
    if (!out.includes(e)) out.push(e);
  }
  return out;
}

/**
 * Typeahead: the first label starting with `buffer` (case-insensitive), searching from `from`
 * forward and wrapping, so repeated presses of one letter cycle through its matches. -1 when
 * nothing matches; an empty buffer matches nothing rather than everything.
 */
export function typeaheadIndex(labels: string[], buffer: string, from: number): number {
  const q = buffer.trim().toLowerCase();
  if (!q || labels.length === 0) return -1;
  const start = ((from % labels.length) + labels.length) % labels.length;
  for (let step = 1; step <= labels.length; step++) {
    const i = (start + step) % labels.length;
    if (labels[i].toLowerCase().startsWith(q)) return i;
  }
  return -1;
}

/** The verb a confirm prompt re-arms on: the first word of the question ("Delete this message?" -> "Delete"). */
export function confirmVerb(label: string): string {
  const word = label.trim().split(/\s+/)[0] ?? "";
  return word.replace(/[?.!]+$/, "") || "Confirm";
}

/** The question the confirm header shows: the label, ending in a question mark exactly once. */
export function confirmQuestion(label: string): string {
  const t = label.trim().replace(/[?.!]+$/, "");
  return `${t}?`;
}
