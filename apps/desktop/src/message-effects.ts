export type TextEffectGroup = "Motion" | "Mood" | "Signal" | "Pride" | "Utility";

export type TextEffectDefinition = {
  id: string;
  label: string;
  description: string;
  group: TextEffectGroup;
  preview: string;
  animated: boolean;
};

const BASE_EFFECTS: TextEffectDefinition[] = [
  { id: "shake", label: "Shaky", description: "Nervous, excited, or unstable lettering.", group: "Motion", preview: "systems unstable", animated: true },
  { id: "wave", label: "Wavy", description: "Letters rise and fall in sequence.", group: "Motion", preview: "signal rolling", animated: true },
  { id: "sparkle", label: "Rainbow sparkles", description: "A bright spectrum with small starbursts.", group: "Motion", preview: "critical success!", animated: true },
  { id: "animalese", label: "Animalese", description: "Reveals in speech beats with deterministic voice blips once on screen.", group: "Motion", preview: "hello operator", animated: true },
  { id: "flame", label: "Angry flame", description: "Hot, forceful text with an ember edge.", group: "Mood", preview: "I am FURIOUS", animated: true },
  { id: "gloom", label: "Gloom", description: "Heavy, drained text that seems to sink.", group: "Mood", preview: "not my day", animated: true },
  { id: "cyber", label: "Cyber", description: "Cyan/magenta terminal interference.", group: "Signal", preview: "BREACH READY", animated: true },
  { id: "crt", label: "CRT / scanline", description: "Green phosphor lettering with a passing raster band.", group: "Signal", preview: "LINK ESTABLISHED", animated: true },
  { id: "censor", label: "Censored", description: "Obscures the phrase until the reader reveals it.", group: "Utility", preview: "classified", animated: false },
];

const PRIDE_FLAGS: Array<[string, string, string[]]> = [
  ["rainbow", "Rainbow", ["#e40303", "#ff8c00", "#ffed00", "#008026", "#24408e", "#732982"]],
  ["progress", "Progress", ["#000000", "#784f17", "#5bcefa", "#f5a9b8", "#ffffff", "#e40303", "#ffed00", "#008026", "#24408e", "#732982"]],
  ["lesbian", "Lesbian", ["#d52d00", "#ef7627", "#ff9a56", "#ffffff", "#d162a4", "#b55690", "#a30262"]],
  ["gay-men", "Gay men", ["#078d70", "#26ceaa", "#98e8c1", "#ffffff", "#7bade2", "#5049cc", "#3d1a78"]],
  ["bisexual", "Bisexual", ["#d60270", "#9b4f96", "#0038a8"]],
  ["trans", "Trans", ["#5bcefa", "#f5a9b8", "#ffffff", "#f5a9b8", "#5bcefa"]],
  ["nonbinary", "Nonbinary", ["#fff430", "#ffffff", "#9c59d1", "#2d2d2d"]],
  ["pansexual", "Pansexual", ["#ff218c", "#ffd800", "#21b1ff"]],
  ["asexual", "Asexual", ["#171717", "#a3a3a3", "#ffffff", "#800080"]],
  ["aromantic", "Aromantic", ["#3da542", "#a8d47a", "#ffffff", "#a9a9a9", "#1f1f1f"]],
  ["intersex", "Intersex", ["#ffd800", "#7902aa", "#ffd800"]],
  ["genderfluid", "Genderfluid", ["#ff75a2", "#ffffff", "#be18d6", "#202020", "#333ebd"]],
  ["genderqueer", "Genderqueer", ["#b57edc", "#ffffff", "#4a8123"]],
  ["agender", "Agender", ["#171717", "#b9b9b9", "#ffffff", "#b8f483", "#ffffff", "#b9b9b9", "#171717"]],
  ["polysexual", "Polysexual", ["#f61cb9", "#07d569", "#1c92f6"]],
  ["omnisexual", "Omnisexual", ["#ff9ace", "#ff53bf", "#200044", "#6760ff", "#8ca8ff"]],
  ["demisexual", "Demisexual", ["#171717", "#ffffff", "#6e2a8e", "#a3a3a3"]],
  ["demiromantic", "Demiromantic", ["#171717", "#ffffff", "#3da542", "#a3a3a3"]],
];

export const TEXT_EFFECTS: readonly TextEffectDefinition[] = [
  ...BASE_EFFECTS,
  ...PRIDE_FLAGS.map(([id, label]) => ({
    id: `pride/${id}`,
    label: `${label} pride`,
    description: `${label} flag colours with a calm shimmer.`,
    group: "Pride" as const,
    preview: "proud & present",
    animated: true,
  })),
];

export const TEXT_EFFECT_GROUPS: readonly TextEffectGroup[] = ["Motion", "Mood", "Signal", "Pride", "Utility"];
export const TEXT_EFFECT_RE = /^\[fx:([a-z0-9/-]{1,40})\]([^\n]{1,320}?)\[\/fx\]/i;

