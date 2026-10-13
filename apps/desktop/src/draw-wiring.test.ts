import assert from "node:assert/strict";
import test from "node:test";
import { DRAW_CHANNEL_ID, DRAW_CHANNEL_LABEL, DrawClaimsSession, encodeClaimFrame } from "./draw-claims.ts";
import { drawConnectionState, openDrawEdge, type DrawDataChannel } from "./draw-wiring.ts";

const A = "aa".repeat(32);
const B = "bb".repeat(32);
const OBJ = "0123456789abcdef0123456789abcdef";
const FR1 = "11".repeat(16);
const SN = "0123456789abcdef";

class FakeChannel implements DrawDataChannel {
  readyState = "connecting";
  sent: string[] = [];
  closed = false;
  throwOnSend = false;
  onopen: ((ev: Event) => unknown) | null = null;
  onmessage: ((ev: MessageEvent) => unknown) | null = null;
  onclose: ((ev: Event) => unknown) | null = null;
  send(data: string) {
    if (this.throwOnSend) throw new DOMException("send queue is full", "OperationError");
    this.sent.push(data);
  }
  close() { this.closed = true; this.readyState = "closed"; }
  open() { this.readyState = "open"; this.onopen?.(new Event("open")); }
  deliver(data: string) { this.onmessage?.(new MessageEvent("message", { data })); }
}

function setup(opts: { live?: () => boolean; refuse?: boolean } = {}) {
  let t = 1_000;
  const clock = { now: () => t, advance: (ms: number) => { t += ms; } };
  let n = 0;
  const claims = new DrawClaimsSession({ self: A, now: clock.now, randomHex: (b) => (++n).toString(16).padStart(b * 2, "0") });
  const made: Array<{ label: string; init: RTCDataChannelInit; dc: FakeChannel }> = [];
  const pc = {
    createDataChannel(label: string, init: RTCDataChannelInit) {
      if (opts.refuse) throw new Error("no data channels");
      const dc = new FakeChannel();
      made.push({ label, init, dc });
      return dc;
    },
  };
  let changes = 0;
  const edge = openDrawEdge(pc, claims, B, { live: opts.live ?? (() => true), changed: () => { changes += 1; } });
  return { claims, clock, made, edge, changes: () => changes };
}

test("the draw channel is negotiated id 8, ordered, created before any callback can run", () => {
  const { made, edge } = setup();
  assert.equal(made.length, 1);
  assert.equal(made[0]!.label, DRAW_CHANNEL_LABEL);
  assert.deepEqual(made[0]!.init, { negotiated: true, id: DRAW_CHANNEL_ID, ordered: true });
  assert.ok(edge);
});

test("a send before the channel opens, or one that throws, is retried once it can go", () => {
  const { claims, made, clock } = setup();
  const dc = made[0]!.dc;
  claims.claim(OBJ, FR1);
  assert.deepEqual(dc.sent, [], "not open: refused, nothing queued");
  dc.open();
  assert.equal(dc.sent.length, 1, "open: the claim the transport refused goes out");
  claims.release();
  dc.throwOnSend = true;
  claims.claim(OBJ, FR1);
  const before = dc.sent.length;
  dc.throwOnSend = false;
  clock.advance(1_000);
  claims.tick();
  assert.equal(dc.sent.length, before + 1, "the throwing send was retried on the next tick");
});

test("open only readies a live edge; messages and close reach the session; views rebuild only on change", () => {
  let live = false;
  const { claims, made, changes } = setup({ live: () => live });
  const dc = made[0]!.dc;
  dc.open();
  assert.equal(dc.sent.length, 0);
  live = true;
  dc.open();
  const c0 = changes();
  dc.deliver("{");
  assert.equal(changes(), c0, "a refused frame rebuilds nothing");
  dc.deliver(encodeClaimFrame({ sn: SN, q: 0, obj: OBJ, fr: FR1, on: 1, ask: 0 }));
  assert.equal(claims.peerEntries().length, 1);
  assert.ok(changes() > c0);
  const c1 = changes();
  dc.onclose?.(new Event("close"));
  assert.deepEqual(claims.peerEntries(), [], "closing the channel drops its claims");
  assert.ok(changes() > c1);
});

test("connection state: leaving connected hides the peer; connected again on an open channel recovers it", () => {
  const { claims, made, edge } = setup();
  const dc = made[0]!.dc;
  dc.open();
  dc.deliver(encodeClaimFrame({ sn: SN, q: 0, obj: OBJ, fr: FR1, on: 1, ask: 0 }));
  let changes = 0;
  const changed = () => { changes += 1; };
  for (const state of ["new", "connecting"] as const) drawConnectionState(claims, B, state, edge, changed);
  assert.equal(changes, 0);
  assert.equal(claims.peerEntries().length, 1);
  drawConnectionState(claims, B, "disconnected", edge, changed);
  assert.deepEqual(claims.peerEntries(), []);
  assert.equal(changes, 1);
  dc.deliver(encodeClaimFrame({ sn: SN, q: 1, obj: OBJ, fr: FR1, on: 1, ask: 0 }));
  assert.deepEqual(claims.peerEntries(), [], "a delivery while unreachable is held, not shown");
  drawConnectionState(claims, B, "failed", edge, changed);
  assert.equal(changes, 1, "already unreachable: nothing further to show");
  drawConnectionState(claims, B, "connected", edge, changed);
  assert.equal(claims.peerEntries().length, 1);
  assert.equal(changes, 2);
});

test("no data channels, or an ended call, leaves the edge without claims and closes what it made", () => {
  assert.equal(setup({ refuse: true }).edge, null);
  let t = 0;
  const claims = new DrawClaimsSession({ self: A, now: () => t++, randomHex: (b) => "1".repeat(b * 2) });
  claims.dispose();
  const dc = new FakeChannel();
  assert.equal(openDrawEdge({ createDataChannel: () => dc }, claims, B, { live: () => true, changed: () => {} }), null);
  assert.equal(dc.closed, true);
});
