/**
 * Correlation across the IPC boundary.
 *
 * # What this is for
 *
 * A user-visible operation in this app crosses ten stages: a Svelte handler, a Tauri invoke, a
 * bridge command, an actor mailbox, sync or MLS or storage or network, an actor event, a Tauri
 * event, a frontend listener, state reconciliation, and a render. Until now nothing carried one
 * identifier through them, so "my message did not arrive" could not be narrowed to a stage, and
 * concurrent sends, reconnects, server switches and retries were indistinguishable from each other
 * in the record.
 *
 * A trace id is that identifier. It is allocated when a user does something, travels with the
 * invoke, comes back on the event the operation causes, and is attached to the decision the UI
 * makes about it.
 *
 * # What it costs
 *
 * Instrumentation that slows the app becomes a cause of the problems it exists to explain. So:
 * nothing here blocks, records are batched rather than sent one IPC call at a time, the queue is
 * bounded, and a failed send is counted rather than retried into a loop. The only per-call work on
 * the hot path is a counter increment and pushing a small object onto an array.
 */

import type { UiLogRecord } from "./uilog.ts";

/** One stage's worth of structured detail. Numbers and short enums only. */
export type SafeField = string | number | boolean;

/** A trace id, as the native side renders one: sixteen hex characters. */
export type TraceId = string;

/**
 * Allocate a trace.
 *
 * A counter plus a per-session random prefix, not a UUID. It only has to be unique within one
 * session and quotable in a bug report, and a counter is reproducible under test in a way a random
 * source is not. The prefix keeps two sessions' traces from looking like the same operation when
 * their reports are compared.
 */
export function makeTraceSource(prefix: number) {
  let next = 0;
  return {
    next(): TraceId {
      next += 1;
      return (BigInt(prefix >>> 0) * 0x100000000n + BigInt(next)).toString(16).padStart(16, "0");
    },
  };
}

/** The first four characters, which is how a trace is referred to in prose. */
export function shortTrace(trace: TraceId): string {
  return trace.slice(0, 4);
}

// --- event sequence gaps ---------------------------------------------------------------------

/** What a missing event looks like once it has been noticed. */
export type SeqAnomaly =
  | { kind: "gap"; event: string; expected: number; received: number; missed: number }
  | { kind: "repeat"; event: string; previous: number; received: number };

/**
 * Watch the `__seq` the native side stamps on every emitted event.
 *
 * The distinction this exists to make: an update that was coalesced and an update that was lost
 * look identical from the webview, and only one of them leaves the UI stale. A backend that
 * changed a channel while the frontend never heard about it is a wrong unread badge with no
 * evidence, and that is precisely the class of bug that has been hardest to pin down here.
 *
 * Per event name, because sequences are allocated per name. A frontend remount resets the tracker,
 * which is correct: it has not missed anything, it has simply not seen anything yet.
 */
export function makeSeqTracker() {
  const last = new Map<string, number>();
  return {
    observe(event: string, seq: unknown): SeqAnomaly | null {
      // A payload that is not a JSON object cannot carry a sequence. Those events are numbered
      // natively but not checkable here, and silence is the honest answer rather than a guess.
      if (typeof seq !== "number" || !Number.isFinite(seq)) return null;
      const previous = last.get(event);
      if (previous === undefined) {
        last.set(event, seq);
        return null;
      }
      if (seq <= previous) {
        // Delivered twice, or out of order. Not a gap, but not nothing either: a listener that
        // was installed twice looks exactly like this, and so does a retry that should not have
        // happened.
        return { kind: "repeat", event, previous, received: seq };
      }
      last.set(event, seq);
      if (seq === previous + 1) return null;
      return { kind: "gap", event, expected: previous + 1, received: seq, missed: seq - previous - 1 };
    },
    /** Forget everything, for a session change rather than a remount. */
    reset() {
      last.clear();
    },
  };
}

// --- recording -------------------------------------------------------------------------------

/** One structured observation from the webview. */
export type UiEvent = {
  /** The section it belongs to, as `catcoms-diagnostics` names them: `ui`, `ipc`, `channels`. */
  section: string;
  /** A stable `AREA.COMPONENT.OUTCOME` code. Never assembled from runtime data. */
  code: string;
  level: "error" | "warn" | "info" | "debug";
  trace?: TraceId;
  /** `start`, `success`, `failure`, `cancel`, `timeout`, or omitted for a bare observation. */
  phase?: string;
  duration_ms?: number;
  fields?: Record<string, SafeField>;
};

