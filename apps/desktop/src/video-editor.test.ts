/**
 * Behavioural tests for `VideoEditor.svelte`, mounted in jsdom with fake probe, exporter and
 * filmstrip seams. jsdom has no media pipeline, so the <video> element gets a minimal stand-in:
 * a writable `currentTime` and inert play/pause/load. What these tests pin is the editor's own
 * contract with the host (exactly one outcome, the right outcome, nothing after the end) and
 * that its controls drive the `video-edit.ts` model the way the labels say.
 */
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { deferred } from "./studio-testkit.ts";
import { OutputTooLargeError, ExportCanceledError, type VideoEditorResult, type VideoProbe } from "./video-export.ts";
import type { VideoEdit } from "./video-edit.ts";
import type { ClipDoc } from "./clip-doc.ts";
import type { ClipMedia } from "./clip-export.ts";
import type { ClipMusicSource } from "./clip-music.ts";

const w = (globalThis as unknown as { window: Window & typeof globalThis }).window;
Object.assign(globalThis, {
  MutationObserver: w.MutationObserver, Event: w.Event, KeyboardEvent: w.KeyboardEvent, MouseEvent: w.MouseEvent,
  HTMLMediaElement: w.HTMLMediaElement,
  requestAnimationFrame: (fn: FrameRequestCallback) => setTimeout(() => fn(0), 0) as unknown as number,
  cancelAnimationFrame: (id: number) => clearTimeout(id),
});
// A <video> that remembers where it was sent and never plays: enough for seeking and the playhead.
const times = new WeakMap<object, number>();
Object.defineProperty(w.HTMLMediaElement.prototype, "currentTime", {
  configurable: true,
  get() { return times.get(this) ?? 0; },
  set(value: number) { times.set(this, value); },
});
Object.defineProperty(w.HTMLMediaElement.prototype, "play", { configurable: true, value: () => Promise.resolve() });
Object.defineProperty(w.HTMLMediaElement.prototype, "pause", { configurable: true, value: () => {} });
Object.defineProperty(w.HTMLMediaElement.prototype, "load", { configurable: true, value: () => {} });
Object.defineProperty(w.HTMLCanvasElement.prototype, "getContext", { configurable: true, value: () => ({ drawImage() {}, clearRect() {} }) });

const { mount, unmount, flushSync } = await import("svelte");
const { default: VideoEditor } = await import("./VideoEditor.svelte");

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
async function settle() { for (let i = 0; i < 3; i++) { await sleep(0); flushSync(); } }

const READY: VideoProbe = { ok: true, duration: 10, width: 1920, height: 1080, hasAudio: true };
const TRACK_WIDTH = 1000;

type Options = {
  context?: "chat" | "status" | "wiki" | "files";
  passKey?: (e: KeyboardEvent) => boolean;
  probe?: (file: Blob) => Promise<VideoProbe>;
  exporter?: (file: File, edit: VideoEdit, options: { capBytes: number; signal?: AbortSignal; onProgress?: (f: number) => void }) => Promise<File>;
  composer?: (doc: ClipDoc, media: ClipMedia, options: { capBytes: number; signal?: AbortSignal; container: string; name: string }) => Promise<File>;
  music?: ClipMusicSource;
  capBytes?: number;
  size?: number;
};

async function harness(options: Options = {}) {
  const target = document.createElement("div");
  document.body.append(target);
  const results: VideoEditorResult[] = [];
  const exports: Array<{ edit: VideoEdit; capBytes: number; signal?: AbortSignal }> = [];
  const compositions: Array<{ doc: ClipDoc; media: ClipMedia; container: string; name: string }> = [];
  const stripCalls: number[][] = [];
  const file = new File([new Uint8Array(options.size ?? 2048)], "holiday.mov", { type: "video/quicktime" });
  const app = mount(VideoEditor, {
    target,
    props: {
      file,
      capBytes: options.capBytes ?? 1 << 20,
      position: { index: 1, count: 2 },
      onfinish: (result: VideoEditorResult) => { results.push(result); },
      ...(options.passKey ? { passKey: options.passKey } : {}),
      // Files by default: a plain upload, so the generic tests read results without `post`. The
      // posting contexts have their own tests.
      context: options.context ?? "files",
      probe: options.probe ?? (async () => READY),
      exporter: async (f: File, edit: VideoEdit, o: { capBytes: number; signal?: AbortSignal; onProgress?: (n: number) => void }) => {
        exports.push({ edit, capBytes: o.capBytes, signal: o.signal });
        if (options.exporter) return options.exporter(f, edit, o);
        return new File([new Uint8Array(8)], "holiday.mp4", { type: "video/mp4" });
      },
      thumbnails: async (_file: Blob, at: readonly number[]) => { stripCalls.push([...at]); },
      composer: async (doc: ClipDoc, media: ClipMedia, o: { capBytes: number; signal?: AbortSignal; container: string; name: string }) => {
        compositions.push({ doc, media, container: o.container, name: o.name });
        if (options.composer) return options.composer(doc, media, o);
        return new File([new Uint8Array(8)], "holiday.mp4", { type: "video/mp4" });
      },
      ...(options.music ? { music: options.music } : {}),
    },
  });
  await settle();
  const card = () => target.querySelector<HTMLElement>('[role="dialog"]')!;
  const track = () => target.querySelector<HTMLElement>(".track")!;
  // jsdom lays nothing out; give the track a real width so pointer positions mean times.
  const layout = () => {
    const el = track();
    if (el) el.getBoundingClientRect = () => ({ left: 0, top: 0, width: TRACK_WIDTH, height: 60, right: TRACK_WIDTH, bottom: 60, x: 0, y: 0, toJSON() {} }) as DOMRect;
  };
  layout();
  const action = (name: string) => target.querySelector<HTMLButtonElement>(`[data-action="${name}"]`);
  return {
    target, results, exports, compositions, stripCalls, file,
    state: () => card().dataset.state,
    /** Every bar on the timeline: "overlay:o2", "audio:a3"... */
    bars: () => [...target.querySelectorAll<HTMLElement>("[data-bar]")].map((b) => b.dataset.bar!),
    bar: (id: string) => target.querySelector<HTMLElement>(`[data-bar="${id}"]`),
    inspector: () => target.querySelector<HTMLElement>("[data-inspector]")?.dataset.inspector ?? "",
    field: <T extends HTMLElement = HTMLInputElement>(name: string) => target.querySelector<T>(`[data-field="${name}"]`),
    /** Type into a field the way a person does: value, then an input event. Keys go through the editor. */
    type: (el: HTMLInputElement, text: string) => {
      for (const ch of text) {
        el.dispatchEvent(new w.KeyboardEvent("keydown", { key: ch, bubbles: true, cancelable: true }));
        el.value += ch;
        el.dispatchEvent(new w.Event("input", { bubbles: true }));
      }
      flushSync();
    },
    action,
    click: (name: string) => { const b = action(name); assert.ok(b, `no ${name} button`); b.click(); flushSync(); },
    key: (key: string, init: KeyboardEventInit = {}) => {
      card().dispatchEvent(new w.KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init }));
      flushSync();
    },
    /** A pointer event at track x. Moves carry the primary button as held unless `buttons` says otherwise. */
    pointer: (type: string, x: number, on: Element | null = null, buttons = type === "pointerup" ? 0 : 1) => {
      (on ?? track()).dispatchEvent(new w.MouseEvent(type, { clientX: x, button: 0, buttons, bubbles: true, cancelable: true }));
      flushSync();
    },
    pieces: () => [...target.querySelectorAll<HTMLElement>("[data-piece]")].map((p) => p.dataset.removed === "true"),
    cuts: () => target.querySelectorAll("[data-cut]").length,
    chip: () => target.querySelector(".foot .ve-chip .micro")?.textContent ?? "",
    layout,
    destroy: async () => { await unmount(app); target.remove(); },
  };
}

