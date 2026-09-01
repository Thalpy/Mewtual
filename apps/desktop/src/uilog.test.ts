import assert from "node:assert/strict";
import test from "node:test";
import {
  formatLogArgs,
  installUiLogging,
  makeBatcher,
  makeRepeatCollapser,
  uiLoggingState,
  DEBUG_LOG_FILE_DISCLOSURE,
  MAX_QUEUE,
  MAX_UI_LOG_CHARS,
  type UiLogRecord,
} from "./uilog.ts";

test("the raw-file disclosure names arbitrary prose instead of promising content exclusion", () => {
  assert.match(DEBUG_LOG_FILE_DISCLOSURE, /not filtered through Safe diagnostics/i);
  assert.match(DEBUG_LOG_FILE_DISCLOSURE, /frontend console\/error/);
  assert.match(DEBUG_LOG_FILE_DISCLOSURE, /native app\/tracing/);
  assert.match(DEBUG_LOG_FILE_DISCLOSURE, /message fragments/);
  assert.match(DEBUG_LOG_FILE_DISCLOSURE, /paths/);
  assert.match(DEBUG_LOG_FILE_DISCLOSURE, /tokens/);
  assert.match(DEBUG_LOG_FILE_DISCLOSURE, /Review the actual file before sharing it/);
  assert.doesNotMatch(
    DEBUG_LOG_FILE_DISCLOSURE,
    /does not contain|never contains|content[- ]free|safe to share/i,
  );
});

test("strings pass through and join", () => {
  assert.equal(formatLogArgs(["voice signal failed", "peer a1b2"]), "voice signal failed peer a1b2");
});

test("an Error keeps its stack, which is the whole point of logging it", () => {
  const err = new Error("boom");
  const line = formatLogArgs([err]);
  assert.ok(line.includes("Error: boom"));
  assert.ok(line.includes("uilog.test"), "the frames must survive");
});

test("objects are serialised rather than stringified to [object Object]", () => {
  assert.equal(formatLogArgs([{ peer: "a1", code: 701 }]), '{"peer":"a1","code":701}');
});

test("a cyclic object does not throw and still reports its type", () => {
  const cyclic: Record<string, unknown> = { a: 1 };
  cyclic.self = cyclic;
  assert.equal(formatLogArgs([cyclic]), "[object Object]");
});

test("an object with a throwing getter does not take the logger down with it", () => {
  const hostile = {
    get boom() {
      throw new Error("nope");
    },
  };
  assert.doesNotThrow(() => formatLogArgs([hostile]));
});

test("a very long line is truncated rather than shipped whole", () => {
  const line = formatLogArgs(["x".repeat(MAX_UI_LOG_CHARS * 2)]);
  assert.ok(line.length < MAX_UI_LOG_CHARS + 40);
  assert.ok(line.endsWith("[truncated]"));
});

// --- repeat aggregation ---------------------------------------------------------------------

test("the first of a run goes out immediately, because a first error is news", () => {
  const c = makeRepeatCollapser(2000);
  assert.deepEqual(c.push("warn", "ICE candidate rejected", 0), [
    { level: "warn", message: "ICE candidate rejected", repeats: 1 },
  ]);
});

test("repeats are counted rather than deleted, so a storm stays visible as a storm", () => {
  const c = makeRepeatCollapser(2000);
  c.push("warn", "ICE candidate rejected", 0);
  for (let i = 1; i < 312; i += 1) assert.deepEqual(c.push("warn", "ICE candidate rejected", i), []);
  // The run settles when something else happens. The count follows the line it belongs to.
  const out = c.push("warn", "something else", 1500);
  assert.deepEqual(out, [
    { level: "warn", message: "ICE candidate rejected", repeats: 311 },
    { level: "warn", message: "something else", repeats: 1 },
  ]);
});

test("a run still open at shutdown is settled rather than lost", () => {
  const c = makeRepeatCollapser(2000);
  c.push("error", "boom", 0);
  c.push("error", "boom", 1);
  c.push("error", "boom", 2);
  assert.deepEqual(c.flush(), [{ level: "error", message: "boom", repeats: 2 }]);
  assert.deepEqual(c.flush(), [], "settling twice does not invent a second summary");
});

test("past the window the line is news again, so a continuing loop keeps saying so", () => {
  const c = makeRepeatCollapser(2000);
  c.push("warn", "same", 0);
  c.push("warn", "same", 500);
  const out = c.push("warn", "same", 2001);
  assert.deepEqual(out, [
    { level: "warn", message: "same", repeats: 1 },
    { level: "warn", message: "same", repeats: 1 },
  ]);
});

