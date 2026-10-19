/**
 * What happens to things dropped on the window from outside the app.
 *
 * Tauri's own drag-and-drop handling is switched off for the main window (`dragDropEnabled: false`
 * in tauri.conf.json): on Windows it otherwise swallows every file dropped on the window, so none
 * of the page's drop zones (chat, status, wiki, Files, event art) ever saw one. With it off, the
 * webview behaves like a browser, which brings three defaults this window must not have:
 *
 * 1. A file dropped outside every zone is opened, navigating the whole app to a local file.
 * 2. A link dragged in from another app and dropped anywhere that is not a text field is followed,
 *    which navigates the main webview (there is no back button in this window).
 * 3. A file released over an embedded third-party player (YouTube, Spotify and the other
 *    `frame-src` sites) is delivered to THAT site's page, which could read it.
 *
 * So, at window level:
 * - an unaccepted `dragover` of an outside drag gets the drop effect `none` (the cursor says no and
 *   releasing performs no operation), and its `drop` has its default prevented. A zone that
 *   accepted the drag has already handled it on the way up;
 * - text-entry fields keep their native text drop, and visible `<input type=file>` controls their
 *   native file drop, since those are what the person is aiming at. Any other control (a checkbox,
 *   a slider, a disabled or read-only field) is not a drop target, and the engine's fallback there
 *   is to follow the link, so those are guarded like the page;
 * - while files are dragged over the window, every iframe stops taking pointer events (the rule is
 *   in drag-drop.css), so a drop over an embed lands on this page beneath it: in the chat that is
 *   the chat's own drop zone, elsewhere it is refused. The shield lifts on drop or dragend, on the
 *   first pointer event after the drag (pointer events do not fire during one), or after
 *   `SHIELD_SAFETY_MS` with no drag event at all.
 *
 * Known limit: a drag that enters the window directly over an iframe, without first crossing part
 * of this page, is seen by that frame and not here, so it cannot raise the shield.
 *
 * Drags that start inside the page (selected text, a name effect) are not refused, but a file drag
 * of either origin raises the shield: nothing in the app drops files into a frame.
 */

/** The attribute on `<html>` that lowers every iframe out of the way during a file drag. */
export const FILE_DRAG_ATTR = "data-file-drag";

/**
 * Last-resort lift for a shield nothing else lowered. Long on purpose: `dragover` may arrive only
 * every ~550 ms, and on some platforms only when the pointer moves, so a short idle timer would
 * drop the shield under a drag that is merely holding still over an embed.
 */
export const SHIELD_SAFETY_MS = 3000;

type DragLike = {
  defaultPrevented: boolean;
  preventDefault(): void;
  target?: unknown;
  dataTransfer: { types: ArrayLike<string> | Iterable<string>; dropEffect: string } | null;
};

/** Whether a drag carries files. */
export function carriesFiles(e: { dataTransfer: { types: ArrayLike<string> | Iterable<string> } | null }): boolean {
  const types = e.dataTransfer?.types;
  return !!types && Array.from(types as Iterable<string>).includes("Files");
}

type ElementLike = { closest?(selector: string): unknown };

/** Inputs whose native drop inserts text; every other input type does not take a text drop. */
const TEXT_ENTRY = [
  "textarea", "input:not([type])", 'input[type="text"]', 'input[type="search"]', 'input[type="url"]',
  'input[type="email"]', 'input[type="tel"]', 'input[type="password"]', 'input[type="number"]',
].map((s) => `${s}:not(:disabled):not([readonly])`).join(", ");

/** A target that should keep its own native drop: a text-entry field, or a visible file input. */
export function keepsNativeDrop(target: unknown, files: boolean): boolean {
  const el = target as ElementLike | null;
  if (!el || typeof el.closest !== "function") return false;
  if (files) return !!el.closest('input[type="file"]:not(:disabled)');
  return !!el.closest(`${TEXT_ENTRY}, [contenteditable=""], [contenteditable="true"]`);
}

/**
 * The window-level decisions, separated from listener wiring so they are testable without a DOM.
 * `internal` is whether a drag that started inside this page is in progress.
 */
