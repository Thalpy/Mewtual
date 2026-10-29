import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { addPendingSend, durableStopOriginal, matchingPendingMutation, matchingPendingSend, MAX_PENDING_SENDS,
  samePendingIntent, sanitizePendingSends, type PendingSend } from "./pending-sends.ts";
import { planLegacyReadMarkMigration, sanitizeUiContinuity } from "./ui-continuity.ts";
import { deferred, pendingManagerApp, settle } from "./pending-manager-fixture.test.ts";
import { visualFixtureResponse } from "./visual-fixture.ts";

const token = "1".repeat(32), context = "a".repeat(64), target = "b".repeat(64);
const post: PendingSend = { token, server: 7, channel: "9", expectedContext: context, text: "same text", replyTo: "" };
const edit: PendingSend = { ...post, kind: "edit", messageId: "message", target, replyTo: "" };
const deletion: PendingSend = { ...edit, kind: "delete", text: "" };
const collection = (item: PendingSend) => ({ [item.token]: item });
const durable = { accepted: true, persistence: { status: "durable" as const }, disposition: "committed" as const };
const stopped = { accepted: false, persistence: { status: "durable" as const }, disposition: "stopped-before-commit" as const };

test("v1 sends retain their original shape while v2 action and Stop hydration reject downgrade/coercion", () => {
  assert.equal(JSON.stringify(sanitizePendingSends(collection(post))), JSON.stringify(collection(post)));
  const other = { drafts: { room: "draft" }, readMarks: { room: 42 }, studioDraft: null,
    statusCursors: { "7": { ts: 9, ids: [] } }, embedAutoLoad: true };
  const state = sanitizeUiContinuity({ version: 2, ...other, pendingSends: collection(edit) });
  assert.deepEqual(state.pendingSends, collection(edit));
  assert.equal(state.version, 2);
  assert.equal(planLegacyReadMarkMigration(state, '{"legacy":1}').state.version, 2);
  assert.deepEqual(sanitizePendingSends(collection(deletion), 2), collection(deletion));
  for (const input of [{ version: 3 }, { version: "2" },
    { version: 1, pendingSends: collection(edit) }, { version: 1, pendingSends: collection({ ...post, resolution: "cancel" }) },
    ...["future", ["edit"], {}, null].map(kind => ({ version: 2, pendingSends: collection({ ...edit, kind } as unknown as PendingSend) })),
    { version: 2, pendingSends: collection({ ...post, action: "edit" } as unknown as PendingSend) },
    { version: 2, pendingSends: collection({ ...post, target } as unknown as PendingSend) }]) {
    assert.throws(() => sanitizeUiContinuity(input), /sealed copy has been kept/);
  }
});

test("mutation matching binds kind, immutable target and exact text; composer matching sees only posts", () => {
  const pending = collection(edit);
  assert.equal(matchingPendingSend(pending, 7, "9", edit.text, ""), undefined);
  assert.equal(matchingPendingMutation(pending, 7, "9", "message", target, "edit", edit.text), edit);
  for (const [digest, kind, text] of [["c".repeat(64), "edit", edit.text], [target, "delete", ""], [target, "edit", "other"]] as const) {
    assert.equal(matchingPendingMutation(pending, 7, "9", "message", digest, kind, text), undefined);
  }
  assert.equal(samePendingIntent(edit, { ...edit, target: "c".repeat(64) }), false);
  assert.equal(samePendingIntent(edit, { ...edit, resolution: "cancel" }), false);
  assert.deepEqual(durableStopOriginal(edit), { kind: "edit", token, expected_context: context, channel: "9", message_id: "message", target, text: edit.text });
  assert.deepEqual(durableStopOriginal(deletion), { kind: "delete", token, expected_context: context, channel: "9", message_id: "message", target });
});

test("sends, edits and deletes share the same non-evicting pending pressure cap", () => {
  let pending = {};
  for (let n = 0; n < MAX_PENDING_SENDS; n++) pending = addPendingSend(pending,
    { ...[post, edit, deletion][n % 3], token: n.toString(16).padStart(32, "0") } as PendingSend);
  assert.throws(() => addPendingSend(pending, { ...deletion, token: "f".repeat(32) }), /need attention/);
  assert.equal(Object.keys(pending).length, MAX_PENDING_SENDS);
});

test("mutation seals sticky v2 and ambiguous exact custody before IPC; lost replies retry without a context lookup or new token", async () => {
  const app = pendingManagerApp({}); await app.hydrate();
  app.answer(async () => { throw new Error("lost acknowledgement"); });
  await assert.rejects(app.mutation(7, "9", "message", "edit", edit.text), /lost acknowledgement/);
  const first = app.state().submissions[0], originalToken = first.args.retryToken as string;
  assert.equal(first.command, "edit_message");
  assert.equal(first.sealed.version, 2);
  assert.equal(first.sealed.pendingSends[originalToken].kind, "edit");
  assert.equal(first.sealed.pendingSends[originalToken].acceptance, "ambiguous");
  assert.equal(first.args.id, "message"); assert.equal(first.args.target, target);
  app.lock(); await app.unlock(); app.answer(async () => durable);
  await app.retry(true);
  assert.equal(app.state().submissions[1].args.retryToken, originalToken);
  assert.equal(app.state().contextCalls, 1, "retry uses captured native target, even after removal or epoch movement");
  assert.deepEqual(app.state().pendingSends, {});
  assert.equal(app.state().sealed.version, 2, "v2 remains sticky after all mutations leave the queue");
  app.done();
});

