import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  CLAIM_FRAME_BUCKET_BURST,
  CLAIM_MAX_LIVE_PER_SENDER,
  CLAIM_RESEND_MS,
  CLAIM_TTL_MS,
  DRAW_FRAME_LIMITS,
  DRAW_HOLD_MAX,
  DRAW_TOLD_MAX,
  DRAW_MAX_SESSIONS_PER_CHANNEL,
  DrawClaimsSession,
  decodeDrawFrame,
  encodeClaimFrame,
  type ClaimNotice,
  type DrawChannel,
} from "./draw-claims.ts";

// Deterministic seams: a hand-advanced monotonic clock and a counter-based "random" source.
class FakeClock {
  t = 1_000;
  readonly now = () => this.t;
  advance(ms: number) { this.t += ms; }
}
function counterHex(seed: number): (bytes: number) => string {
  let n = seed * 1_000;
  return (bytes) => {
    n += 1;
    return n.toString(16).padStart(bytes * 2, "0").slice(-bytes * 2);
  };
}

const A = "aa".repeat(32);
const B = "bb".repeat(32);
const C = "cc".repeat(32);
const VICTIM = "1a2b3c4d" + "0".repeat(56);
const TWIN = "1a2b3c4d" + "f".repeat(56);
const OBJ = "0123456789abcdef0123456789abcdef";
const FR1 = "11".repeat(16);
const FR2 = "22".repeat(16);
const FR3 = "33".repeat(16);
const FR4 = "44".repeat(16);
const FR5 = "55".repeat(16);

type Member = { session: DrawClaimsSession; notices: ClaimNotice[] };

function member(self: string, clock: FakeClock, seed: number, sequenceMax?: number): Member {
  const session = new DrawClaimsSession({ self, now: clock.now, randomHex: counterHex(seed), sequenceMax });
  const notices: ClaimNotice[] = [];
  session.onnotice = (n) => notices.push(n);
  return { session, notices };
}

/**
 * Connect two members with one in-memory "draw channel" each way. Returns the receiver-side
 * capabilities and a log of every raw frame each side sent, for replay tests.
 */
function connect(x: Member, xId: string, y: Member, yId: string) {
  const sent = { xy: [] as string[], yx: [] as string[] };
  let atY: DrawChannel | null = null; // y's capability for frames from x
  let atX: DrawChannel | null = null; // x's capability for frames from y
  atX = x.session.openEdge(yId, (raw) => { sent.xy.push(raw); if (atY) y.session.receive(atY, raw); return true; });
  atY = y.session.openEdge(xId, (raw) => { sent.yx.push(raw); if (atX) x.session.receive(atX, raw); return true; });
  assert.ok(atX && atY);
  x.session.edgeReady(atX);
  y.session.edgeReady(atY);
  return { atX: atX!, atY: atY!, sent };
}

function frame(sn: string, q: number, fr: string, on: 0 | 1, ask: 0 | 1, obj = OBJ): string {
  return encodeClaimFrame({ sn, q, obj, fr, on, ask });
}
const SN1 = "0123456789abcdef";
const SN2 = "fedcba9876543210";

test("the claim frame is the exact seven-key shape and nothing else", () => {
  const raw = frame(SN1, 7, FR1, 1, 0);
  assert.equal(raw, `{"t":"c","sn":"${SN1}","q":7,"obj":"${OBJ}","fr":"${FR1}","on":1,"ask":0}`);
  assert.ok(new TextEncoder().encode(raw).byteLength < 200);
  const ok = decodeDrawFrame(raw);
  assert.equal(ok.ok && ok.action, "claim");
  assert.equal((decodeDrawFrame(frame(SN1, 7, FR1, 0, 1)) as { action?: string }).action, "ask");
  assert.equal((decodeDrawFrame(frame(SN1, 7, FR1, 0, 0)) as { action?: string }).action, "release");
  // Key order on the wire does not matter; the key SET does.
  const shuffled = JSON.stringify({ ask: 0, on: 1, fr: FR1, obj: OBJ, q: 7, sn: SN1, t: "c" });
  assert.equal(decodeDrawFrame(shuffled).ok, true);

  const bad: [string, string][] = [
    ["claim and ask at once", frame(SN1, 1, FR1, 1, 1)],
    ["missing key", JSON.stringify({ t: "c", sn: SN1, q: 1, obj: OBJ, fr: FR1, on: 1 })],
    ["extra key", JSON.stringify({ t: "c", sn: SN1, q: 1, obj: OBJ, fr: FR1, on: 1, ask: 0, by: A })],
    ["proto key", `{"t":"c","sn":"${SN1}","q":1,"obj":"${OBJ}","fr":"${FR1}","on":1,"ask":0,"__proto__":{}}`],
    ["uppercase hex", frame(SN1, 1, FR1, 1, 0, OBJ.toUpperCase())],
    ["uppercase session", frame(SN1.toUpperCase(), 1, FR1, 1, 0)],
    ["short frame id", frame(SN1, 1, FR1.slice(2), 1, 0)],
    ["all-zero session", frame("0".repeat(16), 1, FR1, 1, 0)],
    ["short session", frame(SN1.slice(1), 1, FR1, 1, 0)],
    ["reserved sequence", frame(SN1, 0xffff_ffff, FR1, 1, 0)],
    ["negative sequence", frame(SN1, -1, FR1, 1, 0)],
    ["fractional sequence", frame(SN1, 1.5, FR1, 1, 0)],
    ["string flag", JSON.stringify({ t: "c", sn: SN1, q: 1, obj: OBJ, fr: FR1, on: "1", ask: 0 })],
    ["boolean flag", JSON.stringify({ t: "c", sn: SN1, q: 1, obj: OBJ, fr: FR1, on: true, ask: 0 })],
    ["array", "[1,2,3]"],
    ["not json", "{"],
    ["stroke (C4a, unsupported)", JSON.stringify({ t: "k", sn: SN1, q: 1 })],
    ["oversize", frame(SN1, 1, FR1, 1, 0) + " ".repeat(120)],
  ];
  for (const [why, raw] of bad) assert.equal(decodeDrawFrame(raw).ok, false, why);
});

