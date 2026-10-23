// Test-only peer of the native acceptance dispatcher. Every invoke response comes from the
// real Rust adapter: this runner supplies no synthetic views, blob receipts or saved results.
import assert from "node:assert/strict";
import { createInterface } from "node:readline";
import { StudioSession } from "../src/studio-session.ts";
import { decodePixa } from "../src/pixa.ts";

const input = createInterface({ input: process.stdin });
const pending = new Map();
let sequence = 0, initialize;
const initialized = new Promise((resolve) => { initialize = resolve; });
input.on("line", (line) => {
  const value = JSON.parse(line);
  if (initialize) { const resolve = initialize; initialize = null; resolve(value); return; }
  const call = pending.get(value.id);
  assert.ok(call, "native response must identify an outstanding invoke");
  pending.delete(value.id);
  if (value.error !== undefined) call.reject(new Error(value.error)); else call.resolve(value.value);
});
const init = await initialized;
const calls = [];
const ipc = {
  invoke(cmd, args) {
    const id = ++sequence;
    calls.push(cmd);
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      process.stdout.write(`${JSON.stringify({ id, cmd, args })}\n`);
    });
  },
  // No native events are needed during this isolated request/response phase. Event delivery
  // and all nine invalidations are exercised separately through the real session contract.
  async listen() { return () => {}; },
};
let identity = 0x700;
const session = new StudioSession({ ipc, me: init.me, newId: () => (++identity).toString(16).padStart(32, "0"),
  schedule: (fn) => { const timer = setTimeout(fn, 0); return () => clearTimeout(timer); } });
function waitFor(predicate) {
  return new Promise((resolve, reject) => {
    let stop = () => {};
    const check = () => {
      const failed = session.doc?.error || session.indexError || session.overlay?.error
        || session.saves.find((s) => s.status === "blocked" || s.status === "uncertain")?.error;
      if (failed) { stop(); reject(new Error(failed)); }
      else if (predicate()) { stop(); resolve(); }
    };
    stop = session.onChange(check);
    check();
  });
}
try {
  await session.attach();
  session.setScope({ server: init.server, channel: init.channel });
  await waitFor(() => session.index !== null && !session.indexLoading);
  session.open(init.object);
  await waitFor(() => session.doc?.view && session.overlay && !session.overlay.loading);
  let exportId = init.exportId;
  if (init.mode === "publish") {
    const published = await session.publishFlipnotePixa(init.object);
    assert.equal(published.warning, null);
    assert.ok(published.referenceSaveId, "publication must schedule a separate reference Save");
    await waitFor(() => session.saves.length === 0
      && session.doc?.model?.sound.exports.some((e) => e.cid === published.publication.cid));
    const record = session.doc.model.sound.exports.find((e) => e.cid === published.publication.cid);
    assert.equal(record.bytes, published.publication.bytes);
    exportId = record.id;
    assert.ok(calls.includes("publish_pixa") && calls.includes("studio_apply"));
  }
  const bytes = await session.preparePublishedPixa(init.object, exportId);
  const value = decodePixa(bytes);
  // Independent authored expectations: two equally broken peers must not count as success.
  assert.equal(value.take.group, init.group);
  assert.deepEqual(value.take.parts, [init.exporter]);
  assert.equal(value.fps, 17);
  assert.deepEqual(value.frames.map((f) => Buffer.from(f).toString("base64")), init.frames);
  assert.deepEqual(value.take.patches, [init.recipe]);
  assert.deepEqual(value.take.events.map(({ ms, n, on, p }) => ({ ms, n, on, ...(p === undefined ? {} : { p }) })),
    // Existing take quantization releases one millisecond before the next step boundary
    // (also pinned by studio-export.test.ts), preserving adjacency without overlapping holds.
    [{ ms: 125, n: 62, on: 1, p: 0 }, { ms: 374, n: 62, on: 0 }]);
  assert.deepEqual(value.sfx, [{ frame: 1, patch: 0, note: 72, durationMs: 375 }]);
  assert.ok(calls.includes("request_blob_bounded"));
  assert.equal(pending.size, 0);
  session.detach();
  process.stdout.write(`${JSON.stringify({ done: true, result: { exportId, bytesB64: Buffer.from(bytes).toString("base64"), calls } })}\n`);
} finally {
  session.detach();
  input.close();
  process.stdin.destroy();
}
