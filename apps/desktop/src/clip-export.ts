/**
 * Makes the posted file for a COMPOSED clip: one with text, pictures, extra videos, added sound, or
 * its own sound changed. (A clip that is only cut goes through `exportEditedVideo` instead, which
 * re-encodes the kept frames without drawing them; an untouched one uploads its original bytes.)
 *
 * Every output frame is drawn by `drawClipFrame`, the same function that draws the editor's stage,
 * then encoded. The sound is mixed by `mixClipSound` in segments and encoded alongside.
 *
 * Invariants kept from the cut-only exporter, for the same reasons (see `video-export.ts`):
 * - Cut-out time is absent from the bytes: frames are drawn only at kept times, and sound comes only
 *   from kept ranges (`soundPieces`). Nothing is copied packet-for-packet.
 * - No descriptive metadata is carried over: the output is a fresh file written from pixels.
 * - Output is capped (`CappedBytes`) and cancellable; failures surface as sentences for the person.
 *
 * Bounds that keep a composition affordable in a WebView:
 * - `MAX_COMPOSED_SECONDS`: every frame of a composed clip is decoded, drawn and encoded, and the
 *   sound is decoded too. Long clips can still be cut with the lighter exporter.
 * - `MAX_OUTPUT_EDGE`: frames are drawn at most this big on the long edge.
 * - The muxer is fed one sound segment, then the frames up to the end of it, alternately, so
 *   neither track runs far ahead and the muxer never buffers much of one waiting for the other.
 */
import type { InputAudioTrack, InputVideoTrack, StreamTargetChunk, Input, Output } from "mediabunny";
import { clipDuration, layerSourceTime, type ClipDoc, type Overlay, type VideoLayer } from "./clip-doc.ts";
import { chunkPlacement, mixClipSound, soundPieces, type LoadedSound, type SoundPiece } from "./clip-audio.ts";
import { containRect, drawClipFrame, textFontPx, type Picture, type TextSprite } from "./clip-render.ts";
import { CLIP_SAMPLE_RATE } from "./jam-offline.ts";
import { collectTextEffectCss, isAnimated, prepareText, snapshotImage, type PreparedText } from "./text-fx-raster.ts";
import {
  CappedBytes, containerMime, editedFileName, EXPORT_FAILED, ExportCanceledError, ExportRefusal, loadLibrary, openInput,
  OutputTooLargeError, type ExportOptions, type Mediabunny, type VideoContainer,
} from "./video-export.ts";

/** Longest composed clip, in seconds. */
export const MAX_COMPOSED_SECONDS = 600;
/** Longest frame edge drawn, in px. */
export const MAX_OUTPUT_EDGE = 1920;
export const DEFAULT_FPS = 30;
export const MAX_FPS = 60;

/** The frame size for a base video `w` x `h`: fitted inside `MAX_OUTPUT_EDGE`, both sides even (encoders need it). */
export function outputSize(w: number, h: number): { width: number; height: number } {
  if (!(w > 0) || !(h > 0)) return { width: 1280, height: 720 };
  const k = Math.min(1, MAX_OUTPUT_EDGE / Math.max(w, h));
  const even = (n: number) => Math.max(2, Math.round((n * k) / 2) * 2);
  return { width: even(w), height: even(h) };
}

/** The output frame rate for a source measured at `packetRate` frames a second. */
export function outputFps(packetRate: number | null | undefined): number {
  if (!packetRate || !Number.isFinite(packetRate) || packetRate < 1) return DEFAULT_FPS;
  return Math.min(MAX_FPS, Math.max(10, Math.round(packetRate)));
}

/** Which output frames a layer shows on, and its own source time at each. Ascending in both. */
export function layerSchedule(doc: ClipDoc, layer: VideoLayer, fps: number, frameCount: number): { frames: number[]; times: number[] } {
  const frames: number[] = [], times: number[] = [];
  for (let k = 0; k < frameCount; k++) {
    const s = layerSourceTime(doc, layer, k / fps);
    if (s === null) continue;
    frames.push(k);
    times.push(s);
  }
  return { frames, times };
}

