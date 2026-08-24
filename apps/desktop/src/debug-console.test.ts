import assert from "node:assert/strict";
import test from "node:test";
import {
  BRIDGED_CODE,
  CAPTURE_MODES,
  DEFAULT_LEVELS,
  appendEvents,
  alias,
  captureModeIsRevealing,
  captureModeNote,
  copyBundle,
  dropNote,
  eventLine,
  eventParts,
  eventText,
  filterEvents,
  formatDuration,
  inView,
  isFrontend,
  latestSeq,
  makeAliases,
  maybeRedact,
  redactText,
  routeChip,
  routeExplanation,
  routeState,
  shortTrace,
  shownCount,
  taskChip,
  taskConsequence,
  traceEvents,
  collectAllEvents,
  deviceLines,
  levelClass,
  mediaPathChip,
  routeFindings,
  routeLines,
  voiceLines,
  webrtcChip,
  type DebugDevice,
  type DebugVoicePeer,
  type LogEvent,
  type MemberRoute,
  type TaskHealth,
} from "./debug-console.ts";

/**
 * One canonical event, with the parts a test rarely cares about defaulted.
 *
 * `message` is not a property any more: an event's headline is its stable code, and an un-migrated
 * `tracing` line carries its prose in a `message` **field** under `BRIDGED_CODE`. `bridged()` below
 * builds that shape, and the distinction is the point rather than an inconvenience.
 */
function ev(over: Partial<LogEvent> = {}): LogEvent {
  return {
    seq: 1,
    at_ms: Date.UTC(2026, 7, 23, 12, 0, 0),
    monotonic_ms: 0,
    section: "transport",
    view: "network",
    level: "INFO",
    code: "NET.TEST",
    phase: "observation",
    operation: "",
    trace: "",
    span: "",
    parent_span: "",
    refs: [],
    duration_ms: null,
    attempt: null,
    target: "catcoms_net",
    fields: [],
    capture: "safe",
    ...over,
  };
}

/** An un-migrated `tracing` event: prose in a `message` field, under the bridge's code. */
function bridged(message: string, over: Partial<LogEvent> = {}): LogEvent {
  return ev({
    code: BRIDGED_CODE,
    ...over,
    fields: [{ name: "message", value: message, sensitive: false }, ...(over.fields ?? [])],
  });
}

/** A rendered field, for the shape `LogEvent.fields` takes. */
const f = (name: string, value: string, sensitive = false) => ({ name, value, sensitive });

const route: MemberRoute = {
  fingerprint: "741af9ff",
  peer: "2b5df389",
  addresses: ["/ip4/203.0.113.9/udp/31484/quic-v1"],
  seq: 4,
  connected: false,
  dial_attempts: 0,
  next_dial_in_ms: 0,
};

/**
 * The heuristics this replaces. The console used to split Backend from Frontend on the tracing
 * target and pick out voice events by searching each rendered line for the word "voice", so a
 * structured voice event emitted from the webview landed in Frontend and, once its prose became
 * `VOICE.SIGNAL.NO_MEMBER_ROUTE`, appeared in no voice section at all. Events state their own
 * section now. Found by adversarial review (P3-005).
 */
test("an event goes where it says it belongs, not where its target or its wording suggests", () => {
  const uiVoice = ev({ target: "catcoms_ui", section: "voice", view: "voice", code: "VOICE.ICE.FAILED" });
  assert.ok(inView(uiVoice, "voice"), "a webview voice event is a voice event");
  assert.ok(!isFrontend(uiVoice), "and is not filed under the process that emitted it");

  // A backend line that merely mentions the word is not a voice event.
  const mentions = bridged("stored the voice memo attachment", { section: "files", view: "storage" });
  assert.ok(!inView(mentions, "voice"));
  assert.ok(inView(mentions, "storage"));

  assert.ok(isFrontend(ev({ section: "ui", view: "frontend" })));
  assert.ok(inView(ev({ section: "join", view: "network" }), "network"), "join is a network question");
});

test("debug is captured but off by default, so the net firehose is one click away and not on", () => {
  assert.deepEqual(DEFAULT_LEVELS, ["ERROR", "WARN", "INFO"]);
});

