/**
 * The clip maker's media bin: every picture, sound and extra video the person brings into a clip,
 * keyed by the `source` ids the document (`clip-doc.ts`) refers to.
 *
 * The document stays plain data; the bytes, decoded pictures and decoded sounds live here, owned by
 * one editor session and released when it closes (`dispose`).
 *
 * Everything here is LOCAL: files the person picked or dropped from their own disk, emoji drawn by
 * this window, the built-in sounds, and Studio music rendered on this device. Nothing is fetched.
 * (Server stickers and shared Files media are a later, separately reviewed step: those would be
 * bytes from other people, and they need the native CORS change first.)
 *
 * Bounds, so one file cannot exhaust the WebView. A file's size in bytes says little about what it
 * decodes to (a small PNG can declare 20000 x 20000 pixels; a low-bitrate sound can run for hours),
 * so each kind is bounded by what it DECODES to, before decoding:
 * - `MEDIA_LIMITS.imageBytes`/`soundBytes`/`takeBytes` refuse oversized files outright;
 * - a picture's dimensions are read from its header (`imageSize`) and anything over
 *   `MEDIA_LIMITS.imagePixels` is refused before it is decoded; it is then kept at most
 *   `MEDIA_LIMITS.imageEdge` on its long side;
 * - a sound is decoded only as far as the caller can use (the clip's length plus room to slide),
 *   and never past `MEDIA_LIMITS.soundFrames` samples per channel;
 * - the bin holds at most `MEDIA_LIMITS.items` things and `MEDIA_LIMITS.bytes` of decoded media.
 * SVG is refused outright: it is a document, not a picture, and nothing here needs one.
 */
import { TAKE_MAX_BYTES } from "./jam-contract.ts";
import { loadLibrary } from "./video-export.ts";

export const MEDIA_LIMITS = {
  imageBytes: 25 * 1024 * 1024,
  soundBytes: 60 * 1024 * 1024,
  takeBytes: TAKE_MAX_BYTES,
  /** Largest picture decoded at all (8000 x 5000): about 160 MB while it is scaled down. */
  imagePixels: 40_000_000,
  /** Long edge pictures are kept at; a sticker never needs more than a 1080p frame does. */
  imageEdge: 2048,
  /** Longest sound decoded, in samples per channel: ten and a half minutes at 48 kHz. */
  soundFrames: 48_000 * 630,
  /** Pictures, sounds and videos held by one session, the clip itself included. */
  items: 64,
  /** Decoded pictures and sounds held by one session, in bytes. */
  bytes: 1024 * 1024 * 1024,
} as const;

/** The refusal when the session's media is at its count or byte limit. */
export const BIN_FULL = "This clip holds as much as it can.";

/** A refusal whose message is a sentence for the person. */
export class MediaRefusal extends Error {
  constructor(message: string) { super(message); this.name = "MediaRefusal"; }
}

/**
 * A picture's width and height from the start of its file (PNG, JPEG, GIF, WebP, BMP), without
 * decoding it; null when the header is not one of these or is malformed.
 */