test("a different line always passes, so collapsing cannot hide a new problem", () => {
  const c = makeRepeatCollapser(2000);
  assert.equal(c.push("warn", "first", 0).length, 1);
  assert.equal(c.push("warn", "second", 1).length, 1);
  assert.equal(c.push("warn", "first", 2).length, 1);
});

test("the same text at a different level is a different line", () => {
  const c = makeRepeatCollapser(2000);
  c.push("warn", "transport closed", 0);
  assert.deepEqual(c.push("error", "transport closed", 1), [
    { level: "error", message: "transport closed", repeats: 1 },
  ]);
});

// --- batching -------------------------------------------------------------------------------

/** A controllable timer, so batching is tested by wall-clock decisions rather than by waiting. */
function timers() {
  const scheduled = new Map<number, { fn: () => void; delay: number }>();
  let next = 1;
  return {
    scope: {
      setTimeout: (fn: () => void, delay: number) => {
        const id = next++;
        scheduled.set(id, { fn, delay });
        return id;
      },
      clearTimeout: (handle: unknown) => {
        scheduled.delete(handle as number);
      },
    },
    get delays() {
      return [...scheduled.values()].map((s) => s.delay);
    },
    run() {
      const due = [...scheduled.values()];
      scheduled.clear();
      for (const s of due) s.fn();
    },
  };
}

test("records wait for the interval and then go together", () => {
  const t = timers();
  const sent: UiLogRecord[][] = [];
  const b = makeBatcher((r) => sent.push(r), t.scope);
  b.push([{ level: "warn", message: "a", repeats: 1 }]);
  b.push([{ level: "warn", message: "b", repeats: 1 }]);
  assert.deepEqual(sent, [], "nothing has left yet");
  t.run();
  assert.equal(sent.length, 1, "one call, not one per record");
  assert.deepEqual(sent[0].map((r) => r.message), ["a", "b"]);
});

test("an error does not sit in a timer while the page may be dying", () => {
  const t = timers();
  const b = makeBatcher(() => {}, t.scope);
  b.push([{ level: "warn", message: "a", repeats: 1 }]);
  assert.deepEqual(t.delays, [250]);
  b.push([{ level: "error", message: "boom", repeats: 1 }]);
  assert.deepEqual(t.delays, [0], "the error pulls the flush forward");
});

test("a full queue drops the oldest and counts it, never silently", () => {
  const t = timers();
  const b = makeBatcher(() => {}, t.scope);
  const flood = Array.from({ length: MAX_QUEUE + 50 }, (_, i) => ({
    level: "warn" as const,
    message: `line ${i}`,
    repeats: 1,
  }));
  b.push(flood);
  assert.equal(b.dropped, 50);
  assert.equal(b.pending, MAX_QUEUE);
});

test("a sink that throws is counted as loss rather than retried into a loop", () => {
  const t = timers();
  const b = makeBatcher(() => {
    throw new Error("bridge is down");
  }, t.scope);
  b.push([{ level: "warn", message: "a", repeats: 1 }]);
  t.run();
  assert.equal(b.dropped, 1);
  assert.equal(b.pending, 0, "the queue drains rather than growing forever");
});

// --- installation lifecycle -----------------------------------------------------------------

/** A console double plus its own realm, so the installation guard is exercised in isolation. */
function harness(registry: Record<symbol, unknown> = {}) {
  const printed: string[] = [];
  const sent: UiLogRecord[] = [];
  const listeners: Array<{ type: string; fn: (e: Event) => void }> = [];
  let clock = 0;
  const console = {
    error: (...a: unknown[]) => printed.push(`error:${a.join(" ")}`),
    warn: (...a: unknown[]) => printed.push(`warn:${a.join(" ")}`),
  };
  const t = timers();
  const install = () =>
    installUiLogging((records) => sent.push(...records), {
      console: console as unknown as Pick<Console, "error" | "warn">,
      addEventListener: ((type: string, fn: (e: Event) => void) => {
        listeners.push({ type, fn });
      }) as unknown as Window["addEventListener"],
      removeEventListener: ((type: string, fn: (e: Event) => void) => {
        const at = listeners.findIndex((l) => l.type === type && l.fn === fn);
        if (at >= 0) listeners.splice(at, 1);
      }) as unknown as Window["removeEventListener"],
      // Always outside the repeat window, so lifecycle tests count installations rather than runs.
      now: () => (clock += 5000),
      ...t.scope,
      registry,
    });
  const fire = (type: string, event: unknown) => {
    for (const l of [...listeners]) if (l.type === type) l.fn(event as Event);
  };
  return { printed, sent, listeners, console, install, fire, timers: t, registry };
}

