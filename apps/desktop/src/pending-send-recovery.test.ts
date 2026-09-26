import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { addPendingSend, matchingPendingSend, pendingSendRetryBlock } from "./pending-sends.ts";
import { persistenceWarning, sendAndRefresh } from "./message-send.ts";
import { planLegacyReadMarkMigration, sanitizeUiContinuity } from "./ui-continuity.ts";

const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
const transpile = (start: string, end: string) => {
  const from = source.indexOf(start);
  const to = source.indexOf(end, from);
  assert.ok(from >= 0 && to > from, "execute the actual production functions");
  return ts.transpile(source.slice(from, to), { target: ts.ScriptTarget.ES2022 });
};

test("retry refuses IPC until the caller identity survives an actual successful continuity save", async () => {
  const body = transpile("  async function submitPendingSend(", "  // Inline edit of one of your own messages.");
  const app = new Function("addPendingSend", "matchingPendingSend", "sendAndRefresh", "persistenceWarning", "pendingSendRetryBlock", `
    let draft = "retain this identity", cur = { active: "1" }, activeServerId = 1, sending = false;
    let locked = false, uiStateLoadGeneration = 0, replyingTo = "", mentionQuery = null;
    let drafts = { room: draft }, draftRevisions = {}, pendingSendNonce = 0, chatStickToBottom = false;
    let pendingSends = {}, pendingSendErrors = {}, retryingPendingSends = false, uiStateReady = true, uiStateSaveTimer;
    let tailLoaded = false, messageWindowScope = "", messages = [], pageTotal = 0;
    let replyingToRow, error = "", sealed = null, savesFail = true, submissions = [];
    const chanKey = () => "room", chatScopeKey = () => "room", scheduleUiStateSave = () => {};
    const toast = () => {}, errorText = String, refresh = async () => {};
    const invoke = async () => "a".repeat(64);
    const continuityJson = () => JSON.stringify({drafts, pendingSends});
    const queueUiStateSave = async json => {
      if (savesFail) throw new Error("injected continuity failure");
      sealed = JSON.parse(json);
    };
    const saveUiStateImmediately = async () => {
      try { await queueUiStateSave(continuityJson()); return true; } catch { return false; }
    };
    const invokeDebugged = async (_, args) => {
      submissions.push({ args, savedAtSubmission: structuredClone(sealed) });
      return { value: { accepted: true, persistence: { status: "durable" } } };
    };
    ${body}
    return {
      send, retryPendingSends,
      allowSave() { savesFail = false; },
      state() { return { draft, pendingSends, submissions, sealed }; }
    };
  `)(addPendingSend, matchingPendingSend, sendAndRefresh, persistenceWarning, pendingSendRetryBlock);

  await app.send();
  const [token] = Object.keys(app.state().pendingSends);
  assert.ok(token, "failed first save retains an in-memory intent");
  assert.equal(app.state().sealed, null);
  assert.equal(app.state().submissions.length, 0);
  await app.retryPendingSends();
  assert.equal(app.state().submissions.length, 0, "Retry must not bypass the failed first barrier");
  assert.equal(app.state().draft, "retain this identity");
  app.allowSave();
  await app.retryPendingSends();
  const [{ args, savedAtSubmission }] = app.state().submissions;
  assert.equal(args.retryToken, token);
  assert.equal(savedAtSubmission.pendingSends[token].token, token);
  assert.equal(savedAtSubmission.pendingSends[token].text, args.text);
  assert.equal(savedAtSubmission.pendingSends[token].expectedContext, args.expectedContext);
  assert.deepEqual(app.state().pendingSends, {});
});

test("failed continuity hydration cannot replace the sealed pending identities with empty state", async () => {
  const body = transpile("  function queueUiStateSave(", "  function chanKey(");
  const intent = { token: "1".repeat(32), server: 1, channel: "1", expectedContext: "2".repeat(64),
    text: "a committed message whose acknowledgement was lost", replyTo: "" };
  const initial = sanitizeUiContinuity({ pendingSends: { [intent.token]: intent }, drafts: { room: intent.text } });
  const app = new Function("initial", "sanitizeUiContinuity", "planLegacyReadMarkMigration", "console", `
    let locked = false, uiStateLoadGeneration = 7, uiStateReady = false, uiStateSaveTimer;
    let uiStateSaveChain = Promise.resolve(), uiStateSaveFailed = false, uiStateFailureToast = 0;
    let pendingSends = structuredClone(initial.pendingSends), drafts = structuredClone(initial.drafts);
    let readMarks = {}, statusCursors = {}, fileTrustPolicies = {}, latePast = {}, embedAutoLoad = false, error = "";
    let sealed = JSON.stringify(initial), failLoad = true, writes = 0, flushed = 0;
    const localStorage = { getItem() { return null; }, removeItem() {} };
    const flushPendingStatusMarks = () => { flushed++; };
    const updateToast = () => {}, toast = () => 1;
    const invoke = async (command, args) => {
      if (command === "get_ui_state") {
        if (failLoad) throw new Error("injected authenticated read failure");
        return sealed;
      }
      if (command === "save_ui_state") { writes++; sealed = args.json; return; }
      throw new Error("unexpected command");
    };
    ${body}
    return {
      load: () => loadUiContinuity(7), save: saveUiStateImmediately,
      allowLoad() { failLoad = false; },
      state() { return { uiStateReady, pendingSends, drafts, writes, flushed, sealed, error }; }
    };
  `)(initial, sanitizeUiContinuity, planLegacyReadMarkMigration, { warn() {} });

  await app.load();
  assert.equal(app.state().uiStateReady, false);
  assert.deepEqual(app.state().pendingSends, initial.pendingSends);
  assert.deepEqual(app.state().drafts, initial.drafts);
  assert.equal(app.state().flushed, 0, "deferred writes must wait for successful hydration");
  assert.equal(await app.save(), false);
  assert.equal(app.state().writes, 0);
  assert.deepEqual(JSON.parse(app.state().sealed).pendingSends, initial.pendingSends);

  app.allowLoad();
  await app.load();
  assert.equal(app.state().uiStateReady, true);
  assert.equal(app.state().flushed, 1);
  assert.equal(await app.save(), true);
  assert.deepEqual(JSON.parse(app.state().sealed).pendingSends, initial.pendingSends);
});
