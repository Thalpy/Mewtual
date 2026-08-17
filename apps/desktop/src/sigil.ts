// Lattice geometry + encoding for the sigil unlock lock.
//
// Same split as melody.ts: everything here is pure (no Svelte, no DOM, no pointer events), so
// `sigil.test.ts` can pin it. Two jobs:
//
//   1. ENCODING — folding the factors (drawn path, node colour marks, focus-emoji set, magic
//      word) into the deterministic string that feeds the vault KDF. Security-critical: the
//      same inputs must always produce byte-identical output, incidental things (pointer
//      wobble, where a node happens to sit in pixels, how a platform spells an emoji, the
//      ORDER emoji were toggled in) must never change it, and two DIFFERENT inputs must never
//      produce the same string. `encodeSigil` is the only function here the crypto path
//      depends on.
//   2. GEOMETRY — the fixed magic-circle lattice and its hit-testing. The artwork is cosmetic,
//      but the NODE INDICES are the secret, so the index assignment is frozen forever even if
//      the radii or the rendering move.
//
// This lock REPLACES the spell lock ("spell:v1:", glyphs by catalog index): spell joins melody
// v1/v2 in retirement — a vault sealed under it must be re-entered under a scheme this build
// can still produce. The scheme prefix keeps a sigil and a melody from ever colliding.

// --- Lattice -------------------------------------------------------------------------------
// One centre node, an inner ring of 6, an outer ring of 12 — 19 nodes, indexed:
//
//   0        centre
//   1…6      inner ring, index 1 at 12 o'clock, clockwise
//   7…18     outer ring, index 7 at 12 o'clock, clockwise
//
// These indices ARE the wire format. Never renumber, never insert, never reorder.

export const SIGIL_VIEW = 288; // square viewBox side; every radius below fits inside it
export const SIGIL_C = SIGIL_VIEW / 2;
export const R_INNER = 52;
export const R_OUTER = 96;
// The decorated layers sit outside the node art (which reaches R_OUTER + NODE_R = 108): the
// inscription baseline first, then the emoji points at the rim. Cosmetic — move freely.
export const R_TEXT = 114; // ring-inscription baseline radius
export const R_EMOJI = 133; // the chosen emoji repeats as the circle's points at this radius
/** Visual node radius. Purely cosmetic. */
export const NODE_R = 12;
/**
 * Catch radius, deliberately SMALLER than the node art: the pointer must reach a node's inner
 * disc to latch it. Because every catch disc is disjoint (the closest two nodes are 44 units
 * apart — see the invariant test), the space between nodes is dead: once latched you stay
 * latched until you genuinely enter another node. That dead-zone hysteresis is what makes a
 * shakily drawn sigil land on exactly the same index sequence every time — and exact
 * reproduction is the whole game, because the vault has no fuzzy matching to forgive a miss.
 */
export const CATCH_R = 10;

export type LatticeNode = { x: number; y: number; ring: 0 | 1 | 2 };

const polar = (r: number, deg: number, ring: 0 | 1 | 2): LatticeNode => ({
  // -90° puts index k=0 at 12 o'clock; screen y grows downward, so increasing angles run
  // clockwise — the direction people expect a "dial" of nodes to be numbered.
  x: SIGIL_C + r * Math.cos(((deg - 90) * Math.PI) / 180),
  y: SIGIL_C + r * Math.sin(((deg - 90) * Math.PI) / 180),
  ring,
});

export const LATTICE: LatticeNode[] = [
  { x: SIGIL_C, y: SIGIL_C, ring: 0 },
  ...Array.from({ length: 6 }, (_, k) => polar(R_INNER, k * 60, 1 as const)),
  ...Array.from({ length: 12 }, (_, k) => polar(R_OUTER, k * 30, 2 as const)),
];

/** Accessible name for a node — the lattice must be usable by keyboard, node by node. */
export const nodeLabel = (i: number): string =>
  i === 0 ? "centre" : i <= 6 ? `inner ring ${i} of 6` : `outer ring ${i - 6} of 12`;

