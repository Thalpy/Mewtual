import assert from "node:assert/strict";
import test from "node:test";
import ts from "typescript";
import { addPendingSend, matchingPendingSend, pendingSendRetryBlock } from "./pending-sends.ts";
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

// Execute the actual composer and pending-send functions, with only their I/O replaced.
function composer(submit: (args: Record<string, unknown>) => Promise<SendMessageResult>,
  refresh = async () => {}, context = async () => "a".repeat(64)) {
  const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  const body = ts.transpile(source.slice(source.indexOf("  async function submitPendingSend("),
    source.indexOf("  // Inline edit of one of your own messages.")), { target: ts.ScriptTarget.ES2022 });
  return new Function("submit", "refresh", "context", "sendAndRefresh", "persistenceWarning", "addPendingSend", "matchingPendingSend", "pendingSendRetryBlock", `
    let draft = "one message", cur = { active: "1" }, activeServerId = 1, sending = false;
    let locked = false, uiStateLoadGeneration = 0, replyingTo = "", mentionQuery = null;
    let drafts = { room: draft }, draftRevisions = {}, pendingSendNonce = 0, chatStickToBottom = false;
    let pendingSends = {}, pendingSendErrors = {}, retryingPendingSends = false, uiStateReady = true, uiStateSaveTimer;
    let tailLoaded = false, messageWindowScope = "", messages = [], pageTotal = 0;
    let replyingToRow, error = "", warnings = [], sealed = null;
    const chanKey = () => "room", scheduleUiStateSave = () => {}, chatScopeKey = () => "room";
    const toast = text => warnings.push(text), errorText = String;
    const invoke = () => context();
    const continuityJson = () => JSON.stringify({drafts,pendingSends});
    const queueUiStateSave = async json => { sealed = JSON.parse(json); };
    const saveUiStateImmediately = async () => { await queueUiStateSave(continuityJson()); return true; };
    const invokeDebugged = (_, args) => submit(args).then(value => ({ value }));
    ${body}
    return {
      send, retryPendingSends, movePendingToDraft,
      type(text) { draft = text; drafts.room = text; draftRevisions.room = (draftRevisions.room ?? 0) + 1; },
      lock() { sealed = JSON.parse(continuityJson()); locked = true; uiStateLoadGeneration++; draft = ""; drafts = {}; pendingSends = {}; sending = false; pendingSendNonce++; },
      state() { return { draft, drafts, pendingSends, sealed, warnings, error, sending }; }
    };
  `)(submit, refresh, context, sendAndRefresh, persistenceWarning, addPendingSend, matchingPendingSend, pendingSendRetryBlock);
}

test("composer keeps the draft until its retry identity can be sealed", async () => {
  let finishContext!: (value: string) => void;
  let submissions = 0;
  const app = composer(async () => { submissions++; return { accepted: true, persistence: { status: "durable" } }; },
    async () => {}, () => new Promise(resolve => { finishContext = resolve; }));
  const sending = app.send();
  assert.equal(app.state().draft, "one message");
  app.lock();
  finishContext("a".repeat(64));
  await sending;
  assert.equal(submissions, 0);
  assert.deepEqual(app.state().sealed.drafts, { room: "one message" });
  assert.deepEqual(app.state().pendingSends, {});
});

test("composer retries an ambiguous submission with the same sealed token", async () => {
  const tokens: unknown[] = [];
  const app = composer(async args => { tokens.push(args.retryToken); throw new Error("response lost"); });
  await app.send();
  assert.equal(app.state().draft, "one message");
  assert.equal(Object.keys(app.state().sealed.pendingSends).length, 1);
  await app.send();
  assert.equal(tokens.length, 2);
  assert.equal(tokens[0], tokens[1]);
});

test("an unsaved preparation stays retryable without restoring another identity", async () => {
  const tokens: unknown[] = [];
  const app = composer(async args => {
    tokens.push(args.retryToken);
    return tokens.length === 1 ? { accepted: false, persistence: { status: "pending", reason: "write_failed" } }
      : { accepted: true, persistence: { status: "durable" } };
  });
  await app.send();
  assert.equal(Object.keys(app.state().pendingSends).length, 1);
  await app.retryPendingSends();
  assert.equal(tokens[0], tokens[1]);
  assert.deepEqual(app.state().pendingSends, {});
});

test("composer never restores an accepted message when its refresh fails", async () => {
  const app = composer(async () => ({ accepted: true, persistence: { status: "durable" } }),
    async () => { throw new Error("refresh failed"); });
  await app.send();
  assert.equal(app.state().draft, "");
  assert.deepEqual(app.state().drafts, {});
  assert.deepEqual(app.state().pendingSends, {});
  assert.match(app.state().error, /conversation could not refresh/);
});

test("superseded pending send warns that automatic retry is paused and the actual retry path skips it", async () => {
  let attempts = 0;
  const app = composer(async () => {
    attempts++;
    return { accepted: false, persistence: { status: "superseded" } };
  });
  await app.send();
  const [intent] = Object.values(app.state().pendingSends) as Array<{ retryBlock?: string }>;
  assert.equal(intent.retryBlock, "context_changed");
  assert.equal(app.state().warnings.length, 1);
  assert.match(app.state().warnings[0], /Automatic retry is paused/);
  assert.match(app.state().warnings[0], /move it to a draft/);
  assert.doesNotMatch(app.state().warnings[0], /will retry automatically/);
  await app.retryPendingSends(true);
  assert.equal(attempts, 1, "warning agrees with the production automatic retry predicate");
});

test("a stale authoring context requires an explicit new-send decision", async () => {
  const tokens: unknown[] = [];
  const app = composer(async args => {
    tokens.push(args.retryToken);
    if (tokens.length <= 2) throw new Error("CHAT_SEND_CONTEXT_CHANGED: review old work");
    return { accepted: true, persistence: { status: "durable" } };
  });
  await app.send();
  await app.retryPendingSends();
  assert.equal(tokens[0], tokens[1]);
  const token = Object.keys(app.state().pendingSends)[0];
  await app.movePendingToDraft(token);
  assert.deepEqual(app.state().pendingSends, {});
  assert.equal(app.state().draft, "one message");
  await app.send();
  assert.notEqual(tokens[2], tokens[0]);
});

test("a new identical draft survives the preceding send's acknowledgement", async () => {
  let finish!: (result: SendMessageResult) => void;
  const app = composer(() => new Promise(resolve => { finish = resolve; }));
  const pending = app.send();
  for (let n = 0; n < 20 && !finish; n++) await Promise.resolve();
  assert.ok(finish);
  app.type("one message");
  finish({ accepted: true, persistence: { status: "durable" } });
  await pending;
  assert.equal(app.state().draft, "one message");
  assert.deepEqual(app.state().drafts, { room: "one message" });
});