test("an unreadable video offers the original or nothing, and the original obeys the limit", async () => {
  const h = await harness({ probe: async () => ({ ok: false, reason: "This video's format (hevc) can't be edited here." }) });
  try {
    assert.equal(h.state(), "unavailable");
    assert.match(h.target.querySelector('[role="alert"]')!.textContent!, /hevc/);
    assert.equal(h.action("upload"), null, "no edit means no edited upload");
    h.click("upload-original");
    assert.deepEqual(h.results, [{ kind: "original" }]);
  } finally { await h.destroy(); }

  const big = await harness({ probe: async () => ({ ok: false, reason: "nope" }), capBytes: 100, size: 2048 });
  try {
    assert.equal(big.action("upload-original")!.disabled, true, "an over-limit original cannot be sent");
    big.click("close");
    assert.deepEqual(big.results, [{ kind: "skip" }]);
  } finally { await big.destroy(); }
});

test("a probe that throws is treated as unreadable, not as a crash", async () => {
  const h = await harness({ probe: async () => { throw new Error("boom"); } });
  try {
    assert.equal(h.state(), "unavailable");
  } finally { await h.destroy(); }
});

test("uploading an untouched video sends the original without exporting, exactly once", async () => {
  const h = await harness();
  try {
    assert.equal(h.state(), "ready");
    assert.equal(h.chip(), "unedited");
    h.click("upload");
    h.click("upload");
    h.key("Escape");
    assert.deepEqual(h.results, [{ kind: "original" }], "one outcome, whatever follows");
    assert.equal(h.exports.length, 0);
  } finally { await h.destroy(); }
});

test("the filmstrip asks for evenly spaced, ascending tiles across the clip", async () => {
  const h = await harness();
  try {
    assert.equal(h.stripCalls.length, 1);
    const at = h.stripCalls[0];
    assert.ok(at.length >= 1 && at.length <= 48);
    for (let i = 1; i < at.length; i++) assert.ok(at[i] > at[i - 1]);
    assert.ok(at[0] > 0 && at[at.length - 1] < 10);
  } finally { await h.destroy(); }
});

test("split at the playhead, remove the piece under it, and export only the kept part", async () => {
  const h = await harness();
  try {
    h.key("s");
    assert.equal(h.cuts(), 0, "no cut at the very start");
    for (let i = 0; i < 3; i++) h.key("ArrowRight", { shiftKey: true });
    h.key("s");
    assert.equal(h.cuts(), 1);
    assert.deepEqual(h.pieces(), [false, false]);
    h.key("Delete");
    assert.deepEqual(h.pieces(), [false, true], "the piece under the playhead was removed");
    // No "·"-chained fragments in the chip: how many pieces are out is its tooltip.
    assert.equal(h.chip(), "keeps 0:03.0 of 0:10.0");
    assert.equal(h.target.querySelector<HTMLElement>(".foot .ve-chip")!.title, "1 piece removed");
    h.click("upload");
    await settle();
    assert.equal(h.exports.length, 1);
    assert.deepEqual(h.exports[0].edit.removed, [false, true]);
    assert.equal(h.exports[0].capBytes, 1 << 20);
    assert.equal(h.results.length, 1);
    const [result] = h.results;
    assert.equal(result.kind, "edited");
    assert.equal(result.kind === "edited" && result.file.name, "holiday.mp4");
  } finally { await h.destroy(); }
});

test("undo and redo step through edits, and a refused edit leaves undo alone", async () => {
  const h = await harness();
  try {
    assert.equal(h.action("undo")!.disabled, true);
    h.key("s"); // refused at 0
    assert.equal(h.action("undo")!.disabled, true, "a refused split is not an undo step");
    h.key("ArrowRight", { shiftKey: true });
    h.key("s");
    h.key("Delete");
    assert.deepEqual(h.pieces(), [false, true]);
    h.key("z", { ctrlKey: true });
    assert.deepEqual(h.pieces(), [false, false]);
    h.key("z", { ctrlKey: true });
    assert.equal(h.cuts(), 0);
    h.key("Z", { ctrlKey: true, shiftKey: true });
    assert.equal(h.cuts(), 1);
    h.key("y", { ctrlKey: true });
    assert.deepEqual(h.pieces(), [false, true]);
  } finally { await h.destroy(); }
});

test("the blade cuts where the timeline is clicked, and Escape leaves blade mode before closing", async () => {
  const h = await harness();
  try {
    h.key("b");
    assert.equal(h.action("blade")!.getAttribute("aria-pressed"), "true");
    h.pointer("pointerdown", 500);
    h.pointer("pointerup", 500);
    assert.equal(h.cuts(), 1);
    assert.equal(h.target.querySelector<HTMLElement>("[data-cut]")!.style.left, "50%");
    h.key("Escape");
    assert.equal(h.action("blade")!.getAttribute("aria-pressed"), "false");
    assert.equal(h.results.length, 0, "the first Escape only left blade mode");
    h.key("Escape");
    assert.ok(h.action("discard"), "the second asks before throwing the cut away");
    assert.equal(h.results.length, 0);
    h.click("discard");
    assert.deepEqual(h.results, [{ kind: "skip" }]);
  } finally { await h.destroy(); }
});

test("dragging a trim handle is a single undo step and trims the upload", async () => {
  const h = await harness();
  try {
    const handle = h.target.querySelector('[data-handle="in"]')!;
    h.pointer("pointerdown", 0, handle);
    for (const x of [100, 150, 200]) h.pointer("pointermove", x);
    h.pointer("pointerup", 200);
    assert.equal(h.target.querySelector<HTMLElement>('[data-handle="in"]')!.style.left, "20%");
    assert.match(h.chip(), /keeps 0:08\.0 of 0:10\.0/);
    h.key("z", { ctrlKey: true });
    assert.equal(h.target.querySelector<HTMLElement>('[data-handle="in"]')!.style.left, "0%");
    assert.equal(h.action("undo")!.disabled, true, "the whole drag was one step");
  } finally { await h.destroy(); }
});

test("scrubbing selects the piece under the pointer and Delete acts on the selection", async () => {
  const h = await harness();
  try {
    h.key("b");
    h.pointer("pointerdown", 400); h.pointer("pointerup", 400);
    h.pointer("pointerdown", 700); h.pointer("pointerup", 700);
    h.key("b");
    // Click the last piece, then move the playhead into the first: Delete follows the selection.
    h.pointer("pointerdown", 900); h.pointer("pointerup", 900);
    h.key("Home");
    h.key("Delete");
    assert.deepEqual(h.pieces(), [false, false, true]);
  } finally { await h.destroy(); }
});

