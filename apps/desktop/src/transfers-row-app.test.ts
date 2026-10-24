import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";

// Transfers UX (PR #51): the real App.svelte handlers for the merged row, the inline remove
// confirmation and Show in folder, executed against stand-ins for the state they read. Function
// text is taken from the production script, so a change to a handler is what these tests run.
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["liveSaveFor", "confirmingRemove", "askRemove", "cancelRemove", "runSavedRowAction",
  "rowButtonClick", "revealSavedRow"]);
const handlers = tree.statements.filter((n) => ts.isFunctionDeclaration(n) && n.name && names.has(n.name.text));
assert.equal(handlers.length, names.size);
const token = tree.statements.find((n) => ts.isVariableStatement(n) &&
  n.declarationList.declarations.some((d) => ts.isIdentifier(d.name) && d.name.text === "confirmToken"));
assert.ok(token);
const production = ts.transpileModule([token, ...handlers].map((n) => n.getFullText(tree)).join("\n"),
  { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

const row = { id: "d".repeat(64), cid: "c".repeat(64), intent: "3", status: "paused" };
const next = { id: "e".repeat(64), cid: "f".repeat(64), intent: "1", status: "incomplete" };
const remove = { kind: "remove", label: "Remove", confirm: true, disabled: false, disabledReason: null };

function app() {
  return new Function("rowValue", "nextValue", `
    let activeServerId = 1, viewGeneration = 1, locked = false;
    let downloads = {}, activeSaves = {}, confirmRemove = {}, revealingSaved = {}, savedChecks = {};
    let present = new Set([rowValue.id, nextValue.id]);
    const ran = [], focused = [], calls = [], notices = [], checks = [];
    let replies = new Map();
    const savedRowGroups = { needs: [{ id: rowValue.id }, { id: nextValue.id }], progress: [], saved: [] };
    const dlKey = (server, cid) => server + ":" + cid;
    const tick = async () => {};
    const element = (key) => ({ focus() { focused.push(key); } });
    const document = {
      body: { tag: "body" }, activeElement: null,
      querySelector(selector) {
        const row = selector.match(/data-sdl-row="([0-9a-f]+)"/);
        if (row && !present.has(row[1])) return null;
        return element(selector);
      },
      getElementById(id) { return element("#" + id); },
    };
    const persist = new Set();
    const runPrivateDownloadAction = async (target, kind) => { ran.push([target.id, kind]); if (!persist.has(target.id)) present.delete(target.id); };
    const invoke = async (command, args) => { calls.push([command, args]); const reply = replies.get(command); if (reply instanceof Error) throw reply; return reply ?? null; };
    const unlockedViewCurrent = (gen, server) => !locked && gen === viewGeneration && server === activeServerId;
    const toast = (...args) => notices.push(args);
    const checkSavedRows = async () => { checks.push("check"); };
    ${production}
    document.activeElement = document.body;
    return {
      ran, focused, calls, notices, checks, replies,
      set downloads(value) { downloads = value; }, set activeSaves(value) { activeSaves = value; },
      set server(value) { activeServerId = value; },
      set savedChecks(value) { savedChecks = value; }, get savedChecks() { return savedChecks; },
      get confirmRemove() { return confirmRemove; },
      keep(id) { persist.add(id); },
      liveSaveFor, confirmingRemove, askRemove, cancelRemove, rowButtonClick, runSavedRowAction, revealSavedRow,
    };
  `)(row, next);
}

test("a save in flight feeds its own row only while it is the active invocation", () => {
  const a = app();
  const key = `1:${row.cid}`;
  // `done` and `bytesDone` include chunks held before the save; the row counts only what this
  // save wrote (`savedDone`, `savedBytes`), and nothing before its first write.
  a.downloads = { [key]: { operation: "save-1", done: 7, total: 9, bytesDone: 700, bytesTotal: 900 } };
  assert.equal(a.liveSaveFor(row), null, "no active save of this file");
  a.activeSaves = { [key]: "save-1" };
  assert.deepEqual(a.liveSaveFor(row), { done: 0, total: 9, bytesDone: 0, bytesTotal: 900 });
  a.downloads = { [key]: { operation: "save-1", done: 7, total: 9, bytesDone: 700, bytesTotal: 900, savedDone: 5, savedBytes: 500 } };
  assert.deepEqual(a.liveSaveFor(row), { done: 5, total: 9, bytesDone: 500, bytesTotal: 900 });
  a.activeSaves = { [key]: "save-2" };
  assert.equal(a.liveSaveFor(row), null, "a stale record of an older save");
  a.activeSaves = { [key]: "save-1" };
  a.server = 2;
  assert.equal(a.liveSaveFor(row), null, "another server's transfer");
});

test("Remove asks inline first, confirms on the second press, and Cancel puts it back", async () => {
  const a = app();
  a.rowButtonClick(row, remove, "needs");
  await Promise.resolve();
  assert.deepEqual(a.ran, [], "the first press only asks");
  assert.equal(a.confirmingRemove(row), true);
  assert.ok(a.focused.some((f: string) => f.includes(`data-sdl-confirm="${row.id}"`)), "focus moves to Confirm remove");
  await a.cancelRemove(row);
  assert.equal(a.confirmingRemove(row), false);
  assert.ok(a.focused.at(-1).includes(`data-sdl-remove="${row.id}"`), "Cancel returns focus to Remove");
  await a.askRemove(row);
  a.rowButtonClick(row, remove, "needs");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(a.ran, [[row.id, "remove"]]);
  assert.equal(a.confirmingRemove(row), false, "the confirmation is spent");
});

test("a confirmation never carries over to a row that changed meanwhile", async () => {
  const a = app();
  await a.askRemove(row);
  const changed = { ...row, intent: "4" };
  assert.equal(a.confirmingRemove(changed), false);
  a.rowButtonClick(changed, remove, "needs");
  await Promise.resolve();
  assert.deepEqual(a.ran, [], "it asks again instead of removing");
  const otherState = { ...row, status: "incomplete" };
  assert.equal(a.confirmingRemove(otherState), false);
});

test("actions that need no confirmation run at once, and disabled ones do nothing", async () => {
  const a = app();
  a.rowButtonClick(row, { kind: "resume", label: "Resume download", confirm: false, disabled: false, disabledReason: null }, "progress");
  a.rowButtonClick(row, { ...remove, disabled: true, disabledReason: "Another action on this file is running" }, "needs");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(a.ran, [[row.id, "resume"]]);
});

test("focus moves to the next row, or the group heading, when a row goes away", async () => {
  const a = app();
  await a.runSavedRowAction(row, remove, "needs");
  assert.ok(a.focused.at(-1).includes(`data-sdl-row="${next.id}"`), "the next row's first action");
  const b = app();
  await b.runSavedRowAction(next, remove, "needs");
  assert.equal(b.focused.at(-1), "#sdl-needs", "the last row hands focus to its group heading");
  // A row that is still there keeps focus where it is.
  const c = app();
  c.keep(row.id);
  await c.runSavedRowAction(row, { ...remove, kind: "pause" }, "needs");
  assert.deepEqual(c.focused, []);
});

test("Show in folder sends only the Download ID and revision, and a refusal re-checks the row", async () => {
  const a = app();
  await a.revealSavedRow({ ...row, status: "saved" });
  assert.deepEqual(a.calls, [["reveal_saved_download", { server: 1, download: row.id, expectedIntent: "3" }]]);
  assert.deepEqual(a.notices, []);
  a.replies.set("reveal_saved_download", new Error("The saved file is no longer there."));
  a.savedChecks = { [row.id]: { intent: "3", result: "present", visit: 4 } };
  await a.revealSavedRow({ ...row, status: "saved" });
  assert.deepEqual(a.notices.at(-1), ["Error: The saved file is no longer there.", "warn", 6000]);
  assert.equal(a.savedChecks[row.id].visit, -1, "the row is checked again");
  assert.deepEqual(a.checks, ["check"]);
});

test("the Transfers markup keeps one row per file and the new contracts", () => {
  // The live list leaves out a download whose private row carries its save.
  assert.match(source, /let liveTransferList = \$derived\(\s*transferList\.filter\(\(t\) => t\.direction !== "download" \|\| !privateDownloadCids\.has\(t\.cid\)\)/);
  assert.match(source, /\{#each liveTransferList as d \(d\.key\)\}/);
  assert.doesNotMatch(source, /\{#each transferList as d/);
  // Rows come from the pure view, grouped and labelled by their headings.
  assert.match(source, /<section class="sdl" aria-labelledby=\{`sdl-\$\{group\.id\}`\}>/);
  assert.match(source, /<ul class="sdl-list" aria-labelledby=\{`sdl-\$\{group\.id\}`\}>/);
  assert.match(source, /data-state=\{saved\.state\} data-tone=\{saved\.tone\}/);
  // One polite live region for the tab.
  assert.equal(source.match(/class="sdl-sr" aria-live="polite"/g)?.length, 1);
  // The old dense paragraph is gone; its one-line replacements live in transfer-rows.ts.
  assert.doesNotMatch(source, /Resume continues a partly saved file only if it is still the file Mewtual created/);
  // The replace setting was deferred to a follow-up PR: no trace of it here.
  assert.doesNotMatch(source, /Replace my older saved copy|replaceOlder/);
  // Action buttons name their file and say why they are disabled.
  assert.match(source, /aria-label=\{action\.accessibleName\}/);
  assert.match(source, /title=\{action\.disabledReason \?\? undefined\}/);
  assert.match(source, /aria-label=\{`Show \$\{saved\.name\} in folder`\}/);
});
