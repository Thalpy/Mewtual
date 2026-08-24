/**
 * Send what the webview sees to the native diagnostics.
 *
 * Half of this app runs in the webview, and until this existed none of it reached the log: every
 * `console.warn` in the voice path (failed signals, rejected ICE candidates, offer handling that
 * threw) went to a devtools console nobody had open. A log that cannot see the frontend is a log of
 * half the app, which is why voice has been the hardest thing here to diagnose from a bug report.
 *
 * Three properties this file has to hold, each of which it previously did not:
 *
 * 1. **Installing twice does not double the output.** The app supports F5 and HMR remounts while
 *    the native process stays alive. The old cleanup restored the console methods and left both
 *    window listeners attached, and the caller discarded it anyway, so every remount added another
 *    pair. One exception then arrived two, three, four times and a retry loop looked like it was
 *    accelerating when it was the logger multiplying. A diagnostic tool that fabricates evidence is
 *    worse than one that captures nothing.
 * 2. **Repetition is counted, not deleted.** The old deduper dropped identical consecutive lines
 *    and told nobody, so "one timeout" and "four thousand timeouts in two seconds" produced the
 *    same evidence, and the second of those is a retry storm.
 * 3. **Logging cannot break what it observes.** Everything here is wrapped: the original console
 *    method is always called first, a throwing sink is swallowed, and the queue is bounded so a
 *    render loop costs a counter rather than the tab's memory.
 */

import { makeOutbox, type SendOutcome } from "./diagnostics.ts";

export type UiLogLevel = "error" | "warn" | "info" | "debug";

/** One line for the native side, as `log_ui_batch` accepts it. */
export type UiLogRecord = {
  level: UiLogLevel;
  message: string;
  /**
   * How many identical lines this record stands for.
   *
   * `1` on a line's first appearance, which is sent straight away because a first error is news.
   * A later record with `repeats > 1` accounts for the ones that were collapsed after it: the
   * total is the first plus this. The native side renders it, so the count reaches the log rather
   * than a counter nobody reads.
   */
  repeats: number;
};

/** How many characters of one line survive. The native side truncates too; this saves the IPC. */
export const MAX_UI_LOG_CHARS = 2000;

/** How long identical consecutive lines collapse for. */
export const REPEAT_WINDOW_MS = 2000;

/** How long records wait to be sent together. Errors do not wait; see `flushSoon`. */
export const BATCH_INTERVAL_MS = 250;

/** The most records one IPC call carries. Matches the native cap. */
export const MAX_BATCH = 256;

/**
 * How many records may be waiting before the oldest are dropped.
 *
 * Bounded because the webview is exactly where an unbounded producer lives. Dropping is counted
 * and reported, never silent.
 */
export const MAX_QUEUE = 1000;

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
 * Collapse identical consecutive lines while keeping the count.
 *
 * A render or reconnect loop can produce thousands of identical lines a second, and forwarding each
 * one costs an IPC round trip and buries whatever else happened. Suppressing them without saying so
 * is the other failure: the frequency *is* the diagnosis, because a retry storm and a single
 * failure look identical once the repetition is gone.
 *
 * So the first line goes out immediately, the ones behind it are counted, and when the run ends the
 * count follows. Nothing is lost except the redundant copies.
 */
export function makeRepeatCollapser(windowMs = REPEAT_WINDOW_MS) {
  let line = "";
  let level: UiLogLevel = "info";
  let openedAt = -Infinity;
  let suppressed = 0;

  /** The summary owed for the run that just ended, if any. */
  function close(): UiLogRecord[] {
    if (suppressed <= 0) return [];
    const owed: UiLogRecord = { level, message: line, repeats: suppressed };
    suppressed = 0;
    return [owed];
  }

  return {
    /** Feed one line. Returns whatever is ready to send now. */
    push(next: UiLogLevel, text: string, now: number): UiLogRecord[] {
      const sameRun = text === line && level === next && now - openedAt < windowMs;
      if (sameRun) {
        suppressed += 1;
        return [];
      }
      // A new line, or the same one after the window: the run that was open is settled first so
      // its count lands in the log before the thing that replaced it.
      const owed = close();
      line = text;
      level = next;
      openedAt = now;
      return [...owed, { level: next, message: text, repeats: 1 }];
    },
    /** Settle any open run, so a count is not still pending when capture stops. */
    flush(): UiLogRecord[] {
      return close();
    },
  };
}

/**
 * Queue records and send them in bounded batches.
 *
 * One IPC call per `console.warn` is most expensive exactly when the webview is emitting fastest,
 * which is the moment worth capturing. Errors still leave on the next tick rather than waiting out
 * the interval: a page that is about to die should not be holding its explanation in a timer.
 */
