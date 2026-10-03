import assert from "node:assert/strict";
import test from "node:test";
import { MAX_PENDING_SENDS, pendingAcceptanceWarning, type PendingSend } from "./pending-sends.ts";
import { deferred, pendingManagerApp, settle } from "./pending-manager-fixture.test.ts";

const intent = (n = 1, overrides: Partial<PendingSend> = {}): PendingSend => ({
  token: n.toString(16).padStart(32, "0"), server: 77, channel: "9", expectedContext: "a".repeat(64),
  text: `orphaned message ${n}`, replyTo: "original-parent", ...overrides,
});
const collection = (entries: PendingSend[]) => Object.fromEntries(entries.map(item => [item.token, item]));

test("all permanent and uncertain orphaned requests can be recovered without a mounted conversation or publication", async () => {
  for (const retryBlock of ["invalid", "conflict", "context_changed", undefined] as const) {
    const item = intent(1, retryBlock ? { retryBlock } : {});
    const app = pendingManagerApp({ pendingSends: collection([item]) });
    await app.hydrate();
    assert.match(pendingAcceptanceWarning(app.state().pendingSends[item.token]), /may already have been accepted/);
    await app.resolve(item.token, "recover");
    assert.deepEqual(app.state().pendingSends, {});
    assert.deepEqual(app.state().recoveredSendDrafts[item.token], item);
    assert.deepEqual(app.state().submissions, []);
    app.lock(); await app.unlock();
    assert.deepEqual(app.state().pendingSends, {});
    assert.deepEqual(app.state().recoveredSendDrafts[item.token], item);
    await app.retry(true);
    assert.equal(app.state().submissions.length, 0, "recovered copies are never retry intents");
    await app.remove(item.token);
    app.lock(); await app.unlock();
    assert.deepEqual(app.state().recoveredSendDrafts, {});
    app.done();
  }
});

test("capacity retires only after an explicit sealed decision and full recovery still permits stop retrying", async () => {
  const entries = Array.from({ length: MAX_PENDING_SENDS }, (_, n) => intent(n + 1, { retryBlock: "invalid" }));
  const recovered = Array.from({ length: MAX_PENDING_SENDS }, (_, n) => intent(n + 100));
  const app = pendingManagerApp({ pendingSends: collection(entries), recoveredSendDrafts: collection(recovered) });
  await app.hydrate();
  assert.throws(() => app.add(intent(99)), /need attention/);
  await assert.rejects(app.resolve(entries[0].token, "recover"), /need attention/);
  assert.equal(Object.keys(app.state().pendingSends).length, 32);
  const gate = deferred(); app.holdSave(gate.promise);
  const resolve = app.resolve(entries[0].token, "cancel"); await settle();
  assert.throws(() => app.add(intent(99)), /need attention/, "capacity remains occupied until the write succeeds");
  gate.resolve(); await resolve;
  assert.equal(Object.keys(app.state().pendingSends).length, 31);
  app.lock(); await app.unlock();
  app.add(intent(99));
  assert.equal(Object.keys(app.state().pendingSends).length, 32);
  assert.equal(Object.keys(app.state().recoveredSendDrafts).length, 32);
  assert.equal(app.state().submissions.length, 0);
  app.done();
});

test("failed resolution and envelope size refusal preserve exact pending identity, payload and capacity", async () => {
  for (const failure of ["disk", "size"]) {
    const item = intent(); const app = pendingManagerApp({ pendingSends: collection([item]) });
    await app.hydrate();
    if (failure === "disk") app.failSaves(true); else app.bulkyPreferences();
    await assert.rejects(app.resolve(item.token, "recover"), /decision was not completed/);
    assert.deepEqual(app.state().pendingSends[item.token], item);
    assert.deepEqual(app.state().recoveredSendDrafts, {});
    assert.deepEqual(app.state().sealed.pendingSends[item.token], item);
    app.lock(); await app.unlock();
    assert.deepEqual(app.state().pendingSends[item.token], item);
    app.done();
  }
});

test("queued ordinary saves use current post-decision state and cannot resurrect retired work", async () => {
  const item = intent(); const app = pendingManagerApp({ pendingSends: collection([item]) });
  await app.hydrate();
  const first = deferred(); app.holdSave(first.promise);
  const ordinaryBefore = app.save(); await settle();
  const resolution = app.resolve(item.token, "recover");
  const ordinaryAfter = app.save();
  first.resolve(); await Promise.all([ordinaryBefore, resolution, ordinaryAfter]);
  assert.equal(app.state().writes.length, 3);
  assert.deepEqual(app.state().writes[0].pendingSends[item.token], item);
  for (const saved of app.state().writes.slice(1)) {
    assert.deepEqual(saved.pendingSends, {});
    assert.deepEqual(saved.recoveredSendDrafts[item.token], item);
  }
  app.lock(); await app.unlock();
  assert.deepEqual(app.state().pendingSends, {});
  app.done();
});

