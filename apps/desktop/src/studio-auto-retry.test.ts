// [desync fixes]: a failed Studio save resolves on its own, leaving never waits for a save, and
// playback never asks about save status. These run the production session (`autosaveMs` 5000) on a
// fake clock: `tick(ms)` fires every timer due inside the window, in order, including ones scheduled
// while it runs.
import assert from "node:assert/strict";
import test from "node:test";
import { StudioSession } from "./studio-session.ts";
import { decodeStudioDraft } from "./studio-draft.ts";
import { decodePixa } from "./pixa.ts";
import { CHANNEL, ME, SERVER, b64, fakeIpc, flipnoteContent, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, previewView, settle } from "./studio-testkit.ts";

const score = id32(42), flip = id32(43), frame = id32(44), group = "deadbeef";
/** The exact refusal from the field report: Tauri's ACL turning the batch command away. */
const ACL_REFUSAL = "studio_apply_batch not allowed. Command not found";

function scoreContent(cells: Record<string, unknown> = {}) {
  return { kind: "score", title: null, bpm: null, bpb: null, steps: null, cells, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } };
}

async function harness(opts: { autosaveMs?: number } = {}) {
  const ipc = fakeIpc(), timers = new Set<{ due: number; fn: () => void }>();
  let now = 0, seq = 100;
  ipc.on("studio_list", () => ({ ...ordinaryView(indexContent({ objects: { [score]: indexEntry({ kind: "score", title: "song" }), [flip]: indexEntry({ title: "cat" }) } })), group }));
  ipc.on("studio_read", (args) => args.objectKind === "score" ? { ...ordinaryView(scoreContent()), group } : { ...ordinaryView(flipnoteContent({ frames: [] })), group });
  ipc.on("studio_apply", (args) => args.objectKind === "score" ? { ...ordinaryView(scoreContent()), group } : { ...ordinaryView(flipnoteContent({ frames: [] })), group });
  const session = new StudioSession({ ipc, me: ME, now: () => now, newId: () => id32(++seq), autosaveMs: opts.autosaveMs ?? 5000,
    schedule: (fn, ms) => { const timer = { due: now + ms, fn }; timers.add(timer); return () => { timers.delete(timer); }; } });
  session.setScope({ server: SERVER, channel: CHANNEL }); session.flush(); await settle(8);
  session.open(score); await settle(8);
  const tick = async (ms: number) => {
    const end = now + ms;
    for (;;) {
      const next = [...timers].filter(t => t.due <= end).sort((a, b) => a.due - b.due)[0];
      if (!next) break;
      timers.delete(next); now = next.due; next.fn(); await settle(8);
    }
    now = end; await settle(8);
  };
  const calls = (cmd: string) => ipc.calls.filter(c => c.cmd === cmd);
  return { ipc, session, tick, calls, now: () => now };
}

// The field report: several notes placed quickly leave as one batch, Tauri refuses the batch
// command, and every edit sat "save uncertain" until each was retried by hand, one card at a time.
test("a batch refused as a whole resolves on its own: each edit is resent alone, in order, and lands", async () => {
  const h = await harness();
  let hold: (() => void) | null = null;
  h.ipc.on("studio_apply", () => new Promise(resolve => { hold = () => resolve({ ...ordinaryView(scoreContent()), group }); }));
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000); // the deadline sends the first note
  assert.equal(h.calls("studio_apply").length, 1);
  for (let step = 1; step <= 4; step++) h.session.setScoreCell(score, step, 0, { k: "d", pad: 1 });
  await h.tick(5000); // their deadline releases them behind the one still out
  h.ipc.on("studio_apply_batch", () => { throw ACL_REFUSAL; });
  h.ipc.on("studio_apply", () => ({ ...ordinaryView(scoreContent()), group }));
  hold!(); await settle(8);
  assert.equal(h.calls("studio_apply_batch").length, 1, "the four queued notes left as one batch");
  const batch = (h.calls("studio_apply_batch")[0].args as { operations: { nonce: string; body: string }[] }).operations;
  assert.deepEqual(h.session.saveTally(), { left: 4, retrying: 4, stuck: 0, discarded: 0 }, "refused after dispatch, so uncertain, and each is due to be resent");
  for (const s of h.session.saves) assert.ok(h.session.autoRetryAt(s.id) !== null);
  await h.tick(1000);
  assert.equal(h.session.saves.length, 0, "every note landed without anyone pressing retry");
  assert.equal(h.calls("studio_apply_batch").length, 1, "the refused batch was never sent again");
  const resent = h.calls("studio_apply").slice(1).map(c => ({ nonce: c.args.nonce, body: c.args.body }));
  assert.deepEqual(resent, batch, "each item went alone, with its own exact request, in lane order");
});

