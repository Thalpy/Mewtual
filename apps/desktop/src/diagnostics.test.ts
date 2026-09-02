import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_PENDING,
  classifyInvokeFailure,
  describeError,
  errorText,
  eventCorrelation,
  makeInvokeDebugged,
  makeOutbox,
  makeRecorder,
  makeResync,
  makeSeqTracker,
  makeTraceSource,
  needsResync,
  shortTrace,
  vaultUnlockErrorText,
  type SendOutcome,
  type UiEvent,
} from "./diagnostics.ts";

test("a trace is unique within a session and quotable in a bug report", () => {
  const traces = makeTraceSource(0x7f2c);
  const first = traces.next();
  const second = traces.next();
  assert.notEqual(first, second);
  assert.equal(first.length, 16);
  assert.match(first, /^[0-9a-f]{16}$/);
  assert.equal(shortTrace(first), first.slice(0, 4));
});

test("two sessions do not mint traces that look like each other's", () => {
  assert.notEqual(makeTraceSource(1).next(), makeTraceSource(2).next());
});

test("native trace provenance is returned unchanged and never invented by the webview", () => {
  assert.deepEqual(
    eventCorrelation({
      __trace: "7f2c000000000001",
      __trace_proof: "session-proof",
    }),
    { trace: "7f2c000000000001", trace_proof: "session-proof" },
  );
  assert.deepEqual(
    eventCorrelation({ __trace_proof: "orphan-proof" }),
    {},
    "a proof without its native trace cannot become diagnostic metadata",
  );
});

// --- sequence gaps ----------------------------------------------------------------------------

/**
 * The distinction the whole mechanism exists for. A coalesced update and a lost one look identical
 * from the webview, and only one of them leaves the unread badge wrong.
 */
/** An event as the native side stamps it: this family's sequence, the stream's, and the run. */
const stamped = (seq: number, ord: number, gen = 7) => ({ __seq: seq, __ord: ord, __gen: gen });

test("a missing event is reported with how many went missing", () => {
  const seq = makeSeqTracker();
  assert.equal(seq.observe("channel-updated", stamped(1198, 1)), null, "the first sighting cannot be a gap");
  assert.equal(seq.observe("channel-updated", stamped(1199, 2)), null);
  assert.deepEqual(seq.observe("channel-updated", stamped(1202, 5)), {
    kind: "gap",
    event: "channel-updated",
    expected: 1200,
    received: 1202,
    missed: 2,
  });
});

test("an unbroken run reports nothing", () => {
  const seq = makeSeqTracker();
  for (let n = 1; n <= 50; n += 1) {
    assert.equal(seq.observe("channel-updated", stamped(n, n)), null, `at ${n}`);
  }
});

/**
 * The loss a per-family sequence can never show.
 *
 * If the event that went missing was the last of its family, no later event of that family is ever
 * numbered against it, so its own counter stays unbroken forever. The stream's order is what
 * notices, on the next event of any family at all.
 */
test("an event lost from a family that then goes quiet is still noticed", () => {
  const seq = makeSeqTracker();
  seq.observe("channel-updated", stamped(1, 100));
  // A `files-updated` was emitted at ord 101 and never arrived. Its family says nothing, because
  // this is the first one this webview has seen of it.
  assert.deepEqual(seq.observe("members-changed", stamped(1, 102)), {
    kind: "stream-gap",
    event: "members-changed",
    expected: 101,
    received: 102,
    missed: 1,
  });
});

/**
 * The window an F5 or a hot reload opens: the native process keeps running and keeps emitting while
 * the webview comes back with no memory of what it had seen.
 *
 * Unseeded, the tracker takes whatever it sees first as its baseline, so everything missed during
 * that window is invisible. That is the difference between a diagnostic that reports gaps and one
 * that reports the gaps it happened to be present for.
 */