test("a line carries its time, level, section, trace and structured fields", () => {
  const line = eventLine(
    bridged("dial failed", { level: "WARN", fields: [f("error", "network unreachable")] }),
    makeAliases(),
    false,
  );
  assert.match(line, /WARN /);
  assert.match(line, /transport/);
  assert.match(line, /dial failed error=network unreachable/);
});

/**
 * A migrated call site's headline is its code, and everything that qualifies the outcome travels
 * with it. This is the whole return on the instrumentation: the old projection kept four characters
 * of the trace and dropped the phase, the duration, the attempt and the references entirely.
 */
test("a migrated event renders its code, phase, duration, attempt and references", () => {
  const e = ev({
    level: "WARN",
    section: "join",
    code: "JOIN.ROUTES.EXHAUSTED",
    phase: "failure",
    operation: "join_server",
    trace: "7f2c000000000001",
    duration_ms: 60123,
    attempt: 4,
    refs: [["server", "server-91ab"]],
    fields: [f("direct_candidates", "4")],
  });
  const text = eventText(e);
  assert.match(text, /^JOIN\.ROUTES\.EXHAUSTED /);
  assert.match(text, /phase=failure/);
  assert.match(text, /duration=60123ms/);
  assert.match(text, /attempt=4/);
  assert.match(text, /server=server-91ab/);
  assert.match(text, /direct_candidates=4/);
  assert.equal(eventParts(e, makeAliases(), false).trace, "7f2c");
  assert.equal(shortTrace("7f2c000000000001"), "7f2c");
});

test("an un-migrated line still reads as its prose, and is tellable apart from a migrated one", () => {
  // Most of the record is still bridged, so the console has to stay readable against it. The
  // substitution keys on the bridge's code, not on "has a message field", so a structured event
  // that happened to carry one would still show what it is.
  assert.equal(eventText(bridged("dial failed", { fields: [f("peer", "abc")] })), "dial failed peer=abc");
  const structured = ev({ code: "NET.DIAL.FAILED", fields: [f("message", "ignore me")] });
  assert.match(eventText(structured), /^NET\.DIAL\.FAILED message=ignore me$/);
});

test("a phase of observation is not printed, because every bare event would carry it", () => {
  assert.equal(eventText(ev({ code: "NET.LISTEN" })), "NET.LISTEN");
});

test("a frontend line drops the target, which would read catcoms_ui on every row", () => {
  const line = eventLine(
    bridged("voice signal failed", { target: "catcoms_ui", section: "ui", view: "frontend" }),
    makeAliases(),
    false,
  );
  assert.ok(!line.includes("catcoms_ui"), line);
  assert.match(line, /voice signal failed/);
});

test("the rendered parts are the source of truth and the joined line agrees with them", () => {
  // Rendering used to slice the joined line back apart by counting characters, and the drift
  // showed up as the level printed twice in the attention list.
  const e = bridged("dial failed", { level: "WARN", fields: [f("addr", "/ip6/2601::1/udp/1")] });
  const a = makeAliases();
  const p = eventParts(e, a, false);
  assert.equal(p.level, "WARN");
  assert.equal(p.target, "catcoms_net");
  assert.equal(p.section, "transport");
  assert.equal(p.text, "dial failed addr=/ip6/2601::1/udp/1");
  const line = eventLine(e, a, false);
  assert.ok(line.includes(p.ts) && line.includes(p.target) && line.includes(p.text), line);
  assert.equal(line.match(/WARN/g)?.length, 1, "the level appears exactly once");
});

test("parts redact their text and drop the target for a frontend event", () => {
  const a = makeAliases();
  const p = eventParts(bridged("dial 203.0.113.9", { section: "ui", view: "frontend", target: "catcoms_ui" }), a, true);
  assert.equal(p.target, "");
  assert.equal(p.text, "dial [ip 1]");
});

test("fields alone still render when an event has nothing but its code", () => {
  assert.equal(eventText(ev({ code: "NET.X", fields: [f("peer", "abc")] })), "NET.X peer=abc");
});

test("redaction gives each distinct value a stable alias, so correlation survives", () => {
  // The whole diagnosis of the hour-long isolation was "it keeps dialling the same two
  // addresses". A mask that renders both as [redacted] destroys exactly that.
  const a = makeAliases();
  const first = redactText("dial /ip4/203.0.113.9/udp/1 failed", a);
  const second = redactText("dial /ip4/203.0.113.9/udp/1 failed again", a);
  const other = redactText("dial /ip4/198.51.100.7/udp/1 failed", a);
  assert.match(first, /\[ip 1\]/);
  assert.ok(second.includes("[ip 1]"), "the same address keeps its alias");
  assert.ok(other.includes("[ip 2]"), "a different address gets a different one");
  assert.ok(!first.includes("203.0.113.9"));
});

