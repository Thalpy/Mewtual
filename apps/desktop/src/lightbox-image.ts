/**
 * Planning and painting for the image viewer's "fit to window" copy.
 *
 * Why this exists: a Linux tester's whole laptop froze while they flipped an image in the viewer
 * between "fit" and "actual size". Their kernel log blamed the Intel GPU's render engine, stuck in
 * WebKitGTK's compositor and Skia GPU worker threads inside Mewtual's web process, and then the
 * driver's reset of the chip timed out, which took the display down with it (2026-10-20). The
 * failed reset is a driver bug, but the job it choked on was ours: the viewer showed the
 * full-resolution original in both modes, so every switch to "fit" had the GPU shrink a large
 * photo (up to the native 25-megapixel bound) again from scratch.
 *
 * The fit view now shows a copy resampled once, when the viewer opens, to the size the screen can
 * actually show. Toggling only hides one element and shows the other, so the expensive shrink
 * happens at most once per opening instead of once per switch. "Actual size" still shows the
 * original, because a 1:1 view needs every pixel.
 *
 * The copy is drawn from the `<img>` the chat already decoded, not from the file's bytes: the
 * webview's CSP keeps `catcoms-media:` out of `connect-src`, so script cannot read media bytes,
 * and widening that boundary for a rendering optimisation is not worth it. Drawing a cross-origin
 * image into a canvas taints it, which is fine here because the canvas is only displayed.
 */

/** A size in device pixels. */
export type PixelSize = { width: number; height: number };

/**
 * The formats a fit copy may be made for. A canvas holds one still frame, so a format that can
 * animate would show frozen in the fit view. What reaches the webview animated is decided natively:
 * `serve_media` decodes every shared image in Rust and serves a PNG it produced, and only GIF and
 * animated WebP come out as APNG; PNG input (including an uploaded APNG) is always flattened to a
 * still (`media_decode.rs`). GIF is excluded here outright. An animated WebP qualifies only with at
 * least twice the screen's pixels (see `MIN_PIXEL_SAVING`), and the native animation budget keeps
 * at most 24 million pixels of frames, so at that size it has a handful of frames at most; leaving
 * every large photo on the expensive path to spare that would be the worse trade. If one does
 * arrive, its fit view shows the first frame and "Actual size" still plays it.
 *
 * Eligibility must stay on the declared MIME from the file index, never the served `Content-Type`:
 * that header is always `image/png`, so switching to it would freeze every GIF. The native decoder
 * is also chosen by the declared MIME, so a mislabelled file fails to decode rather than reaching
 * the viewer as something else.
 */
export const PRESCALE_MIMES: ReadonlySet<string> = new Set(["image/jpeg", "image/png", "image/webp"]);

/**
 * Only resample when the original has at least this many times the pixels of the whole box (the
 * screen at full scale), whatever its shape. Measured against the box, not against the copy: a
 * portrait clip on a landscape screen gets a small copy, so "twice the copy" would freeze an
 * animated WebP no bigger than the screen while saving the GPU nothing (found in review). Below
 * this the GPU's shrink is cheap and the resample is not worth a still frame.
 */
export const MIN_PIXEL_SAVING = 2;

/** Upper bound on either side of the copy, whatever the screen and scale claim. */
export const MAX_COPY_SIDE = 8192;

/** Device-pixel ratios outside this range are treated as nonsense and clamped. */
const MIN_DPR = 1;
const MAX_DPR = 4;

function positive(n: number): boolean {
  return Number.isFinite(n) && n > 0;
}

/**
 * The most device pixels the fit view can ever show: the larger of the screen and the window, at
 * the current scale. Taking the screen rather than the stage means a later maximise or resize
 * still has enough pixels, so the copy is made once per opening and never redone. Moving the
 * window to a higher-density monitor while the viewer is open leaves the copy slightly soft until
 * it is reopened.
 *
 * Returns null when nothing usable is known, which turns the optimisation off rather than guessing.
 */
export function viewerPixelBox(screen: PixelSize, inner: PixelSize, dpr: number): PixelSize | null {
  const scale = Number.isFinite(dpr) ? Math.min(MAX_DPR, Math.max(MIN_DPR, dpr)) : MIN_DPR;
  const side = (a: number, b: number): number => {
    const known = [a, b].filter(positive);
    return known.length ? Math.max(...known) : 0;
  };
  const w = side(screen.width, inner.width);
  const h = side(screen.height, inner.height);
  if (!w || !h) return null;
  return {
    width: Math.min(MAX_COPY_SIDE, Math.ceil(w * scale)),
    height: Math.min(MAX_COPY_SIDE, Math.ceil(h * scale)),
  };
}

