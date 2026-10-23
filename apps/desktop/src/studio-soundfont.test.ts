// Tests for the soundfont editor dialog and its pure helpers. The component is mounted for real
// against the jsdom document the preloads install; events are dispatched as a browser would
// dispatch them (bubbling, so Svelte's delegated root listener sees them).
import assert from "node:assert/strict";
import test from "node:test";
import type { JamPatch } from "./jam-contract.ts";
import { JAM_ENV_OFF, JAM_FILTER_OFF, JAM_FILTER_ON } from "./jam-editor.ts";
import { validateJamPatch } from "./jam-patch.ts";
import {
  blankSoundfontPatch,
  HOME_ROW_SEMITONES,
  importName,
  importSoundfontText,
  knobArc,
  knobDragValue,
  knobStep,
  oscSummary,
  PIANO_BLACK_KEYS,
  PIANO_WHITE_KEYS,
  recipeKey,
  registerBase,
  registerLabel,
} from "./studio-soundfont.ts";
import { deferred, settle } from "./studio-testkit.ts";

const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
// What Svelte's client runtime reaches for beyond what the DOM preload installs: the observer
// behind bindings, the input and media constructors its event helpers identity-check against.
Object.assign(globalThis, { MutationObserver: w.MutationObserver, HTMLInputElement: w.HTMLInputElement, HTMLMediaElement: w.HTMLMediaElement, Event: w.Event });
const { mount, unmount, flushSync } = await import("svelte");
const { default: StudioSoundfont } = await import("./StudioSoundfont.svelte");

// The call drawer's two presets, verbatim, so the shelf under test is the shelf a user sees.
const LANDING: JamPatch = { v: 1, o: [{ w: 3, t: 0, c: -8, l: 84 }, { w: 3, t: 0, c: 8, l: 84 }, { w: 0, t: -12, c: 0, l: 46 }], e: { a: 1200, d: 400, s: 62, r: 4800 }, f: { m: 0, c: 1800, q: 22, e: 56 }, l: { r: 40, d: 30, t: 1 }, x: { c: 55, d: 18, r: 72 } };
const DUSK: JamPatch = { v: 1, o: [{ w: 2, t: 0, c: -6, l: 70 }, { w: 1, t: 12, c: 6, l: 40 }], e: { a: 15, d: 260, s: 45, r: 900 }, f: { m: 0, c: 2600, q: 35, e: 30 }, l: { r: 500, d: 18, t: 1 }, x: { c: 20, d: 45, r: 35 } };
const clone = (p: JamPatch): JamPatch => JSON.parse(JSON.stringify(p));

type Log = { audition: [JamPatch, number][]; use: [string, JamPatch][]; save: [string, JamPatch][]; share: [string, JamPatch][]; notices: [string, string][]; closes: number };

/** Mount the dialog with recording callbacks; `props` overrides any of them. */
function harness(props: Record<string, unknown> = {}) {
  const target = document.createElement("div");
  document.body.append(target);
  const log: Log = { audition: [], use: [], save: [], share: [], notices: [], closes: 0 };
  const app = mount(StudioSoundfont, { target, props: {
    initial: LANDING, startFrom: [{ name: "LANDING", patch: LANDING }, { name: "DUSK", patch: DUSK }], targetName: "moon cat", recipeCount: 3, recipeMax: 64, libraryCount: 3,
    onaudition: (p: JamPatch, n: number) => { log.audition.push([p, n]); },
    onuse: async (n: string, p: JamPatch) => { log.use.push([n, p]); },
    onsave: async (n: string, p: JamPatch) => { log.save.push([n, p]); },
    onshare: async (n: string, p: JamPatch) => { log.share.push([n, p]); },
    onnotice: (t: string, k: string) => { log.notices.push([t, k]); },
    onclose: () => { log.closes += 1; },
    ...props,
  } });
  flushSync();
  const dialog = target.querySelector<HTMLElement>('[role="dialog"]')!;
  const knob = (name: string) => target.querySelector<HTMLElement>(`[role="slider"][aria-label="${name}"]`)!;
  const value = (name: string) => Number(knob(name).getAttribute("aria-valuenow"));
  const button = (text: string, within: ParentNode = target) => [...within.querySelectorAll("button")].find((b) => b.textContent!.trim().startsWith(text))!;
  const modes = (label: string) => target.querySelector<HTMLElement>(`[role="group"][aria-label="${label}"]`)!;
  const edited = () => [...target.querySelectorAll(".micro")].some((el) => el.textContent!.includes("edited"));
  const nameInput = target.querySelector<HTMLInputElement>('input[aria-label="Soundfont name"]')!;
  const registerUp = () => target.querySelector<HTMLButtonElement>('[aria-label="Register up"]')!;
  return { target, app, log, dialog, knob, value, button, modes, edited, nameInput, registerUp, async done() { await unmount(app); target.remove(); } };
}
const key = (el: EventTarget, k: string, init: KeyboardEventInit = {}) => el.dispatchEvent(new w.KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init }));
const press = (el: EventTarget) => el.dispatchEvent(new w.PointerEvent("pointerdown", { bubbles: true, cancelable: true, pointerId: 1 }));
async function pickFile(target: ParentNode, name: string, text: string, size = 100) {
  const file = target.querySelector<HTMLInputElement>('input[type="file"]')!;
  Object.defineProperty(file, "files", { configurable: true, value: [{ name, size, text: async () => text }] });
  file.dispatchEvent(new w.Event("change", { bubbles: true }));
  await settle();
  flushSync();
}

