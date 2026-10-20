import assert from "node:assert/strict";
import test from "node:test";
import { addAudio, addLayer, newClipDoc, updateAudio, updateLayer, withBaseEdit } from "./clip-doc.ts";
import { setRemoved, splitAt } from "./video-edit.ts";
import {
  chunkPlacement, DECLICK_SECONDS, envelopeAt, envelopeCorners, MIX_SEGMENT_SECONDS, mixClipSound, piecesInWindow, SOFT_CLIP_KNEE,
  softClipCurve, soundPieces, type SoundPiece,
} from "./clip-audio.ts";

const close = (a: number, b: number, eps = 1e-9) => Math.abs(a - b) <= eps;

test("a cut video contributes only its kept sound, packed end to end", () => {
  // 10 s clip; blade at 2 and 5, remove the middle: keeps 0-2 and 5-10.
  let doc = newClipDoc("base", 10);
  doc = withBaseEdit(doc, setRemoved(splitAt(splitAt(doc.layers[0].edit, 2), 5), 1, true));
  const pieces = soundPieces(doc);
  assert.deepEqual(pieces.map((p) => [p.from, p.at, p.length]), [[0, 0, 2], [5, 2, 5]]);
  assert.ok(pieces.every((p) => p.kind === "video" && p.source === "base" && p.gain === 1));
});

test("muted, silent and out-of-clip sound is left out; the rest is cut at the clip's end", () => {
  let doc = newClipDoc("base", 10);
  doc = addLayer(doc, "pip", 8, 6);
  doc = addAudio(doc, "music", "song", 30, 1);
  doc = addAudio(doc, "sfx", "pow", 0.5, 3);
  const [song, pow] = doc.audio;
  doc = updateAudio(doc, pow.id, { muted: true });
  const pieces = soundPieces(doc);
  assert.deepEqual(pieces.map((p) => p.source), ["base", "pip", "song"]);
  const pip = pieces.find((p) => p.source === "pip")!;
  assert.equal(pip.at, 6);
  assert.equal(pip.length, 4, "the picture-in-picture clip is cut where the clip ends");
  assert.equal(pieces.find((p) => p.source === "song")!.length, song.length);
  doc = updateLayer(doc, doc.layers[0].id, { muted: true });
  doc = updateLayer(doc, doc.layers[1].id, { volume: 0 });
  assert.deepEqual(soundPieces(doc).map((p) => p.source), ["song"]);
});

test("a trimmed sound starts from its offset", () => {
  let doc = newClipDoc("base", 10);
  doc = addAudio(doc, "music", "song", 30, 2);
  doc = updateAudio(doc, doc.audio[0].id, { offset: 4, length: 3 });
  const song = soundPieces(doc).find((p) => p.source === "song")!;
  assert.deepEqual([song.from, song.at, song.length, song.gain], [4, 2, 3, 0.8]);
});

const piece: SoundPiece = { kind: "music", source: "s", from: 1, at: 2, length: 3, gain: 0.5 };

test("every piece fades in and out over a few milliseconds and is flat between", () => {
  assert.equal(envelopeAt(piece, 2), 0);
  assert.ok(close(envelopeAt(piece, 2 + DECLICK_SECONDS / 2), 0.25));
  assert.equal(envelopeAt(piece, 3), 0.5);
  assert.ok(close(envelopeAt(piece, 5 - DECLICK_SECONDS / 2), 0.25));
  assert.equal(envelopeAt(piece, 5), 0);
  assert.equal(envelopeAt(piece, 7), 0);
  // A piece shorter than two fades peaks in its middle instead of overlapping the ramps.
  const blip: SoundPiece = { ...piece, length: 0.002 };
  const corners = envelopeCorners(blip);
  assert.ok(close(corners[1][0], corners[2][0]), "the two fades meet in the middle");
  for (const [t, level] of corners) assert.ok(close(envelopeAt(blip, t), level, 1e-6) || level === 0);
});

