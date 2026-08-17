// Unit tests for the sigil unlock lock's encoding + lattice geometry.
//
// Run with `npm test` (Node's built-in runner + type stripping — no extra dependencies).
//
// As with melody.test.ts, the two halves are not equally serious:
//
//   ENCODING is security-critical. `encodeSigil` feeds the vault KDF directly, so a change to
//   its output silently locks every existing sigil vault out — there is no recovery path. The
//   tests pin the exact bytes for known inputs, pin that incidental representation differences
//   (emoji variation selectors, skin tones, NFC forms, the ORDER emoji were toggled in) cannot
//   fork the secret, and pin that things which MUST be distinct secrets (direction, stroke
//   boundaries, node marks, a word containing the field delimiter) really are.
//
//   GEOMETRY is cosmetic in pixels but the node INDICES are the wire format, so the lattice
//   shape, the hit-test hysteresis (disjoint catch discs) and the tap-vs-drag classifier get
//   pinned too: if a refactor moved a node or grew the catch radius past half the minimum
//   spacing, an old sigil could re-trace to different indices — the same disaster as changing
//   the encoding.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  SIGIL_C, R_INNER, R_OUTER, NODE_R, CATCH_R, TAP_SLOP, LATTICE, nodeLabel, hitNode, appendHit,
  classifyGesture, normalizeStrokes, encodeSigilPath, segmentCount, encodeEmoji, encodeEmojiSet,
  MAX_SIGIL_EMOJI, encodeColors, coloredCount, SIGIL_COLORS, COLOR_NAMES, normalizeWord,
  encodeSigil, sigilBits, RING_RUNES, RING_GLYPH_COUNT, ringGlyphs, ringPoints, ringPathD,
} from "./sigil.ts";

const dist = (a: { x: number; y: number }, b: { x: number; y: number }): number =>
  Math.hypot(a.x - b.x, a.y - b.y);

const Z = Array(19).fill(0); // the all-default colouring
const Z19 = "0".repeat(19); // …and its literal wire form

// --- Encoding (security-critical) -----------------------------------------------------------

test("encodeSigil pins the exact v1 wire format", () => {
  assert.equal(
    encodeSigil([[0, 7, 13], [2, 9]], Z, ["🐱"], "mrrp"),
    `sigil:v1:0-7-13_2-9:${Z19}:1f431:4:mrrp`,
  );
  // Colours land as their own fixed-width field; multiple emoji join the set field with "+".
  const C = [...Z];
  C[3] = 2;
  C[7] = 1;
  C[8] = 3;
  assert.equal(
    encodeSigil([[1, 0]], C, ["🔥", "🐱"], "x"),
    "sigil:v1:1-0:0002000130000000000:1f431+1f525:1:x",
  );
});

test("empty input in any REQUIRED factor encodes to \"\" — it must never be able to unlock", () => {
  assert.equal(encodeSigil([], Z, ["🐱"], "word"), ""); // no sigil at all
  assert.equal(encodeSigil([[4]], Z, ["🐱"], "word"), ""); // a stray tap is not a sigil
  assert.equal(encodeSigil([[0, 1]], Z, [], "word"), ""); // no emoji
  assert.equal(encodeSigil([[0, 1]], Z, ["", "\u{FE0F}"], "word"), ""); // nothing survives normalization
  assert.equal(encodeSigil([[0, 1]], Z, ["🐱"], ""), ""); // no word
  assert.equal(encodeSigil([[0, 1]], Z, ["🐱"], "   "), ""); // whitespace is not a word
});

test("colours are NOT required — the all-default colouring is a valid secret, encoded literally", () => {
  const s = encodeSigil([[0, 1]], Z, ["🐱"], "w");
  assert.ok(s.includes(`:${Z19}:`)); // present as its all-zeros string, never omitted
  assert.ok(s.length > 0);
});