/**
 * Which node (if any) the point is latched to: the index whose catch disc contains it, else -1.
 * The discs are disjoint by construction, so the answer never depends on iteration order.
 */
export function hitNode(x: number, y: number): number {
  for (let i = 0; i < LATTICE.length; i++) {
    const dx = LATTICE[i].x - x;
    const dy = LATTICE[i].y - y;
    if (dx * dx + dy * dy <= CATCH_R * CATCH_R) return i;
  }
  return -1;
}

/**
 * Append a hit to the stroke in progress. A miss (-1) is a no-op, and so is re-latching the
 * node you are already on (jitter inside one catch disc must not spam segments) — but a
 * genuine RETURN (A→B→A) is a legal, meaningful part of the secret.
 */
export const appendHit = (stroke: number[], hit: number): number[] =>
  hit < 0 || stroke[stroke.length - 1] === hit ? stroke : [...stroke, hit];

// --- Gesture classification ----------------------------------------------------------------
// A click is a degenerate drag, and both verbs live on the same nodes: a TAP cycles a node's
// colour mark, a DRAG that reaches a second node draws a path stroke. The classifier is pure
// so the disambiguation rule itself is testable, not buried in pointer plumbing.

/**
 * Maximum pointer travel (viewBox units) for a press to still read as a tap. Generous enough
 * for finger wobble, but far below the 44-unit node spacing — so no genuine path drag can be
 * misread as a tap (a path must enter a second node anyway; the slop only separates
 * "deliberate tap" from "wandered off and gave up").
 */
export const TAP_SLOP = 10;

export type SigilGesture = "path" | "colour" | "none";

/**
 * `stroke` is the latched-node sequence of one pointer press, `travel` the maximum distance
 * the pointer strayed from its down-point. Two latched nodes make a path stroke regardless of
 * travel; one node within the slop is a colour tap; one node with a long wander is NEITHER —
 * the user roamed dead space and found nothing, and surprising them with a colour flip on
 * release would make the colour factor feel haunted. Zero nodes is nothing at all.
 */
export function classifyGesture(stroke: number[], travel: number): SigilGesture {
  if (stroke.length >= 2) return "path";
  if (stroke.length === 1 && travel <= TAP_SLOP) return "colour";
  return "none";
}

/**
 * Strokes that never grew a segment (a stray tap that latched one node) are dropped rather
 * than encoded: an accidental touch must not fork the secret, and the UI mirrors this by
 * refusing to commit them in the first place.
 */
export const normalizeStrokes = (strokes: number[][]): number[][] =>
  strokes.filter((s) => s.length >= 2);

/** Path wire format: node indices "-"-joined within a stroke, strokes "_"-joined: "0-7-13_2-9". */
export const encodeSigilPath = (strokes: number[][]): string =>
  normalizeStrokes(strokes)
    .map((s) => s.join("-"))
    .join("_");

/** Segments = pen-down node-to-node hops. The entropy unit, and the "how long is it" readout. */
export const segmentCount = (strokes: number[][]): number =>
  normalizeStrokes(strokes).reduce((n, s) => n + s.length - 1, 0);

/**
 * Emoji → lowercase-hex codepoints, "-"-joined: "🐱" → "1f431", "🐈‍⬛" → "1f408-200d-2b1b".
 * By CODEPOINT, never by catalog index — a picker reorder between builds must not be able to
 * silently re-key every vault. Normalization is pinned to ONE policy so every route an emoji
 * can arrive by (our picker, the OS keyboard, paste) yields the same secret:
 *
 *   - NFC first: composition differences are presentation, not identity.
 *   - VS16 (U+FE0F) is ALWAYS STRIPPED. Pickers and keyboards disagree about emitting it, and
 *     "same-looking emoji, different bytes" here means a vault that will not open.
 *   - Skin-tone modifiers (U+1F3FB–U+1F3FF) are ALWAYS STRIPPED, for the same reason: 👍 and
 *     👍🏽 collapse to one secret, because a thumbs-up re-picked on another day must reopen the
 *     vault whichever variant that day's picker offered. Costs a couple of entropy bits; the
 *     meter is pessimistic anyway.
 *   - ZWJ (U+200D) is KEPT: it is structural — stripping it would fold 🐈‍⬛ into 🐈 + ⬛.
 */