test("a remounted webview notices what it missed while it was not listening", () => {
  const seq = makeSeqTracker();
  seq.seed({ generation: 7, ord: 400 });
  // Four events were emitted between reading the cursor and the listener being live.
  assert.deepEqual(seq.observe("channel-updated", stamped(9, 405)), {
    kind: "stream-gap",
    event: "channel-updated",
    expected: 401,
    received: 405,
    missed: 4,
  });

  // Seeded and unbroken is silent, so the seeding cannot make every start look like a gap.
  const clean = makeSeqTracker();
  clean.seed({ generation: 7, ord: 400 });
  assert.equal(clean.observe("channel-updated", stamped(9, 401)), null);
});

/**
 * A different run of the native process means everything held is about a stream that has ended, so
 * comparing against it would produce nonsense: enormous gaps, or none at all.
 */
test("a stream from a different run of the process is not compared against the old one", () => {
  const seq = makeSeqTracker();
  seq.seed({ generation: 7, ord: 400 });
  seq.observe("channel-updated", stamped(9, 401));
  assert.deepEqual(seq.observe("channel-updated", stamped(1, 1, 99)), {
    kind: "generation",
    event: "channel-updated",
    previous: 7,
    received: 99,
  });
  // And it carries on from the new stream rather than reporting every event as a gap.
  assert.equal(seq.observe("channel-updated", stamped(2, 2, 99)), null);
});

/**
 * A listener installed twice looks exactly like this, and so does a retry that should not have
 * happened. Both are worth surfacing, and neither is a gap.
 */
test("a repeated or out-of-order delivery is reported as itself, not as a gap", () => {
  const seq = makeSeqTracker();
  seq.observe("channel-updated", stamped(10, 10));
  assert.deepEqual(seq.observe("channel-updated", stamped(10, 11)), {
    kind: "repeat",
    event: "channel-updated",
    previous: 10,
    received: 10,
  });
  assert.deepEqual(seq.observe("channel-updated", stamped(4, 12)), {
    kind: "repeat",
    event: "channel-updated",
    previous: 10,
    received: 4,
  });
});

test("sequences are tracked per event name, so a quiet feed cannot fake a gap in a busy one", () => {
  const seq = makeSeqTracker();
  seq.observe("channel-updated", stamped(100, 1));
  seq.observe("members-changed", stamped(3, 2));
  assert.equal(seq.observe("channel-updated", stamped(101, 3)), null);
  assert.equal(seq.observe("members-changed", stamped(4, 4)), null);
});

/**
 * Every event payload is an object now, so every one can carry the envelope. An unstamped event is
 * therefore a fault rather than a shrug: it used to be silence, which made "this family cannot be
 * checked" indistinguishable from "this family is fine".
 */
test("an event that carried no bookkeeping is named rather than passed over", () => {
  const seq = makeSeqTracker();
  assert.deepEqual(seq.observe("reachability-changed", undefined), {
    kind: "unstamped",
    event: "reachability-changed",
  });
  assert.deepEqual(seq.observe("reachability-changed", { __seq: NaN }), {
    kind: "unstamped",
    event: "reachability-changed",
  });
});

// --- repairing what the detector finds ----------------------------------------------------------

/**
 * The half P3-010 says was missing. A gap was recorded and nothing was re-fetched, so the record
 * could say the UI had gone stale while the UI stayed stale.
 */
test("a repeat needs no repair, and everything else does", () => {
  assert.equal(needsResync({ kind: "repeat", event: "e", previous: 1, received: 1 }), false);
  assert.equal(needsResync({ kind: "gap", event: "e", expected: 2, received: 4, missed: 2 }), true);
  assert.equal(
    needsResync({ kind: "stream-gap", event: "e", expected: 2, received: 4, missed: 2 }),
    true,
  );
  assert.equal(needsResync({ kind: "generation", event: "e", previous: 1, received: 2 }), true);
  assert.equal(needsResync({ kind: "unstamped", event: "e" }), true);
});