// --- Pure helpers --------------------------------------------------------------------------------

test("the blank start tile is a valid jam-patch:v1 with every bypass at its neutral setting", () => {
  const blank = blankSoundfontPatch();
  assert.equal(validateJamPatch(blank).ok, true);
  assert.deepEqual(blank.f, JAM_FILTER_OFF);
  assert.deepEqual(blank.x, { c: 0, d: 0, r: 0 });
  assert.equal(blank.l.t, 0);
  assert.notEqual(blankSoundfontPatch().o, blank.o, "each call is a fresh object");
  assert.equal(recipeKey(blank), validateJamPatch(blank).ok && validateJamPatch(blank).canonical);
});

test("knob geometry: the tick sweeps -135..135, the arc starts at the rest position, and steps clamp", () => {
  assert.equal(knobArc(0, 0, 100, false).deg, -135);
  assert.equal(knobArc(100, 0, 100, false).deg, 135);
  assert.equal(knobArc(50, 0, 100, false).deg, 0);
  assert.equal(knobArc(0, -24, 24, true).deg, 0);
  // A bipolar knob at rest draws a zero-length arc from twelve o'clock; a unipolar one from the left stop.
  assert.match(knobArc(0, -24, 24, true).d, /^M16\.00 3\.00 A13 13 0 0 0 16\.00 3\.00$/);
  assert.match(knobArc(0, 0, 100, false).d, /^M6\.81 25\.19 A13 13 0 0 0 6\.81 25\.19$/);
  // Beyond the bound the tick stops at the stop rather than wrapping.
  assert.equal(knobArc(200, 0, 100, false).deg, 135);
  assert.equal(knobStep(62, 0, 100, 1, false), 64, "a sixtieth of 100 rounds to 2");
  assert.equal(knobStep(62, 0, 100, 1, true), 63, "Shift steps by one");
  assert.equal(knobStep(0, -24, 24, -1, false), -1, "a range of 48 still steps by one");
  assert.equal(knobStep(100, 0, 100, 1, false), 100);
  assert.equal(knobStep(0, 0, 100, -1, false), 0);
  assert.equal(knobDragValue(50, 0, 100, 170, false), 100, "170px is the full sweep");
  assert.equal(knobDragValue(50, 0, 100, -85, false), 0, "and it clamps");
  assert.equal(knobDragValue(50, 0, 100, 120, true), 60, "Shift: 1200px is the full sweep");
});