export function encodeEmoji(raw: string): string {
  const cps: number[] = [];
  for (const ch of raw.normalize("NFC").trim()) {
    const cp = ch.codePointAt(0)!;
    if (cp === 0xfe0f || (cp >= 0x1f3fb && cp <= 0x1f3ff)) continue;
    cps.push(cp);
  }
  return cps.map((cp) => cp.toString(16)).join("-");
}

/** Upper bound on the emoji set — enforced in the UI and again here, so the encoder can never
 * be talked into an unbounded field even if the UI check regresses. */
export const MAX_SIGIL_EMOJI = 8;

/**
 * The emoji factor is a SET, encoded canonically: each element normalized by `encodeEmoji`,
 * empties dropped, de-duplicated (👍 and 👍🏽 normalize to the SAME element, so they collapse
 * here too), then SORTED and "+"-joined — "+" mirrors how melody.ts joins chord tones, and
 * stays outside the ":" field alphabet.
 *
 * Sorting is the load-bearing part: with toggle interaction, deselecting and reselecting an
 * emoji reorders the selection array, and encoding insertion order would silently fork the
 * secret — the exact lesson `normalizeEvent` teaches about fingering order. The sort is plain
 * lexicographic over the encoded hex strings (NOT numeric); any deterministic order would do,
 * what matters is that it is canonical and pinned by tests. Oversized sets truncate to the
 * first MAX_SIGIL_EMOJI elements AFTER canonicalization, so even the (UI-prevented) overflow
 * case stays deterministic.
 */
export function encodeEmojiSet(raws: string[]): string {
  const set = [...new Set(raws.map(encodeEmoji).filter(Boolean))].sort();
  return set.slice(0, MAX_SIGIL_EMOJI).join("+");
}

/** NFC + whitespace trim; case is PRESERVED — the word is a password, not a search term. */
export const normalizeWord = (raw: string): string => raw.normalize("NFC").trim();

// --- Node colours --------------------------------------------------------------------------
// Every lattice node carries one of four marks, cycled by tapping (or pressing C on a focused
// node). The mark layer is fully INDEPENDENT of the path: marking a node does not add it to a
// stroke, and a node can be marked whether or not the path ever visits it.

export const SIGIL_COLORS = 4;
/** Accessible names for the four marks. Each variant differs in SHAPE as well as hue (plain /
 * dot / ring / diamond), so hue is never the only signal — see the render layer. */
export const COLOR_NAMES = ["plain", "green dot", "gold ring", "blue diamond"];

/**
 * Colour wire format: a FIXED-LENGTH string of 19 digits 0–3, indexed by node — e.g.
 * "0002000130000000000". Fixed width makes the field trivially injective and order-free, and
 * the all-default colouring encodes as its literal all-zeros string rather than being omitted,
 * so the field never changes shape. Input is defensively clamped (missing → 0, out-of-range →
 * wrapped) so a malformed array still encodes deterministically; the UI only ever produces
 * 0..3.
 */
export const encodeColors = (colors: number[]): string =>
  Array.from({ length: LATTICE.length }, (_, i) => ((Math.trunc(colors[i] ?? 0) % SIGIL_COLORS) + SIGIL_COLORS) % SIGIL_COLORS).join("");

/** How many nodes carry a non-default mark — the entropy unit for the colour layer. */
export const coloredCount = (colors: number[]): number =>
  [...encodeColors(colors)].filter((c) => c !== "0").length;