test("two members observe a claim with a receiver-observed 90 s countdown and 30 s refresh", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  assert.equal(a.session.claim(OBJ, FR1), true);
  assert.deepEqual(b.session.peerEntries(OBJ), [{ peer: A, obj: OBJ, fr: FR1, kind: "claim", secondsLeft: 90 }]);
  assert.deepEqual(a.session.local().claim, { obj: OBJ, fr: FR1 });
  assert.deepEqual(a.session.peerEntries(), [], "my own claim is local state, not a peer entry");

  clock.advance(CLAIM_RESEND_MS - 1);
  a.session.tick();
  assert.equal(b.session.peerEntries(OBJ)[0]?.secondsLeft, 61, "no refresh before 30 s");
  clock.advance(1);
  a.session.tick();
  assert.equal(b.session.peerEntries(OBJ)[0]?.secondsLeft, 90, "the 30 s refresh restarts the receiver's countdown");

  // Kept alive by refreshes for far longer than one TTL.
  for (let i = 0; i < 300; i += 1) { clock.advance(1_000); a.session.tick(); b.session.tick(); }
  assert.equal(b.session.peerEntries(OBJ).length, 1);
});

test("expiry is exactly 90 s after the last accepted receipt, on the receiver's clock", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  a.session.claim(OBJ, FR1);
  // The sender stops refreshing (crashed, or a modified client): the receiver's clock decides.
  clock.advance(CLAIM_TTL_MS - 1);
  b.session.tick();
  assert.equal(b.session.peerEntries(OBJ)[0]?.secondsLeft, 1);
  const rev = b.session.rev;
  clock.advance(1);
  b.session.tick();
  assert.deepEqual(b.session.peerEntries(OBJ), [], "gone at exactly 90 s");
  assert.ok(b.session.rev > rev, "expiry is a visible change");
});

test("Ask shows the holder a countdown, and Pass hands the frame to the asker", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  a.session.claim(OBJ, FR1);
  assert.equal(b.session.ask(OBJ, FR1), true);
  assert.deepEqual(a.session.asksOnMine(), [{ peer: B, obj: OBJ, fr: FR1, kind: "ask", secondsLeft: 90 }]);
  assert.equal(b.session.local().ask?.secondsLeft, 90);
  clock.advance(10_000);
  a.session.tick();
  b.session.tick();
  assert.equal(a.session.asksOnMine()[0]?.secondsLeft, 80, "the sticky notice counts down");

  assert.equal(a.session.pass(OBJ, FR1), true);
  assert.equal(a.session.local().claim, null);
  assert.deepEqual(b.session.local().claim, { obj: OBJ, fr: FR1 }, "the asker took the passed frame");
  assert.equal(b.session.local().ask, null);
  assert.deepEqual(b.notices, [{ kind: "passed", from: A, obj: OBJ, fr: FR1 }]);
  assert.deepEqual(a.session.peerEntries(OBJ).map((e) => [e.peer, e.kind]), [[B, "claim"]]);
  // Having passed, painting does not grab the frame straight back.
  assert.equal(a.session.autoClaim(OBJ, FR1), false);
  // An explicit claim is still allowed: claims are advisory, and this is now a visible collision.
  assert.equal(a.session.claim(OBJ, FR1), true);
  assert.deepEqual(b.session.peerEntries(OBJ).map((e) => [e.peer, e.kind]), [[A, "claim"]]);
  assert.deepEqual(b.session.local().claim, { obj: OBJ, fr: FR1 });
});

test("an Ask is one-shot: it is not refreshed and lapses after 90 s at both ends", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  a.session.claim(OBJ, FR1);
  b.session.ask(OBJ, FR1);
  for (let i = 0; i < 89; i += 1) { clock.advance(1_000); a.session.tick(); b.session.tick(); }
  assert.equal(a.session.asksOnMine()[0]?.secondsLeft, 1);
  clock.advance(1_000);
  a.session.tick();
  b.session.tick();
  assert.deepEqual(a.session.asksOnMine(), []);
  assert.equal(b.session.local().ask, null);
  // The holder's claim, refreshed throughout, is unaffected.
  assert.equal(b.session.peerEntries(OBJ)[0]?.kind, "claim");
});

test("only an explicit release is a Pass: expiry, disconnect and reconnect never auto-claim", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const link = connect(a, A, b, B);
  a.session.claim(OBJ, FR1);
  b.session.ask(OBJ, FR1);

  // The holder's link drops and recovers: the receiver hides its claim at once...
  b.session.peerUnreachable(A);
  assert.deepEqual(b.session.peerEntries(OBJ), []);
  assert.equal(b.session.local().claim, null, "a dead link is not a Pass");
  // ...and its own recovery shows the claim again, still counting down from the last receipt.
  b.session.edgeReady(link.atY);
  assert.equal(b.session.peerEntries(OBJ)[0]?.kind, "claim");

  // Expiry is not a Pass either (the holder silently stopped refreshing).
  clock.advance(CLAIM_TTL_MS);
  b.session.tick();
  assert.deepEqual(b.session.peerEntries(OBJ), []);
  assert.equal(b.session.local().claim, null);
  assert.deepEqual(b.notices, []);
});

test("a peer withdrawing its own Ask is not a Pass", () => {
  // Only a released CLAIM can be passed. Two members asking for the same unclaimed frame, one of
  // whom gives up, must not hand the frame to the other.
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  a.session.ask(OBJ, FR1);
  b.session.ask(OBJ, FR1);
  a.session.cancelAsk();
  assert.equal(b.session.local().claim, null);
  assert.deepEqual(b.notices, []);
  assert.equal(b.session.local().ask?.fr, FR1, "the remaining ask is untouched");
});

test("collisions are shown, never resolved, and both delivery orders agree", () => {
  const views: unknown[] = [];
  for (const order of [[A, B], [B, A]]) {
    const clock = new FakeClock();
    const c = member(C, clock, 3);
    const chans = new Map<string, DrawChannel>();
    for (const peer of [A, B]) chans.set(peer, c.session.openEdge(peer, () => true)!);
    for (const peer of order) {
      const r = c.session.receive(chans.get(peer)!, frame(peer === A ? SN1 : SN2, 1, FR1, 1, 0));
      assert.equal(r.ok, true);
    }
    views.push(c.session.peerEntries(OBJ));
  }
  assert.deepEqual(views[0], views[1]);
  assert.deepEqual((views[0] as { peer: string }[]).map((v) => v.peer), [A, B]);
});