export function imageSize(b: Uint8Array): { width: number; height: number } | null {
  const u16be = (i: number) => (b[i] << 8) | b[i + 1];
  const u16le = (i: number) => b[i] | (b[i + 1] << 8);
  const u32be = (i: number) => ((b[i] << 24) >>> 0) + (b[i + 1] << 16) + (b[i + 2] << 8) + b[i + 3];
  const i32le = (i: number) => b[i] | (b[i + 1] << 8) | (b[i + 2] << 16) | (b[i + 3] << 24);
  const ascii = (i: number, s: string) => s.split("").every((c, k) => b[i + k] === c.charCodeAt(0));
  const ok = (width: number, height: number) => (width > 0 && height > 0 ? { width, height } : null);
  if (b.length >= 24 && b[0] === 0x89 && ascii(1, "PNG") && ascii(12, "IHDR")) return ok(u32be(16), u32be(20));
  if (b.length >= 13 && ascii(0, "GIF8")) return gifSize(b);
  if (b.length >= 26 && ascii(0, "BM")) return ok(Math.abs(i32le(18)), Math.abs(i32le(22)));
  if (b.length >= 30 && ascii(0, "RIFF") && ascii(8, "WEBP")) {
    if (ascii(12, "VP8 ")) return ok(u16le(26) & 0x3fff, u16le(28) & 0x3fff);
    if (ascii(12, "VP8L")) {
      const b0 = b[21], b1 = b[22], b2 = b[23], b3 = b[24];
      return ok(1 + (((b1 & 0x3f) << 8) | b0), 1 + (((b3 & 0x0f) << 10) | (b2 << 2) | ((b1 & 0xc0) >> 6)));
    }
    if (ascii(12, "VP8X")) return ok(1 + (b[24] | (b[25] << 8) | (b[26] << 16)), 1 + (b[27] | (b[28] << 8) | (b[29] << 16)));
    return null;
  }
  if (b.length >= 4 && b[0] === 0xff && b[1] === 0xd8) {
    // Walk the JPEG segments to the first start-of-frame marker, which carries the size.
    let i = 2;
    while (i + 9 < b.length) {
      if (b[i] !== 0xff) return null;
      const marker = b[i + 1];
      if (marker === 0xff) { i++; continue; }
      if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) { i += 2; continue; }
      const isFrame = marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
      if (isFrame) return ok(u16be(i + 7), u16be(i + 5));
      const length = u16be(i + 2);
      if (length < 2) return null;
      i += 2 + length;
    }
    return null;
  }
  return null;
}

/** Size emoji stickers are drawn at, in px. Stickers scale down cleanly from it. */
export const EMOJI_PX = 256;

export type MediaVideo = {
  readonly kind: "video"; readonly id: string; readonly name: string; readonly file: Blob;
  readonly duration: number; readonly width: number; readonly height: number; readonly hasAudio: boolean;
};
export type MediaImage = {
  readonly kind: "image"; readonly id: string; readonly name: string;
  readonly image: CanvasImageSource; readonly width: number; readonly height: number;
  /** Where it came from, and the key that finds it again (an emoji, so adding it twice reuses it). */
  readonly origin: "file" | "emoji"; readonly key?: string;
};
export type MediaSound = {
  readonly kind: "sound"; readonly id: string; readonly name: string; readonly buffer: AudioBuffer;
  readonly origin: "studio" | "file" | "builtin"; readonly key?: string;
};
export type MediaItem = MediaVideo | MediaImage | MediaSound;
type WithoutId<T> = T extends unknown ? Omit<T, "id"> : never;

/** What a picked or dropped file is to the clip maker, or null when it is none of these. */
export type FileRole = "image" | "video" | "audio" | "take";

/**
 * A GIF's size as a browser decodes it: its declared screen, grown to hold every frame. A GIF's
 * screen size is only a claim; Chromium enlarges the image to fit a frame that does not fit it, so
 * a file declaring a 1 x 1 screen can carry a 20000 x 20000 frame. Every frame descriptor is read
 * (frame data is skipped block by block), and a file that ends mid-structure is malformed.
 */
function gifSize(b: Uint8Array): { width: number; height: number } | null {
  const u16le = (i: number) => b[i] | (b[i + 1] << 8);
  let width = u16le(6), height = u16le(8);
  const tableSize = (packed: number) => (packed & 0x80 ? 3 * (1 << ((packed & 7) + 1)) : 0);
  /** Skip a run of data sub-blocks; the index after its terminator, or -1 if the file ends. */
  const skipBlocks = (i: number) => {
    while (i < b.length) {
      const size = b[i];
      if (size === 0) return i + 1;
      i += 1 + size;
    }
    return -1;
  };
  let i = 13 + tableSize(b[10]);
  let frames = 0;
  while (i < b.length) {
    const block = b[i];
    if (block === 0x3b) break; // trailer
    if (block === 0x21) { // extension: label, then sub-blocks
      i = skipBlocks(i + 2);
      if (i < 0) return null;
    } else if (block === 0x2c) { // image descriptor: left, top, width, height, packed
      if (i + 10 > b.length) return null;
      width = Math.max(width, u16le(i + 1) + u16le(i + 5));
      height = Math.max(height, u16le(i + 3) + u16le(i + 7));
      frames++;
      i = skipBlocks(i + 10 + tableSize(b[i + 9]) + 1); // + LZW minimum code size
      if (i < 0) return null;
    } else {
      return null;
    }
  }
  return frames > 0 && width > 0 && height > 0 ? { width, height } : null;
}

