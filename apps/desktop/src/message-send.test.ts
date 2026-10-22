import assert from "node:assert/strict";
import test from "node:test";
import ts from "typescript";
import { addPendingSend, matchingPendingSend, pendingSendRetryBlock, resolvePendingSend } from "./pending-sends.ts";
import { readFileSync } from "node:fs";
import { persistenceWarning, sendAndRefresh, sendOutcome, type SendMessageResult } from "./message-send.ts";

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
  refresh = async () => {}, context = async () => "a".repeat(64), onSeal: () => void = () => {}) {
  const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  const body = ts.transpile(source.slice(source.indexOf("  async function submitPendingSend("),
    source.indexOf("  // Inline edit of one of your own messages.")), { target: ts.ScriptTarget.ES2022 });
  return new Function("submit", "refresh", "context", "onSeal", "sendAndRefresh", "persistenceWarning", "sendOutcome", "addPendingSend", "matchingPendingSend", "pendingSendRetryBlock", "resolvePendingSend", `
    let draft = "one message", cur = { active: "1" }, activeServerId = 1, sending = false;
    let locked = false, uiStateLoadGeneration = 0, replyingTo = "", mentionQuery = null;
    let drafts = { room: draft }, draftRevisions = {}, pendingSendNonce = 0, chatStickToBottom = false;
    let pendingSends = {}, pendingSendErrors = {}, retryingPendingSends = false, uiStateReady = true, uiStateSaveTimer;
    let recoveredSendDrafts = {}, pendingSendResolution = null, pendingManagerOpen = false;
    let windowCloseInFlight = false;
    let tailLoaded = false, messageWindowScope = "", messages = [], pageTotal = 0;
    let replyingToRow, error = "", warnings = [], sealed = null;
    const chanKey = () => "room", scheduleUiStateSave = () => {}, chatScopeKey = () => "room";
    const toast = text => warnings.push(text), errorText = String;
    const invoke = () => context();
    const continuityJson = () => JSON.stringify({drafts,pendingSends,recoveredSendDrafts});
    const queueUiStateSave = async (json, afterSave = () => {}) => { sealed = JSON.parse(typeof json === "function" ? json() : json); afterSave(); onSeal(); };
    const saveUiStateImmediately = async () => { await queueUiStateSave(continuityJson()); return true; };
    const invokeDebugged = (_, args) => submit(args).then(value => ({ value }));
    ${body}
    return {
      send, retryPendingSends, resolvePendingMessage,
      type(text) { draft = text; drafts.room = text; draftRevisions.room = (draftRevisions.room ?? 0) + 1; },
      lock() { sealed = JSON.parse(continuityJson()); locked = true; uiStateLoadGeneration++; draft = ""; drafts = {}; pendingSends = {}; sending = false; pendingSendNonce++; },
      state() { return { draft, drafts, pendingSends, recoveredSendDrafts, sealed, warnings, error, sending }; }
    };
  `)(submit, refresh, context, onSeal, sendAndRefresh, persistenceWarning, sendOutcome, addPendingSend, matchingPendingSend, pendingSendRetryBlock, resolvePendingSend);
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
  await app.resolvePendingMessage(token, "recover");
  assert.deepEqual(app.state().pendingSends, {});
  assert.equal(app.state().recoveredSendDrafts[token].text, "one message");
  assert.equal(tokens.length, 2, "recovering the saved text never publishes it");
  assert.equal(app.state().draft, "one message");
  await app.send();
  assert.notEqual(tokens[2], tokens[0]);
});

test("the actual send reports what it came to, which the composer's text cannot", async () => {
  // Found in review: "post now" guessed the outcome from the composer. Here a send is accepted and
  // saved while the person types and deletes a character: the draft is kept (its revision moved)
  // with the very words that were sent, yet the outcome is "sent".
  let finish!: (result: SendMessageResult) => void;
  const kept = composer(() => new Promise(resolve => { finish = resolve; }));
  const pending = kept.send();
  for (let n = 0; n < 20 && !finish; n++) await Promise.resolve();
  kept.type("one message!");
  kept.type("one message");
  finish({ accepted: true, persistence: { status: "durable" } });
  assert.equal(await pending, "sent");
  assert.equal(kept.state().draft, "one message", "the draft stayed, and still reads like an unsent one");

  const outcome = async (submit: Parameters<typeof composer>[0]) => composer(submit).send();
  assert.equal(await outcome(async () => ({ accepted: true, persistence: { status: "pending", reason: "write_failed" } })), "uncertain");
  assert.equal(await outcome(async () => ({ accepted: false, persistence: { status: "pending", reason: "write_failed" } })), "uncertain");
  assert.equal(await outcome(async () => ({ accepted: true, persistence: { status: "superseded" } })), "uncertain");
  // Refused because the conversation moved on: never authored, held for review, not retried.
  assert.equal(await outcome(async () => ({ accepted: false, persistence: { status: "superseded" } })), "failed");

  const busy = composer(async () => ({ accepted: true, persistence: { status: "durable" } }));
  busy.type("");
  assert.equal(await busy.send(), "refused", "nothing to send");
  let finishContext!: (value: string) => void;
  const locked = composer(async () => ({ accepted: true, persistence: { status: "durable" } }), async () => {},
    () => new Promise(resolve => { finishContext = resolve; }));
  const locking = locked.send();
  locked.lock();
  finishContext("a".repeat(64));
  assert.equal(await locking, "refused", "a lock before dispatch submitted nothing");
});