test("replayed, duplicated and reordered frames are refused without changing anything", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const ch = c.session.openEdge(A, () => true)!;
  assert.equal(c.session.receive(ch, frame(SN1, 5, FR1, 1, 0)).ok, true);
  assert.equal(c.session.receive(ch, frame(SN1, 6, FR1, 0, 0)).ok, true);
  assert.deepEqual(c.session.peerEntries(), []);
  // Replaying the earlier claim, or anything at or below the high-water mark, is refused.
  assert.deepEqual(c.session.receive(ch, frame(SN1, 5, FR1, 1, 0)), { ok: false, reason: "replay" });
  assert.deepEqual(c.session.receive(ch, frame(SN1, 6, FR2, 1, 0)), { ok: false, reason: "replay" });
  assert.deepEqual(c.session.peerEntries(), []);
  // Gaps are fine (one q per frame across all of the sender's edges).
  assert.equal(c.session.receive(ch, frame(SN1, 40, FR2, 1, 0)).ok, true);
});

test("a session change drops that channel's claims; retired sessions are stale; sessions are bounded", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const ch = c.session.openEdge(A, () => true)!;
  assert.equal(c.session.receive(ch, frame(SN1, 9, FR1, 1, 0)).ok, true);
  // A new sender session (e.g. the sender exhausted q) resets the sequence domain and retracts
  // everything the old session asserted, then applies its own frame.
  assert.equal(c.session.receive(ch, frame(SN2, 0, FR2, 1, 0)).ok, true);
  assert.deepEqual(c.session.peerEntries().map((e) => e.fr), [FR2]);
  // A late frame from the retired session cannot resurrect anything.
  assert.deepEqual(c.session.receive(ch, frame(SN1, 10, FR1, 1, 0)), { ok: false, reason: "stale-session" });
  assert.deepEqual(c.session.peerEntries().map((e) => e.fr), [FR2]);
  // Session churn is bounded per channel capability.
  const extra = ["1111111111111111", "2222222222222222", "3333333333333333", "4444444444444444"];
  let accepted = 2;
  for (const sn of extra) if (c.session.receive(ch, frame(sn, 0, FR3, 1, 0)).ok) accepted += 1;
  assert.equal(accepted, DRAW_MAX_SESSIONS_PER_CHANNEL);
  assert.deepEqual(c.session.receive(ch, frame("5555555555555555", 0, FR3, 1, 0)), { ok: false, reason: "session-limit" });
});

test("stale-capability delivery is refused and a replaced channel's claims are gone", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const first = c.session.openEdge(A, () => true)!;
  assert.equal(c.session.receive(first, frame(SN1, 1, FR1, 1, 0)).ok, true);
  const second = c.session.openEdge(A, () => true)!;
  assert.deepEqual(c.session.peerEntries(), [], "reconnect retracts the old generation's claims");
  assert.deepEqual(c.session.receive(first, frame(SN1, 2, FR1, 1, 0)), { ok: false, reason: "stale-channel" });
  // The new generation starts its own sequence domain under the same sender session.
  assert.equal(c.session.receive(second, frame(SN1, 2, FR1, 1, 0)).ok, true);
  c.session.closeEdge(second);
  assert.deepEqual(c.session.peerEntries(), []);
  assert.deepEqual(c.session.receive(second, frame(SN1, 3, FR1, 1, 0)), { ok: false, reason: "stale-channel" });
});

test("removal and the end of the call clear claims, and a disposed call accepts nothing", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const link = connect(a, A, b, B);
  a.session.claim(OBJ, FR1);
  b.session.removePeer(A);
  assert.deepEqual(b.session.peerEntries(), []);
  assert.deepEqual(b.session.receive(link.atY, frame(SN1, 99, FR1, 1, 0)), { ok: false, reason: "stale-channel" });

  const c = member(C, clock, 3);
  const ch = c.session.openEdge(A, () => true)!;
  c.session.receive(ch, frame(SN1, 1, FR1, 1, 0));
  c.session.dispose();
  assert.deepEqual(c.session.peerEntries(), []);
  assert.deepEqual(c.session.receive(ch, frame(SN1, 2, FR2, 1, 0)), { ok: false, reason: "disposed" });
  assert.equal(c.session.claim(OBJ, FR1), false);
  assert.equal(c.session.openEdge(B, () => true), null);
});

test("prefix-colliding identities are independent claimants", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const victim = c.session.openEdge(VICTIM, () => true)!;
  const twin = c.session.openEdge(TWIN, () => true)!;
  assert.notEqual(victim, twin, "two identities, two capabilities, even with one fingerprint");
  assert.equal(c.session.receive(victim, frame(SN1, 1, FR1, 1, 0)).ok, true);
  // The twin releasing, re-sessioning, being removed or flooding touches only its own state.
  assert.equal(c.session.receive(twin, frame(SN2, 1, FR1, 0, 0)).ok, true);
  assert.equal(c.session.receive(twin, frame("1111111111111111", 1, FR2, 1, 0)).ok, true);
  c.session.peerUnreachable(TWIN);
  for (let i = 0; i < 2_000 && c.session.receive(twin, "x").ok === false; i += 1) { /* flood */ }
  c.session.removePeer(TWIN);
  assert.deepEqual(c.session.peerEntries().map((e) => [e.peer, e.fr]), [[VICTIM, FR1]]);
  // And the victim's own sequence domain was never consumed by the twin.
  assert.equal(c.session.receive(victim, frame(SN1, 2, FR1, 1, 0)).ok, true);
});

test("malformed frames are charged before parsing and change no state", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const ch = c.session.openEdge(A, () => true)!;
  const junk: unknown[] = [new Uint8Array([1]), "{", "x".repeat(2_000), frame(SN1, 1, FR1, 1, 1), JSON.stringify({ t: "k" })];
  for (const raw of junk) assert.equal(c.session.receive(ch, raw).ok, false);
  assert.deepEqual(c.session.peerEntries(), []);
  // Every one of those paid the all-frame bucket: the burst is now short by exactly that many.
  for (let i = junk.length; i < DRAW_FRAME_LIMITS.bucketBurst; i += 1) c.session.receive(ch, "{");
  assert.deepEqual(c.session.receive(ch, frame(SN1, 1, FR1, 1, 0)), { ok: false, reason: "frame-rate" });
});

