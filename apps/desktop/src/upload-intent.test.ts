import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { ShiftIntent, shiftPick } from "./upload-intent.ts";
import { readdirSync } from "node:fs";
import {
  FILE_DRAG_ATTR, SHIELD_SAFETY_MS, carriesFiles, installFileDropGuard, keepsNativeDrop, onWindowDragOver, onWindowDrop,
} from "./drop-guard.ts";

const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
const label = { tagName: "LABEL" };
const input = { tagName: "INPUT" };

// --- Shift on the picker ------------------------------------------------------------------------

test("a Shift+click on the picker's label makes the next pick skip the editor, once", () => {
  const intent = new ShiftIntent();
  intent.notePickClick({ shiftKey: true, target: label, currentTarget: label });
  // The label then forwards the click to the input, which bubbles back up through the label.
  intent.notePickClick({ shiftKey: false, target: input, currentTarget: label });
  assert.equal(intent.takePick(), true);
  assert.equal(intent.takePick(), false, "read once");
});

test("a Shift+click on something inside the label (an icon) still counts", () => {
  const intent = new ShiftIntent();
  intent.notePickClick({ shiftKey: true, target: { tagName: "SPAN" }, currentTarget: label });
  assert.equal(intent.takePick(), true);
});

test("a cancelled Shift+pick is forgotten by the next ordinary click", () => {
  const intent = new ShiftIntent();
  intent.notePickClick({ shiftKey: true, target: label, currentTarget: label });
  intent.notePickClick({ shiftKey: false, target: label, currentTarget: label });
  assert.equal(intent.takePick(), false);
});

test("the label action feeds real clicks to the intent and detaches cleanly", () => {
  const labelEl = w.document.createElement("label");
  const inputEl = w.document.createElement("input");
  inputEl.type = "file";
  labelEl.append(inputEl);
  w.document.body.append(labelEl);
  const intent = new ShiftIntent();
  const action = shiftPick(labelEl, intent);
  labelEl.dispatchEvent(new w.MouseEvent("click", { shiftKey: true, bubbles: true }));
  inputEl.dispatchEvent(new w.MouseEvent("click", { shiftKey: false, bubbles: true }));
  assert.equal(intent.takePick(), true);
  action.destroy();
  labelEl.dispatchEvent(new w.MouseEvent("click", { shiftKey: true, bubbles: true }));
  assert.equal(intent.takePick(), false, "no listener after destroy");
  labelEl.remove();
});

// --- Window drop decisions ----------------------------------------------------------------------

function drag(types: string[], target: unknown = w.document.body, defaultPrevented = false) {
  const e = {
    defaultPrevented,
    target,
    dataTransfer: { types, dropEffect: "copy" },
    preventDefault() { e.defaultPrevented = true; },
  };
  return e;
}

test("a file dragged in from outside over no drop zone is refused rather than opened", () => {
  const over = drag(["Files"]);
  onWindowDragOver(over, false);
  assert.equal(over.defaultPrevented, true);
  assert.equal(over.dataTransfer.dropEffect, "none");
  const dropped = drag(["Files"]);
  onWindowDrop(dropped, false);
  assert.equal(dropped.defaultPrevented, true, "the webview never navigates to a dropped file");
});

test("a link or text dragged in from another app cannot navigate the window either", () => {
  const dropped = drag(["text/uri-list", "text/plain"]);
  onWindowDrop(dropped, false);
  assert.equal(dropped.defaultPrevented, true);
});

test("an editable field keeps its native text drop, and a file input its native file drop", () => {
  const field = w.document.createElement("textarea");
  w.document.body.append(field);
  const text = drag(["text/plain"], field);
  onWindowDragOver(text, false);
  onWindowDrop(text, false);
  assert.equal(text.defaultPrevented, false);
  const fileInput = w.document.createElement("input");
  fileInput.type = "file";
  w.document.body.append(fileInput);
  const file = drag(["Files"], fileInput);
  onWindowDrop(file, false);
  assert.equal(file.defaultPrevented, false, "the avatar or icon picker takes the dropped file itself");
  assert.equal(keepsNativeDrop(field, true), false, "a text field does not take a dropped file");
  field.remove();
  fileInput.remove();
});

test("a link dropped on a control that is not a text field cannot navigate the window", () => {
  // The engine's fallback on a non-editable target is to follow the link.
  for (const make of [
    () => Object.assign(w.document.createElement("input"), { type: "checkbox" }),
    () => Object.assign(w.document.createElement("input"), { type: "range" }),
    () => Object.assign(w.document.createElement("input"), { type: "text", readOnly: true }),
    () => Object.assign(w.document.createElement("input"), { type: "text", disabled: true }),
    () => Object.assign(w.document.createElement("textarea"), { disabled: true }),
  ]) {
    const el = make();
    w.document.body.append(el);
    const dropped = drag(["text/uri-list"], el);
    onWindowDrop(dropped, false);
    assert.equal(dropped.defaultPrevented, true, el.outerHTML);
    el.remove();
  }
});

test("a drop zone's own acceptance and in-page drags are left alone", () => {
  const accepted = drag(["Files"], w.document.body, true);
  onWindowDragOver(accepted, false);
  assert.equal(accepted.dataTransfer.dropEffect, "copy");
  const internal = drag(["text/plain"]);
  onWindowDragOver(internal, true);
  onWindowDrop(internal, true);
  assert.equal(internal.defaultPrevented, false);
  assert.equal(carriesFiles({ defaultPrevented: false, preventDefault() {}, dataTransfer: null }), false);
});

// --- The installed guard and the iframe shield --------------------------------------------------

