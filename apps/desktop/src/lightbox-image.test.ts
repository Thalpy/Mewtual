import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import {
  MAX_COPY_SIDE, MIN_PIXEL_SAVING, normalizeMime, paintFitCopy, planFitCopy, releaseCanvas, viewerPixelBox,
} from "./lightbox-image.ts";

const laptop = { width: 1920, height: 1080 };

test("the box is the larger of screen and window, at the device scale", () => {
  assert.deepEqual(viewerPixelBox(laptop, { width: 1280, height: 720 }, 1), laptop);
  // A window bigger than the reported screen (a second, larger monitor) wins.
  assert.deepEqual(viewerPixelBox(laptop, { width: 2560, height: 1440 }, 1), { width: 2560, height: 1440 });
  // Fractional scaling rounds up, so the copy is never a pixel short of the screen.
  assert.deepEqual(viewerPixelBox(laptop, { width: 0, height: 0 }, 1.25), { width: 2400, height: 1350 });
  assert.deepEqual(viewerPixelBox(laptop, laptop, 2), { width: 3840, height: 2160 });
});

test("the box survives nonsense from the platform and turns itself off when it knows nothing", () => {
  // A scale that is missing, zero or absurd is clamped rather than trusted.
  assert.deepEqual(viewerPixelBox(laptop, laptop, Number.NaN), laptop);
  assert.deepEqual(viewerPixelBox(laptop, laptop, 0), laptop);
  assert.deepEqual(viewerPixelBox(laptop, laptop, 0.5), laptop);
  assert.deepEqual(viewerPixelBox(laptop, laptop, 50), { width: 7680, height: 4320 });
  // A screen the engine reports as zero (headless, some compositors) falls back to the window.
  assert.deepEqual(viewerPixelBox({ width: 0, height: 0 }, { width: 1024, height: 768 }, 1), { width: 1024, height: 768 });
  assert.deepEqual(viewerPixelBox({ width: Number.NaN, height: -5 }, { width: 1024, height: 768 }, 1), { width: 1024, height: 768 });
  // Nothing usable at all: no box, so no copy, rather than a guessed size.
  assert.equal(viewerPixelBox({ width: 0, height: 0 }, { width: 0, height: 0 }, 1), null);
  assert.equal(viewerPixelBox({ width: 1920, height: 0 }, { width: 0, height: 0 }, 1), null);
  // Each side is capped, whatever the screen and scale claim.
  assert.deepEqual(viewerPixelBox({ width: 7680, height: 4320 }, laptop, 4), { width: MAX_COPY_SIDE, height: MAX_COPY_SIDE });
});

test("a large photo gets a copy that fits the box and keeps its shape", () => {
  // A 12-megapixel phone photo on a 1080p laptop: about one sixth of the pixels.
  assert.deepEqual(planFitCopy({ mime: "image/jpeg", naturalWidth: 4032, naturalHeight: 3024, box: laptop }), { width: 1440, height: 1080 });
  // A tall image is bounded by height, a wide one by width.
  assert.deepEqual(planFitCopy({ mime: "image/png", naturalWidth: 3000, naturalHeight: 9000, box: laptop }), { width: 360, height: 1080 });
  assert.deepEqual(planFitCopy({ mime: "image/webp", naturalWidth: 9000, naturalHeight: 3000, box: laptop }), { width: 1920, height: 640 });
  // An extreme panorama still gets at least one row, inside the box. (100000x50: it needs twice
  // the box's pixels to qualify at all, so the old 100000x10 example now keeps its original.)
  assert.deepEqual(planFitCopy({ mime: "image/jpeg", naturalWidth: 100000, naturalHeight: 50, box: laptop }), { width: 1920, height: 1 });
});

test("the threshold is the box's pixels, so an image shaped unlike the screen is not frozen for nothing", () => {
  // Found in review: measured against the copy's own pixels, these all got still copies although
  // none has more pixels than the screen, so an animated WebP or APNG would stop in the fit view
  // while saving the GPU nothing.
  assert.equal(planFitCopy({ mime: "image/webp", naturalWidth: 1080, naturalHeight: 1920, box: laptop }), null, "portrait clip on a landscape screen");
  assert.equal(planFitCopy({ mime: "image/webp", naturalWidth: 720, naturalHeight: 1280, box: { width: 1366, height: 768 } }), null, "smaller than the screen");
  assert.equal(planFitCopy({ mime: "image/png", naturalWidth: 300, naturalHeight: 3000, box: laptop }), null, "a tall strip");
  // Shape does not matter, only pixels: a tall image qualifies at twice the box's pixels.
  assert.equal(planFitCopy({ mime: "image/png", naturalWidth: 1400, naturalHeight: 2900, box: laptop }), null, "4.06 MP, under 2 x 2.07 MP");
  assert.deepEqual(planFitCopy({ mime: "image/png", naturalWidth: 2000, naturalHeight: 4200, box: laptop }), { width: 514, height: 1080 }, "8.4 MP");
});