test("emoji encodes by codepoint, never by catalog index", () => {
  assert.equal(encodeEmoji("🐱"), "1f431");
  assert.equal(encodeEmoji("👍"), "1f44d");
  assert.equal(encodeEmoji("⭐"), "2b50");
});

test("VS16 and skin-tone modifiers are stripped; the three thumbs-up forms are ONE secret", () => {
  // Escapes, not literals: an editor or a copy-paste could silently drop the invisible
  // codepoints these tests exist to exercise.
  assert.equal(encodeEmoji("\u{1F44D}"), "1f44d"); // bare 👍
  assert.equal(encodeEmoji("\u{1F44D}\u{FE0F}"), "1f44d"); // 👍 + explicit emoji-presentation VS16
  assert.equal(encodeEmoji("\u{1F44D}\u{1F3FD}"), "1f44d"); // 👍🏽 medium skin tone
  const forms = ["\u{1F44D}", "\u{1F44D}\u{FE0F}", "\u{1F44D}\u{1F3FD}"].map((e) =>
    encodeSigil([[0, 1]], Z, [e], "w"),
  );
  assert.equal(forms[0], forms[1]);
  assert.equal(forms[1], forms[2]);
  assert.equal(encodeEmoji("\u{2764}\u{FE0F}"), "2764"); // ❤️ — the VS16 goes, the heart stays
});

test("ZWJ is structural and is kept — a black cat is not a cat plus a square", () => {
  assert.equal(encodeEmoji("\u{1F408}\u{200D}\u{2B1B}"), "1f408-200d-2b1b"); // 🐈‍⬛
  assert.notEqual(encodeEmoji("\u{1F408}\u{200D}\u{2B1B}"), encodeEmoji("\u{1F408}"));
});

test("the emoji set is canonical: pick order cannot fork the secret", () => {
  // Toggle interaction reorders the selection array freely (deselect + reselect moves an emoji
  // to the end), so insertion order MUST be invisible — same lesson normalizeEvent teaches
  // about fingering order on the melody lock.
  assert.equal(encodeEmojiSet(["🔥", "🐱"]), "1f431+1f525"); // sorted by encoded string…
  assert.equal(encodeEmojiSet(["🐱", "🔥"]), "1f431+1f525"); // …whatever order they were picked
  assert.equal(
    encodeSigil([[0, 1]], Z, ["🔥", "🐱"], "w"),
    encodeSigil([[0, 1]], Z, ["🐱", "🔥"], "w"),
  );
});

test("the emoji set de-duplicates, including duplicates that only normalization reveals", () => {
  assert.equal(encodeEmojiSet(["🐱", "🐱"]), "1f431");
  // 👍 and 👍🏽 normalize to the same element, so they collapse to ONE set member too.
  assert.equal(encodeEmojiSet(["\u{1F44D}", "\u{1F44D}\u{1F3FD}"]), "1f44d");
  // Empty / vanishing elements drop out rather than encoding as empty set members.
  assert.equal(encodeEmojiSet(["", "\u{FE0F}", "🐱"]), "1f431");
});

test("the emoji set caps at MAX_SIGIL_EMOJI, deterministically, after canonicalization", () => {
  const nine = ["🐱", "🌙", "⭐", "🔥", "🍀", "💀", "🌿", "🍄", "⚡"]; // 9 distinct
  const enc = encodeEmojiSet(nine);
  assert.equal(enc.split("+").length, MAX_SIGIL_EMOJI);
  assert.equal(enc, encodeEmojiSet([...nine].reverse())); // truncation is order-independent too
});

