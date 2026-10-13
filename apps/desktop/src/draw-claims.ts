// draw-claims.ts: collaborative Flipnote frame claims on the call `draw` channel (Gate 5;
// design-creative-suite.md 2.7 and 2.10; docs/GATE5-DESIGN.md).
//
// A claim says "I am drawing frame `fr` of flipnote `obj`". It is ADVISORY coordination between
// the members of one call: it is never authorization and never an edit lock. Nothing in the
// Studio's edit/save paths imports this module (a source-hygiene test pins that).
//
// Wire: one exact seven-key JSON frame on the negotiated, ordered `draw` data channel (id 8):
//
//   {"t":"c","sn":<16 hex>,"q":<uint32>,"obj":<32 hex>,"fr":<32 hex>,"on":0|1,"ask":0|1}
//
//   on:1 ask:0  Claim (also the 30 s refresh)
//   on:0 ask:1  Ask: "I would like this frame; whoever is drawing it may Pass"
//   on:0 ask:0  Release: withdraw my claim AND my ask on (obj, fr); Pass is a Release that answers
//               an Ask
//   on:1 ask:1  malformed
//
// Authority comes only from the opaque per-generation channel capability the caller minted for
// the AUTHENTICATED peer identity (the full 64-hex device id the call signal was verified under).
// No frame field names a sender. Sequence numbers are bound to (capability, sender session `sn`):
// within a session `q` must strictly increase, a new `sn` is a session change that drops that
// capability's claims, a retired `sn` is stale, and a capability may change session at most
// DRAW_MAX_SESSIONS_PER_CHANNEL times. Expiry is receiver-observed: an entry lives while less
// than 90 s have passed on THIS device's monotonic clock since its last accepted receipt; no
// timestamps travel. Everything is per call join: the session object is disposed on leave.
//
// Delivery: each edge records what its receiver was told by frames the transport actually TOOK
// (`send` returned true without throwing). A refused or throwing send leaves that record alone, so
// the next flush (every action, every 1 s tick, every recovery) retries on that edge only, and an
// edge that took a frame is never sent it again. What the record says the receiver holds and this
// device no longer asserts gets a Release; anything older than 90 s the receiver has expired.
//
// Reachability: while a peer's connection is not "connected", its entries are hidden from every
// view and its deliveries are held (bounded); recovery shows what is still live and applies the
// held frames in order, so a Pass sent during the gap still lands. Failure closes the channel,
// which drops both.
//
// Time and randomness are injected (`now`, `randomHex`) so tests are deterministic.

import { CLAIM_RESEND_MS, CLAIM_TTL_MS } from "./studio-contract.ts";
import {
  RtPeerAdmission,
  RtSourceChannelRegistry,
  RtTokenBucket,
  fitsUtf8Bytes,
  type RtAdmissionDenial,
  type RtFrameLimits,
  type RtSourceChannel,
} from "./rt-admission.ts";

export { CLAIM_RESEND_MS, CLAIM_TTL_MS };

/** The negotiated data channel the claims ride: created before the first offer, never renegotiated. */
export const DRAW_CHANNEL_LABEL = "draw";
export const DRAW_CHANNEL_ID = 8;

/**
 * The draw channel's pre-parse gate. These are deliberately the jam `inst` numbers: claims alone
 * need far less, but a Gate 5 receiver must not auto-mute a later C4a peer's legitimate stroke
 * traffic (unsupported frame types are still charged here), and an auto-mute is for the call.
 */
export const DRAW_FRAME_LIMITS: RtFrameLimits = Object.freeze({
  maxBytes: 1024,
  bucketBurst: 160,
  bucketRatePerSecond: 80,
  abuseExhaustedMs: 10_000,
  abuseWindowMs: 60_000,
});
/** Every draw frame type fits this after the pre-parse cap; claims are checked before JSON.parse. */
export const DRAW_SMALL_FRAME_MAX_BYTES = 200;
/** `q` is a uint32; the top value is reserved so a sender reopens under a fresh `sn` instead. */
export const DRAW_SEQUENCE_MAX = 0xffff_ffff;
export const DRAW_SESSION_HEX = 16;
/** Studio element ids (objects and frames) are 32 lowercase hex (`ELEMENT_ID_HEX`). */
export const CLAIM_ID_HEX = 32;
/** Claims and Asks (not Releases, which only shrink state) per identity per call. */
export const CLAIM_FRAME_BUCKET_BURST = 10;
export const CLAIM_FRAME_BUCKET_RATE = 2;
/** Live claims plus asks one sender may hold at one receiver. */
export const CLAIM_MAX_LIVE_PER_SENDER = 4;
/** Distinct sender sessions one channel capability may present before it must reconnect. */
export const DRAW_MAX_SESSIONS_PER_CHANNEL = 4;
/** Deliveries held from one unreachable edge until it recovers; the oldest go first beyond this. */
export const DRAW_HOLD_MAX = 32;
/**
 * Frames one edge's record may hold. A receiver keeps at most CLAIM_MAX_LIVE_PER_SENDER of mine,
 * so beyond this the oldest record is forgotten; its entry simply expires there.
 */
