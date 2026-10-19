// Startup evidence belongs to the child/profile we created. A successful HTTP response on a
// familiar port alone can come from another checkout and must never authorize page mutation.
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

export async function waitForOwnedVite(child, port, timeoutMs = 20_000) {
  let output = "", failure = null;
  const observe = chunk => { output = (output + chunk).slice(-16_384); };
  const onError = error => { failure = error; };
  const onExit = (code, signal) => { failure = new Error(`Vite exited during startup (${code}, ${signal}): ${output}`); };
  child.stdout.on("data", observe);
  child.stderr.on("data", observe);
  child.on("error", onError);
  child.on("exit", onExit);
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (failure) throw failure;
    if (child.exitCode !== null || child.signalCode !== null) throw new Error("Owned Vite exited before readiness");
    // Vite prints this only after its own strict-port listen succeeded. Strip ANSI decoration
    // but require the exact port terminator, so 5177 cannot match a server on 51770.
    const plain = output.replace(/\x1b\[[0-9;]*m/g, "");
    if (plain.includes(`http://localhost:${port}/`)) return;
    await sleep(25);
  }
  throw new Error(`Owned Vite did not become ready on ${port}: ${output}`);
}

export function parseDebuggerPort(text) {
  const lines = text.trim().split(/\r?\n/);
  if (lines.length !== 2 || !/^[1-9][0-9]{0,4}$/.test(lines[0]) || Number(lines[0]) > 65535
      || !/^\/devtools\/browser\/[a-zA-Z0-9-]+$/.test(lines[1])) return null;
  return { base: `http://127.0.0.1:${lines[0]}`, path: lines[1] };
}

/** Only the fresh private profile may select the debugger. An old FLOW_CDP_PORT is ignored. */
export async function ownedDebuggerEndpoint(profileDir, browserFailure = () => null, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (browserFailure()) throw browserFailure();
    let endpoint = null;
    try { endpoint = parseDebuggerPort(await readFile(join(profileDir, "DevToolsActivePort"), "utf8")); }
    catch (error) { if (error.code !== "ENOENT") throw error; }
    if (endpoint) {
      try {
        const version = await (await fetch(`${endpoint.base}/json/version`, { signal: AbortSignal.timeout(2_000) })).json();
        if (version.webSocketDebuggerUrl === endpoint.base.replace("http:", "ws:") + endpoint.path) return endpoint.base;
      } catch { /* the owned browser may still be opening its endpoint */ }
    }
    await sleep(25);
  }
  throw new Error("The private Edge profile did not publish its own debugger endpoint");
}
