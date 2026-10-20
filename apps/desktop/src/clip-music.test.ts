import assert from "node:assert/strict";
import test from "node:test";
import { checkSongsOffered, songKey, songOfKey, songsFrom, studioMusic, type ClipMusicSource } from "./clip-music.ts";
import type { IndexEntryView } from "./studio-native.ts";
import { indexContent, indexEntry, ordinaryView } from "./studio-testkit.ts";

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

/** Answers each command from `replies` (a command not listed answers null), recording every call. */
function fakeIpc(replies: Record<string, unknown>) {
  const calls: Array<[string, unknown]> = [];
  return {
    calls,
    ipc: { invoke: async <T,>(command: string, args?: unknown) => { calls.push([command, args]); return (replies[command] ?? null) as T; }, listen: async () => () => {} },
  };
}

const SONG = "a".repeat(32);
/** A channel index holding one song, expiring at `expiresAt` ms when given. */
const listing = (expiresAt?: number) => ordinaryView(indexContent({
  objects: { [SONG]: indexEntry({ kind: "score", title: "Night Drive", expiry: expiresAt === undefined ? { kind: "never" } : { kind: "at", ms: expiresAt } }) },
}));

test("a channel whose Studio holds nothing yet has no songs, not an error", async () => {
  const { ipc, calls } = fakeIpc({});
  const music = studioMusic(ipc as never, { server: 3, channel: "7" }, "f".repeat(64));
  assert.deepEqual(await music.list(), []);
  assert.deepEqual(calls, [["studio_list", { server: 3, channel: "7" }]]);
});

test("a song this device does not hold is refused with a sentence, through the Studio's own read", async () => {
  const { ipc, calls } = fakeIpc({ studio_list: listing() });
  const music = studioMusic(ipc as never, { server: 3, channel: "7" }, "f".repeat(64));
  await assert.rejects(music.load(SONG), /not on this device/);
  assert.deepEqual(calls, [
    ["studio_list", { server: 3, channel: "7" }],
    ["studio_read", { server: 3, channel: "7", object: SONG, objectKind: "score" }],
  ]);
});

test("a song that expired after the list was shown is refused at load, before its content is read", async () => {
  // Raised in review: the list is read once when the tab opens, and a song whose expiry passed
  // while the editor stayed open could still be rendered into a clip.
  let now = 1_000;
  const { ipc, calls } = fakeIpc({ studio_list: listing(5_000) });
  const music = studioMusic(ipc as never, { server: 3, channel: "7" }, "f".repeat(64), () => now);
  assert.deepEqual((await music.list()).map((s) => s.id), [SONG], "offered while it lasts");
  now = 5_000;
  await assert.rejects(music.load(SONG), /not here anymore/);
  assert.ok(!calls.some(([command]) => command === "studio_read"), "its content is never read");
});

test("a song's bin key names it, and no other key names a song", () => {
  assert.equal(songOfKey(songKey(SONG)), SONG);
  assert.equal(songOfKey("emoji:🐱"), null);
  assert.equal(songOfKey("sfx:pow"), null);
  assert.equal(songOfKey(undefined), null);
});

test("posting checks the clip's songs against the list again, and a list it cannot read refuses", async () => {
  // Raised in review: a song loaded while offered could expire with the editor open and still be
  // posted, since only the load was checked.
  let offered = [{ id: "s1", title: "One" }, { id: "s2", title: "Two" }];
  let lists = 0;
  const source: ClipMusicSource = { list: async () => { lists++; return offered; }, load: async () => ({ take: null, gridMs: 0 }) };
  await checkSongsOffered(source, []);
  assert.equal(lists, 0, "a clip with no songs asks nothing");
  await checkSongsOffered(source, ["s1", "s2"]);
  offered = [{ id: "s2", title: "Two" }];
  await assert.rejects(checkSongsOffered(source, ["s1", "s2"]), /not here anymore/);
  const broken: ClipMusicSource = { list: async () => { throw new Error("IPC: index epoch 0xdead not open"); }, load: source.load };
  await assert.rejects(checkSongsOffered(broken, ["s2"]), (e: Error) => /could not be checked/.test(e.message) && !/0xdead/.test(e.message));
  await assert.rejects(checkSongsOffered(null, ["s2"]), /could not be checked/);
});
