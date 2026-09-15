import assert from "node:assert/strict";
import test from "node:test";
import { joinAttemptView, type JoinStep } from "./joinroutes.ts";

const step = (kind: string, status: string, detail = "", target = ""): JoinStep => ({
  at: 1,
  kind,
  target,
  detail,
  status,
});

const base = { pending: true, error: "", fallbackOffered: false, fallbackAllowed: false };

test("nothing tried yet is idle with no rows", () => {
  const v = joinAttemptView({ ...base, steps: [] });
  assert.equal(v.phase, "idle");
  assert.deepEqual(v.rows, []);
  assert.equal(v.headline, "");
});

test("a verified invite with addresses being dialled is a dialling phase", () => {
  const v = joinAttemptView({
    ...base,
    steps: [
      step("invite", "ok", "signature verified; 2 bootstrap address(es), 0 rendezvous entr(ies)"),
      step("dial", "unknown", "dialled", "/ip4/192.168.1.5/udp/7220/quic-v1/p2p/12D3Koo"),
      step("dial", "unknown", "dialled", "/ip4/86.14.202.9/udp/7220/quic-v1/p2p/12D3Koo"),
    ],
  });
  assert.equal(v.phase, "dialling");
  const [invite, direct] = v.rows;
  assert.equal(invite.state, "ok");
  assert.equal(direct.state, "active");
  assert.equal(direct.note, "dialling 2 addresses");
  // Routes are named by kind: no address leaks into what the surface draws.
  for (const r of v.rows) assert.doesNotMatch(r.note + r.label, /192\.168|86\.14|12D3Koo/);
});

test("a relay circuit in the invite gets its own row", () => {
  const v = joinAttemptView({
    ...base,
    steps: [
      step("invite", "ok"),
      step("dial", "unknown", "dialled", "/ip4/1.2.3.4/udp/7220/quic-v1/p2p/A"),
      step("dial", "unknown", "dialled", "/dns4/relay/udp/7220/quic-v1/p2p/R/p2p-circuit/p2p/A"),
    ],
  });
  assert.deepEqual(
    v.rows.map((r) => [r.kind, r.state]),
    [["invite", "ok"], ["direct", "active"], ["relay", "active"]],
  );
  assert.equal(v.rows[1].note, "dialling 1 address");
});

test("no answer, no fallback: the direct row fails and the headline says nobody answered", () => {
  const v = joinAttemptView({
    ...base,
    pending: false,
    error: "timed out connecting to the server; no direct reply route is available",
    steps: [
      step("invite", "ok"),
      step("dial", "unknown", "dialled", "/ip4/1.2.3.4/udp/7220/quic-v1/p2p/A"),
      step("connect", "failed", "none of the dialled addresses answered within 20s"),
      step("reply", "failed", "this joiner has no public listener route to put in a two-way reply"),
    ],
  });
  assert.equal(v.phase, "failed");
  assert.equal(v.rows.find((r) => r.kind === "direct")?.state, "failed");
  assert.equal(v.rows.find((r) => r.kind === "reply")?.state, "failed");
  assert.equal(v.failedOn, "reply");
  assert.match(v.headline, /reply window/);
});

test("a fallback the joiner did not allow is shown as skipped, never as failed", () => {
  const v = joinAttemptView({
    ...base,
    fallbackOffered: true,
    fallbackAllowed: false,
    steps: [
      step("invite", "ok"),
      step("switchboard", "unknown", "standing member fallbacks were present but the joiner did not consent to contact them"),
      step("dial", "unknown", "dialled", "/ip4/1.2.3.4/udp/7220/quic-v1/p2p/A"),
    ],
  });
  const sb = v.rows.find((r) => r.kind === "switchboard");
  assert.equal(sb?.state, "skipped");
  assert.equal(v.phase, "dialling");
});

