// Contract tests for the V2 call driver. The fake native below models the contract a real
// adapter must honour (exact retries, non-dequeueing pulls, lost replies after commit, native
// OfferIds); the fake RTC can hold any operation across replacement. Neither proves native or
// WebView behaviour: they pin what the renderer does with native-authorized results.
import assert from "node:assert/strict";
import test from "node:test";
import {
  VOICE_V2_EARLY_ICE,
  VOICE_V2_MAX_PEERS,
  VOICE_V2_MAX_SDP,
  VOICE_V2_PAIR_WAIT_MS,
  VOICE_V2_REJOINS,
  VoiceV2Call,
  describeVoiceV2Peer,
  voiceV2Activation,
  type VoiceV2Action,
  type VoiceV2Batch,
  type VoiceV2Lease,
  type VoiceV2Native,
  type VoiceV2PeerConnection,
  type VoiceV2StartRequest,
  type VoiceV2StartResult,
  type VoiceV2SubmitResult,
} from "./voice-v2-call.ts";

const SELF = "5".repeat(64);
const LOW = "2".repeat(64); // below SELF: LOW initiates towards us
const HIGH = "9".repeat(64); // above SELF: we initiate towards HIGH
const GROUP = "ab".repeat(32);
const ROOM = "7";
const join = (n: number) => n.toString(16).padStart(2, "0").repeat(32);

