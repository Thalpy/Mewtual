import assert from "node:assert/strict";
import test from "node:test";
import type { TestContext } from "node:test";
import { webcrypto } from "node:crypto";
import { VoiceCryptoBroker, createVoiceCryptoOwner, windowVoiceCryptoBroker } from "./voice-crypto.ts";
import type { VoiceSelectedKeyNative, VoiceSelectedKeyHandle } from "./voice-crypto.ts";
import { voiceJoinKeyNative } from "./voice-join-native.ts";

const real = webcrypto.subtle as unknown as SubtleCrypto;
const hash = (n: number) => n.toString(16).padStart(64, "0");
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function until(check: () => boolean) {
  const deadline = performance.now() + 15_000;
  while (!check()) { if (performance.now() >= deadline) throw new Error("condition did not settle"); await new Promise<void>(yes => setImmediate(yes)); }
}
function fixture(t: TestContext) {
  let generates = 0, exports = 0, ordinaryAdmits = 0, admissions = 0, publications = 0, settlements = 0;
  let loseAdmit = false, losePublish = false, loseSettle = false, capacity = false, closed = false, failEngine = false;
  let holdEngine: ReturnType<typeof gate> | null = null, holdPublish: ReturnType<typeof gate> | null = null, holdSettle: ReturnType<typeof gate> | null = null;
  let point: Uint8Array | null = null;
  const engine = new Proxy(real, { get(target, name) {
    const value = Reflect.get(target, name);
    if (typeof value !== "function") return value;
    return async (...args: unknown[]) => {
      if (name === "generateKey") {
        generates++; if (holdEngine) await holdEngine.promise;
        if (failEngine) throw new Error("provider refused");
      }
      if (name === "exportKey") exports++;
      return value.apply(target, args);
    };
  } });
  const ordinary = { async admit() { ordinaryAdmits++; return "must not be used"; }, async settle() {}, async commitInbound() {} };
  const owner = createVoiceCryptoOwner("native-owner", hash(1), () => true);
  const broker = new VoiceCryptoBroker(ordinary, engine);
  const native: VoiceSelectedKeyNative = {
    async admit(owner, executor) {
      admissions++;
      if (loseAdmit) { loseAdmit = false; throw new Error("accepted reply lost"); }
      return { owner, executor, permit: "native-permit", join: hash(2), kind: closed ? "closed" : "admitted" };
    },
    async publish(owner, executor, permit, bytes) {
      publications++; assert.equal(bytes.length, 65);
      if (point) assert.deepEqual(bytes, point); else point = bytes.slice();
      if (holdPublish) await holdPublish.promise;
      if (capacity) { capacity = false; throw new Error("Root capacity"); }
      if (losePublish) { losePublish = false; throw new Error("accepted publication reply lost"); }
      return { owner, executor, permit, join: hash(2), keyRef: hash(3) };
    },
    async settle() {
      settlements++; closed = true;
      if (holdSettle) await holdSettle.promise;
      if (loseSettle) { loseSettle = false; throw new Error("accepted settlement reply lost"); }
    },
  };
  const selected: VoiceSelectedKeyHandle[] = [];
  t.after(async () => {
    holdEngine?.release(); holdPublish?.release(); holdSettle?.release();
    owner.retire();
    for (const action of selected) void action.attempt();
    await until(() => broker.counts().active === 0);
  });
  return {
    broker, owner, native, ordinary, engine,
    select() { const handle = broker.selectJoinKey(owner, native); selected.push(handle); return handle; },
    counts: () => ({ generates, exports, ordinaryAdmits, admissions, publications, settlements }),
    loseAdmit() { loseAdmit = true; }, losePublish() { losePublish = true; }, loseSettle() { loseSettle = true; },
    capacity() { capacity = true; }, close() { closed = true; }, failEngine() { failEngine = true; },
    holdEngine() { holdEngine = gate(); return holdEngine; },
    holdPublish() { holdPublish = gate(); return holdPublish; }, holdSettle() { holdSettle = gate(); return holdSettle; },
  };
}

