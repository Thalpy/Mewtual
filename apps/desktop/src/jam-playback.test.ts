import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import type { JamPatch, JamTake, JamTakeEvent } from "./jam-contract.ts";
import {
  decodeJamTakeBase64,
  jamTakePlaybackLeaseCurrent,
  JamTakeCache,
  JamTakeLoadCoordinator,
  mayFetchJamTake,
  shouldApplyJamTakeProgress,
  shouldDispatchTakeEvent,
  takeDueBatchEnd,
  takePlaybackIsRemote,
  takeReleaseTailMs,
} from "./jam-playback.ts";
import { JAM_TAKE_CACHE_MAX, TAKE_MAX_BYTES, TAKE_PLAYBACK_EVENTS_PER_TICK } from "./jam-contract.ts";

const noteOn: JamTakeEvent = { ms: 0, lane: 0, n: 60, on: 1, q: 1, w: "triangle" };
const noteOff: JamTakeEvent = { ms: 100, lane: 0, n: 60, on: 0, q: 2 };
const drum: JamTakeEvent = { ms: 50, lane: 0, n: 0, d: 1, q: 3 };

test("take progress is bound to the exact call, server, CID, and cancellation token", () => {
  const active = { callLease: 12, server: 2, cid: "same-cid", cancellation: "jam:12:4" };
  assert.equal(shouldApplyJamTakeProgress(
    active,
    { server: 2, cid: "same-cid", cancellation: "jam:11:4" },
    12,
  ), false, "an old call token cannot mutate a replacement deck with the same CID");
  assert.equal(shouldApplyJamTakeProgress(
    active,
    { server: 1, cid: "same-cid", cancellation: "jam:12:4" },
    12,
  ), false, "another server cannot leak its provider into this deck");
  assert.equal(shouldApplyJamTakeProgress(
    active,
    { server: 2, cid: "same-cid", cancellation: null },
    12,
  ), false, "an unscoped compatibility download cannot impersonate take progress");
  assert.equal(shouldApplyJamTakeProgress(
    active,
    { server: 2, cid: "same-cid", cancellation: "jam:12:4" },
    12,
  ), true);
  assert.equal(shouldApplyJamTakeProgress(active, {
    server: 2, cid: "same-cid", cancellation: "jam:12:4",
  }, 13), false, "a remounted call lease invalidates queued native progress");
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}

test("take loads coalesce and an old call cannot apply into a new call with the same CID", async () => {
  const coordinator = new JamTakeLoadCoordinator<string>();
  const fetchA = deferred<string>();
  let fetches = 0;
  const leaseA = { callLease: 11, server: 1, channel: "general", cid: "abc" };
  let current = { inCall: true, callLease: 11, server: 1 as number | null, channel: "general", cid: "abc" as string | null };
  const first = coordinator.submit("abc", async (coordinatorCurrent) => {
    fetches += 1;
    const value = await fetchA.promise;
    return coordinatorCurrent() && jamTakePlaybackLeaseCurrent(leaseA, current) ? value : "stale";
  }, () => undefined);
  const duplicates = Array.from({ length: 100 }, () =>
    coordinator.submit("abc", () => { fetches += 1; return "duplicate"; }, () => undefined));
  assert.deepEqual(await Promise.all(duplicates), new Array(100).fill(null),
    "same-deck announcements retain neither another task nor another waiter");

  coordinator.invalidate();
  current = { inCall: true, callLease: 12, server: 2, channel: "general", cid: "abc" };
  const replacement = coordinator.submit("abc", () => "call-b", () => undefined);
  fetchA.resolve("call-a");
  assert.equal(await first, null, "acknowledged cancellation retires the stale consumer");
  assert.equal(await replacement, "call-b");
  assert.equal(fetches, 1);
  assert.equal(jamTakePlaybackLeaseCurrent(leaseA, current), false, "matching CID is not a call lease");
});

test("an acknowledged cancellation preempts an old load whose promise never settles", async () => {
  const coordinator = new JamTakeLoadCoordinator<string>();
  const never = new Promise<string>(() => {});
  let cancellationRequests = 0;
  let replacementStarted = false;
  void coordinator.submit("old", () => never, async () => {
    cancellationRequests += 1;
  });
  await Promise.resolve();

  coordinator.invalidate();
  const replacement = coordinator.submit("new", () => {
    replacementStarted = true;
    return "new-call";
  }, () => undefined);

  assert.equal(await replacement, "new-call");
  assert.equal(replacementStarted, true);
  assert.equal(cancellationRequests, 1);
});

test("deafened jukebox takes drop attacks but preserve note-off cleanup", () => {
  assert.equal(shouldDispatchTakeEvent(noteOn, true), false);
  assert.equal(shouldDispatchTakeEvent(drum, true), false);
  assert.equal(shouldDispatchTakeEvent(noteOff, true), true);
});

test("ordinary playback dispatches every event while Deafen is open", () => {
  for (const event of [noteOn, noteOff, drum]) {
    assert.equal(shouldDispatchTakeEvent(event, false), true);
  }
});

test("only shared-deck playback carries remote audio provenance", () => {
  assert.equal(takePlaybackIsRemote(null), false);
  assert.equal(takePlaybackIsRemote("content-id"), true);
});

test("a dense overdue take drains through bounded scheduler passes", () => {
  const events = Array.from({ length: 20_000 }, (_, q): JamTakeEvent => ({
    ms: 0,
    lane: 0,
    n: q % 10,
    d: 1,
    q,
  }));
  const first = takeDueBatchEnd(events, 0, 1_000);
  assert.equal(first, TAKE_PLAYBACK_EVENTS_PER_TICK);
  assert.equal(takeDueBatchEnd(events, first, 1_000), TAKE_PLAYBACK_EVENTS_PER_TICK * 2);
});

test("take teardown honours the longest bounded patch and drum tail", () => {
  const patch: JamPatch = {
    v: 1,
    o: [{ w: 0, t: 0, c: 0, l: 100 }],
    e: { a: 0, d: 0, s: 100, r: 8_000 },
    f: { m: 0, c: 1_000, q: 0, e: 0 },
    l: { r: 100, d: 0, t: 0 },
    x: { c: 0, d: 0, r: 0 },
  };
  const take: JamTake = {
    v: 1,
    group: "g",
    call: "c",
    met: { bpm: 120, bpb: 4 },
    parts: ["alice"],
    lanes: [{ src: 0, sn: "00000000000000aa" }],
    patches: [patch],
    events: [
      { ms: 0, lane: 0, n: 60, on: 1, w: "sine", p: 0, q: 1 },
      { ms: 1, lane: 0, n: 9, d: 1, q: 2 },
    ],
  };
  assert.equal(takeReleaseTailMs(take), 8_000);
});

test("a finished take cannot leave a stale timer that stops its replacement", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("function jamPlayTick()");
  const end = source.indexOf("function jamStopPlayback()", start);
  assert.ok(start >= 0 && end > start);
  const body = source.slice(start, end);
  assert.match(body, /if \(pl\.draining\) return;/, "overlapping timer ticks must not race the async drain");
  assert.match(body, /await engine\.drum\(/, "dense take playback must backpressure async drum hashing");
  assert.match(body, /event\.ms <= pl\.dropAttacksThroughMs/,
    "attacks due during a completed Deafen interval must remain suppressed after reopen");
  const release = body.indexOf("engine.releaseChannelHeld(chan)");
  const arm = body.indexOf("pl.endTimer = setTimeout(() => {");
  const advance = body.indexOf("jukeAdvance(true)");
  assert.ok(release >= 0 && release < arm, "unmatched held notes must enter release before tail cleanup is armed");
  assert.ok(arm >= 0 && advance > arm, "tail cleanup must be armed before a cached next take can start");
  assert.match(body, /if \(jamPlay !== pl\) return;/);
  assert.match(body, /else \{\s*jamStopPlayback\(\);/);
});

test("local note and drum echoes share one causal render lane", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("function renderLocalJamEvent(");
  const end = source.indexOf("// (Remote voices live", start);
  assert.ok(start >= 0 && end > start);
  const body = source.slice(start, end);
  assert.equal(body.match(/renderLocalJamEvent\(engine, channel/g)?.length, 3,
    "note-on, note-off and drum must not overtake each other around an async seed");
  assert.match(body, /roomGeneration === jamRoomRenderGeneration/,
    "queued local events must retain the room gate epoch from admission");
  assert.equal(body.match(/const roomGeneration = jamRoomRenderGeneration;/g)?.length, 3,
    "each local gesture must capture its room gate epoch before patch publication can await");
});

test("Deafen advances the App gate epoch and marks the take interval on both toggle edges", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("function toggleDeafen()");
  const end = source.indexOf("// Flood control moved", start);
  assert.ok(start >= 0 && end > start);
  const body = source.slice(start, end);
  assert.match(body, /callDeafened = !callDeafened;[\s\S]*?jamRoomRenderGeneration \+= 1;/);
  assert.match(body, /jamPlay\.dropAttacksThroughMs = Math\.max\(jamPlay\.dropAttacksThroughMs, jamPlayElapsed\(\)\)/);
});

test("source mute advances the App admission epoch on both gate edges", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("function jamApplyMutes(");
  const end = source.indexOf("function clearPeerJamUi(", start);
  assert.ok(start >= 0 && end > start);
  const body = source.slice(start, end);
  assert.match(body, /const wasMuted = jamSourceGateMuted\[fp\] \?\? false;/);
  assert.match(body, /if \(muted !== wasMuted\)[\s\S]*?jamSourceRenderGeneration\[fp\]/,
    "frames received before or during mute must not revive after either gate transition");
});

