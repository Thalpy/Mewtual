/**
 * Pure logic behind the debug console (`docs/design-debug-console.md`).
 *
 * The console exists because the app used to fail silently. A voice call died while the roster
 * still showed the peer online; a node sat isolated for an hour dialling two IPv6 addresses an
 * IPv4-only machine can never reach, and not one dial failure was surfaced anywhere. Everything
 * here is the part of fixing that which can be tested without a window: which events belong to
 * which section, how a line renders, how redaction keeps a screenshot correlatable, and how a copy
 * bundle is assembled. The markup in App.svelte does the rest.
 *
 * Rendering and copying share these functions deliberately. If copy had its own formatter, the two
 * could disagree, and a pasted bug report that does not match the screenshot it came with is worse
 * than no bug report.
 */

/** One field of an event, rendered at the session's capture mode. */
export type LogField = {
  name: string;
  value: string;
  /** Whether a higher capture mode would show more of this value. */
  sensitive: boolean;
};

/**
 * One diagnostic event, as `get_console_log` returns it.
 *
 * The canonical event, carried whole. This used to be a flattened `tracing` line: the section, the
 * phase, the span parentage and the references were dropped on the way here, twelve of the trace's
 * sixteen characters with them, and every value arrived rendered at a hard-coded Enhanced whatever
 * the user had chosen. The console then guessed the sections back from target names and by
 * searching the text for the word "voice", which is why a migrated voice event stopped appearing in
 * the voice section the moment its prose became a code. Found by adversarial review (P3-005).
 */
export type LogEvent = {
  seq: number;
  at_ms: number;
  /** Milliseconds since the process started. Never goes backwards, unlike `at_ms`. */
  monotonic_ms: number;
  /** The canonical section, one of twenty-two: `join`, `transport`, `channels`. */
  section: string;
  /** The console section it belongs under, one of six. Decided natively, never guessed here. */
  view: string;
  /** `ERROR` | `WARN` | `INFO` | `DEBUG` | `TRACE`. */
  level: string;
  /** A stable `AREA.COMPONENT.OUTCOME` code, or `LOG.TRACING.EVENT` for an un-migrated call site. */
  code: string;
  phase: string;
  operation: string;
  /** Sixteen hex characters, or empty when the event belongs to no operation. */
  trace: string;
  span: string;
  parent_span: string;
  /** Named subjects: `server`, `channel`, `peer`, `document`, `transfer`. */
  refs: [string, string][];
  duration_ms: number | null;
  attempt: number | null;
  /** The emitting module. Kept for locating the code that said this, no longer used to group. */
  target: string;
  fields: LogField[];
  /** The capture mode this event was rendered at. */
  capture: string;
};

/** The counters behind the header roll-up and the rail badges. */
export type LogStats = {
  errors: number;
  warnings: number;
  dropped: number;
  /** Events the capture config excluded. A silent section is not the same as a quiet one. */
  filtered: number;
  latest_seq: number;
  capacity: number;
  capture: string;
  session_id: string;
};

/**
 * The code every un-migrated `tracing` event carries.
 *
 * Must match `BRIDGED_CODE` in `crates/catcoms-log/src/lib.rs`, which has a test pinning the
 * literal for that reason. Counting these is how the migration's progress is measured, and the
 * console renders their prose as the headline where a migrated event shows its code.
 */
export const BRIDGED_CODE = "LOG.TRACING.EVENT";

/**
 * The webview's own tracing target. Everything the frontend logs arrives through `log_ui` under
 * this name.
 */
export const UI_TARGET = "catcoms_ui";

/**
 * Whether an event came from the webview rather than from Rust.
 *
 * Reads the section the event states rather than sniffing its target. The two agree for a bridged
 * console line, and only the section is right for a structured one: a webview event about voice
 * signalling belongs in the voice section, not lumped into "frontend" because of which process
 * happened to emit it.
 */
export function isFrontend(e: LogEvent): boolean {
  return e.view === "frontend";
}

/** Whether an event belongs to one of the console's six sections. */
export function inView(e: LogEvent, view: DbgSection): boolean {
  return e.view === view;
}

/** Levels in severity order, loudest first. The console's chips are rendered in this order. */
export const LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;
export type Level = (typeof LEVELS)[number];

/**
 * Levels shown before the user touches anything.
 *
 * DEBUG is off by default even though the ring captures it: `catcoms_net` at debug narrates every
 * connection and address the node sees, which is the bulk of the volume. It is one click away, and
 * it is the click that answers "what is it actually dialling".
 */
