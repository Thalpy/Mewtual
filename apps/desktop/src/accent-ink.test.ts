// Text on a solid user-chosen colour: a dark custom accent (or a dark member colour) used to keep
// the preset's near-black ink, leaving black text on a near-black button. The helper is tested
// directly; App.svelte cannot be mounted under node:test, so its theme effect and the livery
// preview's variables are run from their production text against a fake root element.
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import { fileURLToPath } from "node:url";
import { DARK_INK, LIGHT_INK, inkOn, onAccentOverride, relativeLuminance, wantsLightInk } from "./accent-ink.ts";

const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
const css = readFileSync(fileURLToPath(new URL("./app.css", import.meta.url)), "utf8");

test("luminance is read only from #rrggbb", () => {
  assert.equal(relativeLuminance("#000000"), 0);
  assert.equal(relativeLuminance("#ffffff"), 1);
  assert.equal(relativeLuminance(" #FFFFFF "), 1, "case and surrounding space do not matter");
  for (const other of ["", "#fff", "var(--accent)", "rgb(0 0 0)", "#12345g", "#1234567"]) {
    assert.equal(relativeLuminance(other), null, other);
  }
  // The saved accent is untyped JSON; a corrupt value must read as "no colour", never throw.
  for (const corrupt of [5, null, undefined, {}, ["#000000"]]) {
    assert.equal(relativeLuminance(corrupt as unknown as string), null, String(corrupt));
    assert.equal(onAccentOverride(corrupt as unknown as string), "");
  }
});

test("dark backgrounds get white text, light ones keep the dark ink", () => {
  // The reported case: a deep red accent behind an uppercase button label.
  for (const dark of ["#6b0f0f", "#5a0000", "#000000", "#1b2a4a", "#3d1f5c", "#0f3d1f"]) {
    assert.equal(wantsLightInk(dark), true, dark);
    assert.equal(inkOn(dark), LIGHT_INK, dark);
    assert.equal(onAccentOverride(dark), LIGHT_INK, dark);
  }
  for (const light of ["#ffffff", "#ffe45c", "#e0574b", "#977df2", "#4f8cff"]) {
    assert.equal(wantsLightInk(light), false, light);
    assert.equal(inkOn(light), DARK_INK, light);
    assert.equal(onAccentOverride(light), "", `${light} keeps the preset's own ink`);
  }
});

test("the flip sits where white and the dark ink contrast equally", () => {
  // Greys either side of the WCAG equal-contrast point against #131218 (luminance ~0.1935).
  assert.equal(wantsLightInk("#797979"), true);
  assert.equal(wantsLightInk("#7a7a7a"), false);
});

test("a background that is not a plain hex falls back rather than being guessed", () => {
  assert.equal(inkOn("var(--accent-hi)"), "var(--on-accent)");
  assert.equal(inkOn("var(--accent)", "inherit"), "inherit");
  assert.equal(inkOn(""), "var(--on-accent)");
  assert.equal(onAccentOverride(""), "", "no accent override means the preset default");
  assert.equal(onAccentOverride("not a colour"), "");
});

