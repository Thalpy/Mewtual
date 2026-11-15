import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

// Execute the actual helper AND final harness case. Testing a detached retry
// loop would not catch a call site still relying on two void completions.
const source = readFileSync(new URL("../scripts/voice-native-interop.ts", import.meta.url), "utf8");
const helper = source.slice(source.indexOf("async function freshOriginalMediaCurrent("), source.indexOf("\ntype Step ="));
const start = source.indexOf('    case "media-current-all": {');
const branch = source.slice(start, source.indexOf('    case "end-peers": {', start));
assert.ok(helper.includes("for (let observation") && branch.includes("all-media-current"));
const compiled = ts.transpile(`${helper}\nreturn async () => { switch ("media-current-all") { ${branch} } };`, { target: ts.ScriptTarget.ES2022 });
type MediaRecord = { plan: object; currents: number; calls: { closed: number } };
function fixture(records: MediaRecord[], complete: (record: MediaRecord) => void, now = () => 0) {
  return new Function("assert", "broker", "owner", "channel", "mediaRecords", "performance", compiled)(
    assert, { async completeNativeMediaHandoff(_owner: object, _channel: object, plan: object) {
      const record = records.find(record => record.plan === plan);
      assert.ok(record, "retry must retain an exact original plan"); complete(record);
    } }, {}, {}, records, { now },
  ) as () => Promise<{ kind: string; peers: number }>;
}
const record = (): MediaRecord => ({ plan: {}, currents: 7, calls: { closed: 0 } });

test("all-peer Current survives repeated void/refused joins on both originals", async () => {
  const records = [record(), record()]; const counts = new Map(records.map(record => [record, 0]));
  const step = fixture(records, record => {
    const count = counts.get(record)! + 1; counts.set(record, count);
    if (count === (record === records[0] ? 8 : 5)) record.currents++;
  });
  assert.deepEqual(await step(), { kind: "all-media-current", peers: 2 });
  assert.deepEqual(records.map(record => record.currents), [8, 8]);
  assert.deepEqual([...counts.values()], [8, 5]);
});

test("void completion cannot prove authenticated Current and retries remain bounded", async () => {
  const original = record(); let calls = 0;
  await assert.rejects(fixture([original], () => { calls++; })(), /bounded original media observations/);
  assert.equal(calls, 33, "one old-flight join plus at most32 observations");
  assert.equal(original.currents, 7);
});

test("a retired original peer cannot pass by receiving an unrelated Current", async () => {
  const original = record(); let calls = 0;
  await assert.rejects(fixture([original], record => {
    calls++; record.calls.closed = 1; record.currents++;
  })(), /original media peer must remain live/);
  assert.equal(calls, 1);
});

test("Current observation keeps its existing five-second harness bound", async () => {
  const original = record(); let calls = 0, now = 0;
  await assert.rejects(fixture([original], () => { calls++; now += 1000; }, () => now)(),
    /same original media peer did not receive fresh authenticated Current/);
  assert.ok(calls < 33);
  assert.equal(original.currents, 7);
});
