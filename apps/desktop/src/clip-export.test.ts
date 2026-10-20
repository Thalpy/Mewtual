import assert from "node:assert/strict";
import test from "node:test";
import { addLayer, newClipDoc, updateLayer, withBaseEdit, type ClipDoc } from "./clip-doc.ts";
import { initialEdit, setRemoved, splitAt } from "./video-edit.ts";
import { DEFAULT_FPS, exportClip, layerSchedule, MAX_FPS, MAX_OUTPUT_EDGE, outputFps, outputSize, type ClipMedia } from "./clip-export.ts";
import { ExportCanceledError, OutputTooLargeError, type Mediabunny } from "./video-export.ts";

test("frames are fitted inside the size limit with even sides, keeping their shape", () => {
  assert.deepEqual(outputSize(1280, 720), { width: 1280, height: 720 });
  assert.deepEqual(outputSize(3840, 2160), { width: MAX_OUTPUT_EDGE, height: 1080 });
  assert.deepEqual(outputSize(1080, 1920), { width: 1080, height: 1920 });
  const odd = outputSize(1279, 721);
  assert.ok(odd.width % 2 === 0 && odd.height % 2 === 0);
  const portrait = outputSize(2160, 3840);
  assert.equal(portrait.height, MAX_OUTPUT_EDGE);
  assert.ok(Math.abs(portrait.width / portrait.height - 2160 / 3840) < 0.01);
  assert.deepEqual(outputSize(0, 0), { width: 1280, height: 720 }, "an unreadable size gets a sensible default");
});

test("the frame rate follows the source within bounds", () => {
  assert.equal(outputFps(29.97), 30);
  assert.equal(outputFps(24), 24);
  assert.equal(outputFps(240), MAX_FPS);
  assert.equal(outputFps(3), 10);
  assert.equal(outputFps(null), DEFAULT_FPS);
  assert.equal(outputFps(Number.NaN), DEFAULT_FPS);
});

test("the base layer shows on every frame, at its kept source times only", () => {
  let doc = newClipDoc("base", 4);
  doc = withBaseEdit(doc, setRemoved(splitAt(splitAt(doc.layers[0].edit, 1), 3), 1, true));
  const frames = 2 * 10;
  const { frames: shown, times } = layerSchedule(doc, doc.layers[0], 10, frames);
  assert.equal(shown.length, frames);
  assert.ok(times.every((s) => s < 1 || s >= 3), "nothing from the cut-out middle");
  assert.ok(times.every((s, i) => i === 0 || s > times[i - 1]), "ascending, as the decoder wants");
});

test("an extra layer shows only from its start until its kept time runs out", () => {
  let doc = newClipDoc("base", 10);
  doc = addLayer(doc, "pip", 2, 3);
  const layer = doc.layers[1];
  const { frames, times } = layerSchedule(doc, layer, 10, 100);
  assert.equal(frames[0], 30);
  assert.equal(frames.at(-1), 49);
  assert.ok(Math.abs(times[0]) < 1e-9 && times.every((s) => s >= 0 && s < 2));
  const moved = updateLayer(doc, layer.id, { start: 9 });
  assert.equal(layerSchedule(moved, moved.layers[1], 10, 100).frames.length, 10, "cut where the clip ends");
});

// --- The compositor's wiring, over a fake media library -------------------------------------------
// Found missing in review: what `exportClip` actually asks the decoders for and draws per frame was
// only checked in a scratch browser harness. A fake library records both.

type Log = { requested: Record<string, number[]>; added: number[]; drawn: string[][]; cancelled: number; outputs: number };

function fakeLibrary(log: Log, outputBytes = 64): Mediabunny {
  class Track {
    readonly name: string;
    constructor(name: string) { this.name = name; }
    async getDisplayWidth() { return 640; }
    async getDisplayHeight() { return 360; }
    async computePacketStats() { return { averagePacketRate: 10 }; }
  }
  class Format {
    getSupportedVideoCodecs() { return ["avc"]; }
    getSupportedAudioCodecs() { return ["aac"]; }
  }
  const lib = {
    MP4: {}, QTFF: {}, WEBM: {}, MATROSKA: {}, QUALITY_HIGH: {},
    BlobSource: class { readonly file: Blob & { name?: string }; constructor(file: Blob & { name?: string }) { this.file = file; } },
    Input: class {
      readonly track: Track;
      constructor(o: { source: { file: { name?: string } } }) { this.track = new Track(o.source.file.name ?? "?"); }
      async getPrimaryVideoTrack() { return this.track; }
      async getPrimaryAudioTrack() { return null; }
      dispose() {}
    },
    Mp4OutputFormat: Format, WebMOutputFormat: Format,
    StreamTarget: class { readonly writable: WritableStream; constructor(writable: WritableStream) { this.writable = writable; } },
    Output: class {
      readonly target: { writable: WritableStream };
      constructor(o: { target: { writable: WritableStream } }) { this.target = o.target; log.outputs++; }
      addVideoTrack() {} addAudioTrack() {}
      async start() {}
      async finalize() {
        const writer = this.target.writable.getWriter();
        await writer.write({ type: "write", data: new Uint8Array(outputBytes), position: 0 });
        await writer.close();
      }
      async cancel() { log.cancelled++; }
    },
    getFirstEncodableVideoCodec: async () => "avc",
    getFirstEncodableAudioCodec: async () => "aac",
    CanvasSource: class { async add(t: number) { log.added.push(Math.round(t * 1000) / 1000); } },
    AudioBufferSource: class { async add() {} },
    CanvasSink: class {
      readonly track: Track;
      constructor(track: Track) { this.track = track; }
      async *canvasesAtTimestamps(times: number[]) {
        log.requested[this.track.name] = [...times].map((t) => Math.round(t * 1000) / 1000);
        for (const t of times) yield { canvas: { tag: `${this.track.name}@${Math.round(t * 10) / 10}`, width: 640, height: 360 } };
      }
    },
  };
  return lib as unknown as Mediabunny;
}