test("an IPv6 literal is masked whole, not chewed into hex pieces", () => {
  const a = makeAliases();
  const out = redactText("[2601:441:4581:a5c0:b81d:9e0b:cab1:de04]:23123 unreachable", a);
  assert.ok(!out.includes("2601:441"), out);
  assert.match(out, /\[ip 1\]/);
});

test("peer ids are masked in both the shapes the app prints them in", () => {
  const a = makeAliases();
  const b58 = redactText("local_peer_id=12D3KooWSaXFXMFgkGxgBF6UPEojspeSj2KaDiP4ks5poLzieKKN", a);
  assert.match(b58, /\[peer 1\]/);
  assert.ok(!b58.includes("12D3Koo"), b58);
  const hex = redactText("peer=2b5df389", a);
  assert.match(hex, /\[peer 2\]/);
});

test("a short or truncated peer id is masked too, rather than slipping past a length rule", () => {
  // Caught by driving the real console with the redact toggle on: the rule wanted 20 base58
  // characters after the prefix, so a shorter id rendered in the clear while the toggle claimed
  // the screen was safe to share. Over-masking is the only acceptable direction here.
  const a = makeAliases();
  const out = redactText("peer=12D3KooWFixtureMoss failed", a);
  assert.ok(!out.includes("12D3Koo"), out);
  assert.match(out, /\[peer 1\]/);
});

test("ip and peer aliases are numbered per kind and do not collide", () => {
  const a = makeAliases();
  assert.equal(alias(a, "ip", "1.2.3.4"), "[ip 1]");
  assert.equal(alias(a, "peer", "abcdef12"), "[peer 1]");
  assert.equal(alias(a, "ip", "5.6.7.8"), "[ip 2]");
  assert.equal(alias(a, "ip", "1.2.3.4"), "[ip 1]", "and are stable");
});

test("redaction is off unless asked for", () => {
  const a = makeAliases();
  assert.equal(maybeRedact("1.2.3.4", a, false), "1.2.3.4");
  assert.equal(maybeRedact("1.2.3.4", a, true), "[ip 1]");
});

test("a filter matches the rendered line, so typing what you see works", () => {
  const a = makeAliases();
  const events = [
    bridged("one", { seq: 1, level: "INFO" }),
    bridged("two", { seq: 2, level: "DEBUG" }),
    bridged("three", { seq: 3, level: "WARN", target: "catcoms_sync", section: "sync" }),
  ];
  assert.equal(filterEvents(events, { levels: ["INFO", "WARN"] }, a, false).length, 2);
  assert.equal(filterEvents(events, { levels: ["INFO", "WARN"], target: "sync" }, a, false).length, 1);
  assert.equal(filterEvents(events, { levels: ["INFO", "WARN"], section: "sync" }, a, false).length, 1);
  assert.equal(filterEvents(events, { levels: ["INFO", "WARN"], text: "THREE" }, a, false).length, 1);
});

/**
 * The filter that turns a log viewer into something that answers "what happened when I pressed
 * send". The four characters on an error banner are the whole query, so a prefix match on the short
 * form has to work as well as the full sixteen.
 */
test("a trace filter narrows a feed to one operation, from either form of the id", () => {
  const a = makeAliases();
  const mine = "7f2c000000000001";
  const events = [
    ev({ seq: 1, code: "IPC.COMMAND.RECEIVED", trace: mine }),
    ev({ seq: 2, code: "NET.CHURN", trace: "64aa000000000009" }),
    ev({ seq: 3, code: "SYNC.POST", trace: mine }),
    ev({ seq: 4, code: "NET.LISTEN" }),
  ];
  assert.deepEqual(
    filterEvents(events, { levels: ["INFO"], trace: "7f2c" }, a, false).map((e) => e.seq),
    [1, 3],
  );
  assert.deepEqual(filterEvents(events, { levels: ["INFO"], trace: mine }, a, false).map((e) => e.seq), [1, 3]);
  // An event with no trace must never be swept in with one that has none typed either.
  assert.deepEqual(traceEvents(events, mine).map((e) => e.seq), [1, 3]);
  assert.deepEqual(traceEvents(events, ""), [], "no trace asked for is not every traceless event");
});

