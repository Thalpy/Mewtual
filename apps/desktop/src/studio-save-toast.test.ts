// [desync fixes]: App's background-save toast (`trackStudioSaves`), run from its production text the
// way studio-exit-routes.test.ts runs App functions: App.svelte cannot be mounted under node:test.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { stripTypeScriptTypes } from "node:module";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { saveToast, type SaveTally } from "./studio-save-words.ts";

const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");

/** App's 2-space function `name`, matched by bracket depth (as in studio-exit-routes.test.ts). */
function functionText(name: string): string {
  const start = source.search(new RegExp(String.raw`\n {2}(?:async )?function ${name}\(`)) + 1;
  assert.ok(start > 0, `${name} not found in App.svelte`);
  const match = (open: string, close: string, from: number) => {
    let depth = 0;
    for (let i = from; i < source.length; i++) { if (source[i] === open) depth++; else if (source[i] === close && --depth === 0) return i; }
    assert.fail(`${name}: unbalanced ${open}${close}`);
  };
  const paramsEnd = match("(", ")", source.indexOf("(", start));
  return source.slice(start, match("{", "}", source.indexOf("{", paramsEnd)) + 1);
}

type Shown = { text: string; kind: string; ms: number; progress: number | null };

function harness(start: Omit<SaveTally, "discarded"> & { discarded?: number }) {
  const first: SaveTally = { discarded: 0, ...start };
  let tally = first, generation = 1, seq = 0;
  const listeners = new Set<() => void>(), shown = new Map<number, Shown>();
  const state = {
    stopStudioSaveWatch: null as (() => void) | null, locked: false, saveToast,
    studioSaveTally: () => ({ tally, generation }),
    onStudioChange: (fn: () => void) => { listeners.add(fn); return () => { listeners.delete(fn); }; },
    get toasts() { return [...shown.keys()].map(id => ({ id })); },
    toast: (text: string, kind: string, ms: number, progress: number | null) => { shown.set(++seq, { text, kind, ms, progress }); return seq; },
    // App's updateToast brings back a dismissed toast; the tracker decides whether to call it.
    updateToast: (id: number, text: string, kind: string, ms: number, progress: number | null) => { shown.set(id, { text, kind, ms, progress }); },
    dismissToast: (id: number) => { shown.delete(id); },
  };
  const body = stripTypeScriptTypes(functionText("trackStudioSaves"), { mode: "strip" });
  const { trackStudioSaves } = new Function("state", `with (state) { const trackStudioSaves = (${body}); return { trackStudioSaves }; }`)(state) as { trackStudioSaves: () => void };
  const change = async (next: Partial<SaveTally> & { generation?: number }) => {
    tally = { ...tally, ...next }; if (next.generation) generation = next.generation;
    for (const fn of [...listeners]) fn();
    for (let i = 0; i < 3; i++) await Promise.resolve(); // the tracker coalesces to a microtask
  };
  return { state, shown, listeners, trackStudioSaves, change };
}

test("the toast follows the saves to the end, its bar only moving forward, and ends saved", async () => {
  const h = harness({ left: 3, retrying: 0, stuck: 0 });
  h.trackStudioSaves();
  assert.deepEqual([...h.shown.values()], [{ text: "Saving Studio edits", kind: "info", ms: 0, progress: 0 }]);
  await h.change({ left: 4 }); // an edit made meanwhile joined the queue
  assert.equal([...h.shown.values()][0].progress, 0);
  await h.change({ left: 1 });
  assert.equal([...h.shown.values()][0].progress, 0.75);
  await h.change({ left: 0 });
  assert.deepEqual([...h.shown.values()], [{ text: "Studio edits saved", kind: "ok", ms: 2500, progress: 1 }]);
  assert.equal(h.listeners.size, 0, "it stops listening once done"); assert.equal(h.state.stopStudioSaveWatch, null);
});

test("a lock or reset ends it silently: an emptied queue was snapshotted, not saved", async () => {
  const h = harness({ left: 2, retrying: 0, stuck: 0 });
  h.trackStudioSaves();
  await h.change({ left: 0, generation: 2 });
  assert.equal(h.shown.size, 0, "no \"saved\" claim after a reset");
  assert.equal(h.listeners.size, 0);
});

