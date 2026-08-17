// Music theory + staff engraving for the melody unlock lock.
//
// Two jobs, both pure (no Svelte, no DOM, no audio), so `melody.test.ts` can pin them:
//
//   1. ENCODING — turning what was played into the deterministic string that feeds the vault KDF.
//      This is security-relevant: the same tune must always produce byte-identical output, and
//      incidental things (which finger landed first, how the notes were spelled) must never
//      change it. `encodeMelody` is the only function here the crypto path depends on.
//   2. ENGRAVING — laying that same sequence out on a grand staff. Cosmetic, but it's what makes
//      an absolute-pitch lock learnable: you can see C6 is not C4.
//
// The two must not drift: the score you read has to be the secret you're sealing, so both are
// driven off the same `MelodyEvent[]` and the tests check them against the same fixtures.

/** One thing the player did: notes that overlapped in time, and how long the group was held. */
export type MelodyEvent = {
  /** Ascending, de-duplicated MIDI note numbers. A single note is just a one-element chord. */
  notes: number[];
  /** Index into `DUR_MAX_MS` / `DUR_NAMES` — 0 = eighth … 3 = whole. */
  dur: number;
};

export const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** Absolute, scientific pitch notation: MIDI 60 → "C4". Octaves carry meaning here. */
export const noteName = (n: number): string => `${NOTE_NAMES[n % 12]}${Math.floor(n / 12) - 1}`;

/**
 * Duration classes, deliberately coarse. A human can reproduce "short / normal / long / very
 * long"; nobody can reproduce a twelve-way rhythmic grid, and an unreproducible lock is a
 * destroyed vault — there is no recovery path. Four classes buys entropy that can still be
 * played back a month later.
 */
export const DUR_MAX_MS = [170, 380, 750, Infinity];
export const DUR_NAMES = ["eighth", "quarter", "half", "whole"];
export const durClass = (ms: number): number => DUR_MAX_MS.findIndex((m) => ms < m);

/**
 * Encode one event. Chord tones join with "+", the duration class follows a "."; with rhythm off
 * the duration is omitted entirely rather than defaulted, so "rhythm off" and "played all
 * quarters" stay distinguishable secrets.
 *
 * Callers must hand in ascending, de-duplicated notes (see `normalizeEvent`) — fingering order
 * must not be able to fork the secret.
 */
export const encodeEvent = (e: MelodyEvent, rhythm: boolean): string =>
  e.notes.join("+") + (rhythm ? `.${e.dur}` : "");

/** Ascending + de-duplicated: which notes, never the order they were pressed in. */
export const normalizeEvent = (notes: number[], dur: number): MelodyEvent => ({
  notes: [...new Set(notes)].sort((a, b) => a - b),
  dur,
});

/**
 * The full scheme-prefixed secret, or "" for an empty sequence (which must never unlock).
 *
 * v3 records what a score records — chords and note values. v1 (pitch-class-folded) and v2 (bare
 * absolute notes) are retired: a vault sealed under either must be re-entered under a scheme this
 * build can still produce. The scheme prefix is what keeps a secret entered on the sigil lock
 * and one played on the melody lock from ever colliding into one vault key.
 */
export const encodeMelody = (seq: MelodyEvent[], rhythm: boolean): string =>
  seq.length ? `melody:v3:${seq.map((e) => encodeEvent(e, rhythm)).join("-")}` : "";

/**
 * Entropy estimate for the strength meter — deliberately pessimistic, because this number talks
 * someone out of a four-note tune. log2(24) for an event's first note (a ~two-octave working
 * range), only +2 bits per extra chord tone (people play triads, not random note sets), and +1.5
 * bits for a duration class (four classes, but held notes skew hard to the middle two).
 */
export const melodyBits = (seq: MelodyEvent[], rhythm: boolean): number =>
  Math.round(seq.reduce((b, e) => b + Math.log2(24) + (e.notes.length - 1) * 2 + (rhythm ? 1.5 : 0), 0));

// --- Chord naming --------------------------------------------------------------------------
// Purely cosmetic: the secret is the note numbers, never the label. The label exists so you can
// tell "I played Am, not Am7" when re-learning your tune a month later.

