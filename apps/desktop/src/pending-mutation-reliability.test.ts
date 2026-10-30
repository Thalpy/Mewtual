import assert from "node:assert/strict";
import test from "node:test";
import { addPendingSend, durableStopOriginal, MAX_PENDING_SENDS, pendingSendRetryBlock,
  quarantinedPendingMutation, sanitizePendingSends, sendRefusedUnaccepted, validateMutationPayload,
  type PendingSend } from "./pending-sends.ts";
import { deferred, pendingManagerApp, settle } from "./pending-manager-fixture.test.ts";

const token = "1".repeat(32);
const edit: PendingSend = { token, server: 7, channel: "9", expectedContext: "a".repeat(64),
  kind: "edit", messageId: "message", target: "b".repeat(64), text: "changed", replyTo: "" };
const collection = (intent: PendingSend) => ({ [intent.token]: intent });
const durable = { accepted: true, persistence: { status: "durable" }, disposition: "committed" };
// Deliberately preserve the preexisting resolution/diagnostics, rather than blessing the
// malformed request as absent or manufacturing a successful native Stop during recovery.
const oversized = { ...edit, text: "😀".repeat(16_385), acceptance: "ambiguous" as const,
  retryBlock: "context_changed" as const, resolution: "cancel" as const };

test("actual editor admits exactly 64 KiB UTF-8 and rejects excess before context, sealing or losing input", async () => {
  for (const text of ["x".repeat(65_536), "😀".repeat(16_384), "é".repeat(32_768)]) {
    const app = pendingManagerApp({}); await app.hydrate(); app.conversation(7, "9");
    const row = { id: "message", text: "old" }; app.startEdit(row); app.edit(text);
    app.answer(async () => durable); await app.saveEdit(row);
    assert.equal(app.state().contextCalls, 1);
    assert.equal(app.state().submissions[0].args.text, text);
    assert.equal(app.state().editDraft, "");
    app.done();
  }
  for (const text of ["x".repeat(65_537), "😀".repeat(16_384) + "x", "é".repeat(32_768) + "x"]) {
    const app = pendingManagerApp({ drafts: { "7:9": "other composer" } }); await app.hydrate(); app.conversation(7, "9");
    const row = { id: "message", text: "old" }; app.startEdit(row); app.edit(text);
    const before = app.state(); await app.saveEdit(row);
    assert.equal(app.state().editDraft, text); assert.equal(app.state().editingId, "message");
    assert.equal(app.state().contextCalls, 0); assert.equal(app.state().submissions.length, 0);
    assert.deepEqual(app.state().pendingSends, {}); assert.deepEqual(app.state().sealed, before.sealed);
    assert.equal(app.state().writes.length, before.writes.length);
    assert.equal(app.state().draft, "other composer"); assert.match(app.state().error, /64 KiB UTF-8/);
    app.done();
  }
});

test("mutation admission validates byte-bounded target and the complete captured request before insertion", async () => {
  validateMutationPayload(7, "9", "é".repeat(64), "edit", "ok");
  const invalid = [
    { ...edit, messageId: "é".repeat(65) }, { ...edit, token: "bad" },
    { ...edit, expectedContext: "bad" }, { ...edit, target: "bad" },
    { ...edit, channel: String(1n << 128n) }, { ...edit, server: -1 },
    { ...edit, text: "  " }, { ...edit, replyTo: "other" },
    { ...edit, messageId: "\ud800" }, { ...edit, messageId: "\udc00" },
    { ...edit, kind: "delete", text: "must be empty" },
  ];
  for (const intent of invalid) assert.throws(() => addPendingSend({}, intent as PendingSend));
  const app = pendingManagerApp({}); await app.hydrate();
  for (const id of ["é".repeat(65), "\ud800", "\udc00"]) {
    await assert.rejects(app.mutation(7, "9", id, "delete", ""), /Invalid message change/);
  }
  assert.equal(app.state().contextCalls, 0); assert.deepEqual(app.state().pendingSends, {});
  app.done();
});

test("actual editor rejects lone UTF-16 surrogates before queue/context while retaining exact input", async () => {
  for (const text of ["\ud800", "\udc00", "before\ud800after", "😀\udc00"]) {
    const app = pendingManagerApp({}); await app.hydrate(); app.conversation(7, "9");
    const row = { id: "message", text: "old" }; app.startEdit(row); app.edit(text);
    const before = app.state(); await app.saveEdit(row);
    assert.equal(app.state().editDraft, text); assert.equal(app.state().editingId, "message");
    assert.equal(app.state().contextCalls, 0); assert.equal(app.state().submissions.length, 0);
    assert.deepEqual(app.state().pendingSends, {}); assert.deepEqual(app.state().sealed, before.sealed);
    assert.equal(app.state().writes.length, before.writes.length); assert.match(app.state().error, /incomplete Unicode/);
    // The failed admission must not poison later continuity either.
    assert.equal(await app.saveImmediately(), true); app.done();
  }
});