test("a filter can search the masked text while redaction is on", () => {
  const a = makeAliases();
  const events = [bridged("dial 203.0.113.9", { seq: 1 })];
  assert.equal(filterEvents(events, { levels: ["INFO"], text: "[ip 1]" }, a, true).length, 1);
});

// --- capture control --------------------------------------------------------------------------

test("every capture mode says what choosing it does, in the terms of the decision", () => {
  // The one control in the app that changes how much of the user's own network is written down.
  // A mode with no note would be a switch whose consequence is discoverable only by exporting
  // twice and comparing.
  for (const mode of CAPTURE_MODES) {
    assert.ok(captureModeNote(mode).length > 40, `${mode} explains itself`);
  }
  assert.match(captureModeNote("safe"), /pasted in public/);
  assert.match(captureModeNote("enhanced"), /Read a report before sharing it/);
  assert.match(captureModeNote("full"), /forgotten at the next launch/);
  assert.equal(captureModeNote("nonsense"), "");
});

test("the modes that start recording addresses are the ones that ask first", () => {
  assert.ok(captureModeIsRevealing("enhanced"));
  assert.ok(captureModeIsRevealing("full"));
  assert.ok(!captureModeIsRevealing("safe"), "the default must not nag");
  assert.ok(!captureModeIsRevealing("off"));
});

test("appending a page never repeats what is already held and stays bounded", () => {
  const held = [ev({ seq: 1 }), ev({ seq: 2 })];
  const withOverlap = appendEvents(held, [ev({ seq: 2 }), ev({ seq: 3 })], 10);
  assert.deepEqual(withOverlap.map((e) => e.seq), [1, 2, 3], "a re-polled event is not shown twice");
  const bounded = appendEvents(held, [ev({ seq: 3 }), ev({ seq: 4 })], 2);
  assert.deepEqual(bounded.map((e) => e.seq), [3, 4], "the newest survive the cap");
  assert.equal(latestSeq(bounded), 4);
  assert.equal(latestSeq([]), 0, "an empty view asks from the beginning");
});

test("a ring that dropped events says so rather than presenting a gap as quiet", () => {
  assert.equal(dropNote(0, 100), "");
  const note = dropNote(1417, 2000);
  assert.match(note, /last 2,000 of 3,417/);
  assert.match(note, /debug log file keeps everything/);
});

test("a filtered feed shows its own arithmetic, so it cannot look empty by accident", () => {
  assert.equal(shownCount(5, 5), "5");
  assert.equal(shownCount(2, 40), "2 of 40 shown");
});

test("route state tells the four failures apart that the roster showed as one grey dot", () => {
  assert.equal(routeState({ ...route, connected: true }), "connected");
  assert.equal(routeState({ ...route, peer: "" }), "no-record");
  assert.equal(routeState({ ...route, addresses: [] }), "no-route");
  assert.equal(routeState({ ...route, dial_attempts: 3 }), "backing-off");
  assert.equal(routeState(route), "reachable");
});

test("status colour is a job: failure is danger, holding off is advisory", () => {
  assert.equal(routeChip("connected").tone, "ok");
  assert.equal(routeChip("no-record").tone, "danger");
  assert.equal(routeChip("no-route").tone, "danger");
  assert.equal(routeChip("backing-off").tone, "warn");
});

test("an IPv6-only member on a host with no IPv6 route is named as exactly that", () => {
  // The hour of isolation, in one sentence. Every attempt failed instantly and the only trace was
  // a raw sendmsg warning from inside the QUIC stack.
  const v6only: MemberRoute = {
    ...route,
    addresses: ["/ip6/2601:441:4581:a5c0::1/udp/23123/quic-v1"],
    dial_attempts: 8,
    next_dial_in_ms: 900_000,
  };
  const said = routeExplanation(v6only, false);
  assert.match(said, /IPv6/);
  assert.match(said, /no IPv6 route/);
  assert.match(said, /15m/, "and says when it will bother trying again");

  const withV6 = routeExplanation(v6only, true);
  assert.ok(!withV6.includes("no IPv6 route"), "not blamed on the family when the host has one");
});