test("selected join key coalesces one real key and never uses generic native admission", async t => {
  const f = fixture(t), selected = f.select();
  assert.equal(f.select(), selected);
  const first = selected.attempt(); assert.equal(selected.attempt(), first);
  const result = await first; assert.equal(result.kind, "accepted");
  await until(() => f.broker.counts().active === 0);
  assert.deepEqual(f.counts(), { generates: 1, exports: 1, ordinaryAdmits: 0, admissions: 1, publications: 1, settlements: 1 });
  assert.equal(await selected.attempt(), result);
  if (result.kind === "accepted") {
    const point = result.key.publicKey(); point.fill(0);
    assert.equal(result.key.publicKey()[0], 4, "callers cannot mutate the retained point");
    f.owner.retire(); assert.throws(() => result.key.publicKey(), /owner changed/);
  }
});

for (const phase of ["admit", "publish", "settle", "capacity"] as const) test(`selected key recovers ${phase} refusal/reply loss without another generation`, async t => {
  const f = fixture(t), selected = f.select();
  if (phase === "admit") f.loseAdmit(); else if (phase === "publish") f.losePublish(); else if (phase === "settle") f.loseSettle(); else f.capacity();
  assert.equal((await selected.attempt()).kind, "uncertain");
  assert.equal(f.counts().generates, phase === "admit" ? 0 : 1);
  assert.equal(f.broker.counts().active, 1);
  assert.equal((await selected.attempt()).kind, "accepted");
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().generates, 1); assert.equal(f.counts().ordinaryAdmits, 0);
  assert.equal(f.counts().admissions, phase === "admit" ? 2 : 1);
  assert.equal(f.counts().publications, phase === "publish" || phase === "capacity" ? 2 : 1);
  assert.equal(f.counts().settlements, phase === "settle" ? 2 : 1);
});

test("closed native selection is history only and cannot regenerate a missing private key", async t => {
  const f = fixture(t); f.close();
  assert.equal((await f.select().attempt()).kind, "failed");
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().generates, 0); assert.equal(f.counts().publications, 0); assert.equal(f.counts().settlements, 0);
});

for (const phase of ["engine", "publish", "settle"] as const) test(`End during ${phase} keeps actual flight custody until it drains`, async t => {
  const f = fixture(t), selected = f.select();
  const hold = phase === "engine" ? f.holdEngine() : phase === "publish" ? f.holdPublish() : f.holdSettle();
  const pending = selected.attempt();
  await until(() => phase === "engine" ? f.counts().generates === 1 : phase === "publish" ? f.counts().publications === 1 : f.counts().settlements === 1);
  const before = f.broker.counts(); f.owner.retire();
  assert.equal((await pending).kind, "retired");
  assert.equal(f.broker.counts().active, 1); assert.equal(f.broker.counts().bytes, before.bytes);
  hold.release(); await until(() => f.broker.counts().active === 0);
  assert.equal(f.broker.counts().bytes, 0);
  assert.equal(f.counts().generates, 1);
  if (phase === "engine") assert.equal(f.counts().exports, 0);
  if (phase !== "settle") assert.equal(f.counts().settlements, 1);
});

test("End after lost admission settles its immutable identity without knowing the permit", async t => {
  const f = fixture(t); f.loseAdmit(); const selected = f.select();
  assert.equal((await selected.attempt()).kind, "uncertain"); f.owner.retire();
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().generates, 0); assert.equal(f.counts().settlements, 1);
});

test("retired orphan handle can retry a lost settlement reply without restarting work", async t => {
  const f = fixture(t), selected = f.select(); f.loseSettle();
  assert.equal((await selected.attempt()).kind, "uncertain");
  f.owner.retire();
  // Retirement wakes this exact cleanup owner. No later owner is allowed to refund it.
  await until(() => f.broker.counts().active === 0);
  assert.equal((await selected.attempt()).kind, "retired");
  assert.equal(f.counts().generates, 1); assert.equal(f.counts().settlements, 2);
});

