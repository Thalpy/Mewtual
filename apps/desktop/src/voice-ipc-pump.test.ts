import assert from "node:assert/strict";
import test from "node:test";
import { NativeWorkPump, type NativeWorkActions } from "./voice-ipc-pump.ts";
import type { VoiceCryptoChain, VoiceChannelDerivationOutcome, VoiceChannelConfirmationOutcome } from "./voice-crypto.ts";
import type { VoiceWorkDescriptor } from "./voice-ipc-work.ts";

const hash = (n: number) => n.toString(16).padStart(64, "0");
const row = (n: number) => n.toString(16).padStart(16, "0") + "0000000000000001";
const purpose = { kind: "derive" as const, peer: hash(1), join: hash(2), keyRef: hash(3), epoch: "7" };
const parent = { row: row(0), purpose, state: "attached" as const, actorHandoff: null, identity: { action: "1", version: "1", inputTag: hash(4) } };
const child = { ...parent, row: row(1), identity: { ...parent.identity, action: "2" },
  purpose: { kind: "confirmation" as const, parent: parent.row, parentPurpose: purpose, request: "start" as const, predecessor: null } };
function gate() { let release!: () => void; const promise = new Promise<void>(yes => { release = yes; }); return { promise, release }; }
async function flush() { for (let n = 0; n < 24; n++) await Promise.resolve(); }
function fixture() {
  let now = 0, serial = 0, live = true, derived = 0, attempts = 0, returns = 0, children = 0, childAttempts = 0;
  let isReturned = false, deriveOutcome: VoiceChannelDerivationOutcome | null = null;
  let childOutcome: VoiceChannelConfirmationOutcome = { kind: "uncertain" };
  const timers = new Map<number, { at: number; work(): void }>();
  const reads: number[] = [];
  const chain: VoiceCryptoChain = { isCurrent: () => live, close() { live = false; } };
  const ready: VoiceChannelDerivationOutcome = { kind: "ready", chain,
    derivation: { async attempt() { attempts++; return { kind: "accepted", receipt: {} as never }; }, inspect: () => "accepted", pair: () => null, retire() {} } };
  let rows: VoiceWorkDescriptor[] = [parent, child];
  const actions: NativeWorkActions = {
    owner: "voice-0000000000000001", executor: hash(5), live: () => live,
    async read(from) { reads.push(from); const work = rows.find(work => Number(BigInt("0x" + work.row.slice(0, 16))) >= from) ?? null;
      return { kind: "snapshot", owner: actions.owner, executor: actions.executor,
        nextSlot: work ? Number(BigInt("0x" + work.row.slice(0, 16))) + 1 : 32, work }; },
    derive(work) { assert.equal(work.row, parent.row); derived++; return { async attempt() { return deriveOutcome ?? ready; } }; },
    async returnDerivation(actual) { assert.equal(actual, chain); returns++; },
    confirmation(actual, work) { assert.equal(actual, chain); assert.equal(work.row, child.row); children++;
      return { async attempt() { childAttempts++; return childOutcome; }, row: () => child.row }; },
    returned: () => isReturned,
  };
  const pump = new NativeWorkPump(actions, { now: () => now,
    schedule(work, ms) { const id = ++serial; timers.set(id, { at: now + ms, work }); return id; },
    cancel(handle) { timers.delete(handle as number); } });
  return { actions, pump, reads, chain,
    start: () => pump.start(), close: () => pump.close(), rows(value: typeof rows) { rows = value; },
    counts: () => ({ derived, attempts, returns, children, childAttempts, timers: timers.size }),
    deriveOutcome(value: VoiceChannelDerivationOutcome) { deriveOutcome = value; },
    returned() { isReturned = true; childOutcome = { kind: "accepted", receipt: {} as never }; },
    async tick(ms = 250) { now += ms; for (const [id, timer] of [...timers]) if (timer.at <= now) { timers.delete(id); timer.work(); } await flush(); },
  };
}

