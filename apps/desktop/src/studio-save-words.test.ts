// The person-facing words for Studio saves (studio-save-words.ts). The field report that prompted
// them showed the bridge's own text chained with separators: "studio_apply_batch not allowed.
// Command not found · attempt 1 · your work is kept · 4 later saves wait behind it".
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { retryText, saveHeadline, saveProblemText, saveToast } from "./studio-save-words.ts";
import { NATIVE_NO_WRITE } from "./studio-session.ts";

/** The withheld-Save text, as native sends it (`UNDELIVERED_SAVE` in src-tauri/src/studio.rs). */
const UNDELIVERED_SAVE_TEXT = "outcome=uncertain; the Save may have completed, but its result could not be delivered; refresh the Studio document, or retry exactly";

test("the withheld-Save text above is still the one native sends", () => {
  // The Rust constant is split across lines with `\` continuations; join them before comparing.
  const rust = readFileSync(new URL("../src-tauri/src/studio.rs", import.meta.url), "utf8");
  const start = rust.indexOf("pub(crate) const UNDELIVERED_SAVE: &str =");
  assert.ok(start > 0, "UNDELIVERED_SAVE still exists");
  const literal = rust.slice(rust.indexOf('"', start) + 1, rust.indexOf('";', start)).replace(/\\\r?\n\s*/g, "");
  assert.equal(literal, UNDELIVERED_SAVE_TEXT);
});

test("native and session texts become one short sentence with no command names or separators", () => {
  const cases: [string, string][] = [
    ["studio_apply_batch not allowed. Command not found", "This build of the app blocked the save."],
    ...NATIVE_NO_WRITE.map((text): [string, string] => [text, "The app was busy."]),
    ["local draft Save busy; retry the same request", "The app was busy."],
    [UNDELIVERED_SAVE_TEXT, "The app could not confirm this save."],
    ["Shared history changed before this edit was saved. Review it and save it as a new edit.", "The document changed before this edit was saved."],
    ["Studio could not read this document before saving, so nothing was sent. Open it and retry.", "The document had not loaded yet."],
    ["Studio has not read this channel's index yet, so nothing was sent. Retry once it has loaded.", "The Studio list had not loaded yet."],
    ["The server for these saved edits changed. They were not sent.", "The server changed, so these edits were not sent."],
    ["Studio request belongs to a locked or changed UI session", "The app was locked during the save."],
    ["server changed before Studio commit", "The server reloaded during the save."],
    ["Studio request cancelled; its local save may have completed", "The save was interrupted."],
    ["Studio: batched Apply item 2 of 2 was refused (edit belongs to a retired epoch)", "The document changed before this edit was saved."],
    ["Studio: batched Apply item 1 of 3 was refused (cell out of range)", "The app refused this edit."],
    ["epoch studio: edit belongs to a retired epoch", "The document changed before this edit was saved."],
    ["Studio: object already exists; use Apply to edit it", "A document with this id already exists."],
    ["Studio: object id already occurs in the index", "A document with this id already exists."],
    ["Studio: Studio requires current membership", "This device is no longer a member of the server."],
    ["epoch studio: local device is not a current member", "This device is no longer a member of the server."],
    ["Studio: unknown Studio channel", "This channel no longer exists."],
  ];
  for (const [error, words] of cases) {
    assert.equal(saveProblemText(error), words, error);
    assert.doesNotMatch(words, /[·—]|studio_|attempt/, `${error}: plain words only`);
  }
  // An exhausted refusal after an uncertain attempt carries a suffix; the reason is the refusal.
  assert.equal(saveProblemText(`${NATIVE_NO_WRITE[2]}; an earlier attempt may already have committed, so the exact request remains uncertain`), "The app was busy.");
});

test("an unknown text never claims a save failed when it may have landed", () => {
  assert.equal(saveProblemText("receipt lost"), "The save did not go through.");
  assert.equal(saveProblemText("receipt lost", true), "The app could not confirm this save.");
  assert.equal(saveProblemText("", true), "The app could not confirm this save.", "a restored, interrupted delivery has no text at all");
});

test("the headline says confirmed or saved, and whether it is still moving", () => {
  assert.equal(saveHeadline("score recipe", { retrying: true, uncertain: true, others: 5 }), "Not confirmed yet: score recipe, 5 more waiting");
  assert.equal(saveHeadline("fps", { retrying: false, uncertain: false, others: 0 }), "Not saved: fps");
  assert.equal(saveHeadline("fps", { retrying: true, uncertain: false, others: 1 }), "Not saved yet: fps, 1 more waiting");
});

test("the countdown rounds up and says now once due", () => {
  assert.equal(retryText(5_000, 1_000), "Trying again in 4 s");
  assert.equal(retryText(1_001, 1_000), "Trying again in 1 s");
  assert.equal(retryText(1_000, 1_000), "Trying again now");
  assert.equal(retryText(900, 1_000), "Trying again now");
});

test("the background-save toast runs its bar forward and ends saved, or on a warning that stays", () => {
  const t = (left: number, retrying = 0, stuck = 0) => ({ left, retrying, stuck, discarded: 0 });
  const start = saveToast(t(4), 4);
  assert.deepEqual(start, { text: "Saving Studio edits", kind: "info", progress: 0, done: false, dismiss: false });
  assert.equal(saveToast(t(1), 4).progress, 0.75);
  // An edit made meanwhile joins the queue: `total` is the high-water mark, so the bar holds.
  assert.equal(saveToast(t(5), 4).progress, 0);
  assert.deepEqual(saveToast(t(2, 1), 4), { text: "Studio edits not saved yet. Trying again.", kind: "info", progress: 0.5, done: false, dismiss: false });
  assert.deepEqual(saveToast(t(0), 4), { text: "Studio edits saved", kind: "ok", progress: 1, done: true, dismiss: false });
  const stuck = saveToast(t(1, 0, 1), 4);
  assert.equal(stuck.kind, "warn"); assert.equal(stuck.done, true);
  // A stuck edit may have landed unheard: the words claim neither saved nor failed.
  assert.equal(stuck.text, "A Studio edit needs you. Open the Studio to retry or discard it.");
  assert.equal(saveToast(t(3, 0, 2), 4).text, "2 Studio edits need you. Open the Studio to retry or discard them.");
  for (const tally of [t(3), t(3, 2), t(3, 0, 1), t(0)]) assert.doesNotMatch(saveToast(tally, 3).text, /[·—]|could not be saved/);
});

test("a queue the person emptied by discarding is not called saved", () => {
  assert.deepEqual(saveToast({ left: 0, retrying: 0, stuck: 0, discarded: 7 }, 3, 1), { text: "", kind: "info", progress: 1, done: true, dismiss: true });
  assert.equal(saveToast({ left: 0, retrying: 0, stuck: 0, discarded: 7 }, 3, 0).text, "Studio edits saved", "discards before the toast began do not count");
});