/** How many observations may wait before the oldest are dropped. */
export const MAX_PENDING = 500;

/**
 * Queue structured observations and send them in batches.
 *
 * Separate from `uilog`'s batcher because these are structured events rather than console lines,
 * and they go to a different native command. The shape is the same for the same reasons: an
 * unbounded queue converts backpressure into memory growth, and one IPC call per observation is
 * most expensive exactly when the app is busiest.
 */
export function makeRecorder(
  send: (events: UiEvent[]) => void,
  scope: {
    setTimeout: (fn: () => void, ms: number) => unknown;
    clearTimeout: (handle: unknown) => void;
    intervalMs?: number;
  },
) {
  const intervalMs = scope.intervalMs ?? 250;
  let queue: UiEvent[] = [];
  let dropped = 0;
  let timer: unknown = null;
  // Capture starts on, and is never persisted, so a fresh webview always begins beside a process
  // that is recording. The native side says otherwise only when somebody moves the control.
  let capturing = true;

  function flush() {
    if (timer !== null) {
      scope.clearTimeout(timer);
      timer = null;
    }
    if (!queue.length) return;
    const batch = queue;
    queue = [];
    try {
      send(batch);
    } catch {
      // A sink that is down must not break the app it observes, and must not be retried into a
      // loop that needs its own rate limit.
      dropped += batch.length;
    }
  }

  return {
    record(event: UiEvent) {
      // Off has to mean the webview stops paying, not that it keeps building observations and
      // sending them across the bridge for the native side to discard. The check is here rather
      // than at the call site so it is one decision in one place, and so it can be tested.
      if (!capturing) return;
      queue.push(event);
      if (queue.length > MAX_PENDING) {
        dropped += queue.length - MAX_PENDING;
        queue = queue.slice(queue.length - MAX_PENDING);
      }
      if (timer === null) timer = scope.setTimeout(flush, intervalMs);
    },
    /**
     * Follow the native capture setting.
     *
     * Turning it off discards whatever is queued rather than sending it: those are observations the
     * user has just said they do not want kept, and delivering them anyway would make the control
     * mean "stop soon". They are not counted as dropped either, because dropped means lost against
     * the user's wishes and this is the opposite.
     */
    setCapturing(on: boolean) {
      capturing = on;
      if (on) return;
      queue = [];
      if (timer !== null) {
        scope.clearTimeout(timer);
        timer = null;
      }
    },
    flush,
    get capturing() {
      return capturing;
    },
    get dropped() {
      return dropped;
    },
    get pending() {
      return queue.length;
    },
  };
}

// --- the invoke wrapper -----------------------------------------------------------------------

/** What an instrumented invoke records about itself. */
export type InvokeOutcome = {
  command: string;
  trace: TraceId;
  ok: boolean;
  duration_ms: number;
  /** A stable classification of the failure, never the error text. */
  failure?: string;
};

/**
 * Classify a rejected invoke without putting its message in the record.
 *
 * The message is a `Result<_, String>` from the bridge and can contain anything the failing layer
 * chose to interpolate, including a path or an address. It reaches the user's screen either way,
 * but a *diagnostic* record is a thing that gets exported, so it carries the shape of the failure
 * rather than its prose. Replaced wholesale once the bridge returns typed errors.
 */
export function classifyInvokeFailure(error: unknown): string {
  // A migrated command already answered this question properly, and its answer is stable in a way
  // that sniffing a sentence never is. Prefer it, and fall back only for commands still returning
  // prose.
  const view = describeError(error);
  if (view.code) return view.code;
  const text = view.message.toLowerCase();
  if (text.includes("locked") || text.includes("unlock")) return "session_locked";
  if (text.includes("no actor") || text.includes("actor stopped")) return "actor_unavailable";
  if (text.includes("not found") || text.includes("unknown server")) return "not_found";
  if (text.includes("permission") || text.includes("not allowed")) return "not_permitted";
  if (text.includes("timeout") || text.includes("timed out")) return "timeout";
  return "failed";
}

// --- typed errors ------------------------------------------------------------------------------

