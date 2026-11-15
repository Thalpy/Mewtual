import assert from "node:assert/strict";
import test from "node:test";
import { addPendingSend, pendingContinuityVersion, quarantinedPendingIntent, sanitizePendingSends, type PendingSend } from "./pending-sends.ts";
import { sanitizeUiContinuity } from "./ui-continuity.ts";
import { deferred, pendingManagerApp, settle } from "./pending-manager-fixture.test.ts";

const token = "1".repeat(32);
const post: PendingSend = { token, server: 7, channel: "9", expectedContext: "a".repeat(64),
  text: "the complete original message", replyTo: "", acceptance: "ambiguous", retryBlock: "invalid" };
const collection = (intent: PendingSend) => ({ [intent.token]: intent });
const durable = { accepted: true, persistence: { status: "durable" }, disposition: "committed" };
const oldRoot = (intent: PendingSend, version = 1, shelf = "pendingSends") => ({ version,
  drafts: { "7:9": "unrelated draft" }, readMarks: { "7:9": { ts: 42, id: "read" } },
  [shelf]: collection(intent), extensionFromAnotherFeature: { exact: ["keep", 8] } });

test("actual Send admits native UTF-8 boundaries and refuses invalid input before context/queue/seal", async () => {
  for (const [text, reply] of [["x".repeat(65_536), "r".repeat(256)], ["😀".repeat(16_384), "é".repeat(128)]]) {
    const app = pendingManagerApp({}); await app.hydrate(); app.conversation(7, "9");
    app.type(text); app.reply(reply); app.answer(async () => durable);
    assert.equal(await app.send(), "sent"); assert.equal(app.state().contextCalls, 1);
    assert.equal(app.state().submissions[0].args.text, text); assert.equal(app.state().submissions[0].args.replyTo, reply);
    assert.equal(app.state().submissions[0].sealed.version, reply.length > 128 ? 3 : 1, "enlarged read grammar is sealed in critical3 before dispatch");
    app.done();
  }
  for (const [text, reply] of [["x".repeat(65_537), ""], ["😀".repeat(16_384) + "x", ""],
    ["valid text", "x".repeat(257)], ["valid text", "é".repeat(129)], ["\ud800", ""], ["\udc00", ""], ["valid text", "\ud800"]]) {
    const app = pendingManagerApp({}); await app.hydrate(); app.conversation(7, "9");
    app.type(text); app.reply(reply); const before = app.state();
    assert.equal(await app.send(), "failed");
    assert.equal(app.state().draft, text); assert.deepEqual(app.state().pendingSends, {});
    assert.equal(app.state().contextCalls, 0); assert.equal(app.state().submissions.length, 0);
    assert.equal(app.state().writes.length, before.writes.length); assert.deepEqual(app.state().sealed, before.sealed);
    app.done();
  }
});

test("full Send custody is validated on queue insertion while retained invalid originals stay readable", () => {
  for (const patch of [{ token: "bad" }, { expectedContext: "bad" }, { server: -1 }, { channel: "01" },
    { channel: String(1n << 128n) }, { text: "x".repeat(65_537) }, { replyTo: "x".repeat(300) }]) {
    assert.throws(() => addPendingSend({}, { ...post, ...patch }));
  }
  for (const intent of [{ ...post, text: "x".repeat(65_537) }, { ...post, replyTo: "x".repeat(300) },
    { ...post, replyTo: "漢".repeat(100) }]) {
    assert.deepEqual(sanitizePendingSends(collection(intent)), collection(intent));
    assert.equal(quarantinedPendingIntent(intent), true); assert.equal(pendingContinuityVersion(intent), 3);
    for (const shelf of ["pendingSends", "recoveredSendDrafts"]) assert.equal(sanitizeUiContinuity(oldRoot(intent, 1, shelf)).version, 3);
  }
  assert.equal(quarantinedPendingIntent({ ...post, replyTo: "x".repeat(150) }), false);
  assert.equal(pendingContinuityVersion({ ...post, replyTo: "x".repeat(150) }), 3);
  assert.throws(() => sanitizeUiContinuity({ version: 4 }), /unsupported/);
});