test("old oversized edit hydrates losslessly but no retry, direct submit or Stop can dispatch it", async () => {
  for (const resolution of [undefined, "recover", "cancel"] as const) {
    const { resolution: _priorResolution, ...base } = oversized;
    const original = { ...base, ...(resolution ? { resolution } : {}) };
    assert.equal(quarantinedPendingMutation(original), true);
    assert.deepEqual(sanitizePendingSends(collection(original), 2), collection(original));
    const app = pendingManagerApp({ version: 2, pendingSends: collection(original), drafts: { "7:9": "ordinary draft" } });
    await app.hydrate(); await app.retry(true); await app.retry();
    await assert.rejects(app.submit(token), /Open Pending messages to recover/);
    await assert.rejects(app.resolve(token, "cancel"), /This request will stay paused/);
    assert.equal(app.state().submissions.length, 0); assert.equal(app.state().stops.length, 0);
    assert.deepEqual(app.state().pendingSends, collection(original));
    assert.equal(await app.saveImmediately(), true, "quarantine must not block normal continuity saves");
    app.lock(); await app.unlock(); await app.retry(true);
    assert.deepEqual(app.state().pendingSends, collection(original));
    app.conversation(7, "9"); app.type("new ordinary message"); app.answer(async () => durable); await app.send();
    assert.equal(app.state().submissions.length, 1, "ordinary chat remains usable");
    assert.equal(app.state().submissions[0].command, "send_message");
    assert.deepEqual(app.state().pendingSends, collection(original));
    await app.resolve(token, "recover");
    assert.deepEqual(app.state().pendingSends, {});
    assert.deepEqual(app.state().recoveredSendDrafts, collection(original));
    assert.deepEqual(app.state().sealed.recoveredSendDrafts, collection(original));
    app.lock(); await app.unlock(); await app.retry(true);
    assert.deepEqual(app.state().recoveredSendDrafts, collection(original));
    assert.equal(app.state().submissions.length, 1); assert.equal(app.state().stops.length, 0);
    assert.equal(await app.saveImmediately(), true);
    app.done();
  }
});

test("quarantine retains other invalid custody refusal and the existing aggregate storage bound", () => {
  for (const bad of [{ ...oversized, target: "bad" }, { ...oversized, expectedContext: "bad" },
    { ...oversized, messageId: "" }, { ...oversized, text: "x".repeat(256 * 1024) }]) {
    assert.throws(() => sanitizePendingSends(collection(bad), 2), /sealed copy has been kept/);
  }
  assert.throws(() => sanitizePendingSends(collection(oversized), 1), /version 2/);
});

test("old byte-oversized target IDs hydrate and recover exact edit/delete requests without authoring or Stop", async () => {
  const cases = ["é".repeat(65), "x".repeat(150)].flatMap(messageId =>
    (["edit", "delete"] as const).map(kind => ({ messageId, kind })));
  for (const { messageId, kind } of cases) {
    for (const resolution of [undefined, "recover", "cancel"] as const) {
      const original: PendingSend = { ...edit, messageId, kind,
        text: kind === "edit" ? "valid old edit text" : "", replyTo: "",
        acceptance: "ambiguous", retryBlock: "invalid", ...(resolution ? { resolution } : {}) } as PendingSend;
      assert.throws(() => addPendingSend({}, original), /Invalid message change/);
      assert.deepEqual(sanitizePendingSends(collection(original), 2), collection(original), "old sealable payload remains readable");
      const app = pendingManagerApp({ version: 2, pendingSends: collection(original) });
      await app.hydrate(); app.lock(); await app.unlock();
      await app.retry(true); await app.retry(); await assert.rejects(app.submit(token), /cannot be retried/);
      await assert.rejects(app.resolve(token, "cancel"), /This request will stay paused/);
      assert.equal(app.state().stops.length, 0); assert.equal(app.state().submissions.length, 0);
      assert.equal(await app.saveImmediately(), true);
      const before = app.state().sealed; app.failSaves(true);
      await assert.rejects(app.resolve(token, "recover"), /decision was not completed/);
      assert.deepEqual(app.state().pendingSends, collection(original));
      assert.deepEqual(app.state().recoveredSendDrafts, {}); assert.deepEqual(app.state().sealed, before);
      app.failSaves(false); await app.resolve(token, "recover");
      app.lock(); await app.unlock(); await app.retry(true);
      assert.deepEqual(app.state().pendingSends, {});
      assert.deepEqual(app.state().recoveredSendDrafts, collection(original));
      assert.deepEqual(app.state().sealed.recoveredSendDrafts, collection(original));
      assert.equal(app.state().stops.length, 0); assert.equal(app.state().submissions.length, 0);
      app.conversation(7, "9");
      if (kind === "delete") await assert.rejects(app.use(token), /Open a conversation/);
      app.type("ordinary new message"); app.answer(async () => durable); await app.send();
      assert.equal(app.state().submissions.length, 1);
      assert.equal(app.state().submissions[0].command, "send_message");
      assert.deepEqual(app.state().recoveredSendDrafts, collection(original));
      app.done();
    }
  }
});

