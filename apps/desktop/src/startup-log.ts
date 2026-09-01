/**
 * Diagnostics for the window before the application exists.
 *
 * `main.ts` imports `App.svelte` and mounts it, and until now the frontend logger was installed
 * inside that component's `onMount`. Everything ahead of that hook was unobserved: a module that
 * threw while evaluating, a dynamic import that never resolved, a mount that failed. Those are
 * exactly the failures that leave a user looking at a blank window with nothing to send, and they
 * were the ones nothing recorded.
 *
 * This module imports nothing at runtime, and that is a requirement rather than tidiness. A static
 * import is evaluated before the body of the module that asks for it, so anything named at the top
 * of this file would get its chance to throw before `beginStartupCapture()` had run. The capture
 * used to live in `uilog.ts` and be imported from here, which meant a failure while evaluating
 * `uilog.ts` was invisible to the one thing whose job was to see it. `startup-log.test.ts` holds
 * that rule, here and one level up in `main.ts`.
 *
 * Two jobs: buffer startup exceptions until the real logger can take them, and render something
 * honest if the application never arrives at all.
 */

import type { UiLogRecord } from "./uilog.ts";

// --- capture -----------------------------------------------------------------------------------

/**
 * How many failures are held, and how much of each.
 *
 * Bounded because a startup failure is often a loop and none of the rate limiting in `uilog.ts`
 * exists yet at this point. The character bound matches `MAX_UI_LOG_CHARS`, since these records
 * are handed to the same native sink once the bridge is reachable.
 */
const MAX_CAPTURED = 50;
const MAX_CAPTURED_CHARS = 2000;

type CaptureScope = {
  addEventListener: Window["addEventListener"];
  removeEventListener?: Window["removeEventListener"];
  limit?: number;
};

let capture: { drain: () => UiLogRecord[]; stop: () => void } | null = null;

/**
 * Start watching. Called as the first statement of `main.ts`, before anything else is imported, so
 * an error thrown while a module is being evaluated has somewhere to land.
 */
export function beginStartupCapture(scope?: CaptureScope): void {
  if (capture) return;
  const target: CaptureScope = scope ?? {
    addEventListener: window.addEventListener.bind(window),
    removeEventListener: window.removeEventListener.bind(window),
  };
  const limit = target.limit ?? MAX_CAPTURED;
  let held: UiLogRecord[] = [];

  const keep = (message: string) => {
    if (held.length >= limit) return;
    const bounded =
      message.length > MAX_CAPTURED_CHARS ? `${message.slice(0, MAX_CAPTURED_CHARS)} [truncated]` : message;
    held.push({ level: "error", message: bounded, repeats: 1 });
  };
  const onError = (e: Event) => {
    const err = e as ErrorEvent;
    keep(`startup uncaught: ${err.message ?? "error"} at ${err.filename ?? "?"}:${err.lineno ?? 0}`);
  };
  const onRejection = (e: Event) => {
    keep(`startup unhandled rejection: ${describeStartupError((e as PromiseRejectionEvent).reason)}`);
  };
  target.addEventListener("error", onError);
  target.addEventListener("unhandledrejection", onRejection);

  capture = {
    drain: () => {
      const out = held;
      held = [];
      return out;
    },
    stop: () => {
      target.removeEventListener?.("error", onError);
      target.removeEventListener?.("unhandledrejection", onRejection);
    },
  };
}

/**
 * Take what was captured, for forwarding to the native log once the bridge is reachable.
 *
 * Draining rather than copying: these records are handed over exactly once, and the buffer exists
 * only because there was nowhere better to put them.
 */
export function drainStartupLog(): UiLogRecord[] {
  return capture?.drain() ?? [];
}

/** Stop watching, once the full logger has taken over. */
export function endStartupCapture(): void {
  capture?.stop();
  capture = null;
}

// --- describing the failure --------------------------------------------------------------------

/**
 * A short id for one startup failure.
 *
 * Printed on the failure screen and included in whatever the user sends, so a screenshot of a
 * blank-window report can be tied to the records describing it. Derived from the clock: it only
 * has to be quotable and locally unique, and a startup path is the wrong place to reach for
 * anything with a dependency.
 */
export function diagnosticId(now: number = Date.now()): string {
  return `sf-${(now % 0xffffff).toString(16).padStart(6, "0")}`;
}