test("a sustained flood auto-mutes the channel for the call and retracts the flooder's claims", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const ch = c.session.openEdge(A, () => true)!;
  assert.equal(c.session.receive(ch, frame(SN1, 1, FR1, 1, 0)).ok, true);
  let last = c.session.receive(ch, "{");
  for (let ms = 0; ms < 30_000 && !(last.ok === false && last.reason === "abuse-muted"); ms += 1) {
    clock.advance(1);
    last = c.session.receive(ch, "{");
  }
  assert.deepEqual(last, { ok: false, reason: "abuse-muted" });
  assert.deepEqual(c.session.peerEntries(), []);
  // Reconnecting does not clear an auto-mute: the budget belongs to the identity for the call.
  const again = c.session.openEdge(A, () => true)!;
  assert.deepEqual(c.session.receive(again, frame(SN1, 2, FR1, 1, 0)), { ok: false, reason: "abuse-muted" });
});

test("per-sender caps bound claims and asks; releases always go through", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const ch = c.session.openEdge(A, () => true)!;
  const frames = [FR1, FR2, FR3, FR4];
  let q = 0;
  for (const fr of frames) assert.equal(c.session.receive(ch, frame(SN1, ++q, fr, 1, 0)).ok, true);
  assert.equal(frames.length, CLAIM_MAX_LIVE_PER_SENDER);
  assert.deepEqual(c.session.receive(ch, frame(SN1, ++q, FR5, 0, 1)), { ok: false, reason: "claim-cap" });
  // Refreshing an existing entry is not a new one.
  assert.equal(c.session.receive(ch, frame(SN1, ++q, FR1, 1, 0)).ok, true);
  // Spend the rest of the claim burst, then show a release still lands while claims are refused.
  for (let i = 0; i < CLAIM_FRAME_BUCKET_BURST; i += 1) c.session.receive(ch, frame(SN1, ++q, FR2, 1, 0));
  assert.deepEqual(c.session.receive(ch, frame(SN1, ++q, FR3, 1, 0)), { ok: false, reason: "claim-rate" });
  assert.equal(c.session.receive(ch, frame(SN1, ++q, FR4, 0, 0)).ok, true);
  assert.equal(c.session.peerEntries().length, 3);
});

test("sender pacing coalesces a burst of frame changes so an honest receiver refuses nothing", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const c = member(C, clock, 3);
  const refusals: string[] = [];
  let atC: DrawChannel | null = null;
  const atA = a.session.openEdge(C, (raw) => {
    const r = c.session.receive(atC!, raw);
    if (!r.ok) refusals.push(r.reason);
    return true;
  })!;
  atC = c.session.openEdge(A, () => true);
  a.session.edgeReady(atA);
  const ids = Array.from({ length: 25 }, (_, i) => i.toString(16).padStart(32, "0"));
  for (const fr of ids) a.session.claim(OBJ, fr); // a user scrubbing through frames at t=0
  for (let i = 0; i < 10; i += 1) { clock.advance(500); a.session.tick(); }
  assert.deepEqual(refusals, []);
  assert.deepEqual(c.session.peerEntries(OBJ).map((e) => e.fr), [ids.at(-1)], "only the latest claim survives");
});

test("exhausting the sequence space rotates the sender session without losing the claim", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1, 3);
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  const first = a.session.sessionNonce();
  a.session.claim(OBJ, FR1);
  for (let i = 0; i < 6; i += 1) { clock.advance(CLAIM_RESEND_MS); a.session.tick(); b.session.tick(); }
  assert.notEqual(a.session.sessionNonce(), first);
  assert.deepEqual(b.session.peerEntries(OBJ).map((e) => [e.peer, e.fr]), [[A, FR1]]);
});

test("claims stay out of every Studio edit, save and native path", () => {
  // Claims are advisory. No module that decides whether an edit may happen, or performs it,
  // may consult them (a claim must never become an edit lock).
  for (const file of ["studio-session.ts", "studio-native.ts", "studio-state.svelte.ts", "studio-store.ts", "studio-contract.ts"]) {
    const source = readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
    assert.doesNotMatch(source, /draw-claims/, `${file} must not depend on claims`);
  }
  // And the claim module exposes no permission-shaped API.
  const api = Object.getOwnPropertyNames(DrawClaimsSession.prototype).join(" ");
  assert.doesNotMatch(api, /canEdit|mayEdit|isLocked|lock|permit|authori[sz]e/i);
});

test("the Studio snapshot is plain frozen data and the frame card sees collisions", async () => {
  const { snapshotClaims, frameClaimState } = await import("./draw-claims.ts");
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  a.session.claim(OBJ, FR1);
  b.session.claim(OBJ, FR1);
  const snap = snapshotClaims(b.session)!;
  assert.ok(Object.isFrozen(snap) && Object.isFrozen(snap.entries));
  assert.equal(snap.self, B);
  const card = frameClaimState(snap, OBJ, FR1);
  assert.equal(card.mine, true);
  assert.deepEqual(card.claimants.map((e) => e.peer), [A]);
  assert.equal(card.contested, true, "my claim plus a peer's is a visible collision");
  assert.equal(frameClaimState(snap, OBJ, FR2).contested, false);
  assert.equal(frameClaimState(null, OBJ, FR1).claimants.length, 0, "outside a call there is nothing");
  b.session.dispose();
  assert.equal(snapshotClaims(b.session), null);
});