test("an over-limit original can still be sent once it is trimmed", async () => {
  const h = await harness({ capBytes: 100, size: 2048 });
  try {
    assert.equal(h.action("upload")!.disabled, true);
    assert.match(h.action("upload")!.title, /Over this server's/);
    h.key("ArrowRight", { shiftKey: true });
    h.key("o");
    assert.equal(h.action("upload")!.disabled, false, "an edit is re-encoded, so the original's size no longer decides");
  } finally { await h.destroy(); }
});

test("removing everything disables upload", async () => {
  const h = await harness();
  try {
    h.key("Delete");
    assert.deepEqual(h.pieces(), [true]);
    assert.equal(h.chip(), "nothing kept");
    assert.equal(h.action("upload")!.disabled, true);
  } finally { await h.destroy(); }
});

test("cancelling an export returns to editing and a late result is ignored", async () => {
  const pending = deferred<File>();
  const h = await harness({ exporter: () => pending.promise });
  try {
    // Trim the end to the playhead (held 0.1 s after the start), so upload has to export.
    h.key("o");
    h.click("upload");
    assert.equal(h.state(), "exporting");
    assert.equal(h.action("undo")!.disabled, true, "editing is locked while exporting");
    h.click("cancel-export");
    assert.equal(h.state(), "ready");
    assert.equal(h.exports[0].signal?.aborted, true);
    pending.resolve(new File([new Uint8Array(4)], "late.mp4", { type: "video/mp4" }));
    await settle();
    assert.equal(h.results.length, 0, "a cancelled export never becomes the upload");
  } finally { await h.destroy(); }
});

test("Escape during an export cancels the export instead of closing", async () => {
  const pending = deferred<File>();
  const h = await harness({ exporter: (_f, _e, o) => { o.signal?.addEventListener("abort", () => pending.reject(new ExportCanceledError())); return pending.promise; } });
  try {
    h.key("o");
    h.click("upload");
    h.key("Escape");
    await settle();
    assert.equal(h.state(), "ready");
    assert.equal(h.results.length, 0);
  } finally { await h.destroy(); }
});

test("an export over the size cap explains itself and lets the person go back and cut more", async () => {
  const h = await harness({ exporter: async () => { throw new OutputTooLargeError(1 << 20); } });
  try {
    h.key("o");
    h.click("upload");
    await settle();
    assert.equal(h.state(), "failed");
    assert.match(h.target.querySelector('[role="alert"]')!.textContent!, /The edited video is over this server's 1\.0 MiB limit/);
    h.click("back-to-edit");
    assert.equal(h.state(), "ready");
    assert.equal(h.results.length, 0);
  } finally { await h.destroy(); }
});

test("any other export failure offers the original as a way out", async () => {
  const h = await harness({ exporter: async () => { throw new Error("This window can't encode the sound of an edited video."); } });
  try {
    h.key("o");
    h.click("upload");
    await settle();
    assert.equal(h.state(), "failed");
    assert.match(h.target.querySelector('[role="alert"]')!.textContent!, /can't encode the sound/);
    h.click("upload-original");
    assert.deepEqual(h.results, [{ kind: "original" }]);
  } finally { await h.destroy(); }
});

test("unmounted mid-export, it aborts the export and answers skip once, never the late file", async () => {
  const pending = deferred<File>();
  const h = await harness({ exporter: () => pending.promise });
  h.key("o");
  h.click("upload");
  const signal = h.exports[0].signal;
  await h.destroy();
  assert.equal(signal?.aborted, true);
  pending.resolve(new File([new Uint8Array(4)], "late.mp4"));
  await settle();
  assert.deepEqual(h.results, [{ kind: "skip" }], "an editor torn down without an answer answers skip");
});

test("an editor that already answered says nothing more when it is unmounted", async () => {
  const h = await harness();
  h.click("close");
  await h.destroy();
  await settle();
  assert.deepEqual(h.results, [{ kind: "skip" }]);
});

test("undo in the middle of a drag ends the drag instead of fighting it", async () => {
  const h = await harness();
  const handle = h.target.querySelector('[data-handle="in"]')!;
  try {
    h.pointer("pointerdown", 0, handle);
    h.pointer("pointermove", 100);
    h.pointer("pointermove", 200);
    h.key("z", { ctrlKey: true });
    assert.equal(h.target.querySelector<HTMLElement>('[data-handle="in"]')!.style.left, "0%");
    // The pointer is still down and moving, but the drag it belonged to is over.
    h.pointer("pointermove", 300);
    assert.equal(h.target.querySelector<HTMLElement>('[data-handle="in"]')!.style.left, "0%");
    assert.equal(h.action("undo")!.disabled, true);
    assert.equal(h.action("redo")!.disabled, false, "the undone drag can still be redone");
    h.key("y", { ctrlKey: true });
    assert.equal(h.target.querySelector<HTMLElement>('[data-handle="in"]')!.style.left, "20%");
  } finally { await h.destroy(); }
});

test("a move with no button held ends a drag that lost its release", async () => {
  const h = await harness();
  const handle = h.target.querySelector('[data-handle="out"]')!;
  try {
    h.pointer("pointerdown", 1000, handle);
    h.pointer("pointermove", 800);
    h.pointer("pointermove", 500, null, 0); // focus left mid-drag; the release never arrived
    h.pointer("pointermove", 300);
    assert.equal(h.target.querySelector<HTMLElement>('[data-handle="out"]')!.style.left, "80%");
  } finally { await h.destroy(); }
});

test("keys the host passes through reach the app and the editor does not act on them", async () => {
  const h = await harness({ passKey: (e) => e.code === "KeyV" || ((e.ctrlKey || e.metaKey) && e.key === "l") });
  let reached = 0;
  const listener = () => { reached++; };
  w.addEventListener("keydown", listener);
  try {
    h.key("l", { ctrlKey: true });
    h.key("v", { code: "KeyV" });
    assert.equal(reached, 2);
    h.key("s", { code: "KeyS" });
    assert.equal(reached, 2, "an editor key still stays inside");
  } finally {
    w.removeEventListener("keydown", listener);
    await h.destroy();
  }
});

test("pressing the backdrop or focusing something behind keeps focus in the dialog", async () => {
  const h = await harness();
  const outside = document.createElement("button");
  document.body.append(outside);
  try {
    const overlay = h.target.querySelector<HTMLElement>("[data-video-editor]")!;
    const down = new w.MouseEvent("mousedown", { bubbles: true, cancelable: true });
    overlay.dispatchEvent(down);
    assert.equal(down.defaultPrevented, true, "a backdrop press does not move focus to the page");
    outside.focus();
    flushSync();
    assert.equal(document.activeElement, h.target.querySelector('[role="dialog"]'));
  } finally {
    outside.remove();
    await h.destroy();
  }
});

test("focus that drops to the page when a focused control goes away comes back to the dialog", async () => {
  const h = await harness();
  try {
    // Chromium sends focus to <body> with only a focusout when the focused button is disabled
    // (Split once the playhead sits on a cut). jsdom does the same on blur().
    const split = h.action("split")!;
    h.key("ArrowRight", { shiftKey: true });
    split.focus();
    split.blur();
    assert.equal(document.activeElement, document.body);
    await settle();
    assert.equal(document.activeElement, h.target.querySelector('[role="dialog"]'), "focus returned");
    h.key("s");
    assert.equal(h.cuts(), 1, "the editor's keys work again");
  } finally { await h.destroy(); }
});

test("the editor suppresses the app's context menus over it", async () => {
  const h = await harness();
  try {
    const menu = new w.MouseEvent("contextmenu", { bubbles: true, cancelable: true });
    h.target.querySelector("h2")!.dispatchEvent(menu);
    assert.equal(menu.defaultPrevented, true);
  } finally { await h.destroy(); }
});

test("a plain passed key cannot also press the focused editor button", async () => {
  const h = await harness({ passKey: (e) => e.code === "Space" || ((e.ctrlKey || e.metaKey) && e.key === "l") });
  try {
    const space = new w.KeyboardEvent("keydown", { key: " ", code: "Space", bubbles: true, cancelable: true });
    h.action("remove")!.dispatchEvent(space);
    assert.equal(space.defaultPrevented, true, "Space as push-to-talk does not click Remove");
    const lock = new w.KeyboardEvent("keydown", { key: "l", code: "KeyL", ctrlKey: true, bubbles: true, cancelable: true });
    h.target.querySelector('[role="dialog"]')!.dispatchEvent(lock);
    assert.equal(lock.defaultPrevented, false, "a modified passed key is left for the app to handle");
  } finally { await h.destroy(); }
});

test("focus returns to where it started even when the next queued editor opened first", async () => {
  const composer = document.createElement("textarea");
  document.body.append(composer);
  composer.focus();
  const first = await harness();
  // The host's keyed each mounts the next editor while the first is still in the page, with
  // focus inside it.
  first.action("upload")!.focus();
  const second = await harness();
  // A bounce (outgoing editor focuses the composer, the new one's guard pulls focus back) would
  // end in the same place, so watch the composer itself: it must not be focused in between.
  let bounced = 0;
  const onFocus = () => { bounced++; };
  composer.addEventListener("focus", onFocus);
  await first.destroy();
  composer.removeEventListener("focus", onFocus);
  assert.equal(bounced, 0, "the outgoing editor did not hand focus away from the new one");
  assert.equal(document.activeElement, second.target.querySelector('[role="dialog"]'));
  await second.destroy();
  assert.equal(document.activeElement, composer, "the session ends where it began");
  composer.remove();
});

// --- The one action, and leaving -------------------------------------------------------------------

test("the footer has exactly one action, worded for where the video is going", async () => {
  for (const [context, word] of [["chat", "post now"], ["status", "post now"], ["wiki", "embed"], ["files", "upload"]] as const) {
    const h = await harness({ context });
    try {
      const buttons = [...h.target.querySelectorAll<HTMLButtonElement>("footer button")];
      assert.deepEqual(buttons.map((b) => b.dataset.action), ["upload"], `${context}: one button, no "don't upload", no "as is"`);
      assert.equal(buttons[0].textContent!.trim(), word);
      assert.ok(buttons[0].classList.contains("primary"));
    } finally { await h.destroy(); }
  }
});

test("post now on an untouched clip sends the original bytes and asks the host to post", async () => {
  const h = await harness({ context: "chat" });
  try {
    h.click("upload");
    assert.deepEqual(h.results, [{ kind: "original", post: true }]);
    assert.equal(h.exports.length, 0, "no re-render without edits");
  } finally { await h.destroy(); }
});

test("post now on an edit exports first, then hands back the re-encode to post", async () => {
  const h = await harness({ context: "chat" });
  try {
    h.key("o");
    h.key("Enter", { ctrlKey: true });
    await settle();
    assert.equal(h.exports.length, 1);
    assert.equal(h.results.length, 1);
    const [result] = h.results;
    assert.equal(result.kind, "edited");
    assert.equal(result.kind === "edited" && result.post, true);
  } finally { await h.destroy(); }
});

test("the wiki and Files upload without posting, from the button or Ctrl+Enter", async () => {
  for (const context of ["wiki", "files"] as const) {
    const h = await harness({ context });
    try {
      h.key("Enter", { ctrlKey: true });
      assert.deepEqual(h.results, [{ kind: "original" }], context);
    } finally { await h.destroy(); }
  }
});

test("closing an untouched clip leaves at once; with edits it asks first", async () => {
  const clean = await harness();
  try {
    clean.click("close");
    assert.deepEqual(clean.results, [{ kind: "skip" }], "nothing to lose, nothing to ask");
  } finally { await clean.destroy(); }

  const h = await harness();
  try {
    h.key("ArrowRight", { shiftKey: true });
    h.key("s"); // a cut alone is work worth asking about
    h.click("close");
    assert.equal(h.results.length, 0);
    assert.ok(h.target.querySelector('[role="alertdialog"]'), "the question is shown");
    await settle();
    assert.equal(document.activeElement, h.action("keep-editing"), "the safe answer has focus");
    h.click("keep-editing");
    assert.equal(h.target.querySelector('[role="alertdialog"]'), null);
    assert.equal(h.cuts(), 1, "the work is still there");
    h.click("close");
    h.click("discard");
    assert.deepEqual(h.results, [{ kind: "skip" }]);
  } finally { await h.destroy(); }
});

test("while the discard question is up, Escape keeps editing and editing keys do nothing", async () => {
  const h = await harness();
  try {
    h.key("ArrowRight", { shiftKey: true });
    h.key("s");
    h.key("Escape");
    assert.ok(h.action("discard"));
    h.key("Delete");
    h.key("s");
    assert.deepEqual(h.pieces(), [false, false], "nothing changed behind the question");
    h.key("Escape");
    assert.equal(h.action("discard"), null, "Escape answers 'keep editing'");
    assert.equal(h.results.length, 0);
  } finally { await h.destroy(); }
});

test("the Keybinds page lists every key the editor handles", async () => {
  const { VIDEO_EDITOR_SHORTCUTS } = await import("./video-editor-ui.ts");
  const listed = VIDEO_EDITOR_SHORTCUTS.map((s) => `${s.keys} ${s.action}`).join("\n");
  const source = readFileSync(new URL("./VideoEditor.svelte", import.meta.url), "utf8");
  const handled = new Set([...source.matchAll(/\bkey === "([^"]+)"/g)].map((m) => m[1]));
  // How each handled key is written on the Keybinds page. Tab (focus trap) and Backspace (a
  // second Delete) are deliberately not listed.
  const shown: Record<string, string> = {
    " ": "Space", Escape: "Esc", ArrowLeft: "←", ArrowRight: "→", Home: "Home", End: "End", Delete: "Delete",
    Enter: "Ctrl+Enter", b: "B", s: "S", i: "I", o: "O", m: "M", z: "Ctrl+Z", y: "Ctrl+Y", r: "R", l: "L",
  };
  const unlisted = new Set(["Tab", "Backspace"]);
  for (const key of handled) {
    if (unlisted.has(key)) continue;
    assert.ok(shown[key], `the editor handles "${key}" but the test has no spelling for it: add one and list it`);
    assert.ok(listed.includes(shown[key]), `"${shown[key]}" is handled by the editor but missing from the Keybinds page`);
  }
});

test("after a cancel, upload waits until the cancelled export has wound down", async () => {
  const first = deferred<File>();
  let calls = 0;
  const h = await harness({ exporter: () => { calls++; return calls === 1 ? first.promise : Promise.resolve(new File([new Uint8Array(1)], "x.mp4")); } });
  try {
    h.key("o");
    h.click("upload");
    h.click("cancel-export");
    assert.equal(h.state(), "ready");
    assert.equal(h.action("upload")!.disabled, true);
    assert.match(h.action("upload")!.title, /Finishing the cancelled export/);
    first.reject(new ExportCanceledError());
    await settle();
    assert.equal(h.action("upload")!.disabled, false);
    h.click("upload");
    await settle();
    assert.equal(calls, 2);
    assert.equal(h.results[0]?.kind, "edited");
  } finally { await h.destroy(); }
});

test("the close button cancels a running export rather than leaving", async () => {
  const pending = deferred<File>();
  const h = await harness({ exporter: (_f, _e, o) => { o.signal?.addEventListener("abort", () => pending.reject(new ExportCanceledError())); return pending.promise; } });
  try {
    h.key("o");
    h.click("upload");
    h.click("close");
    await settle();
    assert.equal(h.state(), "ready");
    assert.equal(h.results.length, 0);
    h.click("close");
    h.click("discard"); // the trim is work to lose, so leaving asks first
    assert.deepEqual(h.results, [{ kind: "skip" }]);
  } finally { await h.destroy(); }
});

test("the modal keeps its keys from the app's window-level shortcuts behind it", async () => {
  const h = await harness();
  let leaked = 0;
  const listener = () => { leaked++; };
  w.addEventListener("keydown", listener);
  try {
    // Sanity: an event dispatched outside the editor does reach the window listener.
    document.body.dispatchEvent(new w.KeyboardEvent("keydown", { key: "x", bubbles: true }));
    assert.equal(leaked, 1);
    h.key("k", { ctrlKey: true });
    h.key("ArrowRight");
    h.key("Escape");
    assert.equal(leaked, 1, "nothing typed in the editor reached the app");
  } finally {
    w.removeEventListener("keydown", listener);
    await h.destroy();
  }
});

// --- The clip maker ----------------------------------------------------------------------------------

test("text added from the library is selected, ready to type over, and makes the clip a composition", async () => {
  const h = await harness({ context: "chat" });
  try {
    h.target.querySelector<HTMLButtonElement>('[data-add-text="wave"]')!.click();
    await settle();
    assert.deepEqual(h.bars(), ["overlay:o2"]);
    assert.equal(h.inspector(), "overlay");
    assert.equal(document.activeElement, h.field("text"), "the new words have focus");
    h.click("upload");
    await settle();
    assert.equal(h.exports.length, 0, "not the cut-only exporter: every frame needs drawing");
    assert.equal(h.compositions.length, 1);
    const [c] = h.compositions;
    assert.deepEqual(c.doc.overlays.map((o) => o.content.kind === "text" && o.content.effect), ["wave"]);
    assert.equal(c.container, "mp4", "a .mov is written as MP4");
    assert.equal(c.name, "holiday.mov");
    assert.ok(c.media.video("base"), "the compositor can read the clip itself");
    assert.equal(h.results.length, 1);
    assert.equal(h.results[0].kind === "edited" && h.results[0].post, true);
  } finally { await h.destroy(); }
});

test("keys typed into the text field are words, not shortcuts, and one typed word is one undo step", async () => {
  const h = await harness();
  try {
    h.target.querySelector<HTMLButtonElement>('[data-add-text="plain"]')!.click();
    await settle();
    const field = h.field("text")!;
    field.value = "";
    h.type(field, "sob!");
    field.dispatchEvent(new w.Event("change", { bubbles: true }));
    flushSync();
    assert.equal(h.cuts(), 0, "typing an s did not split the clip");
    assert.deepEqual(h.pieces(), [false], "nor did anything else typed change the cut");
    assert.equal(h.bar("overlay:o2")!.title, "sob!");
    h.key("z", { ctrlKey: true });
    assert.equal(h.bar("overlay:o2")!.title, "Hello!", "the whole word undoes at once");
    h.key("z", { ctrlKey: true });
    assert.deepEqual(h.bars(), [], "and then the text itself");
  } finally { await h.destroy(); }
});

test("Escape in the text field hands keys back to the editor; Escape again deselects before it would close", async () => {
  const h = await harness();
  try {
    h.target.querySelector<HTMLButtonElement>('[data-add-text="plain"]')!.click();
    await settle();
    h.field("text")!.dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    flushSync();
    assert.equal(document.activeElement, h.target.querySelector('[role="dialog"]'));
    assert.equal(h.inspector(), "overlay", "still selected");
    h.key("Escape");
    assert.equal(h.inspector(), "base", "deselected");
    assert.equal(h.results.length, 0, "nothing closed");
    h.key("Escape");
    assert.ok(h.action("discard"), "only now does leaving ask about the work");
  } finally { await h.destroy(); }
});

test("Delete removes the selected text, not the piece under the playhead", async () => {
  const h = await harness();
  try {
    h.target.querySelector<HTMLButtonElement>('[data-add-text="plain"]')!.click();
    await settle();
    h.key("Escape"); // leave the text field; the text stays selected
    h.pointer("pointerdown", 10, h.bar("overlay:o2"));
    h.pointer("pointerup", 10, h.bar("overlay:o2"));
    assert.equal(h.inspector(), "overlay");
    h.key("Delete");
    assert.deepEqual(h.bars(), []);
    assert.deepEqual(h.pieces(), [false], "the clip itself was not touched");
    h.key("Delete");
    assert.deepEqual(h.pieces(), [true], "with nothing selected, Delete is the piece again");
  } finally { await h.destroy(); }
});

test("dragging a bar moves its text in one undo step, and dragging an end trims it", async () => {
  const h = await harness();
  try {
    h.target.querySelector<HTMLButtonElement>('[data-add-text="plain"]')!.click();
    await settle();
    h.key("Escape");
    const bar = () => h.bar("overlay:o2")!;
    assert.equal(bar().style.left, "0%");
    h.pointer("pointerdown", 50, bar());
    for (const x of [100, 150, 250]) h.pointer("pointermove", x);
    h.pointer("pointerup", 250);
    assert.equal(bar().style.left, "20%", "moved by 2 s");
    assert.equal(bar().style.width, "30%", "and still 3 s long");
    h.key("z", { ctrlKey: true });
    assert.equal(bar().style.left, "0%", "the whole drag undid at once");
    h.pointer("pointerdown", 300, bar().querySelector('[data-edge="end"]'));
    h.pointer("pointermove", 100);
    h.pointer("pointerup", 100);
    assert.equal(bar().style.width, "10%", "the end came in to 1 s");
  } finally { await h.destroy(); }
});

test("record mode needs something selected, and locks cutting and posting until it stops", async () => {
  const h = await harness();
  try {
    assert.equal(h.action("record")!.disabled, true, "nothing to animate yet");
    h.key("r");
    assert.equal(h.target.querySelector(".rec-chip"), null);
    h.target.querySelector<HTMLButtonElement>('[data-add-text="plain"]')!.click();
    await settle();
    // Escape in the text field, as a person would: keys come back to the editor, the text stays selected.
    h.field("text")!.dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    flushSync();
    h.key("r");
    assert.ok(h.target.querySelector(".rec-chip"), "recording");
    assert.equal(h.action("split")!.disabled, true);
    assert.equal(h.action("upload")!.disabled, true);
    assert.equal(h.action("upload")!.title, "Stop recording first");
    h.key("Escape");
    assert.equal(h.target.querySelector(".rec-chip"), null, "Escape stops recording first");
    assert.equal(h.action("upload")!.disabled, false);
    assert.equal(h.results.length, 0);
  } finally { await h.destroy(); }
});

test("muting the clip's own sound re-renders it; muting and unmuting again posts the original", async () => {
  const muted = await harness();
  try {
    muted.click("mute-v1");
    muted.click("upload");
    await settle();
    assert.equal(muted.compositions.length, 1);
    assert.equal(muted.compositions[0].doc.layers[0].muted, true);
  } finally { await muted.destroy(); }

  const back = await harness();
  try {
    back.click("mute-v1");
    back.click("mute-v1");
    back.click("upload");
    assert.deepEqual(back.results, [{ kind: "original" }], "nothing changed after all: no re-render");
    assert.equal(back.compositions.length + back.exports.length, 0);
  } finally { await back.destroy(); }
});

test("a clip with no sound cannot be muted from the library either, so untouched it still posts its original", async () => {
  // Found in review: the V1 lane's mute was disabled for a soundless clip, but the library's layer
  // list still muted it, and the untouched clip then went through a full re-render.
  const h = await harness({ probe: async () => ({ ...READY, hasAudio: false }) });
  try {
    h.target.querySelector<HTMLButtonElement>('[data-tab="video"]')!.click();
    flushSync();
    const mute = h.target.querySelector<HTMLButtonElement>('[data-layer-mute="V1"]')!;
    assert.equal(mute.disabled, true);
    assert.equal(mute.title, "This video has no sound");
    // The handler refuses too, whatever reaches it.
    mute.disabled = false;
    mute.click();
    flushSync();
    assert.equal(h.action("mute-v1")!.getAttribute("aria-pressed"), "false");
    h.click("upload");
    assert.deepEqual(h.results, [{ kind: "original" }]);
    assert.equal(h.compositions.length + h.exports.length, 0, "no re-render");
  } finally { await h.destroy(); }
});

test("the Music tab lists the channel's Studio songs and shows a song's refusal in its own words", async () => {
  let listed = 0;
  const music: ClipMusicSource = {
    list: async () => { listed++; return [{ id: "s1", title: "Night Drive" }]; },
    load: async () => { throw new Error("resolve score conflicts before playback/export"); },
  };
  const h = await harness({ music });
  try {
    assert.equal(listed, 0, "nothing is read until the tab is opened");
    h.target.querySelector<HTMLButtonElement>('[data-tab="music"]')!.click();
    await settle();
    assert.equal(listed, 1);
    const lib = () => h.target.querySelector(".lib")!.textContent ?? "";
    assert.match(lib(), /Night Drive/);
    h.target.querySelector<HTMLButtonElement>('[data-add-song="s1"]')!.click();
    await settle();
    assert.match(lib(), /Resolve score conflicts before playback\/export/);
    assert.deepEqual(h.bars(), [], "a song that cannot play as written is not added as something else");
  } finally { await h.destroy(); }
});

test("songs clicked in a row are read and rendered one at a time, and a closed editor's turn is skipped", async () => {
  // Found in review: every clicked row read and rendered at once, each render allocating its whole
  // output before the bin's budget was asked.
  const loads: string[] = [];
  const gates = new Map<string, () => void>();
  const music: ClipMusicSource = {
    list: async () => ["s1", "s2", "s3"].map((id) => ({ id, title: id })),
    load: (id) => {
      loads.push(id);
      return new Promise((resolve) => { gates.set(id, () => resolve({ take: null, gridMs: 0 })); });
    },
  };
  const h = await harness({ music });
  try {
    h.target.querySelector<HTMLButtonElement>('[data-tab="music"]')!.click();
    await settle();
    for (const id of ["s1", "s2", "s3"]) h.target.querySelector<HTMLButtonElement>(`[data-add-song="${id}"]`)!.click();
    await settle();
    assert.deepEqual(loads, ["s1"], "the second waits for the first");
    gates.get("s1")!();
    await settle();
    assert.deepEqual(loads, ["s1", "s2"], "then takes its turn");
    assert.match(h.target.querySelector(".lib")!.textContent ?? "", /This music could not be read/, "the first one's outcome shows");
    h.click("close");
    await settle();
    assert.deepEqual(h.results, [{ kind: "skip" }]);
    gates.get("s2")!();
    await settle();
    assert.deepEqual(loads, ["s1", "s2"], "the closed editor's last song is never read");
  } finally {
    for (const open of gates.values()) open();
    await h.destroy();
  }
});

test("undo is refused while recording, and an edit mid-recording drops the live drag and its trail", async () => {
  // Found in review: Ctrl+Z (or Delete) during a recording drag left the overlay drawn where the
  // pointer had been, at every time, and the red trail on screen.
  const h = await harness();
  try {
    h.target.querySelector<HTMLButtonElement>('[data-add-text="plain"]')!.click();
    await settle();
    h.field("text")!.dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    flushSync();
    h.key("r");
    const stage = h.target.querySelector<HTMLElement>(".ve-stage")!;
    stage.dispatchEvent(new w.MouseEvent("pointerdown", { button: 0, buttons: 1, bubbles: true, cancelable: true }));
    flushSync();
    assert.ok(h.target.querySelector(".trail"), "recording: the trail shows");
    h.key("z", { ctrlKey: true });
    assert.deepEqual(h.bars(), ["overlay:o2"], "Ctrl+Z did nothing while recording");
    assert.ok(h.target.querySelector(".trail"), "and the recording carries on");
    h.key("Delete");
    assert.deepEqual(h.bars(), []);
    assert.equal(h.target.querySelector(".trail"), null, "the drag and its trail went with the edit");
    w.dispatchEvent(new w.MouseEvent("pointerup", { bubbles: true }));
    flushSync();
    assert.equal(h.results.length, 0);
  } finally { await h.destroy(); }
});

test("a release anywhere ends a stage drag, even one the stage never hears", async () => {
  const h = await harness();
  try {
    h.target.querySelector<HTMLButtonElement>('[data-add-text="plain"]')!.click();
    await settle();
    h.field("text")!.dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    flushSync();
    h.key("r");
    h.target.querySelector(".ve-stage")!.dispatchEvent(new w.MouseEvent("pointerdown", { button: 0, buttons: 1, bubbles: true, cancelable: true }));
    flushSync();
    assert.ok(h.target.querySelector(".trail"));
    // Capture was lost (Chromium does this on its own sometimes), so the release lands elsewhere.
    document.body.dispatchEvent(new w.MouseEvent("pointerup", { bubbles: true }));
    flushSync();
    assert.equal(h.target.querySelector(".trail"), null, "the drag ended");
  } finally { await h.destroy(); }
});

test("while the discard question is up, Tab stays in it and nothing behind it can post", async () => {
  // Found in review: Shift+Tab from "keep editing" reached the post button behind the question,
  // and Enter there started the export while the question was still showing.
  const h = await harness({ context: "chat" });
  try {
    h.key("o"); // an edit, so closing asks
    h.click("close");
    await settle();
    const keep = h.action("keep-editing")!, discard = h.action("discard")!;
    assert.equal(document.activeElement, keep);
    keep.dispatchEvent(new w.KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true, cancelable: true }));
    assert.equal(document.activeElement, discard, "Shift+Tab wraps within the question");
    discard.dispatchEvent(new w.KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true }));
    assert.equal(document.activeElement, keep, "and so does Tab");
    for (const section of ["header", ".middle", ".transport", ".timeline", "footer"]) {
      assert.equal(h.target.querySelector<HTMLElement>(section)!.inert, true, `${section} is inert behind the question`);
    }
    h.action("upload")!.click();
    flushSync();
    await settle();
    assert.equal(h.exports.length + h.compositions.length, 0, "no export started");
    assert.equal(h.results.length, 0);
    h.click("keep-editing");
    assert.equal(h.target.querySelector<HTMLElement>("footer")!.inert, false, "the editor is live again");
  } finally { await h.destroy(); }
});

