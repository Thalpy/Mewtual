import assert from "node:assert/strict";
import test from "node:test";
import { voiceAppNative, windowVoiceAppCalls } from "./voice-app-native.ts";
import { VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker, type VoiceCryptoAdmission, type VoiceIpcNative } from "./voice-crypto.ts";
import { VOICE_APP_SESSION_BYTES } from "./voice-app-session.ts";
import { voiceJoinKeyNative } from "./voice-join-native.ts";
import { voiceIpcNative } from "./voice-ipc-native.ts";
const hash = (n: number) => n.toString(16).padStart(64, "0");
const controller = "voice-0000000000000001", owner = "voice-0000000000000002";
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const until = performance.now() + 15_000;
  while (!check()) { if (performance.now() >= until) throw new Error("condition did not settle"); await new Promise<void>(yes => setImmediate(yes)); }
}
const unavailable: VoiceCryptoAdmission = { async admit() { throw new Error("not called"); }, async settle() {}, async commitInbound() {} };

test("fixed native metadata adapter propagates original calls and uncertainty without retry", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const native = voiceAppNative(async <T>(command: string, args: Record<string, unknown>) => {
    calls.push({ command, args }); throw new Error("reply lost");
  });
  assert.ok(Object.isFrozen(native));
  await assert.rejects(native.prepareController(), /reply lost/);
  await assert.rejects(native.prepareOwner(controller, 1, "2"), /reply lost/);
  await assert.rejects(native.activate(owner), /reply lost/);
  await assert.rejects(native.end(owner), /reply lost/);
  assert.deepEqual(calls, [
    { command: "prepare_voice_controller", args: {} },
    { command: "prepare_voice_owner", args: { controller, server: 1, room: "2" } },
    { command: "activate_voice_owner", args: { candidate: owner } },
    { command: "end_voice_owner", args: { owner } },
  ]);
});

test("real broker composition retains one window transport and joins held native setup after End", async () => {
  const host = {}, held = gate(), events: string[] = [];
  const invoke = async <T>(command: string, args: Record<string, unknown>): Promise<T> => {
    events.push(command);
    let result: unknown;
    switch (command) {
      case "prepare_voice_controller": result = controller; break;
      case "prepare_voice_owner": result = owner; break;
      case "activate_voice_owner": result = { owner: args.candidate,
        group: args.candidate === controller ? null : hash(1), join: args.candidate === controller ? null : hash(2) }; break;
      case "admit_voice_join_key": result = { owner, executor: args.executor, permit: "original-permit", join: hash(2), kind: "admitted" }; break;
      case "publish_voice_join_key":
        assert.match(args.publicKey as string, /^04[0-9a-f]{128}$/); // Public selected point only.
        result = { owner, executor: args.executor, permit: args.permit, join: hash(2), keyRef: hash(3) }; break;
      case "setup_voice_channel": await held.promise; result = { owner, executor: args.executor, kind: "pending" }; break;
      case "settle_voice_join_key": case "end_voice_owner": case "settle_voice_channel_setup": case "collect_voice_channel": break;
      default: assert.fail("unexpected ordinary IPC command " + command);
    }
    return result as T;
  };
  const calls = windowVoiceAppCalls(invoke, host), broker = windowVoiceCryptoBroker(unavailable, host);
  assert.equal(broker.counts().bytes, VOICE_APP_SESSION_BYTES);
  const remountUrl = "./voice-app-native.ts?remount=production";
  const remounted = await import(remountUrl);
  assert.equal(remounted.windowVoiceAppCalls(invoke, host), calls);
  assert.throws(() => windowVoiceAppCalls(async <T>() => undefined as T, host), /reload required/);
  const call = calls.prepare(1, "2"), started = call.start();
  await until(() => events.includes("setup_voice_channel"));
  const close = call.close(); let returned = false; void close.then(() => { returned = true; });
  await until(() => events.includes("end_voice_owner"));
  assert.equal(returned, false); assert.ok(broker.counts().bytes > VOICE_APP_SESSION_BYTES);
  held.release(); assert.equal(await started, "stopped"); await close;
  assert.deepEqual(broker.counts(), { active: 0, queued: 0, bytes: VOICE_APP_SESSION_BYTES });
  assert.equal(events.filter(command => command === "publish_voice_join_key").length, 1);
  assert.ok(events.includes("settle_voice_channel_setup")); assert.ok(events.includes("collect_voice_channel"));
  assert.deepEqual(Object.keys(calls).sort(), ["close", "prepare", "resume"]);
});