test("old invalid Sends in either shelf upgrade exact root to3 and recover losslessly without positive or Stop IPC", async () => {
  for (const intent of [{ ...post, text: "x".repeat(65_537) }, { ...post, replyTo: "x".repeat(300) },
    { ...post, replyTo: "漢".repeat(100) }]) {
    for (const shelf of ["pendingSends", "recoveredSendDrafts"]) {
      const app = pendingManagerApp({}); const raw = oldRoot(intent, 1, shelf); app.replaceSealed(raw);
      await app.hydrate(); assert.equal(app.state().uiStateReady, true);
      assert.deepEqual(app.state().sealed, { ...raw, version: 3 }, "critical migration changes only original root version");
      await app.retry(true); await app.retry();
      if (shelf === "pendingSends") {
        await assert.rejects(app.submit(token), /cannot be retried/);
        await assert.rejects(app.resolve(token, "cancel"), /stay paused/);
        await app.resolve(token, "recover");
      }
      app.lock(); await app.unlock(); await app.retry(true);
      assert.equal(app.state().submissions.length, 0); assert.equal(app.state().stops.length, 0);
      assert.deepEqual(app.state().pendingSends, {}); assert.deepEqual(app.state().recoveredSendDrafts, collection(intent));
      assert.equal(app.state().sealed.version, 3);
      app.conversation(7, "9"); app.type("new normal message"); app.answer(async () => durable); await app.send();
      assert.equal(app.state().submissions.length, 1); assert.equal(app.state().submissions[0].command, "send_message");
      assert.deepEqual(app.state().recoveredSendDrafts, collection(intent));
      app.done();
    }
  }
});

test("pending v2 Stop decisions on invalid Sends remain exact and never call Stop after critical migration", async () => {
  const original = { ...post, replyTo: "x".repeat(300), resolution: "cancel" as const };
  const app = pendingManagerApp({}); app.replaceSealed(oldRoot(original, 2)); await app.hydrate();
  await app.retry(true); await app.retry(); await assert.rejects(app.submit(token), /cannot be retried/);
  await app.resolve(token, "recover"); app.lock(); await app.unlock();
  assert.deepEqual(app.state().recoveredSendDrafts, collection(original));
  assert.equal(app.state().stops.length, 0); assert.equal(app.state().submissions.length, 0); app.done();
});

test("critical save failure blocks readiness and a retry re-saves even an already3 postrename candidate", async () => {
  const original = { ...post, text: "x".repeat(65_537) }, raw = oldRoot(original);
  const app = pendingManagerApp({}); app.replaceSealed(raw); app.failSaves(true); await app.hydrate();
  assert.equal(app.state().uiStateReady, false); assert.deepEqual(app.state().sealed, raw);
  app.conversation(7, "9"); app.type("new normal message"); await app.send();
  await assert.rejects(app.resolve(token, "recover"), /vault is ready/);
  assert.equal(app.state().contextCalls, 0); assert.equal(app.state().stops.length, 0);
  // Model a real rename followed by failed parent-sync acknowledgement. Seeing3 is
  // not permission: every retry crosses the same covering save barrier again.
  app.replaceSealed({ ...raw, version: 3 }); const writes = app.state().writes.length;
  await app.hydrate(); assert.equal(app.state().uiStateReady, false); assert.equal(app.state().writes.length, writes + 1);
  app.failSaves(false); await app.hydrate();
  assert.equal(app.state().uiStateReady, true); assert.deepEqual(app.state().pendingSends, collection(original));
  app.done();
});

test("held critical save keeps UI unready and lock invalidates its result before reopening exact3", async () => {
  const app = pendingManagerApp({}); const original = { ...post, replyTo: "x".repeat(300) };
  app.replaceSealed(oldRoot(original)); const gate = deferred(); app.holdSave(gate.promise);
  const loading = app.hydrate(); await settle();
  assert.equal(app.state().uiStateReady, false); assert.deepEqual(app.state().pendingSends, {});
  app.conversation(7, "9"); app.type("new normal message"); await app.send(); await app.retry(true);
  await assert.rejects(app.resolve(token, "recover"), /vault is ready/);
  assert.equal(app.state().contextCalls, 0); assert.equal(app.state().submissions.length, 0); assert.equal(app.state().stops.length, 0);
  app.lock(); gate.resolve(); await loading;
  assert.equal(app.state().uiStateReady, false); await app.unlock();
  assert.equal(app.state().uiStateReady, true); assert.deepEqual(app.state().pendingSends, collection(original)); app.done();
});