// The queue, the bounded retry and the loss accounting live in `diagnostics.ts` so the two
// channels cannot drift apart about what "delivered" means.
export function makeBatcher(
  send: (records: UiLogRecord[], meta: { batch: number }) => Promise<SendOutcome>,
  scope: {
    setTimeout: (fn: () => void, ms: number) => unknown;
    clearTimeout: (handle: unknown) => void;
  },
) {
  // The queue, the bounded retry and the accounting are shared with the structured recorder,
  // because both had the same bug: they counted a send that threw *immediately* and the real send
  // is a promise that rejects long after the batch has been retired. See `makeOutbox`.
  const outbox = makeOutbox<UiLogRecord>(send, {
    setTimeout: scope.setTimeout,
    clearTimeout: scope.clearTimeout,
    maxPending: MAX_QUEUE,
    maxBatch: MAX_BATCH,
  });

  return {
    push(records: readonly UiLogRecord[]) {
      if (!records.length) return;
      outbox.push(records);
      // An error goes now. Everything else waits for the batch window, because one IPC call per
      // console line is most expensive exactly when the page is emitting fastest.
      outbox.flushSoon(records.some((r) => r.level === "error") ? 0 : BATCH_INTERVAL_MS);
    },
    flush: () => outbox.flush(),
    stop: () => outbox.stop(),
    /** Losses not yet told to the native side, for the caller to put in the record. */
    takeLoss: () => outbox.takeLoss(),
    /** Records that never reached the native side. Surfaced so a gap is never presented as quiet. */
    get dropped() {
      return outbox.dropped;
    },
    get pending() {
      return outbox.pending;
    },
  };
}

/**
 * Marks the one live installation on the global object.
 *
 * `Symbol.for` rather than a module-level variable on purpose: an HMR reload evaluates a *fresh*
 * copy of this module, so a module-scoped flag would be a new `false` every time and the guard
 * would never fire. The realm outlives the module, so the mark has to live there.
 */
const INSTALLED = Symbol.for("mewtual.uilog.installed");

type Installation = { generation: number; stop: () => void };

type Registry = Record<symbol, unknown>;

/** How many times logging has been installed in this realm, and whether one is live. */
export function uiLoggingState(registry: Registry = globalThis as unknown as Registry): {
  installed: boolean;
  generation: number;
} {
  const held = registry[INSTALLED] as Installation | undefined;
  return { installed: !!held, generation: held?.generation ?? 0 };
}

/** The handle `installUiLogging` returns. */
export type UiLogging = {
  /** Restore the console, remove both listeners, and send whatever is still queued. */
  stop: () => void;
  /** Which installation this is in this realm. A second live one would be a bug; see `INSTALLED`. */
  generation: number;
  /** Records that never reached the native side. */
  readonly dropped: number;
  /** Push everything queued right now. */
  flush: () => void;
};

/**
 * Route console errors and warnings, uncaught exceptions and unhandled rejections to `send`.
 *
 * The original console methods are always called first, so devtools keeps behaving normally and a
 * broken sink cannot swallow output a developer is watching for.
 *
 * Installing while an installation is live stops the old one first. That is not politeness: the
 * app supports F5 and HMR remounts, the previous version leaked a pair of window listeners on every
 * one of them, and duplicated exception records are indistinguishable from a real retry loop.
 */
export function installUiLogging(
  send: (records: UiLogRecord[], meta: { batch: number }) => Promise<SendOutcome>,
  scope: {
    console: Pick<Console, "error" | "warn">;
    addEventListener: Window["addEventListener"];
    removeEventListener?: Window["removeEventListener"];
    now?: () => number;
    setTimeout?: (fn: () => void, ms: number) => unknown;
    clearTimeout?: (handle: unknown) => void;
    /** Where the "one installation" mark lives. Overridable so tests get their own realm. */
    registry?: Registry;
  },
): UiLogging {
  const now = scope.now ?? (() => Date.now());
  const registry = scope.registry ?? (globalThis as unknown as Registry);
  const previous = registry[INSTALLED] as Installation | undefined;
  // Tear the old one down before wrapping again, or this installation's "original" console method
  // is the previous installation's wrapper and every line goes out twice.
  if (previous) previous.stop();
  const generation = (previous?.generation ?? 0) + 1;

  const collapser = makeRepeatCollapser();
  const batcher = makeBatcher(send, {
    setTimeout: scope.setTimeout ?? ((fn, ms) => setTimeout(fn, ms)),
    clearTimeout: scope.clearTimeout ?? ((handle) => clearTimeout(handle as ReturnType<typeof setTimeout>)),
  });
  const original = { error: scope.console.error, warn: scope.console.warn };

  const forward = (level: UiLogLevel, args: unknown[]) => {
    try {
      batcher.push(collapser.push(level, formatLogArgs(args), now()));
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

  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    scope.console.error = original.error;
    scope.console.warn = original.warn;
    // The half the previous version forgot. Restoring the console without this leaves two global
    // exception listeners behind per remount, which is the duplication nobody could account for.
    scope.removeEventListener?.("error", onError);
    scope.removeEventListener?.("unhandledrejection", onRejection);
    batcher.push(collapser.flush());
    batcher.flush();
    if ((registry[INSTALLED] as Installation | undefined)?.generation === generation) {
      delete registry[INSTALLED];
    }
  };

  registry[INSTALLED] = { generation, stop } satisfies Installation;

  return {
    stop,
    generation,
    get dropped() {
      return batcher.dropped;
    },
    flush: () => {
      batcher.push(collapser.flush());
      batcher.flush();
    },
  };
}

// The bootstrap capture that used to live here now lives in `startup-log.ts`. It was imported from
// there, which put this whole module ahead of the capture in the evaluation order and made a
// failure inside it invisible to the thing meant to observe startup failures.