test("a passed push-to-talk key still types into the text field", async () => {
  // Found in review: with push-to-talk on Space (or V), the editor prevented the key everywhere,
  // so a text overlay could not hold a space (or a v) during a call.
  const h = await harness({ passKey: (e) => e.code === "Space" || e.code === "KeyV" });
  try {
    h.target.querySelector<HTMLButtonElement>('[data-add-text="plain"]')!.click();
    await settle();
    for (const [key, code] of [[" ", "Space"], ["v", "KeyV"]]) {
      const typed = new w.KeyboardEvent("keydown", { key, code, bubbles: true, cancelable: true });
      h.field("text")!.dispatchEvent(typed);
      assert.equal(typed.defaultPrevented, false, `${code} types in the field`);
    }
    const onButton = new w.KeyboardEvent("keydown", { key: " ", code: "Space", bubbles: true, cancelable: true });
    h.action("remove")!.dispatchEvent(onButton);
    assert.equal(onButton.defaultPrevented, true, "but still cannot press a focused button");
  } finally { await h.destroy(); }
});

test("a song list that cannot be read is asked once, then waits for the person to try again", async () => {
  // Found in review: a failing list reset the tab to "not asked", and the tab asked again at once,
  // forever (29 calls in 300 ms against a fast failure).
  let calls = 0;
  const music: ClipMusicSource = { list: async () => { calls++; throw new Error("studio is not ready"); }, load: async () => ({ take: null, gridMs: 0 }) };
  const h = await harness({ music });
  try {
    h.target.querySelector<HTMLButtonElement>('[data-tab="music"]')!.click();
    for (let i = 0; i < 5; i++) await settle();
    assert.equal(calls, 1, "asked exactly once");
    assert.match(h.target.querySelector(".lib")!.textContent ?? "", /Studio is not ready/);
    h.click("retry-songs");
    for (let i = 0; i < 3; i++) await settle();
    assert.equal(calls, 2, "and again only when asked");
  } finally { await h.destroy(); }

  const empty = await harness({ music: { list: async () => [], load: async () => ({ take: null, gridMs: 0 }) } });
  try {
    empty.target.querySelector<HTMLButtonElement>('[data-tab="music"]')!.click();
    await settle();
    assert.match(empty.target.querySelector(".lib")!.textContent ?? "", /none yet/, "an empty Studio says so, it is not loading forever");
  } finally { await empty.destroy(); }
});

