import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { acceptCapture, chooseMicrophoneSender, MediaCaptureSession } from "./media-capture.ts";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

function fakeCapture() {
  const track = { stopped: false, stop() { this.stopped = true; } };
  return { track, stream: { getTracks: () => [track] } };
}

test("leaving before a permission prompt resolves stops the late capture", async () => {
  const session = new MediaCaptureSession();
  const lease = session.begin();
  const pending = deferred<ReturnType<typeof fakeCapture>["stream"]>();
  session.invalidate();
  const capture = fakeCapture();
  pending.resolve(capture.stream);
  assert.equal(acceptCapture(session, lease, await pending.promise, false), null);
  assert.equal(capture.track.stopped, true);
});

test("stopping before a permission prompt resolves stops the late capture", async () => {
  const session = new MediaCaptureSession();
  const lease = session.begin();
  const pending = deferred<ReturnType<typeof fakeCapture>["stream"]>();
  session.invalidate();
  const capture = fakeCapture();
  pending.resolve(capture.stream);
  assert.equal(acceptCapture(session, lease, await pending.promise, true), null);
  assert.equal(capture.track.stopped, true);
});

test("a competing camera or screen request supersedes the older chooser", async () => {
  const session = new MediaCaptureSession();
  const firstLease = session.begin();
  const secondLease = session.begin();
  const first = fakeCapture();
  const second = fakeCapture();
  assert.equal(acceptCapture(session, firstLease, first.stream, true), null);
  assert.equal(first.track.stopped, true);
  assert.equal(acceptCapture(session, secondLease, second.stream, true), second.stream);
  assert.equal(second.track.stopped, false);
});

test("microphone hot-swap never selects the mixed screen-audio sender", () => {
  const screenAudio = { track: { kind: "audio" } };
  const parkedMic = { track: null };
  assert.equal(
    chooseMicrophoneSender([screenAudio, parkedMic], null, null, screenAudio),
    parkedMic,
  );
  assert.equal(
    chooseMicrophoneSender([screenAudio], null, null, screenAudio),
    null,
  );
});

test("screen-audio adoption never waits on AudioContext resume before teardown owns the tracks", () => {
  const source = readFileSync(fileURLToPath(new URL("./App.svelte", import.meta.url)), "utf8");
  const start = source.indexOf("async function addScreenAudioStream(");
  const end = source.indexOf("function removeScreenAudioSource(", start);
  assert.ok(start >= 0 && end > start);
  const body = source.slice(start, end);
  assert.doesNotMatch(body, /await context\.resume\(\)/);
  assert.match(body, /void context\.resume\(\)\.catch/);
  assert.match(body, /screenAudioCaptures\.set\(id, \{ id, label, stream, node, gain \}\)/);
  assert.match(body, /node\.connect\(gain\);[\s\S]*gain\.connect\(master\)/);
  assert.match(source, /if \(!screenAudioCaptures\.size\) disposeScreenAudioGraph\(\)/);
  assert.match(source, /disposeStreamAudioGraph\(destination, master, context\)/);
  assert.match(source, /screenAudioSourceSlotAvailable\(screenAudioCaptures\.size\)/);
  assert.match(source, /MAX_STREAM_AUDIO_SOURCES/);
});