test("a magic word containing the field separator cannot collide two secrets", () => {
  const s = encodeSigil([[0, 1]], Z, ["🐱"], "a:b:c");
  assert.equal(s, `sigil:v1:0-1:${Z19}:1f431:5:a:b:c`);
  // The length prefix makes the parse unambiguous: strip the scheme, read path, colours and
  // emoji (whose alphabets exclude ":"), then take exactly <len> code units — the recovered
  // word is intact.
  const rest = s.slice("sigil:v1:".length);
  const [path, colors, emoji, len] = rest.split(":", 4);
  const word = rest.slice(path.length + colors.length + emoji.length + String(len).length + 4);
  assert.equal(word.length, Number(len));
  assert.equal(word, "a:b:c");
  // And two inputs that a naive join could conflate stay distinct secrets.
  assert.notEqual(
    encodeSigil([[0, 1]], Z, ["🐱"], "a:b"),
    encodeSigil([[0, 1]], Z, ["🐱"], "a"),
  );
});

test("the word is NFC-normalized and trimmed, and case is preserved", () => {
  // Escapes, not literals: the two “café” forms must genuinely differ in bytes to test NFC.
  const composed = encodeSigil([[0, 1]], Z, ["🐱"], "caf\u{e9}"); // é as one codepoint
  const decomposed = encodeSigil([[0, 1]], Z, ["🐱"], "cafe\u{301}"); // e + combining acute
  assert.equal(composed, decomposed);
  assert.equal(composed, `sigil:v1:0-1:${Z19}:1f431:4:caf\u{e9}`); // …and the length prefix is 4, not 5
  assert.equal(encodeSigil([[0, 1]], Z, ["🐱"], "  padded  "), `sigil:v1:0-1:${Z19}:1f431:6:padded`);
  assert.notEqual(encodeSigil([[0, 1]], Z, ["🐱"], "Word"), encodeSigil([[0, 1]], Z, ["🐱"], "word"));
});

test("direction is significant — A→B is not B→A", () => {
  assert.equal(encodeSigilPath([[0, 1]]), "0-1");
  assert.equal(encodeSigilPath([[1, 0]]), "1-0");
  assert.notEqual(encodeSigil([[0, 1]], Z, ["🐱"], "w"), encodeSigil([[1, 0]], Z, ["🐱"], "w"));
});

test("stroke boundaries are significant — one stroke is not two strokes over the same nodes", () => {
  assert.equal(encodeSigilPath([[0, 1, 2]]), "0-1-2");
  assert.equal(encodeSigilPath([[0, 1], [1, 2]]), "0-1_1-2");
  assert.notEqual(
    encodeSigil([[0, 1, 2]], Z, ["🐱"], "w"),
    encodeSigil([[0, 1], [1, 2]], Z, ["🐱"], "w"),
  );
});

test("two-digit indices cannot be confused with two one-digit hops", () => {
  assert.notEqual(encodeSigilPath([[1, 2, 3]]), encodeSigilPath([[12, 3]]));
});

test("appendHit ignores misses and same-node jitter but keeps genuine returns", () => {
  assert.deepEqual(appendHit([0, 7], -1), [0, 7]); // between nodes: nothing latches
  assert.deepEqual(appendHit([0, 7], 7), [0, 7]); // wobble inside one catch disc
  assert.deepEqual(appendHit([0, 7], 0), [0, 7, 0]); // A→B→A is a real, legal path
  assert.deepEqual(appendHit([], 5), [5]);
});

test("normalizeStrokes drops tap-only strokes and preserves order", () => {
  assert.deepEqual(normalizeStrokes([[3], [0, 1], [7], [2, 9]]), [[0, 1], [2, 9]]);
  assert.equal(encodeSigilPath([[3]]), "");
  assert.equal(encodeSigilPath([]), "");
});

// --- Node colours ---------------------------------------------------------------------------

test("encodeColors is always exactly 19 digits, whatever shape the input is", () => {
  assert.equal(encodeColors([]), Z19);
  assert.equal(encodeColors([1]).length, 19);
  assert.equal(encodeColors([1]), "1" + "0".repeat(18));
  assert.equal(encodeColors(Array(30).fill(2)).length, 19); // over-length input truncates
});