test("the home row covers seventeen semitones in piano order and the keyboard spans two octaves", () => {
  assert.equal(Object.keys(HOME_ROW_SEMITONES).length, 17);
  assert.equal(HOME_ROW_SEMITONES.a, 0);
  assert.equal(HOME_ROW_SEMITONES.k, 12);
  assert.equal(HOME_ROW_SEMITONES[";"], 16);
  assert.deepEqual(new Set(Object.values(HOME_ROW_SEMITONES)), new Set(Array.from({ length: 17 }, (_u, i) => i)));
  assert.equal(PIANO_WHITE_KEYS.length, 14);
  assert.equal(PIANO_BLACK_KEYS.length, 10);
  assert.deepEqual(PIANO_BLACK_KEYS.map((k) => k.semitone), [1, 3, 6, 8, 10, 13, 15, 18, 20, 22]);
  assert.equal(PIANO_BLACK_KEYS[0].left, 5.29, "C# is centred on the seam after the first white key");
  assert.equal(PIANO_BLACK_KEYS[9].left, 91.01, "A# of the second octave sits on the seam after white key 13");
  assert.equal(registerBase(4), 60, "C4 is MIDI 60");
  assert.equal(registerLabel(4), "C4–C5");
  assert.equal(registerBase(9), registerBase(7), "registers clamp to 1..7");
  assert.equal(registerBase(7) + 23, 119, "the top key never passes MIDI 127");
});

test("oscSummary names the wave and only the offsets that are not zero", () => {
  assert.equal(oscSummary({ w: 3, t: 0, c: -8, l: 84 }), "SAW · -8 ct");
  assert.equal(oscSummary({ w: 0, t: -12, c: 0, l: 46 }), "SIN · -12 st");
  assert.equal(oscSummary({ w: 1, t: 7, c: 3, l: 1 }), "TRI · +7 st · +3 ct");
  assert.equal(oscSummary({ w: 9, t: 0, c: 0, l: 1 }), "SIN", "an out-of-range wave draws as a sine");
});

test("importSoundfontText decodes by shape and reports the right decoder's error", () => {
  const plain = importSoundfontText(JSON.stringify(DUSK));
  assert.ok(plain.ok && plain.notice === null);
  assert.deepEqual(plain.ok && plain.patch, DUSK);
  const emoji = importSoundfontText(JSON.stringify({ v: 1, img: "a".repeat(64), patch: DUSK, lo: 70, hi: 75, len: 333 }));
  assert.ok(emoji.ok && emoji.notice !== null && /pitch range/.test(emoji.notice));
  assert.deepEqual(emoji.ok && emoji.patch, DUSK);
  assert.deepEqual(importSoundfontText("not json"), { ok: false, error: "file is not valid JSON" });
  const brokenSidecar = importSoundfontText(JSON.stringify({ v: 1, img: "zz", patch: DUSK, lo: 70, hi: 75, len: 333 }));
  assert.ok(!brokenSidecar.ok && /emoji sound/.test(brokenSidecar.error), "a sidecar's own decoder explains a sidecar");
  const outOfRange = importSoundfontText(JSON.stringify({ ...DUSK, e: { ...DUSK.e, a: 99_999 } }));
  assert.ok(!outOfRange.ok && /envelope/.test(outOfRange.error));
  const big = importSoundfontText(`"${"x".repeat(5000)}"`);
  assert.ok(!big.ok && /larger/.test(big.error));
  assert.equal(importName("C:\\sounds\\meow purr.jampatch"), "meow purr");
  assert.equal(importName("a-very-long-soundfont-name.json"), "a-very-long-");
});

// --- The dialog ---------------------------------------------------------------------------------

test("a start-from tile replaces the working patch, resets the chip, and names an unnamed sheet", async () => {
  const h = harness({ name: "" });
  try {
    assert.equal(h.edited(), false, "nothing is edited on arrival");
    assert.equal(h.value("Attack"), 1200);
    key(h.knob("Attack"), "ArrowUp");
    flushSync();
    assert.equal(h.value("Attack"), 1283);
    assert.equal(h.edited(), true);
    h.target.querySelector<HTMLButtonElement>('[title="Start from LANDING"]')!.click();
    flushSync();
    assert.equal(h.value("Attack"), 1200);
    assert.equal(h.edited(), false, "back on the opening recipe, byte for byte");
    assert.equal(h.nameInput.value, "LANDING", "an empty name adopts the tile's");
    h.target.querySelector<HTMLButtonElement>('[title="Start from DUSK"]')!.click();
    flushSync();
    assert.equal(h.value("Attack"), 15);
    assert.equal(h.edited(), true);
    assert.equal(h.nameInput.value, "LANDING", "a name already there is kept");
    assert.ok(h.target.querySelector('[title="Start from DUSK"]')!.classList.contains("on"), "the matching tile lights");
    assert.deepEqual(h.log.notices, []);
  } finally { await h.done(); }
});