export const DRAW_TOLD_MAX = 2 * CLAIM_MAX_LIVE_PER_SENDER;

export type ClaimAction = "claim" | "ask" | "release";
export type ClaimFrame = Readonly<{
  t: "c";
  sn: string;
  q: number;
  obj: string;
  fr: string;
  on: 0 | 1;
  ask: 0 | 1;
}>;

export type DrawDecodeFailure = "small-frame" | "json" | "shape" | "unsupported";
export type DrawDecode =
  | Readonly<{ ok: true; frame: ClaimFrame; action: ClaimAction }>
  | Readonly<{ ok: false; reason: DrawDecodeFailure }>;

const SESSION = new RegExp(`^[0-9a-f]{${DRAW_SESSION_HEX}}$`);
const ELEMENT = new RegExp(`^[0-9a-f]{${CLAIM_ID_HEX}}$`);
const CLAIM_KEYS = ["ask", "fr", "obj", "on", "q", "sn", "t"] as const;

function plainRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : null;
}

function validSession(value: unknown): value is string {
  return typeof value === "string" && SESSION.test(value) && !/^0+$/.test(value);
}

function validSequence(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 && value < DRAW_SEQUENCE_MAX;
}

/** True for a Studio element id as claims carry it (an object or frame id). */
export function isClaimElementId(value: unknown): value is string {
  return typeof value === "string" && ELEMENT.test(value);
}

/**
 * Decode one already-admitted draw frame. The caller has charged the pre-parse budget; this
 * bounds the frame to the small cap BEFORE JSON.parse, then requires the exact key set.
 */
export function decodeDrawFrame(raw: string): DrawDecode {
  if (!fitsUtf8Bytes(raw, DRAW_SMALL_FRAME_MAX_BYTES)) return { ok: false, reason: "small-frame" };
  let value: unknown;
  try { value = JSON.parse(raw); } catch { return { ok: false, reason: "json" }; }
  const m = plainRecord(value);
  if (!m || typeof m.t !== "string") return { ok: false, reason: "shape" };
  // Only claims exist in Gate 5. Other draw types (strokes, replay, presence) are C4a and are
  // refused, after having paid the same pre-parse budget as everything else.
  if (m.t !== "c") return { ok: false, reason: "unsupported" };
  const keys = Object.keys(m).sort();
  if (keys.length !== CLAIM_KEYS.length || keys.some((k, i) => k !== CLAIM_KEYS[i])) return { ok: false, reason: "shape" };
  if (
    !validSession(m.sn) || !validSequence(m.q) || !isClaimElementId(m.obj) || !isClaimElementId(m.fr) ||
    (m.on !== 0 && m.on !== 1) || (m.ask !== 0 && m.ask !== 1) || (m.on === 1 && m.ask === 1)
  ) return { ok: false, reason: "shape" };
  const frame = m as unknown as ClaimFrame;
  const action: ClaimAction = frame.on === 1 ? "claim" : frame.ask === 1 ? "ask" : "release";
  return { ok: true, frame, action };
}

/** The canonical claim frame bytes (fixed key order, no whitespace). */
export function encodeClaimFrame(frame: Omit<ClaimFrame, "t">): string {
  return `{"t":"c","sn":${JSON.stringify(frame.sn)},"q":${frame.q},"obj":${JSON.stringify(frame.obj)},` +
    `"fr":${JSON.stringify(frame.fr)},"on":${frame.on},"ask":${frame.ask}}`;
}

export type DrawChannel = RtSourceChannel<"draw">;

export type ClaimRefusal =
  | "disposed" | "stale-channel" | RtAdmissionDenial | DrawDecodeFailure
  | "replay" | "stale-session" | "session-limit" | "claim-rate" | "claim-cap"
  /** Not applied yet: the edge is unreachable, and the frame is held until it recovers. */
  | "held";
export type ClaimReceive =
  | Readonly<{ ok: true; action: ClaimAction; changed: boolean }>
  | Readonly<{ ok: false; reason: ClaimRefusal }>;

/** One live peer entry, as the Studio renders it. */
export type ClaimView = Readonly<{
  peer: string;
  obj: string;
  fr: string;
  kind: "claim" | "ask";
  /** Whole seconds until it lapses unless refreshed (receiver-observed), 1..90. */
  secondsLeft: number;
}>;