function deferred<T = void>() {
  let resolve!: (value: T) => void, reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const settle = async (rounds = 8) => { for (let i = 0; i < rounds; i++) await new Promise((r) => setImmediate(r)); };

/** Contract model of the native V2 path for one device. */
class FakeNative implements VoiceV2Native {
  starts: VoiceV2StartRequest[] = [];
  submits: { lease: string; action: VoiceV2Action }[] = [];
  ends: string[] = [];
  polls: { lease: string; after: string }[] = [];
  /** Committed actions by id: the cached authoritative result for exact retries. */
  #committed = new Map<string, { content: string; result: VoiceV2SubmitResult }>();
  #leases = new Map<string, VoiceV2Lease>(); // by attempt
  #queues = new Map<string, unknown[]>(); // by lease, each item carries its receipt/revision
  #waiters = new Map<string, (() => void)[]>();
  #serial = 0;
  #messages = 0;
  joinCounter = 0x40;
  /** Commit the next N start/submit/events calls but lose their replies. */
  loseStart = 0; loseSubmit = 0; losePoll = 0;
  holdStart: Promise<void> | null = null;
  holdSubmit: Promise<void> | null = null;
  refuseStart: string | null = null;

  async start(request: VoiceV2StartRequest): Promise<VoiceV2StartResult> {
    this.starts.push(request);
    if (this.holdStart) await this.holdStart;
    if (this.refuseStart) return { kind: "refused", reason: this.refuseStart };
    let lease = this.#leases.get(request.attempt);
    if (!lease) {
      lease = Object.freeze({ lease: `lease-${this.#leases.size + 1}`, group: GROUP, room: request.room, self: SELF, join: join(this.joinCounter++) });
      this.#leases.set(request.attempt, lease);
      this.#queues.set(lease.lease, []);
    }
    if (this.loseStart > 0) { this.loseStart--; throw new Error("lost start reply"); }
    return { kind: "started", lease };
  }

  async events(lease: string, after: string): Promise<VoiceV2Batch> {
    this.polls.push({ lease, after });
    const queue = () => (this.#queues.get(lease) ?? []).filter((item) => BigInt((item as { revision: string }).revision) > BigInt(after));
    while (queue().length === 0 && !this.ends.includes(lease)) {
      await new Promise<void>((resolve) => { const list = this.#waiters.get(lease) ?? []; list.push(resolve); this.#waiters.set(lease, list); });
    }
    if (this.losePoll > 0) { this.losePoll--; throw new Error("lost poll reply"); }
    const items = queue();
    const revision = items.length ? (items[items.length - 1] as { revision: string }).revision : after;
    return { revision, items: items.map(({ revision: _r, ...item }: any) => item) };
  }

  async submit(lease: string, action: VoiceV2Action): Promise<VoiceV2SubmitResult> {
    this.submits.push({ lease, action });
    if (this.holdSubmit) await this.holdSubmit;
    const content = JSON.stringify({ ...action, action: undefined });
    let committed = this.#committed.get(action.action);
    if (committed && committed.content !== content) return { status: "refused", action: action.action, reason: "retry-mismatch" };
    if (!committed) {
      const offer = action.kind === "offer" ? { issuer: SELF, message: join(0x80 + ++this.#messages) } : null;
      committed = { content, result: { status: "submitted", action: action.action, offer } };
      this.#committed.set(action.action, committed);
    }
    if (this.loseSubmit > 0) { this.loseSubmit--; throw new Error("lost submit reply"); }
    return committed.result;
  }

  async end(lease: string): Promise<void> {
    this.ends.push(lease);
    this.#wake(lease);
  }

  /** Deliver one native-authorized item to a lease; returns its receipt. */
  push(lease: string, item: Record<string, unknown>, receipt?: string): string {
    const serial = String(++this.#serial);
    const r = receipt ?? serial;
    this.#queues.get(lease)!.push({ receipt: r, ...item, revision: serial });
    this.#wake(lease);
    return r;
  }
  #wake(lease: string) {
    for (const wake of this.#waiters.get(lease) ?? []) wake();
    this.#waiters.set(lease, []);
  }
  offers() { return this.submits.filter((s) => s.action.kind === "offer"); }
  answers() { return this.submits.filter((s) => s.action.kind === "answer"); }
  ices() { return this.submits.filter((s) => s.action.kind === "ice"); }
}

class FakePc implements VoiceV2PeerConnection {
  connectionState = "new";
  signalingState = "stable";
  localDescription: { type: string; sdp: string } | null = null;
  remoteDescription: { type: string; sdp: string } | null = null;
  localGate: Promise<void> | null = null;
  remoteGate: Promise<void> | null = null;
  ice: unknown[] = [];
  remotes: unknown[] = [];
  closed = false;
  localSdp = "v=0 fixture";
  onicecandidate: VoiceV2PeerConnection["onicecandidate"] = null;
  onconnectionstatechange: VoiceV2PeerConnection["onconnectionstatechange"] = null;
  readonly identity: string;
  readonly isCurrent: () => boolean;
  constructor(identity: string, isCurrent: () => boolean) {
    this.identity = identity;
    this.isCurrent = isCurrent;
  }
  async setLocalDescription() {
    if (this.localGate) await this.localGate;
    const type = this.remoteDescription?.type === "offer" ? "answer" : "offer";
    this.localDescription = { type, sdp: `${type}:${this.localSdp}` };
    this.signalingState = type === "offer" ? "have-local-offer" : "stable";
  }
  async setRemoteDescription(value: { type: "offer" | "answer"; sdp: string }) {
    this.remotes.push(value);
    if (this.remoteGate) await this.remoteGate;
    this.remoteDescription = value;
    this.signalingState = value.type === "offer" ? "have-remote-offer" : "stable";
  }
  async addIceCandidate(candidate?: unknown) { this.ice.push(candidate ?? null); }
  close() { this.closed = true; this.connectionState = "closed"; this.signalingState = "closed"; }
  gather(candidate: string) { this.onicecandidate?.({ candidate: { toJSON: () => ({ candidate, sdpMid: "0", sdpMLineIndex: 0, usernameFragment: "u" }) } }); }
  state(value: string) { this.connectionState = value; this.onconnectionstatechange?.(); }
}

let harnesses = 0;
function harness(options: { native?: FakeNative; server?: number; room?: string } = {}) {
  const native = options.native ?? new FakeNative();
  const pcs: FakePc[] = [];
  const retired: FakePc[] = [];
  const diags: string[] = [];
  let now = 1_000;
  let ids = 0;
  // Ids must be unique across drivers sharing one native, as randomUUID is in production:
  // a remounted driver reusing an old attempt id would be handed the old driver's join.
  const prefix = `h${++harnesses}`;
  const sleeps: (() => void)[] = [];
  const call = new VoiceV2Call({
    native,
    rtc: {
      create(identity, isCurrent) { const pc = new FakePc(identity, isCurrent); pcs.push(pc); return pc; },
      retire(_identity, pc) { (pc as FakePc).close(); retired.push(pc as FakePc); },
    },
    clock: {
      now: () => now,
      sleep: () => new Promise<void>((resolve) => sleeps.push(resolve)),
    },
    newId: () => `${prefix}-id-${++ids}`,
    server: options.server ?? 1,
    room: options.room ?? ROOM,
    diagnose: (event) => diags.push(event.code),
  });
  return {
    native, call, pcs, retired, diags,
    advance(ms: number) { now += ms; },
    /** Release every pending backoff sleep once. */
    wake() { for (const resolve of sleeps.splice(0)) resolve(); },
    peer(identity: string) { return call.snapshot().peers.find((p) => p.identity === identity); },
  };
}
function currentLease(native: FakeNative, call: VoiceV2Call): string {
  // Leases are numbered in mint order; the driver's current join identifies its lease.
  const target = call.snapshot().join;
  for (let n = 1; n <= native.starts.length + 1; n++) {
    const lease = `lease-${n}`;
    if (join(0x40 + n - 1) === target) return lease;
  }
  throw new Error("no current lease");
}

async function joined(h: ReturnType<typeof harness>) {
  await h.call.start();
  await settle();
  assert.equal(h.call.snapshot().phase, "joined");
  return currentLease(h.native, h.call);
}
const offerOf = (n: number, issuer = LOW) => ({ issuer, message: join(n) });

test("V2 activation is unavailable until the native adapter exists, and says what is missing", () => {
  const activation = voiceV2Activation();
  assert.equal(activation.available, false);
  assert.ok(!activation.available && activation.missing.length >= 3);
  assert.ok(!activation.available && activation.missing.some((m) => /SDP\/ICE/.test(m)));
});

test("initiator path: fresh peer, native pairing, one offer, applied answer; signalling and media stay separate", async () => {
  const h = harness();
  const lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: HIGH, join: join(1) });
  await settle();
  assert.equal(h.pcs.length, 0, "presence alone creates no RTC object");
  assert.equal(h.peer(HIGH)?.signalling, "waiting-pair");
  h.native.push(lease, { kind: "paired", identity: HIGH, join: join(1) });
  await settle();
  assert.equal(h.pcs.length, 1);
  assert.equal(h.native.offers().length, 1);
  const offer = h.native.offers()[0].action;
  assert.equal(offer.kind === "offer" && offer.peerJoin, join(1), "the offer names the exact peer join");
  assert.equal(h.peer(HIGH)?.signalling, "offer-submitted");
  assert.equal(describeVoiceV2Peer(h.peer(HIGH)!), "Offer sent, waiting for answer", "native admission is not remote receipt");
  const offerId = { issuer: SELF, message: join(0x81) };
  h.native.push(lease, { kind: "answer", identity: HIGH, join: join(1), offer: offerId, sdp: "answer-sdp" });
  await settle();
  assert.deepEqual(h.pcs[0].remotes, [{ type: "answer", sdp: "answer-sdp" }]);
  assert.equal(h.peer(HIGH)?.signalling, "negotiated");
  assert.equal(h.peer(HIGH)?.media, "none");
  assert.equal(describeVoiceV2Peer(h.peer(HIGH)!), "Signalling done, connecting media", "signalling success is not media success");
  h.pcs[0].state("connected");
  assert.equal(h.peer(HIGH)?.media, "connected");
  assert.equal(describeVoiceV2Peer(h.peer(HIGH)!), "Connected");
  h.pcs[0].state("failed");
  assert.equal(describeVoiceV2Peer(h.peer(HIGH)!), "Signalling done, no media route");
});

test("responder path: no unsolicited offer, answer echoes the exact OfferId, local ICE follows the answer", async () => {
  const h = harness();
  const lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: LOW, join: join(2) });
  h.native.push(lease, { kind: "paired", identity: LOW, join: join(2) });
  await settle();
  assert.equal(h.native.offers().length, 0, "the higher identity never offers");
  const pc = h.pcs[0];
  pc.gather("early local candidate");
  assert.equal(h.native.ices().length, 0, "local ICE waits until this end's SDP is submitted");
  h.native.push(lease, { kind: "offer", identity: LOW, join: join(2), offer: offerOf(3), sdp: "offer-sdp" });
  await settle();
  const answers = h.native.answers();
  assert.equal(answers.length, 1);
  const answer = answers[0].action;
  assert.ok(answer.kind === "answer");
  assert.deepEqual(answer.offer, offerOf(3));
  assert.equal(answer.peerJoin, join(2));
  assert.equal(h.peer(LOW)?.signalling, "answer-submitted");
  const ices = h.native.ices().map((s) => s.action);
  assert.equal(ices.length, 1);
  assert.ok(ices[0].kind === "ice");
  assert.deepEqual(ices[0].offer, offerOf(3), "local ICE carries the exact negotiation");
  pc.gather("later candidate");
  await settle();
  assert.equal(h.native.ices().length, 2);
});

test("a delayed same-room bye from a replaced join cannot dismiss the newer join", async () => {
  const h = harness();
  const lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: LOW, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: LOW, join: join(1) });
  await settle();
  const old = h.pcs[0];
  // LOW leaves and rejoins the same room with a new native join before its old bye arrives.
  h.native.push(lease, { kind: "peer", identity: LOW, join: join(2) });
  await settle();
  assert.equal(h.peer(LOW)?.join, join(1), "a working pair stays until the newer join confirms");
  h.native.push(lease, { kind: "paired", identity: LOW, join: join(2) });
  await settle();
  assert.equal(h.peer(LOW)?.join, join(2));
  assert.ok(old.closed, "promotion retires the old pair");
  const replacement = h.pcs[1];
  h.native.push(lease, { kind: "bye", identity: LOW, join: join(1) });
  h.native.push(lease, { kind: "offer", identity: LOW, join: join(1), offer: offerOf(4), sdp: "old join offer" });
  await settle();
  assert.equal(h.peer(LOW)?.join, join(2), "the old join's bye names only that join");
  assert.equal(replacement.closed, false);
  assert.equal(replacement.remotes.length, 0, "old-join SDP cannot reach the replacement peer");
  // Retired object callbacks are inert too.
  old.gather("retired ICE");
  old.state("connected");
  await settle();
  assert.equal(h.native.ices().length, 0);
  assert.equal(h.peer(LOW)?.media, "none");
  // The newer join's own bye does end it.
  h.native.push(lease, { kind: "bye", identity: LOW, join: join(2) });
  await settle();
  assert.equal(h.peer(LOW), undefined);
  assert.ok(replacement.closed);
});

test("simultaneous joins and crossed offers settle on one initiator per pair", async () => {
  const h = harness();
  const lease = await joined(h);
  for (const [identity, n] of [[LOW, 1], [HIGH, 2]] as const) {
    h.native.push(lease, { kind: "peer", identity, join: join(n) });
    h.native.push(lease, { kind: "paired", identity, join: join(n) });
  }
  await settle();
  assert.equal(h.pcs.length, 2, "one RTC object per pair, never a duplicate");
  assert.deepEqual(h.native.offers().map((s) => s.action.peer), [HIGH], "we offer only towards the higher identity");
  // HIGH also (wrongly or simultaneously) offers: the crossed offer is refused, not rolled back.
  const highPc = h.pcs.find((pc) => pc.identity === HIGH)!;
  h.native.push(lease, { kind: "offer", identity: HIGH, join: join(2), offer: offerOf(9, HIGH), sdp: "crossed" });
  await settle();
  assert.equal(highPc.remotes.length, 0);
  assert.equal(h.native.answers().length, 0);
  assert.ok(h.diags.includes("VOICE.V2.CROSSED_OFFER"));
  // LOW's offer towards us is answered once; a second, different offer in the same pair is refused.
  h.native.push(lease, { kind: "offer", identity: LOW, join: join(1), offer: offerOf(5), sdp: "first" });
  h.native.push(lease, { kind: "offer", identity: LOW, join: join(1), offer: offerOf(6), sdp: "second" });
  await settle();
  assert.equal(h.native.answers().length, 1);
  assert.ok(h.diags.includes("VOICE.V2.SECOND_OFFER"));
});

test("ICE that outruns our own OfferId is matched once it is known; foreign negotiations are released", async () => {
  const h = harness();
  const lease = await joined(h);
  const reply = deferred();
  h.native.holdSubmit = reply.promise; // our offer's native reply has not arrived yet
  h.native.push(lease, { kind: "peer", identity: HIGH, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: HIGH, join: join(1) });
  await settle();
  const pc = h.pcs[0];
  const ours = { issuer: SELF, message: join(0x81) };
  // Reordered delivery: HIGH's ICE for our offer, plus ICE naming a negotiation we never made.
  h.native.push(lease, { kind: "ice", identity: HIGH, join: join(1), offer: ours, candidate: { candidate: "early" } });
  h.native.push(lease, { kind: "ice", identity: HIGH, join: join(1), offer: offerOf(7, HIGH), candidate: { candidate: "foreign" } });
  await settle();
  assert.equal(pc.ice.length, 0, "nothing applies before the answer's remote description");
  h.native.holdSubmit = null;
  reply.resolve(); await settle();
  assert.equal(h.peer(HIGH)?.signalling, "offer-submitted");
  h.native.push(lease, { kind: "answer", identity: HIGH, join: join(1), offer: ours, sdp: "answer" });
  await settle();
  assert.deepEqual(pc.ice.map((c: any) => c.candidate), ["early"], "only the matching negotiation's ICE applies");
});

test("stale ICE is refused and early ICE is bounded per pair", async () => {
  const h = harness();
  const lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: HIGH, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: HIGH, join: join(1) });
  await settle();
  const pc = h.pcs[0];
  const ours = { issuer: SELF, message: join(0x81) };
  for (let n = 0; n < VOICE_V2_EARLY_ICE + 1; n++) {
    h.native.push(lease, { kind: "ice", identity: HIGH, join: join(1), offer: ours, candidate: { candidate: `early-${n}` } });
  }
  await settle();
  assert.equal(pc.ice.length, 0);
  assert.ok(h.diags.includes("VOICE.V2.EARLY_ICE_FULL"), "the ninth early candidate is refused, not evicting the first");
  h.native.push(lease, { kind: "answer", identity: HIGH, join: join(1), offer: ours, sdp: "answer" });
  await settle();
  assert.deepEqual(pc.ice.map((c: any) => c.candidate), Array.from({ length: VOICE_V2_EARLY_ICE }, (_, n) => `early-${n}`));
  // ICE for a retired/foreign negotiation or an old join is refused, never reassigned.
  h.native.push(lease, { kind: "ice", identity: HIGH, join: join(1), offer: offerOf(8, HIGH), candidate: { candidate: "stale" } });
  h.native.push(lease, { kind: "ice", identity: HIGH, join: join(3), offer: ours, candidate: { candidate: "old join" } });
  await settle();
  assert.equal(pc.ice.length, VOICE_V2_EARLY_ICE);
  assert.ok(h.diags.includes("VOICE.V2.STALE_ICE"));
  assert.ok(h.diags.includes("VOICE.V2.STALE_JOIN"));
  h.native.push(lease, { kind: "ice", identity: HIGH, join: join(1), offer: ours, candidate: { candidate: "current" } });
  await settle();
  assert.equal((pc.ice.at(-1) as any).candidate, "current");
});