test("a burst of anomalies produces exactly one resynchronisation", async () => {
  // One dropped event usually means several, so several listeners notice within milliseconds of
  // each other. Re-fetching once per anomaly turns one lost event into a burst of refreshes, which
  // is worse than the staleness it repairs.
  const t = timers();
  let ran = 0;
  const resync = makeResync(async () => { ran += 1; }, t.scope);
  for (const reason of ["channel-updated", "members-changed", "files-updated"]) {
    resync.request(reason);
  }
  assert.equal(ran, 0, "nothing happens until the window closes");
  assert.deepEqual(resync.pendingReasons, ["channel-updated", "files-updated", "members-changed"]);
  t.run();
  await Promise.resolve();
  assert.equal(ran, 1);

  // And no follow-up. Each request scheduling its own timer would be absorbed by the guard against
  // concurrent runs and then reappear as a second, pointless refresh once the first finished, so
  // draining again is what tells one run from one-run-and-a-straggler.
  t.run();
  await Promise.resolve();
  assert.equal(ran, 1, "a burst is one refresh, not one and then another");
  assert.equal(resync.runs, 1);
});

test("an anomaly during a resynchronisation schedules exactly one more", async () => {
  // Whatever prompted it may have happened after the snapshot was taken, so it cannot be folded
  // into the run already in flight. It also must not start a second one alongside.
  const t = timers();
  let ran = 0;
  let release: (() => void) | null = null;
  const resync = makeResync(
    () =>
      new Promise<void>((resolve) => {
        ran += 1;
        release = resolve;
      }),
    t.scope,
  );
  resync.request("first");
  t.run();
  await Promise.resolve();
  assert.equal(ran, 1);

  resync.request("during");
  resync.request("during-too");
  t.run();
  await Promise.resolve();
  assert.equal(ran, 1, "the run in flight is not interrupted or duplicated");

  release?.();
  await Promise.resolve();
  await Promise.resolve();
  t.run();
  await Promise.resolve();
  assert.equal(ran, 2, "and exactly one more follows");
});

test("a repair that fails does not take the app with it", async () => {
  const t = timers();
  const resync = makeResync(async () => {
    throw new Error("the snapshot could not be read");
  }, t.scope);
  resync.request("channel-updated");
  t.run();
  await Promise.resolve();
  // The next anomaly asks again rather than the mechanism having wedged itself.
  resync.request("channel-updated");
  t.run();
  await Promise.resolve();
  assert.equal(resync.runs, 2);
});

// --- recording --------------------------------------------------------------------------------

function timers() {
  const scheduled = new Map<number, () => void>();
  let next = 1;
  return {
    scope: {
      setTimeout: (fn: () => void) => {
        const id = next++;
        scheduled.set(id, fn);
        return id;
      },
      clearTimeout: (handle: unknown) => scheduled.delete(handle as number),
    },
    run() {
      const due = [...scheduled.values()];
      scheduled.clear();
      for (const fn of due) fn();
    },
  };
}

const observation = (code: string): UiEvent => ({ section: "ui", code, level: "info" });

test("observations go in one batch rather than one call each", () => {
  const t = timers();
  const sent: UiEvent[][] = [];
  const recorder = makeRecorder((events) => sent.push(events), t.scope);
  recorder.record(observation("A"));
  recorder.record(observation("B"));
  assert.deepEqual(sent, []);
  t.run();
  assert.equal(sent.length, 1);
  assert.deepEqual(sent[0].map((e) => e.code), ["A", "B"]);
});

/**
 * Off has to mean the webview stops paying, not that it keeps building observations and sending
 * them across the bridge for the native side to discard.
 *
 * The webview cannot see the native capture gate, so it is told. Recording starts on because
 * capture starts on and is never persisted: a fresh webview always begins beside a process that is
 * recording, and the only thing that changes that is somebody moving the control.
 */
