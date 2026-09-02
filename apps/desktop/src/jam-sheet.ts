// jam-sheet.ts: engrave a validated jam-take:v1 event log as printable SVG sheet music.
//
// Honest scope, stated where the code can be held to it: this is a REHEARSAL TRANSCRIPT, not a
// typesetting engine. Starts and lengths quantize to a sixteenth grid on the take's own tempo;
// durations round DOWN to the plain values (16th/8th/quarter/half/whole, no dots, no ties, no
// tuplets); simultaneous notes share a column rather than forming engraved chords with shared
// stems. Every player gets their own labelled staff section (clef picked per section by median
// pitch), and drum lanes render on a single percussion line with x heads. The input is validated
// by the caller (the one jam-take validator); this module additionally refuses out-of-bound
// values so a bug upstream cannot draw outside the page.

import { JAM_KIT, JAM_MET_BPM_MAX, JAM_MET_BPM_MIN, type JamTake } from "./jam-contract.ts";

export type SheetNote = Readonly<{ startSix: number; lenSix: number; midi: number }>;
export type SheetHit = Readonly<{ startSix: number; pad: number }>;
export type SheetPart = Readonly<{
  /** Index into the take's parts. */
  part: number;
  notes: readonly SheetNote[];
  hits: readonly SheetHit[];
  clef: "treble" | "bass";
}>;
export type SheetScore = Readonly<{
  bpm: number;
  beatsPerBar: number;
  /** Sixteenths per bar. */
  barSix: number;
  /** Total length in whole bars (at least one). */
  bars: number;
  parts: readonly SheetPart[];
}>;

/** A note-off that never arrived still has to end somewhere the page can hold. */
const ORPHAN_NOTE_BARS = 2;
const MAX_SHEET_SIX = 16 * 4 * 400; // ~400 bars of 4/4: far past the 10-minute take cap

function letterStep(pc: number): number {
  return [0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5, 6][pc] ?? 0;
}

function isSharp(pc: number): boolean {
  return pc === 1 || pc === 3 || pc === 6 || pc === 8 || pc === 10;
}

/** Diatonic step count from C-1, so staff placement is a subtraction. */
export function diatonicStep(midi: number): number {
  const octave = Math.floor(midi / 12) - 1;
  return octave * 7 + letterStep(midi % 12);
}

export function quantizeTake(take: JamTake): SheetScore {
  const bpm = Math.min(JAM_MET_BPM_MAX, Math.max(JAM_MET_BPM_MIN, take.met.bpm));
  const beatsPerBar = Math.min(8, Math.max(1, take.met.bpb));
  const sixMs = 60_000 / bpm / 4;
  const barSix = beatsPerBar * 4;

  type Open = { startMs: number; midi: number };
  const notesByPart = new Map<number, { startSix: number; lenSix: number; midi: number }[]>();
  const hitsByPart = new Map<number, { startSix: number; pad: number }[]>();
  const open = new Map<string, Open>(); // "lane:note" -> the sounding note
  let endMs = 0;

  const laneSrc = (lane: number): number => take.lanes[lane]?.src ?? 0;
  const closeNote = (lane: number, sounding: Open, offMs: number) => {
    const startSix = Math.max(0, Math.round(sounding.startMs / sixMs));
    const lenSix = Math.max(1, Math.round((offMs - sounding.startMs) / sixMs));
    if (startSix > MAX_SHEET_SIX) return;
    const list = notesByPart.get(laneSrc(lane)) ?? [];
    list.push({ startSix, lenSix: Math.min(lenSix, barSix * ORPHAN_NOTE_BARS * 2), midi: sounding.midi });
    notesByPart.set(laneSrc(lane), list);
  };

  for (const event of take.events) {
    endMs = Math.max(endMs, event.ms);
    if ("d" in event) {
      const startSix = Math.round(event.ms / sixMs);
      if (startSix > MAX_SHEET_SIX || event.n < 0 || event.n >= JAM_KIT.length) continue;
      const list = hitsByPart.get(laneSrc(event.lane)) ?? [];
      list.push({ startSix, pad: event.n });
      hitsByPart.set(laneSrc(event.lane), list);
      continue;
    }
    const key = `${event.lane}:${event.n}`;
    if (event.on === 1) {
      const sounding = open.get(key);
      if (sounding) closeNote(event.lane, sounding, event.ms); // re-press: the first hold ends here
      open.set(key, { startMs: event.ms, midi: event.n });
    } else {
      const sounding = open.get(key);
      if (!sounding) continue;
      open.delete(key);
      closeNote(event.lane, sounding, event.ms);
    }
  }
  // Whatever is still sounding when the tape ran out gets a bounded written length.
  for (const [key, sounding] of open) {
    const lane = Number(key.slice(0, key.indexOf(":")));
    closeNote(lane, sounding, sounding.startMs + ORPHAN_NOTE_BARS * barSix * sixMs);
  }

  let lastSix = 1;
  for (const list of notesByPart.values()) {
    for (const note of list) lastSix = Math.max(lastSix, note.startSix + note.lenSix);
  }
  for (const list of hitsByPart.values()) {
    for (const hit of list) lastSix = Math.max(lastSix, hit.startSix + 1);
  }
  lastSix = Math.min(lastSix, MAX_SHEET_SIX);

  const partIndexes = [...new Set([...notesByPart.keys(), ...hitsByPart.keys()])].sort((a, b) => a - b);
  const parts: SheetPart[] = partIndexes.map((part) => {
    const notes = (notesByPart.get(part) ?? []).sort((a, b) => a.startSix - b.startSix || a.midi - b.midi);
    const pitches = notes.map((note) => note.midi).sort((a, b) => a - b);
    const median = pitches.length ? pitches[Math.floor(pitches.length / 2)] : 60;
    return {
      part,
      notes,
      hits: (hitsByPart.get(part) ?? []).sort((a, b) => a.startSix - b.startSix || a.pad - b.pad),
      clef: median >= 60 ? "treble" : "bass",
    };
  });

  return {
    bpm,
    beatsPerBar,
    barSix,
    bars: Math.max(1, Math.ceil(lastSix / barSix)),
    parts,
  };
}