/** The media the document names by `source`, held by the editor. */
export type ClipMedia = {
  /** A video layer's file. */
  readonly video: (source: string) => Blob | null;
  /** A picture overlay's decoded image. */
  readonly image: (source: string) => Picture | null;
  /** Music or a sound effect, already rendered or decoded. */
  readonly sound: (source: string) => AudioBuffer | null;
};

export type ClipExportOptions = ExportOptions & {
  readonly container: VideoContainer;
  /** The picked file's name; the output keeps its stem. */
  readonly name: string;
  /** Test seam: the media library (Mediabunny) to drive. */
  readonly library?: Mediabunny;
};

type LayerFeed = {
  readonly layer: VideoLayer;
  readonly schedule: { frames: number[]; times: number[] };
  readonly frames: AsyncIterator<{ canvas: HTMLCanvasElement | OffscreenCanvas } | null>;
  readonly size: { width: number; height: number };
  next: number;
  current: Picture | null;
};

type TextFeed = { readonly overlay: Overlay; readonly prepared: PreparedText; readonly fontPx: number; still: HTMLImageElement | null; current: HTMLImageElement | null };

/**
 * Render `doc` to a new file. Rejects like `exportEditedVideo`: `ExportCanceledError` on abort,
 * `OutputTooLargeError` past the cap, otherwise an `Error` whose message is for the person.
 */
