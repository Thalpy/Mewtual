// Browser flow checks for chat, pending-message recovery and in-band friend requests. They drive the REAL Svelte app
// (the visual fixture build) in headless Edge over plain CDP: no automation framework, the
// same stance as the screenshot tooling. The fixture's deterministic data stays untouched;
// each scenario patches window.__TAURI_INTERNALS__.invoke at runtime to stand in for the
// native commands the fixture deliberately leaves unimplemented (send_message, join_server).
//
// What these catch: a frontend regression that wedges the composer (the `sending` flag never
// clearing, `cur.active` never being set after a switch), or one that breaks the accept flow
// (the request row not rendering, join_server never invoked, the new DM not landing in the
// rail). What they cannot catch: native-side failures; a hang inside the real send_message or
// join_server looks identical to the user but lives below this seam.
//
// Usage: node scripts/flow-check.mjs   (from apps/desktop; starts its own vite on FLOW_PORT
// or 5177, so a dev server you already have running on 5173 is left alone)

import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import process from "node:process";

const PORT = Number(process.env.FLOW_PORT ?? 5177);
const URL_UNDER_TEST = `http://localhost:${PORT}/?fixture=chat`;
const CDP_PORT = Number(process.env.FLOW_CDP_PORT ?? 9341);

const EDGE_CANDIDATES = [
  process.env.EDGE_PATH,
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
].filter(Boolean);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const isWindows = process.platform === "win32";

// vite runs behind a shell, so child.kill() reaps only the cmd.exe wrapper: the node process
// holding the port and the esbuild helper it started both survive as orphans. Those orphans
// keep node_modules/@esbuild/*/esbuild.exe open, which makes the next npm install abort
// mid-unlink with EPERM and leaves a node_modules with no .bin directory at all.
function killTree(pid) {
  if (!pid) return;
  if (isWindows) {
    spawnSync("taskkill", ["/F", "/T", "/PID", String(pid)], { stdio: "ignore", windowsHide: true });
    return;
  }
  try {
    process.kill(pid, "SIGKILL");
  } catch {
    /* already gone */
  }
}

// Backstop for a vite that outlives its wrapper anyway. Only ever called once our own server
// has answered on this port, so it cannot take down a stranger that happened to hold it.
function killPortListener(port) {
  if (!isWindows) {
    const found = spawnSync("lsof", ["-ti", `tcp:${port}`], { encoding: "utf8" });
    for (const pid of (found.stdout ?? "").split("\n").filter(Boolean)) killTree(Number(pid));
    return;
  }
  const found = spawnSync("netstat", ["-ano", "-p", "TCP"], { encoding: "utf8", windowsHide: true });
  const pids = new Set();
  for (const line of (found.stdout ?? "").split("\n")) {
    const m = line.match(/:(\d+)\s+\S+\s+LISTENING\s+(\d+)/i);
    if (m && Number(m[1]) === port) pids.add(Number(m[2]));
  }
  for (const pid of pids) killTree(pid);
}

async function findEdge() {
  const { access } = await import("node:fs/promises");
  for (const candidate of EDGE_CANDIDATES) {
    try {
      await access(candidate);
      return candidate;
    } catch {
      /* try the next install location */
    }
  }
  throw new Error("msedge.exe not found; set EDGE_PATH");
}

async function waitForVite() {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`http://localhost:${PORT}/`, { signal: AbortSignal.timeout(5_000) });
      if (res.ok) return;
    } catch {
      /* not up yet */
    }
    await sleep(250);
  }
  throw new Error(`vite dev server did not come up on port ${PORT}`);
}

/** Minimal CDP client over the WebSocket devtools endpoint (Node's global WebSocket). */
class Cdp {
  #seq = 0;
  #pending = new Map();
  consoleErrors = [];