test("a knob's arrow keys step the value and clamp at both bounds", async () => {
  const h = harness();
  try {
    const sustain = h.knob("Sustain");
    assert.equal(h.value("Sustain"), 62);
    key(sustain, "ArrowUp"); flushSync();
    assert.equal(h.value("Sustain"), 64);
    key(sustain, "ArrowDown", { shiftKey: true }); flushSync();
    assert.equal(h.value("Sustain"), 63, "Shift is the fine step");
    // Each press is its own event in a browser, with a flush between: a layer knob reads its value
    // through the each-block item, which only moves on flush, so the test flushes per press too.
    for (let i = 0; i < 40; i += 1) { key(sustain, "ArrowUp"); flushSync(); }
    assert.equal(h.value("Sustain"), 100, "clamped at the top");
    assert.equal(sustain.getAttribute("aria-valuemax"), "100");
    const transpose = h.knob("Transpose");
    for (let i = 0; i < 40; i += 1) { key(transpose, "ArrowLeft"); flushSync(); }
    assert.equal(h.value("Transpose"), -24, "clamped at the bottom");
    assert.equal(h.target.querySelector(".sf-sum")!.textContent, "SAW · -24 st · -8 ct", "the folded summary follows");
  } finally { await h.done(); }
});

test("envelope OFF writes the gate values and OFF again restores what was stashed", async () => {
  const h = harness();
  try {
    const off = h.button("OFF", h.modes("Envelope"));
    assert.equal(off.getAttribute("aria-pressed"), "false");
    off.click(); flushSync();
    assert.equal(off.getAttribute("aria-pressed"), "true");
    assert.deepEqual({ a: h.value("Attack"), d: h.value("Decay"), s: h.value("Sustain"), r: h.value("Release") }, JAM_ENV_OFF);
    off.click(); flushSync();
    assert.equal(off.getAttribute("aria-pressed"), "false");
    assert.deepEqual({ a: h.value("Attack"), d: h.value("Decay"), s: h.value("Sustain"), r: h.value("Release") }, LANDING.e);
    assert.equal(h.edited(), false, "a round trip through OFF is not an edit");
    // Swapping in a whole recipe drops the stash: LANDING's envelope must not come back into DUSK.
    off.click(); flushSync();
    h.target.querySelector<HTMLButtonElement>('[title="Start from DUSK"]')!.click(); flushSync();
    h.button("OFF", h.modes("Envelope")).click(); flushSync();
    h.button("OFF", h.modes("Envelope")).click(); flushSync();
    assert.equal(h.value("Attack"), DUSK.e.a, "the stash belongs to the recipe it was taken from");
  } finally { await h.done(); }
});

test("from WIDE, choosing LP turns the filter back on as a lowpass with the restore values", async () => {
  const h = harness({ initial: { ...clone(LANDING), f: { ...JAM_FILTER_OFF } } });
  try {
    const wide = h.button("WIDE", h.modes("Filter")), lp = h.button("LP", h.modes("Filter"));
    assert.equal(wide.getAttribute("aria-pressed"), "true");
    assert.equal(lp.getAttribute("aria-pressed"), "false", "WIDE is mode 0 too, but it is not LP");
    lp.click(); flushSync();
    assert.equal(wide.getAttribute("aria-pressed"), "false");
    assert.equal(lp.getAttribute("aria-pressed"), "true");
    assert.equal(h.value("Cutoff"), JAM_FILTER_ON.c);
    h.button("use in moon cat").click();
    await settle();
    assert.equal(h.log.use.length, 1);
    assert.deepEqual(h.log.use[0][1].f, { ...JAM_FILTER_ON, m: 0 });
  } finally { await h.done(); }
});

