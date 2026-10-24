// The native adapter: every response shape the bridge documents parses into the typed read
// model without losing evidence, and every shape it does not document is refused rather than
// coerced. Command names and argument spellings are pinned here because the security audit only
// proves they are registered, not that they are called the way the backend expects.
import assert from "node:assert/strict";
import test from "node:test";
import {
  CHANNEL, ME, ROOK, SERVER, b64, fakeIpc, flipnoteContent, id32, id64, indexContent, indexEntry, ordinaryView, pixBytes, previewView, recoveryListing, version,
} from "./studio-testkit.ts";
import {
  StudioNativeError,
  base64ToBytes,
  bytesToBase64,
  expiryForBody,
  flipnoteModel,
  indexModel,
  parseFlipnoteView,
  parseIndexView,
  parseRecoveryApplied,
  parseRecoveryExport,
  parseRecoveryListing,
  parseRecoveryPreview,
  parseSettlementChanged,
  parseStudioUpdated,
  parseStudioView,
  publishPix,
  recoveryAcknowledge,
  recoveryApply,
  recoveryPreview,
  requestBlobBounded,
  studioApply,
  studioCreate,
  studioList,
  studioRead,
} from "./studio-native.ts";

const F1 = id32(0x11), F2 = id32(0x12), F3 = id32(0x13);

test("an ordinary flipnote view keeps conflicts, insertions, tombstones and over-cap evidence", () => {
  const raw = ordinaryView(flipnoteContent({
    frames: [
      { id: F1, cid: id64(1), bytes: 100 },
      { id: F2, cid: id64(2), bytes: 200, author: ROOK, conflicts: [{ cid: id64(3), bytes: 300, author: ME, op: 77 }] },
      { id: F3, cid: id64(4), bytes: 400, after: F2 },
    ],
    overCap: { [F3]: { count: false, bytes: true } },
    tombstones: { [id32(0x99)]: [{ opId: id64(9), author: ROOK, nonce: id32(9), ts: 5 }] },
  }), { epoch: "18446744073709551615" });
  const view = parseFlipnoteView(raw);
  assert.equal(view.awaitingTenureReceipt, false);
  assert.equal(view.phase, "open");
  assert.equal(view.publication, "local");
  assert.equal(view.channel, CHANNEL, "u128 channel stays the exact decimal string");
  assert.equal(view.epoch, "18446744073709551615", "u64 epoch stays lossless");
  const m = flipnoteModel(view.content);
  assert.deepEqual(m.frames.map((f) => f.id), [F1, F2, F3], "timeline order, not map order");
  assert.equal(m.frames[1].author, ROOK);
  assert.deepEqual(m.frames[1].conflicts.map((c) => [c.cid, c.author, c.opId]), [[id64(3), ME, id64(77)]]);
  assert.deepEqual(m.frames[2].overCap, { count: false, bytes: true });
  assert.equal(m.overCapCount, 1);
  assert.deepEqual(m.deletedFrames, [id32(0x99)]);
  assert.equal(view.content.frames[F3].insertions[0].value.after, F2);
  assert.equal(m.title, "moon cat");
  assert.equal(m.fps, 12);
});

test("unset title and fps registers fall back to the documented defaults without inventing a source", () => {
  const view = parseFlipnoteView(ordinaryView(flipnoteContent({ title: null, fps: null, frames: [] })));
  assert.equal(view.content.title, null);
  assert.equal(view.content.fps, null);
  const m = flipnoteModel(view.content);
  assert.equal(m.title, "");
  assert.equal(m.fps, 12);
  assert.deepEqual(m.titleConflicts, []);
});

test("awaitingTenureReceipt is the read-only discriminator; provisional alone is not", () => {
  const preview = parseStudioView(previewView(indexContent()));
  assert.equal(preview.awaitingTenureReceipt, true);
  assert.equal(preview.phase, null);
  assert.equal(preview.publication, null);
  const ordinary = parseStudioView(ordinaryView(indexContent()));
  assert.equal(ordinary.awaitingTenureReceipt, false);
  assert.equal(ordinary.provisional, true, "ordinary editable views are provisional too");
  // A preview that smuggles a phase, or an ordinary view without its publication claim, is refused.
  assert.throws(() => parseStudioView({ ...previewView(indexContent()), phase: "open" }), StudioNativeError);
  assert.throws(() => parseStudioView({ ...ordinaryView(indexContent()), publication: undefined }), StudioNativeError);
  assert.throws(() => parseStudioView({ ...ordinaryView(indexContent()), phase: "sealed" }), StudioNativeError);
  assert.throws(() => parseStudioView({ ...ordinaryView(indexContent()), provisional: false }), StudioNativeError);
});

