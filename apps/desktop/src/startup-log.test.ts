import assert from "node:assert/strict";
import test from "node:test";
import {
  beginStartupCapture,
  describeStartupError,
  diagnosticId,
  drainStartupLog,
  endStartupCapture,
  renderStartupFailure,
  startupFailureReport,
} from "./startup-log.ts";

test("a diagnostic id is short enough to read off a screenshot", () => {
  const id = diagnosticId(1_700_000_123_456);
  assert.match(id, /^sf-[0-9a-f]{6}$/);
});

test("two failures a moment apart are distinguishable", () => {
  assert.notEqual(diagnosticId(1_700_000_000_000), diagnosticId(1_700_000_000_001));
});

test("an Error keeps its frames, which is what names the module that failed to load", () => {
  const described = describeStartupError(new Error("Failed to fetch dynamically imported module"));
  assert.ok(described.includes("Error: Failed to fetch dynamically imported module"));
  assert.ok(described.includes("startup-log.test"));
});

test("anything else thrown is still described rather than becoming [object Object]", () => {
  assert.equal(describeStartupError("plain string"), "plain string");
  assert.equal(describeStartupError({ code: 12 }), '{"code":12}');
  const cyclic: Record<string, unknown> = {};
  cyclic.self = cyclic;
  assert.equal(describeStartupError(cyclic), "[object Object]");
});

test("the report carries the id, the failure and anything captured before it", () => {
  const report = startupFailureReport(
    "sf-01b2c3",
    new Error("mount failed"),
    [
      { level: "error", message: "startup uncaught: x is not a function at App.svelte:1", repeats: 1 },
    ],
    "Mozilla/5.0 (Windows NT 10.0) WebView2",
  );
  assert.ok(report.includes("diagnostic: sf-01b2c3"));
  assert.ok(report.includes("WebView2"), "the webview family is half of a startup diagnosis");
  assert.ok(report.includes("Error: mount failed"));
  assert.ok(report.includes("earlier startup errors:"));
  assert.ok(report.includes("x is not a function"));
});

test("a report with nothing captured does not claim an empty earlier-errors section", () => {
  const report = startupFailureReport("sf-000001", new Error("boom"), [], "ua");
  assert.ok(!report.includes("earlier startup errors:"));
});

/**
 * The window this module exists for. Capture has to be live before any application module is
 * imported, because a module that throws while evaluating never reaches the code that would have
 * installed a logger.
 */
test("errors thrown before the application exists are captured and handed over once", () => {
  const listeners: Array<{ type: string; fn: (e: Event) => void }> = [];
  beginStartupCapture({
    addEventListener: ((type: string, fn: (e: Event) => void) => {
      listeners.push({ type, fn });
    }) as unknown as Window["addEventListener"],
    removeEventListener: ((type: string, fn: (e: Event) => void) => {
      const at = listeners.findIndex((l) => l.type === type && l.fn === fn);
      if (at >= 0) listeners.splice(at, 1);
    }) as unknown as Window["removeEventListener"],
  });

  const fire = (type: string, e: unknown) => {
    for (const l of listeners) if (l.type === type) l.fn(e as Event);
  };
  fire("error", { message: "App.svelte failed to evaluate", filename: "App.svelte", lineno: 1 });

  const held = drainStartupLog();
  assert.equal(held.length, 1);
  assert.ok(held[0].message.includes("App.svelte failed to evaluate"));
  assert.deepEqual(drainStartupLog(), [], "handed over exactly once");

  endStartupCapture();
  assert.equal(listeners.length, 0, "and it stops watching when the real logger takes over");
});

test("the failure screen shows the id and the report, and leaves them selectable", () => {
  const target = document.createElement("div");
  target.textContent = "the application that never arrived";
  renderStartupFailure(target, "sf-01b2c3", "Mewtual failed to start\ndiagnostic: sf-01b2c3\nboom");

  assert.ok(!target.textContent?.includes("never arrived"), "the broken app is replaced, not decorated");
  assert.equal(target.querySelector("[role=alert]") !== null, true);
  const pre = target.querySelector("pre") as HTMLElement | null;
  assert.ok(pre?.textContent?.includes("boom"));
  assert.equal(pre?.style.userSelect, "text", "the copy button may itself be the broken part");
  assert.ok(target.textContent?.includes("sf-01b2c3"));
});