test("lost replies retry the exact request and never mint a second join, offer or application", async () => {
  const native = new FakeNative();
  native.loseStart = 2;
  const h = harness({ native });
  const starting = h.call.start();
  await settle(); h.wake(); await settle(); h.wake(); await settle();
  await starting;
  assert.equal(native.starts.length, 3);
  assert.equal(new Set(native.starts.map((s) => s.attempt)).size, 1, "every start retry is the same attempt");
  assert.equal(h.call.snapshot().phase, "joined");
  const lease = currentLease(native, h.call);
  assert.equal(lease, "lease-1", "a lost start reply recovered the same native join");
  native.loseSubmit = 1;
  native.push(lease, { kind: "peer", identity: HIGH, join: join(1) });
  native.push(lease, { kind: "paired", identity: HIGH, join: join(1) });
  await settle(); h.wake(); await settle();
  const offers = native.offers();
  assert.equal(offers.length, 2, "one retry after the lost reply");
  assert.equal(offers[0].action.action, offers[1].action.action, "the retry is the same action");
  assert.deepEqual(offers[0].action, offers[1].action, "with the same bytes");
  assert.equal(h.peer(HIGH)?.signalling, "offer-submitted");
  // A lost pull reply repeats the same cursor; the redelivered item applies once.
  native.losePoll = 1;
  const offerId = { issuer: SELF, message: join(0x81) };
  native.push(lease, { kind: "answer", identity: HIGH, join: join(1), offer: offerId, sdp: "answer" });
  await settle(); h.wake(); await settle();
  assert.equal(h.pcs[0].remotes.length, 1);
  const repeated = native.polls.filter((p) => p.lease === lease).map((p) => p.after);
  assert.ok(repeated.some((after, i) => i > 0 && repeated[i - 1] === after), "the uncertain pull repeated its cursor");
});

