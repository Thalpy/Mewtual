import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import type { JamTakeEvent } from "./jam-contract.ts";
import { shouldDispatchTakeEvent } from "./jam-playback.ts";

const noteOn: JamTakeEvent = { ms: 0, lane: 0, n: 60, on: 1, q: 1, w: "triangle" };
const noteOff: JamTakeEvent = { ms: 100, lane: 0, n: 60, on: 0, q: 2 };
const drum: JamTakeEvent = { ms: 50, lane: 0, n: 0, d: 1, q: 3 };

test("deafened jukebox takes drop attacks but preserve note-off cleanup", () => {
  assert.equal(shouldDispatchTakeEvent(noteOn, true, true), false);
  assert.equal(shouldDispatchTakeEvent(drum, true, true), false);
  assert.equal(shouldDispatchTakeEvent(noteOff, true, true), true);
});

test("ordinary playback dispatches every recorded event", () => {
  for (const event of [noteOn, noteOff, drum]) {
    assert.equal(shouldDispatchTakeEvent(event, true, false), true);
    assert.equal(shouldDispatchTakeEvent(event, false, true), true);
  }
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
  assert.match(body, /if \(jamPlay === pl\) jamStopPlayback\(\)/);
});
