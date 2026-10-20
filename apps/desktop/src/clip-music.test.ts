import assert from "node:assert/strict";
import test from "node:test";
import { songsFrom, studioMusic } from "./clip-music.ts";
import type { IndexEntryView } from "./studio-native.ts";

const entry = (id: string, kind: "score" | "flipnote", title: string, expiry: IndexEntryView["expiry"] = { kind: "never" }): IndexEntryView => ({
  id, kind, title, titleConflicts: 0, createdBy: "", ts: 0, expiry, expiryConflicts: 0, creations: 1, where: "visible",
});

test("only the channel's visible, unexpired songs are offered, with a title even when they have none", () => {
  const songs = songsFrom({
    entries: [
      entry("a", "score", "Night Drive"), entry("b", "flipnote", "Cat walk"), entry("c", "score", "   "),
      entry("d", "score", "Gone at noon", { kind: "at", ms: 1000 }), entry("e", "score", "Later", { kind: "at", ms: 5000 }),
    ],
  }, 2000);
  assert.deepEqual(songs, [{ id: "a", title: "Night Drive" }, { id: "c", title: "Untitled song" }, { id: "e", title: "Later" }]);
});

function fakeIpc(reply: unknown) {
  const calls: Array<[string, unknown]> = [];
  return {
    calls,
    ipc: { invoke: async <T,>(command: string, args?: unknown) => { calls.push([command, args]); return reply as T; }, listen: async () => () => {} },
  };
}

test("a channel whose Studio holds nothing yet has no songs, not an error", async () => {
  const { ipc, calls } = fakeIpc(null);
  const music = studioMusic(ipc as never, { server: 3, channel: "7" }, "f".repeat(64));
  assert.deepEqual(await music.list(), []);
  assert.deepEqual(calls, [["studio_list", { server: 3, channel: "7" }]]);
});

test("a song this device does not hold is refused with a sentence, through the Studio's own read", async () => {
  const { ipc, calls } = fakeIpc(null);
  const music = studioMusic(ipc as never, { server: 3, channel: "7" }, "f".repeat(64));
  await assert.rejects(music.load("a".repeat(32)), /not on this device/);
  assert.deepEqual(calls, [["studio_read", { server: 3, channel: "7", object: "a".repeat(32), objectKind: "score" }]]);
});