test("editor retains its draft on seal failure and cannot clear a newer editor or composer after either await", async () => {
  const app = pendingManagerApp({}); await app.hydrate(); app.conversation(7, "9");
  const row = { id: "message", text: "old" };
  app.startEdit(row); app.edit("edited"); app.failSaves(true); await app.saveEdit(row);
  assert.equal(app.state().editDraft, "edited"); assert.equal(app.state().submissions.length, 0);
  app.failSaves(false);
  const gate = deferred(); app.holdSave(gate.promise);
  const saving = app.saveEdit(row); await settle();
  app.startEdit({ id: "newer", text: "new editor" }); app.type("new composer");
  app.answer(async () => durable); gate.resolve(); await saving;
  assert.equal(app.state().editingId, "newer"); assert.equal(app.state().editDraft, "new editor");
  assert.equal(app.state().draft, "new composer");
  assert.equal(app.state().submissions[0].args.id, "message");
  app.done();
});

test("shared view teardown drops retained editor plaintext on lock and invalidates its old revision", async () => {
  const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  const teardownStart = source.indexOf("  function clearServerView()");
  const teardown = source.slice(teardownStart, source.indexOf("\n  }\n", teardownStart) + 5);
  assert.match(teardown, /cancelEdit\(\);/);
  const lockStart = source.indexOf("  function lockScreen(");
  const lock = source.slice(lockStart, source.indexOf("\n  }\n", lockStart) + 5);
  assert.match(lock, /clearServerView\(\);/);
  const app = pendingManagerApp({}); await app.hydrate(); app.conversation(7, "9");
  const row = { id: "message", text: "private original" };
  app.startEdit(row); app.edit("private retained edit"); app.failSaves(true); await app.saveEdit(row);
  assert.equal(app.state().editDraft, "private retained edit");
  app.lock(); assert.equal(app.state().editingId, ""); assert.equal(app.state().editDraft, "");
  app.failSaves(false); await app.unlock();
  assert.equal(app.state().editingId, ""); assert.equal(app.state().editDraft, "");
  app.done();
});

test("unchanged text still reaches native once for a durable no-op receipt and ambiguity never reevaluates locally", async () => {
  const app = pendingManagerApp({}); await app.hydrate(); app.conversation(7, "9");
  const row = { id: "message", text: "same text" }; app.startEdit(row);
  app.answer(async () => { throw new Error("no-op reply lost"); });
  await app.saveEdit(row);
  const first = app.state().submissions[0];
  assert.equal(first.command, "edit_message"); assert.equal(first.args.text, row.text);
  app.answer(async () => durable); await app.retry();
  assert.deepEqual(app.state().submissions[1].args, first.args);
  assert.equal(app.state().contextCalls, 1);
  app.done();
});

test("unknown continuity action/version preserves all sealed bytes and blocks replacement saves", async () => {
  const app = pendingManagerApp({ version: 2, drafts: { "7:9": "keep draft" }, pendingSends: collection(edit) });
  await app.hydrate();
  for (const saved of [{ version: 3, drafts: { "7:9": "future draft" }, pendingSends: collection(edit) },
    { version: 2, drafts: { "7:9": "future draft" }, pendingSends: collection({ ...edit, kind: "future" } as unknown as PendingSend) }]) {
    app.replaceSealed(saved); const writes = app.state().writes.length;
    await app.hydrate(); assert.equal(await app.saveImmediately(), false);
    await app.retry(true); await app.send();
    assert.deepEqual(app.state().sealed, saved);
    assert.equal(app.state().writes.length, writes);
    assert.equal(app.state().drafts["7:9"], "keep draft");
    assert.deepEqual(app.state().pendingSends, collection(edit));
  }
  app.done();
});

test("mutation captures the original server/channel before context awaits and preserves later same-text editor revisions", async () => {
  const app = pendingManagerApp({}); await app.hydrate(); app.conversation(7, "9");
  const gate = deferred(); app.holdContext(gate.promise);
  const row = { id: "message", text: "old" }; app.startEdit(row); app.edit("edited");
  const saving = app.saveEdit(row); await settle();
  app.edit("newer"); app.edit("edited"); app.conversation(8, "10");
  app.answer(async () => durable); gate.resolve(); await saving;
  assert.equal(app.state().submissions[0].args.server, 7); assert.equal(app.state().submissions[0].args.channel, "9");
  assert.equal(app.state().editDraft, "edited"); assert.equal(app.state().editingId, "message");
  app.done();
});