test("a changed retry under one action id is refused by native and never applied twice", async () => {
  const native = new FakeNative();
  const first = await native.submit("lease-x", { kind: "offer", action: "a", peer: HIGH, peerJoin: join(1), sdp: "one" });
  const again = await native.submit("lease-x", { kind: "offer", action: "a", peer: HIGH, peerJoin: join(1), sdp: "one" });
  const changed = await native.submit("lease-x", { kind: "offer", action: "a", peer: HIGH, peerJoin: join(1), sdp: "two" });
  assert.deepEqual(first, again, "exact retry returns the cached result");
  assert.equal(changed.status, "refused", "the contract the driver relies on");
});

test("duplicate native results apply once: receipts, confirmations and answers", async () => {
  const h = harness();
  const lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: HIGH, join: join(1) }, "41");
  h.native.push(lease, { kind: "peer", identity: HIGH, join: join(1) }, "41");
  h.native.push(lease, { kind: "paired", identity: HIGH, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: HIGH, join: join(1) });
  await settle();
  assert.equal(h.pcs.length, 1, "a duplicate confirmation creates no second peer object");
  assert.equal(h.native.offers().length, 1);
  const offerId = { issuer: SELF, message: join(0x81) };
  h.native.push(lease, { kind: "answer", identity: HIGH, join: join(1), offer: offerId, sdp: "answer" }, "77");
  h.native.push(lease, { kind: "answer", identity: HIGH, join: join(1), offer: offerId, sdp: "answer" }, "77");
  h.native.push(lease, { kind: "answer", identity: HIGH, join: join(1), offer: offerId, sdp: "answer again" });
  await settle();
  assert.equal(h.pcs[0].remotes.length, 1, "one remote description per negotiation");
});