/** The picture formats whose size `imageSize` can read before decoding. AVIF is not among them. */
const IMAGE_TYPES = new Set(["image/png", "image/jpeg", "image/webp", "image/gif", "image/bmp"]);

/** Decide by declared type, then by extension when the type was lost. SVG is never a picture here. */
export function fileRole(name: string, type: string): FileRole | null {
  const mime = type.trim().toLowerCase();
  const ext = name.trim().toLowerCase().replace(/^.*(\.[a-z0-9]+)$/, "$1");
  if (mime === "image/svg+xml" || ext === ".svg") return null;
  if (ext === ".jamtake") return "take";
  if (IMAGE_TYPES.has(mime)) return "image";
  if (mime.startsWith("video/")) return "video";
  if (mime.startsWith("audio/")) return "audio";
  if ([".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp"].includes(ext)) return "image";
  if ([".mp4", ".mov", ".webm", ".m4v"].includes(ext)) return "video";
  if ([".mp3", ".wav", ".ogg", ".oga", ".m4a", ".aac", ".flac", ".opus", ".weba"].includes(ext)) return "audio";
  return null;
}

/** A refusal before decoding, as a sentence for the person, or null when the file may be read. */
export function fileRefusal(role: FileRole, size: number): string | null {
  const cap = role === "image" ? MEDIA_LIMITS.imageBytes : role === "audio" ? MEDIA_LIMITS.soundBytes
    : role === "take" ? MEDIA_LIMITS.takeBytes : Number.POSITIVE_INFINITY;
  if (!Number.isFinite(size) || size < 0) return "This file could not be read.";
  if (size > cap) return role === "image" ? "That picture is too big to use here." : role === "take" ? "That music file is too big." : "That sound file is too big to use here.";
  return null;
}

/** The size to decode a `w` x `h` picture at: inside `MEDIA_LIMITS.imageEdge`, shape kept. */
export function decodeSize(w: number, h: number): { width: number; height: number } {
  if (!(w > 0) || !(h > 0)) return { width: 1, height: 1 };
  const k = Math.min(1, MEDIA_LIMITS.imageEdge / Math.max(w, h));
  return { width: Math.max(1, Math.round(w * k)), height: Math.max(1, Math.round(h * k)) };
}

/** A short display name: the file name without its extension, trimmed to fit a library row. */
export function mediaName(fileName: string, limit = 28): string {
  const base = fileName.trim().replace(/^.*[\\/]/, "");
  const dot = base.lastIndexOf(".");
  const stem = (dot > 0 ? base.slice(0, dot) : base).trim() || "untitled";
  return stem.length > limit ? `${stem.slice(0, limit - 1)}…` : stem;
}

/**
 * The bin itself. Ids are `base` for the clip being edited and `m1`, `m2`... after that, never
 * reused, so a document in undo history always points at the media it meant.
 */
export class MediaBin {
  readonly #items = new Map<string, MediaItem>();
  #seq = 0;
  #bytes = 0;
  readonly #budget: number;

  constructor(base: Omit<MediaVideo, "id" | "kind">, budget: number = MEDIA_LIMITS.bytes) {
    this.#items.set("base", { kind: "video", id: "base", ...base });
    this.#budget = budget;
  }