test("an index view keeps overflow, deleted entries and tombstones apart, and expiry states distinct", () => {
  const raw = ordinaryView(indexContent({
    objects: { [id32(1)]: indexEntry({ title: "moon cat", ts: 10 }), [id32(2)]: indexEntry({ kind: "score", title: "lullaby", createdBy: ROOK, ts: 20, expiry: { kind: "never" } }) },
    overflow: { [id32(3)]: indexEntry({ title: "late", ts: 30, expiry: { kind: "at", ms: 0 } }) },
    deletedObjects: { [id32(4)]: indexEntry({ title: "gone", ts: 40 }) },
    tombstones: { [id32(4)]: [{ opId: id64(8), author: ROOK, nonce: id32(8) }] },
  }));
  const view = parseIndexView(raw);
  const m = indexModel(view.content);
  assert.deepEqual(m.entries.map((e) => [e.id, e.kind, e.title, e.where]), [[id32(1), "flipnote", "moon cat", "visible"], [id32(2), "score", "lullaby", "visible"]]);
  assert.deepEqual(m.overflow.map((e) => e.id), [id32(3)]);
  assert.deepEqual(m.deleted.map((e) => e.id), [id32(4)]);
  assert.deepEqual(m.entries[1].expiry, { kind: "never" });
  assert.deepEqual(m.overflow[0].expiry, { kind: "at", ms: 0 }, "zero milliseconds is a timestamp, not never");
  assert.equal(view.content.tombstones[id32(4)][0].author, ROOK);
  assert.deepEqual(expiryForBody({ kind: "unrecorded" }), {});
  assert.deepEqual(expiryForBody({ kind: "never" }), { expiry: null });
  assert.deepEqual(expiryForBody({ kind: "at", ms: 0 }), { expiry: 0 });
});

test("identifiers are checked, not coerced", () => {
  const good = ordinaryView(flipnoteContent({ frames: [{ id: F1, cid: id64(1), bytes: 10 }] }));
  assert.doesNotThrow(() => parseFlipnoteView(good));
  assert.throws(() => parseFlipnoteView({ ...good, epochId: "ABCDEF".padEnd(32, "0") }), /32 lowercase hex/);
  assert.throws(() => parseFlipnoteView({ ...good, epoch: "007" }), /canonical decimal/);
  assert.throws(() => parseFlipnoteView({ ...good, channel: 42 }), /canonical decimal/);
  const shortCid = ordinaryView(flipnoteContent({ frames: [{ id: F1, cid: "ab".repeat(16), bytes: 10 }] }));
  assert.throws(() => parseFlipnoteView(shortCid), /64 lowercase hex/);
  const badBytes = ordinaryView(flipnoteContent({ frames: [{ id: F1, cid: id64(1), bytes: 1.5 }] }));
  assert.throws(() => parseFlipnoteView(badBytes), /safe non-negative integer/);
  const badKey = ordinaryView({ ...flipnoteContent({ frames: [] }), frames: { nope: {} } });
  assert.throws(() => parseFlipnoteView(badKey), /object id/);
});

test("commands use the documented names and camelCase arguments, and a null read is absence", async () => {
  const ipc = fakeIpc();
  ipc.on("studio_list", () => ordinaryView(indexContent()));
  ipc.on("studio_read", () => null);
  ipc.on("studio_create", () => ordinaryView(flipnoteContent({ frames: [] })));
  ipc.on("studio_apply", () => ordinaryView(flipnoteContent({ frames: [] })));
  const target = { server: SERVER, channel: CHANNEL };
  await studioList(ipc, target);
  assert.equal(await studioRead(ipc, target, id32(1)), null);
  await studioCreate(ipc, target, { object: id32(1), nonce: id32(2), title: "t", createdAtMs: 123 });
  await studioApply(ipc, target, { object: id32(1), epochId: id32(3), nonce: id32(4), body: "{}" });
  assert.deepEqual(ipc.calls.map((c) => [c.cmd, Object.keys(c.args).sort()]), [
    ["studio_list", ["channel", "server"]],
    ["studio_read", ["channel", "object", "server"]],
    ["studio_create", ["channel", "createdAtMs", "nonce", "object", "server", "title"]],
    ["studio_apply", ["body", "channel", "epochId", "nonce", "object", "server"]],
  ]);
  assert.equal(ipc.calls[0].args.channel, CHANNEL, "the channel travels as the decimal string");
  await assert.rejects(studioCreate(ipc, target, { object: id32(1), nonce: id32(2), title: "t", createdAtMs: -1 }), /createdAtMs/);
});

