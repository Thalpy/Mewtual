import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import ts from "typescript";
import { PendingSendRetry, type RetryClock } from "./pending-send-retry.ts";
import { MAX_PENDING_SEND_BYTES, pendingSendRetryBlock, sanitizePendingSends, type PendingSend } from "./pending-sends.ts";
import { persistenceWarning } from "./message-send.ts";

class FakeClock implements RetryClock {
  now = 0;
  next = 0;
  timers = new Map<number, { at: number; callback: () => void }>();
  delays: number[] = [];
  maxTimers = 0;
  afterMicrotask = () => {};
  setTimeout(callback: () => void, delay: number): number {
    const id = this.next++;
    this.timers.set(id, { at: this.now + delay, callback });
    this.delays.push(delay);
    this.maxTimers = Math.max(this.maxTimers, this.timers.size);
    return id;
  }
  clearTimeout(id: unknown): void { this.timers.delete(id as number); }
  async flush(): Promise<void> {
    for (let n = 0; n < 30; n++) { await Promise.resolve(); this.afterMicrotask(); }
  }
  async advance(ms: number): Promise<void> {
    const until = this.now + ms;
    for (;;) {
      const next = [...this.timers].sort((a, b) => a[1].at - b[1].at)[0];
      if (!next || next[1].at > until) break;
      this.now = next[1].at;
      this.timers.delete(next[0]);
      next[1].callback();
      await this.flush();
    }
    this.now = until;
    await this.flush();
  }
}

test("one quiet retry timer coalesces updates and caps exponential backoff", async () => {
  const clock = new FakeClock();
  const attempts: number[] = [];
  const retry = new PendingSendRetry(async () => { attempts.push(clock.now); throw new Error("busy"); }, clock);
  retry.update(7, true, false);
  await clock.advance(4_000);
  for (let n = 0; n < 1_000; n++) retry.update(7, true, false);
  await clock.advance(191_000);
  assert.deepEqual(attempts, [5_000, 15_000, 35_000, 75_000, 135_000, 195_000]);
  assert.deepEqual(clock.delays, [5_000, 10_000, 20_000, 40_000, 60_000, 60_000, 60_000]);
  assert.equal(clock.maxTimers, 1);
  retry.update(7, false, false);
  assert.equal(clock.timers.size, 0);
  retry.update(7, true, false);
  assert.equal(clock.delays.at(-1), 5_000, "new work after an empty queue starts a fresh backoff");
});

test("busy passes do not overlap, and old session callbacks cannot affect a replacement", async () => {
  const clock = new FakeClock();
  const sessions: number[] = [];
  let release!: () => void;
  const retry = new PendingSendRetry(async session => {
    sessions.push(session);
    if (session === 1) await new Promise<void>(resolve => { release = resolve; });
  }, clock);
  retry.update(1, true, false);
  const stale = [...clock.timers.values()][0].callback;
  await clock.advance(5_000);
  for (let n = 0; n < 20; n++) retry.update(1, true, false);
  await clock.advance(60_000);
  assert.deepEqual(sessions, [1]);
  assert.equal(clock.timers.size, 0, "one unfinished pass owns the retry slot");
  retry.cancel();
  retry.update(2, true, false);
  stale();
  release();
  await clock.flush();
  assert.equal(clock.timers.size, 1, "old completion cannot clear the new session's wake");
  await clock.advance(5_000);
  assert.deepEqual(sessions, [1, 2]);
  retry.update(null, true, false);
  await clock.advance(120_000);
  assert.deepEqual(sessions, [1, 2]);
});

test("foreground work pauses retries without overlapping it or discarding the backoff", async () => {
  const clock = new FakeClock();
  let attempts = 0;
  const retry = new PendingSendRetry(async () => { attempts++; }, clock);
  retry.update(1, true, false);
  await clock.advance(5_000);
  retry.update(1, true, true);
  await clock.advance(120_000);
  assert.equal(attempts, 1);
  retry.update(1, true, false);
  await clock.advance(9_999);
  assert.equal(attempts, 1);
  await clock.advance(1);
  assert.equal(attempts, 2);
});