test("saved-text recovery commits both shelves only after sealing and rolls both back on failure", async () => {
  const saved = { ...edit, token: "2".repeat(32) };
  const app = pendingManagerApp({ version: 2, pendingSends: collection(oversized), recoveredSendDrafts: collection(saved) });
  await app.hydrate(); const before = app.state();
  const gate = deferred(); app.holdSave(gate.promise);
  const recovering = app.resolve(token, "recover");
  const rejected = assert.rejects(recovering, /decision was not completed/); await settle();
  assert.deepEqual(app.state().pendingSends, before.pendingSends);
  assert.deepEqual(app.state().recoveredSendDrafts, before.recoveredSendDrafts);
  app.failSaves(true); gate.resolve(); await rejected;
  assert.deepEqual(app.state().pendingSends, before.pendingSends);
  assert.deepEqual(app.state().recoveredSendDrafts, before.recoveredSendDrafts);
  assert.deepEqual(app.state().sealed, before.sealed);
  app.failSaves(false); await app.resolve(token, "recover"); app.lock(); await app.unlock();
  assert.deepEqual(app.state().recoveredSendDrafts, { ...collection(saved), ...collection(oversized) });
  assert.deepEqual(app.state().pendingSends, {}); assert.equal(app.state().stops.length, 0);
  app.done();
});

test("all permanent mutation refusals pause across reload without erasing prior ambiguity or original Stop identity", async () => {
  const reasons = { CHAT_MUTATION_TARGET_CHANGED: "context_changed", CHAT_MUTATION_AUTHOR_CONFLICT: "conflict",
    CHAT_MUTATION_NOT_AUTHORIZED: "invalid", CHAT_MUTATION_INVALID: "invalid" };
  for (const [code, block] of Object.entries(reasons)) {
    for (const original of [edit, { ...edit, kind: "delete" as const, text: "" as const }]) {
      assert.equal(pendingSendRetryBlock(`${code}: refusal`), block);
      assert.equal(sendRefusedUnaccepted(`${code}: refusal`), false, "retry policy cannot prove historical absence");
      const app = pendingManagerApp({ version: 2, pendingSends: collection({ ...original, acceptance: "ambiguous" }) });
      await app.hydrate(); app.answer(async () => { throw new Error(`${code}: refusal`); });
      await app.retry(true);
      const first = app.state().submissions[0].args;
      assert.equal(app.state().sealed.pendingSends[token].retryBlock, block);
      assert.equal(app.state().sealed.pendingSends[token].acceptance, "ambiguous");
      app.lock(); await app.unlock(); await app.retry(true); await app.retry(true);
      assert.equal(app.state().submissions.length, 1);
      await app.retry(); assert.equal(app.state().submissions.length, 2);
      assert.deepEqual(app.state().submissions[1].args, first);
      assert.equal(app.state().pendingSends[token].acceptance, "ambiguous");
      await app.resolve(token, "cancel");
      assert.deepEqual(app.state().stops[0].args, { server: 7, original: durableStopOriginal(original) });
      assert.deepEqual(app.state().pendingSends, {});
      app.done();
    }
  }
});

test("a full recovered shelf refuses local recovery without releasing quarantine custody", async () => {
  const recovered = Object.fromEntries(Array.from({ length: MAX_PENDING_SENDS }, (_, n) => {
    const item = { ...edit, token: (n + 2).toString(16).padStart(32, "0") };
    return [item.token, item];
  }));
  const app = pendingManagerApp({ version: 2, pendingSends: collection(oversized), recoveredSendDrafts: recovered });
  await app.hydrate(); const before = app.state().sealed;
  await assert.rejects(app.resolve(token, "recover"), /Pending messages need attention/);
  assert.deepEqual(app.state().pendingSends, collection(oversized));
  assert.deepEqual(app.state().recoveredSendDrafts, recovered); assert.deepEqual(app.state().sealed, before);
  assert.equal(app.state().stops.length, 0); app.done();
});