test("an uncertain Save is resent on its own with its exact request after a pause, and lands", async () => {
  const h = await harness(); let applies = 0;
  h.ipc.on("studio_apply", () => { if (++applies === 1) throw new Error("receipt lost"); return { ...ordinaryView(scoreContent()), group }; });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  const [failed] = h.session.saves;
  assert.equal(failed.status, "uncertain");
  assert.equal(h.session.autoRetryAt(failed.id), h.now() + 1000, "the first pause is one second");
  await h.tick(999); assert.equal(applies, 1, "not before the pause ends");
  await h.tick(1); assert.equal(applies, 2); assert.equal(h.session.saves.length, 0);
  const [first, second] = h.calls("studio_apply").map(c => c.args);
  assert.deepEqual(second, first, "the same nonce, epoch and body: native answers a held operation as saved");
});

test("automatic resends back off, then stop and leave the edit to the member", async () => {
  const h = await harness();
  h.ipc.on("studio_apply", () => { throw new Error("receipt lost"); });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  const [failed] = h.session.saves, gaps: number[] = [];
  for (let at = h.session.autoRetryAt(failed.id); at !== null; at = h.session.autoRetryAt(failed.id)) {
    gaps.push(at - h.now()); await h.tick(at - h.now());
  }
  assert.deepEqual(gaps, [1000, 2000, 4000, 8000, 15_000, 30_000, 60_000], "growing pauses, about two minutes in all");
  assert.equal(h.calls("studio_apply").length, 8, "the first send and seven resends");
  assert.deepEqual(h.session.saveTally(), { left: 1, retrying: 0, stuck: 1, discarded: 0 }, "now the member's to retry or discard");
  await h.tick(10 * 60_000); assert.equal(h.calls("studio_apply").length, 8, "nothing more is sent on its own");
  h.session.retry(failed.id); await settle(8);
  assert.equal(h.calls("studio_apply").length, 9, "the member's retry still sends it");
});

test("a later edit waits behind the one being resent, and both land in order", async () => {
  const h = await harness(); let applies = 0;
  h.ipc.on("studio_apply", () => { if (++applies === 1) throw new Error("receipt lost"); return { ...ordinaryView(scoreContent()), group }; });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  h.session.setScoreCell(score, 1, 0, { k: "d", pad: 2 }); h.session.sendBeforeLeaving(); await settle(8);
  assert.deepEqual(h.session.saves.map(s => s.status), ["uncertain", "queued"], "released, the later edit still holds behind the failed one");
  assert.equal(h.calls("studio_apply").length, 1);
  await h.tick(1000);
  assert.equal(h.session.saves.length, 0);
  const steps = h.calls("studio_apply").map(c => JSON.parse(String(c.args.body)).step);
  assert.deepEqual(steps, [0, 0, 1], "the resend went before the later edit");
});

test("Retry now replaces the resend that was due: one send, not two", async () => {
  const h = await harness(); let applies = 0;
  h.ipc.on("studio_apply", () => { if (++applies === 1) throw new Error("receipt lost"); return { ...ordinaryView(scoreContent()), group }; });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  h.session.retry(h.session.saves[0].id); await settle(8);
  assert.equal(applies, 2); assert.equal(h.session.saves.length, 0);
  await h.tick(60_000); assert.equal(applies, 2, "the cancelled resend never fires");
});

test("a lock or scope reset cancels a due resend", async () => {
  const h = await harness();
  h.ipc.on("studio_apply", () => { throw new Error("receipt lost"); });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  h.session.reset();
  await h.tick(10 * 60_000);
  assert.equal(h.calls("studio_apply").length, 1, "nothing was sent after the reset");
});

test("restored work and the immediate test seam are never resent on their own", async () => {
  const h = await harness();
  h.ipc.on("studio_apply", () => { throw new Error("receipt lost"); });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 });
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  draft.records[0].status = "inflight"; draft.records[0].attempts = 1; // interrupted by a lock
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  const [restored] = h.session.saves;
  assert.equal(restored.status, "uncertain");
  assert.equal(h.session.autoRetryAt(restored.id), null, "only an explicit Save or retry sends restored work");
  await h.tick(10 * 60_000); assert.equal(h.calls("studio_apply").length, 0);
  assert.deepEqual(h.session.saveTally(), { left: 1, retrying: 0, stuck: 1, discarded: 0 });

  const immediate = await harness({ autosaveMs: 0 });
  immediate.ipc.on("studio_apply", () => { throw new Error("receipt lost"); });
  immediate.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await settle(8);
  assert.equal(immediate.session.saves[0].status, "uncertain");
  assert.equal(immediate.session.autoRetryAt(immediate.session.saves[0].id), null);
});

// A fresh head may batch the queued edits behind it. A member that may already have been delivered
// (here: restored after an interrupted delivery, then retried) travels alone instead, so a batch
// that failed once can never be rebuilt around it.
test("an edit that may already have been delivered never rides a fresh edit's batch", async () => {
  const h = await harness();
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); h.session.setScoreCell(score, 1, 0, { k: "d", pad: 1 });
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  draft.records[1].status = "inflight"; draft.records[1].attempts = 1;
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  h.session.open(score); await settle(8);
  const [fresh, interrupted] = h.session.saves;
  assert.deepEqual([fresh.status, interrupted.status], ["queued", "uncertain"]);
  h.ipc.on("studio_apply_batch", () => assert.fail("no batch may carry the possibly delivered edit"));
  h.session.retry(interrupted.id);
  await h.session.save(); await settle(8);
  assert.equal(h.calls("studio_apply").length, 2, "both went alone"); assert.equal(h.session.saves.length, 0);
});