export const DEFAULT_LEVELS: Level[] = ["ERROR", "WARN", "INFO"];

/** Local wall clock as `HH:MM:SS.mmm`, the format every feed line starts with. */
export function formatTime(atMs: number): string {
  const d = new Date(atMs);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

/**
 * A duration as the tables phrase it: "42s", "3m 20s", "now".
 *
 * Used for a re-dial countdown and for ages. Sub-second reads as "now" rather than "0s", because a
 * countdown that sits on zero looks stuck when it is actually about to fire.
 */
export function formatDuration(ms: number): string {
  if (ms <= 999) return "now";
  const total = Math.round(ms / 1000);
  if (total < 60) return `${total}s`;
  const m = Math.floor(total / 60);
  const s = total % 60;
  return s === 0 ? `${m}m` : `${m}m ${s}s`;
}

// --- redaction ---------------------------------------------------------------------------

/**
 * A stable per-session naming of the values redaction hides.
 *
 * Masking has to preserve correlation or it destroys the evidence. "it keeps dialling the same two
 * addresses" is the whole diagnosis of the hour-long isolation, and a screenshot where both
 * addresses read `[redacted]` no longer says it. So each distinct value gets its own alias, kept
 * for the life of the console, and the same address is the same `[ip 2]` every time it appears.
 */
export type Aliases = Map<string, string>;

export function makeAliases(): Aliases {
  return new Map();
}

/** The alias for one value, minting a new one on first sight. */
export function alias(aliases: Aliases, kind: string, value: string): string {
  const key = `${kind}:${value}`;
  const existing = aliases.get(key);
  if (existing) return existing;
  // Numbered per kind, so `[ip 1]` and `[peer 1]` can coexist without reading as the same thing.
  let n = 1;
  for (const k of aliases.keys()) if (k.startsWith(`${kind}:`)) n += 1;
  const minted = `[${kind} ${n}]`;
  aliases.set(key, minted);
  return minted;
}

/** IPv4, and IPv6 in the forms a multiaddr or a socket error actually prints. */
const IPV4 = /\b\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}\b/g;
const IPV6 = /\b(?:[0-9a-f]{1,4}:){2,7}[0-9a-f]{1,4}\b/gi;
/**
 * A libp2p peer id (base58btc, `12D3Koo…`) or a hex fingerprint of 8 or more digits.
 *
 * The `12D3Koo` prefix is matched with a permissive tail on purpose. A real id carries 45 more
 * base58 characters, and an earlier version required at least 20 of them; anything shorter, a
 * truncated id in a log line or a short form in a test fixture, then rendered unmasked while the
 * toggle claimed the screen was safe to share. The prefix alone is the giveaway, so the rule
 * masks on it and over-masks rather than leaks.
 */
const PEER_B58 = /\b12D3Koo[1-9A-HJ-NP-Za-km-z]*/g;
const PEER_HEX = /\b[0-9a-f]{8,}\b/gi;

/**
 * Replace every identifying value in one line with its alias.
 *
 * Order matters: IPv6 before hex, because an IPv6 literal is made of hex groups and the hex rule
 * would otherwise chew it into pieces that no longer correlate with anything.
 *
 * This is deliberately a text substitution over the rendered line rather than a CSS overlay. The
 * console's copy buttons copy what is on screen, so the two must be the same string; an overlay
 * would show a masked screen and copy an unmasked bundle, which is the exact failure this feature
 * is supposed to prevent.
 */
export function redactText(text: string, aliases: Aliases): string {
  return text
    .replace(IPV6, (m) => alias(aliases, "ip", m.toLowerCase()))
    .replace(IPV4, (m) => alias(aliases, "ip", m))
    .replace(PEER_B58, (m) => alias(aliases, "peer", m))
    .replace(PEER_HEX, (m) => alias(aliases, "peer", m.toLowerCase()));
}

/** `redactText` when redaction is on, the original text when it is off. */
export function maybeRedact(text: string, aliases: Aliases, on: boolean): string {
  return on ? redactText(text, aliases) : text;
}

// --- feed lines --------------------------------------------------------------------------

/** The short form of a trace, which is how one is referred to in prose. */
export function shortTrace(trace: string): string {
  return trace.slice(0, 4);
}