/**
 * Just enough of an `OfflineAudioContext` for `renderTakeOffline` to render a take with no notes:
 * every node is inert and the render is silence of the asked length. jsdom has none of Web Audio.
 * `gate`, when given, holds every render open until it resolves.
 */
function installSilentOffline(gate: Promise<void> = Promise.resolve()): () => void {
  const g = globalThis as { OfflineAudioContext?: unknown };
  const previous = g.OfflineAudioContext;
  const node = () => ({ connect: (to: unknown) => to, disconnect() {} });
  const param = () => ({ value: 0, setValueAtTime() {}, linearRampToValueAtTime() {}, exponentialRampToValueAtTime() {}, setTargetAtTime() {}, cancelScheduledValues() {}, cancelAndHoldAtTime() {} });
  const source = () => ({ ...node(), start() {}, stop() {}, onended: null });
  g.OfflineAudioContext = class {
    readonly length: number;
    readonly sampleRate: number;
    readonly destination = node();
    readonly currentTime = 0;
    readonly state = "suspended";
    constructor(o: { length: number; sampleRate: number }) { this.length = o.length; this.sampleRate = o.sampleRate; }
    createGain() { return { ...node(), gain: param() }; }
    createBiquadFilter() { return { ...node(), type: "lowpass", frequency: param(), Q: param(), detune: param(), gain: param() }; }
    createDelay() { return { ...node(), delayTime: param() }; }
    createDynamicsCompressor() { return { ...node(), threshold: param(), knee: param(), ratio: param(), attack: param(), release: param() }; }
    createWaveShaper() { return { ...node(), curve: null, oversample: "none" }; }
    createStereoPanner() { return { ...node(), pan: param() }; }
    createConstantSource() { return { ...source(), offset: param() }; }
    createOscillator() { return { ...source(), type: "sine", frequency: param(), detune: param() }; }
    createBufferSource() { return { ...source(), buffer: null, loop: false, playbackRate: param() }; }
    createBuffer(_channels: number, length: number) { const data = new Float32Array(length); return { length, getChannelData: () => data }; }
    suspend() { return Promise.resolve(); }
    resume() { return Promise.resolve(); }
    async startRendering() {
      await gate;
      return { length: this.length, sampleRate: this.sampleRate, numberOfChannels: 2, duration: this.length / this.sampleRate };
    }
  };
  return () => { g.OfflineAudioContext = previous; };
}
const SILENT_TAKE = {
  v: 1, group: "group", call: "call", met: { bpm: 120, bpb: 4 }, parts: ["author"], lanes: [{ src: 0, sn: "a".repeat(16) }], patches: [], events: [],
};