test("a member with no record is told what is missing and how it arrives", () => {
  const said = routeExplanation({ ...route, peer: "" }, true);
  assert.match(said, /PEX/);
  assert.match(said, /cannot be dialled/);
});

test("durations read as a countdown, and a sub-second wait is not a stuck zero", () => {
  assert.equal(formatDuration(0), "now");
  assert.equal(formatDuration(999), "now");
  assert.equal(formatDuration(42_000), "42s");
  assert.equal(formatDuration(200_000), "3m 20s");
  assert.equal(formatDuration(180_000), "3m");
});

test("a copied bundle states its redaction mode and carries the privacy contract", () => {
  // The person who receives a pasted report never read the footer it came from.
  const text = copyBundle(
    { version: "0.3.0", at: Date.UTC(2026, 7, 23), redacted: true, capture: "safe", session: "eb887278" },
    [{ title: "network", lines: ["a", "b"] }],
  );
  assert.match(text, /redaction: on/);
  // Safe and Enhanced reports look alike and mean very different things, so a report says which it
  // is rather than leaving the reader to infer it from whether an address looks complete.
  assert.match(text, /capture: safe/);
  assert.match(text, /session: eb887278/, "so an excerpt can be matched back to its report");
  assert.match(text, /== NETWORK ==/);
  // Describes what a report may contain rather than promising what it does not. The old sentence
  // ended "never includes ... names", and the report writes every server's name into itself: a
  // false safety label is the difference between someone reviewing a report before pasting it
  // into a public issue and someone not bothering.
  assert.match(text, /Read it before you share it/);
  assert.ok(!/never includes/.test(text), "a report must not promise what it cannot enforce");
  assert.match(copyBundle({ version: "0.3.0", at: 0, redacted: false }, []), /redaction: off/);
});

test("an empty section is labelled rather than silently missing from a report", () => {
  const text = copyBundle({ version: "0.3.0", at: 0, redacted: false }, [{ title: "voice", lines: [] }]);
  assert.match(text, /\(nothing captured\)/);
});

// --- section serialisers ---------------------------------------------------------------------
//
// These carry the same values the console renders, and they are tested separately because that
// equality is the point: a pasted report that disagrees with the screenshot it arrived with makes
// the reader work out which one is lying before they can start on the bug.

const device: DebugDevice = {
  public_ipv4: ["203.0.113.9"],
  public_ipv6: [],
  public_direct: false,
  autonat: "private",
  router_maps: true,
  relay_likely_required: true,
  advice: "Calls to peers behind NAT need a relay.",
};

test("this device's lines say what was observed and what was not", () => {
  const lines = deviceLines(device, makeAliases(), false);
  assert.match(lines.join("\n"), /public ipv4: 203\.0\.113\.9/);
  // Absence has to read as absence. An empty value would look like a value that failed to render.
  assert.match(lines.join("\n"), /public ipv6: \(none\)/);
  assert.match(lines.join("\n"), /directly reachable: no/);
});

test("a device with no report at all yields no lines rather than a row of blanks", () => {
  assert.deepEqual(deviceLines(null, makeAliases(), false), []);
});

test("device lines are redacted by the same rules the screen uses", () => {
  const aliases = makeAliases();
  const shown = maybeRedact("203.0.113.9", aliases, true);
  const lines = deviceLines(device, aliases, true);
  assert.ok(lines[0].includes(shown), `${lines[0]} should carry ${shown}`);
  assert.ok(!lines[0].includes("203.0.113.9"), "the real address must not survive redaction");
});

/** The row lines, as distinct from the findings that now lead each server's block. */
const rowLines = (lines: string[]) => lines.filter((l) => !/\[(WARN|DANGER)\]/.test(l));

test("route lines carry the state, the counters and every candidate address", () => {
  const servers = [{ id: 1, name: "Studio" }];
  const routes = { 1: [{ ...route, dial_attempts: 3, next_dial_in_ms: 42_000 }] };
  const [line] = rowLines(routeLines(servers, routes, makeAliases(), false));
  assert.match(line, /^Studio /);
  assert.match(line, /BACKING OFF/);
  assert.match(line, /fails=3/);
  assert.match(line, /next=42s/);
  assert.match(line, /\/ip4\/203\.0\.113\.9\/udp\/31484\/quic-v1/);
});