test("publish_pix returns the backend's cid and must echo the exact byte length", async () => {
  const ipc = fakeIpc();
  const pix = pixBytes(1);
  ipc.on("publish_pix", (args) => ({ cid: id64(0xc1), bytes: base64ToBytes(String(args.bytesB64)).length }));
  const p = await publishPix(ipc, SERVER, pix);
  assert.deepEqual(p, { cid: id64(0xc1), bytes: pix.length });
  assert.deepEqual(base64ToBytes(String(ipc.calls[0].args.bytesB64)), pix);
  ipc.on("publish_pix", () => ({ cid: id64(0xc1), bytes: pix.length + 1 }));
  await assert.rejects(publishPix(ipc, SERVER, pix), /different length/);
  ipc.on("publish_pix", () => ({ cid: "not-a-cid", bytes: pix.length }));
  await assert.rejects(publishPix(ipc, SERVER, pix), /64 lowercase hex/);
  await assert.rejects(publishPix(ipc, SERVER, new Uint8Array(64 * 1024 + 1)), /64 KiB/);
});

test("request_blob_bounded passes the declared size and requires exactly that many bytes back", async () => {
  const ipc = fakeIpc();
  const pix = pixBytes(2);
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  const got = await requestBlobBounded(ipc, SERVER, id64(5), pix.length);
  assert.deepEqual(got, pix);
  assert.deepEqual(ipc.calls[0].args, { server: SERVER, cid: id64(5), maxBytes: pix.length });
  ipc.on("request_blob_bounded", () => null);
  assert.equal(await requestBlobBounded(ipc, SERVER, id64(5), pix.length), null, "null is unavailable, not an error");
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix), bytes: pix.length }));
  await assert.rejects(requestBlobBounded(ipc, SERVER, id64(5), pix.length - 1), /declared size/);
  ipc.on("request_blob_bounded", () => ({ bytes_b64: b64(pix.subarray(0, 10)), bytes: pix.length }));
  await assert.rejects(requestBlobBounded(ipc, SERVER, id64(5), pix.length), /declared size/);
  await assert.rejects(requestBlobBounded(ipc, SERVER, "zz", 1), /64 lowercase hex/);
});

test("recovery listing keeps the eviction deadline as the backend's decimal and versions newest-first as given", () => {
  const raw = recoveryListing({
    object: id32(1),
    versions: [version(2), version(1), version(3, { staged: true, reason: "excluded" })],
    evictionPending: { oldestSnapshot: id64(0x501), stagedSnapshot: id64(0x503), deadlineMs: "18446744073709551615" },
    pendingIntents: 2,
  });
  const l = parseRecoveryListing(raw);
  assert.equal(l.kind, "recoveryList");
  assert.deepEqual(l.versions.map((v) => [v.epoch, v.staged, v.reason]), [["2", false, "rewound"], ["1", false, "rewound"], ["3", true, "excluded"]]);
  assert.equal(l.evictionPending?.deadlineMs, "18446744073709551615");
  assert.equal(l.pendingIntents, 2);
  assert.equal(parseRecoveryListing(recoveryListing({ kind: "recoveryAcknowledged" })).kind, "recoveryAcknowledged");
  assert.equal(parseRecoveryListing(recoveryListing({ source: null })).source, null, "null source is local absence");
  assert.throws(() => parseRecoveryListing(recoveryListing({ versions: [{ ...version(1), reason: "pruned" }] })), /reason/);
  assert.throws(() => parseRecoveryListing(recoveryListing({ evictionPending: { oldestSnapshot: id64(1), stagedSnapshot: id64(2), deadlineMs: 5 } })), /deadlineMs/);
});