// Owner decision 2026-10-20: a Score edit the history moved under is saved again on its own, as a
// fresh edit in the current epoch. (Review M1 before it: an exact retry of such a refusal skipped
// admission, sent the stale request, and could then never be saved as a new edit; `retry` still
// refuses it.)
test("a Score edit the history moved under is saved again into the current epoch on its own", async () => {
  const h = await harness();
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 });
  const original = h.session.saves[0].kind === "apply" ? h.session.saves[0].request.nonce : "";
  h.ipc.on("studio_read", () => ({ ...ordinaryView(scoreContent(), { epochId: id32(0xe2) }), group }));
  await h.session.refreshDoc(score); // the document rotated before the deadline
  h.session.retry(h.session.saves[0].id); // not failed yet: a retry of a queued edit does nothing
  await h.tick(5000);
  const sent = h.calls("studio_apply");
  assert.equal(sent.length, 1, "the stale request was never sent; the re-saved edit was");
  assert.equal(sent[0].args.epochId, id32(0xe2), "the new edit names the current epoch");
  assert.notEqual(sent[0].args.nonce, original, "a fresh edit, not a replay");
  assert.equal(h.session.saves.length, 0, "no card was left for the member");
});

// Design review H4: the composer now places every note as a layered `set_note`, so the re-save
// must cover it or the owner's decision silently stops applying to placements. A layered clear
// stays the member's, like `clear_cell`.
test("a layered note the history moved under is re-saved on its own; a layered clear stays the member's", async () => {
  const h = await harness();
  h.session.applyScoreOp(score, { op: "set_note", step: 0, row: 0, value: { k: "d", pad: 1 } });
  h.ipc.on("studio_read", () => ({ ...ordinaryView(scoreContent(), { epochId: id32(0xe2) }), group }));
  await h.session.refreshDoc(score);
  await h.tick(5000);
  const sent = h.calls("studio_apply");
  assert.equal(sent.length, 1); assert.equal(sent[0].args.epochId, id32(0xe2));
  assert.equal(JSON.parse(String(sent[0].args.body)).op, "set_note");
  assert.equal(h.session.saves.length, 0, "no card was left for the member");
  const clear = await harness();
  clear.session.applyScoreOp(score, { op: "clear_note", step: 0, row: 0, layer: "d" });
  clear.ipc.on("studio_read", () => ({ ...ordinaryView(scoreContent(), { epochId: id32(0xe2) }), group }));
  await clear.session.refreshDoc(score);
  await clear.tick(5000);
  assert.equal(clear.calls("studio_apply").length, 0, "a clear is never re-authored unseen");
  assert.equal(clear.session.saves.length, 1); assert.equal(clear.session.needsDecision(clear.session.saves[0].id), true);
});

test("after native's retired-epoch refusal, the Score edit is re-read and re-saved on its own", async () => {
  const h = await harness(); let applies = 0, epoch = id32(0xe1);
  h.ipc.on("studio_read", () => ({ ...ordinaryView(scoreContent(), { epochId: epoch }), group }));
  h.ipc.on("studio_apply", () => {
    if (++applies === 1) { epoch = id32(0xe2); throw "epoch studio: edit belongs to a retired epoch"; } // native rotated first
    return { ...ordinaryView(scoreContent(), { epochId: epoch }), group };
  });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  const [stale, fresh] = h.calls("studio_apply").map(c => c.args);
  assert.equal(stale.epochId, id32(0xe1)); assert.equal(fresh?.epochId, id32(0xe2), "the document was read again first");
  assert.equal(h.session.saves.length, 0);
});

test("a history that keeps moving is re-saved a bounded number of times, then left to the member", async () => {
  const h = await harness(); let epoch = 0xe1;
  h.ipc.on("studio_read", () => ({ ...ordinaryView(scoreContent(), { epochId: id32(epoch) }), group }));
  h.ipc.on("studio_apply", () => { epoch++; throw "epoch studio: edit belongs to a retired epoch"; });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000); await h.tick(10 * 60_000);
  assert.equal(h.calls("studio_apply").length, 4, "the first send and three re-saves");
  const [left] = h.session.saves;
  assert.equal(left.status, "blocked"); assert.equal(h.session.canReauthor(left.id), true, "the member can still save it as a new edit");
});