test("encodeColors clamps defensively — the UI only produces 0–3, but garbage still encodes deterministically", () => {
  assert.equal(encodeColors([4])[0], "0"); // wraps mod 4
  assert.equal(encodeColors([-1])[0], "3");
  assert.equal(encodeColors([2.9])[0], "2"); // truncates, never emits fractions
});

test("a node mark changes the secret; marking is independent of the path", () => {
  const plain = encodeSigil([[0, 1]], Z, ["🐱"], "w");
  const marked = [...Z];
  marked[14] = 3; // a node the path never visits — still a legal, meaningful mark
  assert.notEqual(plain, encodeSigil([[0, 1]], marked, ["🐱"], "w"));
});

test("coloredCount counts only non-default marks", () => {
  assert.equal(coloredCount(Z), 0);
  const c = [...Z];
  c[0] = 1;
  c[18] = 3;
  assert.equal(coloredCount(c), 2);
  assert.equal(COLOR_NAMES.length, SIGIL_COLORS); // every variant has an accessible name
});

// --- Gesture classification -----------------------------------------------------------------

test("classifyGesture: a second node makes a path, however short the travel", () => {
  assert.equal(classifyGesture([0, 7], 0), "path");
  assert.equal(classifyGesture([0, 7], 300), "path");
  assert.equal(classifyGesture([0, 7, 0], 120), "path");
});

test("classifyGesture: one node within the slop is a colour tap — boundary inclusive", () => {
  assert.equal(classifyGesture([5], 0), "colour");
  assert.equal(classifyGesture([5], TAP_SLOP), "colour");
});

test("classifyGesture: a long wander that never found a second node is nothing at all", () => {
  assert.equal(classifyGesture([5], TAP_SLOP + 0.1), "none");
  assert.equal(classifyGesture([], 2), "none"); // pressed dead space
});

test("TAP_SLOP stays small enough that a tap can never span toward another node", () => {
  assert.ok(TAP_SLOP <= 2 * CATCH_R); // a tap lives inside one node's neighbourhood
});

// --- Lattice geometry -----------------------------------------------------------------------

test("the lattice is 1 + 6 + 12 nodes with frozen index assignment", () => {
  assert.equal(LATTICE.length, 19);
  assert.deepEqual(LATTICE.map((n) => n.ring), [0, ...Array(6).fill(1), ...Array(12).fill(2)]);
  // Index 0 dead centre; 1 and 7 straight up from it — pinned so a refactor can't renumber.
  assert.equal(LATTICE[0].x, SIGIL_C);
  assert.equal(LATTICE[0].y, SIGIL_C);
  assert.ok(Math.abs(LATTICE[1].x - SIGIL_C) < 1e-9 && Math.abs(LATTICE[1].y - (SIGIL_C - R_INNER)) < 1e-9);
  assert.ok(Math.abs(LATTICE[7].x - SIGIL_C) < 1e-9 && Math.abs(LATTICE[7].y - (SIGIL_C - R_OUTER)) < 1e-9);
});

test("every ring node sits exactly on its ring radius", () => {
  const c = { x: SIGIL_C, y: SIGIL_C };
  for (let i = 1; i <= 6; i++) assert.ok(Math.abs(dist(LATTICE[i], c) - R_INNER) < 1e-9);
  for (let i = 7; i <= 18; i++) assert.ok(Math.abs(dist(LATTICE[i], c) - R_OUTER) < 1e-9);
});

test("catch discs are disjoint and smaller than the node art — the hysteresis invariants", () => {
  let min = Infinity;
  for (let i = 0; i < LATTICE.length; i++) {
    for (let j = i + 1; j < LATTICE.length; j++) min = Math.min(min, dist(LATTICE[i], LATTICE[j]));
  }
  assert.ok(min > 2 * CATCH_R); // no point can be inside two catch discs at once
  assert.ok(CATCH_R < NODE_R); // you must reach a node's INNER radius, not just graze its art
});

