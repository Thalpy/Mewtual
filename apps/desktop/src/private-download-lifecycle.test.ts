import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { saveGroupFile, resumeGroupFile, completedDownload, downloadSavedNotice } from "./native-download.ts";
import { acceptSaveProgress, privateDownloadChatSave, privateDownloadOffers } from "./private-downloads.ts";

// Execute the real App handlers and real progress listener across held native replies. This
// catches races that pure status-label tests cannot see without compiling another fake app.
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["downloadFile", "resumePrivateDownload", "changePrivateDownload", "runPrivateDownloadAction", "saveInProgress"]);
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
const saved = { id: "d".repeat(64), cid: file.cid, intent: "3", grant: "1", status: "incomplete", verifiedChunks: 1, savedChunks: 0, totalChunks: 2 };
const result = { path: "/Downloads/cat.bin", displayed: true };
function deferred() { let resolve!: (value: unknown) => void, reject!: (error: unknown) => void; const promise = new Promise((yes,no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
function fixture() {
  return new Function("saveGroupFile", "resumeGroupFile", "completedDownload", "downloadSavedNotice", "acceptSaveProgress",
    "privateDownloadChatSave", "privateDownloadOffers", "file", `
    let activeServerId = 1, viewGeneration = 2, locked = false, error = "", privateDownloadBusy = null;
    let privateDownloads = [], privateDownloadSaves = {}, downloads = {}, activeSaves = {}, uuid = 0, refreshes = 0;
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
      rows(value) { privateDownloads = value; },
      lock() { locked = true; viewGeneration++; },
      unlock() { locked = false; viewGeneration++; },
      server(value) { activeServerId = value; viewGeneration++; },
      progress(operation) { applyProgress({ payload: { server: 1, cid: file.cid, cancellation: operation, done: 2, total: 2,
        bytes_done: 100, bytes_total: 100, network_bytes_done: 0, provider: null } }); },
      state: () => ({ error, refreshes, downloads, activeSaves, busy: privateDownloadBusy, saves: privateDownloadSaves }) };
  `)(saveGroupFile, resumeGroupFile, completedDownload, downloadSavedNotice, acceptSaveProgress,
    privateDownloadChatSave, privateDownloadOffers, file);
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
