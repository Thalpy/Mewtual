/**
 * Send what the webview sees to the native debug log.
 *
 * Half of this app runs in the webview, and until this existed none of it reached the log file:
 * every `console.warn` in the voice path (failed signals, rejected ICE candidates, offer handling
 * that threw) went to a devtools console nobody had open. A debug log that cannot see the
 * frontend is a log of half the app, which is why voice has been the hardest thing here to
 * diagnose from a bug report.
 *
 * Everything is best-effort: logging must never be able to break the thing it is observing, so a
 * failed send is swallowed rather than retried or surfaced.
 */

export type UiLogLevel = "error" | "warn" | "info" | "debug";

/** How many characters of one line survive. The native side truncates too; this saves the IPC. */
export const MAX_UI_LOG_CHARS = 2000;

/**
 * Render console arguments as one line.
 *
 * Errors are unwrapped to `name: message` plus their stack, because `String(err)` on an Error
 * gives a bare message with no frames and those frames are the entire value of the report.
 */
export function formatLogArgs(args: readonly unknown[]): string {
  const parts = args.map((a) => {
    if (typeof a === "string") return a;
    if (a instanceof Error) return a.stack ? `${a.name}: ${a.message}\n${a.stack}` : `${a.name}: ${a.message}`;
    try {
      return JSON.stringify(a);
    } catch {
      // Cyclic, or something with a throwing getter. Its type is still worth knowing.
      return Object.prototype.toString.call(a);
    }
  });
  const line = parts.join(" ");
  return line.length > MAX_UI_LOG_CHARS ? `${line.slice(0, MAX_UI_LOG_CHARS)} [truncated]` : line;
}

/**
 * Drop a line that repeats one we just sent.
 *
 * A render loop or a reconnect loop can produce thousands of identical lines a second, which
 * costs an IPC round trip each and buries whatever else happened. Identical consecutive lines
 * inside the window collapse; a different line always passes, so this cannot hide a new problem.
 */
export function makeDeduper(windowMs = 2000) {
  let lastLine = "";
  let lastAt = -Infinity;
  return (line: string, now: number): boolean => {
    if (line === lastLine && now - lastAt < windowMs) return false;
    lastLine = line;
    lastAt = now;
    return true;
  };
}

/**
 * Route console errors/warnings, uncaught exceptions and unhandled rejections to `sink`.
 *
 * The original console methods are always called first, so devtools keeps behaving normally and
 * a broken sink cannot swallow output a developer is watching for.
 */
export function installUiLogging(
  sink: (level: UiLogLevel, message: string) => void,
  scope: {
    console: Pick<Console, "error" | "warn">;
    addEventListener: Window["addEventListener"];
    now?: () => number;
  },
): () => void {
  const now = scope.now ?? (() => Date.now());
  const allow = makeDeduper();
  const original = { error: scope.console.error, warn: scope.console.warn };

  const forward = (level: UiLogLevel, args: unknown[]) => {
    try {
      const line = formatLogArgs(args);
      if (allow(line, now())) sink(level, line);
    } catch {
      /* logging must never break what it observes */
    }
  };

  scope.console.error = (...args: unknown[]) => {
    original.error.apply(scope.console, args as []);
    forward("error", args);
  };
  scope.console.warn = (...args: unknown[]) => {
    original.warn.apply(scope.console, args as []);
    forward("warn", args);
  };

  const onError = (e: Event) => {
    const err = e as ErrorEvent;
    forward("error", [
      `uncaught: ${err.message ?? "error"} at ${err.filename ?? "?"}:${err.lineno ?? 0}`,
      err.error,
    ]);
  };
  const onRejection = (e: Event) => {
    forward("error", ["unhandled rejection:", (e as PromiseRejectionEvent).reason]);
  };
  scope.addEventListener("error", onError);
  scope.addEventListener("unhandledrejection", onRejection);

  return () => {
    scope.console.error = original.error;
    scope.console.warn = original.warn;
  };
}
