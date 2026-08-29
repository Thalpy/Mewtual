import assert from "node:assert/strict";
import test from "node:test";
import {
  clearDeliverySnapshotAfterFailedQuery,
  deliveryClass,
  deliveryGlyph,
  deliveryLabel,
  deliveryVerdict,
  mergeDelivery,
  replaceDeliverySnapshot,
  type DeliveryEvidence,
} from "./delivery.ts";

const ev = (over: Partial<DeliveryEvidence> = {}): DeliveryEvidence => ({
  others: 2,
  delivered: 0,
  reachable: 2,
  anyPeer: true,
  pending: false,
  latest: true,
  ...over,
});

test("proof of arrival is never overridden by the state of the network", () => {
  // The regression: a peer confirmed, then its connection flapped, and the tick went red on a
  // message that had demonstrably arrived. A holder does not stop holding it.
  assert.equal(deliveryVerdict(ev({ delivered: 1, reachable: 0, others: 2 })), "partial");
  assert.equal(deliveryVerdict(ev({ delivered: 2, reachable: 0, others: 2 })), "everyone");
  assert.equal(deliveryVerdict(ev({ delivered: 1, reachable: 1, others: 2 })), "partial");
});

test("an offline holder cannot stand in for a different reachable member", () => {
  // Three devices: Bob confirmed and then disconnected; Carol is reachable but has not confirmed.
  // The backend currently reports the two set cardinalities, not their intersection, so 1 === 1
  // must remain a generic positive rather than claiming every reachable member confirmed.
  assert.equal(deliveryVerdict(ev({ others: 2, delivered: 1, reachable: 1 })), "partial");
  assert.equal(
    deliveryLabel("partial", ev({ others: 2, delivered: 1, reachable: 1 })),
    "held by 1 peer",
  );
});

test("red requires a measurement that says nothing can leave this device", () => {
  // Reported, and genuinely nothing connected.
  assert.equal(deliveryVerdict(ev({ delivered: 0, reachable: 0, anyPeer: false })), "queued");
  // Connected, but no member resolved to that connection yet. `reachable` counts members matched
  // through signed peer records, so it reads zero while ops gossip out perfectly well: this is
  // the case that kept announcing a delivery failure on messages that had already arrived.
  assert.equal(deliveryVerdict(ev({ delivered: 0, reachable: 0, anyPeer: true })), "waiting");
  // Not reported at all: the window right after pressing send. Absence of a measurement is not
  // a measurement of absence.
  assert.equal(deliveryVerdict(ev({ delivered: null, reachable: null, anyPeer: null })), "waiting");
  assert.equal(deliveryVerdict(ev({ delivered: 0, reachable: 1 })), "waiting");
});

test("an unreported older message says nothing rather than sending forever", () => {
  // The actor keeps only its most recent own messages per channel, in memory, so after a restart
  // an old message has no evidence either way. Silence is the honest answer; "sending…" is not.
  assert.equal(deliveryVerdict(ev({ delivered: null, reachable: null, latest: false })), null);
  assert.equal(deliveryVerdict(ev({ delivered: null, reachable: null, latest: true })), "waiting");
  // A reported older message still shows its state: that is the whole point of per-message ticks.
  assert.equal(deliveryVerdict(ev({ delivered: 2, others: 2, latest: false })), "everyone");
});

test("alone in a group there is nothing to claim", () => {
  assert.equal(deliveryVerdict(ev({ others: 0 })), null);
  assert.equal(deliveryVerdict(ev({ others: 0, delivered: null })), null);
  // Except while it is still being written locally, which is about this device, not the group.
  assert.equal(deliveryVerdict(ev({ others: 0, pending: true })), "pending");
});

