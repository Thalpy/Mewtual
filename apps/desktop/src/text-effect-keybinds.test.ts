import assert from "node:assert/strict";
import test from "node:test";
import {
  DEFAULT_TEXT_EFFECT_KEYBINDS,
  effectForKeybind,
  keybindConflict,
  keybindFromEvent,
  sanitizeTextEffectKeybinds,
} from "./text-effect-keybinds.ts";

test("key chords normalize modifiers in a stable order", () => {
  assert.equal(keybindFromEvent({ key: "7", ctrlKey: false, metaKey: false, altKey: true, shiftKey: true }), "Alt+Shift+7");
  assert.equal(keybindFromEvent({ key: "&", code: "Digit7", ctrlKey: false, metaKey: false, altKey: true, shiftKey: true }), "Alt+Shift+7");
  assert.equal(keybindFromEvent({ key: "Shift", ctrlKey: false, metaKey: false, altKey: false, shiftKey: true }), "");
});

test("bindings reject reserved and duplicate chords", () => {
  assert.match(keybindConflict(DEFAULT_TEXT_EFFECT_KEYBINDS, "cyber", "Ctrl+B"), /reserved/);
  assert.match(keybindConflict(DEFAULT_TEXT_EFFECT_KEYBINDS, "cyber", "Alt+Shift+1"), /Shaky/);
  assert.equal(keybindConflict(DEFAULT_TEXT_EFFECT_KEYBINDS, "cyber", "Alt+X"), "");
});

test("stored bindings keep only catalogued, modified, unique, non-reserved chords", () => {
  const clean = sanitizeTextEffectKeybinds({ shake: "Alt+X", wave: "Alt+X", cyber: "Ctrl+B", crt: "A", nope: "Alt+Y" });
  assert.deepEqual(clean, { shake: "Alt+X" });
  assert.equal(effectForKeybind({ ...DEFAULT_TEXT_EFFECT_KEYBINDS, cyber: "Alt+X" }, "Alt+X"), "cyber");
});

test("a saved Animalese shortcut migrates to Speakese", () => {
  assert.deepEqual(sanitizeTextEffectKeybinds({ animalese: "Alt+M" }), { speakese: "Alt+M" });
  assert.equal(DEFAULT_TEXT_EFFECT_KEYBINDS.speakese, "Alt+Shift+4");
  assert.equal(DEFAULT_TEXT_EFFECT_KEYBINDS.animalese, undefined);
});

test("the two theatrical effects have mnemonic defaults and remain customizable", () => {
  assert.equal(DEFAULT_TEXT_EFFECT_KEYBINDS["red-truth"], "Alt+Shift+R");
  assert.equal(DEFAULT_TEXT_EFFECT_KEYBINDS["perfect-cherry-blossom"], "Alt+Shift+C");
  assert.deepEqual(sanitizeTextEffectKeybinds({
    "red-truth": "Ctrl+Alt+T",
    "perfect-cherry-blossom": "Ctrl+Alt+P",
  }), {
    "perfect-cherry-blossom": "Ctrl+Alt+P",
    "red-truth": "Ctrl+Alt+T",
  });
});