  static async connect(browserFailure = () => null) {
    let wsUrl = null;
    const deadline = Date.now() + 15_000;
    while (!wsUrl && Date.now() < deadline) {
      if (browserFailure()) throw browserFailure();
      try {
        const list = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`, { signal: AbortSignal.timeout(2_000) })).json();
        wsUrl = list.find((t) => t.type === "page" && t.url.includes("localhost"))?.webSocketDebuggerUrl ?? null;
      } catch {
        /* browser still starting */
      }
      if (!wsUrl) await sleep(250);
    }
    if (!wsUrl) throw new Error("no CDP page target appeared");
    const cdp = new Cdp();
    cdp.ws = new WebSocket(wsUrl);
    cdp.ws.onmessage = (ev) => cdp.#onMessage(JSON.parse(ev.data));
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("CDP WebSocket did not open within ten seconds")), 10_000);
      cdp.ws.onopen = () => { clearTimeout(timer); resolve(); };
      cdp.ws.onerror = (error) => { clearTimeout(timer); reject(error); };
    });
    await cdp.send("Runtime.enable");
    await cdp.send("Page.enable");
    return cdp;
  }

  #onMessage(msg) {
    if (msg.id && this.#pending.has(msg.id)) {
      this.#pending.get(msg.id)(msg);
      this.#pending.delete(msg.id);
      return;
    }
    // Uncaught page exceptions fail the run: a boot-time crash is exactly the kind of
    // regression that makes "everything silently stopped working" reports.
    if (msg.method === "Runtime.exceptionThrown") {
      const d = msg.params.exceptionDetails;
      this.consoleErrors.push(`${d.text} ${d.exception?.description ?? ""}`.trim());
    }
    if (msg.method === "Runtime.consoleAPICalled" && msg.params.type === "error") {
      this.consoleErrors.push(msg.params.args.map((a) => a.value ?? a.description ?? "").join(" "));
    }
  }

  send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const id = ++this.#seq;
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new Error(`CDP ${method} did not answer within sixty seconds`));
      }, 60_000);
      this.#pending.set(id, (reply) => { clearTimeout(timer); resolve(reply); });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  async eval(expression) {
    const r = await this.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (r.result?.exceptionDetails) {
      throw new Error(`page eval threw: ${r.result.exceptionDetails.text} ${r.result.exceptionDetails.exception?.description ?? ""}`);
    }
    return r.result?.result?.value;
  }

  async navigate(url) {
    await this.send("Page.navigate", { url });
  }

  /** Stop the browser fetching anything matching these patterns, or lift the block when given none. */
  async blockUrls(patterns) {
    await this.send("Network.enable");
    await this.send("Network.setBlockedURLs", { urls: patterns });
  }

  /** The fixture stamps data-visual-ready once switchServer's final awaited load returns.
   *  The generous default absorbs a cold vite start, where the first page load pays for
   *  dependency pre-bundling and the App.svelte transform. */
  async waitReady(timeoutMs = 90000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      // Polled inside a try, because a poll that lands mid-navigation finds no document at all and
      // throws on `documentElement`. That is the normal state of a page that has not arrived yet,
      // not a failure, and letting it escape turned an ordinary race into an intermittent red run
      // whose message pointed nowhere near the cause.
      try {
        if (await this.eval("document.documentElement?.dataset.visualReady ?? ''")) return;
      } catch {
        /* not navigated yet */
      }
      await sleep(250);
    }
    throw new Error("visual fixture never became ready");
  }
}

// Each scenario is an IIFE string evaluated in the page. They return plain objects so the
// assertions live here in Node, where a failure produces a readable diff.

const SEND_SCENARIO = `(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const out = {};
  // Stand in for the native side: acknowledge send_message and serve the appended history
  // back through get_messages, the same contract the actor honors.
  const internals = window.__TAURI_INTERNALS__;
  const base = internals.invoke.bind(internals);
  const sent = [];
  internals.invoke = async (cmd, payload, opts) => {
    if (cmd === "durable_send_context") return "a".repeat(64);
    if (cmd === "send_message") {
      sent.push({
        id: "sent-" + sent.length,
        author: "a4f29c110b7d8365a4f29c110b7d8365",
        text: payload.text,
        ts: Date.now(),
        edited: 0,
        reactions: [],
        reply_to: payload.replyTo ?? "",
        pinned: false,
      });
      return { accepted: true, persistence: { status: "durable" } };
    }
    if (cmd === "get_messages" && payload.server === 1 && payload.channel === "general") {
      const rows = await base(cmd, payload, opts);
      return rows.concat(sent);
    }
    // The log reads pages now; the appended rows ride the tail page the fixture serves.
    if (cmd === "get_message_page" && payload.server === 1 && payload.channel === "general") {
      const page = await base(cmd, payload, opts);
      const extra = sent.map((m) => ({ ...m, targets_me: false, reply_count: 0, reply_to_preview: null }));
      return { ...page, rows: page.rows.concat(extra), total: page.total + sent.length };
    }
    return base(cmd, payload, opts);
  };

  const composer = document.querySelector(".composer textarea") ?? document.querySelector("form textarea");
  const form = composer?.closest("form");
  out.composerFound = !!form;
  if (!form) return out;

  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value").set;
  const type = (text) => {
    setter.call(composer, text);
    composer.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const visible = (text) =>
    Array.from(document.querySelectorAll(".messages li")).some((li) => li.textContent.includes(text));

  type("flow probe one");
  form.requestSubmit();
  await sleep(500);
  out.firstShown = visible("flow probe one");
  out.composerClearedAfterFirst = composer.value === "";

  // The regression this guards: one send wedging the 'sending' flag and silently eating
  // every send after it. A second send must still work.
  type("flow probe two");
  form.requestSubmit();
  await sleep(500);
  out.secondShown = visible("flow probe two");
  out.errorToast = document.querySelector(".error-toast")?.textContent?.trim() ?? null;
  return out;
})();`;

// Search reads every message of every channel in scope, so the scan runs in a worker. That worker
// is the part no unit test can prove: it has to be constructible under the app's own CSP, and its
// answer has to arrive and land in the list. Both are invisible from Node, and a failure is
// silent (the app falls back to scanning inline), so this drives the real thing.
const SEARCH_SCENARIO = `(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const out = {};
  const toggle = document.querySelector(".search-toggle");
  out.toggleFound = !!toggle;
  if (!toggle) return out;
  toggle.click();
  await sleep(200);
  const input = document.querySelector(".msg-search input");
  out.inputFound = !!input;
  if (!input) return out;
  // The result list lives in the advanced panel; the plain bar only steps through matches.
  const filtersToggle = document.querySelector(".search-filters-toggle");
  out.filtersFound = !!filtersToggle;
  if (!filtersToggle) return out;
  filtersToggle.click();
  await sleep(150);

  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  const type = (text) => {
    setter.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const rows = () => Array.from(document.querySelectorAll(".search-results li"));

  type("spacing");
  await sleep(600);
  out.hitCount = rows().length;
  out.hitText = rows()[0]?.textContent?.includes("spacing") ?? false;

  // Narrowing to something absent must empty the list rather than leave the previous answer up:
  // a stale result is the failure mode a worker introduces and an inline scan cannot have.
  type("zzzz-no-such-message");
  await sleep(600);
  out.missCount = rows().length;

  // And back, to prove the worker is still answering after a query that matched nothing.
  type("spacing");
  await sleep(600);
  out.returnCount = rows().length;
  return out;
})();`;

const ACCEPT_SCENARIO = `(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const out = {};
  // Stand in for the native side of the accept flow: one pending request carried by server 1,
  // join_server minting a new DM server, dismiss clearing the request.
  const internals = window.__TAURI_INTERNALS__;
  const base = internals.invoke.bind(internals);
  let requestPending = true;
  const nativeCalls = [];
  internals.invoke = async (cmd, payload, opts) => {
    if (cmd === "get_dm_requests") {
      return requestPending && payload.server === 1
        ? [{ from_fp: "62e80f475ac4931162e80f475ac49311", from_name: "Juniper", invite: "deadbeef" }]
        : [];
    }
    if (cmd === "join_server") {
      nativeCalls.push({ cmd, payload });
      return { server: 7, channel: "dm", channels: [{ id: "dm", name: "general" }], is_dm: true };
    }
    if (cmd === "dismiss_dm_request") {
      nativeCalls.push({ cmd, payload });
      requestPending = false;
      return null;
    }
    if (cmd === "dm_stats") return [];
    return base(cmd, payload, opts);
  };

  document.querySelector('[title="Direct messages & friends"]').click();
  await sleep(800);
  out.requestShown = !!document.querySelector(".dm-requests");
  out.requestText = document.querySelector(".dm-req-name")?.textContent ?? null;

  const acceptBtn = Array.from(document.querySelectorAll(".dm-req-actions button")).find(
    (b) => b.textContent.trim() === "Accept",
  );
  out.acceptFound = !!acceptBtn;
  if (!acceptBtn) return out;
  acceptBtn.click();
  await sleep(1000);

  out.joinInvoked = nativeCalls.some((c) => c.cmd === "join_server" && c.payload.inviteHex === "deadbeef");
  // The identity fix's contract: the DM's rail label is the friend's name, while the joined
  // profile name comes from your own profile (or a fallback), never the friend's.
  const join = nativeCalls.find((c) => c.cmd === "join_server");
  out.joinServerName = join?.payload.serverName ?? null;
  out.dismissInvoked = nativeCalls.some((c) => c.cmd === "dismiss_dm_request");
  out.requestGone = !document.querySelector(".dm-requests");
  out.newDmInRail = Array.from(document.querySelectorAll(".dm-list li")).length >= 2;
  out.errorToast = document.querySelector(".error-toast")?.textContent?.trim() ?? null;
  return out;
})();`;

// Exercise the full App's locked -> hydrated zero-server -> founding -> Settings path. The
// three orphaned requests are already paused; uncertainty is not evidence of non-acceptance.
// Only the native boundary is mocked. No Svelte state, token or DOM is inserted directly.
const ORPHANED_PENDING_SCENARIO = `(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const wait = async (ready, label) => {
    for (let n = 0; n < 100; n++) { if (ready()) return; await sleep(50); }
    throw new Error("pending flow timed out: " + label);
  };
  const out = {};
  const tokens = ["1".repeat(32), "2".repeat(32), "3".repeat(32)];
  const fullText = "Full orphaned text: " + "x".repeat(40000) + String.fromCharCode(10) + "complete tail";
  const pending = Object.fromEntries(tokens.map((token, index) => [token, {
    token, server: 999, channel: String(index + 1), expectedContext: "a".repeat(64),
    text: index === 0 ? fullText : "Orphaned request " + index, replyTo: "original-reply",
    retryBlock: ["invalid", "conflict", "context_changed"][index],
    ...(index === 2 ? { acceptance: "ambiguous" } : {}),
  }]));
  let sealed = { version: 1, drafts: {}, readMarks: {}, pendingSends: pending };
  let unlocks = 0, sends = 0, contexts = 0, newTokens = 0;
  const writes = [];
  const internals = window.__TAURI_INTERNALS__, base = internals.invoke.bind(internals);
  const originalUuid = crypto.randomUUID.bind(crypto);
  crypto.randomUUID = () => { newTokens++; return originalUuid(); };
  internals.invoke = async (cmd, payload, opts) => {
    if (cmd === "unlock") { unlocks++; return []; }
    if (cmd === "lock_session") {
      // The first lock leaves the starting chat fixture; subsequent locks save this fixture vault.
      if (unlocks && payload.uiStateJson) sealed = JSON.parse(payload.uiStateJson);
      return { continuity_error: null };
    }
    if (cmd === "get_ui_state") return JSON.stringify(sealed);
    if (cmd === "save_ui_state") {
      if (unlocks) { sealed = JSON.parse(payload.json); writes.push(structuredClone(sealed)); }
      return null;
    }
    if (cmd === "send_message") { sends++; throw new Error("Unexpected orphan publication"); }
    if (cmd === "durable_send_context") { contexts++; return "b".repeat(64); }
    return base(cmd, payload, opts);
  };
  const unlock = async () => {
    document.querySelector("button.sb-lock")?.click();
    await wait(() => document.querySelector('input[placeholder="passphrase"]'), "lock gate");
    const input = document.querySelector('input[placeholder="passphrase"]');
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(input, "fixture secret");
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    await wait(() => document.querySelector(".start-wide"), "hydrated founding screen");
  };
  const openManager = async () => {
    const button = [...document.querySelectorAll(".start-wide button")].find(b => b.textContent.includes("Pending messages & saved drafts"));
    if (!button) throw new Error("No pending-manager entry on the zero-server founding screen");
    button.click();
    await wait(() => document.querySelector('[aria-label="Vault pending messages"]'), "vault manager");
  };
  const click = (token, text) => {
    const card = document.querySelector('[data-pending-token="' + token + '"]');
    const button = [...(card?.querySelectorAll("button") ?? [])].find(b => b.textContent.trim() === text);
    if (!button || button.disabled) throw new Error("Missing enabled action: " + text);
    button.click();
  };
  await unlock();
  out.zeroServerFounding = !!document.querySelector(".start-wide") && !document.querySelector(".composer");
  await openManager();
  out.hydratedOrphans = document.querySelectorAll("[data-pending-token]").length;
  out.uncertainWarning = document.querySelector('[aria-label="Vault pending messages"]').textContent.includes("may already have been accepted");
  click(tokens[0], "Recover to saved draft");
  await wait(() => [...document.querySelectorAll("button")].some(b => b.textContent.trim() === "Confirm recovery"), "recovery confirmation");
  out.beforeConfirmationRetained = Object.keys(sealed.pendingSends).length === 3
    && writes.every(saved => tokens.every(token => saved.pendingSends[token]));
  click(tokens[0], "Confirm recovery");
  await wait(() => !document.querySelector('[data-pending-token="' + tokens[0] + '"]'), "sealed recovery");
  out.savedFullText = sealed.recoveredSendDrafts?.[tokens[0]]?.text === fullText;
  out.savedOriginalIdentity = sealed.recoveredSendDrafts?.[tokens[0]]?.token === tokens[0]
    && sealed.recoveredSendDrafts?.[tokens[0]]?.replyTo === "original-reply";
  out.recoveryReleasedOne = Object.keys(sealed.pendingSends).length === 2;
  for (const token of tokens.slice(1)) {
    click(token, "Stop retrying");
    await wait(() => [...document.querySelectorAll("button")].some(b => b.textContent.trim() === "Confirm stop retrying"), "stop confirmation");
    click(token, "Confirm stop retrying");
    await wait(() => !document.querySelector('[data-pending-token="' + token + '"]'), "sealed stop retrying");
  }
  out.savedQueueEmpty = Object.keys(sealed.pendingSends).length === 0;
  out.onlyOriginalRecovery = Object.keys(sealed.recoveredSendDrafts).join() === tokens[0];
  await unlock(); await openManager();
  out.reopenedQueueEmpty = document.querySelectorAll("[data-pending-token]").length === 0;
  out.reopenedFullText = document.querySelector('[data-recovered-token="' + tokens[0] + '"] textarea')?.value === fullText;
  out.unlocks = unlocks;
  out.sendInvocations = sends; out.contextInvocations = contexts; out.newRetryTokens = newTokens;
  crypto.randomUUID = originalUuid;
  return out;
})();`;

/**
 * Open the debug console and visit every section.
 *
 * The gap this fills: a runtime error in one section's markup is invisible to everything else we
 * run. `svelte-check` type-checks and never renders; the unit suites exercise the pure functions in
 * `debug-console.ts` and never mount the component. So the only thing standing between a broken
 * section and a release was somebody opening it and looking, and the tool whose entire job is to
 * explain failures is a poor choice for the one that fails silently.
 *
 * A smoke test, and honest about it: it proves each section rendered something and threw nothing,
 * not that it rendered the right thing. That is still the difference between a blank panel shipping
 * and not.
 */
const CONSOLE_SECTIONS_SCENARIO = `(async () => {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const out = { opened: false, sections: [], empty: [] };
  const byText = (selector, text) =>
    [...document.querySelectorAll(selector)].find((e) =>
      e.textContent.trim().toLowerCase().includes(text),
    );

  // The console lives behind Settings, Diagnostics. Navigating there is part of what is being
  // checked: a button nobody can reach is as broken as a section that will not render.
  document.querySelector('button[aria-label="Settings"]')?.click();
  await sleep(400);
  byText(".stx-item, .stx-nav button, nav button, .stx-sub button", "diagnostics")?.click();
  await sleep(400);
  out.reached = !!byText("button", "open debug console");
  byText("button", "open debug console")?.click();
  // Polled, not slept: the console is a dynamic import so ordinary chat startup does not pay for
  // it, which means the first open waits on a module fetch. A fixed delay here is a race that
  // fails on a cold vite and passes on a warm one.
  for (let i = 0; i < 60 && !document.querySelector(".dbg"); i += 1) await sleep(100);
  out.opened = !!document.querySelector(".dbg");
  if (!out.opened) return out;

  for (const name of ["overview", "network", "voice", "backend", "frontend", "storage"]) {
    const item = byText(".dbg-rail-item", name);
    if (!item) { out.empty.push(name + ":no-rail-item"); continue; }
    item.click();
    await sleep(350);
    // A section that threw while rendering leaves the panel behind entirely, so this catches the
    // whole-console crash as well as the empty one.
    const cards = document.querySelectorAll(".dbg-content .dbg-card").length;
    out.sections.push(name);
    if (!cards) out.empty.push(name);
  }
  out.visited = out.sections.length;
  out.broken = out.empty.join(",");
  return out;
})();`;

/**
 * The boot-failure panel in `index.html`, and whether it is on screen.
 *
 * Written to survive the panel having been removed from the document, which is what
 * `public/boot-failure.js` does once the application mounts: "gone" and "hidden" are both success.
 * `display` is reported separately from `showing` because the two hiding mechanisms are different
 * claims, and a check that only asked "is it on screen" could not tell them apart.
 */
const BOOT_PANEL_STATE = `(() => {
  const el = document.getElementById("boot-failure");
  if (!el) return { present: false, showing: false, display: "", summary: "", detail: "", copyOffered: false, appChildren: -1 };
  const style = getComputedStyle(el);
  return {
    present: true,
    showing: style.display !== "none" && style.visibility === "visible",
    display: style.display,
    heading: el.querySelector("h1")?.textContent?.trim() ?? "",
    summary: document.getElementById("boot-failure-summary")?.textContent ?? "",
    detail: document.getElementById("boot-failure-detail")?.textContent ?? "",
    copyOffered: !!document.querySelector("#boot-failure-actions button"),
    appChildren: document.getElementById("app")?.children.length ?? -1,
  };
})();`;

/**
 * Read the panel's state until `done` is satisfied, or until the cap runs out.
 *
 * The waiting is done here rather than inside the page on purpose. These checks run over ten-second
 * stretches, and an evaluation that long straddles the navigation that started it: the execution
 * context is torn down under the promise and CDP answers with nothing at all, which reads as a
 * failing assertion about a panel nobody ever looked at. Short reads, polled from Node, survive it,
 * and polling for the state the new page should be in is also how the harness waits out a
 * navigation that has not committed yet.
 */
async function watchBootPanel(cdp, capMs, done) {
  const deadline = Date.now() + capMs;
  let seen = {};
  for (;;) {
    try {
      seen = (await cdp.eval(BOOT_PANEL_STATE)) ?? seen;
    } catch {
      /* a poll that lands mid-navigation has no document to read yet */
    }
    if (done(seen) || Date.now() >= deadline) return seen;
    await sleep(250);
  }
}

const panelShowing = (seen) => !!seen.showing;

function assertEqual(scenario, got, want) {
  const failures = [];
  for (const [key, expected] of Object.entries(want)) {
    if (got?.[key] !== expected) failures.push(`  ${key}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(got?.[key])}`);
  }
  if (failures.length) {
    console.error(`FAIL ${scenario}\n${failures.join("\n")}\n  full result: ${JSON.stringify(got)}`);
    return false;
  }
  console.log(`ok ${scenario}`);
  return true;
}

const vite = spawn("npm", ["run", "dev", "--", "--port", String(PORT), "--strictPort"], {
  cwd: new URL("..", import.meta.url),
  stdio: "ignore",
  shell: true,
  windowsHide: true,
});
const profileDir = mkdtempSync(join(tmpdir(), "catcoms-flow-"));
let edge = null;
let failed = false;
let viteUp = false;
let cleanedUp = false;

function cleanup() {
  if (cleanedUp) return;
  cleanedUp = true;
  edge?.kill();
  killTree(vite.pid);
  if (viteUp) killPortListener(PORT);
}

// Held outside the try so a failure on the way *in* can still say what the page complained about.
// Without this, a module that will not parse reports only "visual fixture never became ready",
// and the SyntaxError naming the line sits in a buffer nobody prints.
let connected = null;

// Ctrl-C skips the finally block, which would leak exactly the orphans cleanup exists to
// prevent. The temp profile is left behind on this path; it lives in tmpdir and is disposable.
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    cleanup();
    process.exit(130);
  });
}

try {
  await waitForVite();
  viteUp = true;
  edge = spawn(
    await findEdge(),
    [
      "--headless=new",
      "--disable-gpu",
      `--remote-debugging-port=${CDP_PORT}`,
      "--no-first-run",
      `--user-data-dir=${profileDir}`,
      "--window-size=1280,800",
      URL_UNDER_TEST,
    ],
    { stdio: "ignore", windowsHide: true },
  );
  let browserFailure = null;
  edge.once("error", error => { browserFailure = new Error("Edge launch failed: " + error.message); });
  edge.once("exit", (code, signal) => {
    if (!cleanedUp) browserFailure = new Error(`Edge exited before completion (code ${code}, signal ${signal})`);
  });
  connected = await Cdp.connect(() => browserFailure);
  const cdp = connected;

  await cdp.waitReady();
  const send = await cdp.eval(SEND_SCENARIO);
  failed |= !assertEqual("send flow", send, {
    composerFound: true,
    firstShown: true,
    composerClearedAfterFirst: true,
    secondShown: true,
    errorToast: null,
  });

  await cdp.navigate(URL_UNDER_TEST);
  await cdp.waitReady();
  const search = await cdp.eval(SEARCH_SCENARIO);
  failed |= !assertEqual("search flow", search, {
    toggleFound: true,
    inputFound: true,
    filtersFound: true,
    hitCount: 1,
    hitText: true,
    missCount: 0,
    returnCount: 1,
  });

  // A fresh load keeps the scenarios independent: the send test's IPC patch and its
  // optimistic rows must not leak into the accept test's view of the world.
  await cdp.navigate(URL_UNDER_TEST);
  await cdp.waitReady();
  const accept = await cdp.eval(ACCEPT_SCENARIO);
  failed |= !assertEqual("accept friend request flow", accept, {
    requestShown: true,
    requestText: "Juniper wants to DM you",
    acceptFound: true,
    joinInvoked: true,
    joinServerName: "Juniper",
    dismissInvoked: true,
    requestGone: true,
    newDmInRail: true,
    errorToast: null,
  });

  await cdp.navigate(URL_UNDER_TEST);
  await cdp.waitReady();
  const orphaned = await cdp.eval(ORPHANED_PENDING_SCENARIO);
  failed |= !assertEqual("zero-server pending-message recovery flow", orphaned, {
    zeroServerFounding: true, hydratedOrphans: 3, uncertainWarning: true,
    beforeConfirmationRetained: true, savedFullText: true, savedOriginalIdentity: true,
    recoveryReleasedOne: true, savedQueueEmpty: true, onlyOriginalRecovery: true,
    reopenedQueueEmpty: true, reopenedFullText: true, unlocks: 2,
    sendInvocations: 0, contextInvocations: 0, newRetryTokens: 0,
  });

  // A fresh load again, so the accept test's patched IPC cannot decide what the console shows.
  await cdp.navigate(URL_UNDER_TEST);
  await cdp.waitReady();
  // Piggybacks on the load the console scenario is about to use. An application that mounted is
  // the success signal, and the bootstrap script takes the panel out of the document when it sees
  // one, so on a healthy start there should be no panel left at all.
  const bootOk = await cdp.eval(BOOT_PANEL_STATE);
  failed |= !assertEqual(
    "boot-failure panel is removed once the app mounts",
    { present: bootOk?.present, showing: bootOk?.showing },
    { present: false, showing: false },
  );

  const sections = await cdp.eval(CONSOLE_SECTIONS_SCENARIO);
  failed |= !assertEqual("debug console renders every section", sections, {
    reached: true,
    opened: true,
    visited: 6,
    broken: "",
  });

  // Everything past here loads a deliberately broken page, so the errors it prints are the point
  // rather than a regression. Anything the healthy scenarios reported is kept and checked below.
  const errorsBeforeBootChecks = cdp.consoleErrors.length;

  // The other half of the healthy case, with the bootstrap script blocked so nothing can take the
  // panel away on purpose. What is left is the stylesheet's own rule, and `display` is asserted
  // rather than "is it on screen": the visual fixture disables every animation on the page so it
  // can screenshot deterministically, which means a reveal-timer check on a fixture page would
  // pass whether or not anything was hiding the panel.
  await cdp.blockUrls(["*boot-failure.js*"]);
  await cdp.navigate(URL_UNDER_TEST);
  const bootQuiet = await watchBootPanel(cdp, 40000, (seen) => seen.present && seen.appChildren > 0);
  failed |= !assertEqual(
    "the stylesheet alone hides the panel once the app mounts",
    { present: bootQuiet?.present, display: bootQuiet?.display, showing: bootQuiet?.showing },
    { present: true, display: "none", showing: false },
  );

  // The failure this whole panel exists for: the module bundle never arrives, so no application
  // code runs and every diagnostic the project has built is inside the thing that failed. Blocking
  // the entry module at the network layer is that failure exactly.
  await cdp.blockUrls(["*main.ts*"]);
  await cdp.navigate(URL_UNDER_TEST);
  const bootFailure = await watchBootPanel(cdp, 20000, panelShowing);
  failed |= !assertEqual(
    "boot-failure panel reports a bundle that never loaded",
    {
      showing: bootFailure?.showing,
      heading: bootFailure?.heading,
      appEmpty: bootFailure?.appChildren === 0,
      namesTheModule: /main\.ts/.test(bootFailure?.summary ?? ""),
      summaryRedacted: !/localhost:/.test(bootFailure?.summary ?? ""),
      rawDetailKeptWhole: /localhost:/.test(bootFailure?.detail ?? ""),
      copyOffered: bootFailure?.copyOffered,
    },
    {
      showing: true,
      heading: "Mewtual could not start",
      appEmpty: true,
      namesTheModule: true,
      summaryRedacted: true,
      rawDetailKeptWhole: true,
      copyOffered: true,
    },
  );

  // And the same failure with the bootstrap script blocked too, which is the layer that has to hold
  // when there is no script running anywhere: the panel is visible by default and the stylesheet
  // only hides it once an application mounts, so a window that never mounts one says so on its own.
  await cdp.blockUrls(["*main.ts*", "*boot-failure.js*"]);
  await cdp.navigate(URL_UNDER_TEST);
  const bootNoScript = await watchBootPanel(cdp, 40000, panelShowing);
  failed |= !assertEqual(
    "boot-failure panel shows with no script running at all",
    { showing: bootNoScript?.showing, heading: bootNoScript?.heading, copyOffered: bootNoScript?.copyOffered },
    { showing: true, heading: "Mewtual could not start", copyOffered: false },
  );

  await cdp.blockUrls([]);
  cdp.consoleErrors.length = errorsBeforeBootChecks;

  if (cdp.consoleErrors.length) {
    console.error(`FAIL page errors:\n  ${cdp.consoleErrors.join("\n  ")}`);
    failed = true;
  }
} catch (e) {
  console.error(`FAIL harness: ${e.message}`);
  // Whatever the page managed to say before it gave up. This is usually the actual diagnosis: a
  // "never became ready" is nearly always a module that threw or would not parse, and the browser
  // has already reported which line.
  if (connected?.consoleErrors.length) {
    console.error(`  page said:\n    ${connected.consoleErrors.join("\n    ")}`);
  }
  failed = true;
} finally {
  cleanup();
  await sleep(300);
  try {
    rmSync(profileDir, { recursive: true, force: true });
  } catch {
    /* the browser may still hold a lock for a moment; the temp dir is disposable */
  }
}
process.exit(failed ? 1 : 0);