  get size(): number { return this.#items.size; }
  get full(): boolean { return this.#items.size >= MEDIA_LIMITS.items; }
  /** Decoded bytes held. */
  get bytes(): number { return this.#bytes; }

  /**
   * Whether one more item costing `bytes` would be kept. Asked before decoding or rendering, so
   * nothing large is made only to be refused by `add` afterwards (found in review: a song was
   * rendered in full, then dropped for want of room).
   */
  room(bytes: number): boolean {
    return !this.full && Number.isFinite(bytes) && bytes >= 0 && this.#bytes + bytes <= this.#budget;
  }

  /** Keep `item`; returns its new id, or null when the bin is full (by count or by bytes). */
  add(item: WithoutId<MediaItem>): string | null {
    const cost = decodedBytes(item);
    if (!this.room(cost)) return null;
    const id = `m${++this.#seq}`;
    this.#items.set(id, { ...item, id } as MediaItem);
    this.#bytes += cost;
    return id;
  }

  get(id: string): MediaItem | null { return this.#items.get(id) ?? null; }
  video(id: string): MediaVideo | null { const m = this.#items.get(id); return m?.kind === "video" ? m : null; }
  image(id: string): MediaImage | null { const m = this.#items.get(id); return m?.kind === "image" ? m : null; }
  sound(id: string): MediaSound | null { const m = this.#items.get(id); return m?.kind === "sound" ? m : null; }
  /** An item added before under `key` (the same emoji, built-in sound or Studio song). */
  byKey(key: string): MediaItem | null {
    for (const m of this.#items.values()) if (m.kind !== "video" && m.key === key) return m;
    return null;
  }
  list<K extends MediaItem["kind"]>(kind: K): Extract<MediaItem, { kind: K }>[] {
    return [...this.#items.values()].filter((m): m is Extract<MediaItem, { kind: K }> => m.kind === kind);
  }

  /** Release decoded pictures. The bin is unusable afterwards. */
  dispose(): void {
    for (const m of this.#items.values()) {
      if (m.kind === "image") (m.image as { close?: () => void }).close?.();
    }
    this.#items.clear();
  }
}

/**
 * Runs the clip maker's heavy work (decoding a picture or a sound, rendering a song) one job at a
 * time, in the order asked. Without it, clicking down a list of songs started every render at
 * once, each allocating its whole output before any of them reached the bin's budget check.
 *
 * One queue is shared by every editor (see the module script in `VideoEditor.svelte`), so an
 * editor closing while its job runs does not let the next editor's job start alongside it.
 *
 * A job holds the queue until it settles OR its `signal` aborts (its editor answered or went).
 * Without the second, one job that never settled (a decoder stalled by a crafted file, say) would
 * hold up every later job in every later editor until the app restarted (found in review). A
 * released job may still be running; what it can hold was admitted against its own bin first.
 * The abort only skips that job's own turn: it never lets later jobs past one queued before it.
 * A job queued by an editor that has since closed should check that first and return straight
 * away.
 */
export class JobQueue {
  #tail: Promise<void> = Promise.resolve();

  /** Run `job` once every job queued before it has been released; resolves or rejects as it does. */
  run<T>(job: () => Promise<T>, signal?: AbortSignal): Promise<T> {
    const previous = this.#tail;
    const result = previous.then(job);
    const settled = result.then(() => {}, () => {});
    // Waits for `previous` first either way, so an abort cannot release the queue early.
    this.#tail = signal ? previous.then(() => Promise.race([settled, abortOf(signal)])) : settled;
    return result;
  }
}

/** Resolves when `signal` aborts (at once if it already has). */
function abortOf(signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted) resolve();
    else signal.addEventListener("abort", () => resolve(), { once: true });
  });
}

/** What an item costs in decoded memory: RGBA pictures, float samples. A video stays a file. */
export function decodedBytes(item: WithoutId<MediaItem>): number {
  if (item.kind === "image") return item.width * item.height * 4;
  if (item.kind === "sound") return item.buffer.length * item.buffer.numberOfChannels * 4;
  return 0;
}

// --- Browser decoders ---------------------------------------------------------------------------

/**
 * Decode a picture file, kept at most `MEDIA_LIMITS.imageEdge` on its long side. Its size is read
 * from the header first, and a picture that would decode over `MEDIA_LIMITS.imagePixels` (or whose
 * header cannot be read) is refused without being decoded. Browser-only.
 *
 * `admit` is asked, with the bytes the kept picture will hold, before anything is decoded; false
 * refuses it as "this clip holds as much as it can".
 */
export async function decodeImageFile(file: Blob, admit?: (bytes: number) => boolean): Promise<{ image: ImageBitmap; width: number; height: number }> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  const size = imageSize(bytes);
  if (!size) throw new MediaRefusal("That picture could not be read.");
  if (size.width * size.height > MEDIA_LIMITS.imagePixels) throw new MediaRefusal("That picture is too big to use here.");
  // The kept size from the header's shape. EXIF rotation may swap the sides, never the area.
  const kept = decodeSize(size.width, size.height);
  if (admit && !admit(kept.width * kept.height * 4)) throw new MediaRefusal(BIN_FULL);
  // Decoded at full size first (at most the pixel cap), then scaled from what was DECODED: a
  // phone photo's EXIF orientation is applied by the decode, so its upright width and height can
  // be the header's swapped. Scaling to the header's shape would squash it (found in review).
  // Rotation never changes the pixel count, so the check above still bounds this decode.
  const full = await createImageBitmap(new Blob([bytes]));
  const want = decodeSize(full.width, full.height);
  if (want.width === full.width && want.height === full.height) return { image: full, width: full.width, height: full.height };
  try {
    const image = await createImageBitmap(full, { resizeWidth: want.width, resizeHeight: want.height, resizeQuality: "high" });
    return { image, width: image.width, height: image.height };
  } finally {
    full.close();
  }
}

/**
 * Draw an emoji as a sticker picture. Drawn by this window's own emoji font, so it is a picture of
 * what the person saw in the grid. Browser-only.
 */
export async function emojiImage(emoji: string, px = EMOJI_PX): Promise<{ image: ImageBitmap; width: number; height: number }> {
  const canvas = new OffscreenCanvas(px, px);
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.font = `${Math.round(px * 0.82)}px "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif`;
  ctx.fillText(emoji, px / 2, px * 0.54);
  return { image: await createImageBitmap(canvas), width: px, height: px };
}

/** How many seconds of a sound at `rate` to decode: what the caller can use, within the sample cap. */
export function soundDecodeSeconds(capSeconds: number, duration: number, rate: number): number {
  if (!(rate > 0) || !(duration > 0)) return 0;
  return Math.max(0, Math.min(capSeconds, duration, MEDIA_LIMITS.soundFrames / rate));
}

/**
 * Decode a sound file, but only its first `capSeconds` (the caller passes what the clip can use)
 * and never past `MEDIA_LIMITS.soundFrames` samples per channel, so a long or very low-bitrate
 * file cannot decode to gigabytes. Up to two channels are kept. Browser-only.
 *
 * `admit` is asked with the output's size in bytes before it is allocated (false refuses it), and
 * `signal` is checked before that and after every decoded chunk; a cancel rejects with an
 * `AbortError` and leaves the decoder (ending the loop ends the sink's iterator).
 */
export async function decodeSoundFile(
  file: Blob,
  capSeconds: number,
  options: { readonly admit?: (bytes: number) => boolean; readonly signal?: AbortSignal } = {},
): Promise<AudioBuffer> {
  const stop = () => { if (options.signal?.aborted) throw new DOMException("decode aborted", "AbortError"); };
  stop();
  const mb = await loadLibrary();
  const input = new mb.Input({ source: new mb.BlobSource(file), formats: mb.ALL_FORMATS });
  try {
    const track = await input.getPrimaryAudioTrack();
    if (!track || !(await track.canDecode())) throw new MediaRefusal("That sound file could not be read.");
    const rate = await track.getSampleRate();
    const channels = Math.min(2, Math.max(1, await track.getNumberOfChannels()));
    const seconds = soundDecodeSeconds(capSeconds, await input.computeDuration([track]), rate);
    const frames = Math.ceil(seconds * rate);
    if (frames < 1) throw new MediaRefusal("That sound file is empty.");
    stop();
    if (options.admit && !options.admit(frames * channels * 4)) throw new MediaRefusal(BIN_FULL);
    const out = new AudioBuffer({ length: frames, numberOfChannels: channels, sampleRate: rate });
    for await (const chunk of new mb.AudioBufferSink(track).buffers(0, seconds)) {
      stop();
      const at = Math.round(chunk.timestamp * rate);
      const from = Math.max(0, -at), to = Math.max(0, at);
      const count = Math.min(chunk.buffer.length - from, frames - to);
      if (count <= 0) continue;
      for (let c = 0; c < channels; c++) {
        out.copyToChannel(chunk.buffer.getChannelData(Math.min(c, chunk.buffer.numberOfChannels - 1)).subarray(from, from + count), c, to);
      }
    }
    return out;
  } finally {
    input.dispose();
  }
}

/** Read a `.jamtake` file's JSON. Validation is the renderer's (`renderTakeOffline`). A parse error
 * is a sentence, not the parser's text (which quotes the file). */
export async function readTakeFile(file: Blob): Promise<unknown> {
  try { return JSON.parse(await file.text()); }
  catch { throw new MediaRefusal("That music file could not be read."); }
}
