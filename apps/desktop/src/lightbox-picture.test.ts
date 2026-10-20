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
  return loadingImage(width, height, opts.complete ?? true).img;
}

/**
 * An <img> whose loading the test finishes by hand, the way the browser would: `complete` and the
 * natural size change as plain DOM properties, and only the `load`/`error` event announces it.
 */
function loadingImage(width: number, height: number, complete = false) {
  const img = document.createElement("img");
  const state = { complete, width: complete ? width : 0, height: complete ? height : 0 };
  Object.defineProperty(img, "complete", { get: () => state.complete });
  Object.defineProperty(img, "naturalWidth", { get: () => state.width });
  Object.defineProperty(img, "naturalHeight", { get: () => state.height });
  // Count what the component leaves attached: the <img> belongs to the chat and outlives the viewer.
  const attached = new Map<string, number>();
  const add = img.addEventListener.bind(img), remove = img.removeEventListener.bind(img);
  img.addEventListener = ((type: string, fn: EventListener) => {
    attached.set(type, (attached.get(type) ?? 0) + 1);
    add(type, fn);
  }) as typeof img.addEventListener;
  img.removeEventListener = ((type: string, fn: EventListener) => {
    attached.set(type, (attached.get(type) ?? 0) - 1);
    remove(type, fn);
  }) as typeof img.removeEventListener;
  return {
    img,
    /** Listeners the component has attached and not removed. */
    listeners: () => [...attached.values()].reduce((sum, n) => sum + n, 0),
    load() { Object.assign(state, { complete: true, width, height }); img.dispatchEvent(new w.Event("load")); flushSync(); },
    fail() { Object.assign(state, { complete: true, width: 0, height: 0 }); img.dispatchEvent(new w.Event("error")); flushSync(); },
  };
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

test("an image opened while still loading gets its copy once it loads, exactly once", async () => {
  // Found in review: `complete` and the natural size are plain DOM properties, so a plan made
  // before the load was never revisited and the opening kept the original's fit path for good.
  contextMode = "ok";
  const pending = loadingImage(4032, 3024);
  const h = harness({ source: pending.img });
  try {
    assert.equal(h.canvas(), null, "the original stands in while loading");
    assert.equal(h.img().hidden, false);
    pending.load();
    const canvas = h.canvas();
    assert.ok(canvas, "the copy is made when the load event arrives");
    assert.equal(canvas.hidden, false);
    assert.equal(h.img().hidden, true);
    assert.equal(calls.draw.length, 1);
    for (let i = 0; i < 5; i++) { h.set({ zoomed: true }); h.set({ zoomed: false }); }
    assert.equal(h.canvas(), canvas, "the same canvas through every toggle");
    assert.equal(calls.draw.length, 1, "and no further draws");
  } finally {
    await h.destroy();
  }
});

test("an image that fails to load keeps the original, and a load after close does nothing", async () => {
  contextMode = "ok";
  const failing = loadingImage(4032, 3024);
  const a = harness({ source: failing.img });
  try {
    failing.fail();
    assert.equal(a.canvas(), null);
    assert.equal(a.img().hidden, false);
    assert.equal(calls.draw.length, 0);
  } finally {
    await a.destroy();
  }
  // Closed (or locked, which closes it) before the image finished loading: the late event must not
  // reach a destroyed viewer.
  const late = loadingImage(4032, 3024);
  const b = harness({ source: late.img });
  assert.ok(late.listeners() > 0, "the viewer listens while the image loads");
  await b.destroy();
  assert.equal(late.listeners(), 0, "and leaves nothing attached to the chat's image when it closes");
  late.load();
  assert.equal(calls.draw.length, 0, "no copy for a viewer that is gone");
  assert.equal(calls.getContext.length, 0);
});

test("the copy belongs to the opening: the file index coming and going does not redraw it", async () => {
  // Found in review: `mime` follows the live index, so an owner unlisting and relisting the file
  // while the viewer was open dropped the copy (back to the original's fit path) and resampled it
  // again on relisting.
  contextMode = "ok";
  for (const zoomed of [false, true]) {
    const h = harness({ zoomed });
    try {
      const canvas = h.canvas();
      assert.ok(canvas);
      for (let i = 0; i < 3; i++) {
        h.set({ mime: "" });
        assert.equal(h.canvas(), canvas, `zoomed=${zoomed}: the copy survives the listing going away`);
        assert.equal(h.img().hidden, !zoomed, `zoomed=${zoomed}: and still stands in for the original in fit view`);
        h.set({ mime: "image/jpeg" });
        assert.equal(h.canvas(), canvas, `zoomed=${zoomed}: and is the same one when it returns`);
      }
      assert.equal(calls.draw.length, 1, `zoomed=${zoomed}: one draw for the whole opening`);
    } finally {
      await h.destroy();
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