test("remount: a start finishing after the old driver ended is ended, never adopted", async () => {
  const native = new FakeNative();
  const gate = deferred();
  native.holdStart = gate.promise;
  const old = harness({ native });
  const starting = old.call.start();
  await settle();
  old.call.end("unmount");
  native.holdStart = null;
  gate.resolve();
  await starting; await settle();
  assert.deepEqual(native.ends, ["lease-1"], "the late native join is ended so its owner cannot linger");
  assert.equal(old.call.snapshot().phase, "ended");
  assert.equal(old.call.snapshot().join, "");
  // The remounted component's driver starts its own fresh join.
  const fresh = harness({ native });
  const lease = await joined(fresh);
  assert.equal(lease, "lease-2");
  assert.notEqual(native.starts[0].attempt, native.starts[1].attempt);
  native.push("lease-1", { kind: "peer", identity: HIGH, join: join(1) });
  await settle();
  assert.equal(fresh.call.snapshot().peers.length, 0, "old lease traffic reaches nobody");
});

test("lock retires peers synchronously and fences suspended RTC work; unlock restores nothing", async () => {
  const h = harness();
  const lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: LOW, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: LOW, join: join(1) });
  await settle();
  const pc = h.pcs[0];
  const gate = deferred();
  pc.remoteGate = gate.promise;
  h.native.push(lease, { kind: "offer", identity: LOW, join: join(1), offer: offerOf(3), sdp: "offer" });
  await settle();
  h.call.end("lock");
  assert.ok(pc.closed, "the peer object is retired before any await");
  assert.deepEqual(h.native.ends, [lease]);
  gate.resolve(); await settle();
  assert.equal(h.native.answers().length, 0, "the suspended offer continuation cannot answer");
  pc.gather("after lock");
  await settle();
  assert.equal(h.native.ices().length, 0);
  // After unlock App builds a new driver: a fresh attempt and a fresh native join, no old peers.
  const after = harness({ native: h.native });
  const relock = await joined(after);
  assert.notEqual(relock, lease);
  assert.equal(after.call.snapshot().peers.length, 0);
});

