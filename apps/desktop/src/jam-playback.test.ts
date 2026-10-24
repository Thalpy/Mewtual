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
  planTakeSeek,
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

function seekTake(events: JamTakeEvent[], lanes = 1): JamTake {
  return {
    v: 1,
    group: "g",
    call: "c",
    met: { bpm: 120, bpb: 4 },
    parts: ["alice", "bob"],
    lanes: Array.from({ length: lanes }, (_u, i) => ({ src: i, sn: `000000000000000${i}` })),
    patches: [],
    events,
  };
}

test("a seek past a note-on re-opens the note it landed inside, at the age it has reached", () => {
  // The original defect, stated as a take: one note held from 0 to 10 s, joined at 5 s. Starting at
  // the first event whose ms reaches the offset leaves nothing due but the key-up, so the listener
  // hears silence for five seconds and then a note-off for a voice that was never opened.
  const take = seekTake([
    { ms: 0, lane: 0, n: 60, on: 1, w: "triangle", q: 1 },
    { ms: 10_000, lane: 0, n: 60, on: 0, q: 2 },
  ]);
  const plan = planTakeSeek(take, 5_000);
  assert.equal(plan.next, 1, "the key-up is still the next scheduled event");
  assert.deepEqual(plan.sounding, [{ index: 0, ageMs: 5_000 }],
    "the note that is sounding at the offset has to come back, five seconds into its life");
});

test("a seek reconstructs only what is still held, per lane and per pitch", () => {
  const take = seekTake([
    { ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 1 }, // closed before the offset
    { ms: 100, lane: 0, n: 62, on: 1, w: "sine", q: 2 }, // still held
    { ms: 200, lane: 1, n: 64, on: 1, w: "square", q: 1 }, // still held, other lane
    { ms: 300, lane: 0, n: 60, on: 0, q: 3 },
    { ms: 400, lane: 0, n: 60, on: 1, w: "sine", q: 4 }, // re-pressed, still held
    { ms: 500, lane: 1, n: 9, d: 1, q: 2 }, // a pad is a one-shot, never revived
    { ms: 900, lane: 0, n: 62, on: 0, q: 5 },
  ], 2);
  const plan = planTakeSeek(take, 600);
  assert.equal(plan.next, 6);
  assert.deepEqual(plan.sounding, [
    { index: 1, ageMs: 500 },
    { index: 2, ageMs: 400 },
    { index: 4, ageMs: 200 },
  ], "the latest un-closed note-on per lane and pitch, in log order");
});

test("a seek emits reconstructed note-ons in an order each lane's sequencing accepts", () => {
  // The engine rejects a sequence at or below the last one it saw on that source, so the revived
  // note-ons have to reach it in the same per-lane order the log had them in, ahead of every event
  // the ordinary scheduler is about to dispatch.
  const take = seekTake([
    { ms: 0, lane: 1, n: 40, on: 1, w: "sine", q: 7 },
    { ms: 10, lane: 0, n: 60, on: 1, w: "sine", q: 3 },
    { ms: 20, lane: 0, n: 67, on: 1, w: "sine", q: 4 },
    { ms: 999, lane: 0, n: 60, on: 0, q: 5 },
  ], 2);
  const plan = planTakeSeek(take, 500);
  const lastPerLane = new Map<number, number>();
  for (const voice of plan.sounding) {
    const event = take.events[voice.index];
    const previous = lastPerLane.get(event.lane);
    assert.ok(previous === undefined || event.q > previous, "sequence must strictly increase per lane");
    lastPerLane.set(event.lane, event.q);
  }
  for (const voice of plan.sounding) {
    assert.ok(voice.index < plan.next, "a revived note-on always precedes the first scheduled event");
    const previous = lastPerLane.get(take.events[voice.index].lane);
    assert.ok(previous !== undefined);
  }
  const scheduled = take.events[plan.next];
  const revivedOnThatLane = plan.sounding
    .filter((v) => take.events[v.index].lane === scheduled.lane)
    .map((v) => take.events[v.index].q);
  assert.ok(revivedOnThatLane.every((q) => q < scheduled.q),
    "everything revived on a lane is below the first event that lane is about to receive");
});

test("playing a take from the start reconstructs nothing", () => {
  const take = seekTake([
    { ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 1 },
    { ms: 500, lane: 0, n: 60, on: 0, q: 2 },
  ]);
  assert.deepEqual(planTakeSeek(take, 0), { next: 0, sounding: [] });
  assert.deepEqual(planTakeSeek(take, -1), { next: 0, sounding: [] }, "a negative offset is the start");
});

test("a seek past the end of a take holds nothing and has nothing left to schedule", () => {
  const take = seekTake([
    { ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 1 },
    { ms: 500, lane: 0, n: 60, on: 0, q: 2 },
  ]);
  const plan = planTakeSeek(take, 60_000);
  assert.equal(plan.next, take.events.length);
  assert.deepEqual(plan.sounding, [], "a note the take already released is not revived by a late seek");
});

test("an unterminated note is revived by a seek past it, because the take never closes it", () => {
  const take = seekTake([{ ms: 0, lane: 0, n: 60, on: 1, w: "sine", q: 1 }]);
  const plan = planTakeSeek(take, 60_000);
  assert.equal(plan.next, take.events.length);
  assert.deepEqual(plan.sounding, [{ index: 0, ageMs: 60_000 }]);
});

test("App starts a seeked take by opening what it landed inside, before the first scheduled event", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("async function jamStartTakePlayback(");
  const end = source.indexOf("function jamPlayTake(", start);
  assert.ok(start >= 0 && end > start);
  const body = source.slice(start, end);
  assert.doesNotMatch(body, /take\.events\.findIndex/,
    "a seek is held state plus a schedule, never just the first event at or after the offset");
  assert.match(body, /const plan = planTakeSeek\(take, baseMs\);/);
  assert.match(body, /next: plan\.next,/);
  const revive = body.indexOf("jamOpenSeekVoices(jamPlay, engine, plan.sounding)");
  // The last one: the earlier match is the interval callback being armed, which is a closure and
  // cannot run before this synchronous run finishes. What matters is the first actual dispatch.
  const firstTick = body.lastIndexOf("void jamPlayTick()");
  assert.ok(revive >= 0 && firstTick > revive,
    "revived note-ons must reach the engine before the scheduler dispatches this lane's next event");

  const opener = source.indexOf("function jamOpenSeekVoices(");
  assert.ok(opener > 0);
  const openerBody = source.slice(opener, source.indexOf("function jamPlayElapsed(", opener));
  assert.match(openerBody, /shouldDispatchTakeEvent\(event, callDeafened\)/,
    "a revived attack is still an attack: Deafen must drop it exactly as it drops a scheduled one");
  assert.match(openerBody, /ageMs: voice\.ageMs/,
    "a revived voice starts at the age it has reached, not at the start of its attack");
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