test("refused frames, no-op releases and quiet ticks never bump the view revision", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const ch = c.session.openEdge(A, () => true)!;
  const rev = () => c.session.rev;
  let r = rev();
  c.session.receive(ch, "{");
  c.session.receive(ch, frame(SN1, 1, FR1, 1, 1));
  c.session.receive(ch, new Uint8Array([1]));
  assert.equal(rev(), r, "refused frames are not view changes");
  assert.equal(c.session.receive(ch, frame(SN1, 2, FR1, 0, 0)).ok, true);
  assert.equal(rev(), r, "a release that removed nothing is not a view change");
  c.session.tick();
  assert.equal(rev(), r, "a quiet tick is not a view change");
  assert.equal(c.session.isQuiet(), true);
  assert.equal(c.session.receive(ch, frame(SN1, 3, FR1, 1, 0)).ok, true);
  assert.ok(rev() > r, "a claim is");
  assert.equal(c.session.isQuiet(), false, "a live entry is counting down");
  r = rev();
  assert.equal(c.session.receive(ch, frame(SN1, 4, FR1, 1, 0)).ok, true);
  assert.ok(rev() > r, "a refresh moves the countdown");
  // Painting the frame already claimed costs nothing.
  c.session.claim(OBJ, FR2);
  r = rev();
  assert.equal(c.session.autoClaim(OBJ, FR2), false);
  assert.equal(rev(), r);
});

test("an auto-muted draw channel is reported once, without content", () => {
  const clock = new FakeClock();
  const c = member(C, clock, 3);
  const muted: string[] = [];
  c.session.onmuted = (peer) => muted.push(peer);
  const ch = c.session.openEdge(A, () => true)!;
  for (let ms = 0; ms < 30_000; ms += 1) { clock.advance(1); c.session.receive(ch, "{"); }
  assert.deepEqual(muted, [A]);
});

test("selection drives claims: moving off a frame releases it and lapses a Pass suppression", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  a.session.selectionChanged(OBJ, FR1);
  a.session.autoClaim(OBJ, FR1);
  a.session.selectionChanged(OBJ, FR1);
  assert.deepEqual(a.session.local().claim, { obj: OBJ, fr: FR1 }, "re-selecting the same frame keeps the claim");
  // Pass F, look at G without painting, come back to F and paint: F is claimed again.
  a.session.pass(OBJ, FR1);
  assert.equal(a.session.autoClaim(OBJ, FR1), false, "suppressed right after passing");
  a.session.selectionChanged(OBJ, FR2);
  a.session.selectionChanged(OBJ, FR1);
  assert.equal(a.session.autoClaim(OBJ, FR1), true);
  assert.equal(b.session.peerEntries(OBJ)[0]?.fr, FR1);
  // Moving away releases at once; an ask for another frame is withdrawn too.
  b.session.ask(OBJ, FR1);
  a.session.selectionChanged(OBJ, FR3);
  assert.equal(a.session.local().claim, null);
  assert.deepEqual(b.session.local().claim, { obj: OBJ, fr: FR1 }, "moving on is an explicit release, so the asker takes it");
  b.session.ask(OBJ, FR3);
  b.session.selectionChanged(OBJ, FR2);
  assert.equal(b.session.local().ask, null);
  assert.equal(b.session.local().claim, null, "and its own claim on FR1 went with the move");
  assert.deepEqual(a.session.peerEntries(OBJ), [], "the holder saw both withdrawn");
});

test("a session rotation keeps an outstanding Ask tracked and re-sent", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1, 3); // q may be 0, 1, 2; the fourth frame rotates the session
  const b = member(B, clock, 2);
  connect(a, A, b, B);
  b.session.claim(OBJ, FR1);
  const first = a.session.sessionNonce();
  a.session.ask(OBJ, FR1); // q 0
  a.session.claim(OBJ, FR2); // q 1
  clock.advance(CLAIM_RESEND_MS);
  a.session.tick(); // refresh, q 2
  clock.advance(CLAIM_RESEND_MS);
  a.session.tick(); // rotation: claim re-sent, the ask marked unsent and re-sent in the same tick
  assert.notEqual(a.session.sessionNonce(), first);
  assert.notEqual(a.session.local().ask, null, "the ask is still tracked after rotating");
  assert.deepEqual(b.session.asksOnMine().map((e) => e.peer), [A], "and the holder still sees it");
  assert.deepEqual(b.session.peerEntries(OBJ).filter((e) => e.kind === "claim").map((e) => e.fr), [FR2]);
  b.session.pass(OBJ, FR1);
  assert.deepEqual(a.session.local().claim, { obj: OBJ, fr: FR1 }, "so a Pass after rotation still lands");
});

test("a claim that expires inside a refused or no-op delivery still leaves the view", () => {
  // Re-review N1: the prune inside receive used to drop the expired entry without moving rev, and
  // the next quiet tick found nothing to prune, so the app kept rendering a "1 s" ghost.
  for (const late of ["noop-release", "claim-rate"] as const) {
    const clock = new FakeClock();
    const c = member(C, clock, 3);
    const fromA = c.session.openEdge(A, () => true)!;
    const fromB = c.session.openEdge(B, () => true)!;
    assert.equal(c.session.receive(fromA, frame(SN1, 1, FR1, 1, 0)).ok, true);
    let q = 0;
    clock.advance(CLAIM_TTL_MS - 100);
    if (late === "claim-rate") {
      // Spend B's whole (refilled) claim burst on one refreshed claim just before A's expires.
      for (let i = 0; i < 10; i += 1) assert.equal(c.session.receive(fromB, frame(SN2, ++q, FR2, 1, 0)).ok, true);
    }
    const before = c.session.rev;
    clock.advance(200);
    const r = late === "noop-release"
      ? c.session.receive(fromB, frame(SN2, ++q, FR3, 0, 0))
      : c.session.receive(fromB, frame(SN2, ++q, FR3, 1, 0));
    if (late === "claim-rate") assert.deepEqual(r, { ok: false, reason: "claim-rate" });
    assert.ok(c.session.rev > before, `${late}: the expiry it pruned is a visible change`);
    assert.deepEqual(c.session.peerEntries().filter((e) => e.peer === A), []);
  }
});

// --- Delivery: what a transport did not take is retried, on that edge only (Gate 5 review #1) --

type Mode = "ok" | "refuse" | "refuse-releases" | "throw" | "queue";
type Dir = { mode: Mode; took: string[]; queued: string[]; flush(): void };
/**
 * One peer connection between two members: a `draw` edge each way, whose transport can refuse
 * (`send` returns false), throw like a full WebRTC send buffer (OperationError), or take frames
 * and deliver them later. `took` logs only frames a transport accepted. `x` is x's capability for
 * frames from y, and `y` is y's capability for frames from x.
 */