/** What the app can do about a failure, when there is something to do. */
export type Remediation = "unlock" | "check_connection" | "amend_input" | "retry" | "restart";

/** A failure, however the command chose to report it. */
export type ErrorView = {
  /** What the user sees. Always present, and always the same text they saw before the migration. */
  message: string;
  /** The stable code, when the command has been migrated. */
  code?: string;
  /** The trace, so a bug report can quote it and the log can be searched for it. */
  trace?: string;
  retryable?: boolean;
  remediation?: Remediation;
  details?: Record<string, string>;
};

/**
 * Read a rejected invoke, whichever shape it arrived in.
 *
 * This is what makes the error migration incremental. Most commands still reject with a bare
 * string, a few now reject with a typed object, and a call site using this behaves correctly
 * against both. So it can be adopted at a call site *before* that call site's command is migrated,
 * and a half-migrated bridge is indistinguishable from an unmigrated one.
 *
 * Without it, changing a command's error type would silently turn its message into
 * `[object Object]` on screen, which is a worse error report than the one it replaced.
 */
export function describeError(error: unknown): ErrorView {
  if (error && typeof error === "object" && !(error instanceof Error)) {
    const candidate = error as Record<string, unknown>;
    if (typeof candidate.message === "string" && typeof candidate.code === "string") {
      return {
        message: candidate.message,
        code: candidate.code,
        trace: typeof candidate.trace === "string" ? candidate.trace : undefined,
        retryable: typeof candidate.retryable === "boolean" ? candidate.retryable : undefined,
        remediation: typeof candidate.remediation === "string" ? (candidate.remediation as Remediation) : undefined,
        details:
          candidate.details && typeof candidate.details === "object"
            ? (candidate.details as Record<string, string>)
            : undefined,
      };
    }
  }
  return { message: String(error) };
}

/**
 * The message to show, with the diagnostic code appended when there is one.
 *
 * The code is shown rather than hidden because the alternative is a support conversation that
 * starts with "what did it say exactly". A short code a user can read out loud, or screenshot,
 * turns that into one round trip instead of three.
 */
export function errorText(error: unknown): string {
  const view = describeError(error);
  if (!view.code) return view.message;
  const trace = view.trace ? ` · ${view.trace}` : "";
  return `${view.message} (${view.code}${trace})`;
}

/**
 * Build an instrumented `invoke`.
 *
 * Records the frontend's own view of a command: when it started, how long it took, and whether it
 * came back. That is half of what makes a stall diagnosable, because the native record alone
 * cannot distinguish a command that was never sent from one that was sent and never answered.
 *
 * The trace is returned to the caller as well as recorded, so whatever the operation causes later
 * (an emitted event, a state change, a render decision) can be tied back to it.
 */
export function makeInvokeDebugged(
  invoke: <T>(command: string, args?: Record<string, unknown>) => Promise<T>,
  record: (event: UiEvent) => void,
  traces: { next: () => TraceId },
  now: () => number = () => Date.now(),
) {
  return async function invokeDebugged<T>(
    command: string,
    args: Record<string, unknown> = {},
    options: { trace?: TraceId; fields?: Record<string, SafeField> } = {},
  ): Promise<{ value: T; trace: TraceId }> {
    const trace = options.trace ?? traces.next();
    const started = now();
    record({
      section: "ipc",
      code: "IPC.COMMAND.STARTED",
      level: "debug",
      trace,
      phase: "start",
      fields: { command, ...options.fields },
    });
    try {
      // The trace travels with the call, so the native side can stamp its own stages with it.
      // Commands that have not been migrated ignore the extra argument.
      const value = await invoke<T>(command, { ...args, trace });
      record({
        section: "ipc",
        code: "IPC.COMMAND.COMPLETED",
        level: "debug",
        trace,
        phase: "success",
        duration_ms: now() - started,
        fields: { command },
      });
      return { value, trace };
    } catch (error) {
      record({
        section: "ipc",
        code: "IPC.COMMAND.FAILED",
        level: "warn",
        trace,
        phase: "failure",
        duration_ms: now() - started,
        fields: { command, failure: classifyInvokeFailure(error) },
      });
      throw error;
    }
  };
}

/** Console lines and structured events share one shape on the wire; this is the console half. */
export type { UiLogRecord };