/** `"Image/JPEG; foo=bar"` → `"image/jpeg"`. */
export function normalizeMime(mime: string): string {
  return mime.split(";", 1)[0].trim().toLowerCase();
}

/**
 * The size of the fit copy for an image, or null to show the original as before.
 *
 * Null when: the format may animate or is unknown, the image has not decoded (zero natural size),
 * no box is known, the image already fits the box, or the original has fewer than
 * `MIN_PIXEL_SAVING` times the box's pixels. The copy keeps the aspect ratio, never exceeds the box
 * and is never larger than the original.
 */
export function planFitCopy(input: { mime: string; naturalWidth: number; naturalHeight: number; box: PixelSize | null }): PixelSize | null {
  const { naturalWidth: w, naturalHeight: h, box } = input;
  if (!PRESCALE_MIMES.has(normalizeMime(input.mime))) return null;
  if (!positive(w) || !positive(h) || !box || !positive(box.width) || !positive(box.height)) return null;
  if (w <= box.width && h <= box.height) return null;
  if (w * h < MIN_PIXEL_SAVING * box.width * box.height) return null;
  // The side that overflows the box by more is the binding one: it becomes exactly the box's side
  // and the other follows the aspect ratio. Comparing by cross-multiplication, not by dividing,
  // keeps float error from putting the binding side a pixel short. Since the image overflows on
  // the binding side, neither side can come out larger than the original. The other side is at
  // least one pixel, because a 100000×50 panorama still needs a row to draw into.
  let width: number, height: number;
  if (w * box.height >= h * box.width) {
    width = box.width;
    height = Math.max(1, Math.min(box.height, Math.round((h * box.width) / w)));
  } else {
    height = box.height;
    width = Math.max(1, Math.min(box.width, Math.round((w * box.height) / h)));
  }
  return { width, height };
}

/**
 * Resample `source` into `canvas` at `size`. Returns false, leaving the canvas released, when the
 * browser gives no 2D context or the draw throws; the caller then shows the original as before.
 * Only an image the engine already knows is broken makes the draw throw. A decode or allocation
 * that fails inside the engine draws nothing and still returns true, and that cannot be detected
 * here: the canvas is tainted by a cross-origin image, so its pixels cannot be read back.
 *
 * `willReadFrequently` asks the engine for a CPU-backed canvas. That is the point: the one
 * expensive shrink runs on the processor, where it cannot wedge the GPU. It is a hint, not a
 * guarantee; an engine that ignores it does the shrink on the GPU, which is still once per
 * opening instead of once per switch.
 *
 * Cost: the draw runs on the main thread and includes a full-size decode of the source, because
 * the chat only ever needed the image small. The source is bounded upstream: shared images reach
 * the viewer only through the native `catcoms-media:` handler, which decodes them in Rust and
 * serves a PNG it produced, refusing anything over 25 megapixels or 16384 pixels a side
 * (`DecodeBounds::DEFAULT` in `src-tauri/src/media_decode.rs`). Because the source is always that
 * PNG, the decode is a PNG decode. Measured in review in headless Chromium on a fast desktop: 71 to
 * 173 ms at 12 MP and 145 to 418 ms at 25 MP, from flat to grainy content (the native encoder
 * writes about 1.9 bytes a pixel for photos, between those two). So opening a large photo costs
 * roughly 0.1 to 0.4 s once, more on a slower machine. WebKitGTK is unmeasured. Awaiting
 * `img.decode()` first does not help in Chromium (its draw decodes again) and `createImageBitmap`
 * with a resize ran synchronously too, so neither is used.
 */
export function paintFitCopy(canvas: HTMLCanvasElement, source: CanvasImageSource, size: PixelSize): boolean {
  // Size first: assigning width/height clears the canvas, and must happen before the first draw.
  canvas.width = size.width;
  canvas.height = size.height;
  let context: CanvasRenderingContext2D | null = null;
  try {
    context = canvas.getContext("2d", { willReadFrequently: true });
  } catch {
    context = null;
  }
  if (!context) {
    releaseCanvas(canvas);
    return false;
  }
  context.imageSmoothingEnabled = true;
  context.imageSmoothingQuality = "high";
  try {
    context.drawImage(source, 0, 0, size.width, size.height);
  } catch {
    releaseCanvas(canvas);
    return false;
  }
  return true;
}

/** Drop a canvas's pixel buffer now rather than whenever the element is collected. */
export function releaseCanvas(canvas: HTMLCanvasElement): void {
  canvas.width = 0;
  canvas.height = 0;
}
