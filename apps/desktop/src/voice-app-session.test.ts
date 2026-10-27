import assert from "node:assert/strict";
import test from "node:test";
import { VoiceAppSession, VOICE_APP_SESSION_BYTES, type VoiceAppNative, type VoiceAppSessionContext, type VoiceAppActivation } from "./voice-app-session.ts";
import { createVoiceCryptoOwner, type VoiceCryptoOwner, type VoiceIpcClock, type VoiceIpcStatus, type VoiceSelectedKeyOutcome } from "./voice-crypto.ts";

const hash = (n: number) => n.toString(16).padStart(64, "0");
const id = (n: number) => "voice-" + n.toString(16).padStart(16, "0");
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { promise, resolve }; }
const flush = async () => { for (let n = 0; n < 40; n++) await Promise.resolve(); };
function fixture() {
  let now = 0, serial = 0, ids = 0, bound = false, refunded = false, selectCalls = 0, observeCalls = 0, retireCalls = 0, resumes = 0;
  const events: string[] = [], timers = new Map<number, { at: number; work(): void }>();
  const holds = new Map<string, ReturnType<typeof deferred<void>>>(), after = new Set<string>();
  const candidates = new Map<string, { controller: string | null; active: boolean }>();
  const failures = new Map<string, number>();
  let owner: VoiceCryptoOwner | null = null, groupRefusal = false, channelFailure = false;
  const cleanup = deferred<void>(); cleanup.resolve(); let actualCleanup = cleanup.promise;
  const clock: VoiceIpcClock = { now: () => now,
    schedule(work, delay) { const token = ++serial; timers.set(token, { at: now + delay, work }); return token; },
    cancel(token) { timers.delete(token as number); } };
  async function invoke<T>(stage: string, effect: () => T): Promise<T> {
    events.push(stage);
    const hold = holds.get(stage);
    if (hold && !after.has(stage)) await hold.promise;
    const result = effect();
    if (hold && after.has(stage)) await hold.promise;
    const fail = failures.get(stage) ?? 0;
    if (fail) { failures.set(stage, fail - 1); throw new Error("native reply lost"); }
    return result;
  }
  const native: { -readonly [K in keyof VoiceAppNative]: VoiceAppNative[K] } = {
    prepareController: () => invoke("prepare-controller", () => {
      const value = id(++ids); candidates.set(value, { controller: null, active: false }); return value;
    }),
    prepareOwner: (controller, server, room) => invoke("prepare-owner", () => {
      assert.equal(server, 1); assert.equal(room, "2");
      if (!candidates.get(controller)?.active) throw new Error("controller stopped");
      const value = id(++ids); candidates.set(value, { controller, active: false }); return value;
    }),
    activate: candidate => invoke(candidate === id(1) ? "activate-controller" : "activate-owner", () => {
      const value = candidates.get(candidate);
      if (!value || (value.controller && !candidates.get(value.controller)?.active)) throw new Error("candidate stopped");
      value.active = true;
      return { owner: candidate, group: value.controller ? hash(1) : null, join: value.controller ? hash(2) : null };
    }),
    end: candidate => invoke("end-" + candidate, () => {
      for (const [key, value] of candidates) if (key === candidate || value.controller === candidate) candidates.delete(key);
    }),
  };
  const key = Object.freeze({ publicKey: () => new Uint8Array(65) });
  const context: VoiceAppSessionContext = {
    clock,
    bindGroup(group) { assert.equal(group, hash(1)); if (groupRefusal) throw new Error("capacity"); assert.equal(bound, false); bound = true; },
    unbindGroup() { refunded = true; bound = false; },
    owner(token, group, current) { owner = createVoiceCryptoOwner(token, group, current); return owner; },
    select(original) {
      selectCalls++; assert.equal(original, owner); assert.equal(bound, true);
      return { async attempt() {
        const reply = deferred<VoiceSelectedKeyOutcome>();
        const unsubscribe = original.onRetired(() => reply.resolve({ kind: "retired" }));
        void invoke("select", () => ({ kind: "accepted" as const, key })).then(reply.resolve);
        try { return await reply.promise; } finally { unsubscribe(); }
      } };
    },
    channel(original) {
      assert.equal(original, owner); events.push("channel");
      if (channelFailure) throw new Error("subscription refused after internal publication");
      return { status: () => invoke("status", () => ({ kind: "ready", nativeSetupDrained: true } as VoiceIpcStatus)), close: () => actualCleanup };
    },
    observe(original) { assert.equal(original, owner); observeCalls++; return { close: async () => {} }; },
    retire(original) { assert.equal(original, owner); retireCalls++; original.retire(); return actualCleanup; },
    resume() { resumes++; },
  };
  const calls = new VoiceAppSession(native, context).handle;
  return { calls, native, context, events, timers, candidates,
    get bound() { return bound; }, get refunded() { return refunded; }, get selectCalls() { return selectCalls; },
    get observeCalls() { return observeCalls; }, get retireCalls() { return retireCalls; }, get resumes() { return resumes; },
    hold(stage: string, committed = false) { const gate = deferred<void>(); holds.set(stage, gate); if (committed) after.add(stage); return gate; },
    lose(stage: string) { failures.set(stage, 1); },
    holdCleanup() { const gate = deferred<void>(); actualCleanup = gate.promise; return gate; },
    refuseGroup() { groupRefusal = true; }, failChannel() { channelFailure = true; },
    async advance(ms = 250) { now += ms; for (const [token, timer] of [...timers]) if (timer.at <= now) { timers.delete(token); timer.work(); } await flush(); },
  };
}

