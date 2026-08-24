import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  RAW_DETAIL_WARNING,
  beginStartupCapture,
  describeStartupError,
  diagnosticId,
  drainStartupLog,
  endStartupCapture,
  redactStartupText,
  renderStartupFailure,
  startupFailureDetail,
  startupFailureSummary,
  webviewFamily,
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

// --- the ordering the whole module depends on ----------------------------------------------------

/**
 * The guard for the half of this that is about evaluation order rather than content.
 *
 * ES modules evaluate every static import before the body of the importing module, so
 * `beginStartupCapture()` being the first *statement* of `main.ts` still left `svelte`, `app.css`
 * and `uilog.ts` running ahead of it. A module that threw while being evaluated threw into a window
 * with no handler on it, which is the blank window with nothing attached that this file exists for.
 *
 * Prose cannot hold that: the rule is broken by adding one ordinary-looking import line. So the two
 * source files are read and their static imports counted.
 */
test("nothing is evaluated ahead of startup capture", () => {
  const read = (name: string) => readFileSync(new URL(name, import.meta.url), "utf8");
  // Static imports only. `import type` is erased before it runs, and a dynamic `import(...)` is a
  // call in the body rather than a dependency evaluated ahead of it.
  const staticImports = (source: string) =>
    [...source.matchAll(/^import\s+(?!type\b)(?:[^;]*?\sfrom\s+)?["']([^"']+)["']/gm)].map((m) => m[1]);

  assert.deepEqual(
    staticImports(read("./startup-log.ts")),
    [],
    "startup-log.ts must import nothing at runtime, or the module it imports gets to fail first",
  );
  assert.deepEqual(
    staticImports(read("./main.ts")),
    ["./startup-log"],
    "main.ts must import only the capture, and load everything else with a dynamic import inside start()",
  );
});

// --- capture -------------------------------------------------------------------------------------

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
    limit: 3,
  });

  const fire = (type: string, e: unknown) => {
    for (const l of listeners) if (l.type === type) l.fn(e as Event);
  };
  fire("error", { message: "App.svelte failed to evaluate", filename: "App.svelte", lineno: 1 });
  fire("unhandledrejection", { reason: new Error("dynamic import failed") });

  const held = drainStartupLog();
  assert.equal(held.length, 2);
  assert.ok(held[0].message.includes("App.svelte failed to evaluate"));
  assert.ok(held[1].message.includes("dynamic import failed"));
  assert.deepEqual(drainStartupLog(), [], "handed over exactly once");

  // Bounded: a startup failure is often a loop, and there is no rate limiter yet at that point.
  for (let i = 0; i < 10; i += 1) fire("error", { message: `loop ${i}` });
  assert.equal(drainStartupLog().length, 3);

  endStartupCapture();
  assert.equal(listeners.length, 0, "and it stops watching when the real logger takes over");
});

test("one enormous failure cannot fill the buffer meant for fifty", () => {
  const listeners: Array<{ type: string; fn: (e: Event) => void }> = [];
  beginStartupCapture({
    addEventListener: ((type: string, fn: (e: Event) => void) => {
      listeners.push({ type, fn });
    }) as unknown as Window["addEventListener"],
    removeEventListener: (() => {}) as unknown as Window["removeEventListener"],
  });
  for (const l of listeners) {
    if (l.type === "error") l.fn({ message: "x".repeat(50_000), filename: "App.svelte", lineno: 1 } as unknown as Event);
  }
  const [record] = drainStartupLog();
  assert.ok(record.message.length < 3000, "these records go to the same native sink as every other line");
  assert.ok(record.message.endsWith("[truncated]"), "and the cut says it happened");
  endStartupCapture();
});

// --- redaction -----------------------------------------------------------------------------------

test("an install path is reduced to the file that failed", () => {
  const redacted = redactStartupText(
    "at mount (C:\\Users\\marisa\\AppData\\Local\\Mewtual\\assets\\App-9f2c1a4b.js:31:7)",
  );
  assert.ok(!redacted.includes("marisa"), "a Windows install path starts with the account name");
  assert.ok(!redacted.includes("AppData"));
  assert.ok(redacted.includes("App-[id].js"), "the module that failed is the diagnosis and must survive");
});

test("a dev-server URL loses its host and query but keeps the module name", () => {
  const redacted = redactStartupText(
    "Failed to fetch dynamically imported module: http://localhost:1420/src/App.svelte?t=1738000000000",
  );
  assert.ok(!redacted.includes("localhost:1420"), "the host and port of a dev server locate the machine");
  assert.ok(!redacted.includes("1738000000000"), "a cache-busting stamp is still a timestamp for this machine");
  assert.ok(redacted.includes("App.svelte"), "the module that failed to fetch is the diagnosis");
});

test("a POSIX home directory goes, and ordinary prose with a slash stays", () => {
  const home = redactStartupText("at /home/marisa/mewtual/assets/index.js:1");
  assert.ok(!home.includes("marisa"), "a POSIX home directory is named after its owner");
  assert.equal(redactStartupText("read/write failed"), "read/write failed", "one slash is not a path");
});

test("peer ids and fingerprints are masked, because a startup line can carry one", () => {
  const redacted = redactStartupText("session 2b5df389aa11 for 12D3KooWSaXFXMFgkGxgBF6UPEojspeSj2KaDiP4ks5poLzieKKN");
  assert.ok(!redacted.includes("12D3KooW"), "a peer id names this device to anyone who has seen it dial");
  assert.ok(!redacted.includes("2b5df389aa11"), "and a session fingerprint correlates two reports");
});