/**
 * One event's body: the headline, then everything that qualifies it.
 *
 * Mirrors `event_line` in `crates/catcoms-diagnostics/src/render.rs`, which is what the export
 * bundle will be written from. The two must stay in step, because a saved report that disagrees
 * with the screenshot it arrived with makes the reader work out which one is lying before they can
 * start on the bug.
 *
 * The headline is the event's **code**, which is the point of the migration: `SYNC.CATCHUP.STALLED`
 * is greppable, countable and stable across rewording in a way that a sentence never is. An
 * un-migrated `tracing` event has no code of its own, so its prose stands in; the substitution is
 * keyed on the bridge's code rather than on "does it have a message field", so a structured event
 * that happens to carry one still shows what it is.
 */
export function eventText(e: LogEvent): string {
  const bits: string[] = [];
  const bridged = e.code === BRIDGED_CODE;
  const lead = bridged && e.fields[0]?.name === "message" ? e.fields[0] : null;
  bits.push(lead ? lead.value : e.code);
  // Absent for a bare observation, and its absence is information: an operation that started and
  // never finished looks exactly like one that was never attempted without it.
  if (e.phase && e.phase !== "observation") bits.push(`phase=${e.phase}`);
  if (e.duration_ms != null) bits.push(`duration=${e.duration_ms}ms`);
  if (e.attempt != null) bits.push(`attempt=${e.attempt}`);
  for (const [name, value] of e.refs) bits.push(`${name}=${value}`);
  for (const f of e.fields) {
    if (f === lead) continue;
    bits.push(`${f.name}=${f.value}`);
  }
  return bits.join(" ");
}

/**
 * One event split into the spans a feed row renders.
 *
 * Rendering used to slice the joined line back apart by counting characters, which is exactly the
 * sort of arithmetic that silently drifts: the "needs attention" list ended up printing the level
 * twice. The parts are the source of truth, and `eventLine` joins them, so the two cannot disagree.
 */
export function eventParts(
  e: LogEvent,
  aliases: Aliases,
  redact: boolean,
): { ts: string; level: string; section: string; trace: string; target: string; text: string } {
  return {
    ts: formatTime(e.at_ms),
    level: e.level,
    // The canonical section, which is a column that actually varies inside a console section: the
    // Network view holds transport, reachability, discovery and join events, and which one a line
    // came from is most of what tells them apart.
    section: e.section,
    trace: e.trace ? shortTrace(e.trace) : "",
    // A frontend row would otherwise read `catcoms_ui` on every single line.
    target: isFrontend(e) ? "" : e.target,
    text: maybeRedact(eventText(e), aliases, redact),
  };
}

/** The full line as copy writes it and as the filter matches against. */
export function eventLine(e: LogEvent, aliases: Aliases, redact: boolean): string {
  const p = eventParts(e, aliases, redact);
  const trace = p.trace ? ` trace=${p.trace}` : "";
  const target = p.target ? ` ${p.target}` : "";
  return `${p.ts} ${p.level.padEnd(5)} ${p.section.padEnd(12)}${trace}${target} ${p.text}`;
}

/** How a feed narrows what it shows. Display only: capture is never filtered here. */
export type FeedFilter = {
  levels: readonly string[];
  /** Substring match against the tracing target. Empty matches everything. */
  target?: string;
  /** Exact match against the canonical section. Empty matches everything. */
  section?: string;
  /**
   * Prefix match against the trace, in either the short or the full form.
   *
   * The filter the review asks for by name, and the one that turns the console from a log viewer
   * into something that can answer "what happened when I pressed send": paste the four characters
   * off an error banner and what is left is that operation and nothing else.
   */
  trace?: string;
  /** Case-insensitive substring match against the rendered line. Empty matches everything. */
  text?: string;
};

/**
 * Apply a feed's filter.
 *
 * Matching runs against the **rendered** line, redaction included, so what you type matches what
 * you can see. Filtering while redacted and searching for `[ip 2]` is a legitimate thing to do.
 */
export function filterEvents(
  events: readonly LogEvent[],
  f: FeedFilter,
  aliases: Aliases,
  redact: boolean,
): LogEvent[] {
  const needle = (f.text ?? "").trim().toLowerCase();
  const target = (f.target ?? "").trim().toLowerCase();
  const section = (f.section ?? "").trim().toLowerCase();
  const trace = (f.trace ?? "").trim().toLowerCase();
  return events.filter((e) => {
    if (!f.levels.includes(e.level)) return false;
    if (target && !e.target.toLowerCase().includes(target)) return false;
    if (section && e.section !== section) return false;
    if (trace && !e.trace.toLowerCase().startsWith(trace)) return false;
    if (needle && !eventLine(e, aliases, redact).toLowerCase().includes(needle)) return false;
    return true;
  });
}

