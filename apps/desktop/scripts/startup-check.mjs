// Does the application actually start?
//
// Every other gate in this repo tests functions. `cargo test` never constructs a Tauri app or runs
// its `setup`; clippy cannot know which thread a call happens on; the flow check drives the visual
// fixture in a browser and never launches the binary at all. So a panic during startup passed every
// gate and was found by a person running the app and getting exit code 101.
//
// The one that got through: `tokio::spawn` called from `setup`, which runs on the main thread
// before the async runtime is entered. "there is no reactor running". No window, no log past the
// session line, and nothing in CI with an opinion about it.
//
// This launches the built binary and waits for the marker `setup` writes on its last line. It does
// *not* assert that the process stays alive: it opens a real window, and somebody closing that
// window is not a failure. What is a failure is never reaching the end of `setup`, or saying
// "panicked" on the way.
//
// It opens a window while it runs. That is the cost of testing the thing rather than a model of it.
//
// Usage: node scripts/startup-check.mjs   (build first: cargo build --manifest-path src-tauri/…)

import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
import process from "node:process";

// `fileURLToPath`, not the URL's `pathname`: this repository lives under a directory with a space
// in its name, and a raw pathname keeps it percent-encoded.
const ROOT = fileURLToPath(new URL("..", import.meta.url));
const EXE = join(ROOT, "src-tauri", "target", "debug", "mewtual-desktop.exe");
/** How long startup gets to finish. Generous: a cold debug build is slow to open its first window. */
const STARTUP_MS = Number(process.env.STARTUP_WAIT_MS ?? 20000);

/** The last line `setup` writes. Reaching it is the whole assertion. */
const COMPLETE = "STARTUP.SETUP.COMPLETE";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

if (!existsSync(EXE)) {
  console.error(`FAIL no binary at ${EXE}\n  build it first: cargo build --manifest-path src-tauri/Cargo.toml --no-default-features`);
  process.exit(1);
}

/// A dev build has its `devUrl` compiled in, so the frontend has to be on that exact port or the
/// window has nothing to load and the process exits before it has proved anything about `setup`.
const DEV_PORT = 1420;

async function serving() {
  try {
    return (await fetch(`http://localhost:${DEV_PORT}/`)).ok;
  } catch {
    return false;
  }
}

// Reuse a dev server that is already up rather than fighting it for the port: somebody running
// `npm run dev` in another window is the ordinary case, not an error.
let vite = null;
if (await serving()) {
  console.log(`using the dev server already on ${DEV_PORT}`);
} else {
  vite = spawn("npm", ["run", "dev", "--", "--port", String(DEV_PORT), "--strictPort"], {
    cwd: ROOT,
    stdio: "ignore",
    shell: true,
  });
  let up = false;
  for (let i = 0; i < 80 && !up; i += 1) {
    await sleep(500);
    up = await serving();
  }
  if (!up) {
    console.error("FAIL the dev server never came up, so the binary had nothing to load");
    killTree(vite.pid);
    process.exit(1);
  }
}

function killTree(pid) {
  if (!pid) return;
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/F", "/T", "/PID", String(pid)], { stdio: "ignore" });
  } else {
    try { process.kill(pid, "SIGKILL"); } catch { /* already gone */ }
  }
}

const child = spawn(EXE, [], { stdio: ["ignore", "pipe", "pipe"] });
let output = "";
let exited = null;
child.stdout.on("data", (d) => (output += d));
child.stderr.on("data", (d) => (output += d));
child.on("exit", (code) => (exited = code));

// Wait for the marker, or for the process to give up trying to produce one.
const deadline = Date.now() + STARTUP_MS;
while (Date.now() < deadline && !output.includes(COMPLETE) && exited === null) {
  await sleep(200);
}
// A process can write the marker and exit in the same breath; give the pipe a moment to catch up.
await sleep(300);

// Kill the tree: the webview spawns helper processes that outlive a bare kill.
if (exited === null) {
  killTree(child.pid);
  await sleep(300);
}
if (vite) killTree(vite.pid);

const completed = output.includes(COMPLETE);
const panicked = /panicked at/.test(output);
if (!completed || panicked) {
  console.error("FAIL startup did not finish");
  if (panicked) console.error("  it panicked");
  else if (exited !== null) console.error(`  it exited with code ${exited} before finishing setup`);
  else console.error(`  it never reached the end of setup within ${STARTUP_MS}ms`);
  const lines = output.split(/\r?\n/).filter((l) => l.trim()).slice(-12);
  if (lines.length) console.error(`  it said:\n    ${lines.join("\n    ")}`);
  process.exit(1);
}

console.log("ok application startup completed");
process.exit(0);
