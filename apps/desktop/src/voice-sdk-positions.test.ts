import assert from "node:assert/strict";
import test from "node:test";
import { VoiceSdkPositions } from "./voice-sdk-positions.ts";

test("one SDK position transfers FIFO and cancellation only removes unstarted work", async () => {
  let available = false, active = 0, peak = 0;
  const positions = new VoiceSdkPositions(() => {
    if (!available) return null;
    active++; peak = Math.max(peak, active);
    return () => { active--; };
  });
  const a = positions.request(), cancelled = positions.request(), b = positions.request();
  cancelled.cancel(); assert.equal(await cancelled.promise, null);
  available = true; positions.pump();
  const first = await a.promise; assert.ok(first); a.cancel(); assert.equal(active, 1);
  let secondStarted = false; void b.promise.then(() => { secondStarted = true; });
  await Promise.resolve(); assert.equal(secondStarted, false);
  first(); first(); const second = await b.promise; assert.ok(second);
  assert.equal(active, 1); assert.equal(peak, 1); second(); assert.equal(active, 0);
});

test("eighteen paid waiters are bounded and cancelling the head preserves later work", async () => {
  let available = false;
  const positions = new VoiceSdkPositions(() => available ? () => {} : null);
  const requests = Array.from({ length: 18 }, () => positions.request());
  assert.throws(() => positions.request(), /queue full/);
  requests[0].cancel(); assert.equal(await requests[0].promise, null);
  available = true; positions.pump();
  for (const request of requests.slice(1)) { const release = await request.promise; assert.ok(release); release(); }
});

test("position return publishes release before granting a reentrant waiter", async () => {
  let active = 0;
  const positions = new VoiceSdkPositions(() => {
    active++; assert.equal(active, 1);
    return () => { active--; positions.pump(); };
  });
  const a = positions.request(), b = positions.request();
  (await a.promise)!(); (await b.promise)!(); assert.equal(active, 0);
});