test("a piano key plays the right MIDI note and the register stepper shifts it by an octave", async () => {
  const h = harness();
  try {
    assert.deepEqual(h.log.audition, [], "nothing plays on mount");
    press(h.target.querySelector('[aria-label="Play C4"]')!);
    assert.equal(h.log.audition.length, 1);
    assert.equal(h.log.audition[0][1], 60);
    assert.deepEqual(h.log.audition[0][0], LANDING);
    assert.notEqual(h.log.audition[0][0], LANDING, "a clone, never the working state");
    press(h.target.querySelector('[aria-label="Play F♯4"]')!);
    assert.equal(h.log.audition[1][1], 66, "black keys too");
    h.registerUp().click(); flushSync();
    assert.match(h.target.textContent!, /C5–C6/);
    assert.equal(h.target.querySelector('[aria-label="Play C4"]'), null, "the keys are relabelled");
    press(h.target.querySelector('[aria-label="Play C5"]')!);
    assert.equal(h.log.audition[2][1], 72);
    for (let i = 0; i < 9; i += 1) h.registerUp().click();
    flushSync();
    assert.match(h.target.textContent!, /C7–C8/);
    assert.equal(h.registerUp().disabled, true, "registers stop at 7");
    press(h.target.querySelector('[aria-label="Play B8"]')!);
    assert.equal(h.log.audition[3][1], 119);
    key(h.knob("Attack"), "ArrowUp"); flushSync();
    assert.equal(h.log.audition.length, 4, "a knob change never plays by itself");
  } finally { await h.done(); }
});

test("the home row plays while the dialog has focus, not while typing a name, and not on key repeat", async () => {
  const h = harness();
  try {
    assert.equal(document.activeElement, h.nameInput, "the name input is focused on mount");
    key(h.nameInput, "a");
    assert.deepEqual(h.log.audition, [], "letters in the name field are the name");
    key(h.dialog, "a");
    key(h.dialog, "k");
    key(h.dialog, ";");
    assert.deepEqual(h.log.audition.map(([, n]) => n), [60, 72, 76]);
    key(h.dialog, "a", { repeat: true });
    key(h.dialog, "a", { ctrlKey: true });
    assert.equal(h.log.audition.length, 3, "auto-repeat and shortcuts are not notes");
    key(h.knob("Attack"), "s");
    assert.equal(h.log.audition.at(-1)![1], 62, "a focused knob still lets the row play");
    key(document.body, "a");
    assert.equal(h.log.audition.length, 4, "keys outside the dialog are not its business");
    h.registerUp().click(); flushSync();
    key(h.dialog, "a");
    assert.equal(h.log.audition.at(-1)![1], 72, "the row follows the register");
  } finally { await h.done(); }
});

test("Escape closes, and no key typed in the dialog reaches a listener above it", async () => {
  const h = harness();
  const seen: string[] = [];
  const spy = (e: Event) => { seen.push((e as KeyboardEvent).key); };
  document.body.addEventListener("keydown", spy);
  try {
    key(h.dialog, "a");
    key(h.nameInput, "x");
    key(h.knob("Attack"), "ArrowUp");
    key(h.dialog, "Escape");
    assert.equal(h.log.closes, 1);
    assert.deepEqual(seen, [], "the Studio's window hotkeys underneath never see these");
    key(document.body, "z");
    assert.deepEqual(seen, ["z"], "the spy itself works");
    h.button("cancel").click();
    assert.equal(h.log.closes, 2);
    h.target.querySelector<HTMLButtonElement>('[aria-label="Close"]')!.click();
    assert.equal(h.log.closes, 3);
  } finally { document.body.removeEventListener("keydown", spy); await h.done(); }
});

test("a press that starts and ends on the scrim closes; a drag out of the card does not", async () => {
  const h = harness();
  try {
    const overlay = h.target.firstElementChild!;
    assert.equal(overlay.getAttribute("role"), "presentation");
    // A knob drag that ends over the scrim delivers a click whose target is the scrim.
    press(h.knob("Attack"));
    overlay.dispatchEvent(new w.MouseEvent("click", { bubbles: true }));
    assert.equal(h.log.closes, 0);
    press(overlay);
    overlay.dispatchEvent(new w.MouseEvent("click", { bubbles: true }));
    assert.equal(h.log.closes, 1);
  } finally { await h.done(); }
});

