import assert from "node:assert/strict";
import test from "node:test";
import { EventEmitter } from "node:events";
import { resolve } from "node:path";
import { ownedRtcBrowserCleanup, installRtcBrowserSignals } from "./voice-rtc-browser-process.mjs";
const turn = () => new Promise(yes => setImmediate(yes));
function fixture(alreadyExited = false, killHook = () => {}, waitMs = 5000) {
  const child = Object.assign(new EventEmitter(), { pid: 1234, exitCode: alreadyExited ? 0 : null, signalCode: null });
  let killed = 0, removed = 0, closed = 0;
  const target = resolve("target"), profile = resolve(target, "voice-rtc-edge-owned");
  const server = { closeAllConnections() {}, close(done) { closed++; done(); } };
  const cleanup = ownedRtcBrowserCleanup({ getChild: () => child, getSocket: () => ({ close() {} }), server, profile, target,
    kill(original) { assert.equal(original, child); killed++; killHook(); },
    remove() { assert.notEqual(child.exitCode, null, "profile follows actual exit"); removed++; }, waitMs,
  });
  return { child, cleanup, counts: () => ({ killed, removed, closed }), exit() { child.exitCode = 0; child.emit("exit", 0); } };
}
test("RTC cleanup coalesces repeated calls and joins actual child exit before removal", async () => {
  const f = fixture(), first = f.cleanup(); assert.equal(f.cleanup(), first);
  await turn(); assert.deepEqual(f.counts(), { killed: 1, removed: 0, closed: 0 });
  f.exit(); await first; await f.cleanup(); assert.deepEqual(f.counts(), { killed: 1, removed: 1, closed: 1 });
});
test("an already exited browser never causes a kill against its stale PID", async () => {
  const f = fixture(true); await f.cleanup(); assert.deepEqual(f.counts(), { killed: 0, removed: 1, closed: 1 });
});
test("failed kill/exit cleanup fails visibly and preserves the profile", async () => {
  const f = fixture(false, () => { throw new Error("kill refused"); }, 20);
  await assert.rejects(f.cleanup(), /kill refused/); assert.deepEqual(f.counts(), { killed: 1, removed: 0, closed: 1 });
});
for (const signal of ["SIGINT", "SIGTERM"]) test(`${signal} joins SAME cleanup once, including repeated interruption`, async () => {
  const f = fixture(), host = new EventEmitter(), codes = []; host.exit = code => codes.push(code);
  const remove = installRtcBrowserSignals(f.cleanup, host, error => assert.fail(String(error)));
  host.emit(signal); host.emit(signal); await turn();
  assert.deepEqual(f.counts(), { killed: 1, removed: 0, closed: 0 }); assert.deepEqual(codes, []);
  f.exit(); await f.cleanup(); await turn(); assert.deepEqual(codes, [signal === "SIGINT" ? 130 : 143]);
  assert.deepEqual(f.counts(), { killed: 1, removed: 1, closed: 1 });
  remove(); assert.equal(host.listenerCount("SIGINT"), 0); assert.equal(host.listenerCount("SIGTERM"), 0);
});