/**
 * Every stage of one operation, oldest first.
 *
 * The question the whole correlation architecture exists to answer. Given "my message did not
 * arrive", the evidence used to be spread across ten stages with nothing tying them together, so
 * establishing which one failed meant matching wall-clock timestamps by eye across two formats.
 */
export function traceEvents(events: readonly LogEvent[], trace: string): LogEvent[] {
  if (!trace) return [];
  return events.filter((e) => e.trace === trace);
}

// --- the ring ----------------------------------------------------------------------------

/**
 * Append a page of new events, keeping the newest `cap`.
 *
 * The console polls with the highest sequence it holds, so a page should never overlap what is
 * already there; the sequence check is belt and braces against a double-poll, because a feed that
 * shows the same error twice makes a person doubt the feed rather than the error.
 */
export function appendEvents(
  held: readonly LogEvent[],
  incoming: readonly LogEvent[],
  cap: number,
): LogEvent[] {
  const highest = held.length ? held[held.length - 1].seq : 0;
  const fresh = incoming.filter((e) => e.seq > highest);
  const next = held.concat(fresh);
  return next.length > cap ? next.slice(next.length - cap) : next;
}

/** The highest sequence held, which is what the next poll asks from. */
export function latestSeq(held: readonly LogEvent[]): number {
  return held.length ? held[held.length - 1].seq : 0;
}

/**
 * The pinned notice a feed shows when its ring has dropped events.
 *
 * Never a silent truncation: a gap presented as a quiet period is a lie a diagnostic tool cannot
 * afford. Returns an empty string when nothing was dropped.
 */
export function dropNote(dropped: number, kept: number): string {
  if (dropped <= 0) return "";
  const total = dropped + kept;
  return `Ring full: oldest entries dropped. Showing the last ${kept.toLocaleString()} of ${total.toLocaleString()} this session. The debug log file keeps everything.`;
}

/** The `n of m shown` readout, so an over-eager filter can never look like an empty feed. */
export function shownCount(shown: number, total: number): string {
  return shown === total ? `${total.toLocaleString()}` : `${shown.toLocaleString()} of ${total.toLocaleString()} shown`;
}

// --- network view ------------------------------------------------------------------------

/** One member's reachability, as `get_member_routes` returns it. */
export type MemberRoute = {
  fingerprint: string;
  /** Short hex of the transport peer, or empty when no record has been learned yet. */
  peer: string;
  addresses: string[];
  seq: number;
  connected: boolean;
  dial_attempts: number;
  next_dial_in_ms: number;
};

export type RouteState = "connected" | "no-record" | "no-route" | "backing-off" | "reachable";

/**
 * Classify what is happening with one member, which is the column the roster could never show.
 *
 * These four failures used to look identical from outside, and telling them apart is the whole
 * job: a member with no record cannot be dialled or signalled at all; a record carrying no
 * dialable address is a different problem with a different fix; and a peer whose backoff has
 * walked up to the cap is a node quietly giving up, which is what an hour of isolation looks like
 * from the inside.
 */
export function routeState(r: MemberRoute): RouteState {
  if (r.connected) return "connected";
  if (!r.peer) return "no-record";
  if (r.addresses.length === 0) return "no-route";
  if (r.dial_attempts > 0) return "backing-off";
  return "reachable";
}

/** The chip label and semantic colour job for a route state. */
export function routeChip(state: RouteState): { label: string; tone: "ok" | "warn" | "danger" | "faint" } {
  switch (state) {
    case "connected":
      return { label: "CONNECTED", tone: "ok" };
    case "no-record":
      return { label: "NO RECORD", tone: "danger" };
    case "no-route":
      return { label: "NO ROUTE", tone: "danger" };
    case "backing-off":
      return { label: "BACKING OFF", tone: "warn" };
    default:
      return { label: "IDLE", tone: "faint" };
  }
}

/**
 * A one-line explanation of a member's state, in the terms someone can act on.
 *
 * The address-family note is the one that matters most: a host with no IPv6 route dialling an
 * IPv6-only record fails instantly, forever, and the only trace it left in a log was a raw
 * `sendmsg` warning from deep inside the QUIC stack.
 */
