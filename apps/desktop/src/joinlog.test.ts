// Unit tests for the join log + connectivity formatting.
//
// Run with `npm test`. What matters here: every outcome the backend can emit has copy (a missing
// one would show a blank row exactly where a user most needs a sentence), the pasted text is
// deterministic and carries the invite nonce (the field an operator matches against the invite
// they sent), and the reachability summary never claims more than the code can support.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  OUTCOME_COPY,
  connectivityReadout,
  connectivityStatus,
  describeOutcome,
  formatConnectivity,
  formatJoinLog,
  reachabilitySummary,
  reachabilityEventAffectsReport,
  withOrderedConnectivity,
  withOrderedRefreshedInvite,
  withRefreshedInvite,
  type Connectivity,
  type JoinAttempt,
} from "./joinlog.ts";

// The ids `JoinOutcome::as_str` emits, mirrored here so a rename on either side fails a test
// rather than silently blanking a row.
const BACKEND_OUTCOMES = [
  "admitted",
  "relayed",
  "staged",
  "undecodable",
  "wrong-group",
  "not-this-inviter",
  "bad-signature",
  "expired",
  "revoked",
  "already-used",
  "not-authorized",
  "admission-failed",
];

test("every backend outcome has copy, and no two share a label", () => {
  for (const id of BACKEND_OUTCOMES) {
    assert.ok(OUTCOME_COPY[id], `no copy for outcome "${id}"`);
    assert.ok(OUTCOME_COPY[id].note.length > 20, `"${id}" needs a next action, not a word`);
  }
  assert.equal(Object.keys(OUTCOME_COPY).length, BACKEND_OUTCOMES.length);
  const labels = new Set(Object.values(OUTCOME_COPY).map((c) => c.label));
  assert.equal(labels.size, BACKEND_OUTCOMES.length, "two outcomes must never read the same");
});

test("the three causes a user acts on differently never collapse together", () => {
  // The reported field failure: "expired", "revoked" and "already used" all arrived as one
  // silent rejection, and they have three different fixes.
  const expired = describeOutcome("expired");
  const used = describeOutcome("already-used");
  const revoked = describeOutcome("revoked");
  assert.notEqual(expired.note, used.note);
  assert.notEqual(expired.note, revoked.note);
  assert.notEqual(used.note, revoked.note);
});

test("an unknown outcome id degrades to the id rather than an empty row", () => {
  const c = describeOutcome("some-future-outcome");
  assert.equal(c.label, "some-future-outcome");
  assert.ok(c.note.length > 0);
  assert.equal(describeOutcome("").label, "unknown");
});

test("the pasted join log is deterministic and carries the invite nonce", () => {
  const attempts: JoinAttempt[] = [
    {
      at: Date.UTC(2026, 7, 19, 21, 4, 5),
      outcome: "already-used",
      admitted: false,
      peer: "aabbccdd00112233",
      nonce: "0011223344556677",
    },
    {
      at: Date.UTC(2026, 7, 19, 20, 59, 0),
      outcome: "admitted",
      admitted: true,
      peer: "ffeeddcc00112233",
      nonce: "8899aabbccddeeff",
    },
  ];
  const text = formatJoinLog(attempts);
  assert.equal(text, formatJoinLog(attempts), "same input, same bytes");
  assert.match(text, /2 attempts, newest first/);
  // UTC, because the person being asked for help is usually in another timezone.
  assert.match(text, /2026-08-19 21:04:05Z {2}already used {8}invite 0011223344556677/);
  assert.match(text, /invite 8899aabbccddeeff/);
  assert.match(text, /peer aabbccdd00112233/);
});

test("an empty join log says so rather than pasting a bare header", () => {
  assert.match(formatJoinLog([]), /no inbound join attempts/);
});

test("a join log entry with no nonce says the nonce is unknown, never a fake one", () => {
  const text = formatJoinLog([
    { at: 1, outcome: "undecodable", admitted: false, peer: "", nonce: "" },
  ]);
  assert.match(text, /invite unknown/);
  assert.match(text, /peer unknown/);
});