/** What the local user currently asserts. */
export type LocalClaimState = Readonly<{
  claim: Readonly<{ obj: string; fr: string }> | null;
  ask: Readonly<{ obj: string; fr: string; secondsLeft: number }> | null;
}>;

/** A peer released a frame this device had a live Ask on, and this device took it. */
export type ClaimNotice = Readonly<{ kind: "passed"; from: string; obj: string; fr: string }>;

type Entry = {
  peer: string;
  obj: string;
  fr: string;
  kind: "claim" | "ask";
  seenMs: number;
  channel: DrawChannel;
};

type ChannelSession = { sn: string | null; highQ: number; retired: string[]; sessions: number };
type PeerBudget = { admission: RtPeerAdmission; claims: RtTokenBucket; muted: boolean };
/** What one receiver last accepted from me for one frame. */
type Told = { obj: string; fr: string; kind: "claim" | "ask"; atMs: number };
type Edge = {
  send: (raw: string) => boolean;
  open: boolean;
  /** Frame key -> what this edge's transport took for it; changed only by a send that succeeded. */
  told: Map<string, Told>;
  /** Mirrors that receiver's claim bucket for me, charged only for frames the transport took. */
  bucket: RtTokenBucket;
  /** The connection left "connected": entries hidden, deliveries held, nothing sent. */
  suspended: boolean;
  held: { raw: string; atMs: number }[];
};
type Wanted = { key: string; obj: string; fr: string; kind: "claim" | "ask" };

function entryKey(peer: string, obj: string, fr: string): string {
  return `${peer}\u0000${obj}\u0000${fr}`;
}

function frameKey(obj: string, fr: string): string {
  return `${obj}\u0000${fr}`;
}

export type DrawClaimsOptions = Readonly<{
  /** This device's full identity on the call server. */
  self: string;
  /** Monotonic milliseconds (performance.now in the app; a fake in tests). */
  now: () => number;
  /** `bytes` random bytes as lowercase hex (crypto.getRandomValues in the app). */
  randomHex: (bytes: number) => string;
  /** Test seam: the exclusive upper bound of `q` before the sender rotates its session. */
  sequenceMax?: number;
}>;

/**
 * One call join's claim state: the receiver book for every peer edge, plus this device's own
 * claim/ask and the frames it sends. Create one on join and `dispose()` it on leave; a callback
 * retained from an earlier call holds the disposed object, which accepts nothing.
 */
export class DrawClaimsSession {
  readonly self: string;
  /**
   * Bumped on every change a view could show, and ONLY then: refused frames, a release that removed
   * nothing, an `autoClaim` of the frame already claimed and a quiet tick leave it alone, so the app
   * rebuilds its snapshot only when this moves (or while a countdown is running).
   */
  rev = 0;
  /** Called (synchronously) when a Pass hands this device a frame it asked for. */
  onnotice: ((notice: ClaimNotice) => void) | null = null;
  /** Called once when a peer's draw channel auto-mutes for the call (a diagnostic, no content). */
  onmuted: ((peer: string) => void) | null = null;

  private readonly now: () => number;
  private readonly randomHex: (bytes: number) => string;
  private readonly sequenceMax: number;
  private readonly registry = new RtSourceChannelRegistry("draw");
  private readonly edges = new Map<DrawChannel, Edge>();
  private readonly sessions = new WeakMap<DrawChannel, ChannelSession>();
  private readonly budgets = new Map<string, PeerBudget>();
  private readonly entries = new Map<string, Entry>();
  private disposed = false;

  private sn: string;
  private q = 0;
  private rotations = 0;
  private localClaim: { obj: string; fr: string } | null = null;
  private localAsk: { obj: string; fr: string; askedMs: number } | null = null;
  private passed: { obj: string; fr: string } | null = null;
  // A flush can re-enter itself only through a synchronous transport (tests); the outer one then
  // runs again instead, so a record is never written from a decision a nested flush overtook.
  private flushing = false;
  private reflush = false;

  constructor(options: DrawClaimsOptions) {
    this.self = options.self;
    this.now = options.now;
    this.randomHex = options.randomHex;
    this.sequenceMax = options.sequenceMax ?? DRAW_SEQUENCE_MAX;
    this.sn = this.freshSession();
  }

  // --- Edges (one per authenticated peer connection generation) ------------------------------