/** 16th/8th/quarter/half/whole, rounding DOWN so nothing claims longer than it rang. */
export function durationGlyph(lenSix: number): "sixteenth" | "eighth" | "quarter" | "half" | "whole" {
  if (lenSix >= 16) return "whole";
  if (lenSix >= 8) return "half";
  if (lenSix >= 4) return "quarter";
  if (lenSix >= 2) return "eighth";
  return "sixteenth";
}

const BARS_PER_LINE = 4;
const BAR_W = 220;
const STAFF_GAP = 9; // between staff lines
const LINE_PAD_Y = 64; // vertical room per staff line (ledger + flag space)
const LEFT_PAD = 56;
const PAGE_W = LEFT_PAD + BARS_PER_LINE * BAR_W + 24;

function esc(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

type Draw = string[];

function staffLines(out: Draw, x: number, y: number, width: number, count: number) {
  for (let line = 0; line < count; line += 1) {
    out.push(`<line x1="${x}" y1="${y + line * STAFF_GAP}" x2="${x + width}" y2="${y + line * STAFF_GAP}" class="st"/>`);
  }
}

function drawNote(out: Draw, x: number, y: number, glyph: ReturnType<typeof durationGlyph>, sharp: boolean) {
  const open = glyph === "whole" || glyph === "half";
  out.push(`<ellipse cx="${x}" cy="${y}" rx="4.6" ry="3.4" class="${open ? "nh open" : "nh"}"/>`);
  if (sharp) out.push(`<text x="${x - 12}" y="${y + 3.5}" class="acc">#</text>`);
  if (glyph !== "whole") {
    const stemTop = y - 26;
    out.push(`<line x1="${x + 4.4}" y1="${y - 1}" x2="${x + 4.4}" y2="${stemTop}" class="stem"/>`);
    if (glyph === "eighth" || glyph === "sixteenth") {
      out.push(`<path d="M${x + 4.4} ${stemTop} q7 3 8 12" class="flag"/>`);
      if (glyph === "sixteenth") out.push(`<path d="M${x + 4.4} ${stemTop + 6} q7 3 8 12" class="flag"/>`);
    }
  }
}

/**
 * Render the quantized score. `names[part]` labels each section (already display-safe text);
 * the title and footer are drawn as given, escaped here.
 */
export function renderSheetSvg(score: SheetScore, names: readonly string[], title: string): string {
  const out: Draw = [];
  const lines = Math.ceil(score.bars / BARS_PER_LINE);
  let y = 78;

  const sections: { top: number; label: string; body: () => void }[] = [];
  for (const part of score.parts) {
    const label = names[part.part] ?? `player ${part.part + 1}`;
    if (part.notes.length) {
      const top = y;
      y += 26 + lines * LINE_PAD_Y + 12;
      sections.push({ top, label, body: () => drawPitchSection(out, score, part, top) });
    }
    if (part.hits.length) {
      const top = y;
      y += 26 + lines * 46 + 12;
      sections.push({ top, label: `${label} · drums`, body: () => drawDrumSection(out, score, part, top) });
    }
  }
  const height = y + 30;

  for (const section of sections) {
    out.push(`<text x="${LEFT_PAD}" y="${section.top + 4}" class="who">${esc(section.label)}</text>`);
    section.body();
  }

  const head = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${PAGE_W} ${height}" width="${PAGE_W}" height="${height}">` +
    `<style>text{font-family:Georgia,serif}.st{stroke:#444;stroke-width:1}.bar{stroke:#444;stroke-width:1}` +
    `.nh{fill:#111}.nh.open{fill:#fff;stroke:#111;stroke-width:1.6}.stem{stroke:#111;stroke-width:1.4}` +
    `.flag{stroke:#111;stroke-width:1.4;fill:none}.acc{font-size:11px;fill:#111}.clef{font-size:34px;fill:#111}` +
    `.who{font-size:13px;fill:#333;font-style:italic}.ttl{font-size:19px;fill:#111}.sub{font-size:12px;fill:#555}` +
    `.xh{stroke:#111;stroke-width:1.6}.ped{font-size:9px;fill:#666;font-family:monospace}.led{stroke:#444;stroke-width:1}</style>` +
    `<rect x="0" y="0" width="${PAGE_W}" height="${height}" fill="#fffdf6"/>` +
    `<text x="${LEFT_PAD}" y="30" class="ttl">${esc(title)}</text>` +
    `<text x="${LEFT_PAD}" y="48" class="sub">${score.bpm} bpm · ${score.beatsPerBar}/4 · ${score.bars} bars · a Mewtual jam transcript (plain values, no ties)</text>`;
  return `${head}${out.join("")}</svg>`;
}

function drawPitchSection(out: Draw, score: SheetScore, part: SheetPart, top: number) {
  const lines = Math.ceil(score.bars / BARS_PER_LINE);
  // Bottom staff line reference: E4 for treble, G2 for bass.
  const refStep = part.clef === "treble" ? diatonicStep(64) : diatonicStep(43);
  for (let line = 0; line < lines; line += 1) {
    const staffTop = top + 26 + line * LINE_PAD_Y + 14;
    const staffBottom = staffTop + 4 * STAFF_GAP;
    const width = BARS_PER_LINE * BAR_W;
    staffLines(out, LEFT_PAD, staffTop, width, 5);
    out.push(`<text x="${LEFT_PAD - 42}" y="${staffBottom - (part.clef === "treble" ? 9 : 18)}" class="clef">${part.clef === "treble" ? "\u{1D11E}" : "\u{1D122}"}</text>`);
    for (let bar = 0; bar <= BARS_PER_LINE; bar += 1) {
      out.push(`<line x1="${LEFT_PAD + bar * BAR_W}" y1="${staffTop}" x2="${LEFT_PAD + bar * BAR_W}" y2="${staffBottom}" class="bar"/>`);
    }
    const firstSix = line * BARS_PER_LINE * score.barSix;
    const lastSixOnLine = firstSix + BARS_PER_LINE * score.barSix;
    for (const note of part.notes) {
      if (note.startSix < firstSix || note.startSix >= lastSixOnLine) continue;
      const x = LEFT_PAD + ((note.startSix - firstSix) / score.barSix) * BAR_W + 14;
      const staffPos = diatonicStep(note.midi) - refStep; // half-gap steps up from the bottom line
      const yNote = staffBottom - staffPos * (STAFF_GAP / 2);
      for (let ledger = -2; ledger >= staffPos; ledger -= 2) {
        out.push(`<line x1="${x - 8}" y1="${staffBottom - ledger * (STAFF_GAP / 2)}" x2="${x + 8}" y2="${staffBottom - ledger * (STAFF_GAP / 2)}" class="led"/>`);
      }
      for (let ledger = 10; ledger <= staffPos; ledger += 2) {
        out.push(`<line x1="${x - 8}" y1="${staffBottom - ledger * (STAFF_GAP / 2)}" x2="${x + 8}" y2="${staffBottom - ledger * (STAFF_GAP / 2)}" class="led"/>`);
      }
      drawNote(out, x, yNote, durationGlyph(note.lenSix), isSharp(note.midi % 12));
    }
  }
}

function drawDrumSection(out: Draw, score: SheetScore, part: SheetPart, top: number) {
  const lines = Math.ceil(score.bars / BARS_PER_LINE);
  for (let line = 0; line < lines; line += 1) {
    const staffY = top + 26 + line * 46 + 16;
    const width = BARS_PER_LINE * BAR_W;
    staffLines(out, LEFT_PAD, staffY, width, 1);
    for (let bar = 0; bar <= BARS_PER_LINE; bar += 1) {
      out.push(`<line x1="${LEFT_PAD + bar * BAR_W}" y1="${staffY - 10}" x2="${LEFT_PAD + bar * BAR_W}" y2="${staffY + 10}" class="bar"/>`);
    }
    const firstSix = line * BARS_PER_LINE * score.barSix;
    const lastSixOnLine = firstSix + BARS_PER_LINE * score.barSix;
    for (const hit of part.hits) {
      if (hit.startSix < firstSix || hit.startSix >= lastSixOnLine) continue;
      const x = LEFT_PAD + ((hit.startSix - firstSix) / score.barSix) * BAR_W + 14;
      out.push(`<line x1="${x - 3.5}" y1="${staffY - 3.5}" x2="${x + 3.5}" y2="${staffY + 3.5}" class="xh"/>`);
      out.push(`<line x1="${x - 3.5}" y1="${staffY + 3.5}" x2="${x + 3.5}" y2="${staffY - 3.5}" class="xh"/>`);
      out.push(`<text x="${x}" y="${staffY + 16}" class="ped" text-anchor="middle">${esc((JAM_KIT[hit.pad]?.name ?? "?").slice(0, 4))}</text>`);
    }
  }
}

/** The whole pipeline: validated take in, standalone SVG out. */
export function jamTakeSheetSvg(take: JamTake, names: readonly string[], title: string): string {
  return renderSheetSvg(quantizeTake(take), names, title);
}