test("two calls never share state across servers, even with the same room id", async () => {
  const native = new FakeNative();
  const a = harness({ native, server: 1 });
  const leaseA = await joined(a);
  const b = harness({ native, server: 2 });
  const leaseB = await joined(b);
  assert.deepEqual(native.starts.map((s) => s.server), [1, 2], "each call starts on its own server");
  native.push(leaseA, { kind: "peer", identity: HIGH, join: join(1) });
  await settle();
  assert.equal(a.call.snapshot().peers.length, 1);
  assert.equal(b.call.snapshot().peers.length, 0);
  b.call.end("server-left");
  await settle();
  assert.equal(a.call.snapshot().phase, "joined");
  assert.deepEqual(native.ends, [leaseB]);
});

test("epoch replacement rejoins with a fresh join and never revives old peers; the budget is bounded", async () => {
  const h = harness();
  let lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: HIGH, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: HIGH, join: join(1) });
  await settle();
  const old = h.pcs[0];
  const oldJoin = h.call.snapshot().join;
  h.native.push(lease, { kind: "ended", reason: "epoch" });
  await settle();
  assert.ok(old.closed);
  assert.equal(h.call.snapshot().phase, "joined");
  assert.notEqual(h.call.snapshot().join, oldJoin, "a fresh native join, not the old one");
  assert.equal(h.call.snapshot().peers.length, 0, "no old peer, SDP or ICE carries over");
  assert.ok(h.native.ends.includes(lease));
  assert.notEqual(h.native.starts[0].attempt, h.native.starts[1].attempt);
  // Late traffic for the old lease is never read again.
  h.native.push(lease, { kind: "peer", identity: LOW, join: join(2) });
  await settle();
  assert.equal(h.call.snapshot().peers.length, 0);
  for (let n = 1; n < VOICE_V2_REJOINS; n++) {
    lease = currentLease(h.native, h.call);
    h.native.push(lease, { kind: "ended", reason: "policy" });
    await settle();
    assert.equal(h.call.snapshot().phase, "joined");
  }
  lease = currentLease(h.native, h.call);
  h.native.push(lease, { kind: "ended", reason: "epoch" });
  await settle();
  assert.equal(h.call.snapshot().phase, "unavailable");
  assert.equal(h.call.snapshot().reason, "epoch:rejoin-budget", "bounded, and still V2: no legacy fallback");
});

test("removal, room deletion and lock from native end the call without a rejoin", async () => {
  for (const reason of ["removed", "room", "locked", "replaced"]) {
    const h = harness();
    const lease = await joined(h);
    h.native.push(lease, { kind: "ended", reason });
    await settle();
    assert.equal(h.call.snapshot().phase, "unavailable");
    assert.equal(h.call.snapshot().reason, reason);
    assert.equal(h.native.starts.length, 1, `${reason} never starts another join`);
  }
});