export async function exportClip(doc: ClipDoc, media: ClipMedia, options: ClipExportOptions): Promise<File> {
  const total = clipDuration(doc);
  if (!(total > 0)) throw new ExportRefusal("Nothing is left to upload: every part has been cut.");
  if (total > MAX_COMPOSED_SECONDS) throw new ExportRefusal("A clip with added text, pictures, videos or sound can be up to 10 minutes long.");
  if (options.signal?.aborted) throw new ExportCanceledError();
  let mb: Mediabunny;
  try { mb = options.library ?? await loadLibrary(); } catch (error) { throw new Error("The video editor could not be loaded.", { cause: error }); }

  const inputs = new Map<string, Input>();
  const inputFor = (source: string): Input => {
    let input = inputs.get(source);
    if (!input) {
      const file = media.video(source);
      if (!file) throw new ExportRefusal("A video in this clip is missing.");
      input = openInput(mb, file);
      inputs.set(source, input);
    }
    return input;
  };
  const bytes = new CappedBytes(options.capBytes);
  let overflow: OutputTooLargeError | null = null;
  let output: Output | null = null;
  const writable = new WritableStream<StreamTargetChunk>({
    write(chunk) {
      try { bytes.write(chunk.position, chunk.data); } catch (error) {
        if (error instanceof OutputTooLargeError) overflow = error;
        throw error;
      }
    },
  });
  const checkStop = () => {
    if (overflow) throw overflow;
    if (options.signal?.aborted) throw new ExportCanceledError();
  };

  try {
    const baseTrack = await inputFor(doc.layers[0].source).getPrimaryVideoTrack();
    if (!baseTrack) throw new ExportRefusal("This file has no video track to edit.");
    const { width, height } = outputSize(await baseTrack.getDisplayWidth(), await baseTrack.getDisplayHeight());
    const fps = outputFps((await baseTrack.computePacketStats(120)).averagePacketRate);
    const frameCount = Math.max(1, Math.round(total * fps));

    const format = options.container === "mp4" ? new mb.Mp4OutputFormat({ fastStart: false }) : new mb.WebMOutputFormat();
    output = new mb.Output({ format, target: new mb.StreamTarget(writable) });
    const videoCodec = await mb.getFirstEncodableVideoCodec(format.getSupportedVideoCodecs(), { width, height });
    if (!videoCodec) throw new ExportRefusal("This window can't encode the picture of an edited video.");
    const canvas = new OffscreenCanvas(width, height);
    const ctx = canvas.getContext("2d", { alpha: false });
    if (!ctx) throw new Error("no 2d context");
    const videoSource = new mb.CanvasSource(canvas, { codec: videoCodec, bitrate: mb.QUALITY_HIGH });
    output.addVideoTrack(videoSource, { frameRate: fps });

    // A video without an audio track has nothing to mix. Its pieces are dropped before deciding
    // whether the clip needs a sound track at all: a silent recording with only a sticker added
    // must not need (or be refused for want of) an audio encoder.
    const audioTracks = new Map<string, InputAudioTrack | null>();
    for (const piece of soundPieces(doc)) {
      if (piece.kind === "video" && !audioTracks.has(piece.source)) audioTracks.set(piece.source, await inputFor(piece.source).getPrimaryAudioTrack());
    }
    const pieces = soundPieces(doc).filter((piece) => piece.kind !== "video" || audioTracks.get(piece.source));
    let audioSource: InstanceType<Mediabunny["AudioBufferSource"]> | null = null;
    if (pieces.length) {
      const audioCodec = await mb.getFirstEncodableAudioCodec(format.getSupportedAudioCodecs(), { numberOfChannels: 2, sampleRate: CLIP_SAMPLE_RATE });
      if (!audioCodec) throw new ExportRefusal("This window can't encode the sound of an edited video.");
      audioSource = new mb.AudioBufferSource({ codec: audioCodec, bitrate: mb.QUALITY_HIGH });
      output.addAudioTrack(audioSource);
    }
    await output.start();

    // --- Pictures ---------------------------------------------------------------------------------
    const feeds: LayerFeed[] = [];
    for (const [index, layer] of doc.layers.entries()) {
      const track: InputVideoTrack | null = await inputFor(layer.source).getPrimaryVideoTrack();
      if (!track) throw new ExportRefusal("A video in this clip has no picture.");
      // Decode straight to the size the layer is drawn at (its letterboxed box), not the source's.
      const box = index === 0 ? { x: 0, y: 0, w: width, h: height } : { x: 0, y: 0, w: layer.box.w * width, h: layer.box.h * height };
      const fit = containRect(await track.getDisplayWidth(), await track.getDisplayHeight(), box);
      const size = { width: Math.max(2, Math.round(fit.w)), height: Math.max(2, Math.round(fit.h)) };
      const schedule = layerSchedule(doc, layer, fps, frameCount);
      const sink = new mb.CanvasSink(track, { ...size, fit: "fill", poolSize: 2 });
      feeds.push({ layer, schedule, size, frames: sink.canvasesAtTimestamps(schedule.times)[Symbol.asyncIterator](), next: 0, current: null });
    }
    const texts = new Map<string, TextFeed>();
    const css = collectTextEffectCss(document.styleSheets);
    for (const overlay of doc.overlays) {
      if (overlay.content.kind !== "text") continue;
      const fontPx = textFontPx(overlay, height);
      const prepared = prepareText(document, css, { text: overlay.content.text, effect: overlay.content.effect, color: overlay.content.color, fontPx });
      texts.set(overlay.id, { overlay, prepared, fontPx, still: isAnimated(prepared) ? null : await snapshotImage(prepared, 0), current: null });
    }
    const pictures = new Map(feeds.map((feed) => [feed.layer.id, feed] as const));
    const assets = {
      layer: (id: string) => pictures.get(id)?.current ?? null,
      image: (source: string) => media.image(source),
      text: (overlay: Overlay): TextSprite | null => {
        const feed = texts.get(overlay.id);
        const image = feed?.still ?? feed?.current;
        return feed && image ? { image, width: feed.prepared.width, height: feed.prepared.height, pad: feed.prepared.pad, fontPx: feed.fontPx } : null;
      },
    };

    const renderFrame = async (k: number) => {
      checkStop();
      const t = k / fps;
      for (const feed of feeds) {
        if (feed.schedule.frames[feed.next] === k) {
          feed.next++;
          const step = await feed.frames.next();
          const wrapped = step.done ? null : step.value;
          // A frame the decoder has none for (before the first) keeps the layer's last picture.
          if (wrapped) feed.current = { image: wrapped.canvas, ...feed.size };
        } else if (feed.layer !== doc.layers[0]) feed.current = null;
      }
      for (const feed of texts.values()) {
        const { overlay } = feed;
        if (feed.still || !(t >= overlay.start && t < overlay.end)) continue;
        feed.current = await snapshotImage(feed.prepared, t - overlay.start);
      }
      drawClipFrame(ctx, width, height, doc, t, assets);
      await videoSource.add(t, 1 / fps);
      options.onProgress?.(Math.min(1, (k + 1) / frameCount));
    };
    let nextFrame = 0;
    const framesUntil = async (end: number) => {
      while (nextFrame < frameCount && nextFrame / fps < end - 1e-9) await renderFrame(nextFrame++);
    };

    // --- Sound --------------------------------------------------------------------------------------
    if (audioSource) {
      const source = audioSource;
      const load = async (piece: SoundPiece, from: number, length: number, signal?: AbortSignal): Promise<LoadedSound> => {
        if (piece.kind !== "video") {
          const buffer = media.sound(piece.source);
          if (!buffer) throw new ExportRefusal("A sound in this clip is missing.");
          return { buffer, offset: from };
        }
        const track = audioTracks.get(piece.source);
        return track ? decodeWindow(mb, track, from, length, signal) : null;
      };
      let mixed = 0;
      await mixClipSound(pieces, total, load, async (segment) => {
        checkStop();
        await source.add(segment);
        mixed += segment.length / segment.sampleRate;
        await framesUntil(mixed);
      }, { signal: options.signal });
    }
    await framesUntil(Number.POSITIVE_INFINITY);
    checkStop();
    await output.finalize();
  } catch (error) {
    // Leave no encoder running behind the rejection.
    await output?.cancel().catch(() => {});
    if (overflow) throw overflow;
    if (options.signal?.aborted || error instanceof ExportCanceledError || (error instanceof DOMException && error.name === "AbortError")) {
      throw new ExportCanceledError();
    }
    if (error instanceof ExportRefusal) throw error;
    throw new Error(EXPORT_FAILED, { cause: error });
  } finally {
    for (const input of inputs.values()) input.dispose();
  }
  const mime = containerMime(options.container);
  return new File([bytes.toBlob(mime)], editedFileName(options.name, options.container), { type: mime });
}