test("edits queued behind a re-saved one are re-saved too, in lane order, each once", async () => {
  const h = await harness();
  for (let step = 0; step < 3; step++) h.session.setScoreCell(score, step, 0, { k: "d", pad: 1 });
  const stale = new Set(h.session.saves.map(s => s.kind === "apply" ? s.request.nonce : ""));
  const current = () => ({ ...ordinaryView(scoreContent(), { epochId: id32(0xe2) }), group });
  h.ipc.on("studio_read", current); h.ipc.on("studio_apply", current); h.ipc.on("studio_apply_batch", current);
  await h.session.refreshDoc(score);
  await h.tick(5000);
  const sent = [...h.calls("studio_apply"), ...h.calls("studio_apply_batch")].flatMap(c => c.cmd === "studio_apply"
    ? [{ epochId: c.args.epochId, nonce: c.args.nonce, body: c.args.body }]
    : (c.args.operations as { nonce: string; body: string }[]).map(o => ({ epochId: c.args.epochId, ...o })));
  assert.deepEqual(sent.map(o => JSON.parse(String(o.body)).step), [0, 1, 2], "in lane order");
  assert.ok(sent.every(o => o.epochId === id32(0xe2) && !stale.has(String(o.nonce))), "every one a fresh edit in the current epoch");
  assert.equal(h.session.saves.length, 0);
  assert.equal(h.session.saveTally().discarded, 0, "a re-save is never counted as a discard");
});

test("a batch refused at its first item for a retired epoch is re-saved after one shared read", async () => {
  const h = await harness(); let hold: (() => void) | null = null, epoch = id32(0xe1);
  h.ipc.on("studio_read", () => ({ ...ordinaryView(scoreContent(), { epochId: epoch }), group }));
  h.ipc.on("studio_apply", () => new Promise(resolve => { hold = () => resolve({ ...ordinaryView(scoreContent(), { epochId: epoch }), group }); }));
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  for (let step = 1; step <= 3; step++) h.session.setScoreCell(score, step, 0, { k: "d", pad: 1 });
  await h.tick(5000);
  let batches = 0;
  h.ipc.on("studio_apply_batch", (args) => {
    if (++batches === 1) { epoch = id32(0xe2); throw `Studio: batched Apply item 1 of 3 was refused (epoch studio: edit belongs to a retired epoch); the items before it are saved, so retry the same batch exactly or discard that item`; }
    assert.equal(args.epochId, id32(0xe2)); return { ...ordinaryView(scoreContent(), { epochId: epoch }), group };
  });
  const reads = h.calls("studio_read").length;
  hold!(); await settle(8); await h.tick(0);
  assert.equal(h.calls("studio_read").length - reads, 1, "one read for the whole refused batch");
  assert.equal(batches, 2, "the three re-saved edits left together again");
  assert.equal(h.session.saves.length, 0);
});

test("only what is proven unsent is re-saved: a later batch item, a clear, and a closing history stay with the member", async () => {
  // A later batch item may follow saved items, so it is uncertain and never becomes a new edit.
  const later = await harness(); let hold: (() => void) | null = null;
  later.ipc.on("studio_apply", () => new Promise(resolve => { hold = () => resolve({ ...ordinaryView(scoreContent()), group }); }));
  later.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await later.tick(5000);
  later.session.setScoreCell(score, 1, 0, { k: "d", pad: 1 }); later.session.setScoreCell(score, 2, 0, { k: "d", pad: 1 });
  await later.tick(5000);
  later.ipc.on("studio_apply_batch", () => { throw "Studio: batched Apply item 2 of 2 was refused (epoch studio: edit belongs to a retired epoch); the items before it are saved, so retry the same batch exactly or discard that item"; });
  hold!(); await later.tick(10 * 60_000);
  assert.deepEqual(later.session.saves.map(s => s.status), ["uncertain", "uncertain"]);
  assert.equal(later.calls("studio_apply_batch").length, 1, "nothing re-saved");

  // A clear re-authored on newer history could remove what a collaborator added there.
  const clear = await harness();
  clear.session.clearScoreCell(score, 0, 0);
  clear.ipc.on("studio_read", () => ({ ...ordinaryView(scoreContent(), { epochId: id32(0xe2) }), group }));
  await clear.session.refreshDoc(score); await clear.tick(5000); await clear.tick(60_000);
  assert.equal(clear.calls("studio_apply").length, 0); assert.equal(clear.session.canReauthor(clear.session.saves[0].id), true);

  // A rotation in progress: the document's route is not ordinary, so the card stays.
  const closing = await harness();
  closing.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 });
  closing.ipc.on("studio_read", () => ({ ...ordinaryView(scoreContent(), { phase: "closing" }), group }));
  await closing.session.refreshDoc(score); await closing.tick(5000); await closing.tick(60_000);
  assert.equal(closing.calls("studio_apply").length, 0);
  assert.equal(closing.session.saves[0].status, "blocked"); assert.match(closing.session.saves[0].error, /Shared history changed/);
});