  /**
   * Mint the capability for a new `draw` channel to authenticated `peer`, synchronously, before
   * any of its callbacks can run. Replacing a peer's channel drops every entry the old one made.
   */
  openEdge(peer: string, send: (raw: string) => boolean): DrawChannel | null {
    if (this.disposed || !peer || peer === this.self) return null;
    const previous = this.registry.current(peer);
    if (previous) this.forgetChannel(previous);
    const channel = this.registry.open(peer);
    this.edges.set(channel, {
      send,
      open: false,
      told: new Map(),
      // Each receiver charges my claims and asks against its own bucket, so pacing is per edge.
      bucket: new RtTokenBucket(CLAIM_FRAME_BUCKET_BURST, CLAIM_FRAME_BUCKET_RATE),
      suspended: false,
      held: [],
    });
    this.sessions.set(channel, { sn: null, highQ: -1, retired: [], sessions: 0 });
    return channel;
  }

  /**
   * The channel opened, or its connection is "connected" again. A recovering edge shows the
   * entries it hid that are still live and applies what it held, in order. Then this edge is told
   * whatever its record says it is missing (on a new edge: my claim and my ask; after a gap: only
   * what changed, since the receiver kept or restored the rest).
   */
  edgeReady(channel: DrawChannel): void {
    const edge = this.edges.get(channel);
    if (this.disposed || !edge || !this.registry.isCurrent(channel)) return;
    edge.open = true;
    if (edge.suspended) this.resume(channel, edge);
    this.flush();
  }

  /** The channel closed or was replaced: its capability is retired and its entries dropped. */
  closeEdge(channel: DrawChannel): void {
    if (this.disposed) return;
    this.forgetChannel(channel);
  }

  /**
   * The peer connection left "connected" (disconnected, failed). Until `edgeReady` says it is back,
   * that peer is absent: its entries leave every view at once (they keep expiring underneath), a
   * delivery that still arrives is held rather than applied, and nothing is sent to it. A link that
   * never recovers closes its channel, which drops the entries and the held frames.
   */
  peerUnreachable(peer: string): void {
    if (this.disposed) return;
    const channel = this.registry.current(peer);
    const edge = channel ? this.edges.get(channel) : undefined;
    if (!channel || !edge || edge.suspended) return;
    edge.suspended = true;
    if ([...this.entries.values()].some((e) => e.channel === channel)) this.rev += 1;
  }

  /** The peer left the call, said bye, or was removed from the server. */
  removePeer(peer: string): void {
    if (this.disposed) return;
    const current = this.registry.current(peer);
    if (current) this.forgetChannel(current);
    this.dropEntries((e) => e.peer === peer);
  }

  dispose(): void {
    this.disposed = true;
    this.registry.clear();
    this.edges.clear();
    this.entries.clear();
    this.budgets.clear();
    this.localClaim = null;
    this.localAsk = null;
    this.passed = null;
    this.rev += 1;
  }

  isDisposed(): boolean {
    return this.disposed;
  }

  // --- Receive ---------------------------------------------------------------------------------

  /**
   * One delivery on a `draw` channel. Every delivery is charged against the sender identity's
   * call-epoch budget before any content-dependent work; a rejected frame changes nothing.
   */
  receive(channel: DrawChannel, raw: unknown): ClaimReceive {
    if (this.disposed) return { ok: false, reason: "disposed" };
    if (!this.registry.isCurrent(channel)) return { ok: false, reason: "stale-channel" };
    const peer = channel.source;
    const now = this.now();
    const budget = this.budgetFor(peer);
    const admitted = budget.admission.admitFrame(raw, now);
    const edge = this.edges.get(channel)!;
    if (!admitted.ok) {
      if (admitted.reason === "abuse-muted" && !budget.muted) {
        // A flood that auto-mutes the channel also retracts what that peer was asserting.
        budget.muted = true;
        edge.held = [];
        this.dropEntries((e) => e.peer === peer);
        this.onmuted?.(peer);
      }
      return { ok: false, reason: admitted.reason };
    }
    if (edge.suspended) {
      // Already charged. A malformed frame is refused now, as it would be later; a well-formed one
      // is applied (session and sequence checked then), in order, only if the connection recovers.
      const decoded = decodeDrawFrame(admitted.raw);
      if (!decoded.ok) return decoded;
      edge.held.push({ raw: admitted.raw, atMs: now });
      if (edge.held.length > DRAW_HOLD_MAX) edge.held.shift();
      return { ok: false, reason: "held" };
    }
    return this.apply(channel, admitted.raw, now, now);
  }

