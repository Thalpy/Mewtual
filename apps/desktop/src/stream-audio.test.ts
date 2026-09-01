import assert from "node:assert/strict";
import test from "node:test";
import { disposeStreamAudioGraph } from "./stream-audio.ts";

test("disposing an idle stream mixer stops its output track, disconnects and closes", async () => {
  const state = { trackStopped: false, masterDisconnected: false, contextClosed: false };
  disposeStreamAudioGraph(
    { stream: { getTracks: () => [{ stop: () => { state.trackStopped = true; } }] } },
    { disconnect: () => { state.masterDisconnected = true; } },
    { close: async () => { state.contextClosed = true; } },
  );
  await Promise.resolve();
  assert.deepEqual(state, {
    trackStopped: true,
    masterDisconnected: true,
    contextClosed: true,
  });
});
