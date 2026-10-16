import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import test from "node:test";
import { TAKE_MAX_BYTES, type JamPatch, type JamTake } from "./jam-contract.ts";
import { jamPatchId } from "./jam-patch.ts";
import { parseJamTakeJson } from "./jam-recorder.ts";
import { decodePix } from "./pix.ts";
import { decodePixa, encodePixa, exportFlipnotePixa, PIXA_MAX_BYTES, type FlipnoteExportInput } from "./pixa.ts";
import { canonicalJson, FLIPNOTE_FRAME_BYTES_PROMISE } from "./studio-contract.ts";

const vector = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-export-v1.json", import.meta.url), "utf8"));
const ctx = { group: vector.group, exporter: vector.exporter };
const frameId = "1".repeat(32), otherFrameId = "2".repeat(32);
const emptyTake = (): JamTake => JSON.parse(vector.noScoreTake);

/** Independent PIX construction; permits exact encoded-size boundaries without a second codec. */
function pixWithRuns(runs = 108): Uint8Array {
  const bytes = new Uint8Array(23 + 2 * runs);
  bytes.set([80, 73, 88, 49, 191, 143, 3, 1, 19, 18, 24, 2, 232, 230, 240, 3, 151, 125, 242, 0, 224, 122, 184]);
  let remaining = 192 * 144;
  for (let i = 0; i < runs; i++) {
    const length = Math.min(256, remaining - (runs - i - 1));
    bytes[23 + 2 * i] = length - 1; bytes[24 + 2 * i] = i % 2; remaining -= length;
  }
  assert.equal(remaining, 0);
  return bytes;
}
// The golden frame is solid; the size-boundary helper above intentionally alternates run indices.
function goldenFrame(): Uint8Array {
  const bytes = pixWithRuns();
  for (let i = 24; i < bytes.length; i += 2) bytes[i] = 0;
  return bytes;
}
function source(overrides: Partial<FlipnoteExportInput> = {}): FlipnoteExportInput {
  return { id: vector.object, epoch: BigInt(vector.epoch), fps: 12, frames: [{ id: frameId, bytes: goldenFrame() }], patches: {}, sfx: [], ...overrides };
}
/** Test-only framing, deliberately permits invalid take text and out-of-range wire SFX. */
function rawPackage(take: string | Uint8Array, sfx: number[] = []): Uint8Array {
  const text = typeof take === "string" ? new TextEncoder().encode(take) : take;
  const frame = goldenFrame(), bytes = new Uint8Array(7 + 4 + frame.length + 4 + text.length + 2 + sfx.length);
  const view = new DataView(bytes.buffer);
  bytes.set([80, 73, 88, 65, 12, 1, 0]); view.setUint32(7, frame.length, true); bytes.set(frame, 11);
  const offset = 11 + frame.length;
  view.setUint32(offset, text.length, true); bytes.set(text, offset + 4);
  view.setUint16(offset + 4 + text.length, sfx.length / 8, true); bytes.set(sfx, offset + 6 + text.length);
  return bytes;
}
async function patches(count: number): Promise<Record<string, JamPatch>> {
  const table: Record<string, JamPatch> = {};
  for (let i = 0; i < count; i++) {
    const patch: JamPatch = structuredClone(vector.patch); patch.x.c = i;
    table[await jamPatchId(patch)] = patch;
  }
  return table;
}

test("no-score pixa is byte-exact to its independent golden vector and reuses the take/PIX readers", async () => {
  const bytes = await exportFlipnotePixa(source(), ctx);
  assert.equal(Buffer.from(bytes).toString("hex"), vector.pixaHex);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), vector.pixaSha256);
  const decoded = decodePixa(bytes);
  assert.equal(decoded.fps, 12);
  assert.equal(canonicalJson(decoded.take), vector.noScoreTake);
  assert.equal(parseJamTakeJson(canonicalJson(decoded.take)).ok, true, "the jukebox parser consumes the unchanged take format");
  assert.equal(decodePix(decoded.frames[0]).w, 192);
  assert.deepEqual(encodePixa(decoded), bytes);
  assert.deepEqual(decoded.sfx, []);
});

test("native and frontend pixa take validation share exact malformed-input verdicts", () => {
  const shared = JSON.parse(readFileSync(new URL("../../../crates/catcoms-replication/tests/fixtures/studio-pixa-validation-v1.json", import.meta.url), "utf8"));
  for (const entry of shared.cases) {
    const take = { ...emptyTake(), ...entry.patch };
    let accepted = true;
    try { decodePixa(rawPackage(canonicalJson(take))); } catch { accepted = false; }
    assert.equal(accepted, entry.accepted, entry.name);
  }
});

