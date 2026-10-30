import assert from "node:assert/strict";
import test from "node:test";
import { deferred, pendingManagerApp, settle } from "./pending-manager-fixture.test.ts";

const token = "1".repeat(32);
const item = { token, server: 77, channel: "9", expectedContext: "a".repeat(64),
  text: "Do not publish this unresolved request", replyTo: "", acceptance: "not_accepted" };
const initial = () => ({ pendingSends: { [token]: item } });

test("actual close retires the open video review before a held continuity save finishes", async () => {
  const app = pendingManagerApp(initial()); await app.hydrate();
  const answer = app.review(new File([new Uint8Array(1)], "clip.mp4", { type: "video/mp4" }));
  assert.ok(app.state().videoReview);
  const gate = deferred(); app.holdSave(gate.promise);
  const saving = app.save(); await settle();
  const closing = app.close(); await settle();
  assert.equal(app.state().videoReview, null);
  assert.equal(app.state().videoScopeEpoch, 1);
  assert.deepEqual(await answer, { kind: "skip" });
  assert.equal(app.state().closeCalls.length, 0, "native close is still behind the held save");
  gate.resolve(); await Promise.all([saving, closing]);
  app.done();
});

for (const action of ["cancel", "recover"] as const) {
  test(`actual close drains the ${action} Stop seal and preserves it for exact retry after reopening`, async () => {
    const app = pendingManagerApp(initial()); await app.hydrate();
    const gate = deferred(); app.holdSave(gate.promise);
    const decision = app.resolve(token, action); await settle();
    assert.equal(app.state().writes[0].pendingSends[token].resolution, action, "the candidate retains exact custody until native Stop is durable");
    assert.ok(app.state().pendingSends[token], "logical transaction has not committed yet");
    const closing = app.close(); await settle();
    const rejected = assert.rejects(decision, /being resolved or its session changed/);
    const closeDispatchedBeforeDecision = app.state().closeCalls.length;
    gate.resolve(); await Promise.all([rejected, closing]);
    assert.equal(closeDispatchedBeforeDecision, 0, "close may not capture/dispatch a competing old snapshot");
    assert.equal(JSON.parse(app.state().closeCalls[0].uiStateJson).pendingSends[token].resolution, action);
    assert.equal(app.state().stops.length, 0, "close fences new native Stop dispatch after draining its seal");
    await app.unlock(); await app.retry(true);
    assert.deepEqual(app.state().pendingSends, {});
    assert.equal(app.state().submissions.length, 0, "successful resolution cannot become automatic publication after reopening");
    if (action === "recover") assert.deepEqual(app.state().recoveredSendDrafts[token], item);
    else assert.deepEqual(app.state().recoveredSendDrafts, {});
    app.done();
  });

  test(`actual close preserves the original request when ${action} cannot save`, async () => {
    const app = pendingManagerApp(initial()); await app.hydrate();
    const gate = deferred(); app.holdSave(gate.promise);
    const decision = app.resolve(token, action);
    const rejected = assert.rejects(decision, /injected decision failure/); await settle();
    const closing = app.close(); await settle();
    assert.equal(app.state().closeCalls.length, 0);
    gate.reject(new Error("injected decision failure"));
    await Promise.all([rejected, closing]);
    assert.deepEqual(JSON.parse(app.state().closeCalls[0].uiStateJson).pendingSends[token], item);
    await app.unlock();
    assert.deepEqual(app.state().pendingSends[token], item, "failed decision retains identity/payload/capacity, not a cancellation claim");
    assert.deepEqual(app.state().recoveredSendDrafts, {});
    app.done();
  });
}

test("close drains admitted ordinary/decision saves but refuses new decisions and publication", async () => {
  const app = pendingManagerApp(initial()); await app.hydrate();
  app.conversation(item.server, item.channel); app.type("a different draft");
  const gate = deferred(); app.holdSave(gate.promise);
  const preceding = app.save(); await settle();
  const decision = app.resolve(token, "recover");
  const rejected = assert.rejects(decision, /being resolved or its session changed/);
  const following = app.save();
  const closing = app.close(); await settle();
  await assert.rejects(app.save(), /window is closing/);
  await assert.rejects(app.resolve(token, "cancel"), /vault is ready/);
  await assert.rejects(app.submit(token), /being resolved or its session changed/);
  await app.send(); await app.retry(true);
  assert.equal(app.state().contextCalls, 0);
  assert.equal(app.state().submissions.length, 0);
  assert.equal(app.state().closeCalls.length, 0);
  gate.resolve(); await Promise.all([preceding, rejected, following, closing]);
  assert.equal(app.state().writes.length, 3, "every admitted queue entry runs under the close fence");
  assert.equal(app.state().writes[2].pendingSends[token].resolution, "recover");
  const final = JSON.parse(app.state().closeCalls[0].uiStateJson);
  assert.equal(final.pendingSends[token].resolution, "recover");
  assert.deepEqual(final.recoveredSendDrafts, {}, "no text recovery is claimed before native Stop");
  assert.equal(final.drafts["77:9"], "a different draft");
  await app.unlock(); await app.retry(true);
  assert.equal(app.state().recoveredSendDrafts[token].text, item.text);
  assert.equal(app.state().stops.length, 1);
  app.done();
});

