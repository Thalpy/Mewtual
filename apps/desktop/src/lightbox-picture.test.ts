import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";

// --- Environment ---------------------------------------------------------------------------------
// A 1080p screen at scale 1, so the expected copy sizes are easy to check by hand. jsdom has no
// canvas, so the 2D context is a recorder: the claims under test are how often the resample runs,
// on what kind of context, and when; not what pixels come out.

const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, { MutationObserver: w.MutationObserver, Event: w.Event, MouseEvent: w.MouseEvent });
Object.defineProperty(w, "screen", { configurable: true, value: { width: 1920, height: 1080 } });
Object.defineProperty(w, "devicePixelRatio", { configurable: true, value: 1 });

type ContextMode = "ok" | "null" | "draw-throws";
let contextMode: ContextMode = "ok";
const calls = { getContext: [] as unknown[], draw: [] as unknown[][] };
Object.defineProperty(w.HTMLCanvasElement.prototype, "getContext", {
  configurable: true,
  value(this: HTMLCanvasElement, _kind: string, options: unknown) {
    calls.getContext.push(options);
    if (contextMode === "null") return null;
    return {
      imageSmoothingEnabled: false,
      imageSmoothingQuality: "low",
      drawImage: (...args: unknown[]) => {
        calls.draw.push(args);
        if (contextMode === "draw-throws") throw new Error("InvalidStateError");
      },
    };
  },
});

const { mount, unmount, flushSync } = await import("svelte");
const { default: Harness } = await import("../scripts/fixtures/LightboxPictureHarness.svelte");
type HarnessProps = Parameters<InstanceType<typeof Harness>["set"]>[0];

/**
 * An <img> as the chat leaves it. The natural size is independent of `complete`, because a
 * progressive image reports its size before it has finished loading.
 */
function decodedImage(width: number, height: number, opts: { complete?: boolean } = {}): HTMLImageElement {
  const img = document.createElement("img");
  Object.defineProperty(img, "complete", { value: opts.complete ?? true });
  Object.defineProperty(img, "naturalWidth", { value: width });
  Object.defineProperty(img, "naturalHeight", { value: height });
  return img;
}

/** Synchronous on purpose: whatever the component does at mount has happened when this returns. */
function harness(overrides: HarnessProps = {}) {
  calls.getContext.length = 0;
  calls.draw.length = 0;
  const target = document.createElement("div");
  document.body.append(target);
  let toggles = 0;
  const app = mount(Harness, {
    target,
    props: {
      url: "http://catcoms-media.localhost/1/abc", alt: "a cat", source: decodedImage(4032, 3024),
      mime: "image/jpeg", zoomed: false, ontoggle: () => { toggles++; }, ...overrides,
    },
  });
  flushSync();
  return {
    target,
    canvas: () => target.querySelector<HTMLCanvasElement>("canvas.lightbox-fit"),
    img: () => target.querySelector<HTMLImageElement>("img")!,
    toggles: () => toggles,
    set: (next: HarnessProps) => { app.set(next); flushSync(); },
    destroy: async () => { await unmount(app); target.remove(); },
  };
}

// --- Behaviour -----------------------------------------------------------------------------------

test("a large photo is shown fit to the window as a copy resampled once on a CPU-backed context", async () => {
  contextMode = "ok";
  const h = await harness();
  try {
    const canvas = h.canvas();
    assert.ok(canvas, "the fit view is the copy");
    assert.equal(canvas.hidden, false);
    assert.equal(h.img().hidden, true, "the original is not drawn in the fit view");
    assert.equal(canvas.width, 1440);
    assert.equal(canvas.height, 1080);
    assert.deepEqual(calls.getContext, [{ willReadFrequently: true }]);
    assert.equal(calls.draw.length, 1);
    assert.deepEqual(calls.draw[0].slice(1), [0, 0, 1440, 1080]);
    // A canvas has no alt: the button carries the image's name in both modes instead.
    assert.equal(h.target.querySelector("button.lightbox-img")!.getAttribute("aria-label"), "a cat");
    assert.equal(canvas.getAttribute("aria-hidden"), "true");
  } finally {
    await h.destroy();
  }
});

test("switching between fit and actual size never resamples again or reloads the original", async () => {
  contextMode = "ok";
  const h = await harness();
  try {
    const canvas = h.canvas();
    const img = h.img();
    for (let i = 0; i < 5; i++) {
      h.set({ zoomed: true });
      assert.equal(h.canvas(), canvas, "the copy stays mounted while zoomed");
      assert.equal(canvas?.hidden, true);
      assert.equal(h.img(), img, "the original is the same element, not a reload");
      assert.equal(img.hidden, false, "actual size shows the original");
      h.set({ zoomed: false });
      assert.equal(canvas?.hidden, false);
      assert.equal(img.hidden, true);
    }
    assert.equal(calls.draw.length, 1, "one resample for the whole opening, however often it is toggled");
    assert.equal(calls.getContext.length, 1);
  } finally {
    await h.destroy();
  }
});

test("the copy has its pixels in the same flush that shows it", async () => {
  // Painting at mount, synchronously, means the fit view is never an empty box: an earlier version
  // awaited img.decode() first and left the view blank, with nothing to click, while it waited.
  contextMode = "ok";
  const h = harness(); // not awaited: no microtask may run between mount and these checks
  try {
    assert.equal(calls.draw.length, 1, "painted during mount, with no await in between");
    assert.equal(h.canvas()?.hidden, false);
  } finally {
    await h.destroy();
  }
});

