// Test-harness ownership: normal completion and signals share an actual exit join.
import { spawnSync } from "node:child_process";
import { rmSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";
const live = child => child?.pid && child.exitCode === null && child.signalCode === null;
function terminate(child) {
  if (!live(child)) return;
  if (process.platform === "win32") {
    const result = spawnSync("taskkill", ["/F", "/T", "/PID", String(child.pid)], {
      windowsHide: true, stdio: "ignore", timeout: 10_000,
    });
    if (result.error || result.status !== 0) throw new Error("Owned RTC browser kill failed");
  } else if (!child.kill("SIGKILL")) throw new Error("Owned RTC browser kill refused");
}
function exited(child, milliseconds) {
  if (!live(child)) return Promise.resolve();
  return new Promise((yes, no) => {
    const finish = () => { clearTimeout(timer); child.removeListener("exit", finish); yes(); };
    const timer = setTimeout(() => { child.removeListener("exit", finish); no(new Error("Owned RTC browser did not exit")); }, milliseconds);
    child.once("exit", finish);
  });
}
export function ownedRtcBrowserCleanup({ getChild, getSocket, server, profile, target,
    kill = terminate, remove = path => rmSync(path, { recursive: true, force: true }), waitMs = 5000 }) {
  let completion = null;
  return function cleanup() {
    // Publish before any socket/child/server callback can reenter.
    return completion ??= Promise.resolve().then(async () => {
      let failure = null;
      try { getSocket()?.close(); } catch (error) { failure = error; }
      const child = getChild();
      if (live(child)) {
        const joined = exited(child, waitMs); // Register BEFORE requesting termination.
        let refused = null;
        try { kill(child); } catch (error) { refused = error; }
        try { await joined; } catch (error) { failure ??= refused ?? error; }
        // A racing normal exit can make taskkill refuse. Actual exit, rather than
        // command submission, is the oracle; never retry against that old PID.
      }
      try {
        server.closeAllConnections();
        await new Promise((yes, no) => {
          const timer = setTimeout(() => no(new Error("Owned RTC server did not close")), waitMs);
          server.close(error => { clearTimeout(timer); error && error.code !== "ERR_SERVER_NOT_RUNNING" ? no(error) : yes(); });
        });
      } catch (error) { failure ??= error; }
      if (!failure) {
        if (dirname(resolve(profile)) !== resolve(target) || !basename(profile).startsWith("voice-rtc-edge-"))
          throw new Error("RTC profile cleanup escaped its owned target");
        remove(profile);
      }
      if (failure) throw failure;
    });
  };
}
export function installRtcBrowserSignals(cleanup, host = process, report = console.error) {
  let ending = false;
  const end = code => {
    if (ending) return;
    ending = true;
    Promise.resolve().then(cleanup).then(() => host.exit(code), error => { report(error); host.exit(1); });
  };
  const interrupt = () => end(130), terminate = () => end(143);
  host.on("SIGINT", interrupt); host.on("SIGTERM", terminate);
  return () => { host.removeListener("SIGINT", interrupt); host.removeListener("SIGTERM", terminate); };
}