const CHORD_SHAPES: [number[], string][] = [
  [[0, 4, 7], ""], [[0, 3, 7], "m"], [[0, 3, 6], "dim"], [[0, 4, 8], "aug"],
  [[0, 5, 7], "sus4"], [[0, 2, 7], "sus2"], [[0, 2, 4, 7], "add9"],
  [[0, 4, 7, 11], "maj7"], [[0, 4, 7, 10], "7"], [[0, 3, 7, 10], "m7"],
  [[0, 3, 6, 10], "m7♭5"], [[0, 3, 6, 9], "dim7"], [[0, 4, 7, 9], "6"], [[0, 3, 7, 9], "m6"],
  [[0, 2, 5, 7], "sus4add9"], [[0, 4, 6, 10], "7♭5"], [[0, 4, 8, 10], "7♯5"],
];
const INTERVALS = ["oct", "m2", "M2", "m3", "M3", "P4", "TT", "5", "m6", "M6", "m7", "M7"];

/**
 * Name a simultaneity: "" for a single note, an interval for a dyad, a chord symbol for anything
 * that matches a known shape in any inversion (named over its bass, jazz-style: "C/E"), and a
 * bare "root (n)" for a cluster that doesn't. An unnamed cluster is still a perfectly good secret.
 */
export function chordName(notes: number[]): string {
  if (notes.length < 2) return "";
  const bassPc = notes[0] % 12;
  const pcs = [...new Set(notes.map((n) => n % 12))].sort((a, b) => a - b);
  if (pcs.length === 1) return `${NOTE_NAMES[bassPc]} oct`;
  if (pcs.length === 2) return `${NOTE_NAMES[bassPc]} ${INTERVALS[(notes[notes.length - 1] - notes[0]) % 12]}`;
  // The bass gets first refusal on being the root, because some pitch-class sets are genuinely
  // two chords: A-C-E-G is Am7 over an A and C6 over a C, and only the bass tells them apart.
  for (const root of [bassPc, ...pcs.filter((p) => p !== bassPc)]) {
    const iv = pcs.map((p) => (p - root + 12) % 12).sort((a, b) => a - b);
    const hit = CHORD_SHAPES.find(([sh]) => sh.length === iv.length && sh.every((v, i) => v === iv[i]));
    if (hit) {
      const nm = NOTE_NAMES[root] + hit[1];
      return root === bassPc ? nm : `${nm}/${NOTE_NAMES[bassPc]}`; // slash chord = inversion
    }
  }
  return `${NOTE_NAMES[bassPc]} (${pcs.length})`;
}

// --- Staff engraving -----------------------------------------------------------------------
// Notes are placed by DIATONIC step, not semitone: C♯ shares a line with C and carries a ♯.
//
//   step = octave * 7 + letter-degree
//
// Every staff line then lands on an EVEN step and one step is half a line-space, which is what
// makes the whole layout fall out of a single linear map. Treble lines are steps 30–38 (E4…F5),
// bass 18–26 (G2…A3), and middle C (28) lands exactly between them — a real grand staff, with
// its one shared ledger line, for free.

const PC_STEP = [0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5, 6];
/** Which pitch classes engrave with a ♯ (this build spells accidentals sharp; F♭/B♯ never occur). */
export const PC_SHARP = [false, true, false, true, false, false, true, false, true, false, true, false];

export const staffStep = (n: number): number => (Math.floor(n / 12) - 1) * 7 + PC_STEP[n % 12];

export const TREBLE_LINES = [30, 32, 34, 36, 38];
export const BASS_LINES = [18, 20, 22, 24, 26];

/** F5 (the treble top line) at y=20, four px per diatonic step, y growing downward. */
export const yOf = (s: number): number => 20 + (38 - s) * 4;
export const STAFF_TOP = yOf(38); // 20  — treble top line
export const STAFF_BOT = yOf(18); // 100 — bass bottom line

export const HEAD_RX = 5.2;
export const HEAD_RY = 3.9;
const SHEET_X0 = 48; // first note x — leaves the clef gutter clear
const SHEET_PAD = 26; // gap between the last note and the closing barline
const DX_MIN = 22; // below this, note heads start colliding
const DX_MAX = 76; // above this, a three-note tune reads as three lost dots
const STEM_LEN = 26;

/**
 * Ledger lines for one note: only on even steps, only in the direction the note actually leaves
 * the staff, plus the single middle-C line in the gap. Notes in a SPACE just outside a staff
 * (odd steps 17, 27, 29, 39) correctly get none.
 */
