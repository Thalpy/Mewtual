import { test } from "node:test";
import assert from "node:assert/strict";

import { acceptTickerReceipt, messageTickerId, TICKER_RECEIPT_MAX } from "./ticker.ts";

test("a ticker id is accepted once even after its visual queue item is gone", () => {
  const receipts = new Set<string>();
  assert.ok(acceptTickerReceipt(receipts, "status:4:post-a"));
  // There is deliberately no queue argument: queue expiry must not make the receipt reusable.
  assert.equal(acceptTickerReceipt(receipts, "status:4:post-a"), false);
  assert.equal(acceptTickerReceipt(receipts, ""), false);
  assert.ok(acceptTickerReceipt(receipts, "status:4:post-b"));
  assert.deepEqual([...receipts], ["status:4:post-a", "status:4:post-b"]);
});

test("receipts are bounded and evict the oldest acceptance first", () => {
  const receipts = new Set<string>();
  for (let i = 0; i < 5; i += 1) assert.ok(acceptTickerReceipt(receipts, `m${i}`, 3));
  assert.deepEqual([...receipts], ["m2", "m3", "m4"]);
  // An evicted id is eligible again: by then the row it named is long gone from every feed.
  assert.ok(acceptTickerReceipt(receipts, "m0", 3));
  assert.equal(acceptTickerReceipt(receipts, "m4", 3), false);
  assert.ok(TICKER_RECEIPT_MAX >= 1000, "the default bound outlives any replay window");
});

test("message ticker ids are stable and server/channel scoped", () => {
  assert.equal(messageTickerId(7, "general", "m1"), "message:7:general:m1");
  assert.notEqual(messageTickerId(7, "general", "m1"), messageTickerId(8, "general", "m1"));
  assert.notEqual(messageTickerId(7, "general", "m1"), messageTickerId(7, "random", "m1"));
});
