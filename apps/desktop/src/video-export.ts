/**
 * Turns an edited video (`video-edit.ts`) into the file that is uploaded.
 *
 * The work is done inside the WebView with WebCodecs through Mediabunny, loaded on demand so the
 * library costs nothing until someone actually opens the editor. Nothing here touches the native
 * side: the result is an ordinary `File` handed to the same upload path as any other attachment,
 * and the native magic-byte check still decides whether it is served inline.
 *
 * Invariants the exporter keeps, and why:
 *
 * - Removed time is ABSENT from the output bytes, not hidden. Every kept frame is decoded and
 *   re-encoded (`copy: false`); nothing is copied packet-for-packet. A packet copy has to start on
 *   a key frame, so it would carry up to several seconds of footage from before a cut into the
 *   file, invisible to a player but recoverable by anyone holding the bytes. Someone cutting a
 *   moment out of a clip before sharing it expects that moment gone.
 * - Descriptive metadata is dropped (`tags: {}`). Phone footage commonly carries the recording
 *   location and device in its tags; a re-encode is the natural point to stop forwarding them.
 * - Output is bounded. Bytes go through `CappedBytes`, which refuses to grow past the server's
 *   upload limit, so a long or high-bitrate edit fails early with a clear reason instead of
 *   buffering an unbounded file in the WebView and being refused by the native side afterwards.
 *   (Turning the pages into one Blob at the end briefly holds about twice the output.)
 * - An untouched edit is never exported. The caller checks `isUnedited` and uploads the original
 *   bytes, so declining to edit costs no quality and no time.
 *
 * Known limits: an MP4 is written with its index at the end (`fastStart: false`), which is what
 * lets the output stream through the cap rather than being held whole by the muxer. Received
 * videos are fetched whole before they play, so that layout costs nothing here.
 *
 * Mediabunny starts blob-URL workers for video with an alpha channel and for live capture
 * sources. The app's CSP refuses blob workers, so `probeVideo` refuses any track that can be
 * transparent rather than letting the export fail halfway with a raw "Worker" error. Live
 * capture is never used here.
 */
import type {
  AudioSample, Conversion, DiscardedTrack, Input, StreamTargetChunk, VideoSample,
} from "mediabunny";
import { exportPlan, mapAudioChunk, mapVideoFrame, type ExportPlan, type VideoEdit } from "./video-edit.ts";
import { safeMediaMime } from "./media-safe.ts";

export type VideoContainer = "mp4" | "webm";

/**
 * What the editor hands back to the host, exactly once per opening.
 *
 * - `original`: upload the picked file's own bytes (nothing was edited, editing was not possible,
 *   or the person chose "as is" to set their edits aside).
 * - `edited`: upload `file` instead; it is a fresh re-encode named for its container.
 * - `skip`: upload nothing for this file.
 *
 * `post` is set by "post now": once this file is attached, the host sends the message (or posts
 * the announcement) straight away instead of leaving it in the composer. Only offered where there
 * is something to send; see `VideoEditorContext`.
 */
export type VideoEditorResult =
  | { kind: "original"; post?: boolean }
  | { kind: "edited"; file: File; post?: boolean }
  | { kind: "skip" };

/**
 * Where the upload is going, which decides the editor's wording and whether "post now" exists:
 * the chat and status composers attach and can post at once; a wiki page embeds at the caret and
 * the Files tab only shares, and neither has a message to send.
 */
export type VideoEditorContext = "chat" | "status" | "wiki" | "files";

/**
 * Declared types the editor will open, and the container each is written as. QuickTime becomes
 * MP4: the two share a box structure, every WebView plays MP4, and `.mov` is what phones produce.
 * Ogg (Theora) and AVI are absent because the demuxer cannot read their video; those still upload
 * unedited exactly as before.
 */
const EDITABLE_CONTAINERS: Readonly<Record<string, VideoContainer>> = {
  "video/mp4": "mp4",
  "video/quicktime": "mp4",
  "video/webm": "webm",
};