test("a recorder that has been switched off queues nothing and sends nothing", () => {
  const t = timers();
  const sent: UiEvent[][] = [];
  const recorder = makeRecorder((events) => sent.push(events), t.scope);
  assert.equal(recorder.capturing, true, "capture starts on, so recording does");

  recorder.setCapturing(false);
  for (let n = 0; n < 500; n += 1) recorder.record(observation(`E${n}`));
  t.run();
  assert.deepEqual(sent, [], "not one batch crossed the bridge");
  assert.equal(recorder.pending, 0, "and nothing accumulated waiting for one");
  assert.equal(recorder.dropped, 0, "discarded by request is not the same as lost");

  // And it comes straight back, with no restart and no reinstalling anything.
  recorder.setCapturing(true);
  recorder.record(observation("AFTER"));
  t.run();
  assert.deepEqual(sent.map((batch) => batch.map((e) => e.code)), [["AFTER"]]);
});

test("turning capture off discards what was queued rather than delivering it late", () => {
  // Those are observations the user has just said they do not want kept. Flushing them anyway
  // would make the control mean "stop soon", which is the sort of nearly-off that makes a privacy
  // setting untrustworthy.
  const t = timers();
  const sent: UiEvent[][] = [];
  const recorder = makeRecorder((events) => sent.push(events), t.scope);
  recorder.record(observation("QUEUED"));
  assert.equal(recorder.pending, 1);

  recorder.setCapturing(false);
  assert.equal(recorder.pending, 0);
  t.run();
  assert.deepEqual(sent, [], "the pending batch never went");
});

test("a full queue drops the oldest and counts it", () => {
  const t = timers();
  const recorder = makeRecorder(() => {}, t.scope);
  for (let n = 0; n < MAX_PENDING + 25; n += 1) recorder.record(observation(`E${n}`));
  assert.equal(recorder.dropped, 25);
  assert.equal(recorder.pending, MAX_PENDING);
});

// --- the bridge is asynchronous ------------------------------------------------------------------
//
// The bug: both batchers counted a loss only when `send` threw *immediately*, and the real send is
// a promise. The rejection arrived long after the batch had been retired, outside the batcher, and
// the counter still read zero. So the pipeline reported perfect health precisely when the bridge
// was unhealthy, which is the one moment its own losses matter. Found by adversarial review
// (P3-006).

/** A send that can be failed, delayed or made to accept only part of a batch, on demand. */
function bridge() {
  const settle: Array<(outcome: SendOutcome) => void> = [];
  const fail: Array<(error: Error) => void> = [];
  const batches: unknown[][] = [];
  return {
    batches,
    send: (batch: unknown[]) => {
      batches.push([...batch]);
      return new Promise<SendOutcome>((resolve, reject) => {
        settle.push(resolve);
        fail.push(reject);
      });
    },
    /** Accept the oldest outstanding batch, entirely or in part. */
    async accept(count?: number) {
      const at = settle.length - fail.length + 0;
      void at;
      const resolve = settle.shift();
      fail.shift();
      resolve?.({ offered: 0, accepted: count ?? Number.MAX_SAFE_INTEGER });
      await drain();
    },
    async reject() {
      settle.shift();
      const rejecter = fail.shift();
      rejecter?.(new Error("bridge down"));
      await drain();
    },
    get outstanding() {
      return settle.length;
    },
  };
}

/** Let the microtask queue settle, which is where an awaited send resumes. */
const drain = async () => {
  for (let n = 0; n < 20; n += 1) await Promise.resolve();
};

test("a rejected send is counted, not silently retired", async () => {
  const t = timers();
  const b = bridge();
  const outbox = makeOutbox<string>(b.send, { ...t.scope, maxPending: 100 });
  outbox.push(["a", "b", "c"]);
  outbox.flush();
  await drain();
  assert.equal(outbox.dropped, 0, "nothing is lost while the answer is outstanding");
  assert.equal(outbox.pending, 3, "and the batch is not retired until it is answered");

  // One retry, because a single rejection is most likely a remount that has already finished.
  await b.reject();
  assert.equal(b.batches.length, 2, "retried exactly once");
  assert.equal(outbox.dropped, 0);

  await b.reject();
  assert.equal(b.batches.length, 2, "and not again: a down sink is not retried into a loop");
  assert.equal(outbox.dropped, 3, "the records are gone, and the count is not");
  assert.equal(outbox.pending, 0);
});

