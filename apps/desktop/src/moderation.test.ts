import assert from "node:assert/strict";
import test from "node:test";
import {
  buildModerationGraph,
  buildModerationTimeline,
  filterModerationTimeline,
  openKickCases,
  selectTimelineRows,
  voteTally,
  warningMap,
  type ModerationEvent,
} from "./moderation.ts";

const event = (part: Partial<ModerationEvent> = {}): ModerationEvent => ({
  id: "a1",
  kind: "warning",
  actor: "owner",
  signer: "device",
  target: "member",
  channel: "7",
  message_id: "m1",
  message_text: "snapshot",
  message_ts: 10,
  reason: "rule",
  evidence_ids: [],
  case_id: "",
  outcome: "",
  ts: 11,
  signature_valid: true,
  authorized: true,
  ...part,
});

test("chat warnings require a valid signature and authorization", () => {
  const warnings = warningMap([
    event({ id: "bad", signature_valid: false, ts: 20 }),
    event({ id: "old", ts: 12 }),
    event({ id: "new", reason: "newest", ts: 13 }),
    event({ id: "demoted", authorized: false, ts: 30 }),
  ]);
  assert.equal(warnings.size, 1);
  assert.equal(warnings.get("7:m1")?.reason, "newest");
});

test("timeline is stable across messages and moderation events", () => {
  const rows = buildModerationTimeline(
    [{ id: "m", author: "x", text: "hello", ts: 20, channel: "7", channelName: "general" }],
    [event({ ts: 10 })],
  );
  assert.deepEqual(rows.map((row) => row.kind), ["event", "message"]);
});

test("the user filter includes their messages and events involving them", () => {
  const rows = buildModerationTimeline(
    [
      { id: "m1", author: "alice", text: "hello", ts: 10, channel: "7", channelName: "general" },
      { id: "m2", author: "bob", text: "hi", ts: 11, channel: "7", channelName: "general" },
    ],
    [
      event({ id: "warn-bob", actor: "mod", target: "bob", ts: 12 }),
      event({ id: "warn-cat", actor: "mod", target: "cat", ts: 13 }),
    ],
  );
  assert.deepEqual(
    filterModerationTimeline(rows, "bob").map((row) => row.key),
    ["m:7:m2", "e:warn-bob"],
  );
});

test("the graph uses stable identity lanes and connects moderator to subject", () => {
  const rows = buildModerationTimeline(
    [{ id: "m1", author: "bob", text: "hello", ts: 10, channel: "7", channelName: "general" }],
    [event({ id: "warn-bob", actor: "mod", target: "bob", ts: 20 })],
  );
  const graph = buildModerationGraph(rows, 600);
  assert.deepEqual(graph.lanes.map((lane) => lane.identity), ["bob", "mod"]);
  assert.equal(graph.nodes[0].y, graph.lanes[0].y);
  assert.equal(graph.nodes[1].y, graph.lanes[0].y);
  assert.equal(graph.nodes[1].fromY, graph.lanes[1].y);
  assert.ok(graph.nodes[1].x > graph.nodes[0].x);
});

test("shift selection fills the inclusive range", () => {
  const first = selectTimelineRows(["a", "b", "c", "d"], new Set(), "b", "", false);
  const ranged = selectTimelineRows(["a", "b", "c", "d"], first.selected, "d", first.anchor, true);
  assert.deepEqual([...ranged.selected], ["b", "c", "d"]);
});

test("resolved cases close and signed identities get one latest vote", () => {
  const kick = event({ id: "c1", kind: "kick_case", message_id: "", channel: "", reason: "case" });
  assert.deepEqual(openKickCases([kick]).map((item) => item.id), ["c1"]);
  const resolution = event({ id: "f1", kind: "case_resolution", case_id: "c1", message_id: "", channel: "", outcome: "dismissed" });
  assert.deepEqual(openKickCases([kick, resolution]), []);
  assert.deepEqual(voteTally([
    { case_id: "c1", voter: "a", signer: "a1", yes: false, ts: 1, signature_valid: true, eligible: true },
    { case_id: "c1", voter: "a", signer: "a2", yes: true, ts: 2, signature_valid: true, eligible: true },
    { case_id: "c1", voter: "b", signer: "b1", yes: true, ts: 3, signature_valid: false, eligible: true },
    { case_id: "c1", voter: "departed", signer: "d1", yes: true, ts: 4, signature_valid: true, eligible: false },
  ], "c1"), { yes: 1, no: 0 });
});