test("a member with no address says so rather than trailing off", () => {
  const lines = routeLines([{ id: 1, name: "Studio" }], { 1: [{ ...route, addresses: [] }] }, makeAliases(), false);
  assert.match(rowLines(lines)[0], /\(no address\)/);
});

/**
 * Whoever receives a pasted report should meet the conclusion before the evidence. Asking a reader
 * to re-derive it from a column of multiaddrs is how the original incident stayed undiagnosed for
 * an hour.
 */
test("a copied report leads with what is wrong, then the rows it was drawn from", () => {
  const lines = routeLines(
    [{ id: 1, name: "Studio" }],
    { 1: [{ ...route, addresses: ["/ip6/2001:db8::1/udp/1/quic-v1"], dial_attempts: 2 }] },
    makeAliases(),
    false,
    false,
  );
  assert.match(lines[0], /\[DANGER\] NET\.ROUTE\.NO_IPV6_PATH/);
  assert.match(lines[0], /^Studio /, "each finding names its server, as the rows do");
  assert.ok(rowLines(lines).length === 1, "and the row still follows");
});

test("a healthy server's report is rows only, with nothing to conclude", () => {
  const lines = routeLines(
    [{ id: 1, name: "Studio" }],
    { 1: [{ ...route, connected: true }] },
    makeAliases(),
    false,
  );
  assert.deepEqual(lines, rowLines(lines));
});

test("a server with no members contributes nothing rather than an empty row", () => {
  assert.deepEqual(routeLines([{ id: 1, name: "Studio" }], {}, makeAliases(), false), []);
});

test("voice lines carry every state that decides whether a call is working", () => {
  const peers: DebugVoicePeer[] = [
    { fingerprint: "741af9ff", connection: "connected", ice: "completed", signaling: "stable", path: "relayed" },
  ];
  const [line] = voiceLines(peers, makeAliases(), false);
  assert.match(line, /connection=connected/);
  assert.match(line, /ice=completed/);
  assert.match(line, /signaling=stable/);
  assert.match(line, /path=relayed/);
});

test("outside a call the voice section has nothing to say, and says nothing", () => {
  assert.deepEqual(voiceLines([], makeAliases(), false), []);
});

/**
 * ICE reports `completed` once it stops checking, so treating only `connected` as success would
 * paint a working call as a problem. `closed` is the other trap: a call that ended normally is not
 * a failure, and colouring it like one sends people hunting for a bug that is not there.
 */
test("webrtc chips separate success, in-progress, failure and a call that simply ended", () => {
  assert.equal(webrtcChip("connected").tone, "ok");
  assert.equal(webrtcChip("completed").tone, "ok");
  assert.equal(webrtcChip("stable").tone, "ok");
  assert.equal(webrtcChip("checking").tone, "warn");
  assert.equal(webrtcChip("disconnected").tone, "warn");
  assert.equal(webrtcChip("failed").tone, "danger");
  assert.equal(webrtcChip("closed").tone, "faint");
  assert.equal(webrtcChip("connected").label, "CONNECTED");
});

test("a relayed call is worth noticing but is not a fault", () => {
  assert.deepEqual(mediaPathChip("relayed"), { label: "TURN RELAY", tone: "warn" });
  assert.deepEqual(mediaPathChip("direct"), { label: "DIRECT", tone: "ok" });
  assert.deepEqual(mediaPathChip("unknown"), { label: "UNKNOWN", tone: "faint" });
});

// --- supervised background tasks ----------------------------------------------------------------
//
// Only the server actor was supervised; six other long-lived tasks had their handles dropped. The
// event forwarder is the one that matters most, because it can die while the actor stays perfectly
// healthy: the protocol keeps running and the webview is told none of it. Found by adversarial
// review (P3-009).

const task = (over: Partial<TaskHealth> = {}): TaskHealth => ({
  id: 1,
  kind: "event_forwarder",
  server: 1,
  started_ms: 0,
  last_beat_ms: null,
  state: "running",
  fault: false,
  cause: null,
  ...over,
});

