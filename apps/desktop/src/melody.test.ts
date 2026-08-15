// Unit tests for the melody unlock lock's theory + engraving.
//
// Run with `npm test` (Node's built-in runner + type stripping — no extra dependencies).
//
// Two things are worth pinning here, and they are not equally serious:
//
//   ENCODING is security-critical. `encodeMelody` feeds the vault KDF directly, so a change to
//   its output silently locks every existing melody vault out — there is no recovery path. The
//   tests below pin the exact bytes for known sequences, pin that incidental things (finger
//   order, a doubled note) cannot fork the secret, and pin that the schemes which CAN collide
//   (rhythm on vs off, one octave vs another) actually stay distinct.
//
//   ENGRAVING is cosmetic, but it is the thing that makes an absolute-pitch lock learnable, so
//   the grand-staff invariants get pinned too: staff lines on even diatonic steps, middle C
//   exactly between the two staves, ledger lines only where a note really leaves the staff.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  type MelodyEvent, noteName, durClass, DUR_MAX_MS, normalizeEvent, encodeEvent, encodeMelody,
  melodyBits, chordName, staffStep, yOf, ledgersFor, buildSheet, scoreText, TREBLE_LINES,
  BASS_LINES, STAFF_TOP, STAFF_BOT,
} from "./melody.ts";

const ev = (notes: number[], dur = 1): MelodyEvent => normalizeEvent(notes, dur);

// --- Naming ---------------------------------------------------------------------------------

test("noteName is absolute scientific pitch", () => {
  assert.equal(noteName(60), "C4"); // middle C
  assert.equal(noteName(69), "A4"); // A440
  assert.equal(noteName(84), "C6");
  assert.equal(noteName(61), "C#4");
  assert.equal(noteName(21), "A0"); // bottom of an 88-key board
  assert.equal(noteName(108), "C8"); // top of one
});

// --- Duration classes -----------------------------------------------------------------------

test("durClass buckets hold time, and the last bucket is unbounded", () => {
  assert.equal(durClass(0), 0);
  assert.equal(durClass(DUR_MAX_MS[0] - 1), 0);
  assert.equal(durClass(DUR_MAX_MS[0]), 1); // boundaries are exclusive-upper
  assert.equal(durClass(DUR_MAX_MS[1] - 1), 1);
  assert.equal(durClass(DUR_MAX_MS[1]), 2);
  assert.equal(durClass(DUR_MAX_MS[2]), 3);
  assert.equal(durClass(60_000), 3); // a stuck key still classifies, never -1
});

// --- Encoding (security-critical) -------------------------------------------------------------

test("encodeMelody pins the exact v3 wire format", () => {
  assert.equal(encodeMelody([ev([60], 1), ev([64], 0)], true), "melody:v3:60.1-64.0");
  assert.equal(encodeMelody([ev([60, 64, 67], 2)], true), "melody:v3:60+64+67.2");
});

test("an empty sequence encodes to \"\" — it must never be able to unlock", () => {
  assert.equal(encodeMelody([], true), "");
  assert.equal(encodeMelody([], false), "");
});

test("finger order and doubled notes cannot fork the secret", () => {
  const a = encodeMelody([ev([67, 60, 64], 1)], true);
  const b = encodeMelody([ev([60, 64, 67], 1)], true);
  const c = encodeMelody([ev([64, 60, 67, 64], 1)], true); // same note hit twice in one grab
  assert.equal(a, b);
  assert.equal(b, c);
});

test("octave is part of the secret — C4 is not C5", () => {
  assert.notEqual(encodeMelody([ev([60])], true), encodeMelody([ev([72])], true));
});

test("rhythm off omits durations entirely, so it can't collide with rhythm on", () => {
  const seq = [ev([60], 1), ev([62], 1)];
  assert.equal(encodeMelody(seq, false), "melody:v3:60-62");
  assert.notEqual(encodeMelody(seq, false), encodeMelody(seq, true));
});

test("with rhythm on, the same pitches at different lengths are different secrets", () => {
  assert.notEqual(encodeMelody([ev([60], 0)], true), encodeMelody([ev([60], 3)], true));
});