test("pixa builder orders stable SFX ids while preserving frame order and score patch indices", async () => {
  const patch = { [vector.patchId]: vector.patch };
  const input = source({ frames: [{ id: otherFrameId, bytes: goldenFrame() }, { id: frameId, bytes: pixWithRuns() }], patches: patch, sfx: [
    { id: "e".repeat(32), frame: otherFrameId, patch: vector.patchId, note: 127, durationMs: 1000 },
    { id: "a".repeat(32), frame: frameId, patch: vector.patchId, note: 0, durationMs: 20 },
  ] });
  const bytes = await exportFlipnotePixa(input, ctx), decoded = decodePixa(bytes);
  assert.deepEqual(decoded.sfx, [{ frame: 1, patch: 0, note: 0, durationMs: 20 }, { frame: 0, patch: 0, note: 127, durationMs: 1000 }]);
  assert.deepEqual(decoded.frames[0], goldenFrame());
  assert.deepEqual(await exportFlipnotePixa({ ...input, sfx: [...input.sfx].reverse() }, ctx), bytes);
});

test("63 linked-score patches plus one SFX patch round-trip, while 64 plus one refuses", async () => {
  const table = Object.entries(await patches(65)).sort(([a], [b]) => a < b ? -1 : 1);
  const score = { id: vector.object, bpm: 40, bpb: 8, steps: 256, patches: Object.fromEntries(table.slice(0, 63)), cells: { "255:24": { k: "n", p: table[62][0] } } };
  const input = source({ score, patches: Object.fromEntries(table.slice(63, 64)), sfx: [{ id: "a".repeat(32), frame: frameId, patch: table[63][0], note: 60, durationMs: 120 }] });
  const decoded = decodePixa(await exportFlipnotePixa(input, ctx));
  assert.equal(decoded.take.patches.length, 64);
  assert.deepEqual(decoded.sfx, [{ frame: 0, patch: 63, note: 60, durationMs: 120 }]);
  assert.ok("p" in decoded.take.events[0]);
  assert.equal(decoded.take.events[0].p, 62);
  await assert.rejects(exportFlipnotePixa({ ...input, score: { ...score, patches: Object.fromEntries(table.slice(0, 64)) }, patches: Object.fromEntries(table.slice(64)) }, ctx), /union exceeds 64/);
});

test("authored SFX duration has exact golden bytes and survives independently of animation FPS", async () => {
  const sfx = [{ id: "a".repeat(32), frame: frameId, patch: vector.patchId, note: 60, durationMs: 120 }];
  const bytes = await exportFlipnotePixa(source({ patches: { [vector.patchId]: vector.patch }, sfx }), ctx);
  assert.equal(bytes.length, vector.pixaSfxBytes);
  assert.equal(createHash("sha256").update(bytes).digest("hex"), vector.pixaSfxSha256);
  assert.equal(Buffer.from(bytes.subarray(-8)).toString("hex"), vector.sfxEntryHex);
  for (const fps of [1, 24]) for (const durationMs of [1, 20, 120, 1000, 600000]) {
    const decoded = decodePixa(await exportFlipnotePixa(source({ fps, patches: { [vector.patchId]: vector.patch }, sfx: [{ ...sfx[0], durationMs }] }), ctx));
    assert.equal(decoded.sfx[0].durationMs, durationMs, "source emoji/creator duration cannot become one animation frame");
  }
  const take = emptyTake(); take.patches = [vector.patch];
  for (const durationMs of [undefined, 0, -1, 1.5, 600001, NaN]) {
    assert.throws(() => encodePixa({ fps: 12, frames: [goldenFrame()], take, sfx: [{ frame: 0, patch: 0, note: 60, durationMs: durationMs as number }] }), /SFX duration/);
    await assert.rejects(exportFlipnotePixa(source({ patches: { [vector.patchId]: vector.patch }, sfx: [{ ...sfx[0], durationMs: durationMs as number }] }), ctx));
  }
  for (const durationMs of [0, 600001]) {
    const malformed = bytes.slice(); new DataView(malformed.buffer).setUint32(bytes.length - 4, durationMs, true);
    assert.throws(() => decodePixa(malformed), /SFX duration/);
  }
  const oldDraft = rawPackage(canonicalJson(take), [0, 0, 0, 60]);
  new DataView(oldDraft.buffer).setUint16(oldDraft.length - 6, 1, true);
  assert.throws(() => decodePixa(oldDraft), /truncated body/, "the old four-byte draft cannot silently invent a duration");
});

