/**
 * Diagnostics for the window before the application exists.
 *
 * `main.ts` dynamically imports `App.svelte` and mounts it, and until now the frontend logger was
 * installed inside that component's `onMount`. Everything ahead of that hook was unobserved: a
 * module that threw while evaluating, a dynamic import that never resolved, a mount that failed.
 * Those are exactly the failures that leave a user looking at a blank window with nothing to send,
 * and they were the ones nothing recorded.
 *
 * Two jobs, both deliberately dependency-free so they cannot themselves be the thing that fails:
 * buffer startup exceptions until the real logger can take them, and render something honest if
 * the application never arrives at all.
 */

import { installBootstrapCapture, type UiLogRecord } from "./uilog.ts";

let capture: ReturnType<typeof installBootstrapCapture> | null = null;

/**
 * Start watching. Called first thing in `main.ts`, before any application module is imported, so
 * an error thrown during that import has somewhere to land.
 */
export function beginStartupCapture(scope?: {
  addEventListener: Window["addEventListener"];
  removeEventListener?: Window["removeEventListener"];
}): void {
  if (capture) return;
  const target = scope ?? {
    addEventListener: window.addEventListener.bind(window),
    removeEventListener: window.removeEventListener.bind(window),
  };
  capture = installBootstrapCapture(target);
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

/**
 * The text of a startup-failure report, ready to be shown and copied.
 *
 * Kept separate from the DOM so it can be tested, and so the screen and the clipboard cannot
 * disagree about what happened: the same string does both jobs.
 */
export function startupFailureReport(
  id: string,
  error: unknown,
  captured: readonly UiLogRecord[],
  userAgent: string,
): string {
  const lines = [
    `Mewtual failed to start`,
    `diagnostic: ${id}`,
    `environment: ${userAgent}`,
    "",
    describeStartupError(error),
  ];
  if (captured.length) {
    lines.push("", "earlier startup errors:");
    for (const record of captured) lines.push(`  ${record.message}`);
  }
  return lines.join("\n");
}

/**
 * Draw a minimal failure screen.
 *
 * Uses inline styles and no application classes on purpose. Reaching this point means an
 * application module did not load, and the stylesheet is one of the things that may not have
 * arrived; a failure screen that depends on the failure not having happened is not a failure
 * screen. Text is selectable so a user can copy the diagnostic id even if the button is the part
 * that is broken.
 */
export function renderStartupFailure(target: HTMLElement, id: string, report: string): void {
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
    "The application failed to load. Restarting usually clears it. If it does not, the detail " +
    "below identifies the failure and is safe to send: it contains no message text, file " +
    "contents, names or key material.";

  const idLine = document.createElement("p");
  idLine.style.cssText = "margin:0;font-family:ui-monospace,monospace;color:#c9a2ff;user-select:text";
  idLine.textContent = `diagnostic ${id}`;

  const detail = document.createElement("pre");
  detail.textContent = report;
  detail.style.cssText =
    "margin:0;padding:12px;max-width:100%;overflow:auto;background:#0b0a11;border:1px solid #2a2635;" +
    "border-radius:8px;font-family:ui-monospace,monospace;font-size:12px;white-space:pre-wrap;" +
    "overflow-wrap:anywhere;user-select:text";

  const copy = document.createElement("button");
  copy.textContent = "Copy report";
  copy.style.cssText =
    "padding:6px 12px;background:transparent;color:#d8d4e4;border:1px solid #2a2635;border-radius:6px;cursor:pointer";
  copy.onclick = () => {
    void navigator.clipboard
      ?.writeText(report)
      .then(() => (copy.textContent = "Copied"))
      .catch(() => (copy.textContent = "Select the text above instead"));
  };

  wrap.append(heading, advice, idLine, detail, copy);
  target.append(wrap);
}
