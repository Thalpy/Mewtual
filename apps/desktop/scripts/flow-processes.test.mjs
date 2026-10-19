import test from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { spawn } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
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
  let requests = 0;
  const stranger = createServer((_req, res) => { requests++; res.end("another checkout"); });
  const port = await listen(stranger);
  const child = spawn(process.execPath, [fileURLToPath(new URL("../node_modules/vite/bin/vite.js", import.meta.url)), "--host", "127.0.0.1", "--port", String(port), "--strictPort"], {
    cwd: new URL("..", import.meta.url), stdio: ["ignore", "pipe", "pipe"], windowsHide: true,
  });
  try {
    await assert.rejects(waitForOwnedVite(child, port), /Vite exited/);
    assert.equal(requests, 0, "no request may select the stranger as the tested app");
    assert.equal(await (await fetch(`http://127.0.0.1:${port}`)).text(), "another checkout");
  } finally { child.kill(); await close(stranger); }
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
