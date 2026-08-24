import assert from "node:assert/strict";
import test from "node:test";
import {
  MAX_PENDING,
  classifyInvokeFailure,
  describeError,
  errorText,
  makeInvokeDebugged,
  makeRecorder,
  makeSeqTracker,
  makeTraceSource,
  shortTrace,
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

// --- sequence gaps ----------------------------------------------------------------------------

/**
 * The distinction the whole mechanism exists for. A coalesced update and a lost one look identical
 * from the webview, and only one of them leaves the unread badge wrong.
 */
test("a missing event is reported with how many went missing", () => {
  const seq = makeSeqTracker();
  assert.equal(seq.observe("channel-updated", 1198), null, "the first sighting cannot be a gap");
  assert.equal(seq.observe("channel-updated", 1199), null);
  assert.deepEqual(seq.observe("channel-updated", 1202), {
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
    assert.equal(seq.observe("channel-updated", n), null, `at ${n}`);
  }
});

/**
 * A listener installed twice looks exactly like this, and so does a retry that should not have
 * happened. Both are worth surfacing, and neither is a gap.
 */
test("a repeated or out-of-order delivery is reported as itself, not as a gap", () => {
  const seq = makeSeqTracker();
  seq.observe("channel-updated", 10);
  assert.deepEqual(seq.observe("channel-updated", 10), {
    kind: "repeat",
    event: "channel-updated",
    previous: 10,
    received: 10,
  });
  assert.deepEqual(seq.observe("channel-updated", 4), {
    kind: "repeat",
    event: "channel-updated",
    previous: 10,
    received: 4,
  });
});

test("sequences are tracked per event name, so a quiet feed cannot fake a gap in a busy one", () => {
  const seq = makeSeqTracker();
  seq.observe("channel-updated", 100);
  seq.observe("members-changed", 3);
  assert.equal(seq.observe("channel-updated", 101), null);
  assert.equal(seq.observe("members-changed", 4), null);
});

test("an event whose payload cannot carry a sequence is not guessed about", () => {
  const seq = makeSeqTracker();
  // Bare-number payloads (a server id) have nowhere to put __seq. Silence is the honest answer.
  assert.equal(seq.observe("reachability-changed", undefined), null);
  assert.equal(seq.observe("reachability-changed", "12"), null);
  assert.equal(seq.observe("reachability-changed", NaN), null);
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
