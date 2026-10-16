import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { JamTakeSuppression } from "./jam-take-suppression.ts";
import { deferred } from "./studio-testkit.ts";
const takeA = { entry: "entry-a", cid: "a".repeat(64), src: "", link: "" };
const audioB = { entry: "entry-b", cid: "b".repeat(64), src: "", link: "" };

test("pending-download Deafen ABA suppresses the actual deck CID before any transport seats", async () => {
  const gate = new JamTakeSuppression(), download = deferred<void>(); gate.reconcile(takeA);
  let starts = 0;
  const completion = download.promise.then(() => { if (gate.allows(takeA.cid)) starts++; });
  gate.suppress(null, takeA.cid); // Deafen on: jamPlay and jamPendingPlay are still absent.
  gate.suppress(null, takeA.cid); // Deafen off does not restore consent.
  gate.reconcile({ ...takeA }); // Same-track heartbeat is not a new local gesture.
  download.resolve(); await completion; assert.equal(starts, 0);
  gate.localGesture(); assert.equal(gate.allows(takeA.cid), true);
});

test("one cancellation while a replacement deck take is still preparing covers it and the outgoing transport", async () => {
  const takeC = { entry: "entry-c", cid: "c".repeat(64), src: "", link: "" };
  const gate = new JamTakeSuppression(), download = deferred<void>(); gate.reconcile(takeA);
  // takeA's transport is still the active CID when the deck moves to takeC, which starts loading.
  gate.reconcile(takeC);
  let starts = 0;
  const completion = download.promise.then(() => { if (gate.allows(takeC.cid)) starts++; });
  // A single edge (Studio Play, Deafen on, membership, suspended context) with distinct CIDs.
  gate.suppress(takeA.cid, takeC.cid);
  gate.reconcile({ ...takeC }); // Same-track heartbeat is not a new local gesture.
  download.resolve(); await completion;
  assert.equal(starts, 0, "the adopted CID seats no transport after cancellation");
  assert.equal(gate.allows(takeA.cid), false, "the outgoing CID stays cancelled too");
  gate.localGesture(); assert.equal(gate.allows(takeC.cid), true);
});

test("paused or approval-pending take stays suppressed through remote unpause and heartbeats", () => {
  const gate = new JamTakeSuppression(); gate.reconcile(takeA); gate.suppress(null, takeA.cid);
  for (let i = 0; i < 4; i++) gate.reconcile({ ...takeA }); // pause/DJ/offset are intentionally not identity fields.
  assert.equal(gate.allows(takeA.cid), false);
});

test("local Studio Play or membership retirement while downloading cannot be overtaken by completion", () => {
  for (const action of ["Studio Play", "membership edge"]) {
    const gate = new JamTakeSuppression(); gate.reconcile(takeA); gate.suppress(null, takeA.cid);
    assert.equal(gate.allows(takeA.cid), false, action);
  }
});

test("an intervening ordinary audio/video/linked track clears an obsolete take suppression", () => {
  for (const other of [audioB, { ...audioB, cid: "", src: "youtube", link: "https://example.invalid/video" }]) {
    const gate = new JamTakeSuppression(); gate.reconcile(takeA); gate.suppress(null, takeA.cid);
    gate.reconcile(other); gate.reconcile(takeA); assert.equal(gate.allows(takeA.cid), true);
  }
});

test("App binds cancellation to adopted deck CID and reconciles every track before media dispatch", () => {
  const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  const suppress = source.slice(source.indexOf("function suppressCurrentTake()"), source.indexOf("async function jamStartTakePlayback("));
  assert.match(suppress, /jamTakeSuppression\.suppress\(jamDeckPlaybackCid\(\), jukeNow\?\.cid \|\| null\)/);
  const adopt = source.slice(source.indexOf("function jukeAdopt("), source.indexOf("async function jukeApply("));
  const reconcile = adopt.indexOf("jamTakeSuppression.reconcile(jukeNow)"), dispatch = adopt.indexOf("void jukeApply(same)");
  assert.ok(reconcile >= 0 && dispatch > reconcile, "all actual deck track kinds reconcile before their media path begins");
  const start = source.slice(source.indexOf("async function jamStartTakePlayback("), source.indexOf("function jamPlayTake("));
  assert.match(start, /!jamTakeSuppression\.allows\(deckCid\)/);
});
