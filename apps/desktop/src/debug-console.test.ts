import assert from "node:assert/strict";
import test from "node:test";
import {
  BRIDGED_CODE,
  CAPTURE_MODES,
  DEFAULT_LEVELS,
  appendEvents,
  alias,
  belowFileFilter,
  captureModeIsRevealing,
  captureModeNote,
  captureHistory,
  copyBundle,
  debugLogSettingsStatus,
  dropNote,
  retentionStatus,
  sinkLines,
  sinkSummary,
  eventLine,
  eventParts,
  eventText,
  exportMasksAddresses,
  filterEvents,
  formatDuration,
  inView,
  isFrontend,
  latestSeq,
  makeAliases,
  maybeRedact,
  redactText,
  routeChip,
  routeDisplayChip,
  routeOverviewCounts,
  routeActionLabel,
  routeActionScopeLabel,
  routeExplanation,
  routeIndirectEvidence,
  routePathLabel,
  routeState,
  shortTrace,
  shownCount,
  taskChip,
  taskConsequence,
  traceEvents,
  collectAllEvents,
  createRepollingTask,
  createSerialTaskQueue,
  deviceLines,
  finishPublicDiagnosticsIssue,
  levelClass,
  mediaPathChip,
  memberRoutesVisible,
  mergeMemberRoutePoll,
  mergeMemberRouteRead,
  routeFindings,
  routeGroupState,
  routeHistoricalAge,
  routeIsConnected,
  routeLines,
  shouldRefreshMemberRoutes,
  voiceLines,
  webrtcChip,
  type DebugDevice,
  type DebugLogSink,
  type DebugVoicePeer,
  type Aliases,
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
    fields_dropped: 0,
    capture: "safe",
    capture_epoch: 1,
    ...over,
  };
}

/** An un-migrated `tracing` event: prose in a `message` field, under the bridge's code. */
function bridged(message: string, over: Partial<LogEvent> = {}): LogEvent {
  return ev({
    code: BRIDGED_CODE,
    ...over,
    fields: [{ name: "message", value: message, kind: "bridged", sensitive: false }, ...(over.fields ?? [])],
  });
}

/** A rendered field, for the shape `LogEvent.fields` takes. */
const f = (name: string, value: string, sensitive = false) => ({ name, value, kind: "text", sensitive });

const route: MemberRoute = {
  fingerprint: "741af9ff",
  peer: "2b5df389",
  addresses: ["/ip4/203.0.113.9/udp/31484/quic-v1"],
  seq: 4,
  connected: false,
  dial_attempts: 0,
  next_dial_in_ms: 0,
  health: "claimed_peer_dial_eligible",
  binding: "self_asserted",
  active_paths: [],
  last_success: null,
  candidate_families: ["ipv4"],
  candidate_transports: ["quic_v1"],
  actions: [],
  indirect_health: "unknown",
  indirect_witnesses: 0,
  indirect_age_ms: null,
  reciprocal_pending: false,
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
  assert.match(line, /capture=safe#1/);
});