test("console output still reaches devtools as well as the log", () => {
  const h = harness();
  const log = h.install();
  h.console.warn("voice signal had no member route");
  log.flush();
  assert.deepEqual(h.printed, ["warn:voice signal had no member route"]);
  assert.deepEqual(h.sent, [{ level: "warn", message: "voice signal had no member route", repeats: 1 }]);
  log.stop();
});

test("errors and warnings carry their own level through", () => {
  const h = harness();
  const log = h.install();
  h.console.error("ICE failed");
  h.console.warn("retrying");
  log.flush();
  assert.deepEqual(h.sent.map((r) => r.level), ["error", "warn"]);
  log.stop();
});

test("uncaught exceptions and unhandled rejections are captured", () => {
  const h = harness();
  const log = h.install();
  h.fire("error", { message: "x is not a function", filename: "App.svelte", lineno: 42 });
  h.fire("unhandledrejection", { reason: new Error("nope") });
  log.flush();
  assert.equal(h.sent.length, 2);
  assert.ok(h.sent[0].message.includes("uncaught: x is not a function at App.svelte:42"));
  assert.ok(h.sent[1].message.includes("unhandled rejection:"));
  assert.ok(h.sent[1].message.includes("Error: nope"));
  log.stop();
});

test("stopping restores the console and removes both window listeners", () => {
  const h = harness();
  const log = h.install();
  const patched = h.console.warn;
  assert.equal(h.listeners.length, 2, "error and unhandledrejection");
  log.stop();
  assert.notEqual(h.console.warn, patched, "the patched method must not survive stop");
  assert.equal(h.listeners.length, 0, "and neither may the listeners");

  h.console.warn("after");
  h.fire("error", { message: "after" });
  log.flush();
  assert.deepEqual(h.sent, [], "nothing is forwarded once stopped");
});

/**
 * The regression this file exists for. The app supports F5 and HMR remounts while the native
 * process stays alive; the previous version leaked a listener pair on each one and layered another
 * console wrapper over the last, so one exception arrived N times on the Nth remount. A retry loop
 * then looked like it was accelerating when it was the logger multiplying.
 */
test("ten remounts still produce exactly one record per event", () => {
  const h = harness();
  let log = h.install();
  for (let i = 0; i < 9; i += 1) log = h.install();

  assert.equal(h.listeners.length, 2, "one live pair, however many installations happened");
  assert.equal(log.generation, 10, "and the count is visible rather than guessed at");

  h.console.error("boom");
  h.fire("error", { message: "uncaught boom" });
  h.fire("unhandledrejection", { reason: "rejected" });
  log.flush();

  assert.equal(h.sent.length, 3, `one per source event, got ${h.sent.map((r) => r.message).join(" | ")}`);
  log.stop();
});

test("stopping the current installation clears the realm mark", () => {
  const registry: Record<symbol, unknown> = {};
  const h = harness(registry);
  assert.deepEqual(uiLoggingState(registry), { installed: false, generation: 0 });
  const log = h.install();
  assert.deepEqual(uiLoggingState(registry), { installed: true, generation: 1 });
  log.stop();
  assert.deepEqual(uiLoggingState(registry), { installed: false, generation: 0 });
});

test("stopping a superseded installation does not disarm the live one", () => {
  const registry: Record<symbol, unknown> = {};
  const h = harness(registry);
  const first = h.install();
  const second = h.install();
  first.stop();
  assert.deepEqual(uiLoggingState(registry), { installed: true, generation: 2 });
  h.console.warn("still watching");
  second.flush();
  assert.equal(h.sent.length, 1);
});

test("a sink that throws cannot break the console it wraps", () => {
  const printed: string[] = [];
  const console = {
    error: (...a: unknown[]) => printed.push(a.join(" ")),
    warn: () => {},
  };
  const log = installUiLogging(
    () => {
      throw new Error("sink is down");
    },
    {
      console: console as unknown as Pick<Console, "error" | "warn">,
      addEventListener: (() => {}) as unknown as Window["addEventListener"],
      registry: {},
    },
  );
  assert.doesNotThrow(() => console.error("still prints"));
  assert.deepEqual(printed, ["still prints"]);
  log.stop();
  assert.ok(log.dropped >= 1, "and the loss is counted");
});

// Bootstrap capture moved to `startup-log.ts`, and its tests moved with it: importing it from here
// was the reason startup capture could not be installed before this module was evaluated.