test("a window holds just the overlapping part of each piece, relative to the window", () => {
  const other: SoundPiece = { ...piece, source: "late", at: 25, length: 10, from: 0 };
  const windowed = piecesInWindow([piece, other], 4, 24);
  assert.equal(windowed.length, 1);
  assert.deepEqual([windowed[0].from, windowed[0].when, windowed[0].length], [3, 0, 1]);
  const second = piecesInWindow([piece, other], 20, 40);
  assert.deepEqual(second.map((w) => [w.piece.source, w.from, w.when, w.length]), [["late", 0, 5, 10]]);
  assert.equal(piecesInWindow([piece], 5, 9).length, 0, "touching at an edge is not overlapping");
});

test("windows tile a piece exactly: no sound is lost or doubled at a segment boundary", () => {
  const long: SoundPiece = { kind: "video", source: "v", from: 3, at: 7, length: 50, gain: 1 };
  let covered = 0, expectFrom = 3;
  for (let start = 0; start < 60; start += MIX_SEGMENT_SECONDS) {
    for (const w of piecesInWindow([long], start, start + MIX_SEGMENT_SECONDS)) {
      assert.ok(close(w.from, expectFrom, 1e-9), "each window continues where the last stopped");
      expectFrom = w.from + w.length;
      covered += w.length;
    }
  }
  assert.ok(close(covered, 50));
});

test("the soft clip is unchanged for ordinary levels and never exceeds full scale", () => {
  const curve = softClipCurve(2001);
  const at = (x: number) => curve[Math.round((x + 1) / 2 * 2000)];
  for (const x of [-0.8, -0.5, 0, 0.3, SOFT_CLIP_KNEE]) assert.ok(close(at(x), x, 1e-6), `linear at ${x}`);
  for (let i = 1; i < curve.length; i++) assert.ok(curve[i] >= curve[i - 1], "monotonic");
  assert.ok(Math.abs(at(1)) < 1 && Math.abs(at(-1)) < 1);
  assert.ok(at(1) > 0.95, "loud peaks still come out loud");
});

test("decoded chunks are cut to the window, wherever they straddle it", () => {
  // Window: 2.0 s, 1000 frames at 1 kHz.
  assert.deepEqual(chunkPlacement(2, 1000, 1000, 1.9, 300), { from: 100, to: 0, count: 200 }, "starts before: its head is dropped");
  assert.deepEqual(chunkPlacement(2, 1000, 1000, 2.5, 100), { from: 0, to: 500, count: 100 });
  assert.deepEqual(chunkPlacement(2, 1000, 1000, 2.95, 200), { from: 0, to: 950, count: 50 }, "runs past the end: its tail is dropped");
  assert.equal(chunkPlacement(2, 1000, 1000, 1.0, 500), null, "wholly before");
  assert.equal(chunkPlacement(2, 1000, 1000, 3.0, 10), null, "wholly after");
});

// --- The mixer, over a recording fake ---------------------------------------------------------------

type Call = { context: number; kind: string; args: unknown[] };
function recordingFactory() {
  const calls: Call[] = [];
  const sizes: number[] = [];
  const make = (_channels: number, frames: number, rate: number) => {
    const id = sizes.length;
    sizes.push(frames);
    const node = (kind: string, extra: object = {}) => ({
      connect(to: unknown) { calls.push({ context: id, kind: `${kind}.connect`, args: [] }); return to; },
      ...extra,
    });
    const param = (owner: string) => ({
      setValueAtTime: (...args: unknown[]) => calls.push({ context: id, kind: `${owner}.set`, args }),
      linearRampToValueAtTime: (...args: unknown[]) => calls.push({ context: id, kind: `${owner}.ramp`, args }),
    });
    return {
      destination: {},
      createWaveShaper: () => node("shaper", { curve: null }),
      createGain: () => node("gain", { gain: param("gain") }),
      createBufferSource: () => node("source", {
        buffer: null,
        start: (...args: unknown[]) => calls.push({ context: id, kind: "start", args }),
      }),
      startRendering: async () => ({ id, length: frames, sampleRate: rate }) as unknown as AudioBuffer,
    } as unknown as OfflineAudioContext;
  };
  return { calls, sizes, make };
}