const EFFECT_IDS = new Set(TEXT_EFFECTS.map((effect) => effect.id));
const FLAG_COLORS = new Map(PRIDE_FLAGS.map(([id, _label, colors]) => [id, colors]));

function escText(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

function escAttr(value: string): string {
  return escText(value).replace(/"/g, "&quot;");
}

function boundedUnits(units: string[], max = 160): string[] {
  if (units.length <= max) return units;
  return [...units.slice(0, max - 1), units.slice(max - 1).join("")];
}

function unitHtml(text: string, className: string, tones = false): string {
  const chars = boundedUnits(Array.from(text));
  return chars.map((char, index) => {
    let hash = 0;
    for (const code of Array.from(char.normalize("NFKD"))) hash = (hash * 31 + (code.codePointAt(0) ?? 0)) >>> 0;
    const tone = tones ? ` data-fx-tone="${hash % 18}"` : "";
    return `<span class="${className} fx-i-${index % 16}"${tone}>${escText(char)}</span>`;
  }).join("");
}

export function isTextEffectId(id: string): boolean {
  return EFFECT_IDS.has(id);
}

export function parseTextEffect(source: string): { raw: string; id: string; text: string } | null {
  const match = TEXT_EFFECT_RE.exec(source);
  if (!match || !isTextEffectId(match[1])) return null;
  return { raw: match[0], id: match[1], text: match[2] };
}

/** Fixed-catalog HTML only. The shared DOMPurify pass remains the final enforcement boundary. */
export function textEffectHtml(id: string, text: string): string {
  if (!isTextEffectId(id)) return escText(text);
  const safeId = id.replace(/\//g, "-");
  if (id === "wave") {
    return `<span class="text-fx text-fx-wave" data-text-fx="wave" aria-label="${escAttr(text)}"><span class="text-fx-visual" aria-hidden="true">${unitHtml(text, "text-fx-unit")}</span></span>`;
  }
  if (id === "animalese") {
    return `<span class="text-fx text-fx-animalese" data-text-fx="animalese" aria-label="${escAttr(text)}"><span class="text-fx-visual" aria-hidden="true">${unitHtml(text, "text-fx-unit fx-animalese-unit", true)}</span></span>`;
  }
  if (id === "censor") {
    return `<span class="text-fx text-fx-censor" data-text-fx="censor" tabindex="0" role="button" title="Reveal censored text">${escText(text)}</span>`;
  }
  if (id.startsWith("pride/")) {
    const flag = id.slice("pride/".length);
    return `<span class="text-fx text-fx-pride text-fx-${safeId}" data-text-fx="${escAttr(id)}" data-pride="${escAttr(flag)}">${escText(text)}</span>`;
  }
  return `<span class="text-fx text-fx-${safeId}" data-text-fx="${safeId}">${escText(text)}</span>`;
}

export function textEffectGradient(id: string): string {
  if (!id.startsWith("pride/")) return "";
  return FLAG_COLORS.get(id.slice("pride/".length))?.join(",") ?? "";
}

export function formatTextEffect(id: string, text: string): string {
  return isTextEffectId(id) ? `[fx:${id}]${text}[/fx]` : text;
}

export function insertTextEffect(
  source: string,
  start: number,
  end: number,
  id: string,
): { value: string; selectionStart: number; selectionEnd: number } {
  const a = Math.max(0, Math.min(source.length, Math.floor(start)));
  const b = Math.max(a, Math.min(source.length, Math.floor(end)));
  const selected = source.slice(a, b) || (id === "censor" ? "classified" : "text");
  const open = `[fx:${isTextEffectId(id) ? id : "cyber"}]`;
  const close = "[/fx]";
  let tokenCount = 0;
  const lines = selected.split("\n").map((line) => {
    if (!line) return "";
    const chunks: string[] = [];
    let chunk = "";
    for (const unit of Array.from(line)) {
      if (chunk && chunk.length + unit.length > 320) {
        chunks.push(open + chunk + close);
        tokenCount += 1;
        chunk = "";
      }
      chunk += unit;
    }
    if (chunk) { chunks.push(open + chunk + close); tokenCount += 1; }
    return chunks.join("");
  });
  const wrapped = lines.join("\n");
  const value = source.slice(0, a) + wrapped + source.slice(b);
  if (tokenCount === 1 && !selected.includes("\n") && selected.length <= 320) {
    const selectionStart = a + open.length;
    return { value, selectionStart, selectionEnd: selectionStart + selected.length };
  }
  const caret = a + wrapped.length;
  return { value, selectionStart: caret, selectionEnd: caret };
}

export function stripTextEffects(text: string): string {
  return (text ?? "").replace(/\[fx:[a-z0-9/-]{1,40}\]([^\n]{1,320}?)\[\/fx\]/gi, "$1");
}
