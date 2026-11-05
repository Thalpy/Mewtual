import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
function production(start: string, end: string): string {
  const from = source.indexOf(start), to = source.indexOf(end, from);
  assert.ok(from >= 0 && to > from, `actual App handler exists: ${start}`);
  return ts.transpile(source.slice(from, to), { target: ts.ScriptTarget.ES2022 });
}
function deferred() {
  let resolve!: (value: unknown) => void;
  const promise = new Promise<unknown>((done) => { resolve = done; });
  return { promise, resolve };
}
/** Exercise App's actual closures and view invalidation; only IPC and rendering are replaced. */
function app() {
  return new Function(`
    let activeServerId = 1, uiStateLoadGeneration = 10, viewGeneration = 20, chatLedgerMount = 0;
    let locked = false, uiStateReady = true, windowCloseInFlight = false, canModerate = true;
    let view = "moderation", moderationTab = "history";
    const untrack = (fn) => fn(), calls = [], replies = [];
    const invoke = (command, args) => { calls.push({command, args}); return replies.shift() ?? Promise.resolve({status:"page"}); };
    ${production("  function beginViewSwitch()", "  // Is the captured context")}
    ${production("  function chatLedgerReads(", "  async function warnModerationSelection()")}
    return {
      reads: () => chatLedgerReads(activeServerId), calls,
      hold: promise => replies.push(promise),
      switchServer(id) { beginViewSwitch(); activeServerId = id; },
      lock() { locked = true; uiStateLoadGeneration++; },
      unlock() { locked = false; uiStateLoadGeneration++; },
      loading(value) { uiStateReady = !value; },
      closing() { windowCloseInFlight = true; },
      demote() { canModerate = false; },
      activity() { moderationTab = "activity"; },
      scope: () => ({viewGeneration, chatLedgerMount}),
    };
  `)();
}

test("actual App history reads never retarget an old server, including switch away and back", async () => {
  const current = app(), pending = deferred(), old = current.reads();
  current.hold(pending.promise);
  const read = old.page(null);
  assert.deepEqual(current.calls[0], { command: "get_admin_chat_ledger", args: {server: 1, cursor: null, limit: 32} });
  current.switchServer(2); current.switchServer(1);
  pending.resolve({status: "page", private: "old scope"});
  assert.deepEqual(await read, {status: "unavailable", code: "session-unavailable"});
  assert.deepEqual(await old.detail("old-row", "old-revision", 0), {status: "unavailable", code: "session-unavailable"});
  assert.equal(current.calls.length, 1);
  await current.reads().page(null);
  assert.equal(current.calls.length, 2);
  assert.deepEqual(current.scope(), { viewGeneration: 22, chatLedgerMount: 2 });
});

test("actual App detail continuations and unpolled callbacks stay retired after lock/unlock", async () => {
  const current = app(), pending = deferred(), old = current.reads();
  current.hold(pending.promise);
  const read = old.detail("exact-row", "exact-revision", 65536);
  assert.deepEqual(current.calls[0], {command: "get_admin_chat_ledger_detail",
    args: {server: 1, rowId: "exact-row", expectedRevision: "exact-revision", frameOffset: 65536}});
  current.lock(); current.unlock();
  pending.resolve({status: "detail", private: "previous session"});
  assert.deepEqual(await read, {status: "unavailable", code: "session-unavailable"});
  assert.deepEqual(await old.page(null), {status: "unavailable", code: "session-unavailable"});
  assert.equal(current.calls.length, 1);
  await current.reads().page(null);
  assert.equal(current.calls.length, 2);
});

test("actual App rejects positive reads when moderation access, pane, hydration or close is lost", async () => {
  for (const retire of ["demote", "activity", "closing"] as const) {
    const current = app(), pending = deferred(), reads = current.reads();
    current.hold(pending.promise);
    const read = reads.page(null);
    current[retire](); pending.resolve({status: "page", private: "must disappear"});
    assert.deepEqual(await read, {status: "unavailable", code: "session-unavailable"});
    await reads.page(null); assert.equal(current.calls.length, 1);
  }
  const current = app(), reads = current.reads(); current.loading(true);
  await reads.page(null); assert.equal(current.calls.length, 0);
});

test("actual history component is keyed to reactive scope/role and cannot mount during hydration", () => {
  const from = source.indexOf('{#key `${activeServerId}:${chatLedgerMount}:${myRole}`}');
  assert.ok(from > 0);
  assert.match(source.slice(from, from + 500), /chatLedgerReads\(activeServerId\)[\s\S]*<ChatAdminLedger/);
  const pane = source.slice(source.lastIndexOf('{#if moderationTab === "history"}', from), from);
  assert.match(pane, /uiStateReady && !windowCloseInFlight && activeServerId !== null/);
  assert.match(source, /let chatLedgerMount = \$state\(0\)/);
});