/**
 * The full scheme-prefixed secret, or "" when any REQUIRED factor is missing (an empty ""
 * can never unlock — the KDF is never even invoked on it). Path, emoji set and word are
 * required; the colour layer is optional and all-zeros is a perfectly valid colouring.
 *
 *   sigil:v1:<path>:<colours-19>:<emoji-set>:<word-length>:<word>
 *
 * (Amended IN PLACE while sigil:v1 has never sealed a committed vault — the emoji field grew
 * into a set and the colour field was inserted without a version bump, which is only legal
 * because no v1 secret can exist outside this working tree.)
 *
 * Delimiting is the trap here: the word is free text and may itself contain ":", so a naive
 * join would let two different inputs collide into one string. The path alphabet is [0-9 - _],
 * the colour field [0-3] at fixed width, and the emoji alphabet [0-9 a-f - +] — none can
 * contain ":" — and the word is LENGTH-PREFIXED (UTF-16 code units of the normalized word,
 * the unit "".length counts in), which makes the encoding injective by construction rather
 * than by an argument about what a word probably won't contain.
 */
export function encodeSigil(strokes: number[][], colors: number[], emojis: string[], word: string): string {
  const path = encodeSigilPath(strokes);
  const e = encodeEmojiSet(emojis);
  const w = normalizeWord(word);
  if (!path || !e || !w) return "";
  return `sigil:v1:${path}:${encodeColors(colors)}:${e}:${w.length}:${w}`;
}

/**
 * Entropy estimate for the strength meter — deliberately pessimistic, in melodyBits' spirit:
 * this number is what talks someone out of a two-segment sigil, so err low.
 *
 *   - 2.5 bits per path segment. 18 possible next nodes sounds like >4 bits, but human-drawn
 *     lattice paths are nothing like uniform: adjacent hops, smooth turns, symmetric figures
 *     and 12-o'clock starts dominate (Android 3×3 pattern studies measure realized entropy at
 *     a small fraction of the combinatorial bound). 6–10 segments is the intended length; the
 *     other factors carry weight so the sigil doesn't have to.
 *   - Emoji: 6 bits for the first, only +3 per additional, capped at 15. The catalog is ~170
 *     wide (7.4 bits uniform) but people pick from a small popular head — hearts, cats, fire —
 *     and a second pick comes from the SAME head, correlated with the first (cat people pick
 *     two cats), so the marginal picks are worth half the first at best.
 *   - The word: 4 bits for the first character + 2 per additional (counted in codepoints),
 *     capped at 20 — short dictionary words dominate, and the cap stops one long word from
 *     painting the meter green by itself. If you have a genuinely strong passphrase, the
 *     passphrase lock is the right home for it.
 *   - Colours: 1.5 bits per NON-DEFAULT node, capped at 12. Combinatorially the layer is 19 ×
 *     2 = 38 bits, but almost nobody marks many nodes, and those who do choose symmetric,
 *     ring-aligned arrangements — count only what was actually marked, and harshly.
 *
 * Zero when any required factor is missing, because the secret is then "" and cannot unlock.
 */
export function sigilBits(strokes: number[][], colors: number[], emojis: string[], word: string): number {
  if (!encodeSigil(strokes, colors, emojis, word)) return 0;
  const wordCps = [...normalizeWord(word)].length;
  const nEmoji = encodeEmojiSet(emojis).split("+").length;
  return Math.round(
    segmentCount(strokes) * 2.5 +
      Math.min(15, 6 + 3 * (nEmoji - 1)) +
      Math.min(20, 4 + 2 * (wordCps - 1)) +
      Math.min(12, 1.5 * coloredCount(colors)),
  );
}

