import { test } from "node:test";
import assert from "node:assert/strict";

import { MAX_START_S, youtubeEmbedUrl, youtubeLabel, youtubePageUrl, youtubeRef } from "./youtube.ts";

const ID = "dQw4w9WgXcQ";

test("every spelling that circulates names the same video", () => {
  for (const link of [
    `https://www.youtube.com/watch?v=${ID}`,
    `https://youtube.com/watch?v=${ID}`,
    `https://m.youtube.com/watch?v=${ID}`,
    `https://music.youtube.com/watch?v=${ID}`,
    `https://youtu.be/${ID}`,
    `https://www.youtube.com/shorts/${ID}`,
    `https://www.youtube.com/live/${ID}`,
    `https://www.youtube.com/v/${ID}`,
    `https://www.youtube-nocookie.com/embed/${ID}`,
    `https://www.youtube.com/watch?app=desktop&v=${ID}&feature=share`,
  ]) {
    assert.deepEqual(youtubeRef(link), { id: ID, start: 0 }, link);
  }
});

test("a timestamp survives in both spellings the share dialog produces", () => {
  assert.equal(youtubeRef(`https://youtu.be/${ID}?t=90`)?.start, 90);
  assert.equal(youtubeRef(`https://www.youtube.com/watch?v=${ID}&t=90s`)?.start, 90);
  assert.equal(youtubeRef(`https://www.youtube.com/watch?v=${ID}&t=1m30s`)?.start, 90);
  assert.equal(youtubeRef(`https://www.youtube.com/watch?v=${ID}&t=1h2m3s`)?.start, 3723);
  assert.equal(youtubeRef(`https://www.youtube-nocookie.com/embed/${ID}?start=45`)?.start, 45);
});

test("a start offset somebody else supplied cannot become unbounded arithmetic", () => {
  // The offset is seeked to, and in the jukebox it is added to a shared clock, so it is somebody
  // else's number reaching this device's timing. Nothing past a day is a real timestamp.
  assert.equal(youtubeRef(`https://youtu.be/${ID}?t=9999999`)?.start, MAX_START_S);
  assert.equal(youtubeRef(`https://youtu.be/${ID}?t=999h`)?.start, MAX_START_S);
  assert.equal(youtubeRef(`https://youtu.be/${ID}?t=-30`)?.start, 0);
  assert.equal(youtubeRef(`https://youtu.be/${ID}?t=1e9`)?.start, 0, "not a duration, so no offset");
  assert.equal(youtubeRef(`https://youtu.be/${ID}?t=NaN`)?.start, 0);
  // A bad timestamp costs the viewer the timestamp, never the video.
  assert.equal(youtubeRef(`https://youtu.be/${ID}?t=garbage`)?.id, ID);
});

test("only YouTube's own hosts are YouTube", () => {
  for (const bad of [
    `https://youtube.com.evil.example/watch?v=${ID}`,
    `https://evil.example/watch?v=${ID}`,
    `https://notyoutube.com/watch?v=${ID}`,
    `https://youtu.be.evil.example/${ID}`,
    `https://evil.example/youtu.be/${ID}`,
  ]) {
    assert.equal(youtubeRef(bad), null, `${bad} must not be treated as YouTube`);
  }
});

test("an id is eleven characters of a fixed alphabet, and nothing else is one", () => {
  for (const bad of [
    "https://www.youtube.com/watch?v=short",
    "https://www.youtube.com/watch?v=waaaaaaytoolongforanid",
    "https://www.youtube.com/watch?v=has/a/slash",
    "https://www.youtube.com/watch?v=has.a.dots",
    `https://youtu.be/${ID}extra`,
    "https://www.youtube.com/watch",
    "https://www.youtube.com/",
    "https://www.youtube.com/playlist?list=PLabcdefghijklmnop",
  ]) {
    assert.equal(youtubeRef(bad), null, `${bad} must not yield a video`);
  }
});