  /** Decode, validate and commit one admitted frame received at `seenMs`. */
  private apply(channel: DrawChannel, raw: string, now: number, seenMs: number): ClaimReceive {
    const peer = channel.source;
    const budget = this.budgetFor(peer);
    if (budget.muted) return { ok: false, reason: "abuse-muted" };
    const decoded = decodeDrawFrame(raw);
    if (!decoded.ok) return decoded;
    const { frame, action } = decoded;

    // Validate everything before committing anything.
    const session = this.sessions.get(channel)!;
    let sessionChange = false;
    if (session.sn === frame.sn) {
      if (frame.q <= session.highQ) return { ok: false, reason: "replay" };
    } else if (session.retired.includes(frame.sn)) {
      return { ok: false, reason: "stale-session" };
    } else if (session.sessions >= DRAW_MAX_SESSIONS_PER_CHANNEL) {
      return { ok: false, reason: "session-limit" };
    } else {
      sessionChange = true;
    }
    // Expired entries are already gone in every view, but the snapshot the app last rendered may
    // still show them: an expiry removed here is a visible change even if this frame is then refused
    // (otherwise a claim expiring between ticks could stay on screen until something else changed).
    if (this.prune(now)) this.rev += 1;
    const key = entryKey(peer, frame.obj, frame.fr);
    const before = this.entries.get(key);
    // A session change will drop this channel's entries, so the cap is measured after that.
    const retained = sessionChange ? (before?.channel === channel ? undefined : before) : before;
    if (action !== "release") {
      const held = [...this.entries.values()].filter((e) => e.peer === peer && !(sessionChange && e.channel === channel)).length;
      if (!retained && held >= CLAIM_MAX_LIVE_PER_SENDER) return { ok: false, reason: "claim-cap" };
      if (!budget.claims.charge(now)) return { ok: false, reason: "claim-rate" };
    }

    // Commit: session, sequence, then the entry.
    let changed = false;
    if (sessionChange) {
      if (session.sn !== null) {
        session.retired.push(session.sn);
        changed = this.dropEntries((e) => e.channel === channel, false) || changed;
      }
      session.sn = frame.sn;
      session.sessions += 1;
    }
    session.highQ = frame.q;

    if (action === "release") {
      const current = this.entries.get(key);
      const wasLiveClaim = current?.kind === "claim";
      if (current) {
        this.entries.delete(key);
        changed = true;
      }
      if (wasLiveClaim) this.takePassIfAsked(peer, frame.obj, frame.fr);
    } else {
      const current = this.entries.get(key);
      changed = changed || !current || current.kind !== action;
      this.entries.set(key, { peer, obj: frame.obj, fr: frame.fr, kind: action, seenMs, channel });
    }
    // A claim or ask refresh moves its countdown, so it is visible even when nothing else changed;
    // a release that removed nothing is not.
    if (changed || action !== "release") this.rev += 1;
    return { ok: true, action, changed };
  }

  // --- Local claim, ask and pass ---------------------------------------------------------------

  /** Explicitly claim a frame (the "claim" button). Clears a Pass suppression on that frame. */
  claim(obj: string, fr: string): boolean {
    if (this.disposed || !isClaimElementId(obj) || !isClaimElementId(fr)) return false;
    if (this.passed?.obj === obj && this.passed.fr === fr) this.passed = null;
    return this.setClaim(obj, fr);
  }

  /**
   * The user started painting this frame. Claims it unless this device just passed it away, in
   * which case it stays unclaimed until the user selects another frame or claims explicitly.
   */
  autoClaim(obj: string, fr: string): boolean {
    if (this.disposed || !isClaimElementId(obj) || !isClaimElementId(fr)) return false;
    if (this.passed?.obj === obj && this.passed.fr === fr) return false;
    if (this.localClaim?.obj === obj && this.localClaim.fr === fr) return false;
    return this.setClaim(obj, fr);
  }

  /**
   * Stop claiming (selection moved, object closed, Studio left, server changed). Every edge that
   * took the claim is sent a Release, retried until its transport takes it or its receiver has
   * expired the claim anyway; an edge that never took the claim is sent nothing.
   */
  release(): void {
    if (this.disposed) return;
    this.passed = null;
    if (!this.localClaim) return;
    this.localClaim = null;
    this.flush();
    this.rev += 1;
  }

  /**
   * Ask whoever is drawing this frame to Pass it. Replaces any earlier ask. Each edge is sent the
   * Ask once its transport takes it; an Ask is never refreshed.
   */
  ask(obj: string, fr: string): boolean {
    if (this.disposed || !isClaimElementId(obj) || !isClaimElementId(fr)) return false;
    if (this.localClaim?.obj === obj && this.localClaim.fr === fr) return false;
    this.localAsk = { obj, fr, askedMs: this.now() };
    this.flush();
    this.rev += 1;
    return true;
  }

  /** Withdraw my Ask: a Release to every edge that took it, nothing to an edge that never did. */
  cancelAsk(): void {
    if (this.disposed || !this.localAsk) return;
    this.localAsk = null;
    this.flush();
    this.rev += 1;
  }