test("a delayed send keeps its batch until the answer arrives", async () => {
  const t = timers();
  const b = bridge();
  const outbox = makeOutbox<string>(b.send, { ...t.scope, maxPending: 100 });
  outbox.push(["a"]);
  outbox.flush();
  await drain();
  outbox.push(["b", "c"]);
  outbox.flush();
  await drain();
  assert.equal(b.batches.length, 1, "one batch in flight at a time");
  assert.deepEqual(b.batches[0], ["a"]);

  await b.accept();
  assert.deepEqual(b.batches[1], ["b", "c"], "the rest follows once the first is answered");
  await b.accept();
  assert.equal(outbox.sent, 3);
  assert.equal(outbox.dropped, 0);
});

test("a native side that keeps fewer records than it was offered is counted honestly", async () => {
  // The limiter exists to suppress a storm, so accepting four of ten is it working as designed.
  // It is still a loss, and reporting ten delivered would be a lie about the record's completeness.
  const t = timers();
  const b = bridge();
  const outbox = makeOutbox<string>(b.send, { ...t.scope, maxPending: 100 });
  outbox.push(["a", "b", "c", "d", "e"]);
  outbox.flush();
  await drain();
  await b.accept(2);
  assert.equal(outbox.sent, 2);
  assert.equal(outbox.dropped, 3);
});

test("a teardown with records in flight accounts for them rather than losing them quietly", async () => {
  // A reload or a window close ends the page mid-batch, and nothing in it outlives that. What it
  // can do is stop pretending the records arrived.
  const t = timers();
  const b = bridge();
  const outbox = makeOutbox<string>(b.send, { ...t.scope, maxPending: 100 });
  outbox.push(["a", "b"]);
  outbox.flush();
  await drain();
  outbox.push(["c"]);

  const { abandoned } = outbox.stop();
  assert.equal(abandoned, 3, "the batch in flight and the one still queued");
  assert.equal(outbox.dropped, 3);
  assert.equal(outbox.pending, 0);

  // And it accepts nothing further, rather than queueing into a page that is going away.
  outbox.push(["d"]);
  assert.equal(outbox.pending, 0);
});

test("a storm stays inside its bounds and every record is accounted for exactly once", async () => {
  const t = timers();
  const b = bridge();
  const outbox = makeOutbox<number>(b.send, { ...t.scope, maxPending: 500, maxBatch: 100 });
  const offered = 10_000;
  for (let n = 0; n < offered; n += 1) {
    outbox.push([n]);
    assert.ok(outbox.pending <= 600, `memory stayed bounded at ${outbox.pending}`);
  }
  outbox.flush();
  await drain();
  // Drain whatever the bridge is holding, accepting everything.
  for (let n = 0; n < 20 && b.outstanding; n += 1) await b.accept();

  // The invariant that makes the numbers trustworthy: nothing is counted twice, and nothing
  // vanishes between the counters.
  assert.equal(outbox.sent + outbox.dropped + outbox.pending, offered);
  assert.ok(outbox.dropped > 0, "a storm this size cannot fit, and says so");
});

test("what the pipeline lost is told to the native side once it can speak again", async () => {
  // A dropped counter the far side never hears about is not evidence of anything.
  const t = timers();
  const b = bridge();
  const outbox = makeOutbox<string>(b.send, {
    ...t.scope,
    maxPending: 2,
    maxBatch: 10,
    lossRecord: (lost) => `lost:${lost}`,
  });
  outbox.push(["a", "b", "c", "d"]);
  assert.equal(outbox.dropped, 2, "the queue trimmed the oldest two");
  outbox.flush();
  await drain();
  assert.deepEqual(b.batches[0], ["c", "d", "lost:2"], "and says so, last, on the next batch");

  await b.accept();
  outbox.push(["e"]);
  outbox.flush();
  await drain();
  assert.deepEqual(b.batches[1], ["e"], "reported once, not on every batch afterwards");
});