const CONTAINER_MIME: Readonly<Record<VideoContainer, string>> = {
  mp4: "video/mp4",
  webm: "video/webm",
};

/** The container an upload of this declared type would be edited into, or null when it cannot be. */
export function editableContainer(mime: string): VideoContainer | null {
  return EDITABLE_CONTAINERS[safeMediaMime(mime)] ?? null;
}

/** The MIME type the edited file is uploaded as. */
export function containerMime(container: VideoContainer): string {
  return CONTAINER_MIME[container];
}

/**
 * The name for the edited upload: the original stem with the container's extension, so a `.mov`
 * re-encoded as MP4 is not mislabelled. A name with no stem keeps a readable default.
 */
export function editedFileName(name: string, container: VideoContainer): string {
  const trimmed = name.trim();
  const dot = trimmed.lastIndexOf(".");
  const stem = (dot > 0 ? trimmed.slice(0, dot) : trimmed) || "video";
  return `${stem}.${container}`;
}

/**
 * Whether this WebView has the codec API at all. Without it the editor is not offered and the
 * upload goes ahead unedited, exactly as it did before the editor existed. Audio coders are checked
 * per file in `probeVideo`, because a silent clip does not need them.
 */
export function webCodecsAvailable(scope: object = globalThis): boolean {
  const g = scope as Record<string, unknown>;
  return typeof g.VideoDecoder === "function" && typeof g.VideoEncoder === "function";
}

// --- Bounded output -----------------------------------------------------------------------------

/** Thrown when the edited file would be bigger than the cap the export was given. */
export class OutputTooLargeError extends Error {
  readonly cap: number;
  constructor(cap: number) {
    super("the edited video is over the size limit");
    this.name = "OutputTooLargeError";
    this.cap = cap;
  }
}

/** Thrown when an export was cancelled through its signal. */
export class ExportCanceledError extends Error {
  constructor() {
    super("export canceled");
    this.name = "ExportCanceledError";
  }
}

/**
 * A positional byte buffer with a hard size cap, stored in fixed pages.
 *
 * The muxer writes mostly forward but goes back to patch box and element sizes, so this is a
 * random-access buffer rather than a list of chunks. Pages mean growth never copies what was
 * already written (a doubling array would briefly hold twice the file), and a page that was never
 * written (there are none in practice) reads as zeros.
 */
export class CappedBytes {
  readonly cap: number;
  readonly pageBytes: number;
  #pages: Uint8Array[] = [];
  #length = 0;

  constructor(cap: number, pageBytes = 4 * 1024 * 1024) {
    if (!Number.isSafeInteger(cap) || cap <= 0) throw new RangeError("cap must be a positive integer");
    if (!Number.isSafeInteger(pageBytes) || pageBytes <= 0) throw new RangeError("pageBytes must be a positive integer");
    this.cap = cap;
    this.pageBytes = pageBytes;
  }

  /** Bytes from 0 to the furthest byte written. */
  get length(): number {
    return this.#length;
  }

