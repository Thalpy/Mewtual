import assert from "node:assert/strict";
import test from "node:test";
import { formatLogArgs, installUiLogging, makeDeduper, MAX_UI_LOG_CHARS } from "./uilog.ts";

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

test("identical consecutive lines collapse inside the window", () => {
  const allow = makeDeduper(2000);
  assert.equal(allow("same", 0), true);
  assert.equal(allow("same", 500), false);
  assert.equal(allow("same", 1999), false);
  // Past the window it is news again: a loop that is still going should say so periodically.
  assert.equal(allow("same", 2001), true);
});

test("a different line always passes, so dedupe cannot hide a new problem", () => {
  const allow = makeDeduper(2000);
  assert.equal(allow("first", 0), true);
  assert.equal(allow("second", 1), true);
  assert.equal(allow("first", 2), true);
});

/** A console double that records what the original would have printed. */
function harness() {
  const printed: string[] = [];
  const sent: Array<[string, string]> = [];
  const listeners: Record<string, (e: Event) => void> = {};
  const console = {
    error: (...a: unknown[]) => printed.push(`error:${a.join(" ")}`),
    warn: (...a: unknown[]) => printed.push(`warn:${a.join(" ")}`),
  };
  const restore = installUiLogging(
    (level, message) => sent.push([level, message]),
    {
      console: console as unknown as Pick<Console, "error" | "warn">,
      addEventListener: ((type: string, fn: (e: Event) => void) => {
        listeners[type] = fn;
      }) as unknown as Window["addEventListener"],
      now: () => sent.length * 5000, // always outside the dedupe window
    },
  );
  return { printed, sent, listeners, console, restore };
}

test("console output still reaches devtools as well as the log", () => {
  const h = harness();
  h.console.warn("voice signal had no member route");
  assert.deepEqual(h.printed, ["warn:voice signal had no member route"]);
  assert.deepEqual(h.sent, [["warn", "voice signal had no member route"]]);
  h.restore();
});

test("errors and warnings carry their own level through", () => {
  const h = harness();
  h.console.error("ICE failed");
  h.console.warn("retrying");
  assert.deepEqual(
    h.sent.map(([level]) => level),
    ["error", "warn"],
  );
  h.restore();
});

test("uncaught exceptions and unhandled rejections are captured", () => {
  const h = harness();
  h.listeners.error?.({ message: "x is not a function", filename: "App.svelte", lineno: 42 } as unknown as Event);
  h.listeners.unhandledrejection?.({ reason: new Error("nope") } as unknown as Event);
  assert.equal(h.sent.length, 2);
  assert.ok(h.sent[0][1].includes("uncaught: x is not a function at App.svelte:42"));
  assert.ok(h.sent[1][1].includes("unhandled rejection:"));
  assert.ok(h.sent[1][1].includes("Error: nope"));
  h.restore();
});

test("restoring puts the original console back", () => {
  const h = harness();
  const patched = h.console.warn;
  h.restore();
  assert.notEqual(h.console.warn, patched, "the patched method must not survive restore");
  h.console.warn("after");
  assert.equal(h.sent.length, 0, "nothing is forwarded once restored");
});

test("a sink that throws cannot break the console it wraps", () => {
  const printed: string[] = [];
  const console = {
    error: (...a: unknown[]) => printed.push(a.join(" ")),
    warn: () => {},
  };
  installUiLogging(
    () => {
      throw new Error("sink is down");
    },
    {
      console: console as unknown as Pick<Console, "error" | "warn">,
      addEventListener: (() => {}) as unknown as Window["addEventListener"],
    },
  );
  assert.doesNotThrow(() => console.error("still prints"));
  assert.deepEqual(printed, ["still prints"]);
});