test("clicking the picture asks the host to toggle", async () => {
  const h = await harness();
  try {
    h.target.querySelector<HTMLButtonElement>("button.lightbox-img")!.click();
    flushSync();
    assert.equal(h.toggles(), 1);
    assert.equal(h.target.querySelector("button")!.title, "Click to view at actual size");
    h.set({ zoomed: true });
    assert.equal(h.target.querySelector("button")!.title, "Click to fit to the window");
  } finally {
    await h.destroy();
  }
});

test("images that need no copy, or must not get one, show the original in both modes", async () => {
  contextMode = "ok";
  const cases: Array<[string, HarnessProps]> = [
    ["an image that already fits", { source: decodedImage(800, 600) }],
    ["a GIF, which may be animated", { mime: "image/gif", source: decodedImage(6000, 4000) }],
    ["a file the index does not list", { mime: "" }],
    // A progressive image reports its size before it has finished loading; drawing it then would
    // paint a partial or blank copy over a hidden original.
    ["an image that is still loading", { source: decodedImage(4032, 3024, { complete: false }) }],
    ["a broken image", { source: decodedImage(0, 0) }],
    ["an opener with no element", { source: null }],
  ];
  for (const [what, props] of cases) {
    const h = await harness(props);
    try {
      assert.equal(h.canvas(), null, `${what}: no copy`);
      assert.equal(h.img().hidden, false, `${what}: original shown fit to window`);
      h.set({ zoomed: true });
      assert.equal(h.img().hidden, false, `${what}: original shown at actual size`);
      assert.equal(calls.draw.length, 0, `${what}: nothing resampled`);
    } finally {
      await h.destroy();
    }
  }
});

test("a file index that arrives after opening upgrades the fit view to a copy", async () => {
  contextMode = "ok";
  const h = await harness({ mime: "" });
  try {
    assert.equal(h.canvas(), null);
    h.set({ mime: "image/jpeg" });
    assert.ok(h.canvas(), "the copy appears once the format is known");
    assert.equal(h.canvas()?.hidden, false);
    assert.equal(h.img().hidden, true);
    assert.equal(calls.draw.length, 1);
  } finally {
    await h.destroy();
  }
});

test("a browser that will not give a context, or fails the draw, falls back to the original", async () => {
  for (const mode of ["null", "draw-throws"] as const) {
    contextMode = mode;
    const h = await harness();
    try {
      assert.equal(h.canvas(), null, `${mode}: the failed copy is removed`);
      assert.equal(h.img().hidden, false, `${mode}: the original fills the fit view`);
      h.set({ zoomed: true });
      h.set({ zoomed: false });
      assert.equal(h.img().hidden, false, `${mode}: and keeps it after toggling`);
      assert.equal(calls.getContext.length, 1, `${mode}: no retry on every toggle`);
    } finally {
      await h.destroy();
      contextMode = "ok";
    }
  }
});

test("closing the viewer frees the copy's pixel buffer", async () => {
  contextMode = "ok";
  const h = await harness();
  const canvas = h.canvas()!;
  assert.equal(canvas.width, 1440);
  await h.destroy();
  assert.equal(canvas.width, 0);
  assert.equal(canvas.height, 0);
});

// --- Wiring (source shape) -----------------------------------------------------------------------
// App.svelte is too large to mount here, so these pin the host-side facts the behaviour above
// relies on: every opening gets a fresh instance, the decoded element reaches it, and the lock
// takes the viewer (and the element it holds) down.

const app = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const css = readFileSync(new URL("./app.css", import.meta.url), "utf8");

/** The text of a top-level function in App.svelte's script, up to its closing brace. */
function appFunction(name: string): string {
  const start = app.indexOf(`\n  function ${name}(`);
  assert.ok(start >= 0, `App.svelte defines ${name}`);
  return app.slice(start, app.indexOf("\n  }\n", start));
}

test("the host keys the picture on the opening and hands it the decoded element", () => {
  assert.match(app, /\{#key lightbox\}\s*<LightboxPicture[\s\S]*?source=\{lightbox\.source\}[\s\S]*?\/>\s*\{\/key\}/);
  assert.match(appFunction("openLightbox"), /source: el,/, "openLightbox stores the element it was opened from");
  // The old in-place <img> must be gone, or the full original would still be drawn in fit view.
  assert.doesNotMatch(app, /<img src=\{lightbox\.url\}/);
});

test("locking the vault closes the viewer, so it cannot hold the decoded image behind the lock", () => {
  // Found in review: lockScreen cleared every other overlay but not this one, and after unlock it
  // popped back up and repainted from the element it had held through the lock.
  assert.match(appFunction("lockScreen"), /\bcloseLightbox\(\);/);
  const close = appFunction("closeLightbox");
  assert.match(close, /lightbox = null;/);
  assert.match(close, /lightboxZoom = false;/);
});

test("actual size carries no rounded clip or shadow, and the copy sizes like the original", () => {
  const zoomed = css.slice(css.indexOf(".lightbox-stage.zoomed img {"));
  const block = zoomed.slice(0, zoomed.indexOf("}"));
  assert.match(block, /border-radius: 0;/);
  assert.match(block, /box-shadow: none;/);
  assert.match(css, /\.lightbox-stage img,\s*\.lightbox-stage \.lightbox-fit \{\s*max-width: 100%;\s*max-height: 100%;\s*object-fit: contain;/);
});