test("a running task reads as working and an ordinary exit is not painted as a crash", () => {
  assert.equal(taskChip(task()).tone, "ok");
  assert.equal(taskChip(task({ state: "exited" })).tone, "faint");
  assert.equal(taskChip(task({ state: "panicked", fault: true })).tone, "danger");
  assert.equal(taskChip(task({ state: "cancelled", fault: true })).tone, "danger");
  // A stall is a suspicion rather than a certainty: the task is still running and merely late.
  assert.equal(taskChip(task({ state: "stalled", fault: true })).tone, "warn");
});

/**
 * "event_forwarder panicked" is precise and tells a user nothing. The reason a dead task is worth
 * surfacing at all is that each one has a visible consequence, and the consequence is what somebody
 * recognises from their own screen.
 */
test("a dead task is described by what the user will see, not by its own name", () => {
  const forwarder = taskConsequence("event_forwarder");
  assert.match(forwarder, /unread badges/);
  assert.match(forwarder, /server itself keeps working/, "the distinction that explains it");
  assert.match(taskConsequence("server_actor"), /stopped entirely/);
  assert.match(taskConsequence("discovery_timer"), /addresses/);
  // The reachability folds share one consequence: the report goes stale, connections do not.
  for (const kind of ["port_mapping_fold", "autonat_fold", "relay_fold", "mesh_observation_fold"]) {
    assert.match(taskConsequence(kind), /Connections are unaffected/, kind);
  }
  // An unrecognised kind still says something true rather than nothing.
  assert.ok(taskConsequence("something_new").length > 20);
});

test("level classes match the css, including the one that is not just lowercase", () => {
  assert.equal(levelClass("ERROR"), "lvl-err");
  assert.equal(levelClass("WARN"), "lvl-warn");
  assert.equal(levelClass("TRACE"), "lvl-trace");
});

// --- saving a report ---------------------------------------------------------------------------
//
// A saved report is evidence. The view holds the newest events; the native ring is deeper, and a
// report that silently stops at the window boundary is how a bug report arrives missing the run-up
// to the failure it describes.

/** A native ring of `total` events, served in pages the way `get_console_log` does. */
function pager(total: number, pageSize = 500) {
  let calls = 0;
  const page = async (afterSeq: number, limit: number) => {
    calls += 1;
    const out: LogEvent[] = [];
    for (let seq = afterSeq + 1; seq <= total && out.length < Math.min(limit, pageSize); seq += 1) {
      out.push(bridged(`e${seq}`, { seq, at_ms: 0, target: "catcoms_sync", section: "sync", view: "backend" }));
    }
    return out;
  };
  return { page, get calls() { return calls; } };
}

test("saving pages the whole ring rather than stopping at what is on screen", async () => {
  const source = pager(1300);
  const all = await collectAllEvents(source.page);
  assert.equal(all.length, 1300);
  assert.equal(all[0].seq, 1);
  assert.equal(all[all.length - 1].seq, 1300);
  assert.equal(source.calls, 3, "500 + 500 + 300, then it stops on the short page");
});

test("an empty ring produces an empty report rather than a hang", async () => {
  assert.deepEqual(await collectAllEvents(pager(0).page), []);
});

test("a ring that exactly fills its last page still terminates", async () => {
  // The off-by-one that would loop: a full final page looks like there may be more.
  const source = pager(1000);
  assert.equal((await collectAllEvents(source.page)).length, 1000);
  assert.equal(source.calls, 3, "the third page comes back empty and ends it");
});

test("a native side that stops advancing cannot spin the export forever", async () => {
  // Defensive: a page that repeats the same sequences would otherwise never terminate.
  let calls = 0;
  const stuck = async () => {
    calls += 1;
    return [bridged("same", { seq: 1, at_ms: 0, target: "t" })];
  };
  const all = await collectAllEvents(stuck, { pageSize: 10 });
  assert.equal(calls, 1, "the sequence did not advance, so it stopped");
  assert.equal(all.length, 1);
});

test("the page budget bounds the export even if pages keep advancing", async () => {
  const source = pager(1_000_000);
  const all = await collectAllEvents(source.page, { pageSize: 500, maxPages: 4 });
  assert.equal(all.length, 2000);
  assert.equal(source.calls, 4);
});

// --- aggregate route diagnosis ------------------------------------------------------------------
//
// routeExplanation answers "what is happening with this member", which is right once you already
// suspect one. The question nobody could answer was the aggregate: a node sat isolated for an hour
// and the evidence was a scattering of raw sendmsg warnings from inside the QUIC stack.