test("the mix renders segment by segment, in order, covering the clip to the frame", async () => {
  const { sizes, make } = recordingFactory();
  const segments: unknown[] = [];
  const ok = await mixClipSound([piece], 45.5, async () => ({ buffer: {} as AudioBuffer, offset: 0 }),
    (segment) => { segments.push(segment); }, { sampleRate: 1000, createContext: make });
  assert.equal(ok, true);
  assert.deepEqual(sizes, [20_000, 20_000, 5_500]);
  assert.equal(segments.length, 3);
});

test("nothing to hear means no audio track, and no rendering", async () => {
  const { sizes, make } = recordingFactory();
  assert.equal(await mixClipSound([], 10, async () => null, () => {}, { createContext: make }), false);
  assert.equal(sizes.length, 0);
});

test("each window plays the loaded sound from the right point, at the right time, for the right length", async () => {
  const { calls, make } = recordingFactory();
  const loads: unknown[][] = [];
  const crossing: SoundPiece = { kind: "music", source: "m", from: 2, at: 18, length: 4, gain: 1 };
  await mixClipSound([crossing], 30, async (p, from, length) => {
    loads.push([p.source, from, length]);
    return { buffer: {} as AudioBuffer, offset: from + 0.25 };
  }, () => {}, { sampleRate: 1000, createContext: make });
  assert.deepEqual(loads, [["m", 2, 2], ["m", 4, 2]], "the loader is asked for just each window's part");
  const starts = calls.filter((c) => c.kind === "start").map((c) => [c.context, ...c.args]);
  assert.deepEqual(starts, [[0, 18, 2.25, 2], [1, 0, 4.25, 2]]);
});

test("a fade cut by a segment boundary continues in the next segment without a step", async () => {
  const { calls, make } = recordingFactory();
  // Starts 2 ms before the 20 s boundary, so its 4 ms fade-in straddles it.
  const straddle: SoundPiece = { kind: "sfx", source: "s", from: 0, at: 20 - 0.002, length: 1, gain: 1 };
  await mixClipSound([straddle], 30, async () => ({ buffer: {} as AudioBuffer, offset: 0 }), () => {}, { sampleRate: 1000, createContext: make });
  const gainCalls = (context: number) => calls.filter((c) => c.context === context && (c.kind === "gain.set" || c.kind === "gain.ramp"));
  const first = gainCalls(0), second = gainCalls(1);
  const endOfFirst = first.at(-1)!;
  assert.equal(endOfFirst.kind, "gain.ramp");
  assert.ok(close(endOfFirst.args[0] as number, 0.5, 1e-6), "half way up the fade when the first segment ends");
  assert.ok(close(endOfFirst.args[1] as number, MIX_SEGMENT_SECONDS, 1e-9), "at the segment's very end");
  assert.equal(second[0].kind, "gain.set");
  assert.ok(close(second[0].args[0] as number, 0.5, 1e-6), "the next segment starts at the same level");
});

test("a source with no sound is skipped, and a loader that fails fails the mix", async () => {
  const { calls, make } = recordingFactory();
  assert.equal(await mixClipSound([piece], 10, async () => null, () => {}, { createContext: make }), true);
  assert.equal(calls.filter((c) => c.kind === "start").length, 0);
  await assert.rejects(mixClipSound([piece], 10, async () => { throw new Error("decode failed"); }, () => {}, { createContext: make }), /decode failed/);
});

test("an aborted mix stops before the next segment", async () => {
  const { sizes, make } = recordingFactory();
  const abort = new AbortController();
  await assert.rejects(mixClipSound([piece], 60, async () => null, () => { abort.abort(); }, { createContext: make, signal: abort.signal }), { name: "AbortError" });
  assert.equal(sizes.length, 1);
});
