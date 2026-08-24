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
      out.push({ seq, at_ms: 0, level: "INFO", target: "catcoms_sync", message: `e${seq}`, fields: [] });
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
    return [{ seq: 1, at_ms: 0, level: "INFO", target: "t", message: "same", fields: [] }];
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