test("a scheme that is not the web is refused outright", () => {
  for (const bad of [
    `javascript:alert(1)//youtube.com/watch?v=${ID}`,
    `data:text/html,youtube.com/watch?v=${ID}`,
    `file:///youtube.com/watch?v=${ID}`,
    "",
    "   ",
    "not a url",
  ]) {
    assert.equal(youtubeRef(bad), null);
  }
});

test("the frame address is built from validated parts on the no-cookie host", () => {
  const ref = youtubeRef(`https://www.youtube.com/watch?v=${ID}&t=30`)!;
  const url = new URL(youtubeEmbedUrl(ref));
  assert.equal(url.hostname, "www.youtube-nocookie.com", "the advertising cookie is withheld");
  assert.equal(url.pathname, `/embed/${ID}`);
  assert.equal(url.searchParams.get("start"), "30");
  assert.equal(url.searchParams.get("rel"), "0", "end cards stay in the same channel");
  // An uncontrolled card is somebody watching a video by themselves. A frame that answers
  // commands it will never be sent is a capability with no purpose.
  assert.equal(url.searchParams.get("enablejsapi"), null);
  assert.equal(url.searchParams.get("origin"), null);
  assert.equal(url.searchParams.get("autoplay"), null);
});

test("a deck frame is driveable and starts where the room is", () => {
  const ref = youtubeRef(`https://youtu.be/${ID}`)!;
  const url = new URL(youtubeEmbedUrl(ref, { controlled: true, origin: "http://tauri.localhost", start: 42 }));
  assert.equal(url.searchParams.get("enablejsapi"), "1");
  assert.equal(url.searchParams.get("origin"), "http://tauri.localhost", "YouTube checks this on every command");
  assert.equal(url.searchParams.get("start"), "42", "a joiner's frame starts level with the room");
  assert.equal(url.searchParams.get("controls"), "0", "the room's transport owns the deck, not one viewer");
  // The explicit `start` wins over whatever the shared link happened to say.
  const linked = youtubeRef(`https://youtu.be/${ID}?t=600`)!;
  assert.equal(new URL(youtubeEmbedUrl(linked, { controlled: true, start: 5 })).searchParams.get("start"), "5");
  assert.equal(new URL(youtubeEmbedUrl(linked, { controlled: true })).searchParams.get("start"), "600");
});

test("an origin the player would reject is left out rather than passed through", () => {
  // Tauri serves the window from `http://tauri.localhost` on Windows but from the custom scheme
  // `tauri://localhost` on macOS and Linux. Naming a non-web origin made the player refuse to
  // configure itself at all (Error 153) on exactly those platforms, which is worse than naming
  // none: commands are posted to a fixed target origin and replies checked against it either way.
  const ref = { id: ID, start: 0 };
  const windows = new URL(youtubeEmbedUrl(ref, { controlled: true, origin: "http://tauri.localhost" }));
  assert.equal(windows.searchParams.get("origin"), "http://tauri.localhost");

  for (const hostile of ["tauri://localhost", "file://", "", "not an origin", "https://a b"]) {
    const url = new URL(youtubeEmbedUrl(ref, { controlled: true, origin: hostile }));
    assert.equal(url.searchParams.get("origin"), null, `${hostile} must not be named`);
    assert.equal(url.searchParams.get("enablejsapi"), "1", "the frame is still driveable");
  }
});

test("the outbound link keeps the timestamp it was shared with", () => {
  const ref = youtubeRef(`https://youtu.be/${ID}?t=90`)!;
  assert.equal(youtubePageUrl(ref), `https://www.youtube.com/watch?v=${ID}&t=90`);
  assert.equal(youtubePageUrl({ id: ID, start: 0 }), `https://www.youtube.com/watch?v=${ID}`);
  assert.equal(youtubeLabel({ id: ID, start: 0 }), `YouTube video ${ID}`);
});
