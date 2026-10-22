import assert from "node:assert/strict";
import test from "node:test";
import { voiceIpcNative, windowVoiceIpcNative } from "./voice-ipc-native.ts";

test("production metadata adapter exposes only original owner/executor and three fixed commands", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const reply = Object.freeze({ kind: "ready", owner: "owner", executor: "executor", port: 43210, transcript: "public" });
  const native = voiceIpcNative(async <T>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args }); return (command === "setup_voice_channel" ? reply : undefined) as T;
  });
  assert.ok(Object.isFrozen(native));
  assert.equal(await native.setup("owner", "executor"), reply);
  await native.settleSetup("owner", "executor");
  await native.collectChannel("owner", "executor");
  assert.deepEqual(calls, ["setup_voice_channel", "settle_voice_channel_setup", "collect_voice_channel"].map(command => ({
    command, args: { owner: "owner", executor: "executor" },
  })));
});

test("lost metadata or cleanup replies propagate without implicit retries or invented drain success", async () => {
  let calls = 0;
  const failed = new Error("reply lost");
  const native = voiceIpcNative(async () => { calls++; throw failed; });
  await assert.rejects(native.setup("owner", "executor"), error => error === failed);
  await assert.rejects(native.settleSetup("owner", "executor"), error => error === failed);
  await assert.rejects(native.collectChannel("owner", "executor"), error => error === failed);
  assert.equal(calls, 3);
});

test("actual adapter construction survives module remount with exact retained invoke and object", async () => {
  const host = {};
  const invoke = async <T>() => undefined as T;
  const native = windowVoiceIpcNative(invoke, host);
  const remounted = await import("./voice-ipc-native.ts?remount=adapter");
  assert.equal(remounted.windowVoiceIpcNative(invoke, host), native);
  assert.equal(windowVoiceIpcNative(invoke, host), native);
  assert.throws(() => remounted.windowVoiceIpcNative(async <T>() => undefined as T, host), /reload required/);
  assert.equal(windowVoiceIpcNative(invoke, host), native, "refusal preserves original adapter");
});

test("malformed or incompatible retained adapters refuse rather than create a second transport", () => {
  const invoke = async <T>() => undefined as T;
  for (const previous of [undefined, null, 1, { version: 0 }, Object.freeze({ version: 1, invoke, native: undefined })]) {
    const host = { [Symbol.for("mewtual.voice.ipc-native.v1")]: previous };
    assert.throws(() => windowVoiceIpcNative(invoke, host), /reload required/);
  }
});
