import assert from "node:assert/strict";
import test from "node:test";
import { parseStudioSyncState } from "./studio-native.ts";
import { StudioSession } from "./studio-session.ts";
import { CHANNEL, ME, SERVER, deferred, fakeIpc, indexContent, ordinaryView, settle } from "./studio-testkit.ts";

const event = (sequence: string, paused: boolean, instance = "1") => ({ server: SERVER, instance, sequence, paused,
  cause: paused ? "background" : null, detail: paused ? "Inventory could not be read" : null });
async function harness() {
  const ipc = fakeIpc(), tasks: (() => void)[] = [];
  let nonce = 1;
  const session = new StudioSession({ ipc, me: ME, now: () => 1000, newId: () => (++nonce).toString(16).padStart(32, "0"),
    schedule: task => { tasks.push(task); return () => { const index = tasks.indexOf(task); if (index >= 0) tasks.splice(index, 1); }; } });
  ipc.on("studio_list", () => ordinaryView(indexContent()));
  await session.attach(); session.setScope({ server: SERVER, channel: CHANNEL });
  const flush = async () => { for (let i = 0; i < 4; i++) { for (const task of tasks.splice(0)) task(); await settle(2); } };
  await flush(); return { ipc, session, flush };
}

test("sync observations validate lossless ordering and bounded truthful diagnostics", () => {
  assert.equal(parseStudioSyncState(event("18446744073709551615", true)).sequence, "18446744073709551615");
  for (const invalid of [event("01", true), event("18446744073709551616", true), event("9".repeat(10000), true),
    { ...event("1", true), detail: "x".repeat(2049) }, { ...event("1", false), cause: "background" }, { ...event("1", true), cause: "storageCorruption" }]) {
    assert.throws(() => parseStudioSyncState(invalid));
  }
});

test("a late pause cannot undo an authoritative resume across channel and server navigation", async () => {
  const h = await harness();
  assert.equal(h.session.receivePaused, false, "healthy new scope does not default to paused");
  h.ipc.emit("studio-sync-state", event("1", true)); assert.equal(h.session.receivePaused, true);
  h.ipc.emit("studio-sync-state", event("2", false)); assert.equal(h.session.receivePaused, false);
  h.session.setScope({ server: SERVER, channel: "42" });
  h.ipc.emit("studio-sync-state", event("1", true)); h.ipc.emit("studio-receive-paused", { server: SERVER });
  assert.equal(h.session.receivePaused, false); assert.equal(h.session.receivePauseDetail, "");
  h.session.setScope({ server: SERVER + 1, channel: "1" });
  h.ipc.emit("studio-sync-state", {...event("1", true, "2"), server: SERVER + 1});
  h.session.setScope({ server: SERVER, channel: CHANNEL });
  h.ipc.emit("studio-sync-state", event("1", true)); h.ipc.emit("studio-receive-paused", {server: SERVER});
  assert.equal(h.session.receivePaused, false); assert.equal(h.session.receivePauseDetail, "");
  h.session.detach();
});

test("an old successful Read cannot hide a newer actual pause; failed Retry keeps the cause", async () => {
  const h = await harness(), pending = deferred<unknown>();
  h.ipc.emit("studio-sync-state", event("2", false));
  h.ipc.on("studio_list", () => pending.promise);
  const read = h.session.refreshIndex();
  h.ipc.emit("studio-sync-state", event("3", true));
  pending.resolve(ordinaryView(indexContent())); await read;
  assert.equal(h.session.receivePaused, true); assert.equal(h.session.receivePauseDetail, "Inventory could not be read");
  h.ipc.on("studio_list", () => { throw new Error("disk unavailable"); }); await h.session.refreshIndex();
  assert.equal(h.session.receivePaused, true);
  h.ipc.emit("studio-sync-state", event("4", false)); assert.equal(h.session.receivePaused, false);
  h.session.detach();
});

test("a replaced actor starts a new ordered state and old actors cannot relatch it", async () => {
  const h = await harness();
  h.ipc.emit("studio-sync-state", event("90", true));
  h.ipc.emit("studio-sync-state", event("0", false, "2"));
  h.ipc.emit("studio-sync-state", event("91", true));
  assert.equal(h.session.receivePaused, false);
  h.session.reset(); assert.equal(h.session.receivePauseDetail, ""); h.session.detach();
});