const emptyReport: Connectivity = {
  action: "join",
  subject: "aabbccdd",
  at: Date.UTC(2026, 7, 19, 21, 0, 0),
  server: 0,
  advertised: [],
  public_direct: false,
  upnp: "no mapping obtained within 25s (UPnP unavailable; PCP unavailable; NAT-PMP unavailable)",
  autonat: "not tested: no public address candidate and AutoNAT server were available together",
  steps: [],
  last_error: "timed out connecting to the server",
};

test("reachability distinguishes a real AutoNAT callback from weaker evidence", () => {
  const none = reachabilitySummary(emptyReport);
  assert.equal(none.verdict, "unknown");
  assert.match(none.detail, /AutoNAT/);
  assert.equal(reachabilitySummary(null).verdict, "unknown");

  const mapped = reachabilitySummary({
    ...emptyReport,
    upnp: "mapped via PCP TCP: /ip4/203.0.113.7/tcp/9000",
  });
  assert.equal(mapped.verdict, "mapping obtained (not verified)");
  assert.match(mapped.detail, /PCP TCP/);
  assert.match(mapped.detail, /candidate route/);

  const tested = reachabilitySummary({
    ...emptyReport,
    autonat: "reachable /ip4/45.79.12.34/tcp/9000 (verified by AutoNAT server 12D3KooWTest)",
  });
  assert.equal(tested.verdict, "direct callback succeeded");
  assert.match(tested.detail, /fresh callback/);

  const failed = reachabilitySummary({
    ...emptyReport,
    autonat: "unreachable /ip6/2001:4860::1/tcp/9000 from AutoNAT server 12D3KooWTest: dial failed",
  });
  assert.equal(failed.verdict, "direct test failed");
  assert.match(failed.detail, /relay/);

  const relay = reachabilitySummary({
    ...emptyReport,
    advertised: ["/ip4/198.51.100.4/tcp/4000/p2p/RELAY/p2p-circuit/p2p/ME"],
  });
  assert.equal(relay.verdict, "reachable through a relay");

  const directAndRelay = reachabilitySummary({
    ...emptyReport,
    advertised: ["/ip4/198.51.100.4/tcp/4000/p2p/RELAY/p2p-circuit/p2p/ME"],
    autonat: "reachable /ip4/45.79.12.34/tcp/9000 (verified by AutoNAT server 12D3KooWTest)",
  });
  assert.equal(directAndRelay.verdict, "direct callback succeeded");
});

test("the shared connectivity helper exposes honest status-line states and a real readout", () => {
  assert.equal(connectivityStatus(null).tone, "pending");
  assert.equal(
    connectivityStatus({ ...emptyReport, upnp: "waiting for router mapping (UPnP, PCP, NAT-PMP)" }).key,
    "CHECKING…",
  );
  assert.equal(
    connectivityStatus({
      ...emptyReport,
      advertised: ["/ip4/192.168.1.5/tcp/22487/p2p/ME"],
    }).key,
    "THIS NETWORK ONLY",
  );
  assert.equal(
    connectivityStatus({
      ...emptyReport,
      autonat: "reachable /ip4/45.79.12.34/tcp/22487 (verified by AutoNAT server PEER)",
    }).key,
    "DIRECT CALLBACK OK",
  );

  const readout = connectivityReadout({
    ...emptyReport,
    advertised: [
      "/ip4/203.0.113.7/tcp/22487/p2p/ME",
      "/ip6/2001:db8::7/udp/22487/quic-v1/p2p/ME",
    ],
    upnp: "mapped via PCP TCP: /ip4/203.0.113.7/tcp/22487",
  });
  assert.match(readout, /PORT 22487/);
  assert.match(readout, /MAPPING mapped via PCP TCP/);
  assert.match(readout, /IPV6 1 · QUIC offered · RELAY none/);

  const relayOnly = connectivityReadout({
    ...emptyReport,
    advertised: [
      "/ip6/2001:4860::1/udp/4001/quic-v1/p2p/RELAY/p2p-circuit/p2p/ME",
    ],
  });
  assert.match(relayOnly, /PORT unknown/);
  assert.match(relayOnly, /IPV6 0 · QUIC none · RELAY ready/);
});