test("the separately reserved metadata inventory includes overlapping RPC, tasks and cadence", () => {
  assert.equal(1280 + 1024 + 1024 + 1024 + 1024 + 2048 + 768, VOICE_APP_SESSION_BYTES);
});
test("one retained original call reaches actual channel Ready before lifecycle observation", async () => {
  const f = fixture(), call = f.calls.prepare(1, "2");
  assert.equal(f.calls.prepare(1, "2"), call); assert.equal(f.events.length, 0);
  const started = call.start(); assert.equal(call.start(), started);
  assert.equal(await started, "ready"); assert.deepEqual(call.snapshot(), { phase: "ready" });
  assert.equal(f.selectCalls, 1); assert.equal(f.observeCalls, 1);
  assert.deepEqual(f.events.slice(0, 7), ["prepare-controller", "activate-controller", "prepare-owner", "activate-owner", "select", "channel", "status"]);
  const close = call.close(); assert.equal(call.close(), close); await close;
  assert.equal(f.candidates.size, 0); assert.equal(f.timers.size, 0); assert.equal(f.refunded, true);
  assert.deepEqual(call.snapshot(), { phase: "closed" }); assert.equal(await call.start(), "stopped");
  assert.notEqual(f.calls.prepare(1, "2"), call);
  await f.calls.close();
});

for (const stage of ["prepare-controller", "activate-controller", "prepare-owner", "activate-owner"])
  for (const committed of [false, true]) test(`End joins ${stage} (${committed ? "effect before reply" : "admission after wait"})`, async () => {
    const f = fixture(), held = f.hold(stage, committed), call = f.calls.prepare(1, "2"), start = call.start();
    await flush(); assert.ok(f.events.includes(stage));
    const close = call.close(); let finished = false; void close.then(() => { finished = true; });
    await flush(); assert.equal(finished, false); assert.equal(f.refunded, false);
    if (stage !== "prepare-controller") assert.ok(f.events.includes("end-" + id(1)), "known controller End precedes joining its held RPC");
    assert.throws(() => f.calls.prepare(1, "3"), /has not finished/);
    held.resolve(); await start; await close;
    assert.equal(f.candidates.size, 0); assert.equal(f.selectCalls, 0); assert.equal(f.observeCalls, 0);
    assert.equal(f.timers.size, 0); assert.equal(f.refunded, true);
  });

