import assert from "node:assert/strict";
import test from "node:test";
import { decodePix } from "./pix.ts";
import { DEFAULT_PALETTE, StudioError, StudioStore, demoStudio, sceneFrame } from "./studio-store.ts";
import { CLAIM_TTL_MS, DOC_TYPE_STUDIO_INDEX, DOC_TYPE_STUDIO_OBJECT, MAX_DOMAIN_OP_BYTES } from "./studio-contract.ts";

const ME = "1".repeat(64);
function fresh(caps?: Partial<{ frames: number; frameBytes: number; sfx: number; objects: number }>, t = 1_000_000) {
  let seq = 0;
  let now = t;
  const store = new StudioStore({ serverId: "ab".repeat(16), channel: "art", me: ME, now: () => now, caps, newId: () => (++seq).toString(16).padStart(32, "0") });
  return { store, tick: (ms: number) => { now += ms; } };
}

test("logical names carry the backend's document tags and the stable keys", () => {
  const { store } = fresh();
  const id = store.createFlipnote("moon cat");
  assert.deepEqual(store.indexDocument(), { serverId: "ab".repeat(16), docType: DOC_TYPE_STUDIO_INDEX, logicalKey: "art" });
  assert.deepEqual(store.objectDocument(id), { serverId: "ab".repeat(16), docType: DOC_TYPE_STUDIO_OBJECT, logicalKey: id });
  assert.equal(store.index.objects[id].kind, "flipnote");
  assert.equal(store.root(id).fps, 12);
});

test("every edit is one envelope with a canonical JSON body under the op cap", () => {
  const { store } = fresh();
  const id = store.createFlipnote("t");
  const before = store.ops.length;
  const f = store.insertFrame(id, null, sceneFrame("A"));
  assert.equal(store.ops.length, before + 1);
  const env = store.ops[store.ops.length - 1];
  assert.equal(env.docType, DOC_TYPE_STUDIO_OBJECT);
  assert.equal(env.logicalKey, id);
  assert.match(env.nonce, /^[0-9a-f]{32}$/);
  assert.ok(env.body.length <= MAX_DOMAIN_OP_BYTES);
  const body = JSON.parse(env.body);
  assert.equal(body.op, "insert_frame");
  assert.equal(body.frame, f);
  // canonical: keys sorted, no whitespace
  assert.equal(env.body, JSON.stringify(body, Object.keys(body).sort()));
});

test("insert lands after its predecessor, or at the end when the predecessor is gone", () => {
  const { store } = fresh();
  const id = store.createFlipnote("t");
  const a = store.insertFrame(id, null, sceneFrame("A"));
  const b = store.insertFrame(id, a, sceneFrame("B"));
  const c = store.insertFrame(id, a, sceneFrame("A")); // concurrent-style: also after a
  assert.deepEqual(store.root(id).frames, [a, c, b]);
  store.apply(id, { op: "insert_frame", frame: "f".repeat(32), after: "missing".padEnd(32, "0"), cid: "c".repeat(64), bytes: 10 });
  assert.equal(store.root(id).frames[3], "f".repeat(32));
});

test("a tombstone wins over any later insertion of the same id, and drops its sfx and claim", () => {
  const { store } = fresh();
  const id = store.createFlipnote("t");
  const a = store.insertFrame(id, null, sceneFrame("A"));
  store.apply(id, { op: "set_sfx", sfx: "s".repeat(32), frame: a, patch: "meow", note: 60 });
  store.claim(a, ME);
  store.apply(id, { op: "remove_frame", frame: a });
  assert.deepEqual(store.root(id).frames, []);
  assert.deepEqual(store.root(id).sfx, {});
  assert.equal(store.claimOn(a), null);
  store.apply(id, { op: "insert_frame", frame: a, after: null, cid: "c".repeat(64), bytes: 10 });
  assert.deepEqual(store.root(id).frames, [], "the tombstone still wins");
});

test("replace produces an op only when the bytes changed, and the record moves with them", () => {
  const { store } = fresh();
  const id = store.createFlipnote("t");
  const a = store.insertFrame(id, null, sceneFrame("A"));
  const n = store.ops.length;
  assert.equal(store.replaceFrame(id, a, sceneFrame("A")), false);
  assert.equal(store.ops.length, n);
  assert.equal(store.replaceFrame(id, a, sceneFrame("B")), true);
  assert.equal(store.ops.length, n + 1);
  const bytes = store.frameBytes(id, a)!;
  assert.deepEqual(decodePix(bytes).palette, DEFAULT_PALETTE);
});