export function routeExplanation(r: MemberRoute, hasIpv6: boolean): string {
  switch (routeState(r)) {
    case "connected":
      return "A transport connection to this member is live.";
    case "no-record":
      return "No signed peer record learned yet, so this member cannot be dialled, called or sent a friend request. It arrives over PEX once any member that holds it is reachable.";
    case "no-route":
      return "This member's record carries no dialable address. Nothing can be attempted until it publishes one.";
    case "backing-off": {
      const only6 = r.addresses.length > 0 && r.addresses.every(isIpv6Addr);
      const wait = r.next_dial_in_ms > 0 ? ` Next attempt in ${formatDuration(r.next_dial_in_ms)}.` : "";
      if (only6 && !hasIpv6) {
        return `Every address this member advertises is IPv6, and this device has no IPv6 route, so each attempt fails immediately.${wait}`;
      }
      return `${r.dial_attempts} attempt(s) have failed, so the retry backoff is holding.${wait}`;
    }
    default:
      return "A record is held and no dial has failed; this member is simply not connected right now.";
  }
}

/** Whether a multiaddr's host component is IPv6. */
export function isIpv6Addr(addr: string): boolean {
  return addr.startsWith("/ip6/");
}

/** One conclusion about why members cannot be reached, across a whole server. */
export type RouteFinding = {
  /** A stable code, so this can be counted and searched rather than read. */
  code: string;
  severity: "warn" | "danger";
  /** How many members this accounts for. */
  affected: number;
  /** One sentence a person can act on. Never contains an address. */
  detail: string;
};

/**
 * "1 member" / "3 members", because "member(s)" is the sort of thing that makes a person trust the
 * rest of the sentence a little less.
 */
function members(n: number): string {
  return n === 1 ? "1 member" : `${n} members`;
}

/**
 * Diagnose a server's reachability as a whole, rather than one member at a time.
 *
 * `routeExplanation` answers "what is happening with this member", which is the right question when
 * you already suspect a member. The question nobody could answer was the aggregate one: a node sat
 * isolated for an hour dialling addresses it could never reach, and the evidence for that was a
 * scattering of raw `sendmsg` warnings from inside the QUIC stack. Working it out required reading
 * multiaddrs one at a time and knowing that the host had no IPv6 route.
 *
 * These are conclusions, not observations, and they carry codes so they can be counted across
 * reports instead of re-derived by whoever is reading.
 */
export function routeFindings(routes: readonly MemberRoute[], hasIpv6: boolean): RouteFinding[] {
  const findings: RouteFinding[] = [];
  if (routes.length === 0) return findings;

  const unreachable = routes.filter((r) => routeState(r) !== "connected");
  if (unreachable.length === 0) return findings;

  // The failure that stranded a node for an hour. Every candidate is IPv6 on a host with no IPv6
  // route, so each dial fails instantly and forever, and nothing about a retry will ever help.
  const v6Only = unreachable.filter(
    (r) => r.addresses.length > 0 && r.addresses.every(isIpv6Addr),
  );
  if (!hasIpv6 && v6Only.length > 0) {
    findings.push({
      code: "NET.ROUTE.NO_IPV6_PATH",
      severity: "danger",
      affected: v6Only.length,
      detail:
        `${members(v6Only.length)} ${v6Only.length === 1 ? "advertises" : "advertise"} only IPv6 ` +
        `addresses and this device has no IPv6 route, so every attempt fails immediately. Retrying ` +
        `cannot help: they need an IPv4 or relay route, or this device needs IPv6.`,
    });
  }

  const noRecord = unreachable.filter((r) => routeState(r) === "no-record");
  if (noRecord.length > 0) {
    findings.push({
      code: "NET.ROUTE.NO_RECORD",
      severity: "warn",
      affected: noRecord.length,
      detail:
        `${members(noRecord.length)} ${noRecord.length === 1 ? "has" : "have"} no signed peer ` +
        `record yet, so ${noRecord.length === 1 ? "it" : "they"} cannot be dialled, called or sent ` +
        `a friend request. Records arrive over PEX once any member holding them is reachable.`,
    });
  }

  const noRoute = unreachable.filter((r) => routeState(r) === "no-route");
  if (noRoute.length > 0) {
    findings.push({
      code: "NET.ROUTE.NO_DIALABLE_ADDRESS",
      severity: "warn",
      affected: noRoute.length,
      detail:
        `${members(noRoute.length)} ${noRoute.length === 1 ? "has" : "have"} a record carrying no ` +
        `dialable address, so nothing can be attempted until one is published.`,
    });
  }

  // Reported last because it is the consequence, and the findings above are the cause. A reader
  // scanning from the top meets the explanation before the symptom.
  if (unreachable.length === routes.length) {
    findings.push({
      code: "NET.ROUTE.ISOLATED",
      severity: "danger",
      affected: routes.length,
      detail:
        routes.length === 1
          ? "This server's only other member is not reachable from here."
          : `Not one of this server's ${routes.length} other members is reachable from here.`,
    });
  }
  return findings;
}

