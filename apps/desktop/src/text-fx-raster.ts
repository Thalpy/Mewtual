/**
 * Draws a text overlay, in one of the app's text effects, exactly as the effect looks at time t.
 *
 * The 47 text effects are HTML plus CSS keyframes (`message-effects.ts` builds the markup, app.css
 * animates it). There is no canvas version of any of them, and writing 47 by hand would drift from
 * chat. Instead each video frame takes a SNAPSHOT of the real markup and the real rules, frozen at
 * time t, as an SVG `<foreignObject>` image, and the compositor draws that image.
 *
 * How the freeze works: every animated element (and `::before`/`::after`) gets its computed delays
 * replaced by `delay - t` with the animation paused. A paused animation with a negative delay shows
 * the frame `t` seconds in, staggered letters included, because each letter keeps its own delay.
 *
 * Two parts, because the delays only exist once a browser has laid the effect out:
 *
 * 1. `prepareText` (once per overlay, in the live document): builds the markup in an off-screen
 *    shadow root that carries the collected effect rules as its own stylesheet, mirrors what the
 *    app's runtime does to effect markup (`--fx-i` letter numbering; `fx-play` on one-shot
 *    entrances, which in a clip start when the text appears), numbers every element, and reads each
 *    one's computed animation delays and the box size.
 * 2. `snapshotAt` (per frame): a self-contained SVG data URL of that markup with the frozen delays.
 *
 * Why these particular choices, each found the hard way:
 * - A `data:` URL, never a `blob:` one: Chromium taints a canvas that draws a blob-URL SVG holding a
 *   foreignObject, and a tainted canvas cannot become a video frame. A data URL does not taint.
 * - A shadow root with `adoptedStyleSheets`: the shipped CSP refuses runtime `<style>` elements, but
 *   constructed stylesheets are CSSOM and allowed. The shadow root also keeps the person's own
 *   effect setting ("static", "off", reduced motion) from changing what the clip carries: the rules
 *   inside it are keyed on a wrapper that always says `data-text-effects="full"`.
 * - `@media` blocks are left out for the same reason: reduced motion is the sender's local
 *   preference, not part of the clip.
 *
 * Fonts are the system's (the app bundles none), so a clip shows the fonts of the machine that made
 * it; recipients see pixels, not fonts.
 */
import { ONE_SHOT_EFFECT_IDS, textEffectHtml } from "./message-effects.ts";

/** Room around the text for glows, shakes and sparkles that draw outside the letters, in em. */
export const SNAPSHOT_PAD_EM = 0.6;
/** Line height text is laid out with. The clip renderer sizes text by it: one line is `scale` tall. */
export const TEXT_LINE_HEIGHT = 1.15;

/** The minimal shape of a CSS rule this module reads (real `CSSRule`s, or plain objects in tests). */
export type RuleLike = {
  readonly type: number;
  readonly cssText: string;
  readonly selectorText?: string;
  readonly name?: string;
};
type SheetLike = { readonly cssRules: ArrayLike<RuleLike> };

const STYLE_RULE = 1;
const KEYFRAMES_RULE = 7;

/**
 * The rules that draw text effects, from the app's own stylesheets: every style rule whose
 * selector names an effect class (`text-fx…` or `fx-…`), and every `@keyframes text-fx…`/`fx-…`.
 * Grouping rules (`@media`, `@supports`) are skipped on purpose; see the module header. A sheet
 * whose rules cannot be read (cross-origin) is skipped rather than failing the export.
 */
export function collectTextEffectCss(sheets: Iterable<SheetLike>): string {
  const out: string[] = [];
  for (const sheet of sheets) {
    let rules: ArrayLike<RuleLike>;
    try { rules = sheet.cssRules; } catch { continue; }
    for (let i = 0; i < rules.length; i++) {
      const rule = rules[i];
      if (rule.type === STYLE_RULE && /(^|[^\w-])(text-fx|fx-)/.test(rule.selectorText ?? "")) out.push(rule.cssText);
      else if (rule.type === KEYFRAMES_RULE && /^(text-fx|fx-)/.test(rule.name ?? "")) out.push(rule.cssText);
    }
  }
  return out.join("\n");
}

