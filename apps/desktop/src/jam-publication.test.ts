import assert from "node:assert/strict";
import test from "node:test";
import { JamCausalQueue, JamCausalQueueOverflow, JamLatestTaskQueue, JamOutboundEdge, JamPublicationGeneration, JamPublicationPacer } from "./jam-publication.ts";
import { JAM_PATCH_ANNOUNCE_MIN_INTERVAL_MS, type JamPatch } from "./jam-contract.ts";
import { JamFrameDecoder } from "./jam-wire.ts";
import { jamPatchId } from "./jam-patch.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

test("ordered delivery remains causal across an asynchronous patch digest", async () => {
  const queue = new JamCausalQueue();
  const digest = deferred<void>();
  const seen: string[] = [];
  const patch = queue.enqueue(async () => { await digest.promise; seen.push("patch"); });
  const note = queue.enqueue(() => { seen.push("note"); });
  await Promise.resolve();
  assert.deepEqual(seen, []);
  digest.resolve();
  await Promise.all([patch, note]);
  assert.deepEqual(seen, ["patch", "note"]);
});

test("a rejected queued operation does not wedge following ordered work", async () => {
  const queue = new JamCausalQueue();
  await assert.rejects(queue.enqueue(() => { throw new Error("bad patch"); }));
  assert.equal(await queue.enqueue(() => "next note"), "next note");
});

test("a delayed digest cannot retain an unbounded causal backlog", async () => {
  const queue = new JamCausalQueue(2);
  const digest = deferred<void>();
  const first = queue.enqueue(() => digest.promise);
  const second = queue.enqueue(() => undefined);
  await assert.rejects(queue.enqueue(() => undefined), JamCausalQueueOverflow);
  digest.resolve();
  await Promise.all([first, second]);
  assert.equal(await queue.enqueue(() => "recovered"), "recovered");
});

test("editor publication retains only the newest draft behind a stalled digest", async () => {
  const queue = new JamLatestTaskQueue<number>();
  const digest = deferred<void>();
  const ran: number[] = [];
  const first = queue.submit(async () => { ran.push(0); await digest.promise; return 0; });
  await Promise.resolve();

  const superseded: Array<Promise<number | null>> = [];
  for (let draft = 1; draft < 100; draft += 1) {
    superseded.push(queue.submit(() => { ran.push(draft); return draft; }));
  }
  const newest = queue.submit(() => { ran.push(100); return 100; });
  assert.deepEqual(await Promise.all(superseded), new Array(99).fill(null));
  assert.deepEqual(ran, [0]);

  digest.resolve();
  assert.deepEqual(await Promise.all([first, newest]), [0, 100]);
  assert.deepEqual(ran, [0, 100]);
});

test("a new call epoch is not blocked by an old call's stalled digest", async () => {
  const generations = new JamPublicationGeneration();
  const oldGeneration = generations.current();
  const digest = deferred<void>();
  let queue = new JamLatestTaskQueue<string>();
  const old = queue.submit(async () => {
    await digest.promise;
    return generations.isCurrent(oldGeneration) ? "old-published" : "old-stale";
  });

  generations.advance();
  queue = new JamLatestTaskQueue<string>();
  assert.equal(await queue.submit(() => "new-published"), "new-published");
  digest.resolve();
  assert.equal(await old, "old-stale");
});

test("an unopened edge receives each prerequisite announce before dependent events", () => {
  const edge = new JamOutboundEdge(8);
  const oldPatch = { key: "sn:old", announce: "patch-old" };
  const newPatch = { key: "sn:new", announce: "patch-new" };
  const sent: string[] = [];
  const send = (frame: string) => { sent.push(frame); return true; };
  edge.event(oldPatch, "note-old", send);
  edge.event(newPatch, "note-new", send);
  edge.open(newPatch, send);
  assert.deepEqual(sent, ["patch-old", "note-old", "patch-new", "note-new"]);
});

test("unopened-edge overflow drops the transient as a unit and stays bounded", () => {
  const edge = new JamOutboundEdge(2);
  const patch = { key: "sn:id", announce: "patch" };
  const sent: string[] = [];
  const send = (frame: string) => { sent.push(frame); return true; };
  assert.equal(edge.event(patch, "on", send), true);
  assert.equal(edge.event(patch, "off", send), true);
  assert.equal(edge.event(patch, "overflow", send), false);
  edge.open(patch, send);
  assert.deepEqual(sent, ["patch"]);
});

test("an announcement send failure suppresses its dependent event", () => {
  const edge = new JamOutboundEdge(2);
  const publication = { key: "sn:id", announce: "patch" };
  const attempted: string[] = [];
  edge.open(publication, (frame) => { attempted.push(frame); return false; });
  assert.equal(edge.event(publication, "note", (frame) => { attempted.push(frame); return false; }), false);
  assert.deepEqual(attempted, ["patch", "patch"]);
});

test("publication generations reject an older digest completion", () => {
  const generations = new JamPublicationGeneration();
  const older = generations.current();
  const newer = generations.advance();
  assert.equal(generations.isCurrent(older), false);
  assert.equal(generations.isCurrent(newer), true);
});

test("normal editor churn is sender-paced within a real receiver's patch budget", async () => {
  const patch: JamPatch = {
    v: 1,
    o: [{ w: 0, t: 0, c: 0, l: 100 }],
    e: { a: 0, d: 10, s: 80, r: 100 },
    f: { m: 0, c: 1_000, q: 10, e: 0 },
    l: { r: 100, d: 0, t: 0 },
    x: { c: 0, d: 0, r: 0 },
  };
  const id = await jamPatchId(patch);
  const announce = JSON.stringify({ t: "p", v: 1, id, sn: "1111111111111111", d: patch });
  const pacer = new JamPublicationPacer(JAM_PATCH_ANNOUNCE_MIN_INTERVAL_MS);
  const decoder = new JamFrameDecoder();
  const published: number[] = [];
  for (let now = 0; now <= 6_000; now += 400) {
    if (!pacer.publish(now)) continue;
    published.push(now);
    assert.equal(decoder.decode(announce, now).ok, true);
  }
  assert.deepEqual(published, [0, 2_000, 4_000, 6_000]);
  assert.equal(
    decoder.decode(JSON.stringify({ t: "n", on: 1, n: 60, w: "sine", p: id, q: 1 }), 6_001).ok,
    true,
  );
});
