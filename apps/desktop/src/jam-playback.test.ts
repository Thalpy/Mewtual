import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import type { JamPatch, JamTake, JamTakeEvent } from "./jam-contract.ts";
import { decodeJamTakeBase64, JamTakeCache, mayFetchJamTake, shouldDispatchTakeEvent, takeDueBatchEnd, takePlaybackIsRemote, takeReleaseTailMs } from "./jam-playback.ts";
import { JAM_TAKE_CACHE_MAX, TAKE_MAX_BYTES, TAKE_PLAYBACK_EVENTS_PER_TICK } from "./jam-contract.ts";

const noteOn: JamTakeEvent = { ms: 0, lane: 0, n: 60, on: 1, q: 1, w: "triangle" };
const noteOff: JamTakeEvent = { ms: 100, lane: 0, n: 60, on: 0, q: 2 };
const drum: JamTakeEvent = { ms: 50, lane: 0, n: 0, d: 1, q: 3 };

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
  const arm = body.indexOf("pl.endTimer = setTimeout(() => {");
  const advance = body.indexOf("jukeAdvance(true)");
  assert.ok(arm >= 0 && advance > arm, "tail cleanup must be armed before a cached next take can start");
  assert.match(body, /if \(jamPlay !== pl\) return;/);
  assert.match(body, /else \{\s*jamStopPlayback\(\);/);
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