test("hitNode latches inside the catch radius and nowhere else", () => {
  assert.equal(hitNode(LATTICE[0].x, LATTICE[0].y), 0);
  assert.equal(hitNode(LATTICE[13].x, LATTICE[13].y), 13);
  assert.equal(hitNode(LATTICE[7].x + CATCH_R - 0.5, LATTICE[7].y), 7); // just inside
  assert.equal(hitNode(LATTICE[7].x + CATCH_R + 0.5, LATTICE[7].y), -1); // just outside
  // Midpoints between neighbours are dead zone: this is what makes the snap hard.
  assert.equal(hitNode(SIGIL_C, SIGIL_C - R_INNER / 2), -1); // centre ↔ inner-top
  const mid = { x: (LATTICE[7].x + LATTICE[8].x) / 2, y: (LATTICE[7].y + LATTICE[8].y) / 2 };
  assert.equal(hitNode(mid.x, mid.y), -1); // adjacent outer nodes
});

test("nodeLabel names every node for the keyboard path", () => {
  assert.equal(nodeLabel(0), "centre");
  assert.equal(nodeLabel(1), "inner ring 1 of 6");
  assert.equal(nodeLabel(7), "outer ring 1 of 12");
  assert.equal(nodeLabel(18), "outer ring 12 of 12");
});

// --- Strength meter -------------------------------------------------------------------------

test("sigilBits is zero unless all REQUIRED factors are present", () => {
  assert.equal(sigilBits([], Z, ["🐱"], "word"), 0);
  assert.equal(sigilBits([[0, 1]], Z, [], "word"), 0);
  assert.equal(sigilBits([[0, 1]], Z, ["🐱"], ""), 0);
});

test("sigilBits pins the model: 2.5/segment + 6 emoji + 4+2/char word + 1.5/marked node", () => {
  // 2 segments (5) + one emoji (6) + 4-char word (4 + 2*3 = 10) = 21, no marks.
  assert.equal(sigilBits([[0, 1], [1, 2]], Z, ["🐱"], "mrrp"), 21);
  // Two marked nodes add 3.
  const c2 = [...Z];
  c2[3] = 1;
  c2[4] = 2;
  assert.equal(sigilBits([[0, 1], [1, 2]], c2, ["🐱"], "mrrp"), 24);
});

test("extra emoji are worth less than the first, and the emoji term caps", () => {
  const one = sigilBits([[0, 1], [1, 2]], Z, ["🐱"], "mrrp");
  const two = sigilBits([[0, 1], [1, 2]], Z, ["🐱", "🔥"], "mrrp");
  assert.equal(two - one, 3); // marginal pick: half the first pick's 6 bits
  const four = sigilBits([[0, 1]], Z, ["🐱", "🔥", "🌙", "⭐"], "w"); // 6+3*3 = 15: at the cap
  const eight = sigilBits([[0, 1]], Z, ["🐱", "🔥", "🌙", "⭐", "🍀", "💀", "🌿", "🍄"], "w");
  assert.equal(four, eight); // …and it stays there
});

test("sigilBits grows with segments and word length, and the word contribution caps", () => {
  const base = sigilBits([[0, 1], [1, 2]], Z, ["🐱"], "mrrp");
  assert.ok(sigilBits([[0, 1], [1, 2], [2, 3]], Z, ["🐱"], "mrrp") > base); // more segments
  assert.ok(sigilBits([[0, 1], [1, 2]], Z, ["🐱"], "mrrpmrrp") > base); // longer word
  const capped = sigilBits([[0, 1]], Z, ["🐱"], "x".repeat(9)); // 4 + 2*8 = 20: at the cap
  assert.equal(sigilBits([[0, 1]], Z, ["🐱"], "x".repeat(40)), capped); // …and it stays there
});

