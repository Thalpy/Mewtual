import assert from "node:assert/strict";
import test from "node:test";
import {
  DEFAULT_LEVELS,
  appendEvents,
  alias,
  copyBundle,
  dropNote,
  eventLine,
  eventParts,
  eventText,
  filterEvents,
  formatDuration,
  isFrontend,
  latestSeq,
  makeAliases,
  maybeRedact,
  redactText,
  routeChip,
  routeExplanation,
  routeState,
  shownCount,
  type LogEvent,
  type MemberRoute,
} from "./debug-console.ts";

function ev(over: Partial<LogEvent> = {}): LogEvent {
  return {
    seq: 1,
    at_ms: Date.UTC(2026, 7, 23, 12, 0, 0),
    level: "INFO",
    target: "catcoms_net",
    message: "hello",
    fields: [],
    ...over,
  };
}

const route: MemberRoute = {
  fingerprint: "741af9ff",
  peer: "2b5df389",
  addresses: ["/ip4/203.0.113.9/udp/31484/quic-v1"],
  seq: 4,
  connected: false,
  dial_attempts: 0,
  next_dial_in_ms: 0,
};

test("the webview's own events are the frontend section, everything else is the backend", () => {
  // One ring split on one field. If the two sections had separate sources their counts could
  // drift, and a roll-up that disagrees with the feed under it is worse than no roll-up.
  assert.ok(isFrontend(ev({ target: "catcoms_ui" })));
  assert.ok(!isFrontend(ev({ target: "catcoms_net" })));
  assert.ok(!isFrontend(ev({ target: "catcoms_sync" })));
});

test("debug is captured but off by default, so the net firehose is one click away and not on", () => {
  assert.deepEqual(DEFAULT_LEVELS, ["ERROR", "WARN", "INFO"]);
});

test("a line carries its time, level, target and structured fields", () => {
  const line = eventLine(
    ev({ level: "WARN", message: "dial failed", fields: [["error", "network unreachable"]] }),
    makeAliases(),
    false,
  );
  assert.match(line, /WARN /);
  assert.match(line, /catcoms_net/);
  assert.match(line, /dial failed error=network unreachable/);
});

test("a frontend line drops the target, which would read catcoms_ui on every row", () => {
  const line = eventLine(ev({ target: "catcoms_ui", message: "voice signal failed" }), makeAliases(), false);
  assert.ok(!line.includes("catcoms_ui"), line);
  assert.match(line, /voice signal failed/);
});

test("the rendered parts are the source of truth and the joined line agrees with them", () => {
  // Rendering used to slice the joined line back apart by counting characters, and the drift
  // showed up as the level printed twice in the attention list.
  const e = ev({ level: "WARN", target: "catcoms_net", message: "dial failed", fields: [["addr", "/ip6/2601::1/udp/1"]] });
  const a = makeAliases();
  const p = eventParts(e, a, false);
  assert.equal(p.level, "WARN");
  assert.equal(p.target, "catcoms_net");
  assert.equal(p.text, "dial failed addr=/ip6/2601::1/udp/1");
  const line = eventLine(e, a, false);
  assert.ok(line.includes(p.ts) && line.includes(p.target) && line.includes(p.text), line);
  assert.equal(line.match(/WARN/g)?.length, 1, "the level appears exactly once");
});

test("parts redact their text and drop the target for a frontend event", () => {
  const a = makeAliases();
  const p = eventParts(ev({ target: "catcoms_ui", message: "dial 203.0.113.9" }), a, true);
  assert.equal(p.target, "");
  assert.equal(p.text, "dial [ip 1]");
});

test("fields alone still render when an event has no message", () => {
  assert.equal(eventText(ev({ message: "", fields: [["peer", "abc"]] })), "peer=abc");
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
    ev({ seq: 1, level: "INFO", message: "one" }),
    ev({ seq: 2, level: "DEBUG", message: "two" }),
    ev({ seq: 3, level: "WARN", target: "catcoms_sync", message: "three" }),
  ];
  assert.equal(filterEvents(events, { levels: ["INFO", "WARN"] }, a, false).length, 2);
  assert.equal(filterEvents(events, { levels: ["INFO", "WARN"], target: "sync" }, a, false).length, 1);
  assert.equal(filterEvents(events, { levels: ["INFO", "WARN"], text: "THREE" }, a, false).length, 1);
});

test("a filter can search the masked text while redaction is on", () => {
  const a = makeAliases();
  const events = [ev({ seq: 1, message: "dial 203.0.113.9" })];
  assert.equal(filterEvents(events, { levels: ["INFO"], text: "[ip 1]" }, a, true).length, 1);
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
    { version: "0.3.0", at: Date.UTC(2026, 7, 23), redacted: true },
    [{ title: "network", lines: ["a", "b"] }],
  );
  assert.match(text, /redaction: on/);
  assert.match(text, /== NETWORK ==/);
  assert.match(text, /never includes message text/);
  assert.match(copyBundle({ version: "0.3.0", at: 0, redacted: false }, []), /redaction: off/);
});

test("an empty section is labelled rather than silently missing from a report", () => {
  const text = copyBundle({ version: "0.3.0", at: 0, redacted: false }, [{ title: "voice", lines: [] }]);
  assert.match(text, /\(nothing captured\)/);
});