// --- what the console is given ------------------------------------------------------------
//
// The console takes plain data and nothing else. It is a viewer over the diagnostics, not a second
// place where application state lives, and handing it live objects (an `RTCPeerConnection`, a
// server record with its methods) would make it a second owner of things it has no business
// owning. Everything below is a snapshot the host assembles.

/** The part of a server the console reads. */
export type DebugServer = { id: number; name: string; channels?: unknown[] };

/** This device's own reachability, as the connectivity report already describes it. */
export type DebugDevice = {
  public_ipv4: string[];
  public_ipv6: string[];
  public_direct: boolean;
  autonat: string;
  router_maps: boolean;
  relay_likely_required: boolean;
  advice: string;
};

/**
 * One call participant's WebRTC state, flattened out of the live peer connection.
 *
 * Flattened deliberately: `RTCPeerConnection` fields change without telling anything, so a
 * component holding the object would render whatever was true at its last unrelated redraw. A
 * snapshot taken on the console's own poll is both simpler and more honest about how fresh it is.
 */
export type DebugVoicePeer = {
  fingerprint: string;
  connection: string;
  ice: string;
  signaling: string;
  path: "direct" | "relayed" | "unknown";
};

/** The console's sections, in rail order. */
export type DbgSection = "overview" | "network" | "voice" | "backend" | "frontend" | "storage";

export const DBG_SECTIONS: { id: DbgSection; label: string }[] = [
  { id: "overview", label: "Overview" },
  { id: "network", label: "Network" },
  { id: "voice", label: "Voice" },
  { id: "backend", label: "Backend" },
  { id: "frontend", label: "Frontend" },
  { id: "storage", label: "Storage" },
];

/** How many events the webview holds. The native ring is deeper; this is what is rendered. */
export const DBG_VIEW_CAP = 2000;

// --- capture control -----------------------------------------------------------------------
//
// Two independent axes, which one on/off switch used to conflate. With a single switch the only
// honest choices were "capture almost nothing" and "capture the transport layer narrating every
// address this device has ever seen", so it stayed off and nobody had a log when they needed one.

/** The four capture modes, quietest first. */
export const CAPTURE_MODES = ["off", "safe", "enhanced", "full"] as const;
export type CaptureMode = (typeof CAPTURE_MODES)[number];

/** The levels a section can be captured at, plus off. Loudest-only first. */
export const CAPTURE_LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;

/** One section's capture level, as `get_capture_config` returns it. */
export type SectionCapture = {
  id: string;
  /** Which console section it feeds, so twenty-two rows group under six headings. */
  view: string;
  /** Null when the section is off entirely. */
  level: string | null;
};

/** What the diagnostics are capturing right now. */
export type CaptureConfig = {
  mode: string;
  expires_at_restart: boolean;
  /** Whether this mode may render literal addresses. Answered natively, not re-derived here. */
  reveals_addresses: boolean;
  sections: SectionCapture[];
};

/**
 * What choosing a mode actually does, in the terms of the decision being made.
 *
 * Written out rather than left to a label because this is the one control in the app that changes
 * how much of the user's own network is written down. Someone turning on Enhanced to chase a
 * connection problem is making a real trade, and they can only make it if they are told what it is
 * before they press it rather than by comparing two exports afterwards.
 */