const source = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
function production(start: string, end: string): string {
  const from = source.indexOf(start), to = source.indexOf(end, from);
  assert.ok(from >= 0 && to > from, "execute actual production scheduling/submission functions");
  return ts.transpile(source.slice(from, to), { target: ts.ScriptTarget.ES2022 });
}

const intent: PendingSend = { token: "1".repeat(32), server: 3, channel: "9",
  expectedContext: "a".repeat(64), text: "retain exactly this message", replyTo: "parent" };

/** Actual App scheduling effect + submit/retry functions. Only I/O and Svelte reactivity differ. */
function appFixture(clock: FakeClock, failure: string | null = null) {
  class ScheduledRetry extends PendingSendRetry {
    constructor(run: (session: number) => Promise<void>) { super(run, clock); }
  }
  const app = new Function("PendingSendRetry", "pendingSendRetryBlock", "persistenceWarning", "sanitizePendingSends", "intent", "initialFailure", `
    let locked = false, uiStateReady = true, uiStateLoadGeneration = 1, sending = false;
    let pendingSends = { [intent.token]: structuredClone(intent) }, pendingSendErrors = {}, retryingPendingSends = false;
    let draft = intent.text, drafts = { room: intent.text }, draftRevisions = {}, replyingTo = intent.replyTo;
    let cur = { active: intent.channel }, activeServerId = intent.server, error = "";
    let savesFail = false, failure = initialFailure, lostAcknowledgement = false, writes = [], submissions = [], authored = new Set();
    const effects = [], $effect = effect => effects.push(effect), onMount = () => {};
    const chatScopeKey = () => "room", errorText = String, refresh = async () => {};
    const saveUiStateImmediately = async () => {
      if (savesFail || locked || !uiStateReady) return false;
      writes.push(structuredClone(pendingSends)); return true;
    };
    const invokeDebugged = async (_, args) => {
      submissions.push({ args: structuredClone(args), sealed: structuredClone(writes.at(-1)) });
      if (failure) throw new Error(failure);
      authored.add(args.retryToken);
      if (lostAcknowledgement) { lostAcknowledgement = false; throw new Error("IPC acknowledgement lost"); }
      return { value: { accepted: true, persistence: { status: "durable" } } };
    };
    ${production("  const pendingSendRetry =", "  function queueUiStateSave(")}
    ${production("  async function submitPendingSend(", "  async function send()")}
    return {
      sync() { for (const effect of effects) effect(); },
      retry: () => retryPendingSends(),
      storageFailure(value) { savesFail = value; },
      nativeFailure(value) { failure = value; },
      loseAcknowledgement() { lostAcknowledgement = true; },
      setError(value) { pendingSendErrors[intent.token] = value; },
      lock() { pendingSendRetry.cancel(); locked = true; uiStateReady = false; uiStateLoadGeneration++; pendingSends = {}; pendingSendErrors = {}; },
      unlock() { locked = false; uiStateReady = true; uiStateLoadGeneration++; pendingSends = sanitizePendingSends(writes.at(-1)); },
      unhydrated() { uiStateReady = false; },
      state() { return { pendingSends, submissions, writes, authored: authored.size, error, draft }; }
    };
  `)(ScheduledRetry, pendingSendRetryBlock, persistenceWarning, sanitizePendingSends, intent, failure);
  clock.afterMicrotask = () => app.sync();
  app.sync();
  return app;
}

