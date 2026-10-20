import assert from "node:assert/strict";
import test from "node:test";
import {
  collectTextEffectCss, frozenRules, isAnimated, parseTimes, referencedVariables, snapshotAt, snapshotSvg, type PreparedText,
} from "./text-fx-raster.ts";

const style = (selectorText: string, body = "color: red") => ({ type: 1, selectorText, cssText: `${selectorText} { ${body} }` });
const keyframes = (name: string) => ({ type: 7, name, cssText: `@keyframes ${name} { 0% { opacity: 0 } }` });
const media = { type: 4, cssText: "@media (prefers-reduced-motion: reduce) { .text-fx { animation: none } }" };

test("only effect rules and effect keyframes are collected, and grouping rules are left out", () => {
  const css = collectTextEffectCss([
    { cssRules: [
      style('[data-text-effects="full"] .text-fx-wave .text-fx-unit', "animation: text-fx-wave 1s"),
      style(".fx-rainbow"),
      style(".message .author"),
      style(".notfx-thing"),
      keyframes("text-fx-wave"),
      keyframes("pulse"),
      media,
    ] },
  ]);
  assert.match(css, /text-fx-wave \.text-fx-unit/);
  assert.match(css, /\.fx-rainbow/);
  assert.match(css, /@keyframes text-fx-wave/);
  assert.doesNotMatch(css, /author|notfx|@keyframes pulse|@media/);
});

test("a sheet that refuses to list its rules is skipped", () => {
  const locked = { get cssRules(): never { throw new Error("SecurityError"); } };
  assert.equal(collectTextEffectCss([locked, { cssRules: [style(".fx-neon")] }]), ".fx-neon { color: red }");
});

test("the variables the rules read are found, except the per-letter index", () => {
  assert.deepEqual(
    referencedVariables(".a{color:var(--accent);margin:var( --gap , 2px);transform:translateY(calc(var(--fx-i) * 2px))}"),
    ["--accent", "--gap"],
  );
});

test("computed delay lists are read in seconds", () => {
  assert.deepEqual(parseTimes("0.1s, 250ms,0s"), [0.1, 0.25, 0]);
  assert.deepEqual(parseTimes("junk"), [0]);
});

test("each animated part is frozen t seconds in, keeping its own stagger", () => {
  const rules = frozenRules([{ ref: 2, pseudo: "", delays: [0.08, 0] }, { ref: 5, pseudo: "::after", delays: [0] }], 0.5);
  assert.equal(rules,
    '[data-r="2"]{animation-delay:-0.4200s,-0.5000s!important;animation-play-state:paused!important}'
    + '[data-r="5"]::after{animation-delay:-0.5000s!important;animation-play-state:paused!important}');
  assert.match(frozenRules([{ ref: 0, pseudo: "", delays: [0.2] }], Number.NaN), /0\.2000s/, "a bad time freezes at the start");
});

const prepared: PreparedText = {
  markup: '<div xmlns="http://www.w3.org/1999/xhtml" class="tfx-root" data-r="0">A &amp; B</div>',
  css: '.fx-a::after { content: "<&>"; }',
  // A real resolved value: font stacks carry double quotes, which must not end the attribute.
  variables: '--accent:#977df2;--ui:"Segoe UI", system-ui;',
  timed: [{ ref: 0, pseudo: "", delays: [0] }],
  width: 120.4,
  height: 40,
  pad: 10,
};

test("the snapshot is well-formed SVG with the CSS escaped inside it", () => {
  const svg = snapshotSvg(prepared, 0.25);
  const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
  const parsed = new w.DOMParser().parseFromString(svg, "image/svg+xml");
  assert.equal(parsed.getElementsByTagName("parsererror").length, 0, "the XML parses");
  assert.equal(parsed.documentElement.getAttribute("width"), "121", "rounded up to whole pixels");
  const styleText = parsed.getElementsByTagName("style")[0].textContent ?? "";
  assert.match(styleText, /content: "<&>"/, "the parser gives the CSS back unescaped");
  assert.match(styleText, /animation-delay:-0\.2500s/);
  const frame = parsed.getElementsByTagName("div")[0];
  assert.match(frame.getAttribute("style") ?? "", /--ui:"Segoe UI", system-ui;/, "quotes in a variable survive inside the attribute");
});

test("snapshots are data URLs, never blob URLs (a blob-URL foreignObject taints the canvas)", () => {
  assert.match(snapshotAt(prepared, 0), /^data:image\/svg\+xml;charset=utf-8,/);
});

test("a still effect needs one snapshot; a moving one needs one per frame", () => {
  assert.equal(isAnimated(prepared), true);
  assert.equal(isAnimated({ ...prepared, timed: [] }), false);
});
