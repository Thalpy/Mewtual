import assert from "node:assert/strict";
import test from "node:test";
import { StudioSession } from "./studio-session.ts";
import { CHANNEL, ME, SERVER, fakeIpc, id32, id64, indexContent, indexEntry, ordinaryView, previewView, settle } from "./studio-testkit.ts";

const object = id32(42);
function scoreContent() { return { kind: "score", title: null, bpm: null, bpb: null, steps: null, cells: {}, patches: {}, readiness: { cells: {}, patches: {}, headerConflicts: false } }; }
async function harness(preview = false) {
  const ipc = fakeIpc(), timers: (() => void)[] = []; let id = 100;
  const session = new StudioSession({ ipc, me: ME, now: () => 0, newId: () => id32(++id),
    schedule: (fn) => { timers.push(fn); return () => { const index = timers.indexOf(fn); if (index >= 0) timers.splice(index, 1); }; } });
  ipc.on("studio_list", () => ordinaryView(indexContent({ objects: { [object]: indexEntry({ kind: "score", title: "score" }) } })));
  ipc.on("studio_read", () => preview ? previewView(scoreContent()) : ordinaryView(scoreContent()));
  await session.attach(); session.setScope({ server: SERVER, channel: CHANNEL });
  const sync = async () => { for (let pass = 0; pass < 3; pass++) { for (const fn of timers.splice(0)) fn(); await settle(3); } };
  await sync();
  return { ipc, session, sync, calls: (command: string) => ipc.calls.filter((call) => call.cmd === command) };
}

test("Score Index target opens through the same session with retained objectKind on exact Apply retry", async () => {
  const { ipc, session, sync, calls } = await harness();
  session.open(object); await sync();
  assert.equal(calls("studio_read")[0].args.objectKind, "score");
  assert.equal(session.doc?.objectKind, "score"); assert.equal(session.doc?.model, null);
  assert.equal(session.doc?.score?.bpm, 120);
  assert.equal(calls("studio_overlay_read").find((call) => call.args.object === object)?.args.objectKind, "score");
  let attempts = 0;
  ipc.on("studio_apply", () => { if (++attempts === 1) throw new Error("delivery lost"); return ordinaryView(scoreContent()); });
  session.setScoreCell(object, 0, 0, { k: "d", pad: 9 }); await sync();
  assert.equal(session.saves[0]?.status, "uncertain");
  session.retry(session.saves[0].id); await sync();
  assert.deepEqual(calls("studio_apply")[0].args, calls("studio_apply")[1].args);
  assert.equal(calls("studio_apply")[0].args.objectKind, "score");
  assert.deepEqual(JSON.parse(String(calls("studio_apply")[0].args.body)), { op: "set_cell", step: 0, row: 0, value: { k: "d", pad: 9 } });
  assert.equal(session.saves.length, 0);
  assert.throws(() => session.insertFrame(object, null, new Uint8Array([1])), /Flipnote/);
  assert.equal(calls("publish_pix").length, 0);
});

test("Score creation reuses coordinated native Create without inventing a first frame", async () => {
  const { ipc, session, sync, calls } = await harness();
  ipc.on("studio_create", () => ordinaryView(scoreContent()));
  const id = session.createScore("new score"); session.open(id, { read: false }); await sync();
  assert.equal(calls("studio_create")[0].args.objectKind, "score");
  assert.equal(session.doc?.view?.content.kind, "score");
  assert.equal(session.doc?.score?.steps, 16);
  assert.equal(calls("publish_pix").length, 0);
});

test("preview Score Save stays local and preserves kind/ticket/nonce/body on exact retry", async () => {
  const { ipc, session, sync, calls } = await harness(true);
  session.open(object); await sync();
  ipc.on("studio_unconfirmed_overlay_save_begin", () => ({ v: 1, kind: "unconfirmedOverlaySaveTicket", channel: CHANNEL,
    object, basis: id64(1), branch: id64(2), provenance: "unconfirmed" }));
  let attempts = 0;
  ipc.on("studio_unconfirmed_overlay_save", () => ++attempts === 1
    ? { v: 1, kind: "unconfirmedOverlaySave", channel: CHANNEL, object, state: "pending", retry: "sameRequest" }
    : { v: 1, kind: "unconfirmedOverlaySave", channel: CHANNEL, object, state: "saved", basis: id64(1), accepted: 1, localOnly: true, provisional: true, provenance: "unconfirmed" });
  session.clearScoreCell(object, 0, 0); await sync();
  assert.equal(session.saves[0]?.kind, "overlayApply");
  assert.equal(session.saves[0]?.status, "uncertain");
  session.retry(session.saves[0].id); await sync();
  assert.deepEqual(calls("studio_unconfirmed_overlay_save")[0].args, calls("studio_unconfirmed_overlay_save")[1].args);
  assert.equal(calls("studio_unconfirmed_overlay_save")[0].args.objectKind, "score");
  assert.equal(calls("studio_unconfirmed_overlay_save_begin").length, 1);
  assert.equal(calls("studio_apply").length, 0);
});

test("Score recovery choices put recipes first and keep explicit removals Copy-only", async () => {
  const { session } = await harness();
  const raw = scoreContent() as unknown as import("./studio-native.ts").ScoreContent;
  const source = (n: number) => ({ opId: id64(n), author: ME, nonce: id32(n) });
  raw.cells["0:0"] = { removed: true, register: { selected: { value: null, source: source(1) }, conflicts: [] } };
  raw.patches[id64(2)] = { removed: true, register: { selected: { value: null, source: source(3) }, conflicts: [] } };
  const version = { v: 1 as const, kind: "recoveryVersion" as const, historical: true as const, channel: CHANNEL,
    version: { snapshot: id64(4), epoch: "0", staged: false, bytes: 1, reason: "rewound" as const }, content: raw };
  assert.deepEqual(session.choicesFor(version, "restore"), []);
  assert.deepEqual(session.choicesFor(version, "copy").map((choice) => choice.choice), [
    { kind: "scorePatch", patch: id64(2), value: id64(3) }, { kind: "scoreCell", step: 0, row: 0, value: id64(1) },
  ]);
});