export function captureModeNote(mode: string): string {
  switch (mode) {
    case "off":
      return "Nothing is recorded and nothing accumulates. The console will stay empty, and there will be no evidence if something goes wrong while it is off.";
    case "safe":
      return "Stable codes, counts and durations. Identifiers become per-session references and addresses keep only their family and transport, so a Safe report can be pasted in public.";
    case "enhanced":
      return "Safe, plus literal addresses and transport detail. Choose this for a connection or multi-peer problem that cannot be located without knowing which address was actually tried. Read a report before sharing it.";
    case "full":
      return "Enhanced, plus per-span protocol detail. Maintainer reproduction only: it is loud, it reveals the most, and it is forgotten at the next launch rather than left running.";
    default:
      return "";
  }
}

/** Whether a mode should be confirmed before it is entered. */
export function captureModeIsRevealing(mode: string): boolean {
  return mode === "enhanced" || mode === "full";
}

/** The CSS class that colours a feed line's level tag. */
export function levelClass(level: string): string {
  return `lvl-${level === "ERROR" ? "err" : level.toLowerCase()}`;
}

/**
 * The chip for one WebRTC state value.
 *
 * `connected` and `completed` are both success (ICE reports the latter once it stops checking);
 * `failed` and `closed` are the two that end a call, and only one of them is a fault. Everything
 * in between is in progress, which is advisory rather than wrong.
 */
export function webrtcChip(state: string): { label: string; tone: "ok" | "warn" | "danger" | "faint" } {
  const label = state.toUpperCase();
  if (state === "connected" || state === "completed" || state === "stable") return { label, tone: "ok" };
  if (state === "failed") return { label, tone: "danger" };
  if (state === "closed") return { label, tone: "faint" };
  return { label, tone: "warn" };
}

/** The chip for how a call's media is actually travelling. */
export function mediaPathChip(path: DebugVoicePeer["path"]): {
  label: string;
  tone: "ok" | "warn" | "faint";
} {
  if (path === "relayed") return { label: "TURN RELAY", tone: "warn" };
  if (path === "direct") return { label: "DIRECT", tone: "ok" };
  return { label: "UNKNOWN", tone: "faint" };
}

// --- section serialisers -------------------------------------------------------------------

/**
 * This device's own reachability as copyable lines.
 *
 * These serialisers live beside the renderer on purpose. If copy had its own formatting the two
 * could disagree, and a pasted bug report that does not match the screenshot it arrived with is
 * worse than no bug report: the reader has to work out which one is lying before they can start.
 */
export function deviceLines(device: DebugDevice | null, aliases: Aliases, redact: boolean): string[] {
  if (!device) return [];
  const mask = (s: string) => maybeRedact(s, aliases, redact);
  return [
    `public ipv4: ${device.public_ipv4.map(mask).join(" ") || "(none)"}`,
    `public ipv6: ${device.public_ipv6.map(mask).join(" ") || "(none)"}`,
    `directly reachable: ${device.public_direct ? "yes" : "no"}`,
    `autonat: ${device.autonat || "(no verdict)"}`,
    `router maps ports: ${device.router_maps ? "yes" : "no"}`,
    `relay likely required: ${device.relay_likely_required ? "yes" : "no"}`,
  ];
}

/**
 * Every member's reachability as copyable lines, the same values the table renders.
 *
 * Each server's findings lead, before its rows. Whoever receives a pasted report should meet the
 * conclusion before the evidence: the rows are what the conclusion was drawn from, and asking a
 * reader to re-derive it from a column of multiaddrs is how the original incident stayed
 * undiagnosed for an hour.
 */
export function routeLines(
  servers: readonly DebugServer[],
  routes: Readonly<Record<number, MemberRoute[]>>,
  aliases: Aliases,
  redact: boolean,
  hasIpv6 = true,
): string[] {
  const mask = (s: string) => maybeRedact(s, aliases, redact);
  return servers.flatMap((s) => [
    ...routeFindings(routes[s.id] ?? [], hasIpv6).map(
      (f) => `${s.name} [${f.severity.toUpperCase()}] ${f.code} (${f.affected}) ${f.detail}`,
    ),
    ...(routes[s.id] ?? []).map((r) => {
      const chip = routeChip(routeState(r));
      const addresses = r.addresses.map(mask).join(" ") || "(no address)";
      const next = r.next_dial_in_ms ? formatDuration(r.next_dial_in_ms) : "-";
      return `${s.name} ${mask(r.fingerprint)} ${chip.label} peer=${mask(r.peer) || "(none)"} seq=${r.seq} fails=${r.dial_attempts} next=${next} ${addresses}`;
    }),
  ]);
}