test("serial pump retains one parent and one uncertain child until actual Return", async () => {
  const f = fixture(); f.start(); await flush();
  assert.equal(f.counts().derived, 0); // pin before the next bounded turn
  await f.tick(); assert.deepEqual(f.counts(), { derived: 1, attempts: 1, returns: 1, children: 1, childAttempts: 0, timers: 1 });
  await f.tick(); const reads = f.reads.length;
  await f.tick(); assert.equal(f.counts().children, 1); assert.equal(f.reads.length, reads);
  f.returned(); await f.tick(); await f.tick();
  assert.equal(f.counts().derived, 1); assert.equal(f.counts().children, 1);
  await f.close(); assert.equal(f.counts().timers, 0);
});

test("Pending slot reuse remains visible and does not enter completed bitmap", async () => {
  const f = fixture(); f.rows([{ ...parent, state: "pending", identity: null }]); f.start(); await flush();
  assert.equal(f.counts().derived, 0);
  f.rows([parent]); await f.tick(); await f.tick();
  assert.equal(f.counts().derived, 1); assert.equal(f.counts().returns, 1); await f.close();
});

test("Finish custody retries local Claim pressure on the same pump without scanning or adopting old work", async () => {
  const f = fixture(); let requested = false, claims = 0;
  Object.assign(f.actions, { async transition(actual: VoiceCryptoChain) {
    assert.equal(actual, f.chain);
    if (!requested) return false;
    claims++;
    if (claims === 1) throw new Error("native Claim capacity");
    return true;
  } });
  f.start(); await flush(); await f.tick(); await f.tick();
  f.returned(); requested = true; const reads = f.reads.length;
  await f.tick(); await f.tick();
  assert.equal(claims, 2); assert.equal(f.reads.length, reads);
  assert.equal(f.counts().derived, 1); assert.equal(f.counts().children, 1);
  await f.close(); assert.equal(f.counts().timers, 0);
});

for (const kind of ["status", "retired"] as const) test(`historical/terminal Derive ${kind} never adopts another parent`, async () => {
  const f = fixture(); f.deriveOutcome(kind === "status" ? { kind: "status" } as never : { kind });
  f.start(); await flush(); await f.tick();
  f.rows([{ ...parent, row: row(3) }]); await f.tick(); await f.tick();
  assert.equal(f.counts().derived, 1); assert.equal(f.counts().attempts, 0); await f.close();
});

test("End holds the real metadata owner and never adopts its late reply", async () => {
  const f = fixture(), held = gate(); let entered = false, closed = false;
  const original = f.actions.read;
  Object.assign(f.actions, { async read(from: number) { entered = true; await held.promise; return original(from); } });
  f.start(); await flush(); assert.ok(entered);
  const closing = f.close().then(() => { closed = true; }); await flush(); assert.equal(closed, false);
  held.release(); await closing; assert.equal(f.counts().derived, 0); assert.equal(f.counts().timers, 0);
  const stopped = f.pump as unknown as { parent: unknown; child: unknown; actions: unknown };
  assert.equal(stopped.parent, null); assert.equal(stopped.child, null); assert.equal(stopped.actions, null);
});

test("malformed/cross-channel metadata cannot trigger an intake", async () => {
  const f = fixture(); Object.assign(f.actions, { async read() { return { kind: "snapshot", owner: "voice-0000000000000002", executor: hash(5), nextSlot: 1, work: parent }; } });
  f.start(); await flush(); await f.tick(); assert.equal(f.counts().derived, 0); await f.close();
});

test("availability floods keep one timer and minimum cadence; stopped pump cannot restart", async () => {
  const f = fixture(); f.rows([]); f.start(); await flush(); const reads = f.reads.length;
  for (let n = 0; n < 500; n++) f.pump.wake();
  assert.equal(f.counts().timers, 1); await f.tick(49); assert.equal(f.reads.length, reads);
  await f.tick(1); assert.equal(f.reads.length, reads + 1);
  await f.tick(10_000); assert.equal(f.reads.length, reads + 2, "missed intervals never burst");
  await f.close(); f.start(); f.pump.wake(); await f.tick(); assert.equal(f.reads.length, reads + 2);
});

test("whole bounded scan passes Pending peers to find an attached parent", async () => {
  const f = fixture(); const pending = Array.from({ length: 20 }, (_, n) => ({ ...parent, row: row(n), state: "pending" as const, identity: null }));
  f.rows([...pending, { ...parent, row: row(20) }]); f.start(); await flush();
  assert.equal(f.reads.length, 21); await f.close();
});