test("resolution during the pre-dispatch save prevents IPC and no retired token reappears", async () => {
  const item = intent(1, { acceptance: "not_accepted" });
  const app = pendingManagerApp({ pendingSends: collection([item]) }); await app.hydrate();
  const gate = deferred(); app.holdSave(gate.promise);
  const submit = app.submit(item.token); const caught = assert.rejects(submit, /before retrying/);
  await settle();
  const resolution = app.resolve(item.token, "cancel");
  gate.resolve(); await Promise.all([caught, resolution]);
  assert.equal(app.state().submissions.length, 0);
  assert.deepEqual(app.state().pendingSends, {});
  app.lock(); await app.unlock(); assert.deepEqual(app.state().pendingSends, {});
  app.done();
});

test("opening the manager during an automatic pass pauses its remaining requests", async () => {
  const first = intent(1), second = intent(2);
  const app = pendingManagerApp({ pendingSends: collection([first, second]) }); await app.hydrate();
  const ipc = deferred<any>(); app.answer(() => ipc.promise);
  const pass = app.retry(true); await settle();
  assert.equal(app.state().submissions.length, 1);
  app.manager(true);
  ipc.resolve({ accepted: false, persistence: { status: "pending", reason: "write_failed" } }); await pass;
  assert.equal(app.state().submissions.length, 1);
  assert.ok(app.state().pendingSends[second.token]);
  await app.resolve(second.token, "cancel");
  assert.equal(app.state().submissions.length, 1);
  app.done();
});

test("delayed IPC outcomes cannot recreate resolved work or clear a newer composer", async () => {
  for (const outcome of ["durable", "superseded", "error"]) {
    const item = intent(1, { acceptance: "not_accepted" });
    const app = pendingManagerApp({ pendingSends: collection([item]) }); await app.hydrate();
    app.conversation(item.server, item.channel); app.type(item.text);
    const ipc = deferred<any>(); app.answer(() => ipc.promise);
    const submitting = app.submit(item.token); const done = submitting.catch(() => {});
    await settle();
    assert.equal(app.state().submissions[0].sealed.pendingSends[item.token].acceptance, "ambiguous");
    await app.resolve(item.token, "recover"); app.type("newer composer text");
    if (outcome === "error") ipc.reject(new Error("CHAT_SEND_TOKEN_CONFLICT"));
    else ipc.resolve({ accepted: outcome === "durable", persistence: { status: outcome } });
    await done;
    assert.deepEqual(app.state().pendingSends, {});
    assert.equal(app.state().draft, "newer composer text");
    assert.equal(app.state().recoveredSendDrafts[item.token].acceptance, "ambiguous");
    assert.equal(app.state().submissions.length, 1);
    app.done();
  }
});

test("lock invalidates a delayed resolution save and reopening preserves its unresolved payload", async () => {
  const item = intent(); const app = pendingManagerApp({ pendingSends: collection([item]) }); await app.hydrate();
  const gate = deferred(); app.holdSave(gate.promise);
  const resolution = app.resolve(item.token, "cancel");
  const rejected = assert.rejects(resolution, /session changed/); await settle();
  app.lock(); gate.resolve(); await rejected; await app.unlock();
  assert.deepEqual(app.state().pendingSends[item.token], item);
  assert.equal(app.state().pendingSendResolution, null);
  app.done();
});

test("using a recovered copy saves a draft explicitly, retains full source, and never publishes or replaces newer text", async () => {
  const item = intent(); const app = pendingManagerApp({ recoveredSendDrafts: collection([item]) }); await app.hydrate();
  await assert.rejects(app.use(item.token), /Open a conversation/);
  app.conversation(8, "5");
  const gate = deferred(); app.holdSave(gate.promise);
  const use = app.use(item.token); await settle(); app.type("newer work"); gate.resolve(); await use;
  assert.equal(app.state().draft, "newer work");
  await app.save(); app.type(""); await app.use(item.token);
  assert.equal(app.state().draft, item.text);
  assert.equal(app.state().sealed.drafts["8:5"], item.text);
  assert.deepEqual(app.state().recoveredSendDrafts[item.token], item);
  assert.equal(app.state().submissions.length, 0);
  app.done();
});

test("recovered text above the composer limit remains whole and can still be explicitly removed", async () => {
  const item = intent(1, { text: "x".repeat(40_000) });
  const app = pendingManagerApp({ pendingSends: collection([item]) }); await app.hydrate();
  await app.resolve(item.token, "recover"); app.lock(); await app.unlock(); app.conversation(8, "5");
  await assert.rejects(app.use(item.token), /Copy the saved text instead/);
  assert.equal(app.state().recoveredSendDrafts[item.token].text.length, 40_000);
  assert.equal(app.state().draft, "");
  await app.remove(item.token);
  assert.equal(app.state().submissions.length, 0);
  app.done();
});
