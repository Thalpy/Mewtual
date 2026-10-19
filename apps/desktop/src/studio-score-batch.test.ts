// Batched Score saves: the queued edits of one document travel in one native transaction.
import assert from "node:assert/strict";
import test from "node:test";
import { StudioSession } from "./studio-session.ts";
import { STUDIO_APPLY_BATCH_MAX } from "./studio-native.ts";
import { CHANNEL, ME, SERVER, deferred, fakeIpc, id32, indexContent, indexEntry, ordinaryView, settle } from "./studio-testkit.ts";

const object = id32(42);
function scoreContent(cells: Record<string, unknown> = {}) {
  return { kind: "score", title: null, bpm: null, bpb: null, steps: null, cells, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } };
}
async function harness() {
  const ipc = fakeIpc(), timers: (() => void)[] = []; let id = 100;
  const session = new StudioSession({ ipc, me: ME, now: () => 0, newId: () => id32(++id),
    schedule: (fn) => { timers.push(fn); return () => { const index = timers.indexOf(fn); if (index >= 0) timers.splice(index, 1); }; } });
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [object]: indexEntry({ kind: "score", title: "score" }) } })));
  ipc.on("studio_read", () => ordinaryView(scoreContent()));
  await session.attach(); session.setScope({ server: SERVER, channel: CHANNEL });
  const sync = async () => { for (let pass = 0; pass < 3; pass++) { for (const fn of timers.splice(0)) fn(); await settle(3); } };
  await sync(); session.open(object); await sync();
  return { ipc, session, sync, calls: (command: string) => ipc.calls.filter((call) => call.cmd === command) };
}

test("edits queued behind an in-flight save travel as one batch, in order, each with its own nonce", async () => {
  const h = await harness();
  const first = deferred<unknown>();
  h.ipc.on("studio_apply", () => first.promise);
  h.session.setScoreCell(object, 0, 0, { k: "d", pad: 1 }); await settle(3);
  // Three more while the first is out: they wait in the lane, then leave together.
  h.session.setScoreCell(object, 1, 0, { k: "d", pad: 2 });
  h.session.setScoreCell(object, 2, 0, { k: "d", pad: 3 });
  h.session.setScoreHeader(object, "bpm", 90);
  assert.equal(h.session.saves.filter((s) => s.status === "queued").length, 3);
  let batched: Record<string, unknown> | null = null;
  h.ipc.on("studio_apply_batch", (args) => { batched = args; return ordinaryView(scoreContent({ "0:0": { removed: false, register: { selected: { value: { k: "d", pad: 1 }, source: { opId: "1".repeat(64), author: ME, nonce: id32(1) } }, conflicts: [] } } })); });
  first.resolve(ordinaryView(scoreContent())); await h.sync();
  assert.equal(h.calls("studio_apply").length, 1, "the lone first edit went as a single apply");
  assert.equal(h.calls("studio_apply_batch").length, 1);
  assert.ok(batched);
  const sent = batched as { object: string; epochId: string; objectKind: string; operations: { nonce: string; body: string }[] };
  assert.equal(sent.object, object); assert.equal(sent.objectKind, "score");
  assert.deepEqual(sent.operations.map((o) => JSON.parse(o.body).op), ["set_cell", "set_cell", "set_header"]);
  assert.deepEqual(sent.operations.map((o) => JSON.parse(o.body).pad ?? JSON.parse(o.body).value), [{ k: "d", pad: 2 }, { k: "d", pad: 3 }, 90]);
  assert.equal(new Set(sent.operations.map((o) => o.nonce)).size, 3, "every item keeps its own retry identity");
  assert.equal(h.session.saves.length, 0, "the whole batch landed");
  assert.equal(h.session.doc?.score?.cells["0:0"]?.k, "d", "the returned view replaces the projection");
});

test("a move's clear and set travel in one batch in order, and a clear between two sets of one cell is never collapsed", async () => {
  const h = await harness();
  const first = deferred<unknown>();
  h.ipc.on("studio_apply", () => first.promise);
  h.session.setScoreCell(object, 9, 9, { k: "d", pad: 1 }); await settle(3);
  // The composer's move: clear the source, set the destination; then a set, a clear and a set of
  // one cell, which the tail-collapse rule must leave as three records because the clear is not a
  // collapsible slot.
  h.session.clearScoreCell(object, 2, 5);
  h.session.setScoreCell(object, 6, 8, { k: "n", p: "a".repeat(64), len: 3 });
  h.session.setScoreCell(object, 0, 0, { k: "d", pad: 1 });
  h.session.clearScoreCell(object, 0, 0);
  h.session.setScoreCell(object, 0, 0, { k: "d", pad: 2 });
  assert.equal(h.session.saves.filter((s) => s.status === "queued").length, 5);
  h.ipc.on("studio_apply_batch", () => ordinaryView(scoreContent()));
  first.resolve(ordinaryView(scoreContent())); await h.sync();
  const sent = (h.calls("studio_apply_batch")[0].args as { operations: { body: string }[] }).operations.map((o) => JSON.parse(o.body));
  assert.deepEqual(sent.map((op) => [op.op, op.step, op.row]), [["clear_cell", 2, 5], ["set_cell", 6, 8], ["set_cell", 0, 0], ["clear_cell", 0, 0], ["set_cell", 0, 0]]);
  assert.equal(sent[4].value.pad, 2);
  assert.equal(h.session.saves.length, 0);
});