  /**
   * The Studio's selection moved to `(obj, fr)` (or to nothing). A claim or ask for any other frame
   * is withdrawn, and a Pass suppression for another frame lapses: having passed F, going to G and
   * back to F and painting claims F again. Selecting the same frame changes nothing.
   */
  selectionChanged(obj: string, fr: string): void {
    if (this.disposed) return;
    if (this.passed && (this.passed.obj !== obj || this.passed.fr !== fr)) this.passed = null;
    if (this.localClaim && (this.localClaim.obj !== obj || this.localClaim.fr !== fr)) {
      const passed = this.passed;
      this.release();
      this.passed = passed;
    }
    if (this.localAsk && (this.localAsk.obj !== obj || this.localAsk.fr !== fr)) this.cancelAsk();
  }

  /** True when nothing is counting down, so a periodic view rebuild would show nothing new. */
  isQuiet(): boolean {
    return this.disposed || (this.entries.size === 0 && this.localAsk === null);
  }

  /** Pass my claim on this frame: release it and do not auto-claim it again right away. */
  pass(obj: string, fr: string): boolean {
    if (this.disposed || this.localClaim?.obj !== obj || this.localClaim.fr !== fr) return false;
    this.release();
    this.passed = { obj, fr };
    return true;
  }

  /**
   * Once a second: expire entries, lapse my own Ask after 90 s, then flush: refresh my claim on
   * each edge 30 s after that edge last took it, and retry whatever an edge's transport refused or
   * the local pacing deferred.
   */
  tick(): void {
    if (this.disposed) return;
    const now = this.now();
    if (this.prune(now)) this.rev += 1;
    if (this.localAsk && !(now - this.localAsk.askedMs < CLAIM_TTL_MS)) {
      this.localAsk = null;
      this.rev += 1;
    }
    this.flush();
  }

  // --- Views -----------------------------------------------------------------------------------

  /** Live peer entries for one object (or all), sorted by frame, then identity: order-independent. */
  peerEntries(obj?: string): ClaimView[] {
    const now = this.now();
    const out: ClaimView[] = [];
    for (const e of this.entries.values()) {
      if (obj !== undefined && e.obj !== obj) continue;
      const age = now - e.seenMs;
      if (!(age < CLAIM_TTL_MS) || this.hidden(e)) continue;
      out.push({ peer: e.peer, obj: e.obj, fr: e.fr, kind: e.kind, secondsLeft: Math.max(1, Math.ceil((CLAIM_TTL_MS - age) / 1_000)) });
    }
    return out.sort((a, b) => (a.obj < b.obj ? -1 : a.obj > b.obj ? 1 : a.fr < b.fr ? -1 : a.fr > b.fr ? 1 : a.peer < b.peer ? -1 : a.peer > b.peer ? 1 : 0));
  }

  /** Live Asks from peers on the frame I am claiming: each wants a sticky notice with Pass. */
  asksOnMine(): ClaimView[] {
    const claim = this.localClaim;
    if (!claim) return [];
    return this.peerEntries(claim.obj).filter((e) => e.kind === "ask" && e.fr === claim.fr);
  }

  local(): LocalClaimState {
    const now = this.now();
    const ask = this.localAsk && now - this.localAsk.askedMs < CLAIM_TTL_MS
      ? { obj: this.localAsk.obj, fr: this.localAsk.fr, secondsLeft: Math.max(1, Math.ceil((CLAIM_TTL_MS - (now - this.localAsk.askedMs)) / 1_000)) }
      : null;
    return { claim: this.localClaim ? { obj: this.localClaim.obj, fr: this.localClaim.fr } : null, ask };
  }

  /** The sender session this device currently numbers its frames under (diagnostics/tests). */
  sessionNonce(): string {
    return this.sn;
  }

  // --- Internals -------------------------------------------------------------------------------

  private setClaim(obj: string, fr: string): boolean {
    // A claim on the frame I was asking for replaces that ask at every receiver.
    if (this.localAsk?.obj === obj && this.localAsk.fr === fr) this.localAsk = null;
    this.localClaim = { obj, fr };
    this.flush();
    this.rev += 1;
    return true;
  }

  /** What this device asserts now: at most one claim and one ask, on different frames. */
  private wanted(now: number): Map<string, Wanted> {
    const out = new Map<string, Wanted>();
    const claim = this.localClaim;
    if (claim) out.set(frameKey(claim.obj, claim.fr), { key: frameKey(claim.obj, claim.fr), obj: claim.obj, fr: claim.fr, kind: "claim" });
    const ask = this.localAsk;
    if (ask && now - ask.askedMs < CLAIM_TTL_MS) {
      out.set(frameKey(ask.obj, ask.fr), { key: frameKey(ask.obj, ask.fr), obj: ask.obj, fr: ask.fr, kind: "ask" });
    }
    return out;
  }

