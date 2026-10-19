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
  capBytes?: number;
  size?: number;
};

async function harness(options: Options = {}) {
  const target = document.createElement("div");
  document.body.append(target);
  const results: VideoEditorResult[] = [];
  const exports: Array<{ edit: VideoEdit; capBytes: number; signal?: AbortSignal }> = [];
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
    target, results, exports, stripCalls, file,
    state: () => card().dataset.state,
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
    assert.match(h.chip(), /keeps 0:03\.0 of 0:10\.0 · 1 removed/);
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
    Enter: "Ctrl+Enter", b: "B", s: "S", i: "I", o: "O", m: "M", z: "Ctrl+Z", y: "Ctrl+Y",
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