function link(x: Member, xId: string, y: Member, yId: string) {
  let atX: DrawChannel | null = null;
  let atY: DrawChannel | null = null;
  const dir = (to: Member, at: () => DrawChannel): Dir => ({
    mode: "ok", took: [], queued: [],
    flush() { for (const raw of this.queued.splice(0)) to.session.receive(at(), raw); },
  });
  const xy = dir(y, () => atY!);
  const yx = dir(x, () => atX!);
  const sender = (d: Dir, to: Member, at: () => DrawChannel) => (raw: string) => {
    if (d.mode === "refuse") return false;
    if (d.mode === "refuse-releases" && raw.endsWith('"on":0,"ask":0}')) return false;
    if (d.mode === "throw") throw new DOMException("send queue is full", "OperationError");
    d.took.push(raw);
    if (d.mode === "queue") d.queued.push(raw);
    else to.session.receive(at(), raw);
    return true;
  };
  atX = x.session.openEdge(yId, sender(xy, y, () => atY!));
  atY = y.session.openEdge(xId, sender(yx, x, () => atX!));
  assert.ok(atX && atY);
  x.session.edgeReady(atX!);
  y.session.edgeReady(atY!);
  return { xy, yx, x: atX!, y: atY! };
}
function kinds(raws: readonly string[]): string[] {
  return raws.map((raw) => {
    const d = decodeDrawFrame(raw);
    assert.ok(d.ok);
    return `${d.action}:${d.frame.fr.slice(0, 2)}`;
  });
}

test("an Ask a transport refused is retried on that edge only, and delivered before expiry", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const c = member(C, clock, 3);
  const ab = link(a, A, b, B);
  const ac = link(a, A, c, C);
  ab.xy.mode = "refuse";
  assert.equal(a.session.ask(OBJ, FR1), true);
  assert.deepEqual(b.session.peerEntries(OBJ), [], "B's transport refused it");
  assert.deepEqual(kinds(ac.xy.took), ["ask:11"], "partial success: C took it");
  clock.advance(5_000);
  a.session.tick();
  assert.deepEqual(b.session.peerEntries(OBJ), [], "still refused");
  ab.xy.mode = "ok";
  clock.advance(5_000);
  a.session.tick();
  assert.deepEqual(b.session.peerEntries(OBJ).map((e) => [e.peer, e.kind]), [[A, "ask"]], "retried once B's transport takes frames");
  assert.equal(a.session.local().ask?.secondsLeft, 80, "the asker's own countdown never restarted");
  assert.deepEqual(kinds(ac.xy.took), ["ask:11"], "C, which took it the first time, was not sent it again");
  assert.equal(c.session.peerEntries(OBJ)[0]?.secondsLeft, 80, "so C's countdown did not restart");
});

test("an Ask whose send throws OperationError is retried, not forgotten", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  b.session.claim(OBJ, FR1);
  ab.xy.mode = "throw";
  a.session.ask(OBJ, FR1);
  assert.deepEqual(b.session.asksOnMine(), []);
  ab.xy.mode = "ok";
  clock.advance(1_000);
  a.session.tick();
  assert.deepEqual(b.session.asksOnMine().map((e) => e.peer), [A]);
  // ...and the handover it exists for still works.
  b.session.pass(OBJ, FR1);
  assert.deepEqual(a.session.local().claim, { obj: OBJ, fr: FR1 });
});

test("a Pass whose Release the transport refused is retried until the asker takes the frame", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  a.session.claim(OBJ, FR1);
  b.session.ask(OBJ, FR1);
  ab.xy.mode = "throw";
  assert.equal(a.session.pass(OBJ, FR1), true);
  assert.equal(a.session.local().claim, null, "the holder let go locally");
  assert.equal(b.session.local().claim, null, "but nothing reached the asker yet");
  assert.equal(b.session.peerEntries(OBJ)[0]?.kind, "claim");
  clock.advance(1_000);
  a.session.tick();
  assert.deepEqual(b.notices, []);
  ab.xy.mode = "ok";
  clock.advance(1_000);
  a.session.tick();
  assert.deepEqual(b.session.local().claim, { obj: OBJ, fr: FR1 }, "the retried Release completed the Pass");
  assert.deepEqual(b.notices, [{ kind: "passed", from: A, obj: OBJ, fr: FR1 }]);
  assert.equal(a.session.autoClaim(OBJ, FR1), false, "and the Pass suppression held throughout");
});

test("cancelling an Ask that no transport took sends nothing, then or later", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  ab.xy.mode = "refuse";
  a.session.ask(OBJ, FR1);
  a.session.cancelAsk();
  ab.xy.mode = "ok";
  for (let i = 0; i < 5; i += 1) { clock.advance(1_000); a.session.tick(); }
  assert.deepEqual(ab.xy.took, [], "no Ask and no Release for an Ask that never went out");
  assert.deepEqual(b.session.peerEntries(), []);
});

test("an Ask still undelivered when it lapses is never sent; a delivered one is withdrawn at the lapse", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const c = member(C, clock, 3);
  const ab = link(a, A, b, B);
  const ac = link(a, A, c, C);
  ab.xy.mode = "refuse";
  a.session.ask(OBJ, FR1);
  for (let i = 0; i < 90; i += 1) { clock.advance(1_000); a.session.tick(); }
  assert.equal(a.session.local().ask, null);
  ab.xy.mode = "ok";
  clock.advance(1_000);
  a.session.tick();
  assert.deepEqual(ab.xy.took, [], "the lapsed Ask is not delivered late");
  // C's receiver expired it on its own clock, but the Release still follows (a frame the transport
  // took may arrive late after an outage); at C it removes nothing.
  assert.deepEqual(kinds(ac.xy.took), ["ask:11", "release:11"]);
  assert.deepEqual(c.session.peerEntries(), []);
});