// Review M4: leaving while an edit retries, then coming back and discarding it, turned the toast
// green ("Studio edits saved") for an edit that was thrown away.
test("a queue emptied by the member's discard ends the toast silently, never as saved", async () => {
  const h = harness({ left: 1, retrying: 1, stuck: 0, discarded: 4 });
  h.trackStudioSaves();
  await h.change({ left: 0, retrying: 0, discarded: 5 });
  assert.equal(h.shown.size, 0, "no claim at all about an edit that was discarded");
  assert.equal(h.listeners.size, 0);
});

test("a dismissed toast stays gone, except for the warning that an edit will not save on its own", async () => {
  const h = harness({ left: 2, retrying: 0, stuck: 0 });
  h.trackStudioSaves();
  h.state.dismissToast([...h.shown.keys()][0]);
  await h.change({ left: 2, retrying: 1 });
  assert.equal(h.shown.size, 0, "progress does not pop back up");
  await h.change({ left: 1, retrying: 0, stuck: 1 });
  assert.deepEqual([...h.shown.values()].map(t => [t.kind, t.ms]), [["warn", 0]], "the warning comes back and stays");
  assert.match([...h.shown.values()][0].text, /needs you\. Open the Studio to retry or discard it\./);
  assert.equal(h.listeners.size, 0);
});

test("one toast at a time, and none when nothing is outstanding", async () => {
  const h = harness({ left: 2, retrying: 0, stuck: 0 });
  h.trackStudioSaves(); h.trackStudioSaves();
  assert.equal(h.shown.size, 1); assert.equal(h.listeners.size, 1);
  const idle = harness({ left: 0, retrying: 0, stuck: 0 });
  idle.trackStudioSaves();
  assert.equal(idle.shown.size, 0); assert.equal(idle.listeners.size, 0);
});

// Integration preserves the accepted save-before-switch contract. The incoming background
// departure proposal is intentionally not adopted: a rejected Save must keep the editor open.
test("leaving the Studio and posting from it await Save before changing the view", async () => {
  const start = source.indexOf("const studioExit = createStudioExit({");
  const end = source.indexOf("const leaveStudioThen = studioExit.leaveThen;", start);
  assert.ok(start >= 0 && end > start, "extract only the actual App exit host");
  const host = source.slice(start, end);
  assert.match(host, /save: async \(\) => \{\s*try \{ await studioSave\?\.\(\); return true; \}/);
  const callback = host.match(/save: (async \(\) => \{[\s\S]*?\n {4}\})/)?.[1];
  assert.ok(callback);
  const warnings: string[] = [];
  let finish!: () => void;
  const receipt = new Promise<void>(resolve => { finish = resolve; });
  const make = (save: () => Promise<void>) => new Function("studioSave", "toast", "locked",
    `return (${stripTypeScriptTypes(callback)});`)(save, (text: string) => warnings.push(text), false) as () => Promise<boolean>;
  let settled = false;
  const pending = make(() => receipt)().then(result => { settled = true; return result; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(settled, false, "the actual App callback waits for its Save receipt");
  finish(); assert.equal(await pending, true);
  assert.equal(await make(async () => { throw new Error("save refused"); })(), false,
    "an actual App Save refusal vetoes departure");
  assert.match(warnings[0], /save refused/);
  for (const name of ["postStudioPixa", "postStudioScore"]) {
    const text = functionText(name);
    assert.match(text, /await studioSave\?\.\(\);/, `${name} saves before opening Chat`);
  }
  // Window close waits under its own close fence as well.
  assert.match(source, /studioBeginClose\?\.\(\); await studioSave\?\.\(\);/);
  // Saves already sent and retrying leave the gate nothing to hand over, so its `save` never runs:
  // any change of view away from the Studio starts the toast too (found by screenshot, 2026-10-19).
  assert.match(source, /if \(viewLeftFrom === "studio" && now !== "studio" && !locked\) trackStudioSaves\(\);/);
});