/**
 * Decode `length` seconds of a video's sound from source time `from` into one buffer starting
 * exactly there. Checks `signal` before allocating and after every decoded chunk, so a cancel stops
 * the decoder within one chunk (leaving the loop ends the sink's iterator, which stops decoding).
 */
async function decodeWindow(mb: Mediabunny, track: InputAudioTrack, from: number, length: number, signal?: AbortSignal): Promise<LoadedSound> {
  const stop = () => { if (signal?.aborted) throw new ExportCanceledError(); };
  const rate = await track.getSampleRate();
  const channels = Math.min(2, Math.max(1, await track.getNumberOfChannels()));
  const frames = Math.max(1, Math.ceil(length * rate));
  stop();
  const out = new AudioBuffer({ length: frames, numberOfChannels: channels, sampleRate: rate });
  const sink = new mb.AudioBufferSink(track);
  for await (const chunk of sink.buffers(from, from + length)) {
    stop();
    const place = chunkPlacement(from, rate, frames, chunk.timestamp, chunk.buffer.length);
    if (!place) continue;
    for (let c = 0; c < channels; c++) {
      const data = chunk.buffer.getChannelData(Math.min(c, chunk.buffer.numberOfChannels - 1));
      out.copyToChannel(data.subarray(place.from, place.from + place.count), c, place.to);
    }
  }
  return { buffer: out, offset: 0 };
}