test("a refused batch settles every member the same way and an exact retry resends the same items", async () => {
  const h = await harness();
  const first = deferred<unknown>();
  h.ipc.on("studio_apply", () => first.promise);
  h.session.setScoreCell(object, 0, 0, { k: "d", pad: 1 }); await settle(3);
  h.session.setScoreCell(object, 1, 0, { k: "d", pad: 2 });
  h.session.setScoreCell(object, 2, 0, { k: "d", pad: 3 });
  h.ipc.on("studio_apply_batch", () => { throw new Error("Studio: batched Apply item 2 of 2 was refused (edit belongs to a retired epoch); the items before it are saved, so retry the same batch exactly or discard that item"); });
  first.resolve(ordinaryView(scoreContent())); await h.sync();
  const members = h.session.saves;
  assert.equal(members.length, 2);
  assert.ok(members.every((s) => s.status === "uncertain" && /item 2 of 2/.test(s.error)), "native may have written the items before the one it names");
  const refused = (h.calls("studio_apply_batch")[0].args as { operations: { nonce: string; body: string }[] }).operations;
  const before = h.ipc.calls.length;
  h.ipc.on("studio_apply_batch", () => ordinaryView(scoreContent()));
  h.ipc.on("studio_apply", () => ordinaryView(scoreContent()));
  // Each member is retried by its own card. A retry starts the lane at once, so the first goes
  // alone and the second follows; whichever way they travel, the bytes are the refused ones.
  h.session.retry(members[0].id); h.session.retry(members[1].id); await h.sync();
  const resent = h.ipc.calls.slice(before).flatMap((c) => c.cmd === "studio_apply" ? [{ nonce: c.args.nonce, body: c.args.body }]
    : c.cmd === "studio_apply_batch" ? (c.args as { operations: { nonce: string; body: string }[] }).operations : []);
  assert.deepEqual(resent, refused, "the retries are the identical operations, nonce and body, in order");
  assert.equal(h.session.saves.length, 0);
});

test("a retried record travels alone, so a member behind it still meets admission against the current view", async () => {
  // Autosave mode is where admission (the authoring binding against the current view) applies.
  const ipc = fakeIpc(), timers: (() => void)[] = []; let id = 100; let epoch = 1;
  const view = () => ({ ...ordinaryView(scoreContent()), epochId: id32(0xe0 + epoch), epoch: String(epoch) });
  const session = new StudioSession({ ipc, me: ME, now: () => 0, newId: () => id32(++id), autosaveMs: 5000,
    schedule: (fn) => { timers.push(fn); return () => { const index = timers.indexOf(fn); if (index >= 0) timers.splice(index, 1); }; } });
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [object]: indexEntry({ kind: "score", title: "score" }) } })));
  ipc.on("studio_read", view);
  await session.attach(); session.setScope({ server: SERVER, channel: CHANNEL });
  const sync = async () => { for (let pass = 0; pass < 3; pass++) { for (const fn of timers.splice(0)) fn(); await settle(3); } };
  await sync(); session.open(object); await sync();
  ipc.on("studio_apply", () => { throw new Error("outcome=uncertain; the Save may have completed; retry exactly"); });
  session.setScoreCell(object, 0, 0, { k: "d", pad: 1 });
  await session.save().catch(() => {}); await sync();
  const first = session.saves[0];
  assert.equal(first.status, "uncertain");
  // A second edit, released for sending, waits behind the uncertain one.
  session.setScoreCell(object, 1, 0, { k: "d", pad: 2 });
  await session.save().catch(() => {}); await sync();
  assert.equal(session.saves[1].status, "queued");
  // Shared history moves on before the member is ever sent.
  epoch = 2; await session.refreshDoc(object); await sync();
  ipc.on("studio_apply", view);
  session.retry(first.id); await sync();
  assert.equal(ipc.calls.filter((c) => c.cmd === "studio_apply_batch").length, 0, "the retry does not carry the member");
  assert.equal(session.saves.length, 1, "the exact retry landed on its own");
  const member = session.saves[0];
  assert.equal(member.status, "blocked");
  assert.match(member.error, /Shared history changed/);
  assert.equal(session.canReauthor(member.id), true);
});

test("a batch refused before execution is known not written, and the lane keeps its order and bounds", async () => {
  const h = await harness();
  const first = deferred<unknown>();
  h.ipc.on("studio_apply", () => first.promise);
  h.session.setScoreCell(object, 0, 0, { k: "d", pad: 1 }); await settle(3);
  for (let step = 1; step <= STUDIO_APPLY_BATCH_MAX + 2; step++) h.session.setScoreCell(object, step, 0, { k: "d", pad: 1 });
  const batches: number[] = [];
  h.ipc.on("studio_apply_batch", (args) => { batches.push((args as { operations: unknown[] }).operations.length); throw new Error("Studio storage busy; retry the same request"); });
  first.resolve(ordinaryView(scoreContent())); await h.sync(); await h.sync(); await h.sync();
  assert.equal(batches[0], STUDIO_APPLY_BATCH_MAX, "a batch never exceeds native's count bound");
  assert.ok(batches.every((n) => n === STUDIO_APPLY_BATCH_MAX), "the busy refusal was repeated with the same batch");
  const blocked = h.session.saves.filter((s) => s.status === "blocked");
  assert.equal(blocked.length, STUDIO_APPLY_BATCH_MAX, "every member is known not written");
  assert.equal(h.session.saves.filter((s) => s.status === "queued").length, 2, "the overflow waits behind the lane");
  assert.ok(blocked.every((s) => s.mayHaveCommitted === false));
});