test("actual quiet retry crosses the save barrier after storage recovers and reuses ambiguous IPC identity", async () => {
  const clock = new FakeClock();
  const app = appFixture(clock);
  app.storageFailure(true);
  await clock.advance(5_000);
  assert.equal(app.state().submissions.length, 0, "a failed continuity barrier never reaches authoring");
  app.storageFailure(false);
  app.nativeFailure("Message storage is busy; retry this pending message.");
  await clock.advance(10_000);
  assert.equal(app.state().submissions.length, 1);
  app.nativeFailure(null);
  app.loseAcknowledgement();
  await clock.advance(20_000);
  assert.equal(app.state().authored, 1);
  assert.equal(Object.keys(app.state().pendingSends).length, 1);
  for (let n = 0; n < 50; n++) { app.setError(`different diagnostic ${n}`); app.sync(); }
  await clock.advance(39_999);
  assert.equal(app.state().submissions.length, 2);
  await clock.advance(1);
  assert.equal(app.state().submissions.length, 3);
  assert.equal(app.state().authored, 1, "the same idempotency token replays the ambiguous operation");
  assert.deepEqual(app.state().pendingSends, {});
  assert.equal(app.state().draft, "");
  assert.equal(clock.timers.size, 0);
  assert.equal(clock.maxTimers, 1);
  for (const { args, sealed } of app.state().submissions) {
    assert.equal(args.retryToken, intent.token);
    assert.equal(args.expectedContext, intent.expectedContext);
    assert.equal(args.text, intent.text);
    assert.equal(args.replyTo, intent.replyTo);
    assert.equal(sealed[intent.token].expectedContext, args.expectedContext);
    assert.equal(sealed[intent.token].token, args.retryToken);
  }
});

test("permanent actual refusals stay manual across vault reopen; a manual retry keeps the exact identity", async () => {
  for (const [failure, block] of [
    ["CHAT_SEND_CONTEXT_CHANGED: earlier epoch", "context_changed"],
    ["CHAT_SEND_TOKEN_CONFLICT: reused token", "conflict"],
    ["CHAT_SEND_INVALID: length", "invalid"],
  ]) {
    const clock = new FakeClock();
    const app = appFixture(clock, failure);
    await clock.advance(5_000);
    assert.equal(app.state().pendingSends[intent.token].retryBlock, block);
    assert.equal(clock.timers.size, 0);
    app.lock(); app.sync(); app.unlock(); app.sync();
    await clock.advance(120_000);
    assert.equal(app.state().submissions.length, 1);
    assert.equal(app.state().pendingSends[intent.token].retryBlock, block);
    await app.retry();
    await clock.flush();
    assert.equal(app.state().submissions.length, 2);
    assert.equal(app.state().submissions[1].args.retryToken, intent.token);
    assert.equal(clock.timers.size, 0);
  }
});

test("actual scheduling neither invokes nor writes while locked or continuity is unhydrated", async () => {
  for (const stop of ["lock", "unhydrated"]) {
    const clock = new FakeClock();
    const app = appFixture(clock);
    const queued = [...clock.timers.values()][0].callback;
    app[stop](); app.sync();
    queued();
    await clock.advance(120_000);
    assert.equal(app.state().submissions.length, 0);
    assert.equal(app.state().writes.length, 0);
    assert.equal(clock.timers.size, 0);
  }
});

test("temporary structured-send reasons retain automatic retry and unknown persisted blocks fail closed", () => {
  for (const reason of ["CHAT.SEND.REJECTED: Message storage is busy", "CHAT_SEND_CAPACITY: full",
    "CHAT_SEND_DOCUMENT_PENDING: prior operation", "IPC failed", "Save the pending message in this vault before retrying it."]) {
    assert.equal(pendingSendRetryBlock(reason), undefined);
  }
  assert.equal(sanitizePendingSends({ [intent.token]: { ...intent, retryBlock: "future-permanent-reason" } })[intent.token].retryBlock, "invalid");
});

test("pausing a full hydrated payload cannot evict any existing retry identity", () => {
  const pending: Record<string, PendingSend> = {};
  const bytes = (value: unknown) => new TextEncoder().encode(JSON.stringify(value)).length;
  let remaining = MAX_PENDING_SEND_BYTES;
  for (let n = 0; n < 4; n++) {
    const token = n.toString(16).padStart(32, "0");
    const entry = { ...intent, token, text: "" };
    entry.text = "x".repeat(Math.min(65_536, remaining - bytes(entry)));
    remaining -= bytes(entry);
    pending[token] = entry;
  }
  assert.equal(remaining, 0, "fixture reaches the hydration payload boundary exactly");
  assert.equal(Object.keys(sanitizePendingSends(pending)).length, 4);
  for (const entry of Object.values(pending)) entry.retryBlock = "context_changed";
  const restored = sanitizePendingSends(pending);
  assert.equal(Object.keys(restored).length, 4);
  for (const entry of Object.values(restored)) assert.equal(entry.retryBlock, "context_changed");
});