test("a loss report that does not get through is owed again rather than forgotten", async () => {
  const t = timers();
  const b = bridge();
  const outbox = makeOutbox<string>(b.send, {
    ...t.scope,
    maxPending: 1,
    maxBatch: 10,
    lossRecord: (lost) => `lost:${lost}`,
  });
  outbox.push(["a", "b"]);
  assert.equal(outbox.dropped, 1);
  outbox.flush();
  await drain();
  assert.deepEqual(b.batches[0], ["b", "lost:1"]);

  await b.reject();
  await b.reject();
  // Both attempts failed, so the report never landed. It is owed again, and the loss it describes
  // has grown by the record that went with it.
  outbox.push(["c"]);
  outbox.flush();
  await drain();
  assert.deepEqual(b.batches[2], ["c", "lost:2"], "owed again, and the newer loss folded in");
});

test("a sink that throws is counted rather than retried into a loop", () => {
  const t = timers();
  const recorder = makeRecorder(() => {
    throw new Error("bridge down");
  }, t.scope);
  recorder.record(observation("A"));
  t.run();
  assert.equal(recorder.dropped, 1);
  assert.equal(recorder.pending, 0);
});

// --- the invoke wrapper -------------------------------------------------------------------------

/**
 * A diagnostic record is a thing that gets exported, so a failure is recorded by its shape rather
 * than by its prose: the bridge returns `Result<_, String>` and that string can contain whatever
 * the failing layer chose to interpolate.
 */
test("a failure is classified without carrying its message", () => {
  assert.equal(classifyInvokeFailure("session is locked"), "session_locked");
  assert.equal(classifyInvokeFailure("no actor for server 3"), "actor_unavailable");
  assert.equal(classifyInvokeFailure("channel not found"), "not_found");
  assert.equal(classifyInvokeFailure("timed out after 30s"), "timeout");
  assert.equal(classifyInvokeFailure("something nobody anticipated"), "failed");
  assert.equal(classifyInvokeFailure(undefined), "failed");
});

/**
 * A migrated command already answered this question properly, and its answer is stable in a way
 * that sniffing a sentence never is. Sniffing was always a stopgap for commands returning prose.
 */
test("a typed error is classified by its own code rather than by guessing at its words", () => {
  assert.equal(
    classifyInvokeFailure({ code: "CHAT.SEND.REJECTED", message: "message too long" }),
    "CHAT.SEND.REJECTED",
  );
  // And the code wins even when the message would have been sniffed into something else.
  assert.equal(
    classifyInvokeFailure({ code: "CHANNEL.ID.INVALID", message: "not found anywhere" }),
    "CHANNEL.ID.INVALID",
  );
});

function wrapper(invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>) {
  const recorded: UiEvent[] = [];
  let clock = 1000;
  const debugged = makeInvokeDebugged(
    invoke as <T>(c: string, a?: Record<string, unknown>) => Promise<T>,
    (e) => recorded.push(e),
    makeTraceSource(0x7f2c),
    () => (clock += 25),
  );
  return { debugged, recorded };
}

test("an instrumented command records both ends and the time between them", async () => {
  const { debugged, recorded } = wrapper(async () => ({ id: "m1" }));
  const { value, trace } = await debugged<{ id: string }>("send_message", { server: 1 });

  assert.deepEqual(value, { id: "m1" });
  assert.equal(recorded.length, 2);
  assert.equal(recorded[0].code, "IPC.COMMAND.STARTED");
  assert.equal(recorded[1].code, "IPC.COMMAND.COMPLETED");
  // Both ends carry the same trace, which is what ties them to whatever the command causes later.
  assert.equal(recorded[0].trace, trace);
  assert.equal(recorded[1].trace, trace);
  assert.equal(recorded[1].duration_ms, 25);
  assert.equal(recorded[1].fields?.command, "send_message");
});

/**
 * The native record alone cannot tell a command that was never sent from one that was sent and
 * never answered. The frontend's own view of the call is the other half.
 */