test("a chord is not the same secret as its notes played in sequence", () => {
  assert.notEqual(
    encodeMelody([ev([60, 64], 1)], true),
    encodeMelody([ev([60], 1), ev([64], 1)], true),
  );
});

test("encodeEvent is the piece encodeMelody is built from", () => {
  assert.equal(encodeEvent(ev([60, 64], 2), true), "60+64.2");
  assert.equal(encodeEvent(ev([60, 64], 2), false), "60+64");
});

// --- Strength meter ---------------------------------------------------------------------------

test("melodyBits grows with length, chord tones and rhythm", () => {
  const one = melodyBits([ev([60])], false);
  assert.ok(one >= 4 && one <= 5); // log2(24) ≈ 4.6
  assert.ok(melodyBits([ev([60]), ev([62])], false) > one);
  assert.ok(melodyBits([ev([60, 64, 67])], false) > one);
  assert.ok(melodyBits([ev([60])], true) > one);
  assert.equal(melodyBits([], true), 0);
});

// --- Chord naming (cosmetic) --------------------------------------------------------------------

test("chordName names common triads and sevenths", () => {
  assert.equal(chordName([60, 64, 67]), "C");
  assert.equal(chordName([60, 63, 67]), "Cm");
  assert.equal(chordName([60, 63, 66]), "Cdim");
  assert.equal(chordName([60, 64, 68]), "Caug");
  assert.equal(chordName([60, 65, 67]), "Csus4");
  assert.equal(chordName([60, 64, 67, 71]), "Cmaj7");
  assert.equal(chordName([60, 64, 67, 70]), "C7");
  assert.equal(chordName([57, 60, 64, 67]), "Am7");
});

test("chordName reads inversions as slash chords over the actual bass", () => {
  assert.equal(chordName([64, 67, 72]), "C/E"); // first inversion of C major
  assert.equal(chordName([67, 72, 76]), "C/G"); // second inversion
});

test("chordName handles voicings spread over octaves", () => {
  assert.equal(chordName([48, 64, 67, 72]), "C"); // root doubled two octaves down
});

test("chordName degrades gracefully — a single note is unlabelled, a cluster still names", () => {
  assert.equal(chordName([60]), "");
  assert.equal(chordName([]), "");
  assert.equal(chordName([60, 67]), "C 5"); // power chord reads as an interval
  assert.equal(chordName([60, 72]), "C oct");
  assert.equal(chordName([60, 61, 62, 63, 66]), "C (5)"); // no shape matches; still describable
});

// --- Staff geometry -----------------------------------------------------------------------------

test("staffStep is diatonic — a sharp shares its natural's line", () => {
  assert.equal(staffStep(60), staffStep(61)); // C4 and C#4
  assert.equal(staffStep(60), 28); // middle C
  assert.equal(staffStep(62) - staffStep(60), 1); // C4 → D4 is one step
  assert.equal(staffStep(72) - staffStep(60), 7); // an octave is seven steps
});

test("every staff line sits on an even step, so lines and spaces alternate cleanly", () => {
  for (const s of [...TREBLE_LINES, ...BASS_LINES]) assert.equal(s % 2, 0);
});

test("staff lines are evenly spaced and middle C falls exactly between the staves", () => {
  const gaps = TREBLE_LINES.slice(1).map((s, i) => yOf(TREBLE_LINES[i]) - yOf(s));
  assert.deepEqual(gaps, [8, 8, 8, 8]);
  const e4 = yOf(30); // treble bottom line
  const a3 = yOf(26); // bass top line
  assert.equal(yOf(28), (e4 + a3) / 2); // C4 is midway — a genuine grand staff
  assert.equal(yOf(38), STAFF_TOP);
  assert.equal(yOf(18), STAFF_BOT);
});

test("ledger lines appear only where a note actually leaves the staff", () => {
  assert.deepEqual(ledgersFor(34), []); // B4, mid-treble
  assert.deepEqual(ledgersFor(22), []); // D3, mid-bass
  assert.deepEqual(ledgersFor(39), []); // the space just above the treble staff
  assert.deepEqual(ledgersFor(27), []); // B3, the space just above the bass staff
  assert.deepEqual(ledgersFor(29), []); // D4, the space just below the treble staff
  assert.deepEqual(ledgersFor(28), [28]); // middle C — the one line in the gap
  assert.deepEqual(ledgersFor(40), [40]); // A5
  assert.deepEqual(ledgersFor(44), [40, 42, 44]); // stacked upward
  assert.deepEqual(ledgersFor(14), [16, 14]); // and downward
});