test("pixa supports 999 frames and 4096 SFX entries without retaining expanded rasters", () => {
  const take = emptyTake(); take.patches = [vector.patch];
  const frames = Array.from({ length: 999 }, () => goldenFrame());
  const sfx = Array.from({ length: 4096 }, (_, i) => ({ frame: i % 999, patch: 0, note: i % 128, durationMs: i + 1 }));
  const decoded = decodePixa(encodePixa({ fps: 24, frames, take, sfx }));
  assert.equal(decoded.frames.length, 999);
  assert.equal(decoded.sfx.length, 4096);
  assert.deepEqual(decoded.sfx, sfx);
  assert.ok(decoded.frames.every((frame) => frame instanceof Uint8Array && frame.length === 239));
});

test("the exact inclusive 8-MiB frame boundary passes; one additional frame refuses before serialization", () => {
  const full = pixWithRuns(192 * 144), frames = Array.from({ length: 151 }, () => full);
  const remainder = FLIPNOTE_FRAME_BYTES_PROMISE - full.length * frames.length;
  frames.push(pixWithRuns((remainder - 23) / 2));
  assert.equal(frames.reduce((sum, frame) => sum + frame.length, 0), FLIPNOTE_FRAME_BYTES_PROMISE);
  const bytes = encodePixa({ fps: 1, frames, take: emptyTake(), sfx: [] });
  assert.equal(decodePixa(bytes).frames.length, 152);
  assert.throws(() => encodePixa({ fps: 1, frames: [...frames, goldenFrame()], take: emptyTake(), sfx: [] }), /frame byte budget/);
});

test("all truncated prefixes, oversized lengths/counts, bad fps/magic and trailing bytes refuse", () => {
  const bytes = rawPackage(vector.noScoreTake);
  for (let length = 0; length < bytes.length; length++) assert.throws(() => decodePixa(bytes.subarray(0, length)), `prefix ${length}`);
  for (const [offset, value, reason] of [[0, 0, /magic/], [4, 0, /fps/], [4, 25, /fps/]] as const) {
    const bad = bytes.slice(); bad[offset] = value; assert.throws(() => decodePixa(bad), reason);
  }
  for (const count of [0, 1000, 65535]) {
    const bad = bytes.slice(); new DataView(bad.buffer).setUint16(5, count, true); assert.throws(() => decodePixa(bad), /frame count/);
  }
  for (const count of [0, 65537, 0xffff_ffff]) {
    const bad = bytes.slice(); new DataView(bad.buffer).setUint32(7, count, true); assert.throws(() => decodePixa(bad), /frame length/);
  }
  for (const length of [0, TAKE_MAX_BYTES + 1, 0xffff_ffff]) {
    const bad = bytes.slice(); new DataView(bad.buffer).setUint32(11 + goldenFrame().length, length, true); assert.throws(() => decodePixa(bad), /take byte budget/);
  }
  const badCount = bytes.slice(); new DataView(badCount.buffer).setUint16(bytes.length - 2, 4097, true);
  assert.throws(() => decodePixa(badCount), /SFX count/);
  assert.throws(() => decodePixa(new Uint8Array(PIXA_MAX_BYTES + 1)), /whole byte budget/);
  const trailing = new Uint8Array(bytes.length + 1); trailing.set(bytes);
  assert.throws(() => decodePixa(trailing), /trailing bytes/);
});

test("take decoding rejects noncanonical JSON, duplicate fields, invalid UTF-8 and invalid take semantics", () => {
  for (const text of [
    vector.noScoreTake + " ", vector.noScoreTake.replace('"v":1', '"v":1,"v":1'),
    vector.noScoreTake.replace('"bpm":120', '"bpm":1.2e2'), JSON.stringify({ v: 1, ...emptyTake() }),
    vector.noScoreTake.replace('"events":[]', '"events":[{"ms":0}]'),
    canonicalJson({ ...emptyTake(), group: "\ud800" }),
    canonicalJson({ ...emptyTake(), patches: [{ ...vector.patch, extra: true }] }),
  ]) assert.throws(() => decodePixa(rawPackage(text)), `invalid take: ${text.slice(0, 100)}`);
  const text = new TextEncoder().encode(vector.noScoreTake); text[10] = 0xff;
  assert.throws(() => decodePixa(rawPackage(text)), /UTF-8/);
});