test("moving on while a claim is undelivered: the new frame is claimed, the old one never named", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  ab.xy.mode = "refuse";
  a.session.selectionChanged(OBJ, FR1);
  a.session.autoClaim(OBJ, FR1);
  a.session.selectionChanged(OBJ, FR2);
  a.session.autoClaim(OBJ, FR2);
  ab.xy.mode = "ok";
  clock.advance(1_000);
  a.session.tick();
  assert.deepEqual(kinds(ab.xy.took), ["claim:22"]);
  assert.deepEqual(b.session.peerEntries(OBJ).map((e) => e.fr), [FR2]);
});

test("a replaced channel starts with an empty record: my claim and ask go to the new edge", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  link(a, A, b, B);
  a.session.claim(OBJ, FR1);
  a.session.ask(OBJ, FR2);
  const again = link(a, A, b, B); // the peer connection was replaced
  assert.deepEqual(kinds(again.xy.took).sort(), ["ask:22", "claim:11"]);
  assert.deepEqual(b.session.peerEntries(OBJ).map((e) => [e.fr, e.kind]), [[FR1, "claim"], [FR2, "ask"]]);
});

test("a refused send costs nothing against the pacing that mirrors the receiver's bucket", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  ab.xy.mode = "refuse";
  a.session.claim(OBJ, FR1);
  // Many refused retries at one instant (no refill): each one's charge is given back.
  for (let i = 0; i < 3 * CLAIM_FRAME_BUCKET_BURST; i += 1) a.session.tick();
  ab.xy.mode = "ok";
  a.session.tick();
  assert.deepEqual(b.session.peerEntries(OBJ).map((e) => e.fr), [FR1], "the bucket still had a token");
});

test("a receiver's record is bounded while its Releases keep failing", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  // Claims go through, Releases do not: every frame change leaves one more Release owed.
  ab.xy.mode = "refuse-releases";
  const ids = Array.from({ length: 30 }, (_, i) => (i + 1).toString(16).padStart(32, "0"));
  for (const fr of ids) {
    a.session.claim(OBJ, fr);
    clock.advance(1_000);
    a.session.tick();
  }
  ab.xy.took.length = 0;
  ab.xy.mode = "ok";
  a.session.tick();
  const releases = kinds(ab.xy.took).filter((k) => k.startsWith("release"));
  assert.ok(releases.length <= DRAW_TOLD_MAX, `at most ${DRAW_TOLD_MAX} Releases owed, got ${releases.length}`);
});

// --- Reachability: an unreachable peer is absent until it recovers (Gate 5 review #3) ----------

test("deliveries from an unreachable peer are held, not shown, and applied on recovery", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ba = link(b, B, a, A);
  b.session.claim(OBJ, FR1);
  assert.equal(a.session.peerEntries(OBJ).length, 1);
  a.session.peerUnreachable(B);
  assert.deepEqual(a.session.peerEntries(OBJ), [], "absent at once");
  // A delayed onmessage, and B's refresh, arrive while A still sees the link as down.
  clock.advance(CLAIM_RESEND_MS);
  b.session.tick();
  b.session.claim(OBJ, FR2);
  assert.deepEqual(a.session.peerEntries(OBJ), [], "fresh traffic does not restore an unreachable peer");
  assert.equal(a.session.isQuiet(), false);
  clock.advance(5_000);
  const rev = a.session.rev;
  a.session.edgeReady(ba.y!);
  assert.ok(a.session.rev > rev);
  assert.deepEqual(a.session.peerEntries(OBJ).map((e) => [e.fr, e.secondsLeft]), [[FR2, 85]], "held frames applied in order, aged from arrival");
});

test("a Pass sent while the asker sees the holder unreachable lands on recovery", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ba = link(b, B, a, A);
  b.session.claim(OBJ, FR1);
  a.session.ask(OBJ, FR1);
  a.session.peerUnreachable(B);
  b.session.pass(OBJ, FR1);
  assert.equal(a.session.local().claim, null, "held, not applied");
  a.session.edgeReady(ba.y!);
  assert.deepEqual(a.session.local().claim, { obj: OBJ, fr: FR1 }, "the restored claim's Release is a Pass");
  assert.deepEqual(a.notices, [{ kind: "passed", from: B, obj: OBJ, fr: FR1 }]);
});

test("an unreachable claimant does not block a Pass, and a link that fails instead drops everything", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const c = member(C, clock, 3);
  link(a, A, b, B);
  const ca = link(c, C, a, A);
  b.session.claim(OBJ, FR1);
  c.session.claim(OBJ, FR1);
  a.session.ask(OBJ, FR1);
  a.session.peerUnreachable(C);
  b.session.pass(OBJ, FR1);
  assert.deepEqual(a.session.local().claim, { obj: OBJ, fr: FR1 }, "C is absent, so nobody else visibly claims it");
  c.session.claim(OBJ, FR2);
  a.session.closeEdge(ca.y!);
  assert.equal(a.session.receive(ca.y!, frame(SN1, 1, FR3, 1, 0)).ok, false);
  assert.deepEqual(a.session.peerEntries(OBJ).filter((e) => e.peer === C), [], "closing dropped C's hidden entries and held frames");
});

test("while I see a peer as unreachable I send it nothing; recovery sends only what changed", () => {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  a.session.claim(OBJ, FR1);
  a.session.ask(OBJ, FR3);
  ab.xy.took.length = 0;
  a.session.peerUnreachable(B);
  a.session.claim(OBJ, FR2);
  clock.advance(1_000);
  a.session.tick();
  assert.deepEqual(ab.xy.took, []);
  a.session.edgeReady(ab.x!);
  assert.deepEqual(kinds(ab.xy.took), ["release:11", "claim:22"], "the Ask it already took is not re-sent");
  assert.deepEqual(b.session.peerEntries(OBJ).map((e) => [e.fr, e.kind]), [[FR2, "claim"], [FR3, "ask"]]);
});

function suspendedEdge() {
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const chan = a.session.openEdge(B, () => true)!;
  a.session.edgeReady(chan);
  a.session.peerUnreachable(B);
  return { clock, a, chan };
}

