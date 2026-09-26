import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { persistenceWarning, sendAndRefresh, type SendMessageResult } from "./message-send.ts";

test("accepted pending send refreshes without becoming a rejection or a second submission", async () => {
  let submissions = 0;
  let refreshes = 0;
  const result: SendMessageResult = { accepted: true, persistence: { status: "pending", reason: "write_failed" } };
  const completed = await sendAndRefresh(async () => { submissions++; return result; }, async () => { refreshes++; });
  assert.deepEqual(completed, { result });
  assert.equal(submissions, 1);
  assert.equal(refreshes, 1);
  assert.match(persistenceWarning(completed.result)!, /saving.*has not completed/);
});

test("an accepted message survives a failed acknowledgement refresh without restoring a draft", async () => {
  const result: SendMessageResult = { accepted: true, persistence: { status: "durable" } };
  const unavailable = new Error("refresh unavailable");
  let draftRestored = false;
  const completed = await sendAndRefresh(async () => result, async () => { throw unavailable; })
    .catch(() => { draftRestored = true; return undefined; });
  assert.equal(draftRestored, false);
  assert.deepEqual(completed, { result, refreshError: unavailable });
  assert.equal(persistenceWarning(result), null);
});

test("rejection before acceptance remains retryable and does not refresh", async () => {
  let refreshes = 0;
  await assert.rejects(sendAndRefresh(async () => { throw new Error("not accepted"); }, async () => { refreshes++; }), /not accepted/);
  assert.equal(refreshes, 0);
});

test("a replaced conversation cannot claim persistence or suggest ordinary retry", () => {
  const result: SendMessageResult = { accepted: true, persistence: { status: "superseded" } };
  assert.match(persistenceWarning(result)!, /before its save could be confirmed/);
  assert.match(persistenceWarning(result)!, /could create a duplicate/);
});

// Exercise the actual composer function with its surrounding state replaced by a small harness.
// This pins await ordering and lock fences, which a test of sendAndRefresh alone cannot observe.
function composer(submit: () => Promise<SendMessageResult>, refresh = async () => {}) {
  const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  const body = source.slice(source.indexOf("  async function send() {"), source.indexOf("  // Inline edit of one of your own messages."))
    .replace("invokeDebugged<SendMessageResult>", "invokeDebugged");
  return new Function("submit", "refresh", "sendAndRefresh", "persistenceWarning", `
    let draft = "one message", cur = { active: "1" }, activeServerId = 1, sending = false;
    let locked = false, uiStateLoadGeneration = 0, replyingTo = "", mentionQuery = null;
    let drafts = { room: draft }, pendingSendNonce = 0, chatStickToBottom = false;
    let tailLoaded = false, messageWindowScope = "", messages = [], pageTotal = 0;
    let replyingToRow, error = "", warnings = [];
    const chanKey = () => "room", scheduleUiStateSave = () => {}, chatScopeKey = () => "room";
    const toast = text => warnings.push(text), errorText = String;
    const invokeDebugged = () => submit().then(value => ({ value }));
    ${body}
    return {
      send,
      lock() { locked = true; uiStateLoadGeneration++; draft = ""; drafts = {}; sending = false; pendingSendNonce++; },
      state() { return { draft, drafts, warnings, error, sending }; }
    };
  `)(submit, refresh, sendAndRefresh, persistenceWarning);
}

test("composer submits before yielding so an immediate lock cannot cancel an unsubmitted draft", async () => {
  let submissions = 0;
  let complete!: (value: SendMessageResult) => void;
  const pending = new Promise<SendMessageResult>(resolve => { complete = resolve; });
  const app = composer(() => { submissions++; return pending; });
  const sending = app.send();
  assert.equal(submissions, 1, "IPC starts before the caller can lock");
  app.lock();
  complete({ accepted: true, persistence: { status: "pending", reason: "write_failed" } });
  await sending;
  assert.deepEqual(app.state(), { draft: "", drafts: {}, warnings: [], error: "", sending: false });
});

test("composer restores a rejected send while the original session remains active", async () => {
  const app = composer(async () => { throw new Error("not accepted"); });
  await app.send();
  assert.equal(app.state().draft, "one message");
  assert.deepEqual(app.state().drafts, { room: "one message" });
});

test("composer never restores an accepted message when its refresh fails", async () => {
  const app = composer(async () => ({ accepted: true, persistence: { status: "durable" } }),
    async () => { throw new Error("refresh failed"); });
  await app.send();
  assert.equal(app.state().draft, "");
  assert.deepEqual(app.state().drafts, {});
  assert.match(app.state().error, /Message accepted.*refresh failed/);
});