test("pixa import and export reject non-string take session nonces", () => {
  for (const sn of [1111111111111111, ["1111111111111111"], [1111111111111111]]) {
    const take = JSON.parse(vector.noScoreTake);
    take.lanes[0].sn = sn;
    const text = canonicalJson(take);
    assert.equal(parseJamTakeJson(text).ok, false);
    // Construct framing independently: encoder rejection alone cannot pin decoder admission.
    assert.throws(() => decodePixa(rawPackage(text)), /take lane/);
    assert.throws(() => encodePixa({ fps: 12, frames: [goldenFrame()], take, sfx: [] }), /take lane/);
  }
});

test("PIX validation, exact Flipnote dimensions and SFX indices remain mandatory", () => {
  const bytes = rawPackage(vector.noScoreTake);
  for (const [offset, value] of [[11, 0], [15, 190], [35, 99], [34, 0]] as const) {
    const bad = bytes.slice(); bad[offset] = value; assert.throws(() => decodePixa(bad));
  }
  const take = emptyTake(); take.patches = [vector.patch]; const text = canonicalJson(take);
  for (const sfx of [[1, 0, 0, 60], [0, 0, 1, 60], [0, 0, 0, 128]]) assert.throws(() => decodePixa(rawPackage(text, [...sfx, 120, 0, 0, 0])), /SFX index or note/);
  assert.throws(() => decodePixa(rawPackage(vector.noScoreTake, [0, 0, 0, 60, 120, 0, 0, 0])), /SFX index or note/);
});

test("encoder rejects invalid metadata, too many frames/SFX and oversized frame input", () => {
  const good = { fps: 12, frames: [goldenFrame()], take: emptyTake(), sfx: [] };
  for (const fps of [0, 25, 1.5, NaN]) assert.throws(() => encodePixa({ ...good, fps }), /fps/);
  for (const frames of [[], Array.from({ length: 1000 }, () => goldenFrame()), [new Uint8Array(65537)]]) assert.throws(() => encodePixa({ ...good, frames }));
  assert.throws(() => encodePixa({ ...good, sfx: Array.from({ length: 4097 }, () => ({ frame: 0, patch: 0, note: 60, durationMs: 120 })) }), /SFX count/);
  assert.throws(() => encodePixa({ ...good, take: { ...good.take, v: 2 } as unknown as JamTake }));
});

test("builder rejects duplicate/stale frame and SFX ids, missing recipes and unavailable source frames", async () => {
  const oneSfx = { id: "a".repeat(32), frame: frameId, patch: vector.patchId, note: 60, durationMs: 120 };
  for (const input of [
    source({ frames: [{ id: frameId, bytes: goldenFrame() }, { id: frameId, bytes: goldenFrame() }] }),
    source({ frames: [{ id: "A".repeat(32), bytes: goldenFrame() }] }),
    source({ sfx: [oneSfx] }),
    source({ patches: { [vector.patchId]: vector.patch }, sfx: [{ ...oneSfx, frame: otherFrameId }] }),
    source({ patches: { [vector.patchId]: vector.patch }, sfx: [oneSfx, oneSfx] }),
    source({ sfx: [{ ...oneSfx, note: -1 }] }),
  ]) await assert.rejects(exportFlipnotePixa(input, ctx));
});

test("async builder and decoded frames/take do not alias mutable caller input", async () => {
  const input = structuredClone(source({ patches: { [vector.patchId]: vector.patch }, sfx: [{ id: "a".repeat(32), frame: frameId, patch: vector.patchId, note: 60, durationMs: 120 }] }));
  const expected = await exportFlipnotePixa(input, ctx), pending = exportFlipnotePixa(input, ctx);
  input.frames[0].bytes.fill(0); (input.sfx[0] as { note: number }).note = 99;
  (input.sfx[0] as { durationMs: number }).durationMs = 1000;
  (input as { fps: number }).fps = 1;
  assert.deepEqual(await pending, expected);
  const decoded = decodePixa(expected); const frame = decoded.frames[0].slice();
  expected.fill(0);
  assert.deepEqual(decoded.frames[0], frame);
  assert.equal(decoded.take.call, vector.flipnoteCall);
});

test("decoding a view respects its byte offset rather than reading outside the supplied package", () => {
  const bytes = rawPackage(vector.noScoreTake), padded = new Uint8Array(bytes.length + 16);
  padded.set(bytes, 8);
  assert.deepEqual(encodePixa(decodePixa(padded.subarray(8, 8 + bytes.length))), bytes);
});