/** How a caught startup failure reads, whatever kind of thing was thrown. */
export function describeStartupError(error: unknown): string {
  if (error instanceof Error) return error.stack ? `${error.name}: ${error.message}\n${error.stack}` : `${error.name}: ${error.message}`;
  if (typeof error === "string") return error;
  try {
    return JSON.stringify(error);
  } catch {
    return Object.prototype.toString.call(error);
  }
}

// --- redaction ---------------------------------------------------------------------------------

/**
 * A URL, a Windows path or a POSIX path, in the forms a stack frame prints them.
 *
 * Three branches rather than one because they end differently: a URL runs to the first delimiter,
 * a Windows path starts at a drive letter, and a bare POSIX path is only recognised when it has at
 * least two segments so that ordinary prose containing a slash is left alone.
 */
const LOCATION = /[a-z][a-z0-9+.-]*:\/\/[^\s'"`)\]]+|[A-Za-z]:[\\/][^\s'"`)\]]*|\/[\w.-]+(?:\/[\w.-]+)+/gi;

/**
 * A libp2p peer id or a hex fingerprint, matching the shapes `debug-console.ts` masks.
 *
 * Not reused from there: that module is one of the things that may have failed to load, and
 * importing it would put the failure inside the report about the failure.
 */
const PEER_B58 = /\b12D3Koo[1-9A-HJ-NP-Za-km-z]*/g;
const LONG_HEX = /\b[0-9a-f]{8,}\b/gi;

/**
 * The file name a path or URL ends in, which is the part that names the module that failed.
 *
 * The rest is the install location. On Windows it begins with the account name, which is a real
 * name often enough to matter, and on a development machine the checkout path says where the
 * person keeps their work.
 */
function lastSegment(location: string): string {
  const withoutQuery = location.split(/[?#]/)[0] ?? "";
  const parts = withoutQuery.split(/[\\/]/).filter((part) => part && !part.endsWith(":"));
  const tail = parts[parts.length - 1] ?? "";
  return tail ? `[path]/${tail}` : "[path]";
}

/**
 * Strip the values that identify the machine rather than the failure.
 *
 * Applied to the whole report and not only to the stack: the message of a failed dynamic import is
 * itself a URL, and the buffered records carry the `filename` of every error event.
 */
export function redactStartupText(text: string): string {
  return text
    .replace(LOCATION, (found) => lastSegment(found))
    .replace(PEER_B58, "[id]")
    .replace(LONG_HEX, "[id]");
}

/**
 * The engine family, which is the half of a user agent that diagnoses a startup failure.
 *
 * "WebView2" versus "WebKit" changes which failures are plausible. The build numbers around it
 * narrow the machine down without saying anything further about why the window is blank.
 */
export function webviewFamily(userAgent: string): string {
  if (/WebView2|Edg\//i.test(userAgent)) return "WebView2";
  if (/Chrome|Chromium/i.test(userAgent)) return "Chromium";
  if (/AppleWebKit/i.test(userAgent)) return "WebKit";
  if (/Gecko\//.test(userAgent)) return "Gecko";
  return "unknown webview";
}

// --- the two reports -----------------------------------------------------------------------------

function buildReport(
  id: string,
  error: unknown,
  captured: readonly UiLogRecord[],
  environment: string,
  transform: (text: string) => string,
): string {
  const lines = [
    `Mewtual failed to start`,
    `diagnostic: ${id}`,
    `environment: ${environment}`,
    "",
    transform(describeStartupError(error)),
  ];
  if (captured.length) {
    lines.push("", "earlier startup errors:");
    for (const record of captured) lines.push(`  ${transform(record.message)}`);
  }
  return lines.join("\n");
}

/**
 * The text of a startup-failure report, shown by default and copied by the ordinary button.
 *
 * Redacted before it is offered. The screen previously carried one report and told the user it
 * contained "no message text, file contents, names or key material" while printing a raw stack:
 * every frame in that stack is an absolute path, which on Windows begins with the account name.
 * Nothing native is reachable to validate an export at this point, so the only place the promise
 * can be made true is here, before the text reaches the screen the promise is printed on.
 *
 * Kept separate from the DOM so it can be tested, and so the screen and the clipboard cannot
 * disagree about what happened: the same string does both jobs.
 */
export function startupFailureSummary(
  id: string,
  error: unknown,
  captured: readonly UiLogRecord[],
  userAgent: string,
): string {
  return buildReport(id, error, captured, webviewFamily(userAgent), redactStartupText);
}

/**
 * The same failure with nothing removed, for a reader who has been asked for it.
 *
 * Offered rather than withheld, because the install path and the exact webview build are sometimes
 * the answer: a startup failure that only happens under one WebView2 revision reads as noise in the
 * summary. It sits behind a disclosure that says what it contains, which is the honest version of
 * the sentence the screen used to print.
 */
export function startupFailureDetail(
  id: string,
  error: unknown,
  captured: readonly UiLogRecord[],
  userAgent: string,
): string {
  return buildReport(id, error, captured, userAgent, (text) => text);
}

// --- the screen ----------------------------------------------------------------------------------

/** What the raw section says about itself, printed above the text rather than only in a tooltip. */
export const RAW_DETAIL_WARNING =
  "Not redacted. This can contain the folder the app was installed into, which usually includes " +
  "your account name, along with URLs and identifiers from this device. Read it before sending it.";

function reportBlock(text: string): HTMLPreElement {
  const pre = document.createElement("pre");
  pre.textContent = text;
  pre.style.cssText =
    "margin:0;padding:12px;max-width:100%;overflow:auto;background:#0b0a11;border:1px solid #2a2635;" +
    "border-radius:8px;font-family:ui-monospace,monospace;font-size:12px;white-space:pre-wrap;" +
    "overflow-wrap:anywhere;user-select:text";
  return pre;
}

function copyButton(label: string, text: string): HTMLButtonElement {
  const button = document.createElement("button");
  button.textContent = label;
  button.style.cssText =
    "padding:6px 12px;background:transparent;color:#d8d4e4;border:1px solid #2a2635;border-radius:6px;cursor:pointer";
  button.onclick = () => {
    void navigator.clipboard
      ?.writeText(text)
      .then(() => (button.textContent = "Copied"))
      .catch(() => (button.textContent = "Select the text above instead"));
  };
  return button;
}

/**
 * Draw a minimal failure screen.
 *
 * Uses inline styles and no application classes on purpose. Reaching this point means an
 * application module did not load, and the stylesheet is one of the things that may not have
 * arrived; a failure screen that depends on the failure not having happened is not a failure
 * screen. Text is selectable so a user can copy the diagnostic id even if the button is the part
 * that is broken.
 *
 * Two reports, not one. The summary is what the screen offers, and the raw detail is behind a
 * closed disclosure so that sending it is a decision rather than the default.
 */
export function renderStartupFailure(target: HTMLElement, id: string, summary: string, detail: string): void {
  target.textContent = "";
  const wrap = document.createElement("div");
  wrap.setAttribute("role", "alert");
  wrap.style.cssText =
    "position:fixed;inset:0;display:flex;flex-direction:column;gap:12px;align-items:flex-start;" +
    "padding:32px;background:#12101a;color:#d8d4e4;font:13px/1.5 ui-sans-serif,system-ui,sans-serif;overflow:auto";

  const heading = document.createElement("h1");
  heading.textContent = "Mewtual could not start";
  heading.style.cssText = "margin:0;font-size:1.1rem;font-weight:600";

  const advice = document.createElement("p");
  advice.style.cssText = "margin:0;max-width:60ch;color:#9c96b0";
  advice.textContent =
    "The application failed to load. Restarting usually clears it. The summary below has file " +
    "paths, addresses and identifiers replaced, and is the part to send.";

  const idLine = document.createElement("p");
  idLine.style.cssText = "margin:0;font-family:ui-monospace,monospace;color:#c9a2ff;user-select:text";
  idLine.textContent = `diagnostic ${id}`;

  const raw = document.createElement("details");
  raw.style.cssText = "width:100%;max-width:100%";
  const rawLabel = document.createElement("summary");
  rawLabel.textContent = "Show raw technical detail";
  rawLabel.style.cssText = "cursor:pointer;color:#c9a2ff;margin-bottom:8px";
  const rawWarning = document.createElement("p");
  rawWarning.textContent = RAW_DETAIL_WARNING;
  rawWarning.style.cssText = "margin:0 0 8px;max-width:60ch;color:#e0b0b0";
  const rawCopy = copyButton("Copy raw detail", detail);
  rawCopy.style.marginTop = "8px";
  raw.append(rawLabel, rawWarning, reportBlock(detail), rawCopy);

  wrap.append(heading, advice, idLine, reportBlock(summary), copyButton("Copy report", summary), raw);
  target.append(wrap);
}
