import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { fileInventoryRequests, mutateFileInventory } from "./file-availability.ts";
import type { KeptTransfer } from "./file-availability.ts";

// Run the actual App handlers across held native replies. Pure label tests cannot catch a
// delayed Retry starting work in the next server, or duplicate clicks spending two UI requests.
const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const script = source.slice(source.indexOf(">", source.indexOf("<script")) + 1, source.indexOf("</script>"));
const tree = ts.createSourceFile("App.ts", script, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
const names = new Set(["cancelKeptTransfer", "keepFileHere", "forgetKeptFile", "retryKeptFile", "setKeptPaused", "resumeKeptFile", "checkKeptFile"]);
const handlers = tree.statements.filter(node => ts.isFunctionDeclaration(node) && node.name && names.has(node.name.text));
assert.equal(handlers.length, names.size);
const production = ts.transpileModule(handlers.map(node => node.getFullText(tree)).join("\n"), {
  compilerOptions: { target: ts.ScriptTarget.ES2022 },
}).outputText;

function deferred() {
  let resolve!: (value?: unknown) => void, reject!: (reason: unknown) => void;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function fixture(current = copy) {
  return new Function("fileInventoryRequests", "mutateFileInventory", "copy", `
    let activeServerId = 1, viewGeneration = 3, locked = false, error = "";
    let keptTransfer = null, keptMutation = null;
    let freshIntent = copy.intent, keptFiles = { transfers: [] };
    const fileInventoryGate = fileInventoryRequests(), calls = [], replies = new Map();
    let refreshes = 0;
    const crypto = { randomUUID: () => "12345678-1234-1234-1234-123456789012" };
    const unlockedViewCurrent = (gen, server) => !locked && gen === viewGeneration && server === activeServerId;
    const refreshFiles = async () => {
      refreshes++; keptFiles = { transfers: [{ ...copy, intent: freshIntent, exhausted: false }] };
    };
    const invoke = async (command, args) => {
      calls.push([command, args]);
      return await (replies.get(command) ?? Promise.resolve());
    };
    ${production}
    return {
      calls, replies, keep: keepFileHere, retry: retryKeptFile, forget: forgetKeptFile, pause: setKeptPaused, resume: resumeKeptFile, check: checkKeptFile, stopCheck: cancelKeptTransfer,
      freshIntent(value) { freshIntent = value; },
      switchServer() { activeServerId = 2; viewGeneration++; },
      lock() { locked = true; viewGeneration++; cancelKeptTransfer(); },
      state: () => ({ error, refreshes, keptTransfer, busy: !!keptMutation }),
    };
  `)(fileInventoryRequests, mutateFileInventory, current);
}
const copy: KeptTransfer = {
  cid: "a".repeat(64), manifest_version: "b".repeat(64), transfer: "c".repeat(64),
  phase: "paused", verified_chunks: 1, total_chunks: 2, running: false, exhausted: true,
  grant: "9007199254740993", intent: "9007199254740994", user_paused: true, blocked: false,
};

test("an in-flight Retry blocks duplicate mutations and cannot resume in the next server", async () => {
  const app = fixture(), reply = deferred();
  app.replies.set("retry_kept_file", reply.promise);
  const retry = app.retry(copy);
  await app.retry(copy);
  await app.forget(copy.cid);
  assert.equal(app.calls.length, 1);
  assert.equal(app.calls[0][1].expectedGrant, copy.grant);
  assert.equal(app.state().busy, true);
  app.switchServer(); reply.resolve(); await retry;
  assert.equal(app.calls.length, 1, "stale Retry must not start Keep in either server");
  assert.equal(app.state().refreshes, 0);
  assert.equal(app.state().busy, false);
});

test("a successful Retry preserves intent until explicit Resume and admits the exact original file", async () => {
  const app = fixture(); await app.retry(copy);
  assert.deepEqual(app.calls.map(([command]: [string]) => command), ["retry_kept_file", "set_kept_paused", "request_kept_file"]);
  const retry = app.calls[0][1], resume = app.calls[1][1], keep = app.calls[2][1];
  assert.equal(retry.transfer, copy.transfer);
  assert.match(retry.request, /^[0-9a-f]{32}$/);
  assert.deepEqual(resume, { server: 1, cid: copy.cid, transfer: copy.transfer, expectedIntent: copy.intent, paused: false });
  assert.deepEqual(keep, { server: 1, cid: copy.cid });
  assert.equal(app.state().keptTransfer, null);
  assert.equal(app.state().busy, false);
});

test("a rejected Retry refreshes current evidence without starting a transfer", async () => {
  const app = fixture(), reply = deferred();
  app.replies.set("retry_kept_file", reply.promise);
  const pending = app.retry(copy); reply.reject(new Error("stale grant")); await pending;
  assert.equal(app.calls.length, 1);
  assert.equal(app.state().refreshes, 1);
  assert.match(app.state().error, /stale grant/);
  assert.equal(app.state().busy, false);
});

test("Pause waits for short admission and an inventory identity, then submits the exact intent CAS", async () => {
  const app = fixture(), admission = deferred();
  app.replies.set("request_kept_file", admission.promise);
  const pending = app.keep(copy.cid);
  await app.pause(copy, true);
  assert.deepEqual(app.calls.map(([command]: [string]) => command), ["request_kept_file"]);
  assert.equal(app.state().busy, true, "controls stay disabled before identity-ready acknowledgement");
  admission.resolve(); await pending;
  assert.equal(app.state().refreshes, 1);
  await app.pause(copy, true);
  assert.deepEqual(app.calls[1], ["set_kept_paused", {
    server: 1, cid: copy.cid, transfer: copy.transfer, expectedIntent: copy.intent, paused: true,
  }]);
  assert.equal(app.calls.length, 2, "Pause never starts a replacement job");
});

test("held Pause prevents duplicate Resume and cannot publish or start work after lock", async () => {
  const app = fixture(), paused = deferred();
  app.replies.set("set_kept_paused", paused.promise);
  const pending = app.pause(copy, true);
  await app.resume(copy);
  assert.equal(app.calls.length, 1);
  app.lock(); paused.reject(new Error("late intent refusal")); await pending;
  assert.equal(app.state().error, "");
  assert.equal(app.state().refreshes, 0);
  assert.equal(app.state().busy, false);
});

test("late release errors and Retry replies do not publish into a locked view", async () => {
  for (const command of ["forget_kept_file", "retry_kept_file"]) {
    const app = fixture(), reply = deferred(); app.replies.set(command, reply.promise);
    const pending = command === "forget_kept_file" ? app.forget(copy.cid) : app.retry(copy);
    app.lock(); reply.reject(new Error("late native failure")); await pending;
    assert.equal(app.calls.length, 1);
    assert.equal(app.state().error, "");
    assert.equal(app.state().refreshes, 0);
    assert.equal(app.state().busy, false);
  }
});


test("Retry of an exhausted V2 transfer resumes with the refreshed migrated intent", async () => {
  const legacy = { ...copy, intent: "0" };
  const app = fixture(legacy);
  app.freshIntent("1");
  await app.retry(legacy);
  assert.deepEqual(app.calls.map(([command]: [string]) => command), ["retry_kept_file", "set_kept_paused", "request_kept_file"]);
  assert.equal(app.calls[1][1].expectedIntent, "1");
  assert.equal(app.calls[2][1].cid, legacy.cid);
});


test("completed-copy check can stop before native lease registration without releasing ownership", async () => {
  const app = fixture(), registration = deferred();
  app.replies.set("begin_inline_download", registration.promise);
  const checking = app.check(copy.cid);
  const lease = app.calls[0][1].cancellation;
  app.stopCheck(); registration.resolve(); await checking;
  assert.equal(app.calls.some(([command]: [string]) => command === "keep_file"), false);
  assert.equal(app.calls.some(([command]: [string]) => command === "forget_kept_file"), false);
  assert.ok(app.calls.filter(([command]: [string]) => command === "cancel_inline_download")
    .every(([, args]: [string, { cancellation: string }]) => args.cancellation === lease));
});

test("explicit Resume fences an older Pause even when the transfer already waits unpaused", async () => {
  const waiting = { ...copy, user_paused: false, exhausted: false };
  const app = fixture(waiting); await app.resume(waiting);
  assert.deepEqual(app.calls.map(([command]: [string]) => command), ["set_kept_paused", "request_kept_file"]);
  assert.equal(app.calls[0][1].expectedIntent, waiting.intent);
  assert.equal(app.calls[0][1].paused, false);
});


test("exhausted completed copy retries its grant then starts a cancelable check of the same saved owner", async () => {
  const completed = { ...copy, phase: "completed" as const, user_paused: false };
  const app = fixture(completed);
  await app.retry(completed);
  assert.deepEqual(app.calls.map(([command]: [string]) => command), ["retry_kept_file", "begin_inline_download", "keep_file"]);
  assert.equal(app.calls[0][1].transfer, completed.transfer);
  assert.equal(app.calls[2][1].cid, completed.cid);
  assert.equal(app.calls.some(([command]: [string]) => command === "forget_kept_file"), false);
});
