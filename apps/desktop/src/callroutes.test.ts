import assert from "node:assert/strict";
import test from "node:test";
import { callBarStatus, mappableIcePort, mappingAddressPolicy, routerMappedCandidate } from "./callroutes.ts";

const host = {
  type: "host",
  protocol: "udp",
  address: "192.168.0.231",
  port: 54321,
  foundation: "1234567890",
  component: "rtp",
  priority: 2122260223,
  sdpMid: "0",
  sdpMLineIndex: 0,
  usernameFragment: "abcd",
};

test("only a host UDP candidate with a port is worth a router mapping", () => {
  assert.ok(mappableIcePort(host));
  assert.ok(!mappableIcePort({ ...host, type: "srflx" }), "srflx already traversed the NAT");
  assert.ok(!mappableIcePort({ ...host, type: "relay" }));
  assert.ok(!mappableIcePort({ ...host, protocol: "tcp" }), "the mapped socket is UDP");
  assert.ok(!mappableIcePort({ ...host, port: null }));
  assert.ok(!mappableIcePort({ ...host, port: 0 }), "port zero names nothing");
});

test("the mapped candidate is a well-formed srflx that shadows its host candidate", () => {
  const out = routerMappedCandidate(host, { ip: "213.105.231.38", port: 54321 });
  assert.equal(
    out.candidate,
    "candidate:1234567890R 1 udp 2122260222 213.105.231.38 54321 typ srflx raddr 0.0.0.0 rport 54321",
  );
  // The SDP coordinates must ride along or the remote can't place the candidate.
  assert.equal(out.sdpMid, "0");
  assert.equal(out.sdpMLineIndex, 0);
  assert.equal(out.usernameFragment, "abcd");
});

test("the mapped candidate never collides with or outranks the host candidate", () => {
  const out = routerMappedCandidate(host, { ip: "203.0.113.9", port: 7 });
  const fields = (out.candidate ?? "").split(" ");
  assert.notEqual(fields[0], `candidate:${host.foundation}`, "same foundation reads as redundant");
  assert.equal(Number(fields[3]), host.priority - 1, "a working direct LAN path must still win");
});

test("mapped candidate defaults survive a browser that reports null fields", () => {
  const bare = {
    ...host,
    foundation: null,
    component: null,
    priority: null,
  };
  const out = routerMappedCandidate(bare, { ip: "203.0.113.9", port: 9 });
  const fields = (out.candidate ?? "").split(" ");
  assert.equal(fields[0], "candidate:rmapR");
  assert.equal(fields[1], "1", "an unknown component is RTP");
  assert.ok(Number(fields[3]) >= 1, "priority stays positive");
});

test("the rtcp component number is 2", () => {
  const out = routerMappedCandidate({ ...host, component: "rtcp" }, { ip: "203.0.113.9", port: 9 });
  assert.equal((out.candidate ?? "").split(" ")[1], "2");
});

test("priority clamps at 1 so a zero-priority host candidate cannot go negative", () => {
  const out = routerMappedCandidate({ ...host, priority: 0 }, { ip: "203.0.113.9", port: 9 });
  assert.equal(Number((out.candidate ?? "").split(" ")[3]), 1);
});

test("a candidate's address decides whether and what to claim to the router", () => {
  // A real IPv4 travels as a claim the native side checks against the default-route interface.
  assert.deepEqual(mappingAddressPolicy("192.168.0.231"), { map: true, claim: "192.168.0.231" });
  // mDNS obfuscation and a missing address prove nothing: map permissively, claim nothing.
  // A liveness probe is NOT the fallback here; the firewall's ICMP is indistinguishable from a
  // dead socket and a probe vetoed every mapping in the field.
  assert.deepEqual(mappingAddressPolicy("d9f2e-4a.local"), { map: true, claim: null });
  assert.deepEqual(mappingAddressPolicy(null), { map: true, claim: null });
  assert.deepEqual(mappingAddressPolicy(undefined), { map: true, claim: null });
  assert.deepEqual(mappingAddressPolicy("  "), { map: true, claim: null });
  // An IPv6 or hostname socket is not what an IPv4 mapping reaches: skip entirely.
  assert.deepEqual(mappingAddressPolicy("fe80::1"), { map: false, claim: null });
  assert.deepEqual(mappingAddressPolicy("2001:db8::5"), { map: false, claim: null });
  assert.deepEqual(mappingAddressPolicy("example.net"), { map: false, claim: null });
});

test("the call bar says which half of the path is missing", () => {
  assert.equal(callBarStatus(0, 0, false, 0), "waiting for others…");
  assert.equal(callBarStatus(2, 2, false, 0), "2 connected");
  assert.equal(callBarStatus(2, 1, false, 0), "1/2 · connecting…");
  // A failure with a mapped route on offer blames the far side; without one, this side.
  assert.equal(
    callBarStatus(2, 1, true, 1),
    "1/2 connected · direct route offered; their side may need TURN",
  );
  assert.equal(
    callBarStatus(2, 1, true, 0),
    "1/2 connected · no direct route; set a TURN server or allow router mapping",
  );
});