test("posting waits for a song still being added, and a song that expired meanwhile stops the post", async () => {
  // Found in review: posting while a song was still rendering dropped it with a false "no more
  // sounds fit"; and a song was checked only when added, so one that expired with the editor
  // open was still posted.
  const restore = installSilentOffline();
  const run = async (expires: boolean) => {
    let offered = true;
    let release = () => {};
    const music: ClipMusicSource = {
      list: async () => (offered ? [{ id: "s1", title: "Night Drive" }] : []),
      load: () => new Promise((resolve) => { release = () => resolve({ take: SILENT_TAKE, gridMs: 2000 }); }),
    };
    const h = await harness({ music, context: "chat" });
    try {
      h.target.querySelector<HTMLButtonElement>('[data-tab="music"]')!.click();
      await settle();
      h.target.querySelector<HTMLButtonElement>('[data-add-song="s1"]')!.click();
      await settle();
      assert.equal(h.action("upload")!.disabled, true, "not while the song is still being added");
      assert.equal(h.action("upload")!.title, "Still adding to the clip");
      release();
      for (let i = 0; i < 3; i++) await settle();
      assert.equal(h.action("upload")!.disabled, false);
      assert.equal(h.bars().filter((b) => b.startsWith("audio:")).length, 1, "the song is on the clip");
      if (expires) offered = false;
      h.click("upload");
      for (let i = 0; i < 3; i++) await settle();
      return { compositions: h.compositions.length, results: h.results.map((r) => r.kind), alert: h.target.querySelector(".alert")?.textContent ?? "" };
    } finally { await h.destroy(); }
  };
  try {
    assert.deepEqual(await run(false), { compositions: 1, results: ["edited"], alert: "" }, "still offered: it posts");
    const gone = await run(true);
    assert.equal(gone.compositions, 0, "nothing was rendered");
    assert.deepEqual(gone.results, []);
    assert.match(gone.alert, /A song in this clip is not here anymore/);
  } finally { restore(); }
});