test("the SAME fixed reservation binds group before key admission and does not spend full group capacity", async () => {
  const host = {}, broker = windowVoiceCryptoBroker(unavailable, host), commands: string[] = [];
  const occupants = Array.from({ length: 64 }, (_, n) => createVoiceCryptoOwner("occupant-" + n, hash(1), () => true));
  const keyNative = voiceJoinKeyNative(async <T>() => { assert.fail("unstarted keys cannot invoke"); return undefined as T; });
  for (const original of occupants) broker.selectJoinKey(original, keyNative);
  assert.equal(broker.counts().bytes, 256 * 1024);
  const invoke = async <T>(command: string, args: Record<string, unknown>): Promise<T> => {
    commands.push(command);
    if (command === "prepare_voice_controller") return controller as T;
    if (command === "prepare_voice_owner") return owner as T;
    if (command === "activate_voice_owner") return { owner: args.candidate,
      group: args.candidate === controller ? null : hash(1), join: args.candidate === controller ? null : hash(2) } as T;
    assert.equal(command, "end_voice_owner"); return undefined as T;
  };
  const calls = windowVoiceAppCalls(invoke, host);
  assert.equal(broker.counts().bytes, 256 * 1024 + VOICE_APP_SESSION_BYTES);
  const call = calls.prepare(1, "2"); assert.equal(await call.start(), "unavailable"); await call.close();
  assert.equal(commands.includes("admit_voice_join_key"), false);
  await Promise.all(occupants.map(original => broker.retireNativeOwner(original)));
  assert.deepEqual(broker.counts(), { active: 0, queued: 0, bytes: VOICE_APP_SESSION_BYTES });
});

test("process capacity is reserved before a façade is constructed or an invoke can begin", async () => {
  const held = gate();
  const broker = new VoiceCryptoBroker({ async admit() { return "token"; }, async settle() {}, async commitInbound() {} });
  const work = Array.from({ length: 8 }, (_, n) => broker.run(createVoiceCryptoOwner("load-" + n, hash(n + 1), () => true), "generate", 256 * 1024, () => held.promise));
  const invoke = async <T>() => { assert.fail("capacity refusal must precede invoke"); return undefined as T; };
  assert.throws(() => broker.nativeAppCalls(voiceAppNative(invoke), voiceJoinKeyNative(invoke), voiceIpcNative(invoke)), /application capacity/);
  assert.equal(broker.counts().bytes, 2 * 1024 * 1024);
  held.release(); await Promise.all(work); await until(() => broker.counts().active === 0);
  assert.equal(broker.counts().bytes, 0);
});

test("unknown or incompatible retained application sessions fail without replacing work", () => {
  const invoke = async <T>() => undefined as T;
  for (const value of [undefined, null, 1, {}, { version: 0 }, Object.freeze({ version: 1, invoke, calls: null })]) {
    const host = { [Symbol.for("mewtual.voice.app-session.v1")]: value };
    assert.throws(() => windowVoiceAppCalls(invoke, host), /reload required/);
  }
});

test("owner join finds a published channel even when constructor subscription throws before returning its handle", async () => {
  const broker = new VoiceCryptoBroker(unavailable), held = gate();
  let subscriptions = 0, setups = 0, collections = 0;
  const original = createVoiceCryptoOwner(owner, hash(1), () => true);
  const selectedOwner = Object.freeze({ ...original, onRetired(listener: () => void) {
    const unsubscribe = original.onRetired(listener);
    if (++subscriptions === 2) {
      const retained = (broker as unknown as { ipcChannels: Map<string, { handle: { status(): Promise<unknown> } }> }).ipcChannels.get(owner)!;
      void retained.handle.status(); // Real owner already published; its driver can start.
      throw new Error("subscription failed after registration");
    }
    return unsubscribe;
  } });
  const keyNative = voiceJoinKeyNative(async <T>(command: string, args: Record<string, unknown>) => {
    if (command === "admit_voice_join_key") return { owner, executor: args.executor, permit: "permit", join: hash(2), kind: "admitted" } as T;
    if (command === "publish_voice_join_key") return { owner, executor: args.executor, permit: args.permit, join: hash(2), keyRef: hash(3) } as T;
    assert.equal(command, "settle_voice_join_key"); return undefined as T;
  });
  const selected = await broker.selectJoinKey(selectedOwner, keyNative).attempt(); assert.equal(selected.kind, "accepted");
  if (selected.kind !== "accepted") assert.fail();
  await until(() => broker.counts().active === 0);
  const native: VoiceIpcNative = {
    async setup(owner, executor) { setups++; await held.promise; return { owner, executor, kind: "pending" }; },
    async settleSetup() {}, async collectChannel() { collections++; }, async readWork() { assert.fail(); },
  };
  assert.throws(() => broker.nativeChannel(selectedOwner, selected.key, native), /already started/);
  await until(() => setups === 1);
  const close = broker.retireNativeOwner(selectedOwner); let returned = false; void close.then(() => { returned = true; });
  await new Promise<void>(yes => setImmediate(yes)); assert.equal(returned, false); assert.ok(broker.counts().bytes > 0);
  held.release(); await close;
  assert.equal(collections, 1); assert.deepEqual(broker.counts(), { active: 0, queued: 0, bytes: 0 });
});