test("a close deadline defers before native close, retaining the decision until a later close", async () => {
  const app = pendingManagerApp(initial()); await app.hydrate();
  const gate = deferred(); app.holdSave(gate.promise);
  const decision = app.resolve(token, "cancel"); await settle();
  const closing = app.close(); await settle(); app.expireCloseWait(); await closing;
  assert.equal(app.state().windowCloseInFlight, false);
  assert.equal(app.state().pendingSendResolution, token);
  assert.ok(app.state().pendingSends[token]);
  assert.match(app.state().error, /window remains open/);
  assert.equal(app.state().closeCalls.length, 0);
  assert.equal(app.state().destroyCalls, 0);
  assert.equal(app.state().restartCalls, 0);
  await assert.rejects(app.resolve(token, "recover"), /vault is ready/);
  gate.resolve(); await decision; await app.close(); await app.unlock(); await app.retry(true);
  assert.deepEqual(app.state().pendingSends, {});
  assert.equal(app.state().submissions.length, 0);
  app.done();
});

test("close fences a submission continuation waiting for its pre-dispatch save", async () => {
  const app = pendingManagerApp(initial()); await app.hydrate();
  const gate = deferred(); app.holdSave(gate.promise);
  const submitting = app.submit(token);
  const rejected = assert.rejects(submitting, /before retrying/); await settle();
  const closing = app.close(); gate.resolve(); await Promise.all([rejected, closing]);
  assert.equal(app.state().submissions.length, 0);
  assert.equal(app.state().sealed.pendingSends[token].token, token);
  app.done();
});

test("a late authoring-context answer cannot mint a token while close is draining", async () => {
  const app = pendingManagerApp({}); await app.hydrate();
  app.conversation(77, "9"); app.type("new draft");
  const context = deferred(); app.holdContext(context.promise);
  const sending = app.send(); await settle();
  const gate = deferred(); app.holdSave(gate.promise);
  const ordinary = app.save(); await settle(); const closing = app.close();
  context.resolve(); await sending;
  assert.deepEqual(app.state().pendingSends, {});
  assert.equal(app.state().submissions.length, 0);
  gate.resolve(); await Promise.all([ordinary, closing]);
  assert.equal(app.state().sealed.drafts["77:9"], "new draft");
  app.done();
});

test("immediate lock interrupts a decision without reporting success or letting the old close capture a new session", async () => {
  const app = pendingManagerApp(initial()); await app.hydrate();
  const gate = deferred(); app.holdSave(gate.promise);
  const decision = app.resolve(token, "cancel");
  const rejected = assert.rejects(decision, /session changed/); await settle();
  const closing = app.close(); await settle(); app.lock();
  assert.equal(app.state().locked, true, "visual lock remains immediate");
  gate.resolve(); await Promise.all([rejected, closing]);
  assert.equal(app.state().closeCalls.length, 0, "old close must be retried against the now-locked generation");
  await app.close();
  assert.deepEqual(JSON.parse(app.state().closeCalls[0].uiStateJson).pendingSends[token], item,
    "fresh locked close reuses the exact immutable lock snapshot; decision was interrupted");
  await app.unlock(); assert.deepEqual(app.state().pendingSends[token], item);
  app.done();
});

test("a replacement UI generation aborts the old close before snapshot or native IPC", async () => {
  const app = pendingManagerApp(initial()); await app.hydrate();
  const gate = deferred(); app.holdSave(gate.promise);
  const saving = app.save(); const rejected = assert.rejects(saving, /session changed/); await settle();
  const closing = app.close(); await settle(); app.replaceUiGeneration();
  gate.resolve(); await Promise.all([rejected, closing]);
  assert.equal(app.state().closeCalls.length, 0);
  assert.equal(app.state().windowCloseInFlight, false);
  assert.equal(app.state().locked, false);
  app.done();
});