test("use in passes a trimmed name and a validated clone, disables the footer meanwhile, and closes on success", async () => {
  const gate = deferred<void>();
  const h = harness({ onuse: async (n: string, p: JamPatch) => { h.log.use.push([n, p]); await gate.promise; } });
  try {
    h.nameInput.value = "  meow  ";
    h.nameInput.dispatchEvent(new w.Event("input", { bubbles: true }));
    flushSync();
    const use = h.button("use in moon cat");
    assert.equal(use.disabled, false);
    assert.match(use.textContent!, /3 of 64/);
    use.click(); flushSync();
    assert.equal(h.log.use.length, 1);
    assert.equal(h.log.use[0][0], "meow");
    assert.deepEqual(h.log.use[0][1], LANDING);
    assert.equal(h.button("adding…").disabled, true);
    assert.equal(h.button("save to my library").disabled, true);
    assert.equal(h.button("cancel").disabled, false, "cancel is never held hostage");
    key(h.knob("Attack"), "ArrowUp"); flushSync();
    assert.equal(h.value("Attack"), 1283);
    assert.deepEqual(h.log.use[0][1], LANDING, "what the host received does not follow later edits");
    assert.equal(h.log.closes, 0);
    gate.resolve();
    await settle();
    assert.equal(h.log.closes, 1);
    assert.deepEqual(h.log.notices, []);
  } finally { await h.done(); }
});

test("a rejected save surfaces its message and keeps the dialog open and usable", async () => {
  const h = harness({ onsave: async () => { throw new Error("Patch library full at 12: delete one"); } });
  try {
    h.button("save to my library").click();
    await settle();
    assert.deepEqual(h.log.notices, [["Patch library full at 12: delete one", "error"]]);
    assert.equal(h.log.closes, 0);
    assert.equal(h.button("save to my library").disabled, false, "the footer is re-armed");
    h.log.notices.length = 0;
    h.button("save to my library").click();
    await settle();
    assert.equal(h.log.notices.length, 1, "a second attempt reports again");
  } finally { await h.done(); }
});

test("save and share success notify and stay open; share is hidden without a server name", async () => {
  const h = harness({ shareName: "lantern room", name: "meow" });
  try {
    const share = h.button("share to lantern room");
    assert.match(share.textContent!, /\.jampatch/);
    share.click();
    await settle();
    assert.deepEqual(h.log.share.map(([n]) => n), ["meow"]);
    assert.deepEqual(h.log.notices.at(-1), ["meow is in the lantern room share as a .jampatch", "info"]);
    h.button("save to my library").click();
    await settle();
    assert.deepEqual(h.log.save.map(([n]) => n), ["meow"]);
    assert.deepEqual(h.log.notices.at(-1), ["meow is in your library", "info"]);
    assert.equal(h.log.closes, 0);
  } finally { await h.done(); }
  const quiet = harness();
  try {
    assert.equal([...quiet.target.querySelectorAll("button")].some((b) => /share to/.test(b.textContent!)), false);
  } finally { await quiet.done(); }
});

test("a use result that lands after the dialog closed is ignored: no second onclose", async () => {
  const gate = deferred<void>();
  const h = harness({ onuse: async () => { await gate.promise; } });
  try {
    h.button("use in moon cat").click(); flushSync();
    key(h.dialog, "Escape");
    assert.equal(h.log.closes, 1);
    gate.resolve();
    await settle();
    assert.equal(h.log.closes, 1, "the stale success did not close again");
    assert.deepEqual(h.log.notices, []);
  } finally { await h.done(); }
  // And a rejection landing after unmount neither throws nor notifies.
  const late = deferred<void>();
  const h2 = harness({ onsave: async () => { await late.promise; } });
  h2.button("save to my library").click(); flushSync();
  await h2.done();
  late.reject(new Error("too late"));
  await settle();
  assert.deepEqual(h2.log.notices, []);
});