test("encoder snapshots wrapper scalars/SFX once before checking them", () => {
  const take = emptyTake(); take.patches = [vector.patch];
  let noteReads = 0, fpsReads = 0, durationReads = 0;
  const bytes = encodePixa({ get fps() { return ++fpsReads === 1 ? 12 : 255; }, frames: [goldenFrame()], take,
    sfx: [{ frame: 0, patch: 0, get note() { return ++noteReads === 1 ? 60 : 255; }, get durationMs() { return ++durationReads === 1 ? 120 : 600001; } }],
  });
  const decoded = decodePixa(bytes);
  assert.equal(decoded.fps, 12);
  assert.deepEqual(decoded.sfx, [{ frame: 0, patch: 0, note: 60, durationMs: 120 }]);
  assert.equal(fpsReads, 1); assert.equal(noteReads, 1); assert.equal(durationReads, 1);
});

test("Buffer, hostile typed-array subclass and shared-memory inputs acquire owned snapshots", async () => {
  const golden = Buffer.from(vector.pixaHex, "hex"), decoded = decodePixa(golden);
  golden.fill(0);
  assert.deepEqual(decoded.frames[0], goldenFrame(), "Buffer.slice must not leak an alias");

  class HostileBytes extends Uint8Array {
    override slice(): Uint8Array { throw new Error("caller slice must not run"); }
    override subarray(): Uint8Array { throw new Error("caller subarray must not run"); }
  }
  assert.deepEqual(encodePixa(decodePixa(new HostileBytes(Buffer.from(vector.pixaHex, "hex")))), new Uint8Array(Buffer.from(vector.pixaHex, "hex")));
  // A forged own length cannot turn a large typed array into an admitted one-byte buffer.
  const disguised = new HostileBytes(PIXA_MAX_BYTES + 1);
  Object.defineProperty(disguised, "length", { value: 1 });
  assert.throws(() => decodePixa(disguised), /whole byte budget/);
  const disguisedFrame = new HostileBytes(65537);
  Object.defineProperty(disguisedFrame, "length", { value: 1 });
  assert.throws(() => encodePixa({ fps: 12, frames: [disguisedFrame], take: emptyTake(), sfx: [] }), /frame length/);

  for (const frame of [Buffer.from(goldenFrame()), new HostileBytes(goldenFrame()), new Uint8Array(new SharedArrayBuffer(goldenFrame().length))]) {
    frame.set(goldenFrame());
    const input = source({ frames: [{ id: frameId, bytes: frame }], patches: { [vector.patchId]: vector.patch } });
    const expected = await exportFlipnotePixa(input, ctx), pending = exportFlipnotePixa(input, ctx);
    frame.fill(0);
    assert.deepEqual(await pending, expected, "caller mutation after the digest starts cannot change this export");
    frame.set(goldenFrame());
    const encoded = encodePixa({ fps: 12, frames: [frame], sfx: [], get take() {
      // The take is read after frame snapshots. Simulate a concurrent/shared caller changing
      // those source bytes during the remaining validation/serialization steps.
      frame.fill(0); return emptyTake();
    } });
    assert.deepEqual(decodePixa(encoded).frames[0], goldenFrame(), "the synchronous encoder also validates its owned copy");
  }
  const shared = new Uint8Array(new SharedArrayBuffer(vector.pixaHex.length / 2));
  shared.set(Buffer.from(vector.pixaHex, "hex"));
  const owned = decodePixa(shared); shared.fill(0);
  assert.deepEqual(owned.frames[0], goldenFrame());
});

test("a growable shared frame cannot enlarge an allocation after its length was admitted", async () => {
  function racingFrames() {
    const shared = new SharedArrayBuffer(goldenFrame().length, { maxByteLength: 1024 * 1024 });
    const tracking = new Uint8Array(shared); tracking.set(goldenFrame());
    // The first frame's 239-byte length is admitted. Reading the second frame then simulates a
    // worker growing the first view before copying. The destination must remain exactly 239 bytes.
    return [tracking, { get bytes() { shared.grow(1024 * 1024); return goldenFrame(); } }];
  }
  const [first, trigger] = racingFrames();
  const frames = [first, goldenFrame()] as Uint8Array[];
  Object.defineProperty(frames, "1", { get: () => (trigger as { bytes: Uint8Array }).bytes });
  assert.throws(() => encodePixa({ fps: 12, frames, take: emptyTake(), sfx: [] }), RangeError);
  const [builderFirst, builderTrigger] = racingFrames();
  await assert.rejects(exportFlipnotePixa(source({ frames: [
    { id: frameId, bytes: builderFirst as Uint8Array },
    { id: otherFrameId, get bytes() { return (builderTrigger as { bytes: Uint8Array }).bytes; } },
  ] }), ctx), /frame length/);
});