test("caps: the frame list, the byte promise and fps are refused before anything changes", () => {
  const { store } = fresh({ frames: 2 });
  const id = store.createFlipnote("t");
  const a = store.insertFrame(id, null, sceneFrame("A"));
  store.insertFrame(id, a, sceneFrame("B"));
  assert.throws(() => store.insertFrame(id, null, sceneFrame("A")), (e: unknown) => e instanceof StudioError && e.reason === "document full");
  assert.equal(store.root(id).frames.length, 2);
  assert.throws(() => store.apply(id, { op: "set_header", field: "fps", value: 25 }), /fps range/);
  assert.throws(() => store.apply(id, { op: "set_header", field: "fps", value: 0 }), /fps range/);
  store.apply(id, { op: "set_header", field: "fps", value: 24 });
  assert.equal(store.root(id).fps, 24);
});

test("over the byte promise, later frames are flagged in list order and editing is refused", () => {
  // Room for exactly two scene frames; the third tips the promise.
  const { store } = fresh({ frameBytes: sceneFrame("A").length + sceneFrame("B").length + 16 });
  const id = store.createFlipnote("t");
  const a = store.insertFrame(id, null, sceneFrame("A"));
  const b = store.insertFrame(id, a, sceneFrame("B"));
  const c = store.insertFrame(id, b, sceneFrame("A"));
  const over = store.overCap(store.root(id));
  assert.deepEqual([...over], [c]);
  assert.throws(() => store.insertFrame(id, c, sceneFrame("B")), /document full/);
  // removing the over-cap frame reopens the document
  store.apply(id, { op: "remove_frame", frame: c });
  assert.equal(store.overCap(store.root(id)).size, 0);
  store.insertFrame(id, b, sceneFrame("B"));
});

test("claims live 90 s from the last receipt, renew for the holder, and pass to the asker", () => {
  const { store, tick } = fresh();
  const id = store.createFlipnote("t");
  const a = store.insertFrame(id, null, sceneFrame("A"));
  store.claim(a, ME);
  assert.equal(store.claimSecondsLeft(a), 90);
  tick(CLAIM_TTL_MS - 1000);
  assert.equal(store.claimSecondsLeft(a), 1);
  store.renewClaim(a);
  assert.equal(store.claimSecondsLeft(a), 90);
  tick(CLAIM_TTL_MS + 1);
  assert.equal(store.claimOn(a), null, "expired claims vanish");
  store.claim(a, "2".repeat(64));
  store.renewClaim(a); // not ours: no effect
  tick(10_000);
  assert.equal(store.claimSecondsLeft(a), 80);
  store.passClaim(a, "3".repeat(64));
  assert.equal(store.claimOn(a)?.by, "3".repeat(64));
});

test("the demo studio is self-consistent: every listed frame has a record, fetching frames hold no bytes, the outbox starts empty", () => {
  const { store, moonCat, rooksLoop, lullaby } = demoStudio({ me: ME, rook: "2".repeat(64), mika: "3".repeat(64), wren: "4".repeat(64), owner: "5".repeat(64) }, () => 1_000_000_000);
  const root = store.root(moonCat);
  assert.equal(root.frames.length, 24);
  for (const f of root.frames) assert.ok(root.frame[f], "record for every frame");
  const fetching = root.frames.filter((f) => store.frameState.get(f) === "fetching");
  assert.equal(fetching.length, 2);
  for (const f of fetching) assert.equal(store.frameBytes(moonCat, f), undefined);
  assert.equal(root.frames.filter((f) => store.frameBytes(moonCat, f)).length, 22);
  assert.equal(store.claimOn(root.frames[11])?.ask, true);
  assert.equal(store.claimOn(root.frames[14])?.by, "2".repeat(64));
  assert.ok(store.conflicts.has(root.frames[8]));
  assert.equal(Object.keys(root.sfx).length, 3);
  assert.equal(root.score, lullaby);
  assert.equal(store.settlement.get(rooksLoop)?.label, "rotating");
  assert.ok(store.recovery.get(rooksLoop)?.staged);
  assert.equal(store.overCap(root).size, 0);
  assert.equal(store.ops.length, 0);
  assert.equal(store.index.objects[lullaby].kind, "score");
  // the scene decodes: a full frame of the default palette
  const img = decodePix(store.frameBytes(moonCat, root.frames[0])!);
  assert.equal(img.w, 192);
  assert.equal(img.h, 144);
  assert.ok(img.pixels.some((p) => p === 8), "the pink nose is a literal entry");
});
