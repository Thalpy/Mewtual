import assert from "node:assert/strict";
import test from "node:test";
import { voiceAppNative, windowVoiceAppCalls } from "./voice-app-native.ts";
import { VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker, type VoiceCryptoAdmission, type VoiceIpcNative } from "./voice-crypto.ts";
import { VOICE_APP_SESSION_BYTES, type VoiceAppCall } from "./voice-app-session.ts";
import { voiceJoinKeyNative } from "./voice-join-native.ts";
import { voiceIpcNative } from "./voice-ipc-native.ts";
import { VOICE_RTC_CONFIGURATION_BYTES, releaseVoiceRtcConfiguration, voiceRtcConfigurationCurrent,
  type VoiceRtcConfigurationLease } from "./voice-rtc-configuration.ts";
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

test("retained bundle2 refuses configuration upgrade without replacing its paid owner or cleanup", async () => {
  const invoke = async <T>() => { assert.fail("unstarted cleanup must not invoke"); return undefined as T; };
  const originalHost = {}, calls = windowVoiceAppCalls(invoke, originalHost);
  const broker = windowVoiceCryptoBroker(unavailable, originalHost), call = calls.prepare(1, "2");
  const slot = Symbol.for("mewtual.voice.app-session.v1"), brokerSlot = Symbol.for("mewtual.voice.crypto-broker.v2");
  const original = (originalHost as Record<symbol, object>)[slot];
  const bundle2 = Object.freeze({ ...original, version: 2 });
  const host = { [slot]: bundle2, [brokerSlot]: broker };
  const before = broker.counts();
  assert.throws(() => windowVoiceAppCalls(invoke, host), /reload required/);
  assert.equal(host[slot], bundle2); assert.equal(host[brokerSlot], broker);
  assert.deepEqual(broker.counts(), before, "refusal must keep the original fixed payment");
  await call.close(); assert.equal(call.snapshot().phase, "closed");
  assert.equal(broker.counts().bytes, VOICE_APP_SESSION_BYTES, "window graph remains paid after original cleanup");
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

test("retained application facade cannot bypass broker version or allocation checks during remount", async () => {
  const invoke = async <T>() => undefined as T;
  for (const change of ["version", "allocation", "deleted", "undefined", "null"] as const) {
    const host: Record<symbol, unknown> = {}, calls = windowVoiceAppCalls(invoke, host);
    const broker = windowVoiceCryptoBroker(unavailable, host), call = calls.prepare(1, "2");
    if (change === "version") Object.defineProperty(broker, "version", { value: 41 });
    else if (change === "allocation") host[Symbol.for("mewtual.voice.crypto-broker.v2")] = new VoiceCryptoBroker(unavailable);
    else if (change === "deleted") delete host[Symbol.for("mewtual.voice.crypto-broker.v2")];
    else host[Symbol.for("mewtual.voice.crypto-broker.v2")] = change === "null" ? null : undefined;
    const own = Object.hasOwn(host, Symbol.for("mewtual.voice.crypto-broker.v2"));
    const originalSlot = host[Symbol.for("mewtual.voice.crypto-broker.v2")];
    assert.throws(() => windowVoiceAppCalls(invoke, host), /reload required/);
    assert.equal(Object.hasOwn(host, Symbol.for("mewtual.voice.crypto-broker.v2")), own);
    assert.equal(host[Symbol.for("mewtual.voice.crypto-broker.v2")], originalSlot, "lookup refusal must never construct/publish another pool");
    assert.equal(broker.counts().bytes, VOICE_APP_SESSION_BYTES, "refusal keeps original reservation/graph");
    await call.close(); assert.equal(call.snapshot().phase, "closed");
  }
});

test("accessor-backed retained metadata cannot turn expected-allocation validation into construction", async () => {
  const invoke = async <T>() => undefined as T, originalHost = {};
  const calls = windowVoiceAppCalls(invoke, originalHost), broker = windowVoiceCryptoBroker(unavailable, originalHost);
  const call = calls.prepare(1, "2"); let reads = 0;
  const host = { [Symbol.for("mewtual.voice.app-session.v1")]: Object.freeze({ version: 3, invoke, calls, configurationSource: null,
    get broker() { return ++reads === 1 ? broker : undefined; } }) };
  assert.throws(() => windowVoiceAppCalls(invoke, host), /reload required/);
  assert.equal(reads, 1); assert.equal(Object.hasOwn(host, Symbol.for("mewtual.voice.crypto-broker.v2")), false);
  assert.equal(broker.counts().bytes, VOICE_APP_SESSION_BYTES); await call.close();
});

test("configured facade prepays both inventories and retains group payment through an actual held configuration loan", async t => {
  const host = {}, held = gate(), events: string[] = [], fee = VOICE_APP_SESSION_BYTES + VOICE_RTC_CONFIGURATION_BYTES;
  let broker!: VoiceCryptoBroker, captures = 0;
  const source = () => {
    captures++; events.push("device-settings"); assert.equal(broker.counts().bytes, fee);
    return { stun: "stun:device.example.test", turn: "turns:relay.example.test", username: "device-user", credential: "device-secret", policy: "relay" };
  };
  const invoke = async <T>(command: string, args: Record<string, unknown>): Promise<T> => {
    events.push(command);
    assert.equal(JSON.stringify(args).includes("device-secret"), false);
    assert.equal(JSON.stringify(args).includes("relay.example.test"), false, "ICE settings never enter ordinary IPC");
    let result: unknown;
    switch (command) {
      case "prepare_voice_controller": result = controller; break;
      case "prepare_voice_owner": result = owner; break;
      case "activate_voice_owner": result = { owner: args.candidate,
        group: args.candidate === controller ? null : hash(1), join: args.candidate === controller ? null : hash(2) }; break;
      case "admit_voice_join_key": result = { owner, executor: args.executor, permit: "original-permit", join: hash(2), kind: "admitted" }; break;
      case "publish_voice_join_key": result = { owner, executor: args.executor, permit: args.permit, join: hash(2), keyRef: hash(3) }; break;
      case "setup_voice_channel": await held.promise; result = { owner, executor: args.executor, kind: "pending" }; break;
      case "settle_voice_join_key": case "end_voice_owner": case "settle_voice_channel_setup": case "collect_voice_channel": break;
      default: assert.fail("unexpected ordinary IPC command " + command);
    }
    return result as T;
  };
  const calls = windowVoiceAppCalls(invoke, host, source); broker = windowVoiceCryptoBroker(unavailable, host);
  assert.equal(broker.counts().bytes, fee); assert.equal(captures, 0);
  assert.equal(windowVoiceAppCalls(invoke, host, source), calls);
  assert.throws(() => windowVoiceAppCalls(invoke, host, () => source()), /reload required/);
  assert.equal(broker.counts().bytes, fee);
  const call = calls.prepare(1, "2"); let loan: VoiceRtcConfigurationLease | null = null;
  t.after(async () => { held.release(); if (loan) releaseVoiceRtcConfiguration(loan); await call.close(); });
  const started = call.start();
  await until(() => events.includes("setup_voice_channel"));
  assert.equal(captures, 1); assert.ok(events.indexOf("device-settings") < events.indexOf("admit_voice_join_key"));
  // Retain an actual opaque loan from this exact privately selected App channel. No
  // production getter/test-selected identity is added to the public facade for this.
  const channel = (broker as unknown as { ipcChannels: Map<string, { custody: { rtcConfiguration(): VoiceRtcConfigurationLease | null } }> }).ipcChannels.get(owner)!;
  loan = channel.custody.rtcConfiguration()!; assert.ok(loan);
  const closing = call.close(); let returned = false; void closing.then(() => { returned = true; });
  assert.equal(voiceRtcConfigurationCurrent(loan), false);
  held.release(); assert.equal(await started, "stopped");
  await until(() => events.includes("collect_voice_channel") && broker.counts().bytes === fee);
  assert.equal(returned, false, "native cleanup alone cannot return the configuration group fee");
  const probe = createVoiceCryptoOwner("configuration-pressure", hash(1), () => true);
  const bytes = 256 * 1024 - fee + 1;
  await assert.rejects(broker.run(probe, "generate", bytes, async () => undefined), /capacity/);
  releaseVoiceRtcConfiguration(loan); loan = null; await closing;
  assert.equal(call.snapshot().phase, "closed"); assert.equal(returned, true);
  await assert.rejects(broker.run(probe, "generate", bytes, async () => undefined), /unavailable/);
  await until(() => broker.counts().active === 0);
  assert.deepEqual(broker.counts(), { active: 0, queued: 0, bytes: fee });
});

test("configured process/group refusal precedes its settings callback and all key work", async () => {
  const fee = VOICE_APP_SESSION_BYTES + VOICE_RTC_CONFIGURATION_BYTES;
  for (const pressure of ["process", "group"] as const) {
    const held = gate(), commands: string[] = []; let captures = 0;
    const broker = new VoiceCryptoBroker({ async admit() { return "token"; }, async settle() {}, async commitInbound() {} });
    const source = () => { captures++; assert.fail("configuration capture must follow capacity admission"); };
    const invoke = async <T>(command: string, args: Record<string, unknown>): Promise<T> => {
      commands.push(command);
      if (command === "prepare_voice_controller") return controller as T;
      if (command === "prepare_voice_owner") return owner as T;
      if (command === "activate_voice_owner") return { owner: args.candidate,
        group: args.candidate === controller ? null : hash(1), join: args.candidate === controller ? null : hash(2) } as T;
      assert.equal(command, "end_voice_owner"); return undefined as T;
    };
    if (pressure === "process") {
      const work = Array.from({ length: 8 }, (_, n) => broker.run(createVoiceCryptoOwner("load-" + n, hash(n + 1), () => true), "generate", 256 * 1024, () => held.promise));
      assert.throws(() => broker.nativeConfiguredAppCalls(voiceAppNative(invoke), voiceJoinKeyNative(invoke), voiceIpcNative(invoke), source), /capacity/);
      assert.equal(captures, 0); assert.equal(commands.length, 0);
      held.release(); await Promise.all(work); await until(() => broker.counts().active === 0);
      assert.equal(broker.counts().bytes, 0);
    } else {
      const occupants = Array.from({ length: 64 }, (_, n) => createVoiceCryptoOwner("occupant-" + n, hash(1), () => true));
      const keys = voiceJoinKeyNative(async <T>() => { assert.fail("no key admission"); return undefined as T; });
      for (const original of occupants) broker.selectJoinKey(original, keys);
      const calls = broker.nativeConfiguredAppCalls(voiceAppNative(invoke), keys, voiceIpcNative(invoke), source);
      const call = calls.prepare(1, "2"); assert.equal(await call.start(), "unavailable"); await call.close();
      assert.equal(captures, 0); assert.equal(commands.includes("admit_voice_join_key"), false);
      await Promise.all(occupants.map(original => broker.retireNativeOwner(original)));
      assert.equal(broker.counts().bytes, fee);
    }
  }
});

test("End from the last device-settings getter cannot publish a key or SDK and still releases its group payment", async () => {
  const host = {}, commands: string[] = []; let call!: VoiceAppCall;
  const invoke = async <T>(command: string, args: Record<string, unknown>): Promise<T> => {
    commands.push(command);
    if (command === "prepare_voice_controller") return controller as T;
    if (command === "prepare_voice_owner") return owner as T;
    if (command === "activate_voice_owner") return { owner: args.candidate,
      group: args.candidate === controller ? null : hash(1), join: args.candidate === controller ? null : hash(2) } as T;
    assert.equal(command, "end_voice_owner"); return undefined as T;
  };
  const source = () => ({ stun: "", turn: "", username: "", credential: "", get policy() { void call.close(); return "all"; } });
  const calls = windowVoiceAppCalls(invoke, host, source), broker = windowVoiceCryptoBroker(unavailable, host);
  call = calls.prepare(1, "2"); assert.equal(await call.start(), "stopped"); await call.close();
  assert.equal(commands.includes("admit_voice_join_key"), false);
  assert.equal(broker.counts().bytes, VOICE_APP_SESSION_BYTES + VOICE_RTC_CONFIGURATION_BYTES);
});

test("malformed configured sources never become an unconfigured direct call", () => {
  const invoke = async <T>() => { assert.fail("no native call"); return undefined as T; };
  for (const source of [0, false, "", {}, null, undefined]) {
    const broker = new VoiceCryptoBroker(unavailable);
    assert.throws(() => broker.nativeConfiguredAppCalls(voiceAppNative(invoke), voiceJoinKeyNative(invoke), voiceIpcNative(invoke), source as never), /settings source/);
    assert.equal(broker.counts().bytes, 0);
    if (source === null || source === undefined) continue; // Explicit optional unconfigured API.
    const host = {};
    assert.throws(() => windowVoiceAppCalls(invoke, host, source as never), /settings source/);
    assert.equal(Object.hasOwn(host, Symbol.for("mewtual.voice.crypto-broker.v2")), false);
  }
});