// --- Engraving ------------------------------------------------------------------------------------

test("note values engrave correctly: filled/hollow, stems, flags", () => {
  const [eighth, quarter, half, whole] = buildSheet(
    [ev([60], 0), ev([60], 1), ev([60], 2), ev([60], 3)], true,
  ).events;
  assert.ok(eighth.filled && eighth.flag && eighth.stem);
  assert.ok(quarter.filled && !quarter.flag && quarter.stem);
  assert.ok(!half.filled && !half.flag && half.stem);
  assert.ok(!whole.filled && !whole.flag && whole.stem === null); // a whole note has no stem
});

test("with rhythm off every event engraves as a quarter — the score shows what's sealed", () => {
  for (const e of buildSheet([ev([60], 0), ev([60], 3)], false).events) {
    assert.ok(e.filled && !e.flag && e.stem);
  }
});

test("stems point away from the near edge of whichever staff the note sits on", () => {
  const up = buildSheet([ev([60])], true).events[0].stem!; // middle C, low in the treble
  assert.ok(up.y2 < up.y1); // y grows downward, so an upward stem ends higher
  const down = buildSheet([ev([81])], true).events[0].stem!; // A5, above the middle line
  assert.ok(down.y2 > down.y1);
});

test("a chord is one stem through several heads, ordered low to high", () => {
  const e = buildSheet([ev([60, 64, 67], 1)], true).events[0];
  assert.equal(e.heads.length, 3);
  assert.deepEqual([...e.heads].map((h) => h.y).sort((a, b) => b - a), e.heads.map((h) => h.y));
  assert.equal(e.label, "C");
});

test("notes a second apart are offset across the stem, but a cluster doesn't double-offset", () => {
  const second = buildSheet([ev([60, 62], 1)], true).events[0]; // C4 + D4
  assert.notEqual(second.heads[0].x, second.heads[1].x);
  const cluster = buildSheet([ev([60, 62, 64], 1)], true).events[0]; // C4 + D4 + E4
  assert.equal(cluster.heads[0].x, cluster.heads[2].x); // C and E share a column, D steps out
  assert.notEqual(cluster.heads[1].x, cluster.heads[0].x);
});

test("accidentals are flagged on the head that needs them", () => {
  const e = buildSheet([ev([60, 61], 1)], true).events[0];
  assert.deepEqual(e.heads.map((h) => h.sharp), [false, true]);
});

test("the viewBox grows to fit ledger-heavy extremes instead of clipping them", () => {
  const mid = buildSheet([ev([60])], true);
  const high = buildSheet([ev([108])], true); // C8, far above the treble staff
  const low = buildSheet([ev([24])], true); // C1, far below the bass staff
  assert.ok(high.minY < mid.minY);
  assert.ok(low.minY + low.h > mid.minY + mid.h);
  // Whatever the extreme, the whole grand staff stays inside the box.
  for (const s of [mid, high, low]) {
    assert.ok(s.minY <= STAFF_TOP && s.minY + s.h >= STAFF_BOT);
  }
});

test("the score spreads across the available width and only compresses when it must", () => {
  const wide = buildSheet([ev([60]), ev([62])], true, 800);
  assert.equal(wide.w, 800); // two notes still span the panel
  const long = buildSheet(Array.from({ length: 60 }, () => ev([60])), true, 800);
  assert.ok(long.w > 800); // …and a long tune overflows into a scroll rather than colliding
  const xs = long.events.map((e) => e.x);
  assert.ok(xs.every((x, i) => i === 0 || x > xs[i - 1])); // always left-to-right
});

test("scoreText is a faithful plain-text alternative to the engraving", () => {
  assert.equal(scoreText([ev([60, 64], 2), ev([67], 0)], true), "C4+E4 half, G4 eighth");
  assert.equal(scoreText([ev([60, 64], 2)], false), "C4+E4");
  assert.equal(scoreText([], true), "");
});
