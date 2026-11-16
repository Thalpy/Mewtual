import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { ownedDebuggerEndpoint, parseDebuggerPort, waitForOwnedVite } from "./flow-processes.mjs";

async function listen(server) {
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  return server.address().port;
}
const close = server => new Promise(resolve => server.close(resolve));

test("occupied Vite port refuses without reading or stopping the existing app", async () => {
  const fixture = await mkdtemp(join(tmpdir(), "catcoms-vite-port-test-"));
  let requests = 0;
  const stranger = createServer((_req, res) => { requests++; res.end("another checkout"); });
  let child;
  try {
    const port = await listen(stranger);
    // This checks Vite's actual strict-port refusal and our owned-output observer, not app
    // dependency compilation. A cold full app prebundle can otherwise consume this deadline
    // before Vite attempts to listen. Real application compilation/readiness is test:flows.
    const config = join(fixture, "vite.config.mjs");
    await writeFile(config, "export default { optimizeDeps: { noDiscovery: true, include: [] }, server: { preTransformRequests: false } };\n");
    child = spawn(process.execPath, [fileURLToPath(new URL("../node_modules/vite/bin/vite.js", import.meta.url)), fixture, "--config", config, "--configLoader", "native", "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
      cwd: fixture, stdio: ["ignore", "pipe", "pipe"], windowsHide: true,
    });
    await assert.rejects(waitForOwnedVite(child, port), error => error instanceof Error &&
      /Vite exited/.test(error.message) && error.message.includes(`Port ${port} is already in use`));
    assert.equal(requests, 0, "no request may select the stranger as the tested app");
    assert.equal(await (await fetch(`http://127.0.0.1:${port}`)).text(), "another checkout");
  } finally {
    child?.kill();
    if (stranger.listening) await close(stranger);
    await rm(fixture, { recursive: true, force: true });
  }
});

test("only the private profile's exact debugger identity can select an endpoint", async () => {
  const profile = await mkdtemp(join(tmpdir(), "catcoms-flow-test-"));
  let requests = 0;
  const stranger = createServer((_req, res) => { requests++; res.end("{}"); });
  const port = await listen(stranger);
  const previous = process.env.FLOW_CDP_PORT;
  process.env.FLOW_CDP_PORT = String(port);
  try {
    await assert.rejects(ownedDebuggerEndpoint(profile, () => null, 60), /private Edge profile/);
    assert.equal(requests, 0, "a legacy fixed port is not even probed");
    await writeFile(join(profile, "DevToolsActivePort"), `${port}\n/devtools/browser/our-new-browser\n`);
    await assert.rejects(ownedDebuggerEndpoint(profile, () => null, 60), /private Edge profile/);
    assert.ok(requests > 0);
  } finally {
    if (previous === undefined) delete process.env.FLOW_CDP_PORT; else process.env.FLOW_CDP_PORT = previous;
    await close(stranger); await rm(profile, { recursive: true, force: true });
  }
});

test("debugger profile parsing rejects nonlocal paths and malformed port claims", () => {
  assert.deepEqual(parseDebuggerPort("4567\n/devtools/browser/abc-123\n"), { base: "http://127.0.0.1:4567", path: "/devtools/browser/abc-123" });
  for (const text of ["0\n/devtools/browser/x", "65536\n/devtools/browser/x", "1234\nhttps://other", "1234\n/devtools/browser/x\nextra"]) {
    assert.equal(parseDebuggerPort(text), null);
  }
});

test("private debugger discovery survives a transient publication sharing lock", async () => {
  const profile = await mkdtemp(join(tmpdir(), "catcoms-flow-test-"));
  const path = "/devtools/browser/our-new-browser";
  let port, requests = 0, reads = 0;
  const browser = createServer((req, res) => {
    assert.equal(req.url, "/json/version"); requests++;
    res.setHeader("content-type", "application/json");
    res.end(JSON.stringify({ webSocketDebuggerUrl: `ws://127.0.0.1:${port}${path}` }));
  });
  port = await listen(browser);
  try {
    const file = join(profile, "DevToolsActivePort");
    await writeFile(file, `${port}\n${path}\n`);
    const endpoint = await ownedDebuggerEndpoint(profile, () => null, 2_000, async (name, encoding) => {
      assert.equal(name, file); assert.equal(encoding, "utf8");
      if (++reads === 1) throw Object.assign(new Error("owned file is still being published"), { code: "EBUSY" });
      return readFile(name, encoding);
    });
    assert.equal(endpoint, `http://127.0.0.1:${port}`);
    assert.equal(reads, 2);
    assert.equal(requests, 1, "sharing refusal cannot select or probe another debugger");
  } finally { await close(browser); await rm(profile, { recursive: true, force: true }); }
});

test("debugger sharing retries keep the original deadline and do not mask permission failures", async () => {
  let reads = 0;
  await assert.rejects(ownedDebuggerEndpoint("private-profile", () => null, 60, async () => {
    reads++; throw Object.assign(new Error("busy"), { code: "EBUSY" });
  }), /private Edge profile/);
  assert.ok(reads > 0 && reads <= 4, "retrying never renews the original bounded wait");
  const denied = Object.assign(new Error("not permitted"), { code: "EACCES" });
  await assert.rejects(ownedDebuggerEndpoint("private-profile", () => null, 60, async () => { throw denied; }), error => error === denied);
});