const v6 = (fingerprint: string, over: Partial<MemberRoute> = {}): MemberRoute => ({
  ...route,
  fingerprint,
  addresses: ["/ip6/2001:db8::1/udp/31484/quic-v1"],
  dial_attempts: 3,
  ...over,
});

test("a healthy server produces no findings at all", () => {
  const connected = [{ ...route, connected: true }, { ...route, fingerprint: "b", connected: true }];
  assert.deepEqual(routeFindings(connected, true), []);
});

test("a server with nobody else in it is not a reachability problem", () => {
  assert.deepEqual(routeFindings([], false), []);
});

/** The hour-long isolation, as one sentence instead of a pile of multiaddrs. */
test("v6-only members on a host with no v6 route are named as unreachable forever", () => {
  const found = routeFindings([v6("a"), v6("b")], false);
  const ipv6 = found.find((f) => f.code === "NET.ROUTE.NO_IPV6_PATH");
  assert.ok(ipv6, JSON.stringify(found));
  assert.equal(ipv6.severity, "danger");
  assert.equal(ipv6.affected, 2);
  // The actionable part: this is not something retrying fixes.
  assert.match(ipv6.detail, /Retrying cannot help/);
  assert.ok(!ipv6.detail.includes("2001:db8"), "a finding never carries an address");
});

test("the same members on a host that has IPv6 are not diagnosed that way", () => {
  const found = routeFindings([v6("a"), v6("b")], true);
  assert.equal(found.find((f) => f.code === "NET.ROUTE.NO_IPV6_PATH"), undefined);
});

test("a member with no record is a different problem from one with no address", () => {
  const found = routeFindings(
    [
      { ...route, fingerprint: "a", peer: "" },
      { ...route, fingerprint: "b", addresses: [] },
    ],
    true,
  );
  assert.equal(found.find((f) => f.code === "NET.ROUTE.NO_RECORD")?.affected, 1);
  assert.equal(found.find((f) => f.code === "NET.ROUTE.NO_DIALABLE_ADDRESS")?.affected, 1);
});

test("total isolation is reported, and after the reason for it", () => {
  const found = routeFindings([v6("a"), v6("b")], false);
  const codes = found.map((f) => f.code);
  assert.ok(codes.includes("NET.ROUTE.ISOLATED"));
  // A reader scanning from the top meets the explanation before the symptom.
  assert.ok(
    codes.indexOf("NET.ROUTE.NO_IPV6_PATH") < codes.indexOf("NET.ROUTE.ISOLATED"),
    codes.join(","),
  );
});

test("partial reachability is not reported as isolation", () => {
  const found = routeFindings([v6("a"), { ...route, fingerprint: "b", connected: true }], false);
  assert.equal(found.find((f) => f.code === "NET.ROUTE.ISOLATED"), undefined);
  assert.equal(found.find((f) => f.code === "NET.ROUTE.NO_IPV6_PATH")?.affected, 1);
});

test("a member advertising a mix of families is not blamed on IPv6", () => {
  // One usable v4 candidate means the v6 ones are not the explanation.
  const mixed = v6("a", {
    addresses: ["/ip6/2001:db8::1/udp/1/quic-v1", "/ip4/203.0.113.9/udp/1/quic-v1"],
  });
  assert.equal(
    routeFindings([mixed], false).find((f) => f.code === "NET.ROUTE.NO_IPV6_PATH"),
    undefined,
  );
});

test("a finding reads as a sentence for one member and for several", () => {
  // "1 member(s) advertise" is the sort of thing that makes a reader trust the rest a little less.
  const one = routeFindings([v6("a")], false);
  assert.match(one[0].detail, /^1 member advertises only IPv6/);
  assert.match(one.find((f) => f.code === "NET.ROUTE.ISOLATED").detail, /only other member/);

  const many = routeFindings([v6("a"), v6("b"), v6("c")], false);
  assert.match(many[0].detail, /^3 members advertise only IPv6/);
  assert.match(many.find((f) => f.code === "NET.ROUTE.ISOLATED").detail, /3 other members/);

  const singleNoRecord = routeFindings([{ ...route, peer: "", connected: false }], true);
  assert.match(singleNoRecord[0].detail, /1 member has no signed peer record/);
});