test("the ordinary progression reads the way it should", () => {
  assert.equal(deliveryVerdict(ev({ others: 3, delivered: null, reachable: null })), "waiting");
  assert.equal(deliveryVerdict(ev({ others: 3, delivered: 0, reachable: 2 })), "waiting");
  assert.equal(deliveryVerdict(ev({ others: 3, delivered: 1, reachable: 2 })), "partial");
  assert.equal(deliveryVerdict(ev({ others: 3, delivered: 2, reachable: 2 })), "partial");
  assert.equal(deliveryVerdict(ev({ others: 3, delivered: 3, reachable: 2 })), "everyone");
});

test("every verdict has a glyph and a colour", () => {
  const all = ["pending", "waiting", "partial", "everyone", "queued"] as const;
  for (const v of all) {
    assert.ok(deliveryGlyph(v).length > 0, v);
    assert.ok(deliveryClass(v).startsWith("d-"), v);
  }
  assert.equal(deliveryGlyph("everyone"), "✓✓");
  assert.equal(deliveryGlyph("queued"), "✕");
});

test("labels never say 'of 0 reachable'", () => {
  // Held by somebody while nothing is reachable is a real state, and it has to read as a sentence.
  assert.equal(deliveryLabel("partial", ev({ delivered: 1, reachable: 0, others: 2 })), "held by 1 peer");
  assert.equal(deliveryLabel("partial", ev({ delivered: 1, reachable: 3, others: 4 })), "held by 1 peer");
  assert.equal(deliveryLabel("everyone", ev({ others: 1 })), "delivered · all 1 member");
});

test("a locally accepted send is not labelled as still sending", () => {
  assert.equal(deliveryLabel("pending", ev({ pending: true })), "saving…");
  assert.equal(
    deliveryLabel("waiting", ev({ delivered: 0, reachable: 1 })),
    "sent · awaiting confirmation",
  );
});

test("a current-roster report can drop a removed member's receipt", () => {
  const report = (delivered: number, reachable: number, any_peer = true) => ({
    delivered,
    reachable,
    any_peer,
  });
  // Bob confirmed, then was removed and replaced by Carol. The backend's current snapshot says
  // Carol has not confirmed; retaining Bob's numeric count would falsely claim "everyone".
  assert.deepEqual(mergeDelivery(report(1, 1), report(0, 1)), report(0, 1));
  assert.deepEqual(mergeDelivery(report(1, 1), report(3, 3)), report(3, 3));
  // Reachability and connectedness are live, and are taken exactly as reported.
  assert.deepEqual(mergeDelivery(undefined, report(0, 0, false)), report(0, 0, false));
  assert.deepEqual(mergeDelivery(report(2, 2, true), report(2, 0, false)), report(2, 0, false));
});

test("a full actor snapshot removes evicted delivery rows", () => {
  const report = (id: string, delivered: number): import("./delivery.ts").DeliveryReport => ({
    id,
    delivered,
    reachable: 1,
    any_peer: true,
  });
  const previous = { a: report("a", 1), b: report("b", 1) };
  assert.deepEqual(replaceDeliverySnapshot(
    { revision: 4, reports: previous },
    { revision: 5, states: [report("b", 0)] },
  ), {
    revision: 5,
    reports: { b: report("b", 0) },
  });
});

test("a delayed delivery query cannot overwrite a newer event", () => {
  const report = (id: string, delivered: number): import("./delivery.ts").DeliveryReport => ({
    id,
    delivered,
    reachable: 1,
    any_peer: true,
  });
  const newer = replaceDeliverySnapshot(
    { revision: 0, reports: {} },
    { revision: 12, states: [report("message", 1)] },
  );
  const afterLateQuery = replaceDeliverySnapshot(
    newer,
    { revision: 11, states: [report("message", 0)] },
  );
  assert.strictEqual(afterLateQuery, newer, "stale completions leave the accepted object untouched");

  assert.strictEqual(
    clearDeliverySnapshotAfterFailedQuery(newer, 11),
    newer,
    "a stale failed query cannot erase a newer event either",
  );
  assert.deepEqual(clearDeliverySnapshotAfterFailedQuery(newer, 12), {
    revision: 12,
    reports: {},
  }, "an unavailable current actor clears rows but keeps the anti-stale watermark");
});
