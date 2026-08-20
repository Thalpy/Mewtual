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

test("visual fixture fails loudly for an unsupported native dependency", () => {
  assert.throws(
    () => visualFixtureResponse("new_native_command"),
    /does not implement Tauri command: new_native_command/,
  );
});