test("provider failure is terminal for the selected generation", async t => {
  const f = fixture(t); f.failEngine(); const selected = f.select();
  assert.equal((await selected.attempt()).kind, "failed");
  await until(() => f.broker.counts().active === 0);
  assert.equal((await selected.attempt()).kind, "failed");
  assert.equal(f.counts().generates, 1); assert.equal(f.counts().settlements, 1);
});

test("End while settlement is pending preserves its wake when that reply is lost", async t => {
  const f = fixture(t), selected = f.select(), hold = f.holdSettle(); f.loseSettle();
  const pending = selected.attempt();
  await until(() => f.counts().settlements === 1);
  f.owner.retire(); assert.equal((await pending).kind, "retired");
  hold.release();
  await until(() => f.broker.counts().active === 0);
  assert.equal(f.counts().settlements, 2); assert.equal(f.counts().generates, 1);
  assert.equal(f.broker.counts().bytes, 0);
});

test("remount wakes bounded orphan settlement without its old component handle or a retry loop", async t => {
  const f = fixture(t), host = {}, original = f.native.settle;
  let attempts = 0;
  f.native.settle = async (...args) => { attempts++; if (attempts <= 2) throw new Error("reply unavailable"); await original(...args); };
  const broker = windowVoiceCryptoBroker(f.ordinary, host, f.engine);
  assert.equal((await broker.selectJoinKey(f.owner, f.native).attempt()).kind, "uncertain");
  f.owner.retire();
  await until(() => attempts === 2);
  await new Promise<void>(yes => setImmediate(yes));
  assert.equal(attempts, 2, "retirement permits one retry, not an immediate failure loop");
  assert.equal(broker.counts().active, 1);
  const fresh = await import(`./voice-crypto.ts?selected-orphan=${Date.now()}`);
  assert.equal(fresh.windowVoiceCryptoBroker(f.ordinary, host, f.engine), broker);
  await until(() => broker.counts().active === 0);
  assert.equal(attempts, 3); assert.equal(f.counts().generates, 1); assert.equal(broker.counts().bytes, 0);
});

test("same-token owner substitution and adapter replacement cannot adopt a retained selection", async t => {
  const f = fixture(t); f.select();
  const replacement = createVoiceCryptoOwner(f.owner.token, f.owner.group, () => true);
  assert.throws(() => f.broker.selectJoinKey(replacement, f.native), /selection changed/);
  assert.throws(() => f.broker.selectJoinKey(f.owner, { ...f.native }), /selection changed/);
  assert.equal(f.counts().generates, 0); replacement.retire();
});

test("selected pending job survives module remount on the original window broker", async t => {
  const f = fixture(t), host = {};
  const broker = windowVoiceCryptoBroker(f.ordinary, host, f.engine), selected = broker.selectJoinKey(f.owner, f.native);
  const hold = f.holdPublish(); const first = selected.attempt();
  await until(() => f.counts().publications === 1);
  const fresh = await import(`./voice-crypto.ts?selected-remount=${Date.now()}`);
  const retained = fresh.windowVoiceCryptoBroker(f.ordinary, host, f.engine);
  assert.equal(retained, broker); assert.equal(retained.selectJoinKey(f.owner, f.native), selected);
  hold.release(); assert.equal((await first).kind, "accepted");
  await until(() => broker.counts().active === 0);
  assert.equal(f.counts().generates, 1);
});

test("fixed native key adapter uses exact public commands and never sends a private key", async () => {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const native = voiceJoinKeyNative(async <T>(command: string, args: Record<string, unknown>) => { calls.push({ command, args }); return undefined as T; });
  await native.admit("owner", hash(1));
  await native.publish("owner", hash(1), "permit", new Uint8Array([4, 255]));
  await native.settle("owner", hash(1));
  assert.deepEqual(calls, [
    { command: "admit_voice_join_key", args: { owner: "owner", executor: hash(1) } },
    { command: "publish_voice_join_key", args: { owner: "owner", executor: hash(1), permit: "permit", publicKey: "04ff" } },
    { command: "settle_voice_join_key", args: { owner: "owner", executorId: hash(1) } },
  ]);
});
