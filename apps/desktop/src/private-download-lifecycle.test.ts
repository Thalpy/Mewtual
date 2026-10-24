import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { saveGroupFile, resumeGroupFile, completedDownload, downloadSavedNotice, isSaveAsCancelled } from "./native-download.ts";
import { acceptSaveProgress, checkedSavedCheck, privateDownloadChatSave, privateDownloadOffers, privateDownloadSaveAsIsNewCopy } from "./private-downloads.ts";

// Execute the real App handlers and real progress listener across held native replies. This
// catches races that pure status-label tests cannot see without compiling another fake app.
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["downloadFile", "resumePrivateDownload", "changePrivateDownload", "runPrivateDownloadAction", "saveInProgress",
  "checkSavedRows", "savedCheckFor"]);
const handlers = tree.statements.filter(n => ts.isFunctionDeclaration(n) && n.name && names.has(n.name.text));
assert.equal(handlers.length, names.size);
let progress: ts.ArrowFunction | undefined;
function visit(node: ts.Node) {
  if (ts.isCallExpression(node) && node.expression.getText(tree) === "listen" &&
      ts.isStringLiteral(node.arguments[0]) && node.arguments[0].text === "download-progress") {
    assert.ok(ts.isArrowFunction(node.arguments[1])); progress = node.arguments[1];
  }
  ts.forEachChild(node, visit);
}
visit(tree); assert.ok(progress);
const production = ts.transpileModule(handlers.map(n => n.getFullText(tree)).join("\n") +
  `\nfunction applyProgress(e) ${progress.body.getText(tree)}`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
const file = { cid: "c".repeat(64), name: "cat.bin", author: "cat", held: 0, total: 2, size: 100 };
const saved = { id: "d".repeat(64), cid: file.cid, intent: "3", grant: "1", status: "incomplete", verifiedChunks: 1, savedChunks: 0, totalChunks: 2, chosenPlace: false };
const result = { path: "/Downloads/cat.bin", displayed: true };
function deferred() { let resolve!: (value: unknown) => void, reject!: (error: unknown) => void; const promise = new Promise((yes,no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function fixture() {
  return new Function("saveGroupFile", "resumeGroupFile", "completedDownload", "downloadSavedNotice", "acceptSaveProgress",
    "privateDownloadChatSave", "privateDownloadOffers", "isSaveAsCancelled", "privateDownloadSaveAsIsNewCopy", "checkedSavedCheck", "file", `
    let activeServerId = 1, viewGeneration = 2, locked = false, error = "", privateDownloadBusy = {};
    let privateDownloads = [], privateDownloadSaves = {}, downloads = {}, activeSaves = {}, uuid = 0, refreshes = 0;
    let savedChecks = {}, savedChecking = {}, savedCheckVisit = 1, view = "downloads";
    const calls = [], notices = [], views = [], replies = new Map(), filesByCid = new Map([[file.cid, file]]);
    const crypto = { randomUUID: () => "op-" + (++uuid) }, fileInventoryGate = { invalidate() {} };
    const dlKey = (server,cid) => server + ":" + cid;
    const unlockedViewCurrent = (gen,server) => !locked && gen === viewGeneration && server === activeServerId;
    const invoke = async (command,args) => { calls.push([command,args]); return await (replies.get(command) ?? ${JSON.stringify(result)}); };
    const refreshFiles = async () => { refreshes++; privateDownloads = []; };
    const toast = (...args) => notices.push(args), switchView = (view) => views.push(view);
    const jukeTakeProgressLease = null, activeCallLease = null, inCall = false, callServer = null, jukeKind = "", jukeNow = null;
    const shouldApplyJamTakeProgress = () => false, sampleRate = () => 0, onlineCount = 2;
    ${production}
    return { calls, replies, notices, views, start: downloadFile, resume: resumePrivateDownload, pause: row => changePrivateDownload(row,false),
      act: runPrivateDownloadAction,
      checkSaved: checkSavedRows, savedFor: savedCheckFor,
      visit() { savedCheckVisit++; },
      rows(value) { privateDownloads = value; },
      lock() { locked = true; viewGeneration++; },
      unlock() { locked = false; viewGeneration++; },
      server(value) { activeServerId = value; viewGeneration++; },
      progress(operation) { applyProgress({ payload: { server: 1, cid: file.cid, cancellation: operation, done: 2, total: 2,
        bytes_done: 100, bytes_total: 100, network_bytes_done: 0, provider: null } }); },
      state: () => ({ error, refreshes, downloads, activeSaves, busy: privateDownloadBusy, saves: privateDownloadSaves }) };
  `)(saveGroupFile, resumeGroupFile, completedDownload, downloadSavedNotice, acceptSaveProgress,
    privateDownloadChatSave, privateDownloadOffers, isSaveAsCancelled, privateDownloadSaveAsIsNewCopy, checkedSavedCheck, file);
}
test("resumed save settles its exact row and late/preview progress cannot revive it", async () => {
  const app = fixture(), held = deferred(); app.replies.set("save_group_file", held.promise);
  const saving = app.resume(saved);
  const operation = app.calls[0][1].operation;
  app.progress(operation);
  assert.equal(Object.values(app.state().downloads)[0].status, "verifying");
  held.resolve(result); await saving;
  assert.equal(Object.values(app.state().downloads)[0].status, "done");
  app.progress(operation); app.progress(null);
  assert.equal(Object.values(app.state().downloads)[0].status, "done");
  app.replies.delete("save_group_file"); await app.start(file);
  assert.equal(app.calls.filter(([name]: [string]) => name === "save_group_file").length, 2);
});
test("initial and resumed save share CID ownership while Pause remains available", async () => {
  const app = fixture(), held = deferred(); app.replies.set("save_group_file", held.promise);
  const saving = app.start(file);
  app.rows([{ ...saved, status: "ready" }]);
  await app.resume({ ...saved, status: "ready" }); await app.start(file);
  assert.equal(app.calls.filter(([name]: [string]) => name === "save_group_file").length, 1);
  await app.pause({ ...saved, status: "receiving" });
  assert.equal(app.calls[1][0], "set_private_download_paused");
  held.resolve(result); await saving;
});
test("late completion after lock cannot refill save status or show a saved-path notice", async () => {
  const app = fixture(), held = deferred(); app.replies.set("save_group_file", held.promise);
  const saving = app.resume(saved); app.lock(); app.unlock(); held.resolve(result); await saving;
  assert.equal(app.notices.length, 0);
  assert.equal(app.state().refreshes, 0);
  assert.equal(Object.values(app.state().downloads).length, 0);
  assert.equal(Object.keys(app.state().activeSaves).length, 0);
  app.replies.delete("save_group_file"); await app.start(file);
  assert.equal(Object.values(app.state().downloads)[0].status, "done");
});
test("server ABA retires only the old busy owner on success and failure without showing its result", async () => {
  for (const fail of [false,true]) {
    const app = fixture(), held = deferred(); app.replies.set("save_group_file", held.promise);
    const saving = app.start(file); app.server(2); app.server(1);
    if (fail) held.reject(new Error("old save failed")); else held.resolve(result);
    await saving;
    assert.equal(app.notices.length, 0);
    assert.equal(app.state().error, "");
    assert.equal(Object.keys(app.state().activeSaves).length, 0);
    app.replies.delete("save_group_file"); await app.start(file);
    assert.equal(app.calls.filter(([name]: [string]) => name === "save_group_file").length, 2);
    assert.equal(Object.values(app.state().downloads)[0].status, "done");
  }
});
test("Save new copy sends freshCopy with the exact revision and Keep partly saved file sends keepPartial", async () => {
  const app = fixture();
  await app.act({ ...saved, status: "destination-changed" }, "fresh");
  const [name, args] = app.calls[0];
  assert.equal(name, "save_group_file");
  assert.equal(args.freshCopy, true);
  assert.equal(args.resume, saved.id);
  assert.equal(args.expectedIntent, "3");
  await app.act({ ...saved, status: "destination-changed" }, "resume"); // not offered: no call
  assert.equal(app.calls.length, 1);
  await app.act({ ...saved, status: "removing" }, "keep");
  assert.deepEqual(app.calls[1], ["cancel_private_download", { server: 1, download: saved.id, keepPartial: true }]);
  await app.act({ ...saved, status: "saved" }, "remove");
  assert.deepEqual(app.calls[2], ["cancel_private_download", { server: 1, download: saved.id }]);
});
test("Save as sends only a flag, implies a new copy on halted or saved rows, and never a path", async () => {
  const app = fixture();
  await app.act({ ...saved, status: "saved" }, "saveAs");
  assert.deepEqual(
    { ...app.calls[0][1], operation: undefined },
    { server: 1, cid: file.cid, name: file.name, resume: saved.id, expectedIntent: "3", freshCopy: true, saveAs: true, operation: undefined },
  );
  await app.act({ ...saved, status: "incomplete" }, "saveAs");
  assert.equal(app.calls[1][1].saveAs, true);
  assert.equal(app.calls[1][1].freshCopy, undefined, "no journal to forget on an unfinished row");
  await app.act({ ...saved, status: "save-interrupted" }, "saveAs"); // not offered there
  assert.equal(app.calls.length, 2);
  for (const [, args] of app.calls) {
    for (const key of Object.keys(args)) assert.doesNotMatch(key, /path|folder|dir|destination/i);
  }
});
test("closing the Save as dialog is quiet: no error, no notice, no leftover row", async () => {
  for (const viaRow of [false, true]) {
    const app = fixture();
    app.replies.set("save_group_file", Promise.reject("Save as was cancelled."));
    if (viaRow) await app.act({ ...saved, status: "ready" }, "saveAs");
    else await app.start(file, true);
    assert.equal(app.calls[0][1].saveAs, true);
    assert.equal(app.state().error, "", `viaRow=${viaRow}`);
    assert.equal(app.notices.length, 0);
    assert.equal(Object.keys(app.state().downloads).length, 0, "the pending row is dropped");
    assert.equal(Object.keys(app.state().activeSaves).length, 0);
  }
  // Any other failure is still an error.
  const app = fixture();
  app.replies.set("save_group_file", Promise.reject("That folder cannot be used. Choose another folder."));
  await app.start(file, true);
  assert.match(app.state().error, /folder cannot be used/);
});
test("Save as from the file list goes through a known row, or to Transfers where it is not offered", async () => {
  const app = fixture(); app.rows([{ ...saved, status: "saving" }]);
  await app.start(file, true);
  assert.equal(app.calls.length, 0);
  assert.deepEqual(app.views, ["downloads"]);
  const ready = fixture(); ready.rows([{ ...saved, status: "ready" }]);
  await ready.start(file, true);
  assert.equal(ready.calls[0][1].resume, saved.id);
  assert.equal(ready.calls[0][1].saveAs, true);
});
test("Remove shows native's notice when a file of the save stays on disk, and nothing otherwise", async () => {
  const left = "A file at the partly saved file's name could not be confirmed as Mewtual's, so it was left in place.";
  const app = fixture();
  app.replies.set("cancel_private_download", left);
  await app.act({ ...saved, status: "destination-changed" }, "remove");
  assert.deepEqual(app.notices, [[left, "info", 9000]]);
  app.replies.set("cancel_private_download", null);
  await app.act({ ...saved, status: "saved" }, "remove");
  assert.equal(app.notices.length, 1, "a clean removal shows nothing extra");
  // A notice that arrives after Lock is not shown to the next session.
  const held = deferred(); app.replies.set("cancel_private_download", held.promise);
  const removing = app.act({ ...saved, status: "saved" }, "remove");
  app.lock(); app.unlock(); held.resolve(left); await removing;
  assert.equal(app.notices.length, 1);
});
test("a Remove stalled on its folder holds only its own row; every other row keeps its actions", async () => {
  // Second review R2-M1: native removal can wait indefinitely on external cleanup. The renderer
  // used to hold one global busy token across that call, so Pause on another row dispatched
  // nothing. Ownership is now per download ID.
  const app = fixture(), held = deferred();
  const a = { ...saved, status: "saved" };
  const b = { ...saved, id: "e".repeat(64), cid: "f".repeat(64) };
  app.replies.set("cancel_private_download", held.promise);
  const removing = app.act(a, "remove");
  app.replies.set("cancel_private_download", null);
  await app.act({ ...b, status: "receiving" }, "pause");
  await app.act({ ...b, status: "destination-changed" }, "remove");
  await app.act({ ...b, status: "removing" }, "keep");
  await app.act({ ...b, status: "incomplete" }, "resume");
  const forB = app.calls.slice(1).map(([name, args]: [string, Record<string, unknown>]) => [name, args.download ?? args.resume]);
  assert.deepEqual(forB, [
    ["set_private_download_paused", b.id],
    ["cancel_private_download", b.id],
    ["cancel_private_download", b.id],
    ["save_group_file", b.id],
  ], "B's Pause, Remove, Keep and Resume all dispatch while A's removal is pending");
  // A second action on A itself is still refused while its removal is pending.
  const before = app.calls.length;
  await app.act({ ...a, status: "receiving" }, "pause");
  await app.act(a, "remove");
  await app.act({ ...a, status: "removing" }, "keep");
  await app.act({ ...a, status: "incomplete" }, "resume");
  assert.equal(app.calls.length, before, "no duplicate operation on the pending row");
  assert.ok(app.state().busy[a.id], "A's removal still owns its row");
  held.resolve(null); await removing;
  assert.equal(app.state().busy[a.id], undefined, "released when native answers");
  await app.act(a, "remove");
  assert.equal(app.calls.length, before + 1, "A is free again");
});
test("Transfers checks each Saved row's file once per visit and keeps only a current answer", async () => {
  // User decision, 23 October: a Saved row's file is checked (natively, bounded, read-only) when
  // Transfers is shown. Only an answer for the exact view and row revision is kept.
  const app = fixture();
  const a = { ...saved, status: "saved" };
  const b = { ...saved, id: "e".repeat(64), cid: "f".repeat(64), status: "incomplete" };
  app.rows([a, b]);
  app.replies.set("check_saved_download", "missing");
  await app.checkSaved();
  assert.deepEqual(app.calls, [["check_saved_download", { server: 1, download: a.id, expectedIntent: "3" }]],
    "only the Saved row, with its exact revision, never a path");
  assert.equal(app.savedFor(a), "missing");
  assert.equal(app.savedFor(b), null);
  await app.checkSaved();
  assert.equal(app.calls.length, 1, "once per visit");
  app.visit(); app.replies.set("check_saved_download", "present");
  await app.checkSaved();
  assert.equal(app.calls.length, 2, "each visit checks again");
  assert.equal(app.savedFor(a), "present");
  // Acting on the Saved row checks it again at the next refresh, but its last answer stays on
  // screen until then (decisions review L3).
  await app.act(a, "remove");
  assert.equal(app.savedFor(a), "present");
  const asked = app.calls.filter(([name]: [string]) => name === "check_saved_download").length;
  app.rows([a, b]); // the fixture's refresh empties the list; the native list still has the row
  await app.checkSaved();
  assert.equal(app.calls.filter(([name]: [string]) => name === "check_saved_download").length, asked + 1);
  // A failed or unreadable answer claims nothing new: "unknown" for that same revision.
  for (const reply of [Promise.reject("This download changed. Open Transfers and try again."), "C:\\Users\\me\\cat.bin"]) {
    const app = fixture(); app.rows([a]);
    app.replies.set("check_saved_download", reply);
    await app.checkSaved();
    assert.equal(app.savedFor(a), "unknown");
  }
});
test("Saved rows are checked one at a time and an overlapping refresh asks for nothing twice", async () => {
  // Decisions review M1: native runs two check workers; the renderer asks one at a time.
  const app = fixture(), held = deferred();
  const rows = ["1", "2", "3"].map((n) => ({ ...saved, id: n.repeat(64), cid: n.repeat(64), status: "saved" }));
  app.rows(rows); app.replies.set("check_saved_download", held.promise);
  const checking = app.checkSaved();
  await Promise.resolve(); await Promise.resolve();
  const asked = () => app.calls.filter(([name]: [string]) => name === "check_saved_download").length;
  assert.equal(asked(), 1, "one check in flight");
  await app.checkSaved();
  assert.equal(asked(), 1, "an overlapping refresh does not ask again");
  held.resolve("present"); await checking;
  assert.equal(asked(), 3, "the rest follow, one by one");
  for (const row of rows) assert.equal(app.savedFor(row), "present");
});
test("Transfers runs the check from its refresh and starts a new visit when shown", () => {
  // Decisions review L5: the two call sites that make the check happen at all.
  const text = (name: string) => tree.statements.find((n) => ts.isFunctionDeclaration(n) && n.name?.text === name)?.getText(tree) ?? "";
  assert.match(text("refreshFiles"), /if \(view === "downloads"\) void checkSavedRows\(\);/);
  assert.match(text("switchView"), /if \(v === "downloads"\) savedCheckVisit\+\+;/);
});
test("a saved file check answered after the row or view changed never relabels anything", async () => {
  // The row's revision changes (Save another copy) while its check is in flight: the older
  // revision's answer is dropped, and the newer revision gets its own check at once (re-review
  // LOW 2), never the old answer.
  {
    const app = fixture(), held = deferred(), a = { ...saved, status: "saved" };
    app.rows([a]); app.replies.set("check_saved_download", held.promise);
    const checking = app.checkSaved();
    app.rows([{ ...a, intent: "4" }]);
    held.resolve("replaced");
    app.replies.set("check_saved_download", "present");
    await checking;
    assert.deepEqual(app.calls.map(([, args]: [string, { expectedIntent: string }]) => args.expectedIntent), ["3", "4"]);
    assert.equal(app.savedFor({ ...a, intent: "4" }), "present", "its own answer, not the older revision's");
    assert.equal(app.savedFor(a), null);
  }
  // A row that keeps changing is re-checked only once per batch.
  {
    const app = fixture(), a = { ...saved, status: "saved" };
    app.rows([a]);
    let intent = 3;
    app.replies.set("check_saved_download", {
      then(resolve: (value: string) => void) { app.rows([{ ...a, intent: String(++intent) }]); resolve("present"); },
    });
    await app.checkSaved();
    assert.equal(app.calls.length, 2, "one re-run, then it stops");
  }
  // Lock (or a server switch) lands while it is in flight.
  for (const change of ["lock", "server"] as const) {
    const app = fixture(), held = deferred(), a = { ...saved, status: "saved" };
    app.rows([a]); app.replies.set("check_saved_download", held.promise);
    const checking = app.checkSaved();
    if (change === "lock") { app.lock(); app.unlock(); } else { app.server(2); app.server(1); }
    held.resolve("missing"); await checking;
    assert.equal(app.savedFor(a), null, change);
  }
  // A row that is no longer Saved shows no check, even with a matching revision.
  const app = fixture(), a = { ...saved, status: "saved" };
  app.rows([a]); app.replies.set("check_saved_download", "missing");
  await app.checkSaved();
  assert.equal(app.savedFor({ ...a, status: "removing" }), null);
  // Locked: nothing is asked at all.
  const locked = fixture(); locked.rows([a]); locked.lock();
  await locked.checkSaved();
  assert.equal(locked.calls.length, 0);
});
test("saving a halted, saved or running file from the list opens Transfers instead of acting", async () => {
  for (const status of ["destination-changed", "destination-unavailable", "saved", "removing", "exhausted", "saving", "receiving"]) {
    const app = fixture(); app.rows([{ ...saved, status }]);
    await app.start(file);
    assert.equal(app.calls.length, 0, status);
    assert.deepEqual(app.views, ["downloads"], status);
  }
  const app = fixture(); app.rows([{ ...saved, status: "save-interrupted" }]);
  await app.start(file);
  assert.equal(app.calls[0][0], "save_group_file");
  assert.equal(app.calls[0][1].resume, saved.id);
  assert.equal(app.calls[0][1].freshCopy, undefined, "continuing never forgets the partial file");
});