function fakeTimers() {
  let next = 0;
  const pending = new Map<number, () => void>();
  return {
    setTimeout: (fn: () => void) => { pending.set(++next, fn); return next; },
    clearTimeout: (h: unknown) => { pending.delete(h as number); },
    fireAll: () => { for (const [h, fn] of [...pending]) { pending.delete(h); fn(); } },
  };
}
function dispatchDrag(type: string, types: string[], on: EventTarget = w) {
  const e = new w.Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(e, "dataTransfer", { value: { types, dropEffect: "copy" } });
  on.dispatchEvent(e);
  return e;
}

test("while files are dragged over the window the shield is up, and it lifts once the drag is over", () => {
  const timers = fakeTimers();
  const uninstall = installFileDropGuard(w, w.document, timers);
  const root = w.document.documentElement;
  try {
    dispatchDrag("dragover", ["Files"]);
    assert.equal(root.hasAttribute(FILE_DRAG_ATTR), true, "raised while files are over the window");
    dispatchDrag("drop", ["Files"]);
    assert.equal(root.hasAttribute(FILE_DRAG_ATTR), false, "lifted on drop");
    dispatchDrag("dragenter", ["Files"]);
    assert.equal(root.hasAttribute(FILE_DRAG_ATTR), true);
    // A drag that left the window and was cancelled outside: the next pointer event ends it
    // (pointer events never fire during a drag).
    w.dispatchEvent(new w.Event("pointermove"));
    assert.equal(root.hasAttribute(FILE_DRAG_ATTR), false, "lifted by the first pointer event after the drag");
    dispatchDrag("dragover", ["text/plain"]);
    assert.equal(root.hasAttribute(FILE_DRAG_ATTR), false, "only file drags raise it");
  } finally { uninstall(); }
  assert.equal(root.hasAttribute(FILE_DRAG_ATTR), false);
});

test("the shield holds through a long still hover and only the safety timer lifts it unattended", () => {
  // dragover may arrive only every ~550 ms, or only on movement: silence is not the end of a drag.
  assert.ok(SHIELD_SAFETY_MS >= 2000);
  const timers = fakeTimers();
  const uninstall = installFileDropGuard(w, w.document, timers);
  const root = w.document.documentElement;
  try {
    dispatchDrag("dragover", ["Files"]);
    assert.equal(root.hasAttribute(FILE_DRAG_ATTR), true);
    timers.fireAll(); // SHIELD_SAFETY_MS of nothing at all
    assert.equal(root.hasAttribute(FILE_DRAG_ATTR), false);
  } finally { uninstall(); }
});

test("a drag that started inside the page is not refused, and a file drag of any origin is shielded", () => {
  const timers = fakeTimers();
  const uninstall = installFileDropGuard(w, w.document, timers);
  const root = w.document.documentElement;
  try {
    dispatchDrag("dragstart", ["text/plain"], w.document.body);
    const over = dispatchDrag("dragover", ["Files"]);
    assert.equal(over.defaultPrevented, false, "an in-page drag is not refused");
    assert.equal(root.hasAttribute(FILE_DRAG_ATTR), true, "but a chat image dragged in-page still cannot reach a frame");
    dispatchDrag("dragend", ["text/plain"], w.document.body);
    const outside = dispatchDrag("dragover", ["Files"]);
    assert.equal(outside.defaultPrevented, true, "after it ends, outside drags are guarded again");
  } finally { uninstall(); }
});

test("a drag whose source left the page still ends the in-page state", () => {
  const timers = fakeTimers();
  const uninstall = installFileDropGuard(w, w.document, timers);
  try {
    const source = w.document.createElement("span");
    w.document.body.append(source);
    dispatchDrag("dragstart", ["text/plain"], source);
    source.remove(); // the row was replaced while its text was being dragged out
    dispatchDrag("dragend", ["text/plain"], source); // fires on the detached node, never reaches the document
    const outside = dispatchDrag("dragover", ["Files"]);
    assert.equal(outside.defaultPrevented, true);
  } finally { uninstall(); }
});

test("the shield rule and the picker rule live in a stylesheet, never in runtime or inline styles", () => {
  // Tauri nonces the inline <style> in index.html and adds that nonce to `style-src`, which makes the
  // browser ignore 'unsafe-inline': a <style> made at runtime or a style="" attribute is refused in
  // the shipped app while working in development and in every test.
  const css = readFileSync(new URL("./drag-drop.css", import.meta.url), "utf8");
  assert.match(css, new RegExp(String.raw`html\[${FILE_DRAG_ATTR}\] iframe \{\s*pointer-events: none !important;`));
  assert.match(css, /\.shift-pick \{[^}]*user-select: none/);
  const main = readFileSync(new URL("./main.ts", import.meta.url), "utf8");
  assert.match(main, /await import\("\.\/drag-drop\.css"\);/);
  const dir = new URL("./", import.meta.url);
  const offenders = readdirSync(dir)
    .filter((f) => /\.(ts|svelte)$/.test(f) && !f.endsWith(".test.ts") && f !== "visual-fixture.ts")
    .filter((f) => /createElement\(\s*["']style["']\s*\)/.test(readFileSync(new URL(f, dir), "utf8")));
  assert.deepEqual(offenders, [], "a runtime <style> would be refused by the production CSP");
  const app = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  for (const picker of app.match(/<label[^>]*use:shiftPick[^>]*>/g) ?? []) {
    assert.match(picker, /class="[^"]*\bshift-pick\b/, `a Shift picker label is selectable: ${picker}`);
  }
});

test("the main window lets the page receive dropped files", () => {
  // Without this, Tauri swallows file drops on Windows and no drop zone in the app ever fires.
  const config = JSON.parse(readFileSync(new URL("../src-tauri/tauri.conf.json", import.meta.url), "utf8"));
  const main = config.app.windows.find((win: { label: string }) => win.label === "main");
  assert.equal(main.dragDropEnabled, false);
});