/** Custom properties the collected rules read, except the per-letter `--fx-i` set on each letter. */
export function referencedVariables(css: string): string[] {
  const names = new Set<string>();
  for (const m of css.matchAll(/var\(\s*(--[\w-]+)/g)) if (m[1] !== "--fx-i") names.add(m[1]);
  return [...names].sort();
}

/** `"0.1s, 250ms"` to seconds. Anything unreadable counts as no delay. */
export function parseTimes(list: string): number[] {
  return list.split(",").map((part) => {
    const s = part.trim();
    const n = Number.parseFloat(s);
    if (!Number.isFinite(n)) return 0;
    return s.endsWith("ms") ? n / 1000 : n;
  });
}

/** One animated element (or pseudo-element) and the delays its animations start with. */
export type TimedPart = { readonly ref: number; readonly pseudo: "" | "::before" | "::after"; readonly delays: readonly number[] };

/** Everything needed to draw one text overlay at any time. Plain data: safe to keep and reuse. */
export type PreparedText = {
  /** XHTML of the effect, every element numbered with `data-r`. */
  readonly markup: string;
  /** The collected effect rules. */
  readonly css: string;
  /** `--name:value;` declarations the rules read, resolved from the live document. */
  readonly variables: string;
  readonly timed: readonly TimedPart[];
  /** CSS px, padding included. */
  readonly width: number;
  readonly height: number;
  readonly pad: number;
};

/** The per-frame rules: every animated part frozen `t` seconds in. */
export function frozenRules(timed: readonly TimedPart[], t: number): string {
  const at = Number.isFinite(t) && t > 0 ? t : 0;
  return timed.map((part) => {
    const delays = part.delays.map((d) => `${(d - at).toFixed(4)}s`).join(",");
    return `[data-r="${part.ref}"]${part.pseudo}{animation-delay:${delays}!important;animation-play-state:paused!important}`;
  }).join("");
}

/** Text for an XML text node: `<` and `&` escaped, so CSS can sit inside the SVG's `<style>`. */
function xmlText(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
}

/** A double-quoted XML attribute value. Variable values carry quotes (`"Segoe UI"`). */
function xmlAttr(s: string): string {
  return xmlText(s).replace(/"/g, "&quot;");
}

/** The SVG document for time `t`. */
export function snapshotSvg(p: PreparedText, t: number): string {
  const w = Math.max(1, Math.ceil(p.width));
  const h = Math.max(1, Math.ceil(p.height));
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><foreignObject width="100%" height="100%">`
    + `<div xmlns="http://www.w3.org/1999/xhtml" class="tfx-frame" data-text-effects="full" style="${xmlAttr(`padding:${p.pad}px;${p.variables}`)}">`
    + `<style>${xmlText(p.css + frozenRules(p.timed, t))}</style>${p.markup}</div></foreignObject></svg>`;
}

/** The snapshot for time `t` as a data URL (never a blob URL: see the header). */
export function snapshotAt(p: PreparedText, t: number): string {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(snapshotSvg(p, t))}`;
}

/** Whether the effect moves at all; a still one needs a single snapshot for the whole clip. */
export function isAnimated(p: PreparedText): boolean {
  return p.timed.length > 0;
}

export type TextLook = {
  readonly text: string;
  readonly effect: string | null;
  readonly color: string;
  /** Font size in CSS px: the overlay's scale times the frame height. */
  readonly fontPx: number;
};

/** The font the overlays use: the app's UI face, heavy, so text reads over moving video. */
export const OVERLAY_FONT = `800 1em "Segoe UI", system-ui, -apple-system, sans-serif`;

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/**
 * Lay the effect out once in the live document and record what each frame needs (see the header).
 * Browser-only. The off-screen host is removed before returning.
 */
export function prepareText(doc: Document, css: string, look: TextLook): PreparedText {
  const host = doc.createElement("div");
  // Off-screen, outside <body>: the app's effect runtime watches <body> and must not adopt this
  // copy (it would set it up for sound). Styles go through CSSOM, which the CSP allows.
  for (const [k, v] of [["position", "fixed"], ["left", "-100000px"], ["top", "0"], ["pointer-events", "none"], ["contain", "layout style"]]) {
    host.style.setProperty(k, v);
  }
  doc.documentElement.append(host);
  try {
    const shadow = host.attachShadow({ mode: "open" });
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(css);
    shadow.adoptedStyleSheets = [sheet];
    const root = doc.createElement("div");
    root.className = "tfx-root";
    root.setAttribute("data-text-effects", "full");
    root.style.setProperty("display", "inline-block");
    root.style.setProperty("white-space", "pre");
    root.style.setProperty("font", OVERLAY_FONT);
    root.style.setProperty("font-size", `${look.fontPx}px`);
    root.style.setProperty("line-height", String(TEXT_LINE_HEIGHT));
    root.style.setProperty("color", look.color);
    // textEffectHtml escapes the words itself; plain text is escaped here.
    root.innerHTML = look.effect ? textEffectHtml(look.effect, look.text) : escapeHtml(look.text);
    // What the app's runtime does to effect markup in chat.
    for (const effect of root.querySelectorAll<HTMLElement>("[data-text-fx]")) {
      effect.querySelectorAll<HTMLElement>(".text-fx-unit").forEach((unit, index) => unit.style.setProperty("--fx-i", String(index)));
      if (ONE_SHOT_EFFECT_IDS.includes(effect.dataset.textFx ?? "")) effect.classList.add("fx-play");
    }
    shadow.append(root);
    const view = doc.defaultView!;
    const elements = [root, ...root.querySelectorAll<HTMLElement>("*")];
    const timed: TimedPart[] = [];
    elements.forEach((el, ref) => {
      el.setAttribute("data-r", String(ref));
      for (const pseudo of ["", "::before", "::after"] as const) {
        const cs = view.getComputedStyle(el, pseudo || null);
        if (pseudo && (cs.content === "none" || cs.content === "normal")) continue;
        if (!cs.animationName || cs.animationName === "none") continue;
        timed.push({ ref, pseudo, delays: parseTimes(cs.animationDelay) });
      }
    });
    const rect = root.getBoundingClientRect();
    const pad = Math.ceil(look.fontPx * SNAPSHOT_PAD_EM);
    const rootStyle = view.getComputedStyle(doc.documentElement);
    const variables = referencedVariables(css)
      .map((name) => [name, rootStyle.getPropertyValue(name).trim()] as const)
      .filter(([, value]) => value)
      .map(([name, value]) => `${name}:${value};`)
      .join("");
    // XML, not HTML, serialisation: the markup goes inside an SVG document, where void elements
    // must be closed and the XHTML namespace declared.
    const markup = new XMLSerializer().serializeToString(root);
    return { markup, css, variables, timed, width: rect.width + pad * 2, height: rect.height + pad * 2, pad };
  } finally {
    host.remove();
  }
}

/** Decode a snapshot into something `drawImage` takes. Browser-only. */
export async function snapshotImage(p: PreparedText, t: number): Promise<HTMLImageElement> {
  const img = new Image();
  img.src = snapshotAt(p, t);
  await img.decode();
  return img;
}