test("the copy is only made when it is a real saving, and never enlarges", () => {
  // Already fits: the original is shown as before.
  assert.equal(planFitCopy({ mime: "image/jpeg", naturalWidth: 1920, naturalHeight: 1080, box: laptop }), null);
  assert.equal(planFitCopy({ mime: "image/jpeg", naturalWidth: 800, naturalHeight: 600, box: laptop }), null);
  // A 1440p screenshot on a 1080p screen is only 1.78 times the copy: not worth a main-thread resample.
  assert.equal(planFitCopy({ mime: "image/png", naturalWidth: 2560, naturalHeight: 1440, box: laptop }), null);
  // Just over the threshold does get one.
  const plan = planFitCopy({ mime: "image/png", naturalWidth: 2716, naturalHeight: 1528, box: laptop });
  assert.ok(plan, "1.42 times each side is just over twice the box's pixels");
  assert.ok(2716 * 1528 >= MIN_PIXEL_SAVING * laptop.width * laptop.height);
  assert.ok(2716 * 1528 - MIN_PIXEL_SAVING * laptop.width * laptop.height < 3000, "and only just");
});

test("formats that may animate, unknown formats and undecoded images keep the original", () => {
  const big = { naturalWidth: 6000, naturalHeight: 4000, box: laptop };
  for (const mime of ["image/gif", "image/apng", "image/avif", "image/svg+xml", "image/bmp", "", "application/octet-stream"]) {
    assert.equal(planFitCopy({ mime, ...big }), null, `${mime || "(none)"} must not get a still copy`);
  }
  // The declared label is normalised, not matched byte for byte.
  assert.deepEqual(planFitCopy({ mime: " Image/JPEG; charset=binary", ...big }), { width: 1620, height: 1080 });
  assert.equal(normalizeMime("IMAGE/PNG ;x=y"), "image/png");
  // Zero, negative and non-finite sizes mean the image never decoded.
  for (const [w, h] of [[0, 0], [0, 4000], [6000, 0], [Number.NaN, 4000], [6000, Number.POSITIVE_INFINITY], [-6000, 4000]]) {
    assert.equal(planFitCopy({ mime: "image/jpeg", naturalWidth: w, naturalHeight: h, box: laptop }), null, `${w}x${h}`);
  }
  assert.equal(planFitCopy({ mime: "image/jpeg", ...big, box: null }), null);
  assert.equal(planFitCopy({ mime: "image/jpeg", ...big, box: { width: 0, height: 1080 } }), null);
});

/** A canvas stand-in that records the order of operations the GPU-safety claim depends on. */
function recordingCanvas(context: "ok" | "null" | "throws" | "draw-throws") {
  const log: string[] = [];
  let width = 300, height = 150;
  const ctx = {
    imageSmoothingEnabled: false,
    imageSmoothingQuality: "low",
    drawImage(...args: unknown[]) {
      log.push(`draw ${args.slice(1).join(",")} at ${width}x${height} smoothing=${this.imageSmoothingEnabled}/${this.imageSmoothingQuality}`);
      if (context === "draw-throws") throw new Error("InvalidStateError");
    },
  };
  const canvas = {
    get width() { return width; },
    set width(v: number) { width = v; log.push(`width ${v}`); },
    get height() { return height; },
    set height(v: number) { height = v; log.push(`height ${v}`); },
    getContext(kind: string, options: unknown) {
      log.push(`getContext ${kind} ${JSON.stringify(options)}`);
      if (context === "throws") throw new Error("no context");
      return context === "null" ? null : ctx;
    },
  };
  return { canvas: canvas as unknown as HTMLCanvasElement, log };
}

test("painting sizes the canvas, asks for a CPU-backed context and draws once with high smoothing", () => {
  const { canvas, log } = recordingCanvas("ok");
  const source = {} as CanvasImageSource;
  assert.equal(paintFitCopy(canvas, source, { width: 1440, height: 1080 }), true);
  assert.deepEqual(log, [
    "width 1440",
    "height 1080",
    'getContext 2d {"willReadFrequently":true}',
    "draw 0,0,1440,1080 at 1440x1080 smoothing=true/high",
  ]);
});

test("a context the browser will not give, or a draw that throws, reports failure and frees the buffer", () => {
  for (const mode of ["null", "throws", "draw-throws"] as const) {
    const { canvas } = recordingCanvas(mode);
    assert.equal(paintFitCopy(canvas, {} as CanvasImageSource, { width: 1440, height: 1080 }), false, mode);
    assert.equal(canvas.width, 0, `${mode}: buffer released`);
    assert.equal(canvas.height, 0, `${mode}: buffer released`);
  }
});

test("nothing in the viewer reads the copy's pixels back", () => {
  // The canvas is drawn from decrypted media on another origin. The webview's CSP keeps script
  // from fetching media bytes, and the tainted canvas keeps it from reading them back; an API
  // that reads or exports canvas pixels added here would be the first crack in that boundary.
  // `willReadFrequently` is only a rendering hint and must not invite one.
  for (const file of ["./lightbox-image.ts", "./LightboxPicture.svelte"]) {
    const source = readFileSync(new URL(file, import.meta.url), "utf8");
    // Calls, not mentions: the comments explain why createImageBitmap is not used.
    assert.doesNotMatch(source, /\b(getImageData|toDataURL|toBlob|convertToBlob|captureStream|transferToImageBitmap|createImageBitmap)\s*\(/, file);
  }
});

test("releasing a canvas zeroes its buffer", () => {
  const { canvas } = recordingCanvas("ok");
  paintFitCopy(canvas, {} as CanvasImageSource, { width: 10, height: 10 });
  releaseCanvas(canvas);
  assert.equal(canvas.width, 0);
  assert.equal(canvas.height, 0);
});
