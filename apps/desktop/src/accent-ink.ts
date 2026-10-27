/**
 * Text colour for words drawn on a solid user-chosen colour: accent buttons and badges,
 * member-coloured avatars, instrument pills and lane labels. Text on the fixed semantic fills
 * (--ok/--warn/--danger) uses `--on-signal` in app.css instead, because those fills never change.
 *
 * Every preset pairs its accent with a near-black `--on-accent`, which is right for every preset
 * accent. The accent itself is swappable to any hex (personal appearance, a server's livery, the
 * livery editor's preview), and member colours are whatever hex a member picked, so a dark choice
 * (a deep red, a navy) used to leave near-black text on a near-black box.
 *
 * The rule: white wins whenever it out-contrasts the dark ink by the WCAG 2 contrast ratio. That
 * flips around a relative luminance of about 0.19, and every preset accent and name-palette colour
 * sits above it, so they keep their dark ink exactly as before; only genuinely dark choices change.
 */

/** Nightshade's ink on its accent. The other presets' inks differ from it imperceptibly. */
export const DARK_INK = "#131218";
/** The ink for a background too dark for {@link DARK_INK}. */
export const LIGHT_INK = "#ffffff";

const HEX6 = /^#([0-9a-f]{6})$/i;

/**
 * WCAG 2 relative luminance (0 black .. 1 white) of a `#rrggbb` colour, or null for anything else:
 * `var(...)` references, shorthand hex and unvalidated strings are not guessed at. A non-string is
 * null too, not a throw: the personal accent is read back from localStorage JSON without a type
 * check, and this runs inside the root theme effect, so a throw here would recur on every launch.
 */
export function relativeLuminance(color: string): number | null {
  if (typeof color !== "string") return null;
  const match = HEX6.exec(color.trim());
  if (!match) return null;
  const value = Number.parseInt(match[1], 16);
  const channel = (byte: number) => {
    const c = byte / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(value >> 16) + 0.7152 * channel((value >> 8) & 255) + 0.0722 * channel(value & 255);
}

const DARK_INK_LUMINANCE = relativeLuminance(DARK_INK) ?? 0;

/** True when `color` is a `#rrggbb` dark enough that white text reads better on it than {@link DARK_INK}. */
export function wantsLightInk(color: string): boolean {
  const l = relativeLuminance(color);
  if (l === null) return false;
  const onWhite = (1 + 0.05) / (l + 0.05);
  const onDark = (l + 0.05) / (DARK_INK_LUMINANCE + 0.05);
  return onWhite > onDark;
}

/**
 * The text colour for a box whose background is `background`. A `#rrggbb` gets {@link DARK_INK}
 * or {@link LIGHT_INK}; anything else gets `fallback`. The default fallback is the accent's own ink,
 * which is right only when the non-hex background is an accent token (`var(--accent)` and kin); a
 * caller whose fallback background is something else (a muted grey) must pass its own.
 */
export function inkOn(background: string, fallback = "var(--on-accent)"): string {
  if (relativeLuminance(background) === null) return fallback;
  return wantsLightInk(background) ? LIGHT_INK : DARK_INK;
}

/**
 * The inline `--on-accent` override for the accent actually painted: {@link LIGHT_INK} when that
 * accent is dark, otherwise "" meaning "keep the preset's own ink". An empty or non-hex accent is
 * the preset default, which never needs the override.
 */
export function onAccentOverride(accent: string): string {
  return wantsLightInk(accent) ? LIGHT_INK : "";
}
