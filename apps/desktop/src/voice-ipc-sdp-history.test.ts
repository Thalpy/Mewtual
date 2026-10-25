import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { decodeVoiceSdpHistory } from "./voice-ipc-sdp-history.ts";
import { decodeVoiceIpcActionReply } from "./voice-ipc-replies.ts";
const vectors = JSON.parse(readFileSync(new URL("../src-tauri/src/voice_handoff/sdp-history-vectors.json", import.meta.url), "utf8"));
const bytes = (name: string): Uint8Array => new Uint8Array(Buffer.from(vectors.cases.find((v: {name: string}) => v.name === name).body, "hex"));
const expected = (purpose: "seal" | "open", row: string | null = vectors.row) => ({ parentRow: vectors.parent, row, purpose });
test("shared native SDP history vectors preserve pending publication, negative collection and exact u64 identity", () => {
  for (const item of vectors.cases) {
    const body = bytes(item.name), value = decodeVoiceSdpHistory(body, expected(item.purpose));
    assert.equal(value.row, vectors.row); assert.equal(value.parentRow, vectors.parent);
    assert.equal(value.localDrained, item.drained); assert.equal(value.settled, item.settled);
    assert.deepEqual(value.identity, item.action ? vectors.identity : null);
    assert.equal(value.receipt?.outcome.kind ?? null, item.outcome);
    assert.equal(value.delivery, ["none", "prepared", "admitted", "applied"][item.delivery]);
    assert.throws(() => decodeVoiceIpcActionReply(body, vectors.row), "no legacy decoder fallback");
  }
});
test("lost Begin history binds parent and purpose before accepting a returned row", () => {
  assert.equal(decodeVoiceSdpHistory(bytes("pending"), expected("open", null)).row, vectors.row);
  for (const target of [expected("seal"), { ...expected("open"), parentRow: vectors.row }, { ...expected("open"), row: vectors.parent },
    { ...expected("open"), parentRow: "0".repeat(32) }, { ...expected("open"), purpose: "other" }]) {
    assert.throws(() => decodeVoiceSdpHistory(bytes("pending"), target as ReturnType<typeof expected>));
  }
});
test("every truncation, extension, grammar tag and non-canonical flag refuses", () => {
  const body = bytes("opened-applied");
  for (let end = 0; end < body.length; end++) assert.throws(() => decodeVoiceSdpHistory(body.subarray(0, end), expected("open")));
  assert.throws(() => decodeVoiceSdpHistory(new Uint8Array([...body, 0]), expected("open")));
  for (const [offset, value] of [[0,1],[1,1],[34,2],[35,5],[36,2],[37,2],[86,2],[87,2],[120,0],[185,2],[186,2],[219,4]]) {
    const changed = body.slice(); changed[offset] = value;
    assert.throws(() => decodeVoiceSdpHistory(changed, expected("open")), `offset ${offset}`);
  }
});
test("unused fields, invented settlement/delivery/ACK and wrong terminal purpose refuse", () => {
  for (const [name, offset, value] of [["pending",38,1],["pending",86,1],["pending",186,1],["pending",219,1],
    ["negative-collected",86,1],["opened-applied",121,1],["opened-applied",219,0],["opened-applied",186,0],
    ["rejected-collected",122,1],["rejected-collected",121,12],["rejected-collected",219,1]] as const) {
    const changed = bytes(name); changed[offset] = value;
    assert.throws(() => decodeVoiceSdpHistory(changed, expected("open")), `${name}/${offset}`);
  }
  const sealed = bytes("sealed-collected");sealed[34]=1;
  assert.throws(() => decodeVoiceSdpHistory(sealed, expected("open")));
});
test("decoded history retains no response backing or mutable caller graph", () => {
  const body = bytes("opened-applied"), target = expected("open");
  const value = decodeVoiceSdpHistory(body, Object.assign(target, { unrelated: new Uint8Array(1024) }));
  body.fill(0);target.parentRow="0".repeat(32);
  assert.equal(value.parentRow, vectors.parent);assert.deepEqual(value.identity, vectors.identity);
  assert.ok(Object.isFrozen(value)); assert.ok(Object.isFrozen(value.identity)); assert.ok(Object.isFrozen(value.receipt));
  assert.equal("unrelated" in value, false);
});

test("expectation getters are read once before validation and never substitute caller data later", () => {
  const counts = { parentRow: 0, row: 0, purpose: 0 }, graph = { unrelated: new Uint8Array(1024 * 1024) };
  const target = {
    get parentRow() { return ++counts.parentRow === 1 ? vectors.parent : graph; },
    get row() { return ++counts.row === 1 ? vectors.row : null; },
    get purpose() { return ++counts.purpose === 1 ? "open" : graph; },
  };
  const value = decodeVoiceSdpHistory(bytes("opened-applied"), target as unknown as ReturnType<typeof expected>);
  assert.deepEqual(counts, { parentRow: 1, row: 1, purpose: 1 });
  assert.equal(value.purpose, "open"); assert.equal(value.receipt?.outcome.kind, "opened");
  assert.equal(value.row, vectors.row); assert.equal(value.parentRow, vectors.parent);
  assert.equal(Object.values(value).includes(graph), false);
  assert.throws(() => decodeVoiceSdpHistory(bytes("opened-applied"), {
    ...expected("open"), get purpose() { return graph; },
  } as unknown as ReturnType<typeof expected>));
});
