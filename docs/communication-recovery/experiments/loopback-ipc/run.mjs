// Manual packaged-WebView compatibility gate. No security-disabling flags or application vault.
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseDebuggerPort } from "../../../../apps/desktop/scripts/flow-processes.mjs";
import { childOutputClosed } from "./child-output.mjs";
import { keylessTakeover } from "./takeover.mjs";

const ROOT = fileURLToPath(new URL("../../../../", import.meta.url));
const argument = process.argv.find(value => value.startsWith("--binary="));
const fault = process.argv.find(value => value.startsWith("--fault="))?.slice("--fault=".length) ?? "";
if (!["", "cleanup"].includes(fault)) throw new Error("unknown probe fault");
const executable = argument?.slice("--binary=".length) ?? join(ROOT, "target", "loopback-ipc-probe", "debug", "mewtual-loopback-ipc-probe.exe");
const deadline = performance.now() + 90_000;
const stop = new AbortController();
const watchdog = setTimeout(() => stop.abort(), 90_000);
const profile = await mkdtemp(join(tmpdir(), "mewtual-ipc-probe-"));
const connections = [];
let child, stdioClosed, keyless, exitCode = null, output = "", completed = false, failure = null, candidate = null;
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function bounded(operation, label, maximum = 5000) {
  let timer;
  try { return await Promise.race([operation, new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} timed out`)), Math.max(1, Math.min(maximum, deadline - performance.now()))); })]); }
  finally { clearTimeout(timer); }
}
class Cdp {
  sequence = 0; pending = new Map();
  static async open(url) {
    const cdp = new Cdp();
    cdp.socket = new WebSocket(url);
    connections.push(cdp); // also own a connection whose opening handshake never completes
    await bounded(new Promise((resolve, reject) => { cdp.socket.onopen = resolve; cdp.socket.onerror = reject; }), "debugger connection");
    cdp.socket.onmessage = event => {
      const message = JSON.parse(event.data);
      if (!message.id) return;
      const settle = cdp.pending.get(message.id); cdp.pending.delete(message.id);
      settle?.(message);
    };
    cdp.socket.onclose = () => { for (const settle of cdp.pending.values()) settle({ error: { message: "owned debugger closed" } }); cdp.pending.clear(); };
    await cdp.send("Runtime.enable"); await cdp.send("Page.enable");
    return cdp;
  }
  async send(method, params = {}) {
    const id = ++this.sequence;
    try {
      const reply = await bounded(new Promise(resolve => { this.pending.set(id, resolve); this.socket.send(JSON.stringify({ id, method, params })); }), method);
      if (reply.error) throw new Error(reply.error.message);
      return reply.result;
    } finally { this.pending.delete(id); }
  }
  async eval(expression) {
    const reply = await this.send("Runtime.evaluate", { expression, returnByValue: true });
    // The expressions below return only closed probe metadata. Never print arbitrary exception
    // descriptions/page state, which could accidentally contain a bootstrap reply or its key.
    if (reply.exceptionDetails) throw new Error("probe metadata evaluation refused");
    return reply.result.value;
  }
}
async function debuggerBase() {
  while (performance.now() < deadline) {
    if (exitCode !== null) throw new Error("native probe exited before debugger readiness");
    for (const directory of [profile, join(profile, "EBWebView")]) {
      let endpoint;
      try { endpoint = parseDebuggerPort(await readFile(join(directory, "DevToolsActivePort"), { encoding: "utf8", signal: stop.signal })); }
      catch (error) { if (!["ENOENT", "EBUSY"].includes(error.code)) throw error; }
      if (!endpoint) continue;
      try {
        const version = await bounded(ownedFetch(`${endpoint.base}/json/version`).then(response => response.json()), "debugger identity");
        if (version.webSocketDebuggerUrl === endpoint.base.replace("http:", "ws:") + endpoint.path) return { ...endpoint, browser: version.Browser };
      } catch { /* owned browser still publishing its endpoint */ }
    }
    await sleep(50);
  }
  throw new Error("owned WebView debugger did not become ready");
}
// A timeout must abort the real HTTP operation, not leave a succession of detached requests
// alive behind Promise.race. These requests address only the fresh owned debugger endpoint.
const ownedFetch = url => fetch(url, { signal: AbortSignal.any([stop.signal, AbortSignal.timeout(2000)]) });
try {
  child = spawn(executable, [], { windowsHide: true, stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, WEBVIEW2_USER_DATA_FOLDER: profile, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: "--remote-debugging-port=0" } });
  child.on("exit", code => { exitCode = code; });
  stdioClosed = childOutputClosed(child);
  // Only fixed phase/panic evidence is retained. No arbitrary native IPC payload is logged.
  const observe = chunk => { output = (output + chunk.toString()).slice(-8192); };
  child.stdout.on("data", observe); child.stderr.on("data", observe);
  await bounded(new Promise((resolve, reject) => { child.once("spawn", resolve); child.once("error", reject); }), "native child startup");
  const debuggerInfo = await debuggerBase();
  const surfaces = new Map();
  while (surfaces.size < 3 && performance.now() < deadline) {
    const pages = await bounded(ownedFetch(`${debuggerInfo.base}/json/list`).then(response => response.json()), "page inventory");
    for (const page of pages.filter(page => page.type === "page")) {
      if (surfaces.has(page.id)) continue;
      const cdp = await Cdp.open(page.webSocketDebuggerUrl);
      surfaces.set(page.id, { cdp, result: null });
    }
    if (surfaces.size < 3) await sleep(50);
  }
  assert.equal(surfaces.size, 3, "expected main, restricted and remote WebViews");
  while (performance.now() < deadline) {
    for (const surface of surfaces.values()) {
      if (surface.result) continue;
      const error = await surface.cdp.eval("window.__probeError ?? null");
      if (error) throw new Error(`packaged probe failed: ${error}`);
      // Remote HTML has no separate readiness setter, so read its closed Promise metadata.
      await surface.cdp.send("Runtime.evaluate", { expression: "window.__probe?.then(result => { window.__probeResult = result; })" });
      surface.result = await surface.cdp.eval("window.__probeResult ?? null");
    }
    if ([...surfaces.values()].every(surface => surface.result)) break;
    await sleep(50);
  }
  const all = [...surfaces.values()];
  const main = all.find(surface => surface.result?.surface === "main");
  const restricted = all.find(surface => surface.result?.surface === "restricted");
  const remote = all.find(surface => surface.result?.surface === "remote");
  assert.ok(main && restricted && remote, "all three actual surfaces must finish");
  assert.deepEqual(main.result, { surface: "main", packaged: true, secure: true, lost: true, exhausted: true, healthy: true, cspBlocked: true, corsBlocked: true,
    encryptedMaximum: true, headerTamperRefused: true, bodyTamperRefused: true, replayRefused: true });
  assert.equal(restricted.result.denied, true); assert.equal(restricted.result.bridge, true);
  assert.equal(remote.result.denied, true); assert.equal(remote.result.bridge, true);
  const before = await main.cdp.eval("window.__TAURI_INTERNALS__.invoke('inspect').then(value => { window.__probeCounters = value; })");
  void before;
  let counters;
  while (!counters && performance.now() < deadline) { counters = await main.cdp.eval("window.__probeCounters ?? null"); if (!counters) await sleep(25); }
  assert.equal(counters.bootstrap_entries, 3, "ACL denials never entered the native bootstrap handler");
  assert.equal(counters.bootstrap_grants, 2, "lost reply used the second finite grant");
  assert.equal(counters.good_requests, 1); assert.equal(counters.wrong_cors_requests, 1);
  assert.equal(counters.blocked_requests, 0, "the CSP-denied listener received no request");
  assert.equal(counters.frame_headers, 3, "only authenticated fresh frames spend native admission");
  assert.equal(counters.frame_bodies, 2, "the corrupted body never reaches fixture execution");
  assert.equal(counters.frame_replies, 2);
  assert.ok(counters.options >= 1 && counters.peak_active <= 4);
  await main.cdp.send("Runtime.evaluate", { expression: "window.__TAURI_INTERNALS__.invoke('release_endpoint').then(origin => { window.__releasedOrigin = origin; })" });
  let releasedOrigin;
  while (!releasedOrigin && performance.now() < deadline) { releasedOrigin = await main.cdp.eval("window.__releasedOrigin ?? null"); if (!releasedOrigin) await sleep(25); }
  assert.ok(releasedOrigin, "both actual native listeners must close before port replacement");
  keyless = keylessTakeover(releasedOrigin); // own even a listener whose bind never becomes ready
  await bounded(keyless.ready, "exact-port keyless replacement");
  await main.cdp.send("Runtime.evaluate", { expression: "window.__probeTakeover().then(result => { window.__takeoverResult = result; }).catch(() => { window.__takeoverResult = { refused: false }; })" });
  let takeover;
  while (!takeover && performance.now() < deadline) { takeover = await main.cdp.eval("window.__takeoverResult ?? null"); if (!takeover) await sleep(25); }
  assert.deepEqual(takeover, { refused: true, headerAuthenticationRefused: true }, "a fresh keyless reply must reach and fail HMAC verification");
  const keylessStats = keyless.snapshot();
  assert.equal(keylessStats.posts, 1); assert.equal(keylessStats.ciphertextBytes, 8208);
  assert.equal(keylessStats.plaintextObserved, false); assert.equal(keylessStats.listenerError, false);
  await main.cdp.send("Page.reload", { ignoreCache: true });
  let refreshed;
  while (!refreshed && performance.now() < deadline) { refreshed = await main.cdp.eval("window.__probeResult?.refreshedRefusesBootstrap ?? false"); if (!refreshed) await sleep(50); }
  assert.equal(refreshed, true, "hard refresh cannot recreate fixture bootstrap allowance");
  candidate = { phase: "encrypted-framing-only", browser: debuggerInfo.browser, main: main.result,
    restricted: restricted.result, remote: remote.result, counters, refreshRefusesBootstrap: refreshed,
    authenticatedBinaryFraming: true, actualPortTakeoverTested: true, takeover: keylessStats, wholeCopyCustody: false, productionIntegration: false };
  await main.cdp.send("Runtime.evaluate", { expression: "void window.__TAURI_INTERNALS__.invoke('finish_probe')" }).catch(() => {});
  while (exitCode === null && performance.now() < deadline) await sleep(25);
  const closed = await bounded(stdioClosed, "native output closure");
  assert.equal(exitCode, 0); assert.equal(closed.code, 0);
  assert.ok(output.includes("PROBE.SHUTDOWN.DRAINED"));
  completed = true;
} catch (error) {
  failure = error;
} finally {
  clearTimeout(watchdog); stop.abort();
  for (const connection of connections) { try { connection.socket.close(); } catch { /* failed handshake is still owned and its child is stopped below */ } }
  if (child?.pid && exitCode === null) spawnSync("taskkill", ["/F", "/T", "/PID", String(child.pid)], { windowsHide: true, stdio: "ignore", timeout: 10_000 });
  try { await keyless?.close(); } catch { if (!failure) failure = new Error("owned keyless listener cleanup failed"); }
  assert.equal(dirname(resolve(profile)), resolve(tmpdir()));
  assert.ok(basename(profile).startsWith("mewtual-ipc-probe-"));
  try { await rm(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 }); }
  catch { if (!failure) failure = new Error("owned WebView profile cleanup failed"); }
  if (fault === "cleanup" && !failure) failure = new Error("injected cleanup failure");
}
if (failure) throw failure;
assert.ok(completed && candidate);
// Only terminal success may become evidence. A failed native drain or profile cleanup leaves
// no success record for this run, including the deliberate cleanup-failure regression.
const result = { status: "passed", runId: basename(profile), ...candidate };
await writeFile(new URL("result.json", import.meta.url), JSON.stringify(result, null, 2));
console.log(JSON.stringify(result, null, 2));