test("every shipped accent and name colour keeps its dark ink", () => {
  // Read from the production lists so a new preset or swatch is checked without editing this test.
  const list = (name: string) => {
    const match = new RegExp(String.raw`const ${name} = \[([^\]]*)\]`).exec(source);
    assert.ok(match, `${name} not found in App.svelte`);
    return [...match[1].matchAll(/#[0-9a-f]{6}/gi)].map((m) => m[0]);
  };
  const presetAccents = [...source.matchAll(/\{ id: "[a-z]*", name: "[^"]+", sw: "(#[0-9a-f]{6})"/gi)].map((m) => m[1]);
  const cssAccents = [...css.matchAll(/^\s*--accent: (#[0-9a-f]{6});/gim)].map((m) => m[1]);
  assert.ok(presetAccents.length >= 5 && cssAccents.length >= 5, "the preset accents were found");
  for (const color of [...list("ACCENT_CHOICES"), ...list("NAME_COLORS"), ...presetAccents, ...cssAccents]) {
    assert.equal(wantsLightInk(color), false, `${color} would change ink`);
  }
});

// --- App.svelte production text ------------------------------------------------------------------

/** The text from `start` through its matching close brace. */
function block(from: number): string {
  const open = source.indexOf("{", from);
  let depth = 0;
  for (let i = open; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) return source.slice(open, i + 1);
  }
  assert.fail("unbalanced braces");
}

/** A root element that records inline custom properties and attributes. */
function fakeRoot() {
  const props = new Map<string, string>();
  const attrs = new Map<string, string>();
  return {
    props,
    attrs,
    el: {
      style: {
        setProperty: (k: string, v: string) => void props.set(k, v),
        removeProperty: (k: string) => void props.delete(k),
        fontSize: "",
      },
      setAttribute: (k: string, v: string) => void attrs.set(k, v),
      removeAttribute: (k: string) => void attrs.delete(k),
    },
  };
}

const themeEffectAt = source.indexOf("// Apply the effective theme");
assert.ok(themeEffectAt > 0, "the theme effect was found");
const themeEffectBody = stripTypeScriptTypes(block(source.indexOf("$effect(() =>", themeEffectAt)), { mode: "strip" });
const LIVERY_TOKENS_TEXT = /const LIVERY_TOKENS = (\[[^\]]*\]);/.exec(source)?.[1] ?? "";
assert.ok(LIVERY_TOKENS_TEXT, "LIVERY_TOKENS was found");

type Livery = { preset: string; accent: string; tokens: Record<string, string> };
/** Runs the production theme effect once against `root` and returns the root's state. */
function applyTheme(root: ReturnType<typeof fakeRoot>, opts: { accent?: string; follow?: boolean; livery?: Partial<Livery> }) {
  const state = {
    document: { documentElement: root.el },
    matchMedia: () => ({ matches: false }),
    localStorage: { setItem() {} },
    APPEARANCE_KEY: "test",
    appearance: { preset: "", accent: opts.accent ?? "", density: "", chrome: "", motion: "", textEffects: "", scale: 100 },
    livery: { preset: "", accent: "", tokens: {}, ...opts.livery },
    followLiveryNow: opts.follow ?? false,
    liveryCursorUrl: "",
    LIVERY_TOKENS: new Function(`return ${LIVERY_TOKENS_TEXT}`)(),
    LIVERY_RADIUS: {},
    LIVERY_FONTS: {},
    onAccentOverride,
  };
  new Function("state", `with (state) { (() => ${themeEffectBody})(); }`)(state);
  return root;
}

test("the app's own dark accent turns accent text white, and a light one hands it back", () => {
  const root = fakeRoot();
  applyTheme(root, { accent: "#6b0f0f" });
  assert.equal(root.props.get("--accent"), "#6b0f0f");
  assert.equal(root.props.get("--on-accent"), LIGHT_INK);
  applyTheme(root, { accent: "#e2a83d" });
  assert.equal(root.props.has("--on-accent"), false, "a light accent clears the override so the preset's ink applies");
  applyTheme(root, { accent: "#6b0f0f" });
  applyTheme(root, {});
  assert.equal(root.props.has("--on-accent"), false, "the preset default accent needs no override");
  applyTheme(root, { accent: 5 as unknown as string });
  assert.equal(root.props.has("--on-accent"), false, "a corrupt saved accent is survived, not thrown on");
});

test("a followed livery's dark accent, from either field, turns accent text white", () => {
  const root = fakeRoot();
  applyTheme(root, { follow: true, livery: { accent: "#1b2a4a" } });
  assert.equal(root.props.get("--on-accent"), LIGHT_INK);
  applyTheme(root, { follow: true, livery: { tokens: { "--accent": "#1b2a4a" } } });
  assert.equal(root.props.get("--accent"), "#1b2a4a");
  assert.equal(root.props.get("--on-accent"), LIGHT_INK, "a token accent is the accent painted when no override is set");
  applyTheme(root, { follow: true, livery: { accent: "#e2a83d", tokens: { "--accent": "#1b2a4a" } } });
  assert.equal(root.props.has("--on-accent"), false, "the livery accent paints over its token, so its ink wins too");
  applyTheme(root, { follow: false, accent: "", livery: { accent: "#1b2a4a", tokens: { "--accent": "#1b2a4a" } } });
  assert.equal(root.props.has("--on-accent"), false, "an opted-out livery colours nothing, ink included");
});

function liveryDraftVars(draft: Livery): string {
  const text = stripTypeScriptTypes(source.slice(source.indexOf("function liveryDraftVars()"), source.indexOf("// Livery-draft token editing")), { mode: "strip" });
  const state = {
    liveryDraft: draft,
    PRESETS: [{ id: "", sw: "#977df2" }, { id: "aurum", sw: "#e2a83d" }],
    LIVERY_TOKENS: new Function(`return ${LIVERY_TOKENS_TEXT}`)(),
    HEX_COLOR: /^#[0-9a-fA-F]{6}$/,
    LIVERY_RADIUS: {},
    LIVERY_FONTS: {},
    onAccentOverride,
    DARK_INK,
  };
  return new Function("state", `with (state) { ${text} return liveryDraftVars(); }`)(state);
}

test("the livery preview's accent text follows the accent it paints", () => {
  assert.match(liveryDraftVars({ preset: "aurum", accent: "#6b0f0f", tokens: {} }), /--on-accent:#ffffff/);
  assert.doesNotMatch(liveryDraftVars({ preset: "aurum", accent: "#e2a83d", tokens: {} }), /--on-accent/, "a preset preview keeps its preset's ink");
  assert.match(
    liveryDraftVars({ preset: "", accent: "#e2a83d", tokens: {} }),
    new RegExp(`--on-accent:${DARK_INK}`),
    "a Nightshade preview pins its ink instead of inheriting the root's override",
  );
  const tokenWins = liveryDraftVars({ preset: "aurum", accent: "#e2a83d", tokens: { "--accent": "#1b2a4a" } });
  assert.ok(tokenWins.indexOf("--accent:#1b2a4a") > tokenWins.indexOf("--accent:#e2a83d"), "the token accent is the one painted");
  assert.match(tokenWins, /--on-accent:#ffffff/);
});

test("text on a member's colour takes its ink from that colour", () => {
  assert.doesNotMatch(source, /color:#131218/, "no box hard-codes the dark ink over a member colour");
  // Every avatar-fallback tag that paints a background, in either attribute form, inks it too.
  const fallbacks = [...source.matchAll(/<span class="avatar[^"]*fallback[^>]*>/g)].map((m) => m[0]).filter((tag) => tag.includes("background"));
  assert.ok(fallbacks.length >= 8, `the avatar fallbacks were found (${fallbacks.length})`);
  for (const tag of fallbacks) assert.match(tag, /style=\{[^\n]*background:\$\{([^}]+)\};color:\$\{inkOn\(\1\)\}/, tag);
  // Swatches and dots carry no text; every instrument box that sets a text colour derives it.
  const inked = [...source.matchAll(/background:\$\{instColor\(([^)]+)\)\}[^`]*`/g)].filter((m) => m[0].includes(";color:"));
  assert.ok(inked.length >= 4, `the instrument pills and keys were found (${inked.length})`);
  for (const inst of inked) assert.ok(inst[0].includes(`color:\${inkOn(instColor(${inst[1]}))}`), inst[0]);
  assert.match(css, /\.piano-key\.sharp\.held \{[^}]*color: var\(--on-accent\);/, "a held key's label uses the accent's ink");
});

/** The declarations of the first rule whose selector list is exactly `selector`. */
function rule(selector: string): string {
  const at = css.indexOf(`\n${selector} {`);
  assert.ok(at >= 0, `${selector} not found in app.css`);
  return css.slice(css.indexOf("{", at) + 1, css.indexOf("}", at));
}

test("a reaction chip repaints its text along with its background, hovered too", () => {
  // The chip is a <button> on --panel; the global button ink is the accent's, which is near-black
  // for every preset accent, so the count was invisible until the chip carried its own colour.
  assert.match(rule(".reaction"), /background: var\(--panel\);[^]*color: var\(--text\);/);
  // `button:hover` out-specifies `.reaction` and would put that --text ink on --accent-hi.
  assert.match(rule(".reaction:hover"), /background: var\(--panel\);/);
  assert.ok(css.indexOf("\n.reaction.mine {") > css.indexOf("\n.reaction:hover {"), "a chip of mine keeps its tint on hover");
});

test("a key pressed with the mouse keeps its lit ground under its held ink", () => {
  assert.match(css, /\n\.piano-key:hover:not\(\.sharp\):not\(\.held\) \{ background: #fffefa; \}/);
  assert.doesNotMatch(css, /\n\.piano-key:hover:not\(\.sharp\) \{/);
});

test("text on a fixed signal colour does not follow the accent's ink", () => {
  // --on-accent turns white under a dark accent; on gold or red that sank moderation badges and
  // Leave buttons. Every rule with a solid semantic fill and a text colour must use --on-signal.
  const blocks = css.split("}").filter((b) => /background: var\(--(ok|warn|danger|danger-hi)\);/.test(b));
  const inked = blocks.filter((b) => /(^|[;{\s])color:/.test(b.slice(b.indexOf("{"))));
  assert.ok(inked.length >= 5, `the inked signal fills were found (${inked.length})`);
  for (const b of inked) assert.doesNotMatch(b, /color: var\(--on-accent\)/, b.trim().slice(0, 80));
  assert.match(rule(".call-hangup"), /color: var\(--on-signal\);/, "a bare <button> on danger must pin its ink");
  assert.match(css, /:root \{[^}]*--on-signal: #131218;/, "the signal ink is a fixed root token");
  assert.ok(!LIVERY_TOKENS_TEXT.includes("--on-signal"), "a livery cannot set it");
});

test("a Studio sound lane inks its pill from the author's colour, dark on the muted fallback", () => {
  const studio = readFileSync(fileURLToPath(new URL("./Studio.svelte", import.meta.url)), "utf8");
  const line = /\n {2}(function tintInk\([^\n]*)/.exec(studio)?.[1];
  assert.ok(line, "tintInk was found");
  const tintInk = new Function("state", `with (state) { return (${stripTypeScriptTypes(line, { mode: "strip" })}); }`)({
    me: "me",
    colorOf: (id: string) => ({ navy: "#1b2a4a", gold: "#e2a83d" } as Record<string, string>)[id] ?? "var(--muted)",
    inkOn,
    DARK_INK,
  }) as (id: string) => string;
  assert.equal(tintInk("me"), "var(--on-accent)");
  assert.equal(tintInk("navy"), LIGHT_INK);
  assert.equal(tintInk("gold"), DARK_INK);
  assert.equal(tintInk("nobody"), DARK_INK, "the muted grey is not an accent, so the accent's ink would be wrong");
  assert.match(studio, /<i class="pill" style="background: \{tint\(lane\.author\)\}; color: \{tintInk\(lane\.author\)\}">/);
});