test("a failed command is recorded and still throws", async () => {
  const { debugged, recorded } = wrapper(async () => {
    throw new Error("session is locked");
  });
  await assert.rejects(() => debugged("send_message", {}), /locked/);
  assert.equal(recorded.length, 2);
  assert.equal(recorded[1].code, "IPC.COMMAND.FAILED");
  assert.equal(recorded[1].phase, "failure");
  assert.equal(recorded[1].fields?.failure, "session_locked");
  assert.ok(!JSON.stringify(recorded).includes("session is locked"), "the message stays out of the record");
});

test("the trace travels to the native side with the arguments", async () => {
  const seen: Record<string, unknown>[] = [];
  const { debugged } = wrapper(async (_c, args) => {
    seen.push(args ?? {});
    return null;
  });
  const { trace } = await debugged("send_message", { server: 1, text: "hello" });
  assert.equal(seen[0].trace, trace);
  assert.equal(seen[0].server, 1, "the real arguments are untouched");
});

// --- typed errors -------------------------------------------------------------------------------
//
// The property that makes the error migration incremental: one reader handles both shapes, so a
// call site can adopt it before its command is migrated and a half-migrated bridge behaves exactly
// like an unmigrated one.

test("a bare string error still reads as the message it always was", () => {
  assert.deepEqual(describeError("message too long"), { message: "message too long" });
  assert.equal(errorText("message too long"), "message too long");
});

test("a typed error keeps the message and gains a code, a trace and what to do", () => {
  const view = describeError({
    code: "CHAT.SEND.REJECTED",
    message: "message too long",
    trace: "7f2c",
    retryable: false,
    remediation: "amend_input",
  });
  assert.equal(view.message, "message too long", "the text the user already saw is preserved");
  assert.equal(view.code, "CHAT.SEND.REJECTED");
  assert.equal(view.trace, "7f2c");
  assert.equal(view.retryable, false);
  assert.equal(view.remediation, "amend_input");
});

/**
 * The regression this reader exists to prevent. Without it, migrating a command's error type turns
 * its message into "[object Object]" on screen, which is a worse report than the one it replaced.
 */
test("a typed error never renders as [object Object]", () => {
  const shown = errorText({ code: "SESSION.LOCKED", message: "session is locked", trace: "0001" });
  assert.equal(shown, "session is locked (SESSION.LOCKED · 0001)");
  assert.ok(!shown.includes("object Object"));
});

test("the vault gate explains an unlock refusal in user terms and keeps its support ids", () => {
  assert.equal(
    vaultUnlockErrorText({
      message: "decryption/authentication failed",
      code: "VAULT.OPEN.REFUSED",
      trace: "trace-7",
      retryable: true,
      remediation: "amend_input",
    }),
    "That passphrase did not unlock this vault. Check it and try again. (VAULT.OPEN.REFUSED · trace-7)",
  );
  assert.equal(vaultUnlockErrorText("vault unavailable"), "vault unavailable");
});

test("an Error instance is stringified rather than mistaken for a typed error", () => {
  // An Error has a `message`, so without the guard it would be read as a partly-formed AppError.
  assert.deepEqual(describeError(new Error("boom")), { message: "Error: boom" });
});

test("an object that only half looks like a typed error falls back rather than inventing fields", () => {
  assert.deepEqual(describeError({ message: "no code here" }), { message: "[object Object]" });
  assert.deepEqual(describeError({ code: "X.Y" }), { message: "[object Object]" });
});

test("nothing at all still produces something to show", () => {
  assert.equal(describeError(undefined).message, "undefined");
  assert.equal(describeError(null).message, "null");
});

test("a caller can continue an existing trace rather than starting a new one", async () => {
  const { debugged, recorded } = wrapper(async () => null);
  const { trace } = await debugged("a", {}, { trace: "00000000000000ff" });
  assert.equal(trace, "00000000000000ff");
  assert.ok(recorded.every((e) => e.trace === "00000000000000ff"));
});