test("a reset while a re-save waits for its read re-saves nothing", async () => {
  const h = await harness(); let release: (() => void) | null = null, refused = false;
  h.ipc.on("studio_apply", () => { refused = true; throw "epoch studio: edit belongs to a retired epoch"; });
  h.ipc.on("studio_read", () => refused ? new Promise(resolve => { release = () => resolve({ ...ordinaryView(scoreContent(), { epochId: id32(0xe2) }), group }); })
    : { ...ordinaryView(scoreContent()), group });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  const [pending] = h.session.saves;
  assert.equal(h.session.resaving(pending.id), true, "the re-save waits for its read");
  assert.deepEqual(h.session.saveTally(), { left: 1, retrying: 1, stuck: 0, discarded: 0 }, "and counts as moving, not as needing the member");
  assert.equal(h.session.needsDecision(pending.id), false);
  h.session.reset();
  release!(); await h.tick(60_000);
  assert.equal(h.calls("studio_apply").length, 1, "nothing was sent after the reset");
});

test("a flipnote edit the history moved under stays the member's to save again", async () => {
  const h = await harness();
  h.session.open(flip); await settle(8);
  h.session.setFps(flip, 7);
  h.ipc.on("studio_read", () => ({ ...ordinaryView(flipnoteContent({ frames: [] }), { epochId: id32(0xe2) }), group }));
  await h.session.refreshDoc(flip);
  await h.tick(5000); await h.tick(10 * 60_000);
  const [refused] = h.session.saves;
  assert.match(refused.error, /Shared history changed/);
  assert.deepEqual([h.session.canRetry(refused.id), h.session.canReauthor(refused.id)], [false, true]);
  assert.equal(h.calls("studio_apply").length, 0, "nothing was sent on its own");
});

test("a refusal no resend can change is the member's at once: gone member or channel, existing id", async () => {
  // These may follow a write, so they stay uncertain and cannot become a new edit. (A Score edit in a
  // retired epoch is re-saved on its own instead: see the bounded re-save test above.)
  for (const [refusal, reauthor] of [["Studio: Studio requires current membership", false],
    ["Studio: unknown Studio channel", false], ["Studio: object already exists; use Apply to edit it", false]] as const) {
    const h = await harness();
    h.ipc.on("studio_apply", () => { throw refusal; });
    h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
    const [failed] = h.session.saves;
    assert.equal(h.session.autoRetryAt(failed.id), null, refusal);
    assert.equal(h.session.canRetry(failed.id), false, `${refusal}: Retry is not offered`);
    assert.equal(h.session.canReauthor(failed.id), reauthor, `${refusal}: save as a new edit`);
    assert.deepEqual(h.session.saveTally(), { left: 1, retrying: 0, stuck: 1, discarded: 0 }, refusal);
    await h.tick(10 * 60_000); assert.equal(h.calls("studio_apply").length, 1, `${refusal}: sent once, never again`);
    h.session.discard(failed.id); assert.equal(h.session.saveTally().discarded, 1);
  }
});

test("an Index Apply and an ordinary frame are resent exactly, and land", async () => {
  const h = await harness(), pixels = pixBytes(5);
  const listed = () => ({ ...ordinaryView(indexContent({ objects: { [score]: indexEntry({ kind: "score", title: "song" }), [flip]: indexEntry({ title: "cat" }) } })), group });
  let index = 0, frames = 0;
  h.ipc.on("studio_apply_index", () => { if (++index === 1) throw new Error("receipt lost"); return listed(); });
  h.session.setEntryExpiry(score, { kind: "never" }); await h.tick(5000);
  assert.equal(h.session.saves[0].status, "uncertain");
  await h.tick(1000);
  assert.equal(h.session.saves.length, 0);
  const [a, b] = h.calls("studio_apply_index").map(c => c.args);
  assert.deepEqual(b, a);

  h.ipc.on("studio_read", (args) => args.objectKind === "score" ? { ...ordinaryView(scoreContent()), group }
    : { ...ordinaryView(flipnoteContent({ frames: [{ id: frame, cid: id64(9), bytes: pixels.length }] })), group });
  h.ipc.on("publish_pix", () => ({ cid: id64(10), bytes: pixels.length }));
  h.ipc.on("studio_apply", () => { if (++frames === 1) throw new Error("receipt lost"); return { ...ordinaryView(flipnoteContent({ frames: [{ id: frame, cid: id64(10), bytes: pixels.length }] })), group }; });
  h.session.open(flip); await settle(8);
  h.session.saveFrame(flip, frame, pixels); await h.tick(5000);
  assert.equal(h.session.saves[0].status, "uncertain");
  await h.tick(1000);
  assert.equal(h.session.saves.length, 0);
  assert.equal(h.calls("publish_pix").length, 1, "the published pixels are reused");
  const [first, second] = h.calls("studio_apply").map(c => c.args);
  assert.deepEqual(second, first, "the same frame Apply, nonce and all");
});