test("the colour term counts only marked nodes and caps hard", () => {
  const eight = [...Z];
  for (let i = 0; i < 8; i++) eight[i] = 1; // 8 × 1.5 = 12: at the cap
  const all = Array(19).fill(2);
  const withEight = sigilBits([[0, 1]], eight, ["🐱"], "w");
  assert.equal(withEight, sigilBits([[0, 1]], all, ["🐱"], "w")); // 19 marks credit no more
  assert.equal(withEight - sigilBits([[0, 1]], Z, ["🐱"], "w"), 12);
});

// --- Ring inscription (cosmetic, but the leak policy is pinned) -------------------------------

test("empty word ⇒ no runes; every length ≥ 1 ⇒ the SAME constant count", () => {
  // A per-character inscription leaks length outright; a REPEATED sequence leaks it via its
  // period. Constant count is the only shape with nothing to measure. The empty case hides
  // the ring entirely — that reveals only empty-vs-non-empty, a single bit the disabled
  // Unlock button already gives away (an empty word can't form a valid secret at all).
  assert.deepEqual(ringGlyphs("", 7), []);
  assert.deepEqual(ringGlyphs("   ", 7), []); // whitespace normalizes to empty, same as the encoder
  assert.equal(ringGlyphs("x", 7).length, RING_GLYPH_COUNT);
  assert.equal(ringGlyphs("xy", 7).length, RING_GLYPH_COUNT);
  assert.equal(ringGlyphs("x".repeat(30), 7).length, RING_GLYPH_COUNT);
});

test("ringGlyphs is deterministic per (word, seed) and drawn from the rune set", () => {
  const a = ringGlyphs("mrrp", 0xdecafbad);
  assert.deepEqual(a, ringGlyphs("mrrp", 0xdecafbad));
  for (const g of a) assert.ok(RING_RUNES.includes(g));
});

test("typing visibly changes the inscription — the feedback channel survives", () => {
  assert.notDeepEqual(ringGlyphs("a", 7), ringGlyphs("ab", 7));
  assert.notDeepEqual(ringGlyphs("a", 7), ringGlyphs("b", 7));
});

test("the same word under two different session seeds yields different runes", () => {
  // 42 runes from a 24-glyph alphabet colliding across two pinned seeds would be ~2^-192 luck;
  // the assertion is deterministic once it passes at all. This is the non-invertibility story:
  // the observable depends on a seed that is reseeded every mount and never rendered.
  assert.notDeepEqual(ringGlyphs("mrrp", 1), ringGlyphs("mrrp", 2));
});

// --- Assembly geometry (cosmetic) -------------------------------------------------------------

test("ringPoints spreads n points on the radius, starting at 12 o'clock", () => {
  const r = 124;
  const pts = ringPoints(12, r);
  assert.equal(pts.length, 12);
  const c = { x: SIGIL_C, y: SIGIL_C };
  for (const p of pts) assert.ok(Math.abs(dist(p, c) - r) < 1e-9);
  assert.ok(Math.abs(pts[0].x - SIGIL_C) < 1e-9 && Math.abs(pts[0].y - (SIGIL_C - r)) < 1e-9);
});

test("ringPathD is a non-degenerate full-circle arc at the requested radius", () => {
  const d = ringPathD(110);
  assert.ok(d.startsWith(`M ${SIGIL_C} ${SIGIL_C - 110} a 110 110`));
  assert.ok(d.includes("1 1")); // large-arc + clockwise sweep: text reads across the top
});

test("normalizeWord trims and NFC-composes without touching case", () => {
  assert.equal(normalizeWord("  Word  "), "Word");
  assert.equal(normalizeWord("cafe\u{301}"), "caf\u{e9}");
});

test("segmentCount counts hops across all real strokes", () => {
  assert.equal(segmentCount([]), 0);
  assert.equal(segmentCount([[4]]), 0);
  assert.equal(segmentCount([[0, 7, 13], [2, 9]]), 3);
});