test("bridge loss is delay, not downgrade: held submissions finish later, uncertainty stays V2", async () => {
  const h = harness();
  const lease = await joined(h);
  const bridge = deferred();
  h.native.holdSubmit = bridge.promise; // B vanished; native is hedging to D within the packet's life
  h.native.push(lease, { kind: "peer", identity: LOW, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: LOW, join: join(1) });
  h.native.push(lease, { kind: "offer", identity: LOW, join: join(1), offer: offerOf(3), sdp: "offer" });
  await settle();
  assert.equal(h.peer(LOW)?.signalling, "answering");
  h.advance(60_000);
  assert.equal(h.peer(LOW)?.signalling, "answering", "time alone changes nothing and selects no legacy path");
  h.native.holdSubmit = null;
  bridge.resolve(); await settle();
  assert.equal(h.peer(LOW)?.signalling, "answer-submitted");
  // Every retry uncertain: report uncertainty, keep the V2 peer.
  const h2 = harness();
  const lease2 = await joined(h2);
  h2.native.loseSubmit = 99;
  h2.native.push(lease2, { kind: "peer", identity: HIGH, join: join(1) });
  h2.native.push(lease2, { kind: "paired", identity: HIGH, join: join(1) });
  await settle();
  for (let i = 0; i < 4; i++) { h2.wake(); await settle(); }
  assert.equal(h2.peer(HIGH)?.signalling, "uncertain");
  assert.equal(describeVoiceV2Peer(h2.peer(HIGH)!), "Signalling uncertain, still trying");
  assert.equal(h2.call.snapshot().mode, "v2");
});

test("presence expiry keeps live media honest; removal always tears down", async () => {
  const h = harness();
  const lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: LOW, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: LOW, join: join(1) });
  await settle();
  h.pcs[0].state("connected");
  h.native.push(lease, { kind: "peer-gone", identity: LOW, join: join(1), reason: "expired" });
  await settle();
  assert.equal(h.peer(LOW)?.presence, "stale");
  assert.equal(describeVoiceV2Peer(h.peer(LOW)!), "Connected, presence not renewed");
  h.native.push(lease, { kind: "peer-gone", identity: LOW, join: join(1), reason: "removed" });
  await settle();
  assert.equal(h.peer(LOW), undefined);
  assert.ok(h.pcs[0].closed);
});

test("start refusal reports V2 unavailable instead of falling back", async () => {
  const native = new FakeNative();
  native.refuseStart = "no-policy";
  const h = harness({ native });
  await h.call.start();
  assert.equal(h.call.snapshot().phase, "unavailable");
  assert.equal(h.call.snapshot().reason, "start-refused:no-policy");
  assert.equal(h.call.snapshot().mode, "v2");
});

test("malformed native projections are refused before any RTC effect", async () => {
  const h = harness();
  const lease = await joined(h);
  const bad: Record<string, unknown>[] = [
    { kind: "peer", identity: SELF, join: join(1) }, // self is never a counterpart
    { kind: "peer", identity: HIGH.slice(0, 8), join: join(1) }, // a fingerprint is not an identity
    { kind: "peer", identity: HIGH, join: "0".repeat(64) },
    { kind: "offer", identity: LOW, join: join(1), offer: offerOf(3, HIGH), sdp: "issuer mismatch" },
    { kind: "offer", identity: LOW, join: join(1), offer: offerOf(3), sdp: "x".repeat(VOICE_V2_MAX_SDP + 1) },
    { kind: "answer", identity: HIGH, join: join(1), offer: offerOf(3, HIGH), sdp: "answers echo our own offer" },
    { kind: "ended", reason: "because" },
    { kind: "mystery", identity: HIGH, join: join(1) },
  ];
  for (const item of bad) h.native.push(lease, item);
  await settle();
  assert.equal(h.call.snapshot().peers.length, 0);
  assert.equal(h.call.snapshot().phase, "joined");
  assert.equal(h.diags.filter((d) => d === "VOICE.V2.ITEM_REFUSED").length, bad.length);
});

test("fresh peers beyond the per-call ceiling are refused, not evicted", async () => {
  const h = harness();
  const lease = await joined(h);
  for (let n = 0; n <= VOICE_V2_MAX_PEERS; n++) {
    const identity = (0xa0 + n).toString(16).repeat(32);
    h.native.push(lease, { kind: "peer", identity, join: join(n + 1) });
  }
  await settle();
  assert.equal(h.call.snapshot().peers.length, VOICE_V2_MAX_PEERS);
  assert.ok(h.diags.includes("VOICE.V2.PEER_CAPACITY"));
});

