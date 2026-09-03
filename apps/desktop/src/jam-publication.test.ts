import assert from "node:assert/strict";
import test from "node:test";
import { JamCausalQueue, JamCausalQueueOverflow, JamInitialPublicationGate, JamLatestTaskQueue, JamOutboundEdge, JamPublicationGeneration, JamPublicationPacer, JamResettableCausalQueue, type JamPublishedFrame } from "./jam-publication.ts";
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

test("an unopened edge drops paced history and establishes only the current patch", async () => {
  const edge = new JamOutboundEdge();
  const decoder = new JamFrameDecoder();
  const sent: string[] = [];
  const send = (frame: string) => { sent.push(frame); return true; };
  const publications: JamPublishedFrame[] = [];
  for (let index = 0; index < 4; index += 1) {
    const patch: JamPatch = {
      v: 1,
      o: [{ w: 0, t: 0, c: 0, l: 100 }],
      e: { a: 0, d: 10, s: 80, r: 100 },
      f: { m: 0, c: 1_000 + index, q: 10, e: 0 },
      l: { r: 100, d: 0, t: 0 },
      x: { c: 0, d: 0, r: 0 },
    };
    const id = await jamPatchId(patch);
    publications.push({
      key: `1111111111111111:${id}`,
      announce: JSON.stringify({ t: "p", v: 1, id, sn: "1111111111111111", d: patch }),
    });
    assert.equal(edge.event(
      publications[index],
      JSON.stringify({ t: "n", on: 1, n: 60, w: "sine", p: id, q: index }),
      send,
    ), false);
  }
  // Exercise both receiver attack and all-frame burst ceilings. None of this stale performance
  // is emitted, so elapsed sender history cannot become a receiver-budget burst on open.
  for (let q = 4; q < 205; q += 1) {
    assert.equal(edge.event(
      publications[3],
      JSON.stringify({ t: "d", n: q % 10, q }),
      send,
    ), false);
  }
  assert.deepEqual(sent, []);

  edge.open(publications[3], send);
  assert.equal(sent.length, 1);
  assert.equal(decoder.decode(sent[0], 10_000).ok, true, "the sole current prerequisite fits a fresh decoder budget");

  // Fresh held-state edges remain paired even though their sequence follows explicitly lost
  // history. This is the live path, not a replay of the old note-on/note-off stream.
  assert.equal(edge.event(
    publications[3],
    JSON.stringify({ t: "n", on: 1, n: 64, w: "sine", q: 205 }),
    send,
  ), true);
  assert.equal(edge.event(
    publications[3],
    JSON.stringify({ t: "n", on: 0, n: 64, q: 206 }),
    send,
  ), true);
  assert.equal(sent.length, 3);
  assert.equal(decoder.decode(sent[1], 10_001).ok, true);
  assert.equal(decoder.decode(sent[2], 10_002).ok, true);
});

test("an edge opened during the initial digest cannot receive the App prepublication backlog", async () => {
  const patch: JamPatch = {
    v: 1,
    o: [{ w: 0, t: 0, c: 0, l: 100 }],
    e: { a: 0, d: 10, s: 80, r: 100 },
    f: { m: 0, c: 1_000, q: 10, e: 0 },
    l: { r: 100, d: 0, t: 0 },
    x: { c: 0, d: 0, r: 0 },
  };
  const id = await jamPatchId(patch);
  const publication: JamPublishedFrame = {
    key: `1111111111111111:${id}`,
    announce: JSON.stringify({ t: "p", v: 1, id, sn: "1111111111111111", d: patch }),
  };
  const gate = new JamInitialPublicationGate<JamPublishedFrame>(256);
  const edge = new JamOutboundEdge();
  const sent: string[] = [];
  const send = (frame: string) => { sent.push(frame); return true; };
  let localEvents = 0;

  // More than both the receiver attack and frame burst budgets waits while the digest is absent.
  for (let q = 0; q < 201; q += 1) {
    assert.equal(gate.submit(null, (ready, outbound) => {
      localEvents += 1;
      if (outbound) edge.event(ready, JSON.stringify({ t: "d", n: q % 10, q }), send);
    }), "queued");
  }
  edge.open(publication, send);
  gate.flush(publication);
  assert.equal(localEvents, 201, "the bounded backlog remains available to local render/recording");
  assert.deepEqual(sent, [publication.announce], "digest completion emits no historical wire burst");

  assert.equal(gate.submit(publication, (ready, outbound) => {
    assert.equal(outbound, true);
    edge.event(ready, JSON.stringify({ t: "n", on: 1, n: 64, w: "sine", q: 201 }), send);
  }), "live");
  assert.equal(sent.length, 2, "a fresh post-publication gesture reaches the ready edge");
  const decoder = new JamFrameDecoder();
  assert.equal(decoder.decode(sent[0], 10_000).ok, true);
  assert.equal(decoder.decode(sent[1], 10_001).ok, true);
});

test("an announcement send failure suppresses its dependent event", () => {
  const edge = new JamOutboundEdge();
  const publication = { key: "sn:id", announce: "patch" };
  const attempted: string[] = [];
  edge.open(publication, (frame) => { attempted.push(frame); return false; });
  assert.equal(edge.event(publication, "note", (frame) => { attempted.push(frame); return false; }), false);
  assert.deepEqual(attempted, ["patch", "patch"]);
});

test("local queue overflow retires retained attacks and admits a fresh generation", async () => {
  const queue = new JamResettableCausalQueue(2);
  const first = deferred<void>();
  const ran: string[] = [];
  let invalidations = 0;
  const active = queue.enqueue(async () => { await first.promise; ran.push("active-finished"); }, () => {
    invalidations += 1;
  });
  await Promise.resolve();
  const retained = queue.enqueue(() => { ran.push("retained-attack"); }, () => {
    invalidations += 1;
  });
  const overflow = queue.enqueue(() => { ran.push("overflowing-attack"); }, () => {
    invalidations += 1;
  });
  assert.equal(invalidations, 1, "overflow invalidates the engine synchronously");
  const fresh = queue.enqueue(() => { ran.push("fresh-attack"); }, () => {
    invalidations += 1;
  });
  await fresh;
  first.resolve();
  await Promise.all([active, retained, overflow]);
  assert.deepEqual(ran, ["fresh-attack", "active-finished"]);
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