test("an add that never finishes can be stopped, and posting works again", async () => {
  // Found in re-review: posting waits for pending adds, so one decode or probe that never ended
  // disabled the post for good, and the only way out threw the clip away.
  let answer: ((probe: VideoProbe) => void) | null = null;
  const h = await harness({ probe: (f: Blob) => (f instanceof File && f.name === "second.mp4" ? new Promise<VideoProbe>((r) => { answer = r; }) : Promise.resolve(READY)) });
  try {
    h.key("o"); // something to post
    const drop = new w.Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(drop, "dataTransfer", { value: { types: ["Files"], files: [new File(["x"], "second.mp4", { type: "video/mp4" })] } });
    h.target.querySelector('[role="dialog"]')!.dispatchEvent(drop);
    await settle();
    assert.equal(h.action("upload")!.disabled, true, "waiting for the video");
    h.click("stop-adding");
    assert.equal(h.action("upload")!.disabled, false, "posting works again");
    assert.equal(h.action("stop-adding"), null);
    answer!(READY);
    for (let i = 0; i < 3; i++) await settle();
    assert.deepEqual(h.bars().filter((b) => b.startsWith("layer:")), [], "the video given up on is not added when it finally answers");
    h.click("upload");
    await settle();
    assert.equal(h.exports.length, 1, "and the clip posts");
  } finally { await h.destroy(); }
});