/** The live call's per-peer state as copyable lines. Empty outside a call. */
export function voiceLines(peers: readonly DebugVoicePeer[], aliases: Aliases, redact: boolean): string[] {
  return peers.map(
    (p) =>
      `${maybeRedact(p.fingerprint, aliases, redact)} connection=${p.connection} ice=${p.ice} signaling=${p.signaling} path=${p.path}`,
  );
}

// --- copy --------------------------------------------------------------------------------

/**
 * The privacy sentence, appended to every copied bundle.
 *
 * It travels with the text because the person who receives a pasted report is not the person who
 * read the footer.
 *
 * # Why this no longer promises anything
 *
 * It used to end "It never includes message text, file contents, names or key material", and that
 * was false. An adversarial review listed the counterexamples: the report writes every server's
 * name, the `tracing` compatibility bridge carries arbitrary message prose, forwarded console
 * warnings and stacks carry whatever they were given, and the regex redactor only knows about
 * addresses and peer ids. A legacy warning like `failed to render "Private Support": C:\Users\...`
 * lands in a report whose footer told the recipient it contained no names.
 *
 * A false safety label is worse than none: it is the difference between someone reviewing a report
 * before pasting it into a public issue and someone not bothering. So this describes what a report
 * may contain and asks the reader to look, and it will keep saying so until the export validator
 * exists to make a stronger claim true. See `docs/reviews/Mewtual_PFixes_Part3_Adversarial_Review.md`
 * finding P3-002.
 */
export const PRIVACY_NOTE =
  "This report may contain IP addresses, peer and device identifiers, server names, local file paths, URLs, error text and activity metadata. Read it before you share it.";

/** One section of a copy bundle, under a heading copy can be scanned by. */
export function copySection(title: string, lines: readonly string[]): string {
  const body = lines.length ? lines.join("\n") : "(nothing captured)";
  return `== ${title.toUpperCase()} ==\n${body}`;
}

/**
 * Page the whole ring, rather than only what the console is currently rendering.
 *
 * The view holds the newest `DBG_VIEW_CAP` events; the native ring is deeper. For a copy that is
 * about to be read on screen the view is the right thing, but a saved report is evidence, and
 * evidence that silently stops at the window boundary is how a bug report arrives missing the
 * run-up to the failure it is describing.
 *
 * Bounded twice over: the loop stops when a page comes back short, and again at `maxPages`, so a
 * native side that kept answering could not spin this forever.
 */
export async function collectAllEvents(
  page: (afterSeq: number, limit: number) => Promise<LogEvent[]>,
  { pageSize = 500, maxPages = 40 }: { pageSize?: number; maxPages?: number } = {},
): Promise<LogEvent[]> {
  const all: LogEvent[] = [];
  let after = 0;
  for (let n = 0; n < maxPages; n += 1) {
    const batch = await page(after, pageSize);
    if (!batch.length) break;
    all.push(...batch);
    const newest = batch[batch.length - 1].seq;
    // A page that did not advance the sequence would loop forever. Treat it as the end.
    if (newest <= after) break;
    after = newest;
    if (batch.length < pageSize) break;
  }
  return all;
}

/**
 * Assemble the whole report.
 *
 * States its own redaction mode, because a reader who cannot tell masked output from real output
 * will read `[ip 1]` as a literal address and waste their time. It states its capture mode for the
 * stronger version of the same reason: Safe and Enhanced reports look alike and mean very different
 * things, and a reader who assumes Safe will read a reduced address as the whole story while a
 * reader who assumes Enhanced will go hunting for detail that was never captured.
 *
 * The session id is what matches an excerpt somebody pasted into a chat back to the report it was
 * taken from, which is otherwise guesswork once two of them exist.
 */
export function copyBundle(
  meta: { version: string; at: number; redacted: boolean; capture?: string; session?: string },
  sections: readonly { title: string; lines: string[] }[],
): string {
  const head = [
    `Mewtual debug console report`,
    `version: ${meta.version}`,
    `captured: ${new Date(meta.at).toISOString()}`,
    ...(meta.session ? [`session: ${meta.session}`] : []),
    ...(meta.capture ? [`capture: ${meta.capture}`] : []),
    `redaction: ${meta.redacted ? "on" : "off"}`,
  ].join("\n");
  const body = sections.map((s) => copySection(s.title, s.lines)).join("\n\n");
  return `${head}\n\n${body}\n\n${PRIVACY_NOTE}\n`;
}
