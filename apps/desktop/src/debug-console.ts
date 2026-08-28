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

// Borrowed rather than restated: a second byte formatter would eventually round differently from
// the transfer readouts, and two sizes for one number is the sort of thing a reader stops to
// reconcile in the middle of diagnosing something else.
import { formatBytes } from "./transfer-visual.ts";

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
  /** Fields this event had to drop at the cap, so a shortened list reads as shortened. */
  fields_dropped: number;
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
export type Aliases = {
  /**
   * Random per console session, and never exported.
   *
   * The alias has to be unguessable, not just stable. Deriving it from something printed in the
   * report, the session id say, would make it reversible: IPv4 is a four-billion-value space and a
   * reader holding the salt could simply try them all. A value nobody outside this window has is
   * the difference between masking an address and encoding it.
   */
  salt: string;
  /** Pure cache. The alias is a function of its inputs, so this only saves the arithmetic. */
  cache: Map<string, string>;
};

/**
 * A fresh aliasing scheme.
 *
 * `salt` is injectable for tests, which need reproducible aliases; nothing in the app passes one.
 */
export function makeAliases(salt = randomSalt()): Aliases {
  return { salt, cache: new Map() };
}

function randomSalt(): string {
  const bytes = new Uint8Array(16);
  globalThis.crypto.getRandomValues(bytes);
  return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/**
 * A short stable tag for one value.
 *
 * FNV-1a, truncated. Not a security boundary: what protects the underlying value is the salt above,
 * which never leaves this window. This only has to be stable and collide rarely, and at six hex
 * characters a report holding a hundred distinct addresses has about a three-in-ten-thousand chance
 * of any pair sharing a tag.
 */
function tag(salt: string, kind: string, value: string): string {
  let hash = 0x811c9dc5;
  // The separators are written as escapes on purpose. As raw bytes they made this file read as
  // binary to grep and every other text tool, and anything that quietly strips them would join
  // the three fields into one string where a kind boundary could land anywhere.
  for (const ch of `${salt}\x00${kind}\x00${value}`) {
    hash ^= ch.codePointAt(0) ?? 0;
    // The FNV prime, by shift-add because JavaScript's `*` would lose the low bits to a double.
    hash = (hash + ((hash << 1) + (hash << 4) + (hash << 7) + (hash << 8) + (hash << 24))) >>> 0;
  }
  return (hash & 0xffffff).toString(16).padStart(6, "0");
}

/**
 * The alias for one value.
 *
 * # Why this is not a counter any more
 *
 * It used to mint `[ip 1]`, `[ip 2]` in the order values were first seen, in a map kept for the
 * console's lifetime. So merely visiting a section, typing a filter or rendering a route before
 * pressing Save decided which address got which number, and the same events exported differently
 * depending on where the user had clicked first. A report that cannot be diffed against another
 * cannot be compared between two peers, which is how some sync bugs are localised at all. Found by
 * adversarial review (P3-015).
 *
 * A function of the value instead, so encounter order cannot reach it. The property that matters is
 * unchanged and is the reason redaction is aliased rather than blanked: the same address is the
 * same alias every time it appears, so "it keeps dialling the same two addresses" survives masking,
 * and that sentence is the whole diagnosis of the hour-long isolation.
 */
export function alias(aliases: Aliases, kind: string, value: string): string {
  const key = `${kind}:${value}`;
  const existing = aliases.cache.get(key);
  if (existing) return existing;
  const minted = `[${kind} ${tag(aliases.salt, kind, value)}]`;
  aliases.cache.set(key, minted);
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
  // Last, and only when it happened. An event that hit the field cap shows a shortened list, and
  // without this the shortening is the one thing the line does not mention: a reader would take
  // what is there for the whole of it. The native renderings say the same.
  if (e.fields_dropped > 0) bits.push(`fields_dropped=${e.fields_dropped}`);
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
 *
 * The one line of `retentionStatus`, for a caller that wants prose rather than parts. Pass the sink
 * snapshot: without one this can only say that the file's state is unknown, which is honest but is
 * the weakest thing it could say.
 */
export function dropNote(dropped: number, kept: number, sink: DebugLogSink | null = null): string {
  return retentionStatus({ dropped, kept, sink }).text;
}

/** The `n of m shown` readout, so an over-eager filter can never look like an empty feed. */
export function shownCount(shown: number, total: number): string {
  return shown === total ? `${total.toLocaleString()}` : `${shown.toLocaleString()} of ${total.toLocaleString()} shown`;
}

// --- what the log file actually holds ------------------------------------------------------
//
// The drop notice used to end "The debug log file keeps everything", which was false in six
// separate ways at once: file logging may be switched off, its initialisation may have failed, its
// filter is deliberately narrower than the ring's, its queue drops under pressure, it rotates under
// a session quota, and an event the capture settings excluded never reached either store. That is
// the exact shape of reassurance the logger-health work exists to remove, and it was worse than
// saying nothing: somebody reads it, stops screenshotting, and goes looking for a file that either
// does not exist or does not contain the part they needed. Found by adversarial review (P3-020).
//
// Everything below is decided by a value the console can actually read, and where it cannot read
// one it says so rather than guessing kindly.

/**
 * The file sink's own account of itself, as `get_debug_logging` returns it.
 *
 * A structural subset of that reply, so the shell can hand its existing snapshot straight in
 * without a conversion that could quietly disagree with it.
 *
 * `enabled` is what the user asked for; `state` is what the writer is doing. They are separate
 * fields because the entire value of this record is that they can disagree, and every sentence
 * derived from it below is decided by `state`. A preference is a request, not evidence.
 */
export type DebugLogSink = {
  /** Whether a file was asked for. */
  enabled: boolean;
  /** Whether this process is writing one. Derived from bytes that reached a file. */
  active: boolean;
  /** `stopped` | `active` | `degraded` | `failed`. */
  state: string;
  /** Why it is degraded or failed. Shown verbatim: "permission denied" is actionable. */
  error: string;
  /** The file this process opened, or empty when it opened none. */
  file: string;
  events_written: number;
  bytes_written: number;
  /** Events that never reached the file: queue overflow, or emitted after the quota stopped it. */
  events_dropped: number;
  /** Events that reached the file with their tail cut off. Present, and they say so. */
  events_truncated: number;
  queue_high_water: number;
  /** The session byte quota, so how close this run has come to it can be stated rather than felt. */
  session_quota_bytes: number;
};

/** A colour job, shared by the sink summary and the retention notice. */
export type SinkTone = "ok" | "warn" | "danger" | "faint";

/**
 * The targets the shared log file records at `debug`. Everything else it holds at `info`.
 *
 * Mirrors `APP_FILE_FILTER` in `crates/catcoms-log/src/lib.rs`, which is deliberately narrower than
 * the ring's `CONSOLE_RING_FILTER`: the file is the thing a user pastes to somebody else, and the
 * transport, storage and replication layers at `debug` are both the bulk of the volume and the most
 * identifying part of it. The narrowing is a privacy decision and a good one; what it is not is a
 * detail the console may leave out while telling somebody the file has their missing entries.
 */
export const FILE_DEBUG_TARGETS = [
  "catcoms_app",
  "catcoms_sync",
  "catcoms_mls",
  "catcoms_discovery",
  UI_TARGET,
] as const;

/**
 * Whether the file layer's filter refused this event outright.
 *
 * True for anything at TRACE, and for DEBUG outside the five targets above. Such an event is in the
 * ring and was never eligible for the file, so no amount of sink health makes the file a fallback
 * for it.
 */
export function belowFileFilter(e: LogEvent): boolean {
  if (e.level === "TRACE") return true;
  if (e.level !== "DEBUG") return false;
  return !FILE_DEBUG_TARGETS.some((t) => e.target === t || e.target.startsWith(`${t}::`));
}

/** How full a session's quota has to get before it is worth mentioning unprompted. */
const QUOTA_NOTICE = 0.9;

/**
 * How the sink's state reads to someone who did not write it.
 *
 * The single place the console phrases file-log health, so the drop notice, the sink panel and a
 * copied report cannot end up disagreeing about whether a log exists. Every branch is chosen by
 * `state`, which the writer sets from bytes that reached a file, and never by `enabled`.
 */
export function sinkSummary(sink: DebugLogSink | null): { tone: SinkTone; text: string } {
  // No snapshot is its own answer, and a rare one: the command needs an unlocked session, so a
  // console opened over a locked vault genuinely does not know. Saying so beats assuming either way.
  if (!sink) {
    return { tone: "warn", text: "The debug log file's state has not been read, so what it holds is unknown." };
  }
  if (sink.state === "failed") {
    return {
      tone: "danger",
      text: sink.error
        ? `The debug log file is not being written: ${sink.error}`
        : "The debug log file is not being written, and the reason was not recorded.",
    };
  }
  if (sink.state === "degraded") {
    return {
      tone: "warn",
      text: sink.events_dropped
        ? `Writing, but ${sink.events_dropped.toLocaleString()} record(s) never reached the file.`
        : "Writing, but close enough to this session's limit to matter.",
    };
  }
  if (sink.state === "active") {
    return {
      tone: "ok",
      text: `Writing. ${sink.events_written.toLocaleString()} entries, ${formatBytes(sink.bytes_written)} so far.`,
    };
  }
  return {
    tone: "faint",
    text: sink.enabled
      ? "Not writing yet. A log can only be opened when the app starts, so restart Mewtual."
      : "Not writing, because you have logging switched off.",
  };
}

/**
 * The sink's health as copyable lines.
 *
 * Beside the summary for the reason every serialiser here is: a pasted report that does not match
 * the screenshot it arrived with makes the reader work out which one is lying first.
 */
export function sinkLines(sink: DebugLogSink | null): string[] {
  if (!sink) return ["debug log: not read"];
  const quota = sink.session_quota_bytes > 0 ? ` of ${formatBytes(sink.session_quota_bytes)}` : "";
  return [
    `preference: ${sink.enabled ? "on" : "off"}`,
    `sink state: ${sink.state}`,
    `file: ${sink.file || "(none opened)"}`,
    `written: ${sink.events_written.toLocaleString()} entries, ${formatBytes(sink.bytes_written)}${quota}`,
    `dropped: ${sink.events_dropped.toLocaleString()}`,
    `truncated: ${sink.events_truncated.toLocaleString()}`,
    `queue high water: ${sink.queue_high_water.toLocaleString()}`,
    ...(sink.error ? [`last error: ${sink.error}`] : []),
  ];
}

/** One measured reason the file cannot be taken for a complete record. */
export type RetentionCaveat = {
  /** A stable code, so this can be counted and searched rather than read. */
  code: string;
  text: string;
};

/** What the console can honestly claim about history the ring no longer holds. */
export type RetentionStatus = {
  dropped: number;
  kept: number;
  total: number;
  /**
   * `none` when the file demonstrably has nothing more, `partial` when it has been written this
   * session and may have some of it, `unknown` when the sink was never read. Never `all`: no state
   * of this system supports that word.
   */
  file: "none" | "partial" | "unknown";
  tone: SinkTone;
  /** What the ring itself lost. Empty when nothing was dropped. */
  ring: string;
  /** What the file's own reported state says about the gap. */
  sink: string;
  caveats: RetentionCaveat[];
  /** Every sentence above, joined, which is what the pinned notice and a copied report use. */
  text: string;
};

/**
 * What happened to the entries the ring dropped, in terms the console can back up.
 *
 * Structured rather than a formatted blob because the console renders the parts differently: the
 * ring sentence is the headline, the sink sentence carries the tone, and the caveats are a list. A
 * caller that only wants prose has `dropNote`.
 *
 * `filtered` and `events` are what turn two of these sentences from configuration trivia into
 * measurements: `LogStats.filtered` counts what the capture settings excluded from both stores, and
 * the held events are where the file's narrower filter can be counted rather than merely asserted.
 *
 * Nothing was dropped means there is nothing to say, and every field comes back empty.
 */
export function retentionStatus(input: {
  dropped: number;
  kept: number;
  sink: DebugLogSink | null;
  /** Events the capture config excluded outright, from `LogStats`. */
  filtered?: number;
  /** The events the console still holds, so the file's filter can be counted against them. */
  events?: readonly LogEvent[];
}): RetentionStatus {
  const dropped = Math.max(0, input.dropped);
  const kept = Math.max(0, input.kept);
  const total = dropped + kept;
  if (dropped <= 0) {
    return { dropped: 0, kept, total, file: "unknown", tone: "faint", ring: "", sink: "", caveats: [], text: "" };
  }

  const ring = `Ring full: oldest entries dropped. Showing the last ${kept.toLocaleString()} of ${total.toLocaleString()} this session.`;
  const s = input.sink;
  const caveats: RetentionCaveat[] = [];

  // Applies whatever the file is doing, and it is the one loss neither store can make good: an
  // event the capture settings refused was never built, so raising the mode now brings back nothing.
  const filtered = input.filtered ?? 0;
  if (filtered > 0) {
    caveats.push({
      code: "LOG.CAPTURE.EXCLUDED",
      text: `${filtered.toLocaleString()} event(s) were never captured at all under the current capture settings, so neither this console nor the file has them.`,
    });
  }

  let file: RetentionStatus["file"];
  let sink: string;
  let tone: SinkTone;
  if (!s) {
    file = "unknown";
    tone = "warn";
    sink = "The debug log file's state has not been read here, so whether it kept any of them is unknown. Settings, Diagnostics reports what the file is actually doing.";
  } else if (s.state === "failed") {
    file = "none";
    tone = "danger";
    sink = s.error
      ? `No file is being written (${s.error}), so the dropped entries are gone.`
      : "No file is being written, and the reason was not recorded, so the dropped entries are gone.";
  } else if (s.state === "stopped") {
    file = "none";
    tone = "warn";
    // The preference matters here and only here. Off is a choice the user made and the sentence
    // should name it; on-but-stopped is the app failing to honour one, and the two want different
    // next steps from the person reading.
    sink = s.enabled
      ? "Logging is switched on but this session opened no file, so the dropped entries are gone. A log can only be opened when the app starts."
      : "File logging is switched off, so nothing was written to disk and the dropped entries are gone.";
  } else {
    file = "partial";
    tone = s.state === "degraded" || s.events_dropped > 0 ? "warn" : "faint";
    const named = s.file ? ` (${s.file})` : "";
    sink = `The debug log file${named} has taken ${s.events_written.toLocaleString()} entries this session and may hold some of these, but it is not a complete record of them.`;

    if (s.events_dropped > 0) {
      caveats.push({
        code: "LOG.FILE.DROPPED",
        text: `${s.events_dropped.toLocaleString()} record(s) never reached the file: its queue overflowed, or the session quota had already stopped it.`,
      });
    }
    if (s.events_truncated > 0) {
      caveats.push({
        code: "LOG.FILE.TRUNCATED",
        text: `${s.events_truncated.toLocaleString()} record(s) reached the file with their tail cut off.`,
      });
    }
    if (s.session_quota_bytes > 0 && s.bytes_written >= s.session_quota_bytes * QUOTA_NOTICE) {
      caveats.push({
        code: "LOG.FILE.QUOTA",
        text:
          s.bytes_written >= s.session_quota_bytes
            ? `This session has reached its ${formatBytes(s.session_quota_bytes)} file limit, so nothing further is being written.`
            : `This session has written ${formatBytes(s.bytes_written)} of a ${formatBytes(s.session_quota_bytes)} limit, and writing stops at the limit.`,
      });
    }

    // Always, because it is true of every healthy file too, and it is the caveat somebody reading
    // "the file has more" would never think to check. Counted against what is on screen when there
    // is something to count, because a number is harder to wave away than a configuration note.
    const excluded = (input.events ?? []).filter(belowFileFilter).length;
    caveats.push({
      code: "LOG.FILE.NARROWER_FILTER",
      text: excluded
        ? `The file's capture filter is narrower than this console's: ${excluded.toLocaleString()} of the entries still shown here were never eligible for it.`
        : "The file's capture filter is narrower than this console's, so some kinds of entry never reach it whatever its state.",
    });
  }

  return {
    dropped,
    kept,
    total,
    file,
    tone,
    ring,
    sink,
    caveats,
    text: [ring, sink, ...caveats.map((c) => c.text)].join(" "),
  };
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
  health:
    | "no_peer_record"
    | "claimed_peer_has_no_route"
    | "claimed_peer_connected_direct"
    | "claimed_peer_connected_relay"
    | "claimed_peer_connected_other"
    | "claimed_peer_dial_cooling_down"
    | "claimed_peer_dial_eligible"
    | string;
  /** Self-asserted; reciprocal repair is not a dual-key transport-ownership proof. */
  binding: "absent" | "self_asserted" | string;
  active_paths: ConnectionPath[];
  last_success: { path: ConnectionPath; age_ms: number } | null;
  candidate_families: ConnectionFamily[];
  candidate_transports: ConnectionTransport[];
  actions: MemberRouteAction[];
  /** Signed, request-bound observations from currently proven member paths. */
  indirect_health: "unknown" | "reachable_via_member" | "suspected_unreachable" | string;
  indirect_witnesses: number;
  indirect_age_ms: number | null;
  /** This device queued a bounded request to a live helper for this member to dial us back. */
  reciprocal_pending: boolean;
};

export type MemberRoutePollAnswer =
  | { id: number; ok: true; rows: MemberRoute[] }
  | { id: number; ok: false; rows: MemberRoute[] };

/**
 * Merge one asynchronous all-server route poll without ever matching answers by array position.
 *
 * A failed or not-yet-requested server keeps its last rows so an operator can inspect the previous
 * snapshot, but is explicitly marked unavailable. Callers must not derive live findings from rows
 * whose id is in `unavailable`.
 */
export function mergeMemberRoutePoll(
  currentServerIds: readonly number[],
  previous: Readonly<Record<number, MemberRoute[]>>,
  answers: readonly MemberRoutePollAnswer[],
): { routes: Record<number, MemberRoute[]>; unavailable: Set<number> } {
  const current = new Set(currentServerIds);
  const routes: Record<number, MemberRoute[]> = {};
  const unavailable = new Set<number>();
  for (const answer of answers) {
    if (!current.has(answer.id)) continue;
    if (answer.ok) routes[answer.id] = answer.rows;
    else {
      routes[answer.id] = previous[answer.id] ?? [];
      unavailable.add(answer.id);
    }
  }
  for (const id of current) {
    if (!(id in routes)) {
      routes[id] = previous[id] ?? [];
      unavailable.add(id);
    }
  }
  return { routes, unavailable };
}

/** Apply one main-Connectivity read while preserving stale-server and failed-read truthfulness. */
export function mergeMemberRouteRead(
  currentServer: number | null,
  requestedServer: number,
  previousRoutes: readonly MemberRoute[],
  previousUnavailable: boolean,
  response: readonly MemberRoute[] | null,
): { applied: boolean; routes: MemberRoute[]; unavailable: boolean } {
  if (currentServer !== requestedServer) {
    return {
      applied: false,
      routes: [...previousRoutes],
      unavailable: previousUnavailable,
    };
  }
  if (response === null) {
    return { applied: true, routes: [...previousRoutes], unavailable: true };
  }
  return { applied: true, routes: [...response], unavailable: false };
}

export type ConnectionFamily = "ipv4" | "ipv6" | "dns" | "memory" | "unknown" | string;
export type ConnectionTransport =
  | "tcp"
  | "quic_v1"
  | "websocket"
  | "circuit_relay"
  | "memory"
  | "unknown"
  | string;
export type ConnectionPath = {
  family: ConnectionFamily;
  transport: ConnectionTransport;
  direction: "dialer" | "listener" | string;
};
export type MemberRouteAction = {
  scope: "this_device" | "member_device" | "group" | string;
  kind:
    | "wait_for_automatic_recovery"
    | "check_member_connectivity"
    | "keep_another_member_connected"
    | "configure_fallback_node"
    | "probe_through_members"
    | "retry_group_now"
    | string;
};

export type RouteState =
  | "direct"
  | "relay"
  | "connected-other"
  | "no-record"
  | "no-route"
  | "cooldown"
  | "dial-eligible"
  | "unknown";

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
  switch (r.health) {
    case "claimed_peer_connected_direct": return "direct";
    case "claimed_peer_connected_relay": return "relay";
    case "claimed_peer_connected_other": return "connected-other";
    case "no_peer_record": return "no-record";
    case "claimed_peer_has_no_route": return "no-route";
    case "claimed_peer_dial_cooling_down": return "cooldown";
    case "claimed_peer_dial_eligible": return "dial-eligible";
    default: return "unknown";
  }
}

export type RouteConnectionState = "connected" | "disconnected" | "unknown";

/** Preserve unknown as a third state; treating it as disconnected invents an isolation verdict. */
export function routeConnectionState(r: MemberRoute): RouteConnectionState {
  const state = routeState(r);
  if (["direct", "relay", "connected-other"].includes(state)) return "connected";
  if (state === "unknown") return "unknown";
  return "disconnected";
}

export function routeIsConnected(r: MemberRoute): boolean {
  return routeConnectionState(r) === "connected";
}

export function routeIsDisconnected(r: MemberRoute): boolean {
  return routeConnectionState(r) === "disconnected";
}

export type RouteGroupState = "alone" | "all-connected" | "none-connected" | "partial" | "unknown";

export function routeGroupState(routes: readonly MemberRoute[]): RouteGroupState {
  if (routes.length === 0) return "alone";
  const states = routes.map(routeConnectionState);
  if (states.includes("unknown")) return "unknown";
  const connected = states.filter((state) => state === "connected").length;
  if (connected === routes.length) return "all-connected";
  if (connected === 0) return "none-connected";
  return "partial";
}

/** Current-count cells for the overview; stale rows cannot masquerade as live arithmetic. */
export function routeOverviewCounts(
  routes: readonly MemberRoute[],
  unavailable: boolean,
): { connected: string; roster: string } {
  if (unavailable) return { connected: "—", roster: "—" };
  const connected = routes.filter(routeIsConnected).length;
  return { connected: `${connected} / ${routes.length}`, roster: String(routes.length + 1) };
}

/** The chip label and semantic colour job for a route state. */
export function routeChip(state: RouteState): { label: string; tone: "ok" | "warn" | "danger" | "faint" } {
  switch (state) {
    case "direct":
      return { label: "DIRECT PATH", tone: "ok" };
    case "relay":
      return { label: "RELAY PATH", tone: "ok" };
    case "connected-other":
      return { label: "PATH LIVE", tone: "ok" };
    case "no-record":
      return { label: "NO RECORD", tone: "danger" };
    case "no-route":
      return { label: "NO ROUTE", tone: "danger" };
    case "cooldown":
      return { label: "DIAL COOLDOWN", tone: "warn" };
    case "dial-eligible":
      return { label: "DIAL ELIGIBLE", tone: "faint" };
    default:
      return { label: "UNKNOWN", tone: "warn" };
  }
}

/** A retained row must never reuse the live green/danger visual language after its read failed. */
export function routeDisplayChip(
  state: RouteState,
  unavailable: boolean,
): { label: string; tone: "ok" | "warn" | "danger" | "faint" } {
  const current = routeChip(state);
  return unavailable ? { label: `LAST: ${current.label}`, tone: "warn" } : current;
}

/**
 * A one-line explanation of a member's state, in the terms someone can act on.
 *
 * Address-family shape is useful evidence, but this snapshot does not test outbound IPv6. It must
 * therefore describe IPv6-only candidates as a clue and never promote them into a causal verdict.
 */
export function routeExplanation(r: MemberRoute, _hasPublicIpv6Observation = false): string {
  switch (routeState(r)) {
    case "direct":
      return "This device has a live non-circuit path to the transport identity in this member's signed record. That device-to-transport binding is self-asserted, not independently proven.";
    case "relay":
      return "This device has a live circuit-relay path to the transport identity in this member's signed record. That device-to-transport binding is self-asserted, not independently proven.";
    case "connected-other":
      return "A connection to the transport identity in this member's signed record is live, but this transport supplied no path classification. The identity binding is self-asserted.";
    case "no-record":
      return "No signed peer record learned yet, so this member cannot be dialled, called or sent a friend request. It arrives over PEX once any member that holds it is reachable.";
    case "no-route":
      return "This member's record carries no dialable address. Nothing can be attempted until it publishes one.";
    case "cooldown": {
      const only6 =
        r.candidate_families.length > 0 &&
        r.candidate_families.every((family) => family === "ipv6");
      const wait = r.next_dial_in_ms > 0 ? ` Next attempt in ${formatDuration(r.next_dial_in_ms)}.` : "";
      const family = only6
        ? " Its advertised candidates are IPv6-only; this report does not measure outbound IPv6 capability, so that is a clue rather than a proven cause."
        : "";
      return `${r.dial_attempts} policy-approved dial batch(es) have been submitted and the scheduler cooldown is active. Submission alone does not prove that every candidate was attempted or failed.${family}${wait}`;
    }
    case "dial-eligible":
      return "A signed record has candidate routes and its claimed peer is eligible for an automatic dial pass; it is not connected here right now.";
    default:
      return "This app version does not recognize the backend's route-health value, so it will not guess whether the claimed peer is connected.";
  }
}

/**
 * Explain helper evidence without turning a signed observation about a claimed transport peer
 * into presence, identity-control, or internet-reachability proof.
 */
export function routeIndirectEvidence(r: MemberRoute): string | null {
  const age = r.indirect_age_ms === null ? "" : ` ${formatDuration(r.indirect_age_ms)} ago`;
  const witnesses = `${r.indirect_witnesses} authenticated member${r.indirect_witnesses === 1 ? "" : "s"}`;
  switch (r.indirect_health) {
    case "reachable_via_member":
      return `${witnesses} reported a live path to this claimed peer${age}. That is indirect path evidence, not proof the person is online.`;
    case "suspected_unreachable":
      return `${witnesses} did not observe a live path to this claimed peer${age}. This is suspicion, not proof that the person or device is offline.`;
    default:
      return r.reciprocal_pending
        ? "This device queued a bounded reciprocal-dial request to a live member. Queueing does not prove the helper or target received it or opened a path."
        : null;
  }
}

/** Short, stable wording for the backend-owned recommended actions. */
export function routeActionLabel(action: MemberRouteAction): string {
  switch (action.kind) {
    case "wait_for_automatic_recovery": return "Wait for automatic retry";
    case "check_member_connectivity": return "Ask them to run Connectivity";
    case "keep_another_member_connected": return "Keep another group member connected";
    case "configure_fallback_node": return "Configure a trusted fallback node";
    case "probe_through_members": return "Check paths through connected members";
    case "retry_group_now": return "Retry this group's current routes now";
    default: return "Update Mewtual to understand this recommendation";
  }
}

export function routeActionScopeLabel(action: MemberRouteAction): string {
  switch (action.scope) {
    case "this_device": return "you";
    case "member_device": return "that member";
    case "group": return "group";
    default: return "unknown actor";
  }
}

export function routePathLabel(path: ConnectionPath): string {
  const transport: Record<string, string> = {
    tcp: "TCP",
    quic_v1: "QUIC",
    websocket: "WebSocket",
    circuit_relay: "circuit relay",
    memory: "memory",
    unknown: "unknown transport",
  };
  const family: Record<string, string> = {
    ipv4: "IPv4",
    ipv6: "IPv6",
    dns: "DNS",
    memory: "memory",
    unknown: "unknown family",
  };
  return `${family[path.family] ?? "unknown family"} · ${transport[path.transport] ?? "unknown transport"}`;
}

/** Advance a historical backend age using the frontend's local monotonic-enough wall-clock tick. */
export function routeHistoricalAge(
  route: MemberRoute,
  receivedAtMs: number,
  nowMs: number,
): number | null {
  if (!route.last_success) return null;
  return route.last_success.age_ms + Math.max(0, nowMs - receivedAtMs);
}

/** Whether an event can affect the member-route rows currently visible in Connectivity. */
export function shouldRefreshMemberRoutes(
  activeServer: number | null,
  view: string,
  changedServer: number,
): boolean {
  return activeServer === changedServer && view === "connectivity";
}

/** Time-derived cooldown and history expiry need a bounded refresh even without an event. */
export function memberRoutesVisible(activeServer: number | null, view: string): boolean {
  return activeServer !== null && view === "connectivity";
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
 * isolated for an hour dialling IPv6-only records, and the evidence for that was a scattering of
 * raw `sendmsg` warnings from inside the QUIC stack. Candidate shape is useful context, but this
 * report deliberately does not infer outbound IPv6 capability from inbound/public advertisements.
 *
 * These are conclusions, not observations, and they carry codes so they can be counted across
 * reports instead of re-derived by whoever is reading.
 */
export function routeFindings(
  routes: readonly MemberRoute[],
  _hasPublicIpv6Observation = false,
): RouteFinding[] {
  const findings: RouteFinding[] = [];
  if (routes.length === 0) return findings;

  const withoutLiveConnection = routes.filter(routeIsDisconnected);
  if (withoutLiveConnection.length === 0) return findings;

  // IPv6-only candidates are worth surfacing, but are not a causal verdict: the reachability
  // snapshot does not currently measure the host's outbound IPv6 route.
  const v6Only = withoutLiveConnection.filter(
    (r) =>
      r.candidate_families.length > 0 &&
      r.candidate_families.every((family) => family === "ipv6"),
  );
  if (v6Only.length > 0) {
    findings.push({
      code: "NET.ROUTE.IPV6_ONLY",
      severity: "warn",
      affected: v6Only.length,
      detail:
        `${members(v6Only.length)} ${v6Only.length === 1 ? "advertises" : "advertise"} only IPv6 ` +
        `addresses. This report does not measure outbound IPv6 capability, so it cannot prove that ` +
        `the address family explains the absence of a live connection; if dials do not connect, ` +
        `check IPv6 or add a relay route.`,
    });
  }

  const noRecord = withoutLiveConnection.filter((r) => routeState(r) === "no-record");
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

  const noRoute = withoutLiveConnection.filter((r) => routeState(r) === "no-route");
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
  if (routes.every(routeIsDisconnected)) {
    findings.push({
      code: "NET.ROUTE.NO_LIVE_MEMBER_CONNECTION",
      severity: "warn",
      affected: routes.length,
      detail:
        routes.length === 1
          ? "No claimed peer for this server's only other member is connected here right now. This does not prove its routes are unreachable."
          : `No claimed peer for this server's ${routes.length} other members is connected here right now. This does not prove their routes are unreachable.`,
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

/**
 * One supervised background task, as `get_task_health` returns it.
 *
 * The answer that used to be a log line and then, once the line aged out of the ring, nothing at
 * all. A task can die while everything around it stays healthy: the event forwarder in particular
 * can stop while the server actor is fine, so the protocol keeps running and the webview is told
 * none of it. What a user sees is a stale unread badge, and what the app used to say was that
 * everything was working.
 */
export type TaskHealth = {
  id: number;
  /** `event_forwarder`, `server_actor`, `discovery_timer`, and so on. */
  kind: string;
  server: number | null;
  started_ms: number;
  /** Only for a task that declared a rhythm; null means it never promised one. */
  last_beat_ms: number | null;
  /** `running` | `exited` | `cancelled` | `panicked` | `stalled`. */
  state: string;
  /** Whether somebody should be told. Decided natively, so there is only one opinion about it. */
  fault: boolean;
  cause: string | null;
};

/** The chip for a task's state. */
export function taskChip(task: TaskHealth): { label: string; tone: "ok" | "warn" | "danger" | "faint" } {
  const label = task.state.toUpperCase();
  if (!task.fault) return { label, tone: task.state === "running" ? "ok" : "faint" };
  return { label, tone: task.state === "stalled" ? "warn" : "danger" };
}

/**
 * What a dead task means for the person looking at the app, rather than for the process.
 *
 * "event_forwarder panicked" is precise and tells a user nothing. The whole reason this is worth
 * surfacing is that each of these has a visible consequence, and the consequence is what someone
 * recognises from their own screen.
 */
export function taskConsequence(kind: string): string {
  switch (kind) {
    case "event_forwarder":
      return "This server's updates are no longer reaching the window. Messages, unread badges, presence and the jukebox will all look frozen while the server itself keeps working. Reopening the app restores it.";
    case "server_actor":
      return "This server has stopped entirely. Nothing will send or arrive on it until the app is reopened.";
    case "discovery_timer":
      return "This server has stopped looking for members' current addresses, so peers that move will gradually become unreachable.";
    case "network_monitor":
      return "Network changes are no longer noticed immediately. Reconnection still happens on the ordinary poll, just later.";
    case "port_mapping_fold":
    case "autonat_fold":
    case "relay_fold":
    case "mesh_observation_fold":
      return "This server's reachability report has stopped updating. Connections are unaffected; what is shown about them may be out of date.";
    default:
      return "A background task stopped. What it was responsible for is no longer being done.";
  }
}

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

/**
 * Must anything leaving this console have its addresses masked, whatever the toggle says?
 *
 * Safe capture is documented as "no literal addresses, so a Safe report is one a user can paste in
 * public", and the events honour that: an address value asks the mode before it renders. The
 * reachability and device tables never did, because they are live snapshots read straight from
 * their own commands rather than events, and they masked only when the screenshot toggle was on.
 *
 * So a Safe report carried this machine's public address by default, and the native validator
 * refused to write it: the very first Save in a fresh console failed. The rule was right and the
 * data was wrong. Deciding it here brings the tables under the same promise the events already
 * keep, rather than lowering the promise to fit them.
 *
 * The screen is unaffected. An operator looking at the Network section still sees the addresses;
 * this governs only what is copied or saved, which is the part that leaves the machine.
 */
export function exportMasksAddresses(capture: string, redact: boolean): boolean {
  return redact || !captureModeIsRevealing(capture);
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
  unavailable: ReadonlySet<number> = new Set(),
): string[] {
  const mask = (s: string) => maybeRedact(s, aliases, redact);
  return servers.flatMap((s) => {
    if (unavailable.has(s.id)) {
      return [`${s.name} [UNAVAILABLE] member-route refresh failed; retained rows were omitted because they are only a last snapshot.`];
    }
    return [
      ...routeFindings(routes[s.id] ?? [], hasIpv6).map(
        (f) => `${s.name} [${f.severity.toUpperCase()}] ${f.code} (${f.affected}) ${f.detail}`,
      ),
      ...(routes[s.id] ?? []).map((r) => {
      const chip = routeChip(routeState(r));
      const addresses = r.addresses.map(mask).join(" ") || "(no address)";
      const next = r.next_dial_in_ms ? formatDuration(r.next_dial_in_ms) : "-";
      const paths = r.active_paths.map(routePathLabel).join(",") || "(none)";
      const actions = r.actions
        .map((action) => `${action.scope}:${action.kind}`)
        .join(",") || "(none)";
      return `${s.name} ${mask(r.fingerprint)} ${chip.label} binding=${r.binding} peer=${mask(r.peer) || "(none)"} seq=${r.seq} submits=${r.dial_attempts} next=${next} paths=${paths} actions=${actions} ${addresses}`;
      }),
    ];
  });
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