  /**
   * Bring every open, reachable edge's receiver in line with what I assert. Idempotent: an edge
   * whose record already matches is sent nothing. A sequence rotation part-way through invalidates
   * the records it has already written, so the pass runs again (bounded; the next tick finishes).
   */
  private flush(): void {
    if (this.disposed) return;
    if (this.flushing) {
      this.reflush = true;
      return;
    }
    this.flushing = true;
    try {
      for (let pass = 0; pass < 4; pass += 1) {
        this.reflush = false;
        const rotations = this.rotations;
        for (const [channel, edge] of this.edges) {
          if (!edge.open || edge.suspended || !this.registry.isCurrent(channel)) continue;
          this.reconcileEdge(edge);
        }
        if (!this.reflush && this.rotations === rotations) break;
      }
    } finally {
      this.flushing = false;
    }
  }

  private reconcileEdge(edge: Edge): void {
    const now = this.now();
    const want = this.wanted(now);
    // Whatever that receiver last heard about 90 s ago it has expired: nothing to withdraw there.
    for (const [key, told] of edge.told) if (!(now - told.atMs < CLAIM_TTL_MS)) edge.told.delete(key);
    // Withdraw first, so the receiver's per-sender cap never sees more of mine than I assert.
    for (const [key, told] of edge.told) {
      if (want.has(key)) continue;
      if (this.sendTo(edge, told.obj, told.fr, 0, 0)) edge.told.delete(key);
    }
    for (const w of want.values()) {
      const told = edge.told.get(w.key);
      // A claim is refreshed 30 s after this edge last took it; an Ask is sent once per edge.
      const due = !told || told.kind !== w.kind || (w.kind === "claim" && now - told.atMs >= CLAIM_RESEND_MS);
      if (!due || !edge.bucket.charge(now)) continue;
      if (this.sendTo(edge, w.obj, w.fr, w.kind === "claim" ? 1 : 0, w.kind === "ask" ? 1 : 0)) {
        edge.told.set(w.key, { obj: w.obj, fr: w.fr, kind: w.kind, atMs: now });
      } else {
        edge.bucket.refund();
      }
    }
    // Bounded: a record beyond this is a frame the receiver could not be holding anyway.
    while (edge.told.size > DRAW_TOLD_MAX) {
      let oldest: string | null = null;
      let at = Infinity;
      for (const [key, told] of edge.told) if (!want.has(key) && told.atMs < at) { oldest = key; at = told.atMs; }
      if (oldest === null) break;
      edge.told.delete(oldest);
    }
  }

  /**
   * Number one frame and offer it to one edge's transport. True only if the transport took it:
   * `send` returned true and did not throw (a full buffer throws OperationError; a closing channel
   * refuses). A frame the transport did not take changes no record, so it is retried.
   */
  private sendTo(edge: Edge, obj: string, fr: string, on: 0 | 1, ask: 0 | 1): boolean {
    if (this.q >= this.sequenceMax) this.rotateSession();
    const raw = encodeClaimFrame({ sn: this.sn, q: this.q, obj, fr, on, ask });
    this.q += 1;
    try {
      return edge.send(raw) === true;
    } catch {
      return false;
    }
  }

  /**
   * Exhausting the uint32 sequence space starts a new sender session. A receiver treats the new
   * `sn` as a session change and drops everything of mine, so each edge's record of what I still
   * assert is cleared (the flush re-sends it under the new session); records of what I no longer
   * assert stay, and their Releases go out under the new session too.
   */
  private rotateSession(): void {
    this.sn = this.freshSession();
    this.q = 0;
    this.rotations += 1;
    const want = this.wanted(this.now());
    for (const edge of this.edges.values()) for (const key of want.keys()) edge.told.delete(key);
  }

  /** The edge is reachable again: show its live entries and apply what it held, in order. */
  private resume(channel: DrawChannel, edge: Edge): void {
    edge.suspended = false;
    const held = edge.held;
    edge.held = [];
    if ([...this.entries.values()].some((e) => e.channel === channel)) this.rev += 1;
    const now = this.now();
    for (const h of held) {
      if (!this.registry.isCurrent(channel) || this.disposed) return;
      this.apply(channel, h.raw, now, h.atMs);
    }
    if (this.prune(now)) this.rev += 1;
  }

  /** An entry from an edge that is currently unreachable: kept, but in no view. */
  private hidden(entry: Entry): boolean {
    return this.edges.get(entry.channel)?.suspended === true;
  }