test("stopping a song still loading frees its row and the queue for the next one", async () => {
  const loads: string[] = [];
  const gates = new Map<string, () => void>();
  const music: ClipMusicSource = {
    list: async () => ["s1", "s2"].map((id) => ({ id, title: id })),
    load: (id) => {
      loads.push(id);
      return new Promise((resolve) => { gates.set(id, () => resolve({ take: null, gridMs: 0 })); });
    },
  };
  const h = await harness({ music });
  try {
    h.target.querySelector<HTMLButtonElement>('[data-tab="music"]')!.click();
    await settle();
    h.target.querySelector<HTMLButtonElement>('[data-add-song="s1"]')!.click();
    await settle();
    assert.equal(h.target.querySelector<HTMLButtonElement>('[data-add-song="s1"]')!.disabled, true, "rendering");
    h.click("stop-adding");
    assert.equal(h.target.querySelector<HTMLButtonElement>('[data-add-song="s1"]')!.disabled, false, "the row is free again");
    h.target.querySelector<HTMLButtonElement>('[data-add-song="s2"]')!.click();
    await settle();
    assert.deepEqual(loads, ["s1", "s2"], "the stuck song no longer holds the queue");
    gates.get("s1")!();
    await settle();
    assert.doesNotMatch(h.target.querySelector(".lib")!.textContent ?? "", /could not be read.*could not be read/s, "the stopped song reports nothing");
  } finally {
    for (const open of gates.values()) open();
    await h.destroy();
  }
});

test("stopping a song mid-render does not let another render start beside it", async () => {
  // Found in re-review: "stop adding" also let go of the queue during a render, which cannot be
  // interrupted, so stopping and adding again ran two renders (and two outputs) at once.
  let open = () => {};
  const restore = installSilentOffline(new Promise<void>((r) => { open = r; }));
  const loads: string[] = [];
  const music: ClipMusicSource = {
    list: async () => [{ id: "s1", title: "Night Drive" }],
    load: async (id) => { loads.push(id); return { take: SILENT_TAKE, gridMs: 2000 }; },
  };
  const h = await harness({ music });
  try {
    h.target.querySelector<HTMLButtonElement>('[data-tab="music"]')!.click();
    await settle();
    h.target.querySelector<HTMLButtonElement>('[data-add-song="s1"]')!.click();
    await settle();
    assert.deepEqual(loads, ["s1"], "rendering");
    h.click("stop-adding");
    assert.equal(h.action("upload")!.disabled, false, "posting does not wait for it");
    h.target.querySelector<HTMLButtonElement>('[data-add-song="s1"]')!.click();
    await settle();
    assert.deepEqual(loads, ["s1"], "the second waits for the render still running");
    open();
    for (let i = 0; i < 4; i++) await settle();
    assert.deepEqual(loads, ["s1", "s1"], "then takes its turn");
    assert.equal(h.bars().filter((b) => b.startsWith("audio:")).length, 1, "only the second lands on the clip");
  } finally { open(); restore(); await h.destroy(); }
});

test("a cancel while the songs are checked at post starts no render", async () => {
  // Found missing in re-review: nothing pinned the cancel check between the list and the render.
  const restore = installSilentOffline();
  let release = () => {};
  let holdList = false;
  const music: ClipMusicSource = {
    list: () => (holdList ? new Promise((resolve) => { release = () => resolve([{ id: "s1", title: "Night Drive" }]); }) : Promise.resolve([{ id: "s1", title: "Night Drive" }])),
    load: async () => ({ take: SILENT_TAKE, gridMs: 2000 }),
  };
  const h = await harness({ music });
  try {
    h.target.querySelector<HTMLButtonElement>('[data-tab="music"]')!.click();
    await settle();
    h.target.querySelector<HTMLButtonElement>('[data-add-song="s1"]')!.click();
    for (let i = 0; i < 3; i++) await settle();
    holdList = true;
    h.click("upload");
    await settle();
    assert.equal(h.state(), "exporting");
    h.click("cancel-export");
    release();
    for (let i = 0; i < 3; i++) await settle();
    assert.equal(h.compositions.length, 0, "the composer never started");
    assert.equal(h.state(), "ready");
    assert.deepEqual(h.results, []);
  } finally { restore(); await h.destroy(); }
});

test("a file dropped while the discard question is up does not change the clip behind it", async () => {
  let probes = 0;
  const h = await harness({ probe: async () => { probes++; return READY; } });
  try {
    h.key("o");
    h.click("close");
    await settle();
    assert.ok(h.action("keep-editing"), "the question is up");
    const drop = new w.Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(drop, "dataTransfer", { value: { types: ["Files"], files: [new File(["x"], "second.mp4", { type: "video/mp4" })] } });
    h.target.querySelector(".ve-confirm")!.dispatchEvent(drop);
    for (let i = 0; i < 3; i++) await settle();
    assert.equal(drop.defaultPrevented, true, "still kept from the page behind");
    assert.equal(probes, 1, "only the clip itself was ever probed");
    assert.deepEqual(h.bars(), []);
  } finally { await h.destroy(); }
});

test("an added video with no sound offers no mute or volume anywhere", async () => {
  const h = await harness({ probe: async () => ({ ...READY, hasAudio: false }) });
  try {
    const drop = new w.Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(drop, "dataTransfer", { value: { types: ["Files"], files: [new File(["x"], "second.mp4", { type: "video/mp4" })] } });
    h.target.querySelector('[role="dialog"]')!.dispatchEvent(drop);
    for (let i = 0; i < 3; i++) await settle();
    assert.equal(h.inspector(), "layer", "the new video is selected");
    assert.equal(h.target.querySelector('[data-inspector] [data-action="mute-item"]'), null, "no mute in the inspector");
    assert.equal(h.field("volume"), null, "and no volume");
    const lane = [...h.target.querySelectorAll<HTMLButtonElement>("[data-lane-head] button")].find((b) => b.getAttribute("aria-label") === "Mute V2")!;
    assert.equal(lane.disabled, true, "its lane's mute is off too");
  } finally { await h.destroy(); }
});

test("a dropped file that cannot go on a clip is refused in the footer, and the drop stays in the editor", async () => {
  const h = await harness();
  try {
    const drop = new w.Event("drop", { bubbles: true, cancelable: true });
    Object.defineProperty(drop, "dataTransfer", { value: { types: ["Files"], files: [new File(["x"], "notes.txt", { type: "text/plain" })] } });
    h.target.querySelector('[role="dialog"]')!.dispatchEvent(drop);
    await settle();
    assert.equal(drop.defaultPrevented, true, "the page behind never sees it as a navigation or an attachment");
    assert.match(h.target.querySelector(".foot .notice")?.textContent ?? "", /can't go on a clip/);
    assert.deepEqual(h.bars(), []);
  } finally { await h.destroy(); }
});