test("Stop seals exact original identity before dispatch and survives lost replies, blocked retries and reload", async () => {
  const app = pendingManagerApp({ version: 2, pendingSends: collection({ ...edit, retryBlock: "context_changed" }) }); await app.hydrate();
  app.answerStop(async () => { throw new Error("Stop reply lost"); });
  await assert.rejects(app.resolve(token, "recover"), /Stop reply lost/);
  const stop = app.state().stops[0];
  assert.equal(stop.sealed.pendingSends[token].resolution, "recover");
  assert.deepEqual(stop.args, { server: 7, original: durableStopOriginal(edit) });
  assert.equal(app.state().pendingSends[token].resolution, "recover");
  app.lock(); await app.unlock(); app.answerStop(async () => stopped);
  await app.retry(true);
  assert.equal(app.state().stops.length, 2); assert.equal(app.state().submissions.length, 0);
  assert.deepEqual(app.state().stops[1].args, stop.args);
  assert.deepEqual(app.state().pendingSends, {});
  assert.equal(app.state().recoveredSendDrafts[token].kind, "edit");
  assert.equal(app.state().recoveredSendDrafts[token].resolution, undefined);
  app.done();
});

test("Stop cannot dispatch before its seal or release capacity on a missing/undurable terminal", async () => {
  const app = pendingManagerApp({ pendingSends: collection(post) }); await app.hydrate(); app.failSaves(true);
  await assert.rejects(app.resolve(token, "cancel"), /decision was not completed/);
  assert.equal(app.state().stops.length, 0); app.failSaves(false);
  for (const reply of [{ accepted: false, persistence: { status: "pending", reason: "write_failed" } },
    { accepted: true, persistence: { status: "superseded" }, disposition: "committed" },
    { accepted: false, persistence: { status: "durable" } },
    { accepted: false, persistence: { status: "durable" }, disposition: "unknown" }]) {
    app.answerStop(async () => reply);
    await assert.rejects(app.resolve(token, "cancel"), /awaiting a durable confirmation/);
    assert.equal(app.state().pendingSends[token].resolution, "cancel");
  }
  app.answerStop(async () => stopped); await app.retry();
  assert.deepEqual(app.state().pendingSends, {}); assert.equal(app.state().submissions.length, 0);
  app.done();
});

test("an unavailable original server cannot claim a durable Stop or silently free its pending slot", async () => {
  const app = pendingManagerApp({ pendingSends: collection({ ...post, server: 999, retryBlock: "invalid" }) }); await app.hydrate();
  app.answerStop(async () => { throw new Error("original server is unavailable"); });
  await assert.rejects(app.resolve(token, "cancel"), /server is unavailable/);
  assert.equal(app.state().pendingSends[token].resolution, "cancel");
  assert.equal(Object.keys(app.state().pendingSends).length, 1);
  app.lock(); await app.unlock(); await app.retry(true);
  assert.equal(app.state().stops.length, 2);
  assert.equal(app.state().submissions.length, 0);
  assert.equal(app.state().sealed.pendingSends[token].resolution, "cancel");
  app.done();
});

test("failed Stop cleanup preserves the saved negative intent and cannot remint a positive retry", async () => {
  const app = pendingManagerApp({ version: 2, pendingSends: collection(deletion) }); await app.hydrate();
  app.answerStop(async () => { app.failSaves(true); return stopped; });
  await assert.rejects(app.resolve(token, "cancel"), /decision was not completed/);
  assert.equal(app.state().pendingSends[token].resolution, "cancel");
  app.failSaves(false);
  await assert.rejects(app.mutation(7, "9", "message", "delete", ""), /awaiting Stop confirmation/);
  assert.equal(app.state().submissions.length, 0);
  app.answerStop(async () => stopped); await app.retry(true);
  assert.deepEqual(app.state().pendingSends, {}); assert.deepEqual(app.state().recoveredSendDrafts, {});
  app.done();
});

test("late positive completion cannot remove resolving Stop or clear a newer composer", async () => {
  const app = pendingManagerApp({ version: 2, pendingSends: collection(edit) }); await app.hydrate(); app.conversation(7, "9");
  const positive = deferred<any>(), negative = deferred<any>();
  app.answer(() => positive.promise); app.answerStop(() => negative.promise);
  const submitting = app.submit(token); await settle();
  const stopping = app.resolve(token, "cancel"); await settle();
  app.type("newer work"); positive.resolve(durable); await submitting;
  assert.equal(app.state().pendingSends[token].resolution, "cancel");
  assert.equal(app.state().draft, "newer work");
  negative.resolve({ ...stopped, disposition: "stopped-after-commit" }); await stopping;
  assert.deepEqual(app.state().pendingSends, {}); assert.equal(app.state().draft, "newer work");
  app.done();
});

test("visual fixture explicitly models mutation metadata and closed durable terminals", () => {
  assert.deepEqual(visualFixtureResponse("durable_mutation_context", { server: 7, channel: "9", id: "message" }), { context, target });
  for (const command of ["edit_message", "delete_message"]) {
    const reply = visualFixtureResponse(command, { server: 7, channel: "9", id: "message" }) as typeof durable;
    assert.equal(reply.disposition, "committed"); assert.equal(reply.persistence.status, "durable");
  }
  assert.equal((visualFixtureResponse("stop_durable_chat", { server: 7, original: durableStopOriginal(edit) }) as typeof stopped).disposition, "stopped-before-commit");
});
