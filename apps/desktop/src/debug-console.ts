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

/** One diagnostic event, as `get_console_log` returns it. */
export type LogEvent = {
  seq: number;
  at_ms: number;
  /** `ERROR` | `WARN` | `INFO` | `DEBUG` | `TRACE`. */
  level: string;
  /** The emitting module: `catcoms_net`, `catcoms_sync`, `catcoms_ui` for the webview, and so on. */
  target: string;
  message: string;
  fields: [string, string][];
};

/** The counters behind the header roll-up and the rail badges. */
export type LogStats = {
  errors: number;
  warnings: number;
  dropped: number;
  latest_seq: number;
  capacity: number;
};

/**
 * The webview's own tracing target. Everything the frontend logs arrives through `log_ui` under
 * this name, which is what lets one ring feed both the Backend and Frontend sections: they are the
 * same stream split on one field, so their counts can never drift apart.
 */
export const UI_TARGET = "catcoms_ui";

/** Whether an event came from the webview rather than from Rust. */
export function isFrontend(e: LogEvent): boolean {
  return e.target === UI_TARGET;
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

/**
 * One event as its rendered line, minus the level tag (which is a separate span so it can carry
 * the loud colour while the message stays readable).
 *
 * Structured `tracing` fields are appended as `key=value`, which is how they read in the log file;
 * someone comparing a screenshot against a pasted log should not have to translate between two
 * renderings of the same event.
 */
export function eventText(e: LogEvent): string {
  const fields = e.fields.map(([k, v]) => `${k}=${v}`).join(" ");
  const head = e.message || "";
  if (!fields) return head;
  return head ? `${head} ${fields}` : fields;
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
): { ts: string; level: string; target: string; text: string } {
  return {
    ts: formatTime(e.at_ms),
    level: e.level,
    // A frontend row would otherwise read `catcoms_ui` on every single line, which is a column
    // that never varies inside the section that only shows that target.
    target: isFrontend(e) ? "" : e.target,
    text: maybeRedact(eventText(e), aliases, redact),
  };
}

/** The full line as copy writes it and as the filter matches against. */
export function eventLine(e: LogEvent, aliases: Aliases, redact: boolean): string {
  const p = eventParts(e, aliases, redact);
  const target = p.target ? ` ${p.target}` : "";
  return `${p.ts} ${p.level.padEnd(5)}${target} ${p.text}`;
}

/** How a feed narrows what it shows. Display only: capture is never filtered here. */
export type FeedFilter = {
  levels: readonly string[];
  /** Substring match against the tracing target. Empty matches everything. */
  target?: string;
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
  return events.filter((e) => {
    if (!f.levels.includes(e.level)) return false;
    if (target && !e.target.toLowerCase().includes(target)) return false;
    if (needle && !eventLine(e, aliases, redact).toLowerCase().includes(needle)) return false;
    return true;
  });
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

// --- copy --------------------------------------------------------------------------------

/**
 * The privacy sentence, appended to every copied bundle.
 *
 * The same contract the Diagnostics settings page states. It travels with the text because the
 * person who receives a pasted report is not the person who read the footer.
 */
export const PRIVACY_NOTE =
  "This report can include your IP addresses, peer and device identifiers, and timing. It never includes message text, file contents, names or key material.";

/** One section of a copy bundle, under a heading copy can be scanned by. */
export function copySection(title: string, lines: readonly string[]): string {
  const body = lines.length ? lines.join("\n") : "(nothing captured)";
  return `== ${title.toUpperCase()} ==\n${body}`;
}

/**
 * Assemble the whole report.
 *
 * States its own redaction mode, because a reader who cannot tell masked output from real output
 * will read `[ip 1]` as a literal address and waste their time.
 */
export function copyBundle(
  meta: { version: string; at: number; redacted: boolean },
  sections: readonly { title: string; lines: string[] }[],
): string {
  const head = [
    `Mewtual debug console report`,
    `version: ${meta.version}`,
    `captured: ${new Date(meta.at).toISOString()}`,
    `redaction: ${meta.redacted ? "on" : "off"}`,
  ].join("\n");
  const body = sections.map((s) => copySection(s.title, s.lines)).join("\n\n");
  return `${head}\n\n${body}\n\n${PRIVACY_NOTE}\n`;
}