test("recovery preview, apply and export results are checked against their documented claims", () => {
  const ready = { v: 1, kind: "recoveryPreview", snapshot: id64(1), epochId: id32(2), expectedProjection: id64(3), disposition: "ready", body: "{\"op\":\"x\"}", originalAuthor: ROOK };
  assert.equal(parseRecoveryPreview(ready).body, "{\"op\":\"x\"}");
  assert.equal(parseRecoveryPreview({ ...ready, disposition: "conflict", body: null }).disposition, "conflict");
  assert.throws(() => parseRecoveryPreview({ ...ready, body: null }), /ready preview carries a body/);
  assert.throws(() => parseRecoveryPreview({ ...ready, disposition: "restored" }), /disposition/);
  const applied = { v: 1, kind: "recoveryApplied", contentSaved: true, alreadySaved: false, provisional: true, pointerRestored: false };
  assert.equal(parseRecoveryApplied(applied).alreadySaved, false);
  assert.throws(() => parseRecoveryApplied({ ...applied, pointerRestored: true }), /claims/);
  const bytes = new Uint8Array([1, 2, 3, 4, 5]);
  const exp = { v: 1, kind: "recoveryExport", snapshot: id64(1), format: "p1-recovery-v1", bytes: 5, bytesB64: bytesToBase64(bytes) };
  assert.equal(parseRecoveryExport(exp).bytes, 5);
  assert.throws(() => parseRecoveryExport({ ...exp, bytes: 6 }), /does not match/);
  assert.throws(() => parseRecoveryExport({ ...exp, format: "pixa" }), /format/);
});

test("recovery commands omit `object` for the Index and echo an apply edit verbatim", async () => {
  const ipc = fakeIpc();
  ipc.on("studio_recovery_preview", () => ({ v: 1, kind: "recoveryPreview", snapshot: id64(1), epochId: id32(2), expectedProjection: id64(3), disposition: "unchanged", body: null, originalAuthor: null }));
  ipc.on("studio_recovery_apply", () => ({ v: 1, kind: "recoveryApplied", contentSaved: true, alreadySaved: true, provisional: true, pointerRestored: false }));
  ipc.on("studio_recovery_acknowledge", () => recoveryListing({ kind: "recoveryAcknowledged" }));
  await recoveryPreview(ipc, { server: SERVER, channel: CHANNEL, object: null }, id64(1), { kind: "object", id: id32(9) }, "restore");
  assert.deepEqual(ipc.calls[0].args, { server: SERVER, channel: CHANNEL, snapshot: id64(1), choice: { kind: "object", id: id32(9) }, mode: "restore" });
  assert.equal("object" in ipc.calls[0].args, false, "the Index target omits object entirely");
  const edit = { snapshot: id64(1), choice: { kind: "frame" as const, id: id32(4), value: id64(5) }, mode: "copy" as const, epochId: id32(2), expectedProjection: id64(3), nonce: id32(6), body: "{}" };
  const applied = await recoveryApply(ipc, { server: SERVER, channel: CHANNEL, object: id32(7) }, edit);
  assert.equal(applied.alreadySaved, true);
  assert.deepEqual(ipc.calls[1].args, { server: SERVER, channel: CHANNEL, object: id32(7), edit });
  await recoveryAcknowledge(ipc, { server: SERVER, channel: CHANNEL, object: null }, { oldestSnapshot: id64(1), stagedSnapshot: id64(2) });
  assert.deepEqual(ipc.calls[2].args, { server: SERVER, channel: CHANNEL, oldestSnapshot: id64(1), stagedSnapshot: id64(2) });
});

test("event payloads parse with their documented scope fields and refuse unknown states", () => {
  assert.deepEqual(parseStudioUpdated({ server: 3, channel: CHANNEL, object: null }), { server: 3, channel: CHANNEL, object: null });
  assert.deepEqual(parseStudioUpdated({ server: 3, channel: CHANNEL, object: id32(1) }).object, id32(1));
  const s = parseSettlementChanged({ server: 3, docType: 16, logicalKey: id32(1), channel: CHANNEL, object: id32(1), state: "recoveryEvictionPending" });
  assert.equal(s.state, "recoveryEvictionPending");
  assert.throws(() => parseSettlementChanged({ server: 3, docType: 16, logicalKey: id32(1), channel: CHANNEL, object: id32(1), state: "receipted" }), /state/);
  assert.throws(() => parseSettlementChanged({ server: 3, docType: 17, logicalKey: id32(1), channel: CHANNEL, object: null, state: "open" }), /docType/);
});