/** The compositor draws into an OffscreenCanvas; jsdom has none. Each frame's draws are recorded. */
function installCanvas(log: Log) {
  const previous = (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas;
  (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas = class {
    getContext() {
      return {
        fillStyle: "", imageSmoothingQuality: "low",
        fillRect: () => { log.drawn.push([]); },
        drawImage: (image: { tag?: string }) => { log.drawn[log.drawn.length - 1]?.push(image.tag ?? "?"); },
        save() {}, restore() {}, translate() {}, rotate() {},
      };
    }
  };
  return () => { (globalThis as { OffscreenCanvas?: unknown }).OffscreenCanvas = previous; };
}

/** A 10 s clip with 3..6 s cut out (7 s posted), a 2 s picture-in-picture at 4 s, both silent. */
function composedDoc(): ClipDoc {
  let doc = newClipDoc("base", 10);
  doc = withBaseEdit(doc, setRemoved(splitAt(splitAt(initialEdit(10), 3), 6), 1, true));
  doc = addLayer(doc, "pip", 2, 4);
  doc = updateLayer(doc, "v1", { muted: true });
  return updateLayer(doc, doc.layers[1].id, { muted: true });
}
const media: ClipMedia = {
  video: (s) => (s === "base" ? new File([], "base") : s === "pip" ? new File([], "pip") : null),
  image: () => null,
  sound: () => null,
};

test("every frame is drawn from kept times only, and the second video only inside its window", async () => {
  const log: Log = { requested: {}, added: [], drawn: [], cancelled: 0, outputs: 0 };
  const restore = installCanvas(log);
  try {
    const file = await exportClip(composedDoc(), media, { capBytes: 1 << 20, container: "mp4", name: "beach.mov", library: fakeLibrary(log) });
    assert.equal(file.name, "beach.mp4");
    assert.equal(file.type, "video/mp4");
    assert.equal(log.added.length, 70, "7 s at 10 frames a second");
    assert.deepEqual(log.added.slice(0, 3), [0, 0.1, 0.2]);
    const base = log.requested.base;
    assert.equal(base.length, 70);
    assert.ok(base.every((t) => t < 3 || t >= 6), "nothing from the cut-out 3..6 s is ever decoded");
    assert.ok(base.every((t, i) => i === 0 || t > base[i - 1]), "asked in order, as the decoder wants");
    assert.equal(log.requested.pip.length, 20, "the second video: 2 s, from output 4 s");
    assert.ok(log.drawn.every((frame) => frame[0]?.startsWith("base@")), "the clip is under every frame");
    const withPip = log.drawn.map((frame, k) => [k, frame.some((tag) => tag.startsWith("pip@"))] as const).filter(([, has]) => has).map(([k]) => k);
    assert.deepEqual(withPip, Array.from({ length: 20 }, (_, i) => 40 + i), "drawn on frames 40..59 and no others");
    assert.equal(log.cancelled, 0);
  } finally { restore(); }
});

test("an output over the cap is refused, and a cancel stops the encoder", async () => {
  const over: Log = { requested: {}, added: [], drawn: [], cancelled: 0, outputs: 0 };
  let restore = installCanvas(over);
  try {
    await assert.rejects(
      exportClip(composedDoc(), media, { capBytes: 32, container: "mp4", name: "b.mp4", library: fakeLibrary(over, 64) }),
      OutputTooLargeError,
    );
    assert.equal(over.cancelled, 1, "the encoder was stopped");
  } finally { restore(); }

  const cancelled: Log = { requested: {}, added: [], drawn: [], cancelled: 0, outputs: 0 };
  restore = installCanvas(cancelled);
  const abort = new AbortController();
  try {
    await assert.rejects(
      exportClip(composedDoc(), media, { capBytes: 1 << 20, container: "webm", name: "b.webm", library: fakeLibrary(cancelled), signal: abort.signal, onProgress: (f) => { if (f > 0.3) abort.abort(); } }),
      ExportCanceledError,
    );
    assert.ok(cancelled.added.length < 70, "it stopped part way");
    assert.equal(cancelled.cancelled, 1);
  } finally { restore(); }
});