test("an async invite refresh updates only the server named by its event", () => {
  const before = [
    { id: 1, invite: "stale-route", name: "one" },
    { id: 2, invite: "active-server", name: "two" },
  ];
  const after = withRefreshedInvite(before, 1, "fresh-route");
  assert.deepEqual(after, [
    { id: 1, invite: "fresh-route", name: "one" },
    { id: 2, invite: "active-server", name: "two" },
  ]);
  assert.equal(after[1], before[1], "an unrelated active server retains object identity");
  assert.deepEqual(
    withRefreshedInvite(before, 99, "late-result"),
    before,
    "a late refresh for a server already left cannot update another server",
  );
  assert.equal(withRefreshedInvite(before, 1, null)[0]?.invite, "");

  const newer = withOrderedRefreshedInvite(before, 1, "new-route", 2, 2);
  const afterLateOlder = withOrderedRefreshedInvite(newer, 1, "old-route", 1, 2);
  assert.equal(afterLateOlder, newer, "an older same-server completion is ignored by identity");
  assert.equal(afterLateOlder[0]?.invite, "new-route");

  assert.equal(
    reachabilityEventAffectsReport({ server: 1 }, 1),
    true,
    "server A's global report refreshes even while some unrelated server B is active",
  );
  assert.equal(reachabilityEventAffectsReport({ server: 1 }, 2), false);
});

test("an older connectivity refresh cannot restore withdrawn reachability", () => {
  const newer = {
    ...emptyReport,
    server: 1,
    advertised: [],
    upnp: "no active router mapping: previous mapping expired; retrying",
    autonat: "not tested: no current public address candidate",
  };
  const older = {
    ...newer,
    advertised: ["/ip4/203.0.113.8/tcp/22487/p2p/ME"],
    upnp: "mapped via PCP",
    autonat: "reachable /ip4/203.0.113.8/tcp/22487",
  };

  assert.equal(
    withOrderedConnectivity(newer, older, 1, 2),
    newer,
    "a late pre-expiry response is ignored by identity",
  );
  assert.equal(withOrderedConnectivity(newer, older, 2, 2), older);
  assert.equal(withOrderedConnectivity(newer, null, 1, 2), newer);
});

test("the connectivity report keeps the last error verbatim", () => {
  const text = formatConnectivity({
    ...emptyReport,
    advertised: ["/ip4/192.168.1.5/tcp/9000/p2p/ME"],
    steps: [
      { at: 1, kind: "dial", target: "/ip4/10.0.0.1/tcp/9000", detail: "dialled", status: "unknown" },
      { at: 2, kind: "connect", target: "", detail: "none of the dialled addresses answered within 20s", status: "failed" },
    ],
  });
  assert.match(text, /Last error \(verbatim\): timed out connecting to the server/);
  assert.match(text, /Automatic port mapping:/);
  assert.match(text, /\[unknown\] dial \/ip4\/10\.0\.0\.1\/tcp\/9000: dialled/);
  assert.match(text, /\[failed\] connect: none of the dialled addresses answered within 20s/);
  assert.match(text, /Addresses this node advertises \(1\):/);
  assert.match(text, /Observed reachability: unknown/);
  assert.match(text, /AutoNAT: not tested:/);
});

test("a connectivity report with nothing attempted says so", () => {
  assert.match(formatConnectivity(null), /nothing has been founded or joined/);
  assert.match(
    formatConnectivity({ ...emptyReport, action: "" }),
    /nothing has been founded or joined/,
  );
});
