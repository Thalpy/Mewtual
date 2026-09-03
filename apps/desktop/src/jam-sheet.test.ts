import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import type { JamTake } from "./jam-contract.ts";
import { diatonicStep, durationGlyph, jamTakeSheetSvg, quantizeTake } from "./jam-sheet.ts";

const NONCE = "00000000000000ab";

function takeWith(events: JamTake["events"], lanes: JamTake["lanes"] = [{ src: 0, sn: NONCE }]): JamTake {
  return {
    v: 1,
    group: "g",
    call: "c",
    met: { bpm: 120, bpb: 4 },
    parts: ["aaaa", "bbbb"],
    lanes,
    patches: [],
    events,
  };
}

test("quantize lands starts and lengths on the sixteenth grid of the take's tempo", () => {
  // 120 bpm: a sixteenth is 125ms. A note at 130ms for 260ms is start 1, length 2 (an eighth).
  const score = quantizeTake(takeWith([
    { ms: 130, lane: 0, n: 64, on: 1, w: "triangle", q: 1 },
    { ms: 390, lane: 0, n: 64, on: 0, q: 2 },
  ]));
  assert.equal(score.parts.length, 1);
  assert.deepEqual(score.parts[0].notes, [{ startSix: 1, lenSix: 2, midi: 64 }]);
  assert.equal(score.barSix, 16);
  assert.equal(score.bars, 1);
});

test("a lost note-off gets a bounded written length instead of an endless tone", () => {
  const score = quantizeTake(takeWith([{ ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 1 }]));
  const note = score.parts[0].notes[0];
  assert.ok(note.lenSix >= 1);
  assert.ok(note.lenSix <= 2 * score.barSix, "an orphan may not stretch the page arbitrarily");
});

test("duration glyphs round down to plain values after the visible minimum", () => {
  assert.equal(durationGlyph(1), "sixteenth");
  assert.equal(durationGlyph(3), "eighth"); // a dotted eighth writes as an eighth, not a quarter
  assert.equal(durationGlyph(7), "quarter");
  assert.equal(durationGlyph(15), "half");
  assert.equal(durationGlyph(16), "whole");
});

test("duration quantization floors before choosing a plain glyph", () => {
  // At 120 bpm a sixteenth is 125 ms; 374 ms is just under three grid units and must remain two.
  const score = quantizeTake(takeWith([
    { ms: 0, lane: 0, n: 64, on: 1, w: "triangle", q: 1 },
    { ms: 374, lane: 0, n: 64, on: 0, q: 2 },
  ]));
  assert.equal(score.parts[0].notes[0].lenSix, 2);
});

test("a sub-sixteenth tap uses the declared one-sixteenth visible minimum", () => {
  const score = quantizeTake(takeWith([
    { ms: 0, lane: 0, n: 64, on: 1, w: "triangle", q: 1 },
    { ms: 1, lane: 0, n: 64, on: 0, q: 2 },
  ]));
  assert.equal(score.parts[0].notes[0].lenSix, 1);
});

test("clef follows the median pitch and lanes group by their PLAYER, not by reconnects", () => {
  // Two lanes, same src (a reconnect mid-take): one section, not two half-players.
  const score = quantizeTake(takeWith(
    [
      { ms: 0, lane: 0, n: 40, on: 1, w: "sine", q: 1 },
      { ms: 500, lane: 0, n: 40, on: 0, q: 2 },
      { ms: 1000, lane: 1, n: 43, on: 1, w: "sine", q: 1 },
      { ms: 1500, lane: 1, n: 43, on: 0, q: 2 },
    ],
    [{ src: 0, sn: NONCE }, { src: 0, sn: "00000000000000cd" }],
  ));
  assert.equal(score.parts.length, 1);
  assert.equal(score.parts[0].notes.length, 2);
  assert.equal(score.parts[0].clef, "bass", "E2/G2 material belongs on a bass staff");
});

test("diatonic steps place middle C one ledger under the treble staff", () => {
  // E4 is the treble bottom line; C4 sits two steps under it (one ledger line through the head).
  assert.equal(diatonicStep(64) - diatonicStep(60), 2);
  // Octaves are seven steps apart, always.
  assert.equal(diatonicStep(72) - diatonicStep(60), 7);
});

test("the rendered page is standalone SVG carrying every player and escaping their names", () => {
  const svg = jamTakeSheetSvg(
    takeWith([
      { ms: 0, lane: 0, n: 64, on: 1, w: "triangle", q: 1 },
      { ms: 500, lane: 0, n: 64, on: 0, q: 2 },
      { ms: 250, lane: 1, n: 0, d: 1, q: 1 },
    ], [{ src: 0, sn: NONCE }, { src: 1, sn: "00000000000000cd" }]),
    ["<mika>", "rook"],
    'take 01 · "general"',
  );
  assert.ok(svg.startsWith("<svg"), "the save command's validation gate depends on this prefix");
  assert.ok(svg.includes('data-mewtual-sheet="v1"'));
  assert.ok(svg.endsWith("</svg>"));
  assert.ok(svg.includes("&lt;mika&gt;"), "player names are untrusted text");
  assert.ok(svg.includes("rook · drums"));
  assert.ok(svg.includes("&quot;general&quot;"));
  assert.ok(!svg.includes("<mika>"));

  // The native command deliberately accepts one inert SVG grammar. Pin the duplicated stylesheet
  // at the language boundary so a harmless engraving change cannot make every export fail.
  const native = readFileSync(
    fileURLToPath(new URL("../src-tauri/src/lib.rs", import.meta.url)),
    "utf8",
  );
  const nativeStyle = native.match(/const JAM_SHEET_STYLE: &str = r#"([^"\r\n]*)"#;/)?.[1];
  const renderedStyle = svg.match(/<style>([^<]*)<\/style>/)?.[1];
  assert.equal(renderedStyle, nativeStyle);
});