  /** Write `data` at `position`. Throws `OutputTooLargeError` (writing nothing) past the cap. */
  write(position: number, data: Uint8Array): void {
    if (!Number.isSafeInteger(position) || position < 0) throw new RangeError("bad write position");
    const end = position + data.byteLength;
    if (end > this.cap) throw new OutputTooLargeError(this.cap);
    let offset = 0;
    while (offset < data.byteLength) {
      const at = position + offset;
      const index = Math.floor(at / this.pageBytes);
      const within = at - index * this.pageBytes;
      const take = Math.min(this.pageBytes - within, data.byteLength - offset);
      let page = this.#pages[index];
      if (!page) {
        page = new Uint8Array(this.pageBytes);
        this.#pages[index] = page;
      }
      page.set(data.subarray(offset, offset + take), within);
      offset += take;
    }
    if (end > this.#length) this.#length = end;
  }

  /** Everything written, as one Blob of `length` bytes. */
  toBlob(type: string): Blob {
    const parts: Uint8Array[] = [];
    let remaining = this.#length;
    for (let index = 0; remaining > 0; index++) {
      const page = this.#pages[index] ?? new Uint8Array(this.pageBytes);
      const take = Math.min(this.pageBytes, remaining);
      parts.push(take === this.pageBytes ? page : page.subarray(0, take));
      remaining -= take;
    }
    return new Blob(parts as BlobPart[], { type });
  }
}

// --- Probe ----------------------------------------------------------------------------------------

export type VideoProbe =
  | { ok: true; duration: number; width: number; height: number; hasAudio: boolean }
  | { ok: false; reason: string };

type Mediabunny = typeof import("mediabunny");

let library: Promise<Mediabunny> | null = null;

/** Load Mediabunny once, on first use. A failed load is retried on the next call. */
function loadLibrary(): Promise<Mediabunny> {
  library ??= import("mediabunny").catch((error: unknown) => {
    library = null;
    throw error;
  });
  return library;
}

function openInput(mb: Mediabunny, file: Blob): Input {
  // Only the containers `EDITABLE_CONTAINERS` admits. Anything else is refused by the demuxer
  // rather than being sniffed into some other format the editor was never meant to open.
  return new mb.Input({ source: new mb.BlobSource(file), formats: [mb.MP4, mb.QTFF, mb.WEBM, mb.MATROSKA] });
}

/**
 * Read enough of `file` to decide whether it can be edited here, and the duration the timeline is
 * drawn against. The duration comes from the demuxer, not from the preview element, because it is
 * the same demuxer's timestamps the export cuts by; and a WebM written by a recorder often has no
 * duration the media element can report at all.
 */
export async function probeVideo(file: Blob): Promise<VideoProbe> {
  if (!webCodecsAvailable()) return { ok: false, reason: "Video editing is not supported in this window." };
  let mb: Mediabunny;
  try {
    mb = await loadLibrary();
  } catch {
    return { ok: false, reason: "The video editor could not be loaded." };
  }
  let input: Input | null = null;
  try {
    input = openInput(mb, file);
    const video = await input.getPrimaryVideoTrack();
    if (!video) return { ok: false, reason: "This file has no video track to edit." };
    if (!(await video.canDecode())) {
      return { ok: false, reason: `This video's format (${(await video.getCodec()) ?? "unknown"}) can't be edited here.` };
    }
    // See the header: alpha would need workers this app's CSP refuses. The decision follows the
    // packets, not `canBeTransparent()`: Chrome's recorder marks every canvas capture's WebM
    // header as alpha-capable, opaque or not, and that answer would turn away ordinary
    // recordings. Mediabunny only starts the workers for a packet that carries alpha data, and a
    // transparent video carries it from the first frame. A file whose alpha only starts later
    // fails at export with the generic message and the "upload unedited" way out.
    if (await video.canBeTransparent()) {
      const first = await new mb.EncodedPacketSink(video).getFirstPacket({ metadataOnly: true });
      if (first?.sideData.alpha || first?.sideData.alphaByteLength) {
        return { ok: false, reason: "Videos with transparency can't be edited here." };
      }
    }
    const audio = await input.getPrimaryAudioTrack();
    if (audio && !(await audio.canDecode())) {
      return { ok: false, reason: `This video's sound (${(await audio.getCodec()) ?? "unknown"}) can't be edited here.` };
    }
    // Over the tracks the export uses (`tracks: "primary"`), not every track the file carries:
    // a long commentary or data track would otherwise stretch the timeline past the picture.
    const duration = await input.computeDuration(audio ? [video, audio] : [video]);
    if (!Number.isFinite(duration) || duration <= 0) {
      return { ok: false, reason: "This video's length could not be read." };
    }
    return {
      ok: true,
      duration,
      width: await video.getDisplayWidth(),
      height: await video.getDisplayHeight(),
      hasAudio: audio !== null,
    };
  } catch {
    return { ok: false, reason: "This file could not be read as a video." };
  } finally {
    input?.dispose();
  }
}

// --- Filmstrip ------------------------------------------------------------------------------------

/** Most tiles a filmstrip will decode, whatever the caller asks for. */
export const FILMSTRIP_MAX_TILES = 48;

/**
 * Decode one small picture per entry of `times` (seconds, ascending) for the timeline filmstrip,
 * `height` pixels tall at the video's own aspect ratio. Each tile goes to `onTile` as it arrives,
 * so the strip fills in left to right. Aborting `signal` stops the walk between tiles.
 *
 * Never rejects: the strip is decoration, and an editor whose thumbnails failed still edits. A
 * timestamp before the first frame yields no tile for that index.
 */
export async function filmstrip(
  file: Blob,
  times: readonly number[],
  height: number,
  onTile: (index: number, image: CanvasImageSource) => void,
  signal?: AbortSignal,
): Promise<void> {
  if (!webCodecsAvailable() || signal?.aborted) return;
  let input: Input | null = null;
  try {
    const mb = await loadLibrary();
    input = openInput(mb, file);
    const track = await input.getPrimaryVideoTrack();
    if (!track || signal?.aborted) return;
    const sink = new mb.CanvasSink(track, { height: Math.max(8, Math.round(height)), fit: "contain" });
    let index = 0;
    for await (const tile of sink.canvasesAtTimestamps(times.slice(0, FILMSTRIP_MAX_TILES))) {
      if (signal?.aborted) break;
      if (tile) onTile(index, tile.canvas);
      index++;
    }
  } catch {
    // Decoration only; see above.
  } finally {
    input?.dispose();
  }
}

// --- Export ---------------------------------------------------------------------------------------

export type ExportOptions = {
  /** Largest output accepted, in bytes; normally the server's upload limit. */
  capBytes: number;
  /** Fraction complete, 0..1, as the conversion advances. */
  onProgress?: (fraction: number) => void;
  /** Aborting it cancels the conversion; the export then rejects with `ExportCanceledError`. */
  signal?: AbortSignal;
};

/**
 * An export refusal whose message is a sentence written for the person. Anything else that goes
 * wrong inside the library is replaced by `EXPORT_FAILED` before it reaches the editor: decoder
 * and muxer internals ("Failed to construct 'Worker'", codec strings) are not something to show.
 * The original stays on `cause` for debugging.
 */
class ExportRefusal extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ExportRefusal";
  }
}