test("a local-draft save is never resent on its own: its retry identity is a ticket the member reconciles", async () => {
  const h = await harness();
  h.ipc.on("studio_read", () => ({ ...previewView(scoreContent()), group }));
  await h.session.refreshDoc(score);
  h.ipc.on("studio_unconfirmed_overlay_save_begin", a => ({ v: 1, kind: "unconfirmedOverlaySaveTicket", channel: a.channel, object: a.object ?? null, basis: id64(1), branch: id64(2), provenance: "unconfirmed" }));
  h.ipc.on("studio_unconfirmed_overlay_save", () => { throw new Error("receipt lost"); });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  const [failed] = h.session.saves;
  assert.equal(failed.kind, "overlayApply"); assert.equal(failed.status, "uncertain");
  assert.equal(h.session.autoRetryAt(failed.id), null);
  await h.tick(10 * 60_000); assert.equal(h.calls("studio_unconfirmed_overlay_save").length, 1);
});

// Review M1 (layers): a composer move is [write the note at its new place, clear its old place].
// Native refused the write for good (its sound was removed meanwhile) and the clear waited behind
// it. Discarding the write released the clear, which then deleted the note it was moving.
test("discarding a gesture's refused write takes that gesture's unsent clear with it, so the note stays", async () => {
  const h = await harness();
  const note = { k: "d" as const, pad: 1 };
  h.ipc.on("studio_apply_batch", () => { throw "Studio: batched Apply item 1 of 2 was refused (cell out of range); the items before it are saved, so retry the same batch exactly or discard that item"; });
  h.ipc.on("studio_apply", (args) => { if (JSON.parse(String(args.body)).op === "set_note") throw "Studio: cell out of range"; return { ...ordinaryView(scoreContent()), group }; });
  h.session.applyScoreOp(score, { op: "set_note", step: 4, row: 0, value: note }, 7);
  h.session.applyScoreOp(score, { op: "clear_cell", step: 2, row: 0 }, 7);
  // An unrelated clear of another gesture stays queued behind it and is saved after.
  h.session.applyScoreOp(score, { op: "clear_cell", step: 9, row: 0 }, 8);
  await h.tick(5000); await settle(8); await h.tick(1000);
  const ops = () => h.session.saves.map((s) => s.kind === "apply" ? s.op.op : s.kind);
  assert.deepEqual(ops(), ["set_note", "clear_cell", "clear_cell"], "the clears wait behind the refused write");
  h.session.discard(h.session.saves[0].id); await settle(8); await h.tick(1000);
  const sent = h.calls("studio_apply").map((c) => JSON.parse(String(c.args.body)) as { op: string; step: number });
  assert.equal(sent.some((op) => op.op === "clear_cell" && op.step === 2), false, "the moved note's old place was never cleared");
  assert.ok(sent.some((op) => op.op === "clear_cell" && op.step === 9), "another gesture's clear is saved as usual");
  assert.equal(h.session.saves.length, 0);
});

// Re-review H1 (a): with two clears behind the refused write, discarding them one by one pumped
// between them, and the second clear started before it could be dropped.
test("discarding a refused write drops every clear of its gesture at once: none is sent", async () => {
  const h = await harness();
  h.ipc.on("studio_apply_batch", () => { throw "Studio: batched Apply item 1 of 3 was refused (cell out of range); the items before it are saved, so retry the same batch exactly or discard that item"; });
  h.ipc.on("studio_apply", (args) => { if (JSON.parse(String(args.body)).op === "set_note") throw "Studio: cell out of range"; return { ...ordinaryView(scoreContent()), group }; });
  h.session.applyScoreOp(score, { op: "set_note", step: 4, row: 0, value: { k: "d", pad: 1 } }, 11);
  h.session.applyScoreOp(score, { op: "clear_cell", step: 2, row: 0 }, 11);
  h.session.applyScoreOp(score, { op: "clear_cell", step: 3, row: 0 }, 11);
  await h.tick(5000); await settle(8); await h.tick(60_000); // automatic resends make the clears queued again
  assert.deepEqual(h.session.saves.map((s) => s.kind === "apply" ? s.op.op : s.kind), ["set_note", "clear_cell", "clear_cell"]);
  assert.equal(h.session.discardCount(h.session.saves[0].id), 3, "the confirmation can say three edits go");
  assert.equal(h.session.discardCount(h.session.saves[1].id), 1, "a clear goes alone");
  h.session.discard(h.session.saves[0].id); await settle(8); await h.tick(60_000);
  const sent = h.calls("studio_apply").map((c) => JSON.parse(String(c.args.body)) as { op: string });
  assert.equal(sent.some((op) => op.op === "clear_cell"), false, "no clear of the gesture was sent");
  assert.equal(h.session.saves.length, 0);
});

