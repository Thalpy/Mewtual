import assert from "node:assert/strict";
import test from "node:test";
import { VISUAL_FIXTURE_NOW, visualFixtureResponse } from "./visual-fixture.ts";

test("visual fixture resumes into a deterministic unlocked server", () => {
  const servers = visualFixtureResponse("resume_session") as Array<{
    server: number;
    name: string;
    channels: Array<{ id: string; name: string }>;
  }>;

  assert.equal(servers[0]?.name, "Lantern Room");
  assert.deepEqual(servers[0]?.channels.map((channel) => channel.id), ["general", "design", "notes"]);
});

test("visual fixture returns isolated channel message copies", () => {
  const first = visualFixtureResponse("get_messages", { server: 1, channel: "general" }) as Array<{
    id: string;
    ts: number;
  }>;
  const second = visualFixtureResponse("get_messages", { server: 1, channel: "general" }) as Array<{
    id: string;
    ts: number;
  }>;

  assert.equal(first.at(-1)?.id, "msg-5");
  assert.equal(first.at(-1)?.ts, VISUAL_FIXTURE_NOW - 7 * 60_000);
  first.pop();
  assert.equal(second.length, 5);
});

test("visual fixture supplies every read used by the empty Wiki and lazy Help route", () => {
  assert.deepEqual(visualFixtureResponse("get_wiki_pages"), []);
  assert.deepEqual(visualFixtureResponse("get_wiki_map"), {});
  assert.deepEqual(visualFixtureResponse("get_wiki_meta"), {});
  assert.equal(visualFixtureResponse("get_wiki_review_days"), 0);
  assert.deepEqual(visualFixtureResponse("get_wiki_pending"), []);
});

test("visual fixture supplies the full degraded connectivity assistant state", () => {
  const report = visualFixtureResponse("get_connectivity") as {
    action: string;
    advertised: string[];
    upnp: string;
    steps: unknown[];
  };
  assert.equal(report.action, "found");
  assert.equal(report.advertised.length, 2);
  assert.match(report.upnp, /PCP unavailable/);
  assert.equal(report.steps.length, 2);
});

test("visual fixture exposes explicit standing switchboard consent and hosts", () => {
  const status = visualFixtureResponse("get_switchboard_status", { server: 1 }) as {
    offered: boolean;
    eligible: boolean;
    online: Array<{ fingerprint: string }>;
  };
  assert.equal(status.offered, false);
  assert.equal(status.eligible, true);
  assert.equal(status.online.length, 2);
});

test("visual fixture fails loudly for an unsupported native dependency", () => {
  assert.throws(
    () => visualFixtureResponse("new_native_command"),
    /does not implement Tauri command: new_native_command/,
  );
});