test("an answer that outruns our own offer's submit reply waits for the OfferId instead of being lost", async () => {
  const h = harness();
  const lease = await joined(h);
  const reply = deferred();
  h.native.holdSubmit = reply.promise;
  h.native.push(lease, { kind: "peer", identity: HIGH, join: join(1) });
  h.native.push(lease, { kind: "paired", identity: HIGH, join: join(1) });
  await settle();
  // The remote answered over a fast route while our native submit reply is still delayed.
  h.native.push(lease, { kind: "answer", identity: HIGH, join: join(1), offer: { issuer: SELF, message: join(0x81) }, sdp: "fast answer" });
  await settle();
  assert.equal(h.pcs[0].remotes.length, 0);
  h.native.holdSubmit = null;
  reply.resolve(); await settle();
  assert.deepEqual(h.pcs[0].remotes, [{ type: "answer", sdp: "fast answer" }]);
  assert.equal(h.peer(HIGH)?.signalling, "negotiated");
  assert.ok(!h.diags.includes("VOICE.V2.STALE_ANSWER"));
});

test("retirement of an old join before the newer join's confirmation keeps the newer join", async () => {
  for (const retirement of ["bye", "peer-gone"]) {
    const h = harness();
    const lease = await joined(h);
    h.native.push(lease, { kind: "peer", identity: LOW, join: join(1) });
    h.native.push(lease, { kind: "paired", identity: LOW, join: join(1) });
    h.native.push(lease, { kind: "peer", identity: LOW, join: join(2) });
    // Native promotion order is not fixed: the old join's retirement arrives first.
    h.native.push(lease, retirement === "bye"
      ? { kind: "bye", identity: LOW, join: join(1) }
      : { kind: "peer-gone", identity: LOW, join: join(1), reason: "retired" });
    await settle();
    assert.ok(h.pcs[0].closed, "the old pair is retired");
    assert.equal(h.peer(LOW)?.join, join(2), `${retirement}: the newer join becomes the candidate`);
    assert.equal(h.peer(LOW)?.signalling, "waiting-pair");
    h.native.push(lease, { kind: "paired", identity: LOW, join: join(2) });
    await settle();
    assert.equal(h.pcs.length, 2);
    assert.equal(h.peer(LOW)?.signalling, "paired");
  }
});

test("ending during an uncertain start still learns the native join and ends it", async () => {
  const native = new FakeNative();
  const gate = deferred();
  native.holdStart = gate.promise;
  native.loseStart = 1;
  const h = harness({ native });
  const starting = h.call.start();
  await settle();
  h.call.end("leave"); // the user leaves while the first start's reply is in flight
  native.holdStart = null;
  gate.resolve(); await settle(); // that reply is lost: native committed a join we cannot name
  h.wake(); await settle();
  await starting;
  assert.equal(native.starts.length, 2);
  assert.equal(native.starts[0].attempt, native.starts[1].attempt, "the same attempt recovers the same join");
  assert.deepEqual(native.ends, ["lease-1"], "so no phantom participant is left behind");
  assert.equal(h.call.snapshot().phase, "ended");
});

test("an ended call that never sent a start sends none", async () => {
  const h = harness();
  h.call.end("leave");
  await h.call.start();
  assert.equal(h.native.starts.length, 0);
});

test("sustained uncertain pulls report V2 unavailable, keep polling and recover", async () => {
  const native = new FakeNative();
  const h = harness({ native });
  const lease = await joined(h);
  native.losePoll = 8;
  native.push(lease, { kind: "peer", identity: HIGH, join: join(1) });
  // Failure k happens after k-1 backoff wakes: seven wakes reach the eighth failure.
  for (let i = 0; i < 7; i++) { await settle(); h.wake(); }
  await settle();
  assert.equal(h.call.snapshot().phase, "unavailable");
  assert.equal(h.call.snapshot().reason, "events-uncertain");
  h.wake(); await settle();
  assert.equal(h.call.snapshot().phase, "joined", "a successful pull restores the joined state");
  assert.equal(h.peer(HIGH)?.join, join(1), "and the repeated cursor still delivers the item once");
});

test("V2 activation can only become available together with App driver wiring", async () => {
  const { readFileSync } = await import("node:fs");
  const app = readFileSync(new URL("./App.svelte", import.meta.url), "utf8");
  // Flipping activation alone would pin joins to V2, suppress all legacy traffic and never build
  // a driver: a silent dead call. This fails the moment activation outruns the wiring.
  if (voiceV2Activation().available) assert.match(app, /new VoiceV2Call\(/);
  else assert.doesNotMatch(app, /new VoiceV2Call\(/, "no half-wired driver exists while unavailable");
});

test("a peer that never completes key confirmation is reported, still without fallback", async () => {
  const h = harness();
  const lease = await joined(h);
  h.native.push(lease, { kind: "peer", identity: HIGH, join: join(1) });
  await settle();
  h.advance(VOICE_V2_PAIR_WAIT_MS + 1);
  assert.equal(h.peer(HIGH)?.signalling, "pair-unavailable");
  assert.equal(describeVoiceV2Peer(h.peer(HIGH)!), "In the room, key confirmation not reached");
  assert.equal(h.pcs.length, 0);
});