export function onWindowDragOver(e: DragLike, internal: boolean): void {
  if (internal || e.defaultPrevented) return;
  if (keepsNativeDrop(e.target, carriesFiles(e))) return;
  e.preventDefault();
  if (e.dataTransfer) e.dataTransfer.dropEffect = "none";
}

export function onWindowDrop(e: DragLike, internal: boolean): void {
  if (internal || e.defaultPrevented) return;
  if (keepsNativeDrop(e.target, carriesFiles(e))) return;
  e.preventDefault();
}

type Listening = {
  addEventListener(type: string, listener: (event: Event) => void, options?: boolean | AddEventListenerOptions): void;
  removeEventListener(type: string, listener: (event: Event) => void, capture?: boolean): void;
};
type DocumentLike = Listening & {
  documentElement: { setAttribute(name: string, value: string): void; removeAttribute(name: string): void };
};
type Timers = {
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
};

/**
 * Install the guard on the window and document; returns the uninstaller. `timers` is injectable so
 * the shield's safety lift can be tested without waiting.
 */
export function installFileDropGuard(
  win: Listening,
  doc: DocumentLike,
  timers: Timers = { setTimeout: (fn, ms) => setTimeout(fn, ms), clearTimeout: (h) => clearTimeout(h as number) },
): () => void {
  let internal = false;
  let raised = false;
  let safety: unknown = null;

  const lower = () => {
    if (safety !== null) { timers.clearTimeout(safety); safety = null; }
    if (!raised) return;
    raised = false;
    doc.documentElement.removeAttribute(FILE_DRAG_ATTR);
  };
  const raise = () => {
    if (!raised) { raised = true; doc.documentElement.setAttribute(FILE_DRAG_ATTR, ""); }
    if (safety !== null) timers.clearTimeout(safety);
    safety = timers.setTimeout(lower, SHIELD_SAFETY_MS);
  };
  // Pointer events never fire while a drag is in progress, so the first one after a drag means it
  // is over, however it ended (a drop, Esc, a drag source removed before its dragend reached us).
  const settle = () => { internal = false; lower(); };

  const onDragStart = (event: Event) => {
    internal = true;
    // `dragend` fires on the source node, and a node removed from the page meanwhile never lets it
    // bubble here; listening on the source itself still hears it.
    (event.target as Listening | null)?.addEventListener?.("dragend", settle, { once: true });
  };
  // Capture phase: the shield must rise before any zone handles the event, and a zone that stops
  // propagation must not keep it from being refreshed.
  const onOverCapture = (event: Event) => { if (carriesFiles(event as unknown as DragLike)) raise(); };
  const onOver = (event: Event) => onWindowDragOver(event as unknown as DragLike, internal);
  const onDrop = (event: Event) => { onWindowDrop(event as unknown as DragLike, internal); settle(); };
  // Deliberately no `dragleave` lift: `dragenter` on the next element fires BEFORE `dragleave` on
  // the last, and `relatedTarget` is not reliable on drag events, so lowering on dragleave would
  // drop the shield for a moment on every element crossing, exactly as the pointer reaches an
  // embed. A drag that leaves the window is covered by the next pointer event or the safety lift.

  doc.addEventListener("dragstart", onDragStart, true);
  doc.addEventListener("dragend", settle, true);
  win.addEventListener("dragenter", onOverCapture, true);
  win.addEventListener("dragover", onOverCapture, true);
  win.addEventListener("dragover", onOver);
  win.addEventListener("drop", onDrop);
  win.addEventListener("pointermove", settle, true);
  win.addEventListener("pointerdown", settle, true);
  return () => {
    doc.removeEventListener("dragstart", onDragStart, true);
    doc.removeEventListener("dragend", settle, true);
    win.removeEventListener("dragenter", onOverCapture, true);
    win.removeEventListener("dragover", onOverCapture, true);
    win.removeEventListener("dragover", onOver);
    win.removeEventListener("drop", onDrop);
    win.removeEventListener("pointermove", settle, true);
    win.removeEventListener("pointerdown", settle, true);
    settle();
  };
}