test("an allowed fallback waits, then becomes active once the direct route fails", () => {
  const steps = [
    step("invite", "ok"),
    step("switchboard", "ok", "the inviter endorsed 2 currently dialable standing member fallback(s); unexpired routes may be tried only after the direct route"),
    step("dial", "unknown", "dialled", "/ip4/1.2.3.4/udp/7220/quic-v1/p2p/A"),
  ];
  const waiting = joinAttemptView({ ...base, fallbackOffered: true, fallbackAllowed: true, steps });
  assert.equal(waiting.rows.find((r) => r.kind === "switchboard")?.state, "idle");

  const trying = joinAttemptView({
    ...base,
    fallbackOffered: true,
    fallbackAllowed: true,
    steps: [...steps, step("connect", "failed", "none of the dialled addresses answered within 20s")],
  });
  assert.equal(trying.rows.find((r) => r.kind === "direct")?.state, "failed");
  assert.equal(trying.rows.find((r) => r.kind === "switchboard")?.state, "active");
  assert.equal(trying.phase, "dialling");

  const helped = joinAttemptView({
    ...base,
    fallbackOffered: true,
    fallbackAllowed: true,
    steps: [
      ...steps,
      step("connect", "failed", "none of the dialled addresses answered within 20s"),
      step("switchboard", "ok", "connected to an inviter-endorsed standing member fallback"),
      step("connect", "ok", "connected to an inviter-endorsed standing switchboard"),
    ],
  });
  assert.equal(helped.rows.find((r) => r.kind === "switchboard")?.state, "ok");
  assert.equal(helped.rows.find((r) => r.kind === "admission")?.state, "active");
});

test("the reply path: waiting, then the inviter dials back, then admission", () => {
  const steps = [
    step("invite", "ok"),
    step("dial", "unknown", "dialled", "/ip4/1.2.3.4/udp/7220/quic-v1/p2p/A"),
    step("connect", "failed", "none of the dialled addresses answered within 20s"),
    step("reply", "unknown", "generated a 60-second two-way reply with 2 direct candidate(s); waiting for the inviter to dial back"),
  ];
  const waiting = joinAttemptView({ ...base, steps });
  assert.equal(waiting.phase, "dialling");
  assert.equal(waiting.rows.find((r) => r.kind === "reply")?.state, "active");

  const done = joinAttemptView({
    ...base,
    pending: false,
    steps: [
      ...steps,
      step("connect", "ok", "connected to the named inviter"),
      step("join", "ok", "admitted to the group"),
    ],
  });
  assert.equal(done.phase, "connected");
  assert.equal(done.rows.find((r) => r.kind === "reply")?.state, "ok");
  assert.equal(done.rows.find((r) => r.kind === "admission")?.state, "ok");
  assert.equal(done.headline, "You are in.");
});

test("a refused admission is its own failure, not a routing one", () => {
  const v = joinAttemptView({
    ...base,
    pending: false,
    error: "the server refused the join",
    steps: [
      step("invite", "ok"),
      step("dial", "unknown", "dialled", "/ip4/1.2.3.4/udp/7220/quic-v1/p2p/A"),
      step("connect", "ok", "connected to the named inviter"),
      step("join", "failed", "refused; the server refused the join, and only the serving node knows why"),
    ],
  });
  assert.equal(v.phase, "failed");
  assert.equal(v.failedOn, "admission");
  assert.equal(v.rows.find((r) => r.kind === "direct")?.state, "ok");
  assert.match(v.headline, /did not admit/);
});

test("a bad invite fails before anything is dialled", () => {
  const v = joinAttemptView({
    ...base,
    pending: false,
    error: "invite signature invalid",
    steps: [step("invite", "failed", "invite signature invalid")],
  });
  assert.equal(v.phase, "failed");
  assert.equal(v.failedOn, "invite");
  assert.equal(v.rows[0].state, "failed");
  assert.equal(v.rows[1].state, "idle");
  assert.match(v.headline, /could not be read/);
});

test("a command that fails with no steps at all still produces a failed view", () => {
  const v = joinAttemptView({ ...base, pending: false, error: "vault is locked", steps: [] });
  assert.equal(v.phase, "failed");
  assert.equal(v.rows[0].kind, "invite");
  assert.equal(v.rows[0].state, "failed");
});