test("importing a bad file reports an error and leaves the patch untouched; good files replace it", async () => {
  const h = harness({ name: "" });
  try {
    await pickFile(h.target, "broken.jampatch", "not json");
    assert.deepEqual(h.log.notices, [["broken.jampatch: file is not valid JSON", "error"]]);
    assert.equal(h.value("Attack"), 1200);
    assert.equal(h.edited(), false);
    assert.equal(h.nameInput.value, "", "a refused file names nothing");
    h.log.notices.length = 0;
    await pickFile(h.target, "huge.jampatch", "{}", 5000);
    assert.match(h.log.notices[0][0], /larger than a soundfont can be/);
    assert.equal(h.value("Attack"), 1200);
    h.log.notices.length = 0;
    await pickFile(h.target, "dusk purr.jampatch", JSON.stringify(DUSK));
    assert.deepEqual(h.log.notices, []);
    assert.equal(h.value("Attack"), DUSK.e.a);
    assert.equal(h.nameInput.value, "dusk purr", "the file's stem names an unnamed sheet");
    assert.equal(h.edited(), true);
    await pickFile(h.target, "cat.json", JSON.stringify({ v: 1, img: "b".repeat(64), patch: LANDING, lo: 70, hi: 75, len: 333 }));
    assert.equal(h.value("Attack"), LANDING.e.a);
    assert.equal(h.log.notices.length, 1);
    assert.equal(h.log.notices[0][1], "warn");
    assert.match(h.log.notices[0][0], /pitch range and length were not kept/);
    assert.equal(h.nameInput.value, "dusk purr", "a name already there is kept");
    assert.equal(h.edited(), false, "LANDING again is the opening recipe");
  } finally { await h.done(); }
});

test("layers: exactly one opens at a time, add opens the new one, remove keeps at least one", async () => {
  const h = harness();
  try {
    const heads = () => [...h.target.querySelectorAll<HTMLButtonElement>(".sf-layer-head")];
    assert.deepEqual(heads().map((b) => b.getAttribute("aria-expanded")), ["true", "false", "false"]);
    assert.equal(h.target.querySelectorAll(".sf-layer-body").length, 1);
    heads()[2].click(); flushSync();
    assert.deepEqual(heads().map((b) => b.getAttribute("aria-expanded")), ["false", "false", "true"]);
    assert.equal(h.target.querySelector(".micro.sf-layer-full")!.textContent, "3 of 3 layers");
    h.target.querySelector<HTMLButtonElement>('[aria-label="Remove this layer"]')!.click(); flushSync();
    assert.equal(heads().length, 2);
    assert.deepEqual(heads().map((b) => b.getAttribute("aria-expanded")), ["false", "true"], "the open index slides to the last layer");
    h.button("＋ layer").click(); flushSync();
    assert.equal(heads().length, 3);
    assert.deepEqual(heads().map((b) => b.getAttribute("aria-expanded")), ["false", "false", "true"], "the new layer opens");
    assert.equal(heads()[2].textContent!.trim().startsWith("SAW"), true, "a quiet saw, as in the call drawer");
    h.button("SIN", h.modes("Layer 3 wave")).click(); flushSync();
    assert.equal(heads()[2].textContent!.trim().startsWith("SIN"), true);
    h.target.querySelector<HTMLButtonElement>('[aria-label="Remove this layer"]')!.click(); flushSync();
    h.target.querySelector<HTMLButtonElement>('[aria-label="Remove this layer"]')!.click(); flushSync();
    assert.equal(heads().length, 1);
    assert.equal(h.target.querySelector('[aria-label="Remove this layer"]'), null, "the last layer cannot be removed");
    h.button("use in moon cat").click();
    await settle();
    assert.equal(validateJamPatch(h.log.use[0][1]).ok, true);
    assert.equal(h.log.use[0][1].o.length, 1);
  } finally { await h.done(); }
});

test("an invalid opening recipe disables the footer rather than playing something else", async () => {
  const h = harness({ initial: { ...clone(LANDING), e: { ...LANDING.e, a: -5 } } });
  try {
    assert.equal(h.button("use in moon cat").disabled, true);
    assert.equal(h.button("save to my library").disabled, true);
    assert.equal(h.edited(), true, "nothing valid to compare against");
    press(h.target.querySelector('[aria-label="Play C4"]')!);
    assert.deepEqual(h.log.audition, []);
    assert.equal(h.log.notices.length, 1);
    assert.equal(h.log.notices[0][1], "error");
    h.target.querySelector<HTMLButtonElement>('[title="Start from DUSK"]')!.click(); flushSync();
    assert.equal(h.button("use in moon cat").disabled, false, "a shelf tile is the way out");
  } finally { await h.done(); }
});