// --- Ring inscription ----------------------------------------------------------------------
// The magic word is the ONE factor a shoulder-surfer cannot capture (the path, marks and
// emoji are drawn in the open), so the ring must never render it — and it must not leak the
// LENGTH either. Two designs that look safe and are not:
//
//   - a substitution cipher (1:1 glyph mapping is readable from a photo);
//   - one rune per character REPEATED around the ring (a tiled sequence has a visible period —
//     count runes until the pattern repeats and you have the exact length back).
//
// So from the FIRST character onward the ring holds a CONSTANT number of runes filling the
// full circumference, identical for a one-character word and a thirty-character one: no
// per-character glyphs, no period, no length signal at all. An EMPTY word renders no ring —
// that distinguishes only empty from non-empty, a single bit the disabled Unlock button
// already gives away (an empty word can't produce a valid secret at all), so nothing new
// leaks. Rune identities are derived from (session seed, word), so the whole inscription
// visibly reshuffles as the user types — the "my word is being inscribed" feedback survives —
// but the seed is random, reseeded on every mount, and never displayed, so nothing about the
// word is recoverable from a photo or a recording of the ring. An explicit opt-in toggle
// renders the real word for users who don't care.

/** Elder Futhark, 24 runes. Decorative alphabet only — rune choice never feeds the secret. */
export const RING_RUNES = [..."ᚠᚢᚦᚨᚱᚲᚷᚹᚺᚾᛁᛃᛇᛈᛉᛊᛏᛒᛖᛗᛚᛜᛞᛟ"];

/**
 * How many runes the ring always holds. A layout constant, not a function of anything the
 * user typed — that independence IS the security property, pinned by tests. The renderer
 * stretches the run to the full circumference with textLength, so any count in this ballpark
 * fills the circle.
 */
export const RING_GLYPH_COUNT = 42;

/** FNV-1a over UTF-16 code units: tiny, deterministic, spreads similar words apart. */
const fnv1a = (s: string): number => {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h >>> 0;
};

/**
 * Empty word ⇒ NO runes (empty-vs-non-empty is one already-public bit — see the block comment
 * above); otherwise exactly RING_GLYPH_COUNT runes from (seed, word), regardless of length:
 * same inputs ⇒ same inscription, any keystroke ⇒ a visibly different one. mulberry32 over
 * seed⊕fnv1a(word) inside — tiny and NOT cryptographic, which is fine because the ring is
 * one-way BY THE SEED, not by the hash: the seed is session-random, reseeded each mount and
 * never rendered, so an observer holds 42 runes of a keyed function whose key they will never
 * see again.
 */
export function ringGlyphs(word: string, seed: number): string[] {
  const w = normalizeWord(word);
  if (!w) return [];
  let a = ((seed >>> 0) ^ fnv1a(w)) >>> 0;
  const out: string[] = [];
  for (let i = 0; i < RING_GLYPH_COUNT; i++) {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    out.push(RING_RUNES[((t ^ (t >>> 14)) >>> 0) % RING_RUNES.length]);
  }
  return out;
}

// --- Assembly geometry ---------------------------------------------------------------------
// Cosmetic helpers for composing the circle: where the repeated emoji sit, and the circular
// path the ring text runs along. Nothing below feeds the secret.

/** `n` points evenly around radius `r`, k=0 at 12 o'clock (+phase degrees), clockwise. */
export const ringPoints = (n: number, r: number, phaseDeg = 0): { x: number; y: number }[] =>
  Array.from({ length: n }, (_, k) => {
    const a = ((phaseDeg + (k * 360) / n - 90) * Math.PI) / 180;
    return { x: SIGIL_C + r * Math.cos(a), y: SIGIL_C + r * Math.sin(a) };
  });

/**
 * A full circle as a <path> (an SVG <circle> cannot host a <textPath>): starts at 12 o'clock
 * and runs clockwise, so the inscription reads left-to-right across the top of the ring. The
 * -0.01 keeps the endpoints from coinciding, which would make the arc degenerate.
 */
export const ringPathD = (r: number): string =>
  `M ${SIGIL_C} ${SIGIL_C - r} a ${r} ${r} 0 1 1 -0.01 0`;