// Re-review H1 (b): a slide right is [4 <- B, 3 <- A, clear 2]. With 4 <- B refused for good (the
// score was shortened meanwhile) and discarded, 3 <- A would overwrite B, which then exists nowhere.
test("discarding a refused write also drops the gesture's later writes, so no carried note is overwritten", async () => {
  const h = await harness();
  const a = { k: "n" as const, p: "ab".repeat(32) }, b = { k: "n" as const, p: "ab".repeat(32), vol: 50 };
  h.ipc.on("studio_apply_batch", () => { throw "Studio: batched Apply item 1 of 3 was refused (note length outside score); the items before it are saved, so retry the same batch exactly or discard that item"; });
  h.ipc.on("studio_apply", (args) => { const op = JSON.parse(String(args.body)); if (op.op === "set_note" && op.step === 4) throw "Studio: note length outside score"; return { ...ordinaryView(scoreContent()), group }; });
  h.session.applyScoreOp(score, { op: "set_note", step: 4, row: 0, value: b }, 12);
  h.session.applyScoreOp(score, { op: "set_note", step: 3, row: 0, value: a }, 12);
  h.session.applyScoreOp(score, { op: "clear_note", step: 2, row: 0, layer: a.p }, 12);
  await h.tick(5000); await settle(8); await h.tick(60_000);
  h.session.discard(h.session.saves[0].id); await settle(8); await h.tick(60_000);
  const sent = h.calls("studio_apply").map((c) => JSON.parse(String(c.args.body)) as { op: string; step: number });
  assert.equal(sent.some((op) => op.op === "set_note" && op.step === 3), false, "B at 3 was never overwritten");
  assert.equal(sent.some((op) => op.op === "clear_note"), false);
  assert.equal(h.session.saves.length, 0);
});

// Re-review M2: gesture links are not part of the lock snapshot. Restored, a discarded write used
// to let a bulk Save run its clear, deleting the note the gesture was moving.
test("after a restore, discarding a note write holds the restored note edits behind it for the member", async () => {
  const h = await harness();
  h.session.applyScoreOp(score, { op: "set_note", step: 4, row: 0, value: { k: "d", pad: 1 } }, 21);
  h.session.applyScoreOp(score, { op: "clear_cell", step: 2, row: 0 }, 21);
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  h.session.open(score); await settle(8);
  const [write, clear] = h.session.saves;
  h.session.discard(write.id); await settle(8);
  await h.session.save().catch(() => {}); await h.tick(60_000);
  assert.equal(h.calls("studio_apply").length + h.calls("studio_apply_batch").length, 0, "a bulk Save sends nothing the member did not choose");
  assert.equal(h.session.saves.length, 1);
  assert.equal(h.session.saves[0].id, clear.id); assert.equal(h.session.saves[0].status, "blocked");
  assert.match(h.session.saves[0].error, /earlier edit was discarded/);
  h.session.retry(clear.id); await settle(8); await h.tick(1000);
  assert.equal(h.calls("studio_apply").length, 1, "the member's own retry sends it");
});

// Third review M-B: "Save as a new edit" on a restored write replaced the record without its
// restored mark, so discarding the replacement let a bulk Save run the restored clear.
test("a restored write saved as a new edit and then discarded still holds the restored clear behind it", async () => {
  const h = await harness();
  h.session.applyScoreOp(score, { op: "set_note", step: 4, row: 0, value: { k: "d", pad: 1 } }, 31);
  h.session.applyScoreOp(score, { op: "clear_cell", step: 2, row: 0 }, 31);
  const draft = JSON.parse(h.session.draftSnapshot()!);
  Object.assign(draft.records[0], { status: "blocked", attempts: 1, error: "Studio: cell out of range" }); // refused, proven not written
  h.session.reset(); h.session.restoreDraft(decodeStudioDraft(JSON.stringify(draft))); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  h.session.open(score); await settle(8);
  h.ipc.on("studio_apply", (args) => { if (JSON.parse(String(args.body)).op === "set_note") throw "Studio: cell out of range"; return { ...ordinaryView(scoreContent()), group }; });
  h.session.reauthor(h.session.saves[0].id); await settle(8);
  const replacement = h.session.saves[0];
  assert.equal(h.calls("studio_apply").length, 1, "the new edit was sent and refused again");
  h.session.discard(replacement.id); await settle(8);
  await h.session.save().catch(() => {}); await h.tick(60_000);
  assert.equal(h.calls("studio_apply").some((c) => JSON.parse(String(c.args.body)).op === "clear_cell"), false, "the restored clear never ran");
  assert.equal(h.session.saves.length, 1); assert.equal(h.session.saves[0].status, "blocked");
  assert.match(h.session.saves[0].error, /earlier edit was discarded/);
});