test("the user agent is reduced to the engine, which is the part that explains a startup failure", () => {
  assert.equal(webviewFamily("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/131.0.2903.86"), "WebView2");
  assert.equal(webviewFamily("Mozilla/5.0 (Macintosh) AppleWebKit/605.1.15 Version/17.0 Safari/605.1.15"), "WebKit");
  assert.equal(webviewFamily("something else entirely"), "unknown webview");
});

// --- the two reports -----------------------------------------------------------------------------

test("the report carries the id, the failure and anything captured before it", () => {
  const report = startupFailureSummary(
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
  const report = startupFailureSummary("sf-000001", new Error("boom"), [], "ua");
  assert.ok(!report.includes("earlier startup errors:"));
});

/**
 * The half of this finding that was a false promise. The screen told the user the report contained
 * no names while printing a stack whose every frame is an absolute path, and on Windows that path
 * opens with the account name. Nothing native is reachable to validate an export at this point, so
 * the summary has to be redacted before it is offered rather than checked afterwards.
 */
test("the default report redacts the stack and the buffered records, not just the stack", () => {
  const error = new Error("Failed to fetch dynamically imported module");
  error.stack = "Error: boom\n    at start (C:\\Users\\marisa\\Mewtual\\assets\\index-77aa11bc.js:2:9)";
  const summary = startupFailureSummary(
    "sf-01b2c3",
    error,
    [{ level: "error", message: "startup uncaught: bad at http://localhost:1420/src/uilog.ts:4", repeats: 1 }],
    "Mozilla/5.0 (Windows NT 10.0.26200; Win64) AppleWebKit/537.36 Edg/131.0.2903.86",
  );
  assert.ok(!summary.includes("marisa"), "the install path is the account name");
  assert.ok(!summary.includes("localhost:1420"), "a buffered record carries the filename of its error event");
  assert.ok(!summary.includes("10.0.26200"), "an OS build narrows a user down and explains nothing");
  assert.ok(summary.includes("index-[id].js") && summary.includes("uilog.ts"), "the failing modules still read");
});

test("the raw report keeps what the summary removed, so asking for it is worth something", () => {
  const error = new Error("boom");
  error.stack = "Error: boom\n    at start (C:\\Users\\marisa\\Mewtual\\assets\\index-77aa11bc.js:2:9)";
  const detail = startupFailureDetail("sf-01b2c3", error, [], "Mozilla/5.0 (Windows NT 10.0.26200) Edg/131.0.2903.86");
  assert.ok(detail.includes("C:\\Users\\marisa\\Mewtual\\assets\\index-77aa11bc.js"));
  assert.ok(detail.includes("10.0.26200"), "the exact build is sometimes the whole answer");
});

// --- the screen ----------------------------------------------------------------------------------

test("the failure screen shows the id and the report, and leaves them selectable", () => {
  const target = document.createElement("div");
  target.textContent = "the application that never arrived";
  renderStartupFailure(target, "sf-01b2c3", "Mewtual failed to start\ndiagnostic: sf-01b2c3\nboom", "raw boom");

  assert.ok(!target.textContent?.includes("never arrived"), "the broken app is replaced, not decorated");
  assert.equal(target.querySelector("[role=alert]") !== null, true);
  const pre = target.querySelector("pre") as HTMLElement | null;
  assert.ok(pre?.textContent?.includes("boom"));
  assert.equal(pre?.style.userSelect, "text", "the copy button may itself be the broken part");
  assert.ok(target.textContent?.includes("sf-01b2c3"));
});

test("the screen never uses an application class, because the stylesheet may be what failed", () => {
  const target = document.createElement("div");
  renderStartupFailure(target, "sf-01b2c3", "summary", "raw");
  for (const node of target.querySelectorAll("*")) {
    assert.equal(node.className, "", `${node.tagName} would be unstyled if app.css is the module that failed`);
  }
});

/**
 * The raw stack is offered rather than withheld, but sending it has to be a decision. It sits in a
 * closed disclosure with a warning, and the screen no longer prints the sentence claiming the text
 * on it contains no names.
 */
test("raw detail is behind a closed disclosure that says what it contains", () => {
  const target = document.createElement("div");
  renderStartupFailure(target, "sf-01b2c3", "redacted summary", "at C:\\Users\\marisa\\Mewtual\\index.js");

  const raw = target.querySelector("details") as HTMLDetailsElement | null;
  assert.ok(raw, "the raw report needs somewhere to be asked for");
  assert.equal(raw?.open, false, "shown by default is the same as not asking");
  assert.ok(raw?.textContent?.includes(RAW_DETAIL_WARNING), "and it says what it is before it is copied");
  assert.ok(raw?.querySelector("pre")?.textContent?.includes("marisa"), "raw means raw");

  // The exact sentence that was wrong, and the shapes it would come back as.
  const onScreen = target.textContent ?? "";
  assert.ok(
    !/contains no|no names|key material|safe to send/i.test(onScreen),
    "the screen must not promise a property nothing here can check",
  );
});

test("both reports have their own copy button, so the redacted one is what an ordinary click sends", () => {
  const target = document.createElement("div");
  const copied: string[] = [];
  Object.defineProperty(globalThis, "navigator", {
    value: { clipboard: { writeText: (t: string) => (copied.push(t), Promise.resolve()) } },
    configurable: true,
  });
  renderStartupFailure(target, "sf-01b2c3", "redacted summary", "raw detail");

  const buttons = [...target.querySelectorAll("button")] as HTMLButtonElement[];
  assert.equal(buttons.length, 2);
  const outside = buttons.find((b) => !b.closest("details"));
  outside?.onclick?.(new Event("click") as MouseEvent);
  assert.deepEqual(copied, ["redacted summary"], "the button in reach copies the redacted text");
});