test("a send that threw after sealing is uncertain, since its identity retries by itself", async () => {
  // Found in re-review: a lost IPC response was reported "not sent", but its sealed identity stays
  // pending and the quiet retry sends it; an edited resend under a new token then made two.
  const lost = composer(async () => { throw new Error("IPC response lost"); });
  assert.equal(await lost.send(), "uncertain");
  assert.equal(Object.keys(lost.state().pendingSends).length, 1, "still pending, to be retried with its own token");

  // Nothing sealed: the authoring context could not be read, so nothing was or will be sent.
  const noContext = composer(async () => ({ accepted: true, persistence: { status: "durable" } }), async () => {},
    async () => { throw new Error("server unavailable"); });
  assert.equal(await noContext.send(), "failed");
  assert.deepEqual(noContext.state().pendingSends, {});

  // Refused natively for good: blocked from automatic retry, so it waits for the person.
  const invalid = composer(async () => { throw new Error("CHAT_SEND_INVALID: length"); });
  assert.equal(await invalid.send(), "failed");
});

test("a request recovered or stopped while it is still out stays uncertain when its reply fails", async () => {
  // Found in review: the outcome read the pending entry back after the failure, so resolving the
  // entry in Pending messages while the request was out turned a possibly accepted message into
  // "not sent", inviting the recovered text to be sent again under a new token.
  for (const action of ["recover", "cancel"] as const) {
    let reply!: (failure: Error) => void;
    let submissions = 0;
    const app = composer(() => { submissions++; return new Promise((_, reject) => { reply = reject; }); });
    const sending = app.send();
    for (let n = 0; n < 40 && !reply; n++) await Promise.resolve();
    assert.ok(reply, `${action}: the request is out`);
    const [token] = Object.keys(app.state().pendingSends);
    await app.resolvePendingMessage(token, action);
    assert.deepEqual(app.state().pendingSends, {}, `${action}: the entry is resolved while the request is out`);
    reply(new Error("IPC response lost"));
    assert.equal(await sending, "uncertain", `${action}: it may have been accepted`);
    assert.equal(submissions, 1);
    if (action === "recover") {
      assert.equal(app.state().recoveredSendDrafts[token]?.text, "one message", "the recovery stands");
      assert.equal(app.state().recoveredSendDrafts[token]?.acceptance, "ambiguous");
    }
  }

  // A definite native refusal is still "failed", resolved entry or not.
  let refuse!: (failure: Error) => void;
  const refused = composer(() => new Promise((_, reject) => { refuse = reject; }));
  const pending = refused.send();
  for (let n = 0; n < 40 && !refuse; n++) await Promise.resolve();
  await refused.resolvePendingMessage(Object.keys(refused.state().pendingSends)[0], "cancel");
  refuse(new Error("CHAT_SEND_INVALID: length"));
  assert.equal(await pending, "failed");
});

test("a send whose sealed identity is taken over before dispatch is uncertain, not refused", async () => {
  // A lock while the identity is being sealed: it was saved to the vault, and comes back to be
  // retried after unlock, so this press cannot call it unsent.
  let lockDuringSeal = () => {};
  const app = composer(async () => ({ accepted: true, persistence: { status: "durable" } }), async () => {},
    async () => "a".repeat(64), () => lockDuringSeal());
  lockDuringSeal = () => { lockDuringSeal = () => {}; app.lock(); };
  assert.equal(await app.send(), "uncertain");
  assert.equal(Object.keys(app.state().sealed.pendingSends).length, 1, "sealed in the vault before the lock");
});

test("a refused retry cannot disprove an earlier ambiguous send, even after pending resolution", async () => {
  for (const response of ["context", "invalid", "superseded"] as const) {
    for (const action of [undefined, "recover", "cancel"] as const) {
      const tokens: unknown[] = [];
      let resolve!: (result: SendMessageResult) => void;
      let reject!: (error: Error) => void;
      const app = composer(async args => {
        tokens.push(args.retryToken);
        if (tokens.length === 1) throw new Error("IPC response lost after possible acceptance");
        return new Promise<SendMessageResult>((yes, no) => { resolve = yes; reject = no; });
      });
      assert.equal(await app.send(), "uncertain");
      const retry = app.send();
      for (let n = 0; n < 40 && !reject; n++) await Promise.resolve();
      assert.ok(reject, "the same identity's retry is dispatched");
      assert.equal(tokens.length, 2);
      assert.equal(tokens[0], tokens[1]);
      if (action) await app.resolvePendingMessage(Object.keys(app.state().pendingSends)[0], action);
      if (response === "superseded") resolve({ accepted: false, persistence: { status: "superseded" } });
      else reject(new Error(response === "context" ? "CHAT_SEND_CONTEXT_CHANGED: conversation replaced" : "CHAT_SEND_INVALID: length"));
      assert.equal(await retry, "uncertain", `${response}/${action ?? "kept"}: this refusal concerns the retry, not its lost predecessor`);
      assert.equal(tokens.length, 2, "classification never submits another identity");
      if (response === "superseded") {
        assert.match(app.state().warnings.at(-1), /earlier attempt may have sent/i);
        assert.doesNotMatch(app.state().warnings.at(-1), /move it to a draft to start a new send/i);
      }
    }
  }
});

test("background retry warns about earlier possible acceptance when the conversation changes", async () => {
  const tokens: unknown[] = [];
  const app = composer(async args => {
    tokens.push(args.retryToken);
    if (tokens.length === 1) throw new Error("IPC response lost");
    return { accepted: false, persistence: { status: "superseded" } };
  });
  assert.equal(await app.send(), "uncertain");
  await app.retryPendingSends();
  assert.equal(tokens.length, 2);
  assert.equal(tokens[0], tokens[1]);
  assert.match(app.state().error, /earlier attempt may have sent/i);
  assert.doesNotMatch(app.state().error, /move it to a draft to start a new send/i);
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