test("lost accepted activation repeats the original candidate on its paid cadence", async () => {
  const f = fixture(); f.lose("activate-owner");
  const call = f.calls.prepare(1, "2"), started = call.start(); await flush();
  assert.equal(call.snapshot().phase, "uncertain"); assert.equal(f.selectCalls, 0);
  assert.equal(f.events.filter(stage => stage === "prepare-owner").length, 1);
  await f.advance(); assert.equal(await started, "ready");
  assert.equal(f.events.filter(stage => stage === "activate-owner").length, 2);
  assert.equal(f.events.filter(stage => stage === "prepare-owner").length, 1);
  assert.equal(f.selectCalls, 1); await call.close();
});
test("lost prepare reply never automatically manufactures another candidate", async () => {
  const f = fixture(); f.lose("prepare-owner");
  const call = f.calls.prepare(1, "2"); assert.equal(await call.start(), "unavailable"); await call.close();
  assert.equal(f.events.filter(stage => stage === "prepare-owner").length, 1);
  assert.equal(f.events.includes("activate-owner"), false); assert.equal(f.candidates.size, 0);
});
test("End retries exact native cleanup while joining an original held publication", async () => {
  const f = fixture(), held = f.hold("activate-owner", true); f.lose("end-" + id(1));
  const call = f.calls.prepare(1, "2"), started = call.start(); await flush();
  const close = call.close(); await flush(); assert.equal(f.candidates.size, 0);
  await f.advance(); held.resolve(); await started; await close;
  assert.ok(f.events.filter(stage => stage === "end-" + id(1)).length >= 3);
  assert.equal(f.refunded, true);
});
for (const failure of ["selected", "registered-channel"] as const) test("End keeps group payment through actual " + failure + " cleanup", async () => {
  const f = fixture(), cleanup = f.holdCleanup();
  const held = failure === "selected" ? f.hold("select") : null;
  if (failure === "registered-channel") f.failChannel();
  const call = f.calls.prepare(1, "2"), started = call.start(); await flush();
  assert.equal(f.selectCalls, 1);
  const close = call.close(); await flush(); let done = false; void close.then(() => { done = true; });
  assert.equal(f.bound, true); assert.equal(f.refunded, false); assert.ok(f.retireCalls > 0);
  assert.equal(f.candidates.size, 0); assert.equal(done, false);
  held?.resolve(); cleanup.resolve(); await started; await close;
  assert.equal(f.bound, false); assert.equal(f.refunded, true);
});
test("group-capacity refusal ends accepted native owner before selecting any key", async () => {
  const f = fixture(); f.refuseGroup(); const call = f.calls.prepare(1, "2");
  assert.equal(await call.start(), "unavailable"); await call.close();
  assert.equal(f.selectCalls, 0); assert.equal(f.candidates.size, 0);
});
test("reentrant native callback coalesces startup and End before another invoke", async () => {
  const f = fixture(), original = f.native.prepareController; let call = f.calls.prepare(1, "2");
  let reentered: Promise<StartResult> | null = null, closing: Promise<void> | null = null;
  f.native.prepareController = async () => {
    reentered = call.start(); closing = call.close(); return original();
  };
  const start = call.start(); await start; assert.equal(reentered, start); assert.ok(closing); await closing;
  assert.equal(f.events.includes("activate-controller"), false); assert.equal(f.candidates.size, 0);
});
type StartResult = "ready" | "stopped" | "unavailable";

test("closed activation shapes and canonical room bounds refuse before key authority", async () => {
  for (const reply of [null, {}, { owner: id(1), group: hash(1), join: null }, { owner: id(1), group: null, join: null, extra: true }]) {
    const f = fixture(); f.native.activate = async () => reply as VoiceAppActivation;
    const call = f.calls.prepare(1, "2"), started = call.start(); await flush();
    assert.equal(call.snapshot().phase, "uncertain"); assert.equal(f.selectCalls, 0);
    await call.close(); await started;
  }
  const f = fixture();
  for (const [server, room] of [[0, "1"], [NaN, "1"], [1, "01"], [1, "-1"], [1, (1n << 128n).toString()]] as const)
    assert.throws(() => f.calls.prepare(server, room), /invalid voice room/);
  assert.equal(f.events.length, 0);
});