const EXPORT_FAILED = "The edited video could not be made in this window.";

/** A sentence for a track the conversion had to leave out. */
function discardReason(discarded: DiscardedTrack): string {
  const what = discarded.track.type === "audio" ? "sound" : "picture";
  switch (discarded.reason) {
    case "undecodable_source_codec":
      return `This video's ${what} can't be decoded here.`;
    case "no_encodable_target_codec":
      return `This window can't encode the ${what} of an edited video.`;
    default:
      return `This video's ${what} can't be edited here.`;
  }
}

/** The video `process` callback: move a kept frame to its output time, drop a removed one. */
function videoProcessor(plan: ExportPlan) {
  return (sample: VideoSample): VideoSample | null => {
    const placed = mapVideoFrame(plan, sample.timestamp, sample.duration);
    if (!placed) return null;
    // Returning the sample it was given is supported: the conversion still owns and closes it.
    sample.setTimestamp(placed.timestamp);
    sample.setDuration(placed.duration);
    return sample;
  };
}

/** The audio `process` callback: keep only the kept slices of each chunk, at their output time. */
function audioProcessor(plan: ExportPlan) {
  return (sample: AudioSample): AudioSample | AudioSample[] | null => {
    const slices = mapAudioChunk(plan, sample.timestamp, sample.numberOfFrames, sample.sampleRate);
    if (slices.length === 0) return null;
    const whole = slices.length === 1 && slices[0].startFrame === 0 && slices[0].endFrame === sample.numberOfFrames;
    if (whole) {
      sample.setTimestamp(slices[0].timestamp);
      return sample;
    }
    // Each slice is a new sample; the source closes every returned sample once encoded.
    return slices.map((slice) => {
      const part = sample.trim(slice.startFrame, slice.endFrame);
      part.setTimestamp(slice.timestamp);
      return part;
    });
  };
}

