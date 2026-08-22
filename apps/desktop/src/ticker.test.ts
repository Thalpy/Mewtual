import { test } from "node:test";
import assert from "node:assert/strict";

import { acceptTickerReceipt, messageTickerId } from "./ticker.ts";

test("a ticker id is accepted once even after its visual queue item is gone", () => {
  const first = acceptTickerReceipt(new Set(), "status:4:post-a");
  assert.ok(first);
  // There is deliberately no queue argument: queue expiry must not make the receipt reusable.
  assert.equal(acceptTickerReceipt(first, "status:4:post-a"), null);
  assert.equal(acceptTickerReceipt(first, ""), null);
  assert.ok(acceptTickerReceipt(first, "status:4:post-b"));
});

test("message ticker ids are stable and server/channel scoped", () => {
  assert.equal(messageTickerId(7, "general", "m1"), "message:7:general:m1");
  assert.notEqual(messageTickerId(7, "general", "m1"), messageTickerId(8, "general", "m1"));
  assert.notEqual(messageTickerId(7, "general", "m1"), messageTickerId(7, "random", "m1"));
});