test("held deliveries are bounded: superseded frames are shed first", () => {
  const { a, chan } = suspendedEdge();
  // A claim, then more than the bound of later frames for one other frame.
  assert.deepEqual(a.session.receive(chan, frame(SN1, 0, FR1, 1, 0)), { ok: false, reason: "held" });
  for (let q = 1; q <= DRAW_HOLD_MAX; q += 1) {
    assert.deepEqual(a.session.receive(chan, frame(SN1, q, FR2, 0, 0)), { ok: false, reason: "held" });
  }
  a.session.edgeReady(chan);
  assert.deepEqual(a.session.peerEntries().map((e) => e.fr), [FR1], "the claim, still the latest word on FR1, survived");
});

test("held deliveries are bounded: with nothing superseded, the oldest goes", () => {
  const { a, chan } = suspendedEdge();
  a.session.receive(chan, frame(SN1, 0, FR3, 1, 0));
  for (let q = 1; q <= DRAW_HOLD_MAX; q += 1) a.session.receive(chan, frame(SN1, q, q.toString(16).padStart(32, "0"), 0, 0));
  a.session.edgeReady(chan);
  assert.deepEqual(a.session.peerEntries(), [], "33 distinct frames: the oldest, the claim, was shed");
});

test("malformed deliveries are refused while unreachable, and an auto-mute discards what was held", () => {
  const { clock, a, chan } = suspendedEdge();
  assert.deepEqual(a.session.receive(chan, "{"), { ok: false, reason: "json" }, "malformed is refused now, never held");
  assert.deepEqual(a.session.receive(chan, frame(SN1, 0, FR2, 1, 0)), { ok: false, reason: "held" });
  for (let ms = 0; ms < 30_000; ms += 1) { clock.advance(1); a.session.receive(chan, "{"); }
  a.session.edgeReady(chan);
  assert.deepEqual(a.session.peerEntries(), [], "an auto-mute discards what was held");
});

// --- Second review round (32e37c3..32e089b) ------------------------------------------------------

test("held frames are charged at arrival, so a recovery replay cannot refuse the newest Ask", () => {
  // MEDIUM-1: replaying ten held claims and an Ask at one instant spent the receiver's burst and
  // refused the Ask, which the sender had recorded as taken and never re-sent.
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  clock.advance(10_000);
  b.session.peerUnreachable(A); // B sees A unreachable; A still sees B connected
  const ids = Array.from({ length: 10 }, (_, i) => (i + 1).toString(16).padStart(32, "0"));
  for (const fr of ids) a.session.claim(OBJ, fr);
  clock.advance(1_000);
  a.session.ask(OBJ, FR5);
  assert.ok(kinds(ab.xy.took).includes("ask:55"), "A's transport took the Ask");
  clock.advance(5_000);
  b.session.edgeReady(ab.y);
  assert.deepEqual(b.session.peerEntries().filter((e) => e.kind === "ask").map((e) => e.fr), [FR5], "the replayed Ask was admitted");
});

test("a Release follows a frame taken long ago, so a claim delivered after a long outage is withdrawn", () => {
  // LOW-1: a record aged out 90 s after it was TAKEN dropped the owed Release, but SCTP could still
  // deliver the claim itself after the outage; the receiver then showed it for another 90 s.
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const ab = link(a, A, b, B);
  ab.xy.mode = "queue"; // the path is silently dead; the association buffers
  a.session.claim(OBJ, FR1);
  clock.advance(5_000);
  a.session.peerUnreachable(B);
  b.session.peerUnreachable(A);
  a.session.selectionChanged(OBJ, FR2);
  a.session.autoClaim(OBJ, FR2);
  for (let i = 0; i < 95; i += 1) { clock.advance(1_000); a.session.tick(); b.session.tick(); }
  a.session.edgeReady(ab.x);
  b.session.edgeReady(ab.y);
  ab.xy.flush();
  assert.deepEqual(kinds(ab.xy.took), ["claim:11", "release:11", "claim:22"]);
  assert.deepEqual(b.session.peerEntries().map((e) => e.fr), [FR2], "no ghost of the old frame");
});

test("pacing is per peer for the call: a replaced channel does not start with a fresh burst", () => {
  // LOW-2: the receiver's bucket survives a channel replacement; a fresh sender bucket per edge let
  // the new edge's claim and Ask be refused (and the Ask recorded as taken).
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const b = member(B, clock, 2);
  const refusals: string[] = [];
  const pair = () => {
    let atB: DrawChannel | null = null;
    const atA = a.session.openEdge(B, (raw) => {
      const r = b.session.receive(atB!, raw);
      if (!r.ok) refusals.push(r.reason);
      return true;
    })!;
    atB = b.session.openEdge(A, () => true);
    a.session.edgeReady(atA);
    b.session.edgeReady(atB!);
  };
  pair();
  b.session.claim(OBJ, FR5);
  const ids = Array.from({ length: CLAIM_FRAME_BUCKET_BURST }, (_, i) => (i + 1).toString(16).padStart(32, "0"));
  for (const fr of ids) a.session.claim(OBJ, fr);
  pair(); // the peer connection is replaced straight after the burst
  a.session.ask(OBJ, FR5);
  for (let i = 0; i < 10; i += 1) { clock.advance(1_000); a.session.tick(); }
  assert.deepEqual(refusals, [], "nothing was sent that the receiver had to refuse");
  assert.deepEqual(b.session.asksOnMine().map((e) => e.peer), [A], "the Ask arrived once pacing allowed");
});

test("a Release held behind a flood of other frames is not shed", () => {
  // LOW-3: shedding the oldest held frame could drop a Release and let a stale claim reappear.
  const clock = new FakeClock();
  const a = member(A, clock, 1);
  const chan = a.session.openEdge(B, () => true)!;
  a.session.edgeReady(chan);
  a.session.receive(chan, frame(SN1, 0, FR1, 1, 0));
  assert.equal(a.session.peerEntries().length, 1);
  a.session.peerUnreachable(B);
  a.session.receive(chan, frame(SN1, 1, FR1, 0, 0)); // B released FR1
  for (let q = 2; q <= DRAW_HOLD_MAX + 1; q += 1) a.session.receive(chan, frame(SN1, q, FR2, 0, 0));
  a.session.edgeReady(chan);
  assert.deepEqual(a.session.peerEntries(), [], "the Release survived, so the hidden claim did not come back");
});

