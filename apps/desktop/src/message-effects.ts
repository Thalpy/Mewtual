export type TextEffectGroup = "Motion" | "Mood" | "Signal" | "Pride" | "Utility";

export type TextEffectDefinition = {
  id: string;
  label: string;
  description: string;
  group: TextEffectGroup;
  preview: string;
  animated: boolean;
};

export type TextEffectPointerRegion = "palette" | "trigger" | "editor" | "outside";

/** A textarea retains its selection after blur; only the three owning regions keep its Aa bar. */
export function dismissTextEffectPalette(catalogOpen: boolean, region: TextEffectPointerRegion): boolean {
  return !catalogOpen && region === "outside";
}

/** Petals fire once on entry; pointer motion within the same rendered effect never re-arms them. */
export function cherryBlossomShouldBurst(entered: boolean, effectId: string | undefined): boolean {
  return entered && effectId === "perfect-cherry-blossom";
}

const BASE_EFFECTS: TextEffectDefinition[] = [
  { id: "shake", label: "Shaky", description: "Nervous, excited, or unstable lettering.", group: "Motion", preview: "systems unstable", animated: true },
  { id: "wave", label: "Wavy", description: "Letters rise and fall in sequence.", group: "Motion", preview: "signal rolling", animated: true },
  { id: "sparkle", label: "Rainbow sparkles", description: "A bright spectrum with small starbursts.", group: "Motion", preview: "critical success!", animated: true },
  { id: "speakese", label: "Speakese", description: "Pops in letter by letter with a distinct, phoneme-coloured voice blip.", group: "Motion", preview: "hello operator", animated: true },
  { id: "perfect-cherry-blossom", label: "Perfect Cherry Blossom", description: "Petal-pink lettering with drifting sakura; entering the text shakes loose one fresh bloom.", group: "Motion", preview: "petals on the signal", animated: true },
  { id: "red-truth", label: "The Red Truth", description: "A declarative crimson entrance: sharp metallic strike, rising noise wash, then a clean letter reveal.", group: "Mood", preview: "This statement is absolute.", animated: true },
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
export const SPEAKESE_STEP_SECONDS = 0.072;
export const MAX_SPEAKESE_BLIPS = 64;
export const PERFECT_CHERRY_BLOSSOM_PETALS = 7;

export type SpeakeseBlip = {
  at: number;
  stop: number;
  frequency: number;
  endFrequency: number;
  waveform: OscillatorType;
  peak: number;
};

export function speakeseSoundPlan(tones: number[], start: number): SpeakeseBlip[] {
  return tones.slice(0, MAX_SPEAKESE_BLIPS).map((rawTone, index) => {
    const tone = Math.max(0, Math.min(23, Number.isFinite(rawTone) ? rawTone : 0));
    return {
      at: start + index * SPEAKESE_STEP_SECONDS,
      stop: start + index * SPEAKESE_STEP_SECONDS + 0.065,
      frequency: 185 * Math.pow(2, tone / 17),
      endFrequency: 198 * Math.pow(2, tone / 17),
      waveform: tone % 3 === 0 ? "triangle" : "sine",
      peak: 0.036,
    };
  });
}

export type RedTruthNoiseSweep = {
  at: number;
  crest: number;
  stop: number;
  startFrequency: number;
  crestFrequency: number;
  endFrequency: number;
  highpassFrequency: number;
  peak: number;
};

export type RedTruthSoundPlan = {
  strike: SpeakeseBlip[];
  sweep: RedTruthNoiseSweep;
};

/** An original two-layer cue: an inharmonic metal strike, then a delayed rising/falling wash. */
export function redTruthSoundPlan(start: number): RedTruthSoundPlan {
  return {
    strike: [
      { at: start, stop: start + 0.058, frequency: 5_700, endFrequency: 2_450, waveform: "sawtooth", peak: 0.024 },
      { at: start + 0.004, stop: start + 0.29, frequency: 1_810, endFrequency: 1_420, waveform: "sine", peak: 0.052 },
      { at: start + 0.007, stop: start + 0.23, frequency: 3_610, endFrequency: 2_760, waveform: "triangle", peak: 0.027 },
    ],
    sweep: {
      at: start + 0.075,
      crest: start + 0.92,
      stop: start + 1.85,
      startFrequency: 820,
      crestFrequency: 5_200,
      endFrequency: 1_300,
      highpassFrequency: 380,
      peak: 0.027,
    },
  };
}

/** Stateless white noise keeps the authored cue repeatable and avoids ambient randomness. */
export function redTruthNoiseSample(index: number): number {
  let value = (Math.max(0, Math.floor(index)) + 0x6d2b79f5) | 0;
  value = Math.imul(value ^ (value >>> 15), value | 1);
  value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
  return (((value ^ (value >>> 14)) >>> 0) / 0xffff_ffff) * 2 - 1;
}

const EFFECT_IDS = new Set(TEXT_EFFECTS.map((effect) => effect.id));
const EFFECT_ALIASES: Record<string, string> = { animalese: "speakese" };
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

const SPEECH_PAIRS = new Set(["th", "sh", "ch", "ph", "ng", "qu", "ck", "ee", "oo", "ea", "ai", "ou", "ow", "oi", "oy"]);
function speechTone(chars: string[], index: number): number {
  const bare = (value = "") => value.normalize("NFKD").replace(/\p{Mark}/gu, "").toLowerCase();
  const current = bare(chars[index]);
  const ahead = current + bare(chars[index + 1]);
  const behind = bare(chars[index - 1]) + current;
  const phoneme = SPEECH_PAIRS.has(ahead) ? ahead : SPEECH_PAIRS.has(behind) ? behind : current;
  let hash = 0;
  for (const code of Array.from(phoneme)) hash = (hash * 37 + (code.codePointAt(0) ?? 0)) >>> 0;
  // Vowels, consonants/digraphs, and punctuation occupy noticeably different registers.
  if (/^[aeiouy]/.test(phoneme)) return 2 + (hash % 7);
  if (/^[a-z]/.test(phoneme)) return 10 + (hash % 10);
  return 20 + (hash % 4);
}

function unitHtml(text: string, className: string, tones = false): string {
  const chars = boundedUnits(Array.from(text));
  return chars.map((char, index) => {
    const tone = tones ? ` data-fx-tone="${speechTone(chars, index)}"` : "";
    return `<span class="${className} fx-i-${index % 16}"${tone}>${escText(char)}</span>`;
  }).join("");
}

function cherryBlossomPetalsHtml(): string {
  return Array.from({ length: PERFECT_CHERRY_BLOSSOM_PETALS }, (_, index) =>
    `<i class="fx-blossom-petal fx-blossom-petal-${index + 1}"></i>`,
  ).join("");
}

export function isTextEffectId(id: string): boolean {
  return EFFECT_IDS.has(id) || Object.hasOwn(EFFECT_ALIASES, id);
}

export function canonicalTextEffectId(id: string): string {
  return EFFECT_ALIASES[id] ?? id;
}

export function parseTextEffect(source: string): { raw: string; id: string; text: string } | null {
  const match = TEXT_EFFECT_RE.exec(source);
  if (!match || !isTextEffectId(match[1])) return null;
  return { raw: match[0], id: canonicalTextEffectId(match[1]), text: match[2] };
}

/** Fixed-catalog HTML only. The shared DOMPurify pass remains the final enforcement boundary. */
export function textEffectHtml(id: string, text: string): string {
  if (!isTextEffectId(id)) return escText(text);
  id = canonicalTextEffectId(id);
  const safeId = id.replace(/\//g, "-");
  if (id === "wave") {
    return `<span class="text-fx text-fx-wave" data-text-fx="wave" aria-label="${escAttr(text)}"><span class="text-fx-visual" aria-hidden="true">${unitHtml(text, "text-fx-unit")}</span></span>`;
  }
  if (id === "speakese") {
    return `<span class="text-fx text-fx-speakese" data-text-fx="speakese" aria-label="${escAttr(text)}"><span class="text-fx-visual" aria-hidden="true">${unitHtml(text, "text-fx-unit fx-speakese-unit", true)}</span></span>`;
  }
  if (id === "red-truth") {
    return `<span class="text-fx text-fx-red-truth" data-text-fx="red-truth" aria-label="${escAttr(text)}"><i class="fx-red-truth-flourish" aria-hidden="true"></i><span class="text-fx-visual" aria-hidden="true">${unitHtml(text, "text-fx-unit fx-red-truth-unit")}</span></span>`;
  }
  if (id === "perfect-cherry-blossom") {
    return `<span class="text-fx text-fx-perfect-cherry-blossom" data-text-fx="perfect-cherry-blossom" aria-label="${escAttr(text)}"><span class="text-fx-visual" aria-hidden="true">${escText(text)}</span><span class="fx-blossom-petals" aria-hidden="true">${cherryBlossomPetalsHtml()}</span></span>`;
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
  return isTextEffectId(id) ? `[fx:${canonicalTextEffectId(id)}]${text}[/fx]` : text;
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
  const open = `[fx:${isTextEffectId(id) ? canonicalTextEffectId(id) : "cyber"}]`;
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