test("mixed capture history is explicit per row and in report metadata", () => {
  const events = [
    ev({ seq: 1, capture: "safe", capture_epoch: 4 }),
    ev({ seq: 2, capture: "enhanced", capture_epoch: 5 }),
    ev({ seq: 3, capture: "safe", capture_epoch: 4 }),
  ];
  assert.deepEqual(captureHistory(events), ["safe#4", "enhanced#5"]);
  const report = copyBundle(
    {
      version: "1",
      at: 0,
      redacted: true,
      capture: "enhanced",
      captureHistory: captureHistory(events),
    },
    [{ title: "network", lines: events.map((event) => eventLine(event, makeAliases("fixed"), false)) }],
  );
  assert.match(report, /current capture: enhanced/);
  assert.match(report, /capture history: safe#4, enhanced#5/);
  assert.match(report, /capture=safe#4/);
  assert.match(report, /capture=enhanced#5/);
});

test("an overlapping refresh waits for one guaranteed follow-up poll", async () => {
  let finishFirst!: () => void;
  const firstGate = new Promise<void>((resolve) => (finishFirst = resolve));
  let runs = 0;
  const poll = createRepollingTask(async () => {
    runs += 1;
    if (runs === 1) await firstGate;
  });

  const first = poll();
  const refresh = poll();
  assert.equal(runs, 1, "the overlap coalesces while the first native read is active");
  finishFirst();
  await refresh;
  await first;
  assert.equal(runs, 2, "the mode-change refresh receives one post-flight read");
});

test("capture mutations execute and apply in request order", async () => {
  let finishFirst!: () => void;
  const firstGate = new Promise<void>((resolve) => (finishFirst = resolve));
  const queue = createSerialTaskQueue();
  const started: string[] = [];
  let applied = "safe";

  const enhanced = queue(async () => {
    started.push("enhanced");
    await firstGate;
    applied = "enhanced";
  });
  const off = queue(async () => {
    started.push("off");
    applied = "off";
  });

  await Promise.resolve();
  assert.deepEqual(started, ["enhanced"], "the later native mutation cannot overtake the first");
  finishFirst();
  await Promise.all([enhanced, off]);
  assert.deepEqual(started, ["enhanced", "off"]);
  assert.equal(applied, "off", "the newest requested native mode drives the final UI state");
});

test("a failed public-issue clipboard write never produces a copied confirmation", async () => {
  const failure = new Error("clipboard denied");
  assert.deepEqual(
    await finishPublicDiagnosticsIssue({ report: "exact native envelope", truncated: true }, async () => {
      throw failure;
    }),
    {
      status: "issue opened; full report could not be copied: Error: clipboard denied",
      manualReport: "exact native envelope",
    },
  );
  assert.deepEqual(
    await finishPublicDiagnosticsIssue({ report: "already fits", truncated: false }, () => {
      assert.fail("an untruncated issue needs no clipboard fallback");
    }),
    { status: "issue opened for review", manualReport: "" },
  );
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
  assert.equal(
    eventText(bridged("dial failed", { fields: [f("peer", "abc")] })),
    "dial failed peer=abc capture=safe#1",
  );
  const structured = ev({ code: "NET.DIAL.FAILED", fields: [f("message", "ignore me")] });
  assert.match(eventText(structured), /^NET\.DIAL\.FAILED message=ignore me capture=safe#1$/);
});

test("a phase of observation is not printed, because every bare event would carry it", () => {
  assert.equal(eventText(ev({ code: "NET.LISTEN" })), "NET.LISTEN capture=safe#1");
});

/**
 * An event that hit the field cap shows a shortened list, and the shortening has to be the one
 * thing the line does mention: otherwise a reader takes what is there for the whole of it. The
 * native renderings say the same, and this is the third.
 */
test("a line whose event lost fields to the cap says so", () => {
  const kept = ev({ code: "NET.X", fields: [f("a", "1")] });
  assert.equal(eventText(kept), "NET.X a=1 capture=safe#1", "and says nothing when nothing was lost");
  const trimmed = ev({ code: "NET.X", fields: [f("a", "1")], fields_dropped: 9 });
  assert.equal(eventText(trimmed), "NET.X a=1 fields_dropped=9 capture=safe#1");
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
  assert.equal(p.text, "dial failed addr=/ip6/2601::1/udp/1 capture=safe#1");
  const line = eventLine(e, a, false);
  assert.ok(line.includes(p.ts) && line.includes(p.target) && line.includes(p.text), line);
  assert.equal(line.match(/WARN/g)?.length, 1, "the level appears exactly once");
});

test("parts redact their text and drop the target for a frontend event", () => {
  const a = makeAliases();
  const p = eventParts(bridged("dial 203.0.113.9", { section: "ui", view: "frontend", target: "catcoms_ui" }), a, true);
  assert.equal(p.target, "");
  assert.match(p.text, /^dial \[ip [0-9a-f]{6}\] capture=safe#1$/);
});

test("fields alone still render when an event has nothing but its code", () => {
  assert.equal(
    eventText(ev({ code: "NET.X", fields: [f("peer", "abc")] })),
    "NET.X peer=abc capture=safe#1",
  );
});

test("redaction gives each distinct value a stable alias, so correlation survives", () => {
  // The whole diagnosis of the hour-long isolation was "it keeps dialling the same two
  // addresses". A mask that renders both as [redacted] destroys exactly that.
  const a = makeAliases();
  const first = redactText("dial /ip4/203.0.113.9/udp/1 failed", a);
  const second = redactText("dial /ip4/203.0.113.9/udp/1 failed again", a);
  const other = redactText("dial /ip4/198.51.100.7/udp/1 failed", a);
  const mask = first.match(/\[ip [0-9a-f]{6}\]/)?.[0];
  assert.ok(mask, first);
  assert.ok(second.includes(mask), "the same address keeps its alias");
  assert.ok(!other.includes(mask), "a different address gets a different one");
  assert.ok(!first.includes("203.0.113.9"));
});

test("an IPv6 literal is masked whole, not chewed into hex pieces", () => {
  const a = makeAliases();
  const out = redactText("[2601:441:4581:a5c0:b81d:9e0b:cab1:de04]:23123 unreachable", a);
  assert.ok(!out.includes("2601:441"), out);
  assert.match(out, /\[ip [0-9a-f]{6}\]/);
});

test("compressed, zoned, bracketed and mapped IPv6 forms are parsed and masked", () => {
  const a = makeAliases("ipv6-parser");
  for (const input of [
    "loopback ::1 failed",
    "route 2001:db8::1 unreachable",
    "link fe80::1%eth0 closed",
    "socket [2001:db8::1]:443 refused",
    "mapped ::ffff:192.0.2.128 refused",
    "multiaddr /ip6/2001:db8::1/tcp/443/p2p/12D3KooWHp1hLNjWf4ZM4eLaiUdMGTbGnXDDDkhnE56P9CRbHx8E",
  ]) {
    const out = redactText(input, a);
    assert.match(out, /\[ip [0-9a-f]{6}\]/, out);
    assert.doesNotMatch(out, /2001:db8|fe80::|::ffff|192\.0\.2\.128/, out);
  }
});

test("valid Qm peer ids and both peer ids in a relay multiaddr are parser-masked", () => {
  const a = makeAliases("peer-parser");
  const qm = "QmYwAPJzv5CZsnAzt8auVZRnGi2C5DLp8KjN6Jw2eZP9hK";
  const relay = "12D3KooWBGfsSWvGFAJeTz3oBPeRFbSadCwedBJvJ6AFAJtfkSD2";
  const target = "12D3KooWPiZxJceHKQBZcd79cYdqybt5ijzRGHveTKa3CaEESxVb";
  const out = redactText(
    `${qm} /ip4/203.0.113.9/tcp/443/p2p/${relay}/p2p-circuit/p2p/${target}`,
    a,
  );
  assert.doesNotMatch(out, /QmYw|12D3Koo|203\.0\.113\.9/, out);
  assert.equal(out.match(/\[peer [0-9a-f]{6}\]/g)?.length, 3, out);
});

test("peer ids are masked in both the shapes the app prints them in", () => {
  const a = makeAliases();
  const b58 = redactText("local_peer_id=12D3KooWSaXFXMFgkGxgBF6UPEojspeSj2KaDiP4ks5poLzieKKN", a);
  assert.match(b58, /\[peer [0-9a-f]{6}\]/);
  assert.ok(!b58.includes("12D3Koo"), b58);
  const hex = redactText("peer=2b5df389", a);
  assert.match(hex, /\[peer [0-9a-f]{6}\]/);
  assert.notEqual(b58, hex, "two different ids are two different aliases");
});

test("a short or truncated peer id is masked too, rather than slipping past a length rule", () => {
  // Caught by driving the real console with the redact toggle on: the rule wanted 20 base58
  // characters after the prefix, so a shorter id rendered in the clear while the toggle claimed
  // the screen was safe to share. Over-masking is the only acceptable direction here.
  const a = makeAliases();
  const out = redactText("peer=12D3KooWFixtureMoss failed", a);
  assert.ok(!out.includes("12D3Koo"), out);
  assert.match(out, /\[peer [0-9a-f]{6}\]/);
});

test("aliases are per kind and per value, and do not collide across kinds", () => {
  const a = makeAliases("fixed-salt");
  const first = alias(a, "ip", "1.2.3.4");
  assert.match(first, /^\[ip [0-9a-f]{6}\]$/);
  assert.equal(alias(a, "ip", "1.2.3.4"), first, "and are stable");
  assert.notEqual(alias(a, "ip", "5.6.7.8"), first, "a different value is a different alias");
  // `[ip …]` and `[peer …]` must not read as the same thing even for the same underlying string.
  assert.notEqual(alias(a, "peer", "1.2.3.4"), first);
});

/**
 * The property the review asks for by name, and the reason the counter had to go.
 *
 * Aliases were minted in the order values were first seen, so merely visiting a section, typing a
 * filter or rendering a route before pressing Save decided which address got `[ip 1]`. The same
 * events then exported differently depending on where the user had clicked first, and a report that
 * cannot be diffed against another cannot be compared between two peers.
 */
test("what gets exported does not depend on where the user clicked first", () => {
  const events = [
    bridged("dial failed 198.51.100.7", { seq: 1, level: "WARN" }),
    bridged("dial failed 203.0.113.9", { seq: 2, level: "WARN" }),
    bridged("peer 12D3KooWFixtureMoss unreachable", { seq: 3, level: "WARN" }),
  ];
  const routes = { 1: [{ ...route, addresses: ["/ip4/203.0.113.9/udp/1/quic-v1"] }] };
  const servers = [{ id: 1, name: "Studio" }];

  // One salt, because two consoles are two sessions and are *meant* to differ; what must not differ
  // is two exports from the same session that were navigated differently.
  const report = (visit: (a: Aliases) => void) => {
    const a = makeAliases("one-session");
    visit(a);
    return copyBundle({ version: "0.3.0", at: 0, redacted: true, capture: "safe" }, [
      { title: "network", lines: routeLines(servers, routes, a, true) },
      { title: "backend", lines: events.map((e) => eventLine(e, a, true)) },
    ]);
  };

  const straightToSave = report(() => {});
  const readTheFeedFirst = report((a) => {
    for (const e of [...events].reverse()) eventLine(e, a, true);
  });
  const openedTheNetworkTab = report((a) => {
    routeLines(servers, routes, a, true);
    eventLine(events[2], a, true);
  });

  assert.equal(readTheFeedFirst, straightToSave);
  assert.equal(openedTheNetworkTab, straightToSave);
  // And the masking actually happened, so this is not three identical unredacted reports.
  assert.ok(!straightToSave.includes("203.0.113.9"), straightToSave);
  assert.match(straightToSave, /\[ip [0-9a-f]{6}\]/);
});

test("redaction is off unless asked for", () => {
  const a = makeAliases();
  assert.equal(maybeRedact("1.2.3.4", a, false), "1.2.3.4");
  assert.match(maybeRedact("1.2.3.4", a, true), /^\[ip [0-9a-f]{6}\]$/);
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
  // Typing the alias off the screen is a legitimate thing to do, so the filter has to match what
  // is rendered rather than what is underneath.
  const shown = maybeRedact("203.0.113.9", a, true);
  assert.equal(filterEvents(events, { levels: ["INFO"], text: shown }, a, true).length, 1);
});

// --- capture control --------------------------------------------------------------------------

test("every capture mode says what choosing it does, in the terms of the decision", () => {
  // The one control in the app that changes how much of the user's own network is written down.
  // A mode with no note would be a switch whose consequence is discoverable only by exporting
  // twice and comparing.
  for (const mode of CAPTURE_MODES) {
    assert.ok(captureModeNote(mode).length > 40, `${mode} explains itself`);
  }
  assert.match(captureModeNote("safe"), /discarded at capture time/);
  assert.match(captureModeNote("safe"), /separate native allowlist report/);
  assert.match(captureModeNote("enhanced"), /Read local reports before sharing them/);
  assert.match(captureModeNote("full"), /forgotten at the next launch/);
  assert.equal(captureModeNote("nonsense"), "");
});

test("the modes that start recording addresses are the ones that ask first", () => {
  assert.ok(captureModeIsRevealing("enhanced"));
  assert.ok(captureModeIsRevealing("full"));
  assert.ok(!captureModeIsRevealing("safe"), "the default must not nag");
  assert.ok(!captureModeIsRevealing("off"));
});

test("safe exports mask address tables even when the screenshot toggle is off", () => {
  assert.equal(exportMasksAddresses("safe", false), true);
  assert.equal(exportMasksAddresses("off", false), true);
  assert.equal(exportMasksAddresses("enhanced", false), false);
  assert.equal(exportMasksAddresses("full", false), false);
  assert.equal(exportMasksAddresses("enhanced", true), true);
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

// --- retention and sink health (P3-020) -----------------------------------------------------

/** A file sink's own account of itself, healthy unless a test says otherwise. */
function sink(over: Partial<DebugLogSink> = {}): DebugLogSink {
  return {
    enabled: true,
    active: true,
    state: "active",
    error: "",
    file: "debug_log_20260823_120000.txt",
    events_written: 1284,
    bytes_written: 190_432,
    events_dropped: 0,
    events_truncated: 0,
    queue_high_water: 12,
    session_quota_bytes: 52_428_800,
    ...over,
  };
}

/**
 * The claim P3-020 removed, in the forms a well-meaning edit would put it back.
 *
 * The notice used to end "The debug log file keeps everything", which was false whenever logging
 * was off, the sink had failed, its queue had dropped, its quota had stopped it, or the event was
 * below its filter. Every state below asserts against this, so restoring the promise anywhere in
 * the notice fails the whole group rather than one forgotten case.
 */
function promisesCompleteness(text: string): boolean {
  const t = text.toLowerCase();
  return (
    t.includes("keeps everything") ||
    t.includes("has everything") ||
    t.includes("holds everything") ||
    t.includes("kept everything") ||
    /file (?:keeps|has|holds|contains) (?:everything|it all|them all|the rest)/.test(t)
  );
}

test("a ring that dropped events says so rather than presenting a gap as quiet", () => {
  assert.equal(dropNote(0, 100), "", "nothing dropped is nothing to announce");
  const note = dropNote(1417, 2000, sink());
  assert.match(note, /last 2,000 of 3,417/);
  assert.ok(!promisesCompleteness(note), "and never promises the file made the gap good");
});

test("a console that has not read the sink says the file is unknown rather than reassuring", () => {
  const s = retentionStatus({ dropped: 10, kept: 2000, sink: null });
  assert.equal(s.file, "unknown");
  assert.match(s.sink, /has not been read/);
  assert.ok(!promisesCompleteness(s.text));
});

test("file logging switched off is named as that, not hedged into a maybe", () => {
  const s = retentionStatus({ dropped: 10, kept: 2000, sink: sink({ enabled: false, active: false, state: "stopped" }) });
  assert.equal(s.file, "none", "nothing was written, so nothing may be claimed for it");
  assert.match(s.sink, /switched off/, "the user's own choice is named rather than described as a failure");
  assert.match(s.sink, /gone/);
  assert.ok(!promisesCompleteness(s.text));
});

test("a sink that failed to open is not offered as a fallback, and says why", () => {
  const s = retentionStatus({
    dropped: 10,
    kept: 2000,
    sink: sink({ active: false, state: "failed", file: "", error: "permission denied opening the diagnostics directory" }),
  });
  assert.equal(s.file, "none");
  assert.equal(s.tone, "danger");
  assert.match(s.sink, /permission denied/, "the actionable reason survives, rather than becoming 'logging failed'");
  assert.ok(!promisesCompleteness(s.text));
});

test("logging asked for but never started is a different sentence from logging turned off", () => {
  const s = retentionStatus({ dropped: 10, kept: 2000, sink: sink({ enabled: true, active: false, state: "stopped", file: "" }) });
  assert.equal(s.file, "none");
  assert.match(s.sink, /switched on but this session opened no file/);
  assert.match(s.sink, /when the app starts/, "and says the thing that would fix it");
  assert.ok(!promisesCompleteness(s.text));
});

test("settings separates the current log writer from the next-launch preference", () => {
  const writing = debugLogSettingsStatus(sink());
  assert.equal(writing.label, "Writing now");
  assert.match(writing.detail, /1,284 entries/);
  assert.equal(writing.restartNotice, "", "matching current and next-launch states need no second row");

  const stopping = debugLogSettingsStatus(sink({ enabled: false }));
  assert.equal(stopping.label, "Writing now", "the checkbox cannot rewrite this process's subscriber");
  assert.match(stopping.restartNotice, /off for the next launch/);
  assert.match(stopping.restartNotice, /keep writing until you restart/);

  const starting = debugLogSettingsStatus(sink({ active: false, state: "stopped", file: "" }));
  assert.equal(starting.label, "Not writing");
  assert.match(starting.restartNotice, /on for the next launch/);
  assert.match(starting.restartNotice, /Restart Mewtual/);
});

test("settings gives failed and degraded writers plain current-session labels", () => {
  const failed = debugLogSettingsStatus(
    sink({ active: false, state: "failed", error: "permission denied", file: "" }),
  );
  assert.equal(failed.label, "Could not write log");
  assert.equal(failed.detail, "permission denied");
  assert.equal(failed.restartNotice, "", "a restart must not be promised to repair a real failure");

  const degraded = debugLogSettingsStatus(sink({ state: "degraded", events_dropped: 7 }));
  assert.equal(degraded.label, "Writing with gaps");
  assert.match(degraded.detail, /7 record\(s\) did not reach/);
});

test("a sink with a non-zero dropped count cannot imply it kept what the ring lost", () => {
  const s = retentionStatus({ dropped: 10, kept: 2000, sink: sink({ state: "degraded", events_dropped: 37 }) });
  assert.equal(s.file, "partial", "it is writing, so 'may hold some' is the strongest honest claim");
  assert.equal(s.tone, "warn");
  assert.ok(s.caveats.some((c) => c.code === "LOG.FILE.DROPPED" && c.text.includes("37")));
  assert.ok(!promisesCompleteness(s.text));
});

test("a truncated record is reported apart from a dropped one, because they differ to a reader", () => {
  const s = retentionStatus({ dropped: 10, kept: 2000, sink: sink({ events_truncated: 4 }) });
  const cut = s.caveats.find((c) => c.code === "LOG.FILE.TRUNCATED");
  assert.ok(cut && cut.text.includes("4"), "present, and says so, unlike a dropped one");
  assert.ok(!s.caveats.some((c) => c.code === "LOG.FILE.DROPPED"), "and is not counted as a drop");
});

test("a session near its file quota says so, because that is where the history stops", () => {
  const quiet = retentionStatus({ dropped: 10, kept: 2000, sink: sink({ bytes_written: 1_000 }) });
  assert.ok(!quiet.caveats.some((c) => c.code === "LOG.FILE.QUOTA"), "an empty file is not a warning");
  const full = retentionStatus({
    dropped: 10,
    kept: 2000,
    sink: sink({ bytes_written: 52_428_800, session_quota_bytes: 52_428_800 }),
  });
  const quota = full.caveats.find((c) => c.code === "LOG.FILE.QUOTA");
  assert.ok(quota && /nothing further is being written/.test(quota.text));
  assert.ok(!promisesCompleteness(full.text));
});

/**
 * The narrowing that makes "the file has the rest" wrong even for a perfectly healthy sink. The
 * ring takes `catcoms_net` at debug so dial failures are visible in the app; the file holds it at
 * info so a log a user pastes to somebody else is not a list of every address they have ever seen.
 */
test("the file's narrower filter is stated, and counted against what is on screen", () => {
  const held = [
    ev({ level: "DEBUG", target: "catcoms_net" }),
    ev({ level: "DEBUG", target: "catcoms_storage" }),
    ev({ level: "DEBUG", target: "catcoms_sync" }),
    ev({ level: "INFO", target: "catcoms_net" }),
  ];
  const s = retentionStatus({ dropped: 10, kept: held.length, sink: sink(), events: held });
  const narrow = s.caveats.find((c) => c.code === "LOG.FILE.NARROWER_FILTER");
  assert.ok(narrow, "a healthy sink still has a filter, and it is narrower than this console's");
  assert.match(narrow!.text, /\b2\b/, "the two debug lines the file never saw are counted, not asserted");
  assert.ok(!promisesCompleteness(s.text));

  // Still said when there is nothing on screen to count: the configuration is the fact, and a
  // caller that cannot pass its events should not be told the file's filter matches the ring's.
  const unmeasured = retentionStatus({ dropped: 10, kept: 2000, sink: sink() });
  assert.match(
    unmeasured.caveats.find((c) => c.code === "LOG.FILE.NARROWER_FILTER")!.text,
    /narrower than this console's/,
  );
});

test("the file's filter is mirrored per target, not guessed from the level alone", () => {
  assert.ok(belowFileFilter(ev({ level: "DEBUG", target: "catcoms_net" })), "transport debug is ring only");
  assert.ok(belowFileFilter(ev({ level: "DEBUG", target: "catcoms_replication" })));
  assert.ok(!belowFileFilter(ev({ level: "DEBUG", target: "catcoms_sync" })), "product layers reach the file");
  assert.ok(!belowFileFilter(ev({ level: "DEBUG", target: "catcoms_ui::voice" })), "and so do their submodules");
  assert.ok(!belowFileFilter(ev({ level: "INFO", target: "catcoms_net" })), "info reaches it from anywhere");
  assert.ok(belowFileFilter(ev({ level: "TRACE", target: "catcoms_app" })), "trace reaches it from nowhere");
});

test("events the capture settings excluded are named, because neither store has them", () => {
  const s = retentionStatus({ dropped: 10, kept: 2000, sink: sink(), filtered: 812 });
  const excluded = s.caveats.find((c) => c.code === "LOG.CAPTURE.EXCLUDED");
  assert.ok(excluded && excluded.text.includes("812"));
  assert.match(excluded!.text, /neither this console nor the file/);

  // And still said when the file is not writing at all: raising the capture mode now cannot bring
  // back an event that was never built, whatever the sink is doing.
  const off = retentionStatus({ dropped: 10, kept: 2000, sink: sink({ enabled: false, state: "stopped" }), filtered: 5 });
  assert.ok(off.caveats.some((c) => c.code === "LOG.CAPTURE.EXCLUDED"));
});

test("the sink summary is decided by what the writer did, never by the preference", () => {
  // The disagreement the whole record exists to show: the preference is off while the sink opened
  // before the toggle is still writing. A summary taken from `enabled` would call this stopped.
  assert.equal(sinkSummary(sink({ enabled: false, state: "active" })).tone, "ok");
  assert.equal(sinkSummary(sink({ enabled: true, active: false, state: "failed", error: "disk full" })).tone, "danger");
  assert.match(sinkSummary(sink({ state: "failed", error: "disk full" })).text, /disk full/);
  assert.match(sinkSummary(sink({ state: "failed", error: "" })).text, /reason was not recorded/);
  assert.equal(sinkSummary(sink({ state: "degraded", events_dropped: 3 })).tone, "warn");
  assert.equal(sinkSummary(sink({ enabled: false, active: false, state: "stopped" })).tone, "faint");
  assert.match(sinkSummary(null).text, /has not been read/);
});

test("sink health copies the same numbers it shows, including the ones that are zero", () => {
  const lines = sinkLines(sink({ events_dropped: 0, error: "" })).join("\n");
  assert.match(lines, /dropped: 0/, "a zero drop count is evidence, so it is stated rather than omitted");
  assert.match(lines, /sink state: active/);
  assert.ok(!/last error/.test(lines), "and an error line is absent when there is no error");
  assert.match(sinkLines(null)[0], /not read/);
});

test("a filtered feed shows its own arithmetic, so it cannot look empty by accident", () => {
  assert.equal(shownCount(5, 5), "5");
  assert.equal(shownCount(2, 40), "2 of 40 shown");
});

test("backend route health stays authoritative and unknown values fail to unknown", () => {
  assert.equal(routeState({ ...route, health: "claimed_peer_connected_direct" }), "direct");
  assert.equal(routeState({ ...route, health: "claimed_peer_connected_relay" }), "relay");
  assert.equal(routeState({ ...route, health: "claimed_peer_connected_other" }), "connected-other");
  assert.equal(routeState({ ...route, health: "no_peer_record" }), "no-record");
  assert.equal(routeState({ ...route, health: "claimed_peer_has_no_route" }), "no-route");
  assert.equal(routeState({ ...route, health: "claimed_peer_dial_cooling_down" }), "cooldown");
  assert.equal(routeState(route), "dial-eligible");
  assert.equal(routeState({ ...route, health: "newer_backend_state" }), "unknown");
});

test("unknown future health never becomes a connected or isolated verdict", () => {
  const future = { ...route, connected: true, health: "newer_backend_state" };
  assert.equal(routeIsConnected(future), false, "the legacy boolean cannot override typed health");
  assert.equal(routeGroupState([future]), "unknown");
  assert.equal(
    routeFindings([future]).find(
      (finding) => finding.code === "NET.ROUTE.NO_LIVE_MEMBER_CONNECTION",
    ),
    undefined,
  );
});

test("status colour is a job: failure is danger, holding off is advisory", () => {
  assert.equal(routeChip("direct").tone, "ok");
  assert.equal(routeChip("relay").tone, "ok");
  assert.equal(routeChip("no-record").tone, "danger");
  assert.equal(routeChip("no-route").tone, "danger");
  assert.equal(routeChip("cooldown").tone, "warn");
  assert.equal(routeChip("unknown").label, "UNKNOWN");
});

test("an unavailable route read turns every retained status into an explicit warning snapshot", () => {
  assert.deepEqual(routeDisplayChip("direct", true), { label: "LAST: DIRECT PATH", tone: "warn" });
  assert.deepEqual(routeDisplayChip("no-route", true), { label: "LAST: NO ROUTE", tone: "warn" });
  assert.deepEqual(routeDisplayChip("direct", false), routeChip("direct"));
});

test("overview current counts disappear when their only rows are a retained snapshot", () => {
  assert.deepEqual(routeOverviewCounts([{ ...route, health: "claimed_peer_connected_direct" }], true), {
    connected: "—",
    roster: "—",
  });
  assert.deepEqual(routeOverviewCounts([{ ...route, health: "claimed_peer_connected_direct" }], false), {
    connected: "1 / 1",
    roster: "2",
  });
});

test("connected wording preserves the self-asserted transport binding caveat", () => {
  const said = routeExplanation({ ...route, connected: true, health: "claimed_peer_connected_direct" }, true);
  assert.match(said, /transport identity/);
  assert.match(said, /self-asserted/);
  assert.doesNotMatch(said, /member is (online|live)/i);
});

test("indirect member evidence never becomes a presence or delivery claim", () => {
  const reachable = routeIndirectEvidence({
    ...route,
    indirect_health: "reachable_via_member",
    indirect_witnesses: 1,
    indirect_age_ms: 5_000,
  });
  assert.match(reachable ?? "", /1 authenticated member reported a live path/);
  assert.match(reachable ?? "", /not proof the person is online/);

  const reachableWhilePending = routeIndirectEvidence({
    ...route,
    indirect_health: "reachable_via_member",
    indirect_witnesses: 1,
    indirect_age_ms: 5_000,
    reciprocal_pending: true,
  });
  assert.equal(
    reachableWhilePending,
    reachable,
    "present evidence takes precedence without turning queued work into a delivery claim",
  );

  const suspected = routeIndirectEvidence({
    ...route,
    indirect_health: "suspected_unreachable",
    indirect_witnesses: 2,
    indirect_age_ms: 30_000,
  });
  assert.match(suspected ?? "", /2 authenticated members did not observe a live path/);
  assert.match(suspected ?? "", /suspicion, not proof/);

  const queued = routeIndirectEvidence({ ...route, reciprocal_pending: true });
  assert.match(queued ?? "", /queued a bounded reciprocal-dial request/);
  assert.match(queued ?? "", /does not prove the helper or target received it/);
  assert.equal(routeIndirectEvidence(route), null);
});

test("an IPv6-only cooldown is surfaced as a clue, not an invented route verdict", () => {
  const v6only: MemberRoute = {
    ...route,
    addresses: ["/ip6/2601:441:4581:a5c0::1/udp/23123/quic-v1"],
    dial_attempts: 8,
    next_dial_in_ms: 900_000,
    health: "claimed_peer_dial_cooling_down",
    candidate_families: ["ipv6"],
  };
  const said = routeExplanation(v6only, false);
  assert.match(said, /IPv6-only/);
  assert.match(said, /does not measure outbound IPv6/);
  assert.match(said, /does not prove/);
  assert.match(said, /does not prove that every candidate was attempted or failed/);
  assert.match(said, /15m/, "and says when it will bother trying again");

  const withV6 = routeExplanation(v6only, true);
  assert.equal(withV6, said, "an inbound/public observation is not an outbound IPv6 route test");
});

test("a member with no record is told what is missing and how it arrives", () => {
  const said = routeExplanation({ ...route, peer: "", health: "no_peer_record", binding: "absent" }, true);
  assert.match(said, /PEX/);
  assert.match(said, /cannot be dialled/);
});

test("typed paths and recommended actions have safe forward-compatible labels", () => {
  assert.equal(
    routePathLabel({ family: "ipv6", transport: "circuit_relay", direction: "listener" }),
    "IPv6 · circuit relay",
  );
  assert.equal(
    routeActionLabel({ scope: "group", kind: "configure_fallback_node" }),
    "Configure a trusted fallback node",
  );
  assert.equal(
    routeActionLabel({ scope: "device", kind: "probe_through_members" }),
    "Check paths through connected members",
  );
  assert.equal(
    routeActionLabel({ scope: "group", kind: "retry_group_now" }),
    "Retry this group's current routes now",
  );
  assert.equal(routeActionScopeLabel({ scope: "group", kind: "configure_fallback_node" }), "group");
  assert.match(routeActionLabel({ scope: "future", kind: "future_action" }), /Update Mewtual/);
});

test("durations read as a countdown, and a sub-second wait is not a stuck zero", () => {
  assert.equal(formatDuration(0), "now");
  assert.equal(formatDuration(999), "now");
  assert.equal(formatDuration(42_000), "42s");
  assert.equal(formatDuration(200_000), "3m 20s");
  assert.equal(formatDuration(180_000), "3m");
});

test("historical route age advances locally without inventing negative elapsed time", () => {
  const historical = {
    ...route,
    last_success: {
      age_ms: 30_000,
      path: { family: "ipv4", transport: "quic_v1", direction: "dialer" },
    },
  };
  assert.equal(routeHistoricalAge(historical, 1_000, 61_000), 90_000);
  assert.equal(routeHistoricalAge(historical, 61_000, 1_000), 30_000);
  assert.equal(routeHistoricalAge(route, 1_000, 61_000), null);
});

test("member-route refresh decisions are scoped to visible Connectivity", () => {
  assert.equal(memberRoutesVisible(7, "connectivity"), true);
  assert.equal(memberRoutesVisible(null, "connectivity"), false);
  assert.equal(memberRoutesVisible(7, "chat"), false);
  assert.equal(shouldRefreshMemberRoutes(7, "connectivity", 7), true);
  assert.equal(shouldRefreshMemberRoutes(7, "connectivity", 8), false);
  assert.equal(shouldRefreshMemberRoutes(7, "chat", 7), false);
});

test("asynchronous route answers stay bound to server ids and failures retain marked snapshots", () => {
  const older = { 1: [{ ...route, fingerprint: "one" }], 2: [{ ...route, fingerprint: "two" }] };
  const freshTwo = [{ ...route, fingerprint: "two-fresh" }];
  const merged = mergeMemberRoutePoll(
    [2, 1, 3],
    older,
    [
      { id: 1, ok: false, rows: [] },
      { id: 2, ok: true, rows: freshTwo },
      { id: 99, ok: true, rows: [{ ...route, fingerprint: "removed" }] },
    ],
  );
  assert.equal(merged.routes[1][0].fingerprint, "one");
  assert.equal(merged.routes[2][0].fingerprint, "two-fresh");
  assert.deepEqual(merged.routes[3], []);
  assert.deepEqual([...merged.unavailable].sort(), [1, 3]);
  assert.equal(merged.routes[99], undefined);
});

test("main Connectivity retains a failed same-server read and rejects stale-server answers", () => {
  const previous = [{ ...route, fingerprint: "old" }];
  const failed = mergeMemberRouteRead(7, 7, previous, false, null);
  assert.equal(failed.applied, true);
  assert.equal(failed.unavailable, true);
  assert.deepEqual(failed.routes, previous);

  const stale = mergeMemberRouteRead(
    8,
    7,
    previous,
    true,
    [{ ...route, fingerprint: "wrong-server" }],
  );
  assert.equal(stale.applied, false);
  assert.equal(stale.unavailable, true);
  assert.deepEqual(stale.routes, previous);

  const recovered = mergeMemberRouteRead(
    7,
    7,
    previous,
    true,
    [{ ...route, fingerprint: "fresh" }],
  );
  assert.equal(recovered.applied, true);
  assert.equal(recovered.unavailable, false);
  assert.equal(recovered.routes[0].fingerprint, "fresh");
});

test("a copied bundle states its redaction mode and carries the privacy contract", () => {
  // The person who receives a pasted report never read the footer it came from.
  const text = copyBundle(
    { version: "0.3.0", at: Date.UTC(2026, 7, 23), redacted: true, capture: "safe", session: "eb887278" },
    [{ title: "network", lines: ["a", "b"] }],
  );
  assert.match(text, /display masking: on/);
  assert.match(text, /privacy\.addresses: aliased where recognized/);
  assert.match(text, /privacy\.user_content: may be present/);
  assert.match(text, /privacy\.legacy_prose: included/);
  // Safe and Enhanced reports look alike and mean very different things, so a report says which it
  // is rather than leaving the reader to infer it from whether an address looks complete.
  assert.match(text, /current capture: safe/);
  assert.match(text, /session: eb887278/, "so an excerpt can be matched back to its report");
  assert.match(text, /== NETWORK ==/);
  // Describes what a report may contain rather than promising what it does not. The old sentence
  // ended "never includes ... names", and the report writes every server's name into itself: a
  // false safety label is the difference between someone reviewing a report before pasting it
  // into a public issue and someone not bothering.
  assert.match(text, /Read it before you share it/);
  assert.ok(!/never includes/.test(text), "a report must not promise what it cannot enforce");
  assert.match(copyBundle({ version: "0.3.0", at: 0, redacted: false }, []), /display masking: off/);
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

test("a complete Safe bundle cannot leak device or member-route addresses", () => {
  const aliases = makeAliases();
  const masked = exportMasksAddresses("safe", false);
  const text = copyBundle(
    { version: "0.3.0", at: 0, redacted: masked, capture: "safe" },
    [
      { title: "this device", lines: deviceLines(device, aliases, masked) },
      {
        title: "reachability",
        lines: routeLines(
          [{ id: 1, name: "Studio" }],
          { 1: [{ ...route, addresses: ["/ip4/198.51.100.7/udp/31484/quic-v1"] }] },
          aliases,
          masked,
        ),
      },
    ],
  );
  assert.match(text, /display masking: on/);
  assert.doesNotMatch(text, /203\.0\.113\.9/);
  assert.doesNotMatch(text, /198\.51\.100\.7/);
  assert.doesNotMatch(text, /Studio/, "server names are user content and must be aliased too");
});

/** The row lines, as distinct from the findings that now lead each server's block. */
const rowLines = (lines: string[]) => lines.filter((l) => !/\[(WARN|DANGER)\]/.test(l));

test("route lines carry the state, the counters and every candidate address", () => {
  const servers = [{ id: 1, name: "Studio" }];
  const routes = { 1: [{ ...route, health: "claimed_peer_dial_cooling_down", dial_attempts: 3, next_dial_in_ms: 42_000 }] };
  const [line] = rowLines(routeLines(servers, routes, makeAliases(), false));
  assert.match(line, /^Studio /);
  assert.match(line, /DIAL COOLDOWN/);
  assert.match(line, /submits=3/);
  assert.match(line, /next=42s/);
  assert.match(line, /\/ip4\/203\.0\.113\.9\/udp\/31484\/quic-v1/);
});

test("a member with no address says so rather than trailing off", () => {
  const lines = routeLines([{ id: 1, name: "Studio" }], { 1: [{ ...route, health: "claimed_peer_has_no_route", addresses: [] }] }, makeAliases(), false);
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
    {
      1: [{
        ...route,
        health: "claimed_peer_dial_cooling_down",
        addresses: ["/ip6/2001:db8::1/udp/1/quic-v1"],
        candidate_families: ["ipv6"],
        dial_attempts: 2,
      }],
    },
    makeAliases(),
    false,
    false,
  );
  assert.match(lines[0], /\[WARN\] NET\.ROUTE\.IPV6_ONLY/);
  assert.match(lines[0], /^Studio /, "each finding names its server, as the rows do");
  assert.ok(rowLines(lines).length === 1, "and the row still follows");
});

test("a healthy server's report is rows only, with nothing to conclude", () => {
  const lines = routeLines(
    [{ id: 1, name: "Studio" }],
    { 1: [{ ...route, connected: true, health: "claimed_peer_connected_direct" }] },
    makeAliases(),
    false,
  );
  assert.deepEqual(lines, rowLines(lines));
});

test("a server with no members contributes nothing rather than an empty row", () => {
  assert.deepEqual(routeLines([{ id: 1, name: "Studio" }], {}, makeAliases(), false), []);
});

test("an unavailable server copies no retained row as current evidence", () => {
  const lines = routeLines(
    [{ id: 1, name: "Studio" }],
    { 1: [{ ...route, connected: true, health: "claimed_peer_connected_direct" }] },
    makeAliases(),
    false,
    true,
    new Set([1]),
  );
  assert.deepEqual(lines.length, 1);
  assert.match(lines[0], /\[UNAVAILABLE\]/);
  assert.match(lines[0], /last snapshot/);
  assert.doesNotMatch(lines[0], /DIRECT PATH/);
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
  health: "claimed_peer_dial_cooling_down",
  candidate_families: ["ipv6"],
  ...over,
});

test("a healthy server produces no findings at all", () => {
  const connected = [
    { ...route, connected: true, health: "claimed_peer_connected_direct" },
    { ...route, fingerprint: "b", connected: true, health: "claimed_peer_connected_relay" },
  ];
  assert.deepEqual(routeFindings(connected, true), []);
});

test("a server with nobody else in it is not a reachability problem", () => {
  assert.deepEqual(routeFindings([], false), []);
});

/** The hour-long isolation, as one sentence instead of a pile of multiaddrs. */
test("v6-only members are named without claiming the host has no outbound route", () => {
  const found = routeFindings([v6("a"), v6("b")], false);
  const ipv6 = found.find((f) => f.code === "NET.ROUTE.IPV6_ONLY");
  assert.ok(ipv6, JSON.stringify(found));
  assert.equal(ipv6.severity, "warn");
  assert.equal(ipv6.affected, 2);
  assert.match(ipv6.detail, /does not measure outbound IPv6/);
  assert.match(ipv6.detail, /cannot prove/);
  assert.ok(!ipv6.detail.includes("2001:db8"), "a finding never carries an address");
});

test("a public IPv6 observation does not suppress the candidate-shape advisory", () => {
  const found = routeFindings([v6("a"), v6("b")], true);
  assert.equal(found.find((f) => f.code === "NET.ROUTE.IPV6_ONLY")?.affected, 2);
});

test("a member with no record is a different problem from one with no address", () => {
  const found = routeFindings(
    [
      { ...route, fingerprint: "a", peer: "", health: "no_peer_record" },
      { ...route, fingerprint: "b", addresses: [], health: "claimed_peer_has_no_route" },
    ],
    true,
  );
  assert.equal(found.find((f) => f.code === "NET.ROUTE.NO_RECORD")?.affected, 1);
  assert.equal(found.find((f) => f.code === "NET.ROUTE.NO_DIALABLE_ADDRESS")?.affected, 1);
});

test("no live claimed-peer connection is reported without claiming routes are unreachable", () => {
  const found = routeFindings([v6("a"), v6("b")], false);
  const codes = found.map((f) => f.code);
  assert.ok(codes.includes("NET.ROUTE.NO_LIVE_MEMBER_CONNECTION"));
  // A reader scanning from the top meets the explanation before the symptom.
  assert.ok(
    codes.indexOf("NET.ROUTE.IPV6_ONLY") < codes.indexOf("NET.ROUTE.NO_LIVE_MEMBER_CONNECTION"),
    codes.join(","),
  );
  const noLive = found.find((f) => f.code === "NET.ROUTE.NO_LIVE_MEMBER_CONNECTION")!;
  assert.equal(noLive.severity, "warn");
  assert.match(noLive.detail, /does not prove their routes are unreachable/);
});

test("one live claimed-peer connection suppresses the no-live aggregate", () => {
  const found = routeFindings(
    [v6("a"), { ...route, fingerprint: "b", connected: true, health: "claimed_peer_connected_direct" }],
    false,
  );
  assert.equal(
    found.find((f) => f.code === "NET.ROUTE.NO_LIVE_MEMBER_CONNECTION"),
    undefined,
  );
  assert.equal(found.find((f) => f.code === "NET.ROUTE.IPV6_ONLY")?.affected, 1);
});

test("a member advertising a mix of families is not blamed on IPv6", () => {
  // One usable v4 candidate means the v6 ones are not the explanation.
  const mixed = v6("a", {
    addresses: ["/ip6/2001:db8::1/udp/1/quic-v1", "/ip4/203.0.113.9/udp/1/quic-v1"],
    candidate_families: ["ipv4", "ipv6"],
  });
  assert.equal(
    routeFindings([mixed], false).find((f) => f.code === "NET.ROUTE.IPV6_ONLY"),
    undefined,
  );
});

test("a finding reads as a sentence for one member and for several", () => {
  // "1 member(s) advertise" is the sort of thing that makes a reader trust the rest a little less.
  const one = routeFindings([v6("a")], false);
  assert.match(one[0].detail, /^1 member advertises only IPv6/);
  assert.match(
    one.find((f) => f.code === "NET.ROUTE.NO_LIVE_MEMBER_CONNECTION").detail,
    /only other member/,
  );

  const many = routeFindings([v6("a"), v6("b"), v6("c")], false);
  assert.match(many[0].detail, /^3 members advertise only IPv6/);
  assert.match(
    many.find((f) => f.code === "NET.ROUTE.NO_LIVE_MEMBER_CONNECTION").detail,
    /3 other members/,
  );

  const singleNoRecord = routeFindings(
    [{ ...route, peer: "", connected: false, health: "no_peer_record" }],
    true,
  );
  assert.match(singleNoRecord[0].detail, /1 member has no signed peer record/);
});