test("stopping an empty deck clears stale take-load progress before the media early return", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("function jukeStop()");
  const end = source.indexOf("// Controls.", start);
  assert.ok(start >= 0 && end > start);
  const body = source.slice(start, end);
  const clear = body.indexOf("jukeFetch = null");
  const earlyReturn = body.indexOf("if (!el) return");
  assert.ok(clear >= 0 && clear < earlyReturn, "take progress must clear even when no media element exists");
});

test("the deck never treats unscoped progress as progress for a take", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf('}>("download-progress",');
  const end = source.indexOf("const d = downloads[", start);
  assert.ok(start >= 0 && end > start);
  const body = source.slice(start, end);
  assert.match(body, /currentMediaRead = e\.payload\.cancellation == null[\s\S]*?jukeKind !== "take"/,
    "a same-server omitted-token read must not overwrite token-bound take provenance");
});

test("take cancellation ids include a per-WebView nonce before resettable counters", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  assert.match(source, /const jukeTakeCancellationNonce = jamNonce\(\);/);
  assert.match(source,
    /`jam:\$\{jukeTakeCancellationNonce\}:\$\{callLease\}:\$\{jukeTakeCancellationSeq\}`/,
    "a reload must not reuse an active predecessor's native cancellation id");
});

test("take ingress rejects oversized listings and encoded payloads before allocation", () => {
  assert.equal(mayFetchJamTake(TAKE_MAX_BYTES), true);
  assert.equal(mayFetchJamTake(TAKE_MAX_BYTES + 1), false);
  let decodes = 0;
  const oversized = "A".repeat(Math.ceil(TAKE_MAX_BYTES / 3) * 4 + 1);
  assert.equal(decodeJamTakeBase64(oversized, () => { decodes += 1; return ""; }), null);
  assert.equal(decodes, 0, "encoded-size rejection happens before base64 decoding");
});

test("validated take cache is a bounded LRU", () => {
  const cache = new JamTakeCache();
  const take = { v: 1 } as JamTake;
  for (let index = 0; index < JAM_TAKE_CACHE_MAX; index += 1) cache.set(`cid-${index}`, take);
  assert.equal(cache.get("cid-0"), take); // touch oldest
  cache.set("cid-new", take);
  assert.equal(cache.get("cid-1"), undefined);
  assert.equal(cache.get("cid-0"), take);
});
