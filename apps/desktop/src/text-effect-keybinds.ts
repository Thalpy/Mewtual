import { TEXT_EFFECTS, isTextEffectId } from "./message-effects.ts";

export const DEFAULT_TEXT_EFFECT_KEYBINDS: Record<string, string> = {
  shake: "Alt+Shift+1",
  wave: "Alt+Shift+2",
  sparkle: "Alt+Shift+3",
  animalese: "Alt+Shift+4",
  flame: "Alt+Shift+5",
  gloom: "Alt+Shift+6",
  cyber: "Alt+Shift+7",
  crt: "Alt+Shift+8",
  censor: "Alt+Shift+9",
  "pride/rainbow": "Alt+Shift+0",
};

export const RESERVED_KEYBINDS = new Set([
  "Ctrl+B", "Ctrl+I", "Ctrl+K", "Ctrl+L", "Ctrl+O", "Ctrl+F", "Ctrl+Shift+F",
  "Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4", "Ctrl+5", "Ctrl+6", "Ctrl+7",
  "Meta+B", "Meta+I", "Meta+K", "Meta+L", "Meta+O", "Meta+F", "Meta+Shift+F",
  "Meta+1", "Meta+2", "Meta+3", "Meta+4", "Meta+5", "Meta+6", "Meta+7",
]);

function displayKey(key: string, code = ""): string {
  // Shift changes `KeyboardEvent.key` (1 becomes ! on many layouts), but the shortcut's label
  // and saved default describe the physical digit/letter key. `code` keeps those stable while
  // leaving punctuation and non-Latin custom keys readable through `key`.
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (key === " ") return "Space";
  if (key === "Escape") return "Esc";
  if (key === "ArrowUp") return "Up";
  if (key === "ArrowDown") return "Down";
  if (key === "ArrowLeft") return "Left";
  if (key === "ArrowRight") return "Right";
  return key.length === 1 ? key.toUpperCase() : key;
}

export function keybindFromEvent(event: Pick<KeyboardEvent, "key" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey"> & { code?: string }): string {
  if (["Control", "Alt", "Shift", "Meta"].includes(event.key)) return "";
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.metaKey) parts.push("Meta");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  parts.push(displayKey(event.key, event.code));
  return parts.join("+");
}

export function sanitizeTextEffectKeybinds(raw: unknown): Record<string, string> {
  const source = raw && typeof raw === "object" ? raw as Record<string, unknown> : {};
  const out: Record<string, string> = {};
  const used = new Set<string>();
  for (const effect of TEXT_EFFECTS) {
    const chord = source[effect.id];
    if (
      typeof chord !== "string" || chord.length > 48 || !/^(?=.*(?:Ctrl|Meta|Alt))/.test(chord) ||
      RESERVED_KEYBINDS.has(chord) || used.has(chord)
    ) continue;
    out[effect.id] = chord;
    used.add(chord);
  }
  return out;
}

export function keybindConflict(bindings: Record<string, string>, id: string, chord: string): string {
  if (!isTextEffectId(id) || !chord) return "Choose a key plus at least one modifier.";
  if (!/^(?=.*(?:Ctrl|Meta|Alt))/.test(chord)) return "Use Ctrl, Alt, or Cmd so normal typing stays normal.";
  if (RESERVED_KEYBINDS.has(chord)) return "That shortcut is already reserved by the app.";
  const duplicate = Object.entries(bindings).find(([other, value]) => other !== id && value === chord);
  return duplicate ? `Already assigned to ${TEXT_EFFECTS.find((effect) => effect.id === duplicate[0])?.label ?? duplicate[0]}.` : "";
}

export function effectForKeybind(bindings: Record<string, string>, chord: string): string {
  return Object.entries(bindings).find(([, value]) => value === chord)?.[0] ?? "";
}