export function ledgersFor(step: number): number[] {
  const out: number[] = [];
  if (step >= 40) for (let s = 40; s <= step; s += 2) out.push(s);
  else if (step <= 16) for (let s = 16; s >= step; s -= 2) out.push(s);
  else if (step === 28) out.push(28);
  return out;
}

export type SheetHead = { x: number; y: number; sharp: boolean };
export type SheetEvent = {
  x: number;
  heads: SheetHead[];
  ledgers: { y: number; x: number }[];
  stem: { x: number; y1: number; y2: number } | null;
  flag: boolean;
  filled: boolean;
  label: string;
};
export type Sheet = { w: number; minY: number; h: number; labelY: number; events: SheetEvent[] };

/**
 * Lay a sequence out as SVG-ready geometry, spread across `availW` so the staff spans whatever
 * width the panel has. Spacing is clamped: a two-note tune doesn't stretch into two lonely dots,
 * and a long one compresses only to `DX_MIN` before the sheet overflows and the caller scrolls.
 *
 * The viewBox grows vertically to whatever the tune actually reaches (a C1 or a C8 is many
 * ledger lines away) instead of reserving the full MIDI range as empty space.
 *
 * With rhythm off every event engraves as a quarter — the score then shows exactly what the
 * secret contains, which is pitches only.
 */
export function buildSheet(seq: MelodyEvent[], rhythm: boolean, availW = 560): Sheet {
  const events: SheetEvent[] = [];
  let top = STAFF_TOP;
  let bot = STAFF_BOT;
  const room = Math.max(0, availW - SHEET_X0 - SHEET_PAD);
  // Centre each event in its own slice of the width, so the run of notes reads as evenly spaced
  // between the clef and the closing barline rather than crowding the left edge.
  const dx = Math.min(DX_MAX, Math.max(DX_MIN, room / Math.max(1, seq.length)));
  seq.forEach((ev, i) => {
    const x = SHEET_X0 + i * dx + dx / 2;
    const dur = rhythm ? ev.dur : 1;
    const steps = ev.notes.map(staffStep);
    const mean = steps.reduce((a, b) => a + b, 0) / steps.length;
    // Stems point away from the near edge of whichever staff the chord sits on (middle line of
    // the treble is B4/step 34, of the bass D3/step 22).
    const up = mean <= (mean < 28 ? 22 : 34);
    const heads: SheetHead[] = [];
    const ledgers: { y: number; x: number }[] = [];
    const seen = new Set<number>();
    let prevStep = -99;
    let prevOff = false;
    // Notes a second apart can't share a column — the upper one shifts across the stem. Two in a
    // row (a cluster) must not both shift, hence `prevOff`.
    for (const n of [...ev.notes].sort((a, b) => a - b)) {
      const s = staffStep(n);
      const off: boolean = s - prevStep === 1 && !prevOff;
      heads.push({ x: x + (off ? (up ? HEAD_RX * 2 : -HEAD_RX * 2) : 0), y: yOf(s), sharp: PC_SHARP[n % 12] });
      for (const ls of ledgersFor(s)) if (!seen.has(ls)) { seen.add(ls); ledgers.push({ y: yOf(ls), x }); }
      prevStep = s;
      prevOff = off;
    }
    const ys = heads.map((h) => h.y);
    const hiY = Math.min(...ys);
    const loY = Math.max(...ys);
    const stem = dur === 3 ? null : up
      ? { x: x + HEAD_RX, y1: loY, y2: hiY - STEM_LEN }
      : { x: x - HEAD_RX, y1: hiY, y2: loY + STEM_LEN };
    top = Math.min(top, hiY - (stem ? STEM_LEN + 6 : 8), ...ledgers.map((l) => l.y));
    bot = Math.max(bot, loY + (stem ? STEM_LEN + 6 : 8), ...ledgers.map((l) => l.y));
    events.push({ x, heads, ledgers, stem, flag: dur === 0, filled: dur <= 1, label: chordName(ev.notes) });
  });
  const minY = top - 18; // headroom for the chord symbols
  return {
    w: Math.max(availW, SHEET_X0 + seq.length * dx + SHEET_PAD),
    minY,
    h: bot + 10 - minY,
    labelY: minY + 12,
    events,
  };
}

/** Plain-text score: the accessible alternative to the SVG, and a readable "what did I play". */
export const scoreText = (seq: MelodyEvent[], rhythm: boolean): string =>
  seq.map((e) => e.notes.map(noteName).join("+") + (rhythm ? ` ${DUR_NAMES[e.dur]}` : "")).join(", ");
