// Recovers a node_modules that npm left half-written. The failure it undoes: a dev server
// (usually a leaked vite from scripts/flow-check.mjs) still holds node_modules/@esbuild/
// win32-x64/esbuild.exe open, npm hits EPERM partway through unlink, and aborts. What is
// left has no .bin directory and no .package-lock.json, so every "npm run <anything that
// resolves a binary>" dies with "'x' is not recognized" and npm ci cannot self-heal because
// the same lock is still held.
//
// So: kill the holders first, then wipe, then reinstall. Killing is scoped to processes that
// run from THIS project's node_modules, plus node processes whose command line points into
// this project. Anything else on the machine (other repos' dev servers, MCP servers, the
// editor) is left alone, as is this script's own ancestry.
//
// Usage: npm run fix-brick   (from apps/desktop)

import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import process from "node:process";

// join/resolve rather than hand-built separators: this file is read on Windows, where a
// stray backslash in a template literal is an escape sequence, not a path.
const ROOT = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const MODULES = join(ROOT, "node_modules");
const isWindows = process.platform === "win32";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const lower = (s) => (s ?? "").toLowerCase();

// Every process on the box, as {pid, ppid, name, exe, cmd}. Win32_Process is the only view
// that carries both ExecutablePath and CommandLine, and we need both: esbuild is caught by
// its path, a shell-wrapped vite only by its command line.
function snapshot() {
  if (!isWindows) {
    const out = execFileSync("ps", ["-eo", "pid=,ppid=,comm=,args="], { encoding: "utf8" });
    return out.split("\n").filter(Boolean).map((line) => {
      const m = line.trim().match(/^(\d+)\s+(\d+)\s+(\S+)\s*(.*)$/);
      return m ? { pid: +m[1], ppid: +m[2], name: m[3], exe: m[3], cmd: m[4] } : null;
    }).filter(Boolean);
  }
  const ps = "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CommandLine | ConvertTo-Json -Compress";
  const out = execFileSync("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", ps], {
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  const parsed = JSON.parse(out);
  return (Array.isArray(parsed) ? parsed : [parsed]).map((p) => ({
    pid: p.ProcessId,
    ppid: p.ParentProcessId,
    name: p.Name,
    exe: p.ExecutablePath,
    cmd: p.CommandLine,
  }));
}

// npm run fix-brick puts this script's own cmdline inside the project, so it matches the same
// filter the leaked servers do. Walking up from ourselves keeps us from killing our own npm.
function ancestry(procs) {
  const byPid = new Map(procs.map((p) => [p.pid, p]));
  const safe = new Set();
  for (let cur = process.pid; cur && !safe.has(cur); ) {
    safe.add(cur);
    cur = byPid.get(cur)?.ppid;
  }
  return safe;
}

function holders(procs) {
  const safe = ancestry(procs);
  const root = lower(ROOT);
  return procs.filter((p) => {
    if (safe.has(p.pid)) return false;
    if (lower(p.exe).startsWith(lower(MODULES))) return true;
    // A shell-wrapped vite runs from the global node.exe, so only its cmdline gives it away.
    const cmd = lower(p.cmd);
    return cmd.includes(root) && (cmd.includes("vite") || cmd.includes("esbuild") || cmd.includes("rollup"));
  });
}

function kill(pid) {
  const r = isWindows
    ? spawnSync("taskkill", ["/F", "/T", "/PID", String(pid)], { stdio: "ignore" })
    : spawnSync("kill", ["-9", String(pid)], { stdio: "ignore" });
  return r.status === 0;
}

const found = holders(snapshot());
if (!found.length) {
  console.log("no dev servers holding node_modules");
} else {
  for (const p of found) console.log(`killing ${p.pid} ${p.name} ${p.exe ?? p.cmd}`);
  // Parents first: taskkill /T takes the children with them, so a vite kill usually
  // collects its own esbuild and the second attempt is a harmless no-op.
  for (const p of found) kill(p.pid);
  await sleep(500);
  const stragglers = holders(snapshot());
  for (const p of stragglers) kill(p.pid);
  if (holders(snapshot()).length) {
    console.error("FAIL some processes survived; reboot or check for an antivirus lock");
    process.exit(1);
  }
  console.log(`ok killed ${found.length}`);
}

if (existsSync(MODULES)) {
  // Windows releases handles lazily, so a first rm can still lose a race with a dying process.
  try {
    rmSync(MODULES, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
    console.log("ok removed node_modules");
  } catch (e) {
    console.error(`FAIL could not remove node_modules: ${e.message}`);
    process.exit(1);
  }
}

console.log("running npm ci");
const ci = spawnSync("npm", ["ci"], { cwd: ROOT, stdio: "inherit", shell: true });
if (ci.status !== 0) process.exit(ci.status ?? 1);

// The whole point of the exercise: a resolvable binary. If .bin is still empty the install
// silently half-failed again and the next build would give the same confusing message.
const bin = join(MODULES, ".bin", isWindows ? "tauri.cmd" : "tauri");
if (!existsSync(bin)) {
  console.error("FAIL npm ci finished but .bin/tauri is missing");
  process.exit(1);
}
console.log("ok node_modules restored; tauri resolves");