test("a superseded same-generation read cannot cover or expose stale state after a newer load", async () => {
  const app = pendingManagerApp({}); const old = oldRoot({ ...post, replyTo: "x".repeat(300) }); app.replaceSealed(old);
  const read = deferred(); app.holdLoad(read.promise); const first = app.hydrate(); await settle();
  const newer = { ...old, version: 3, drafts: { "7:9": "newer sealed draft" } }; app.replaceSealed(newer);
  await app.hydrate(); app.conversation(7, "9"); app.type("newer current typing"); assert.equal(await app.saveImmediately(), true);
  const sealed = app.state().sealed, writes = app.state().writes.length; read.resolve(); await first;
  assert.equal(app.state().uiStateReady, true); assert.deepEqual(app.state().sealed, sealed);
  assert.equal(app.state().draft, "newer current typing"); assert.equal(app.state().writes.length, writes); app.done();
});

test("overlapping same-generation loads retire prior covering writes before reading and only the last opens UI", async () => {
  const app = pendingManagerApp({}); app.replaceSealed(oldRoot({ ...post, replyTo: "x".repeat(300) }));
  const firstSave = deferred(), secondSave = deferred(); app.holdSave(firstSave.promise); app.holdSave(secondSave.promise);
  const first = app.hydrate(); await settle(); const second = app.hydrate(); await settle();
  assert.equal(app.state().writes.length, 1); firstSave.resolve(); await first; await settle();
  assert.equal(app.state().uiStateReady, false); assert.equal(app.state().writes.length, 2);
  secondSave.resolve(); await second; assert.equal(app.state().uiStateReady, true); app.done();
});

test("native-valid long reply replays exact accepted identity and3 remains sticky through mutations and Stop", async () => {
  const original = { ...post, replyTo: "x".repeat(150), retryBlock: undefined };
  const app = pendingManagerApp({}); app.replaceSealed(oldRoot(original)); await app.hydrate();
  app.answer(async () => { throw new Error("earlier accepted reply lost"); }); await app.retry(true);
  const args = app.state().submissions[0].args;
  assert.equal(args.retryToken, token); assert.equal(args.expectedContext, original.expectedContext);
  assert.equal(args.replyTo, original.replyTo); assert.equal(app.state().submissions[0].sealed.version, 3);
  assert.equal(app.state().contextCalls, 0); app.lock(); await app.unlock(); app.answer(async () => durable); await app.retry(true);
  assert.deepEqual(app.state().submissions[1].args, args); assert.deepEqual(app.state().pendingSends, {});
  app.conversation(7, "9"); await app.mutation(7, "9", "message", "edit", "changed");
  assert.equal(app.state().submissions[2].sealed.version, 3);
  app.type("normal Stop request"); app.answer(async () => { throw new Error("reply lost"); }); await app.send();
  const stopToken = Object.keys(app.state().pendingSends)[0]; await app.resolve(stopToken, "cancel");
  assert.equal(app.state().stops[0].sealed.version, 3); assert.equal(app.state().sealed.version, 3);
  app.lock(); await app.unlock(); assert.equal(app.state().sealed.version, 3); app.done();
});

test("v3 row/byte overflow and malformed fixed custody preserve the whole sealed root without truncation", async () => {
  const overflow = Object.fromEntries(Array.from({ length: 33 }, (_, n) => {
    const item = { ...post, token: n.toString(16).padStart(32, "0") }; return [item.token, item];
  }));
  for (const rows of [overflow, collection({ ...post, text: "x".repeat(256 * 1024) }),
    collection({ ...post, expectedContext: "bad" }), { badToken: post }, { [token]: null }]) {
    const original = { version: 3, drafts: {}, readMarks: {}, pendingSends: rows };
    const app = pendingManagerApp({}); app.replaceSealed(original); await app.hydrate();
    assert.equal(app.state().uiStateReady, false); assert.deepEqual(app.state().sealed, original);
    assert.equal(app.state().writes.length, 0); assert.equal(app.state().submissions.length, 0); assert.equal(app.state().stops.length, 0);
    assert.equal(await app.saveImmediately(), false); app.done();
  }
});

test("unsupported present pending or recovered collections stay sealed and cannot become empty ready queues", async () => {
  assert.deepEqual(sanitizePendingSends(undefined), {});
  for (const shelf of ["pendingSends", "recoveredSendDrafts"]) {
    for (const value of [[post], "unknown collection", 42, null]) {
      const original = { version: 3, drafts: {}, readMarks: {}, [shelf]: value };
      const app = pendingManagerApp({}); app.replaceSealed(original); await app.hydrate();
      assert.equal(app.state().uiStateReady, false); assert.deepEqual(app.state().sealed, original);
      assert.equal(app.state().writes.length, 0); assert.equal(app.state().submissions.length, 0); assert.equal(app.state().stops.length, 0);
      assert.equal(await app.saveImmediately(), false); app.done();
    }
  }
});