  private freshSession(): string {
    for (;;) {
      const sn = this.randomHex(DRAW_SESSION_HEX / 2);
      if (validSession(sn) && sn !== this.sn) return sn;
    }
  }

  /**
   * A live peer claim on a frame I am asking for was explicitly released (`on:0,ask:0` from the
   * holder). If nobody else still claims it, take it. Expiry, a dropped channel or a session
   * change never land here: only an explicit release from a holder is a Pass.
   */
  private takePassIfAsked(from: string, obj: string, fr: string): void {
    const ask = this.localAsk;
    if (!ask || ask.obj !== obj || ask.fr !== fr) return;
    if (!(this.now() - ask.askedMs < CLAIM_TTL_MS)) return;
    // Only claims the user can see count: an unreachable peer is absent until it recovers.
    const othersClaiming = [...this.entries.values()].some((e) => e.kind === "claim" && e.obj === obj && e.fr === fr && !this.hidden(e));
    if (othersClaiming) return;
    this.localAsk = null;
    this.setClaim(obj, fr);
    this.onnotice?.({ kind: "passed", from, obj, fr });
  }

  private forgetChannel(channel: DrawChannel): void {
    if (this.registry.isCurrent(channel)) this.registry.close(channel.source);
    this.edges.delete(channel);
    this.dropEntries((e) => e.channel === channel);
  }

  private dropEntries(match: (entry: Entry) => boolean, bump = true): boolean {
    let dropped = false;
    for (const [key, entry] of this.entries) {
      if (match(entry)) {
        this.entries.delete(key);
        dropped = true;
      }
    }
    if (dropped && bump) this.rev += 1;
    return dropped;
  }

  private prune(now: number): boolean {
    return this.dropEntries((e) => !(now - e.seenMs < CLAIM_TTL_MS), false);
  }

  private budgetFor(peer: string): PeerBudget {
    let budget = this.budgets.get(peer);
    if (!budget) {
      // Per identity for the whole call: a reconnect cannot refill a burst or clear an auto-mute.
      budget = {
        admission: new RtPeerAdmission(DRAW_FRAME_LIMITS),
        claims: new RtTokenBucket(CLAIM_FRAME_BUCKET_BURST, CLAIM_FRAME_BUCKET_RATE),
        muted: false,
      };
      this.budgets.set(peer, budget);
    }
    return budget;
  }
}

// --- The Studio's view of one call's claims ----------------------------------------------------

/**
 * Plain data the Studio renders. Immutable and rebuilt by the app after every change and on each
 * 1 s tick (so countdowns move); the Studio never holds the session itself, so nothing it does
 * can reach the receiver book except through `StudioClaimActions`.
 */
export type StudioClaimSnapshot = Readonly<{
  /** This device's identity on the call server (the same `me` the Studio names authors by). */
  self: string;
  entries: readonly ClaimView[];
  local: LocalClaimState;
  asksOnMine: readonly ClaimView[];
}>;

/** What the Studio may ask the call's claim session to do. Every call is advisory. */
export type StudioClaimActions = Readonly<{
  /** The selected frame changed (obj "" / fr "" for none); see `DrawClaimsSession.selectionChanged`. */
  select(obj: string, fr: string): void;
  claim(obj: string, fr: string): void;
  autoClaim(obj: string, fr: string): void;
  release(): void;
  ask(obj: string, fr: string): void;
  cancelAsk(): void;
  pass(obj: string, fr: string): void;
}>;

export function snapshotClaims(session: DrawClaimsSession): StudioClaimSnapshot | null {
  if (session.isDisposed()) return null;
  return Object.freeze({
    self: session.self,
    entries: Object.freeze(session.peerEntries()),
    local: session.local(),
    asksOnMine: Object.freeze(session.asksOnMine()),
  });
}

/** The live claimants and askers of one frame, in identity order, for the frame card. */
export function frameClaimState(snapshot: StudioClaimSnapshot | null, obj: string, fr: string): Readonly<{
  claimants: readonly ClaimView[];
  askers: readonly ClaimView[];
  mine: boolean;
  myAsk: number | null;
  contested: boolean;
}> {
  const here = snapshot?.entries.filter((e) => e.obj === obj && e.fr === fr) ?? [];
  const claimants = here.filter((e) => e.kind === "claim");
  const mine = snapshot?.local.claim?.obj === obj && snapshot.local.claim.fr === fr;
  const ask = snapshot?.local.ask;
  return {
    claimants,
    askers: here.filter((e) => e.kind === "ask"),
    mine,
    myAsk: ask && ask.obj === obj && ask.fr === fr ? ask.secondsLeft : null,
    // Two or more claimants, counting this device, is a collision. It is shown, never resolved.
    contested: claimants.length + (mine ? 1 : 0) > 1,
  };
}