// Third review L1: a restored clear that had already failed kept its automatic resend, which fired
// a second after the member discarded the write before it.
test("discarding a restored write cancels the automatic resend of a failed restored clear behind it", async () => {
  const h = await harness();
  h.session.applyScoreOp(score, { op: "set_note", step: 4, row: 0, value: { k: "d", pad: 1 } }, 32);
  h.session.applyScoreOp(score, { op: "clear_cell", step: 2, row: 0 }, 32);
  const draft = decodeStudioDraft(h.session.draftSnapshot()!);
  h.session.reset(); h.session.restoreDraft(draft); h.session.setScope({ server: SERVER, channel: CHANNEL }); h.session.flush(); await settle(8);
  h.session.open(score); await settle(8);
  h.ipc.on("studio_apply_batch", () => { throw "receipt lost"; });
  await h.session.save().catch(() => {}); await settle(8);
  assert.deepEqual(h.session.saves.map((s) => s.status), ["uncertain", "uncertain"]);
  assert.ok(h.session.autoRetryAt(h.session.saves[1].id) !== null, "the clear had a resend due");
  h.ipc.on("studio_apply", () => ({ ...ordinaryView(scoreContent()), group }));
  h.session.discard(h.session.saves[0].id); await settle(8); await h.tick(10 * 60_000);
  assert.equal(h.calls("studio_apply").length, 0, "nothing was resent on its own");
  assert.equal(h.session.autoRetryAt(h.session.saves[0].id), null);
  assert.match(h.session.saves[0].error, /earlier edit was discarded/);
});

test("a batch refused at one item: the others land on their own, the refused one stays and holds its lane", async () => {
  const h = await harness();
  let hold: (() => void) | null = null;
  h.ipc.on("studio_apply", () => new Promise(resolve => { hold = () => resolve({ ...ordinaryView(scoreContent()), group }); }));
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  for (let step = 1; step <= 3; step++) h.session.setScoreCell(score, step, 0, { k: "d", pad: 1 });
  await h.tick(5000);
  h.ipc.on("studio_apply_batch", () => { throw "Studio: batched Apply item 2 of 3 was refused (cell out of range); the items before it are saved, so retry the same batch exactly or discard that item"; });
  // Alone, the bad item (step 2) is refused the same way; every other item is accepted.
  h.ipc.on("studio_apply", (args) => { if (JSON.parse(String(args.body)).step === 2) throw "Studio: cell out of range"; return { ...ordinaryView(scoreContent()), group }; });
  hold!(); await settle(8);
  await h.tick(1000);
  const steps = (s: { kind: string }) => s.kind === "apply" ? JSON.parse(String((s as { request: { body: string } }).request.body)).step : null;
  assert.deepEqual(h.session.saves.map(s => [steps(s), s.status]), [[2, "uncertain"], [3, "queued"]], "step 1 landed; step 3 waits behind the refused step 2");
  const bad = h.session.saves[0];
  h.session.discard(bad.id); await settle(8);
  assert.equal(h.session.saves.length, 0, "with the refused item discarded, the rest of its lane lands");
});

test("leaving hands every edit to the background Save at once; nothing unhanded is left", async () => {
  const h = await harness();
  h.ipc.on("studio_apply", () => new Promise(() => {})); // no receipt ever arrives
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); h.session.setScoreHeader(score, "bpm", 90);
  assert.equal(h.session.hasUnsentWork(), true, "both wait for their deadline");
  h.ipc.on("studio_apply_batch", () => new Promise(() => {}));
  h.session.sendBeforeLeaving(); await settle(8);
  assert.equal(h.session.hasUnsentWork(), false, "everything is the background Save's now");
  assert.equal(h.calls("studio_apply_batch").length, 1, "released together, the two Score edits left as one batch");
  assert.deepEqual(h.session.saves.map(s => s.status), ["inflight", "inflight"]);
  assert.equal(h.session.retainsWork, true, "the edits are still held until they land");
});

test("Play is never refused by a failed save; a take export still is", async () => {
  const h = await harness();
  h.ipc.on("studio_apply", () => { throw new Error("receipt lost"); });
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 }); await h.tick(5000);
  assert.equal(h.session.saves[0].status, "uncertain");
  const playback = await h.session.prepareScorePlayback(score);
  assert.ok(playback.take, "the working score plays, saved or not");
  await assert.rejects(h.session.prepareScoreTake(score), /did not save/, "handing the take to someone else still waits for its saves");
});

test("a flipnote plays a linked score's unsaved edits as its editor showed them", async () => {
  const h = await harness(), pixels = pixBytes(3);
  h.ipc.on("studio_read", (args) => args.objectKind === "score" ? { ...ordinaryView(scoreContent()), group }
    : { ...ordinaryView(flipnoteContent({ frames: [{ id: frame, cid: id64(9), bytes: pixels.length }] })), group });
  h.ipc.on("request_blob_bounded", () => ({ bytes: pixels.length, bytes_b64: b64(pixels) }));
  h.ipc.on("studio_apply", () => new Promise(() => {})); // nothing lands: every edit stays queued or in flight
  h.session.setScoreCell(score, 0, 0, { k: "d", pad: 1 });
  h.session.open(flip); await settle(8);
  h.session.linkScore(flip, score); await settle(8);
  await assert.rejects(h.session.prepareFlipnotePixa(flip), /still saving/, "an export waits for the music's saves");
  const pixa = decodePixa(await h.session.prepareFlipnotePixa(flip, { playback: true }));
  assert.equal(pixa.frames.length, 1);
  assert.ok(pixa.take.events.length > 0, "the unsaved drum hit is in the played take");
});