/**
 * Re-encode the kept parts of `file` into a new file of the same container family.
 *
 * Rejects with `ExportCanceledError` when `signal` aborts, `OutputTooLargeError` when the output
 * would pass `capBytes`, and otherwise an `Error` whose message is a sentence for the person
 * (library internals are replaced, see `ExportRefusal`). It never resolves with a file that
 * silently lost its sound: a source audio track that cannot be carried over is an error, not a
 * quieter video.
 */
export async function exportEditedVideo(file: File, edit: VideoEdit, options: ExportOptions): Promise<File> {
  const container = editableContainer(file.type);
  if (!container) throw new ExportRefusal("This kind of video can't be edited.");
  const plan = exportPlan(edit);
  if (!plan) throw new ExportRefusal("Nothing is left to upload: every part has been cut.");
  if (options.signal?.aborted) throw new ExportCanceledError();

  let mb: Mediabunny;
  try {
    mb = await loadLibrary();
  } catch (error) {
    throw new Error("The video editor could not be loaded.", { cause: error });
  }
  const bytes = new CappedBytes(options.capBytes);
  let overflow: OutputTooLargeError | null = null;
  let conversion: Conversion | null = null;
  // Stop decoding and encoding promptly, once. Best-effort: after the cap errors the writable
  // stream, cancelling rejects because the stream can no longer be closed, and that rejection must
  // not escape as an unhandled one. The error that matters is reported through `execute` below.
  let stopped = false;
  const stop = () => {
    if (stopped || !conversion) return;
    stopped = true;
    conversion.cancel().catch(() => {});
  };
  const writable = new WritableStream<StreamTargetChunk>({
    write(chunk) {
      try {
        bytes.write(chunk.position, chunk.data);
      } catch (error) {
        if (error instanceof OutputTooLargeError) {
          overflow = error;
          // There is no point finishing a file that cannot be sent.
          stop();
        }
        throw error;
      }
    },
  });
  const input = openInput(mb, file);
  options.signal?.addEventListener("abort", stop, { once: true });
  try {
    const format = container === "mp4" ? new mb.Mp4OutputFormat({ fastStart: false }) : new mb.WebMOutputFormat();
    const output = new mb.Output({ format, target: new mb.StreamTarget(writable) });
    const hadAudio = (await input.getPrimaryAudioTrack()) !== null;
    conversion = await mb.Conversion.init({
      input,
      output,
      tracks: "primary",
      trim: { start: plan.trimStart, end: plan.trimEnd ?? undefined },
      copy: false,
      tags: {},
      showWarnings: false,
      video: { forceTranscode: true, process: videoProcessor(plan) },
      audio: { forceTranscode: true, process: audioProcessor(plan) },
    });
    const dropped = conversion.discardedTracks.find((d) => d.track.type === "video" || (hadAudio && d.track.type === "audio"));
    if (dropped) throw new ExportRefusal(discardReason(dropped));
    if (!conversion.isValid) throw new ExportRefusal("This video can't be edited here.");
    if (options.onProgress) {
      const report = options.onProgress;
      conversion.onProgress = (fraction) => report(Math.min(1, Math.max(0, fraction)));
    }
    if (options.signal?.aborted) throw new ExportCanceledError();
    await conversion.execute();
  } catch (error) {
    // Whatever failed, leave no encoder running behind the rejection (a no-op once it is done).
    stop();
    // The cap and a user's cancel both stop the conversion through `cancel()`, so the error the
    // conversion surfaces is a cancellation either way; the reason recorded first is the true one.
    if (overflow) throw overflow;
    if (options.signal?.aborted || error instanceof mb.ConversionCanceledError) throw new ExportCanceledError();
    if (error instanceof ExportRefusal || error instanceof ExportCanceledError) throw error;
    throw new Error(EXPORT_FAILED, { cause: error });
  } finally {
    options.signal?.removeEventListener("abort", stop);
    input.dispose();
  }
  const mime = containerMime(container);
  return new File([bytes.toBlob(mime)], editedFileName(file.name, container), { type: mime });
}
