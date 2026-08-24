// Browser flow checks for the two chat paths a broken switch/backend most often takes down:
// sending a message and accepting an in-band friend request. They drive the REAL Svelte app
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
    spawnSync("taskkill", ["/F", "/T", "/PID", String(pid)], { stdio: "ignore" });
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
  const found = spawnSync("netstat", ["-ano", "-p", "TCP"], { encoding: "utf8" });
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
  for (let i = 0; i < 80; i++) {
    try {
      const res = await fetch(`http://localhost:${PORT}/`);
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

  static async connect() {
    let wsUrl = null;
    for (let i = 0; i < 60 && !wsUrl; i++) {
      try {
        const list = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)).json();
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
      cdp.ws.onopen = resolve;
      cdp.ws.onerror = reject;
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
    return new Promise((resolve) => {
      const id = ++this.#seq;
      this.#pending.set(id, resolve);
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
      return null;
    }
    if (cmd === "get_messages" && payload.server === 1 && payload.channel === "general") {
      const rows = await base(cmd, payload, opts);
      return rows.concat(sent);
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
    { stdio: "ignore" },
  );
  connected = await Cdp.connect();
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

  // A fresh load again, so the accept test's patched IPC cannot decide what the console shows.
  await cdp.navigate(URL_UNDER_TEST);
  await cdp.waitReady();
  const sections = await cdp.eval(CONSOLE_SECTIONS_SCENARIO);
  failed |= !assertEqual("debug console renders every section", sections, {
    reached: true,
    opened: true,
    visited: 6,
    broken: "",
  });

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
