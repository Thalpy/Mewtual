import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import {
  SAVED_MARK_LABEL, SAVED_MARK_TITLE, checkedSavedFileChecks, replaceOlderFrom, savedFileCheckKey,
} from "./saved-files.ts";

// PR #56: the real App.svelte handlers for the saved-files list (its batch check, Clear, Save
// again, Show in folder, "Start a new list", the chat mark on built cards and the replace
// preference), executed against stand-ins for the state they read. Function text is taken from
// the production script, so a change to a handler is what these tests run.
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["checkSavedListRows", "savedFileCheckFor", "runSavedListAction", "revealSavedListRow",
  "resetSavedList", "markSavedCards", "readReplaceOlder", "setReplaceOlder"]);
const handlers = tree.statements.filter((n) => ts.isFunctionDeclaration(n) && n.name && names.has(n.name.text));
assert.equal(handlers.length, names.size);
const production = ts.transpileModule(handlers.map((n) => n.getFullText(tree)).join("\n"),
  { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;

const entry = (n: number, extra: Record<string, unknown> = {}) => ({
  id: String(n).repeat(32), cid: String(n).repeat(64), name: `file-${n}.pdf`, savedAt: 1000 * n, size: 10,
  savedTo: "downloads", checkable: true, ...extra,
});

function app(files: ReturnType<typeof entry>[], listed: string[] = []) {
  return new Function("filesValue", "listedValue", "savedFileCheckKey", "checkedSavedFileChecks", "replaceOlderFrom",
    "SAVED_MARK_LABEL", "SAVED_MARK_TITLE", `
    let activeServerId = 1, viewGeneration = 1, locked = false, savedCheckVisit = 3;
    let savedFiles = { state: "ok", files: filesValue }, savedFileChecks = {}, savedFilesChecking = null;
    let savedFileBusy = {}, replaceOlder = false, savedCids = new Set(filesValue.map((f) => f.cid));
    const REPLACE_OLDER_KEY = "catcoms.saves.replaceOlder";
    const store = new Map();
    const localStorage = { getItem: (k) => store.has(k) ? store.get(k) : null, setItem: (k, v) => store.set(k, String(v)),
      removeItem: (k) => store.delete(k) };
    const filesByCid = new Map(listedValue.map((cid) => [cid, { cid, name: "listed name" }]));
    const calls = [], notices = [], saves = [];
    let refreshes = 0, replies = new Map();
    const invoke = async (command, args) => {
      calls.push([command, args]);
      const reply = replies.get(command);
      if (reply instanceof Error) throw reply;
      return typeof reply === "function" ? reply(args) : reply ?? null;
    };
    const unlockedViewCurrent = (gen, server) => !locked && gen === viewGeneration && server === activeServerId;
    const refreshFiles = async () => { refreshes++; };
    const toast = (...args) => notices.push(args);
    const downloadFile = async (file) => { saves.push(file); };
    ${production}
    return { calls, notices, saves, replies,
      check: checkSavedListRows, checkFor: savedFileCheckFor, act: runSavedListAction, reveal: revealSavedListRow,
      reset: resetSavedList, mark: markSavedCards, readPref: readReplaceOlder, setPref: setReplaceOlder,
      visit() { savedCheckVisit++; }, lock() { locked = true; viewGeneration++; },
      server(value) { activeServerId = value; viewGeneration++; },
      list(value) { savedFiles = value; }, saved(value) { savedCids = value; },
      state: () => ({ refreshes, savedFileChecks, savedFileBusy, replaceOlder, store }) };
  `)(files, listed, savedFileCheckKey, checkedSavedFileChecks, replaceOlderFrom, SAVED_MARK_LABEL, SAVED_MARK_TITLE);
}

test("the list's rows are checked in one batch per visit, and only answered entries get an answer", async () => {
  const a = entry(1), b = entry(2), plain = entry(3, { checkable: false });
  const t = app([a, b, plain]);
  // Native reached `a` only (the bounded batch ran out of time before `b`).
  t.replies.set("check_saved_files", [{ id: a.id, name: a.name, check: "missing" }]);
  await t.check();
  assert.deepEqual(t.calls, [["check_saved_files", { server: 1 }]]);
  assert.equal(t.checkFor(a), "missing");
  assert.equal(t.checkFor(b), null, "not reached: not checked yet, never 'could not check'");
  // The same visit asks again only for what has no answer of this visit; nothing new here
  // asks once more (b), and a second visit asks again.
  await t.check();
  assert.equal(t.calls.length, 2);
  t.replies.set("check_saved_files", [{ id: a.id, name: a.name, check: "present" }, { id: b.id, name: b.name, check: "present" }]);
  await t.check();
  assert.equal(t.calls.length, 3);
  await t.check();
  assert.equal(t.calls.length, 3, "everything checked this visit");
  t.visit();
  await t.check();
  assert.equal(t.calls.length, 4);
  // An answer for an entry under another name (a replace moved it) does not apply to the old name.
  const moved = { ...a, name: "file-1 (renamed).pdf" };
  assert.equal(t.checkFor(moved), null);
});

test("a check answered after the view changed claims nothing, and an unreadable answer neither", async () => {
  const a = entry(1);
  const t = app([a]);
  t.replies.set("check_saved_files", () => { t.server(2); return [{ id: a.id, name: a.name, check: "missing" }]; });
  await t.check();
  t.server(1);
  assert.equal(t.checkFor(a), null);
  const u = app([a]);
  u.replies.set("check_saved_files", [{ id: a.id, name: "../x", check: "missing" }]);
  await u.check();
  assert.equal(u.checkFor(a), null);
  // Nothing is asked for a list that cannot be read, or while locked.
  const v = app([a]);
  v.list({ state: "damaged", files: [] });
  await v.check();
  v.lock();
  await v.check();
  assert.equal(v.calls.length, 0);
});

test("Clear forgets the entry by its ID only and refreshes; Save again is the ordinary save", async () => {
  const a = entry(1);
  const t = app([a], [a.cid]);
  const clear = { kind: "forget", label: "Clear", disabled: false };
  await t.act(a, clear);
  assert.deepEqual(t.calls, [["forget_saved_file", { server: 1, entry: a.id }]]);
  assert.equal(t.state().refreshes, 1);
  assert.deepEqual(t.state().savedFileBusy, {});
  // A refusal is shown, and the row still refreshes.
  t.replies.set("forget_saved_file", new Error("Mewtual could not read its list of saved files on this device, so it changed nothing."));
  await t.act(a, clear);
  assert.match(String(t.notices[0][0]), /changed nothing/);
  // Save again saves the listed file through the chat path (no new command, no path).
  await t.act(a, { kind: "again", label: "Save again", disabled: false });
  assert.deepEqual(t.saves, [{ cid: a.cid, name: "listed name" }]);
  // A disabled action does nothing.
  const before = t.calls.length;
  await t.act(a, { ...clear, disabled: true });
  assert.equal(t.calls.length, before);
});

test("Show in folder names the entry only; a refusal says why and checks the row again", async () => {
  const a = entry(1);
  const t = app([a]);
  await t.reveal(a);
  assert.deepEqual(t.calls, [["reveal_saved_file", { server: 1, entry: a.id }]]);
  assert.equal(t.notices.length, 0);
  t.replies.set("reveal_saved_file", new Error("The saved file is no longer there."));
  t.replies.set("check_saved_files", [{ id: a.id, name: a.name, check: "missing" }]);
  await t.reveal(a);
  assert.equal(t.notices[0][0], "Error: The saved file is no longer there.");
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(t.checkFor(a), "missing");
  assert.ok(t.calls.some(([command]: [string]) => command === "check_saved_files"));
});

test("Start a new list asks native, which refuses anything but a damaged record", async () => {
  const t = app([]);
  await t.reset();
  assert.deepEqual(t.calls, [["reset_saved_files", { server: 1 }]]);
  assert.equal(t.state().refreshes, 1);
  t.replies.set("reset_saved_files", new Error("The list of saved files is not damaged, so Mewtual left it as it is."));
  await t.reset();
  assert.match(String(t.notices[0][0]), /not damaged/);
});

test("the chat mark reaches cards built before the list loaded, as text only", () => {
  const a = entry(1), b = entry(2);
  const t = app([a]);
  const container = document.createElement("div");
  const card = (cid: string) => {
    const element = document.createElement("a");
    element.className = "ref-card file-card";
    element.setAttribute("data-file-cid", cid);
    container.appendChild(element);
    return element;
  };
  const savedCard = card(a.cid), otherCard = card(b.cid);
  const status = document.createElement("a");
  status.className = "ref-card status-card";
  container.appendChild(status);
  t.mark(container);
  const mark = savedCard.querySelector(".ref-card-saved");
  assert.ok(mark);
  assert.equal(mark?.textContent, SAVED_MARK_LABEL);
  assert.equal(mark?.getAttribute("title"), SAVED_MARK_TITLE);
  assert.equal(mark?.children.length, 0, "text, never markup");
  assert.equal(otherCard.querySelector(".ref-card-saved"), null);
  assert.equal(status.querySelector(".ref-card-saved"), null);
  // Marking again adds no second mark; a list change moves it.
  t.mark(container);
  assert.equal(savedCard.querySelectorAll(".ref-card-saved").length, 1);
  t.saved(new Set([b.cid]));
  t.mark(container);
  assert.equal(savedCard.querySelector(".ref-card-saved"), null);
  assert.ok(otherCard.querySelector(".ref-card-saved"));
  t.mark(undefined);
});

test("the replace preference is off by default and stored only as its exact word", () => {
  const t = app([]);
  assert.equal(t.readPref(), false);
  t.setPref(true);
  assert.equal(t.state().replaceOlder, true);
  assert.equal(t.state().store.get("catcoms.saves.replaceOlder"), "on");
  assert.equal(t.readPref(), true);
  t.setPref(false);
  assert.equal(t.state().store.has("catcoms.saves.replaceOlder"), false);
  assert.equal(t.readPref(), false);
});

test("the saved-files markup: list rows in Saved, the marks, the setting and the leave note", () => {
  // List rows sit in the Saved group after the private rows, in the same row design.
  assert.match(source, /\{#if group\.id === "saved"\}\s*<!-- PR #56[^]*?\{#each savedListViews as listed \(listed\.id\)\}/);
  assert.match(source, /data-sdl-row=\{`saved-\$\{listed\.id\}`\} data-state="saved" data-tone=\{listed\.tone\}/);
  assert.match(source, /onclick=\{\(\) => revealSavedListRow\(file\)\}>\{@render transferIcon\("show-in-folder"\)\} Show in folder<\/button>/);
  // An unreadable list says so; only a damaged one offers a new start.
  assert.match(source, /\{#if savedFiles\.state === "damaged"\}\s*<button class="sdl-act ghost" type="button" onclick=\{resetSavedList\}>Start a new list<\/button>/);
  // The mark in the Files list and the file details, and on chat cards after every resolve pass.
  assert.equal(source.match(/\{#if savedCids\.has\((?:f|fileInfo)\.cid\)\}/g)?.length, 2);
  assert.match(source, /resolveRefCards\(statusEl\);\s*markSavedCards\(messagesEl\);\s*markSavedCards\(statusEl\);/);
  assert.match(source, /void savedCids; \/\/ PR #56/);
  // The setting, off by default, in Chat & Media.
  assert.match(source, /<h3>Saved files<\/h3>\s*<label class="toggle">\s*<input\s*type="checkbox"\s*checked=\{replaceOlder\}/);
  // Leaving says what happens to saved files.
  assert.match(source, /Files you saved from it stay where they are\. Mewtual forgets where they went\./);
  // Another server's list never marks this server's files.
  assert.match(source, /savedFiles = \{ state: "ok", files: \[\] \};\s*savedFileChecks = \{\};/);
});
