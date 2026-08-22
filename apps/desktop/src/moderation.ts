export type ModerationEvent = {
  id: string;
  kind: "warning" | "kick_case" | "case_resolution";
  actor: string;
  signer: string;
  target: string;
  channel: string;
  message_id: string;
  message_text: string;
  message_ts: number;
  reason: string;
  evidence_ids: string[];
  case_id: string;
  outcome: string;
  ts: number;
  signature_valid: boolean;
  authorized: boolean;
};

export type ModerationVote = {
  case_id: string;
  voter: string;
  signer: string;
  yes: boolean;
  ts: number;
  signature_valid: boolean;
  eligible: boolean;
};

export type ModerationState = { events: ModerationEvent[]; votes: ModerationVote[] };

export type TimelineMessage = {
  id: string;
  author: string;
  text: string;
  ts: number;
  channel: string;
  channelName: string;
};

export type TimelineRow =
  | { key: string; ts: number; kind: "message"; message: TimelineMessage }
  | { key: string; ts: number; kind: "event"; event: ModerationEvent };

export type ModerationGraphLane = { identity: string; y: number };
export type ModerationGraphNode = {
  key: string;
  ts: number;
  x: number;
  y: number;
  fromY: number;
  kind: "message" | ModerationEvent["kind"];
};
export type ModerationGraph = {
  width: number;
  height: number;
  lanes: ModerationGraphLane[];
  nodes: ModerationGraphNode[];
};

/** Only authenticated, currently-authorized warnings affect ordinary chat presentation. */
export function warningMap(events: ModerationEvent[]): Map<string, ModerationEvent> {
  const out = new Map<string, ModerationEvent>();
  for (const event of events) {
    if (event.kind !== "warning" || !event.signature_valid || !event.authorized) continue;
    const key = `${event.channel}:${event.message_id}`;
    const previous = out.get(key);
    if (!previous || event.ts > previous.ts || (event.ts === previous.ts && event.id > previous.id)) {
      out.set(key, event);
    }
  }
  return out;
}

/** Stable, oldest-first audit timeline over messages from every channel and signed mod events. */
export function buildModerationTimeline(
  messages: TimelineMessage[],
  events: ModerationEvent[],
): TimelineRow[] {
  const rows: TimelineRow[] = [
    ...messages.map((message): TimelineRow => ({
      key: `m:${message.channel}:${message.id}`,
      ts: message.ts,
      kind: "message",
      message,
    })),
    ...events.map((event): TimelineRow => ({
      key: `e:${event.id}`,
      ts: event.ts,
      kind: "event",
      event,
    })),
  ];
  return rows.sort((a, b) => a.ts - b.ts || a.key.localeCompare(b.key));
}

/** The identities a row concerns. Events can connect the moderator lane to the subject lane. */
export function timelineIdentities(row: TimelineRow): string[] {
  if (row.kind === "message") return row.message.author ? [row.message.author] : [];
  return [...new Set([row.event.actor, row.event.target].filter(Boolean))];
}

/** Keep every message authored by, or moderation event acting on/by, one selected member. */
export function filterModerationTimeline(rows: TimelineRow[], identity: string): TimelineRow[] {
  if (!identity) return rows;
  return rows.filter((row) => timelineIdentities(row).includes(identity));
}

/**
 * Lay recent activity onto stable per-user lanes. The graph is deliberately a pure projection:
 * it invents no events and carries the same row keys as the detailed audit list below it.
 */
export function buildModerationGraph(
  rows: TimelineRow[],
  width = 960,
  maxRows = 120,
): ModerationGraph {
  const visible = rows.slice(-Math.max(1, maxRows));
  const identities = [...new Set(visible.flatMap(timelineIdentities))].sort();
  const laneTop = 28;
  const laneGap = 34;
  const plotLeft = 150;
  const plotRight = Math.max(plotLeft + 1, width - 24);
  const start = visible[0]?.ts ?? 0;
  const end = visible.at(-1)?.ts ?? start;
  const span = Math.max(1, end - start);
  const laneY = new Map(identities.map((identity, index) => [identity, laneTop + index * laneGap]));
  const lanes = identities.map((identity) => ({ identity, y: laneY.get(identity)! }));
  const nodes = visible.map((row): ModerationGraphNode => {
    const identitiesForRow = timelineIdentities(row);
    const primary = row.kind === "message"
      ? row.message.author
      : row.event.target || row.event.actor;
    const actor = row.kind === "event" ? row.event.actor : primary;
    return {
      key: row.key,
      ts: row.ts,
      x: plotLeft + ((row.ts - start) / span) * (plotRight - plotLeft),
      y: laneY.get(primary) ?? laneTop,
      fromY: laneY.get(actor || identitiesForRow[0]) ?? laneTop,
      kind: row.kind === "message" ? "message" : row.event.kind,
    };
  });
  return {
    width,
    height: Math.max(86, laneTop * 2 + Math.max(0, identities.length - 1) * laneGap),
    lanes,
    nodes,
  };
}

/** Desktop-like click/shift-click selection, shared with the timeline UI and unit tests. */
export function selectTimelineRows(
  orderedKeys: string[],
  current: ReadonlySet<string>,
  key: string,
  anchor: string,
  extend: boolean,
): { selected: Set<string>; anchor: string } {
  if (!extend || !anchor) {
    const selected = new Set(current);
    if (selected.has(key)) selected.delete(key);
    else selected.add(key);
    return { selected, anchor: key };
  }
  const from = orderedKeys.indexOf(anchor);
  const to = orderedKeys.indexOf(key);
  if (from < 0 || to < 0) return { selected: new Set(current), anchor };
  const selected = new Set(current);
  for (const item of orderedKeys.slice(Math.min(from, to), Math.max(from, to) + 1)) selected.add(item);
  return { selected, anchor };
}

export function openKickCases(events: ModerationEvent[]): ModerationEvent[] {
  const resolved = new Set(
    events
      .filter((event) => event.kind === "case_resolution" && event.signature_valid && event.authorized)
      .map((event) => event.case_id),
  );
  return events.filter(
    (event) =>
      event.kind === "kick_case" &&
      event.signature_valid &&
      event.authorized &&
      !resolved.has(event.id),
  );
}

export function voteTally(votes: ModerationVote[], caseId: string): { yes: number; no: number } {
  const latest = new Map<string, ModerationVote>();
  for (const vote of votes) {
    if (vote.case_id !== caseId || !vote.signature_valid || !vote.eligible) continue;
    const old = latest.get(vote.voter);
    if (!old || vote.ts > old.ts || (vote.ts === old.ts && vote.signer > old.signer)) {
      latest.set(vote.voter, vote);
    }
  }
  let yes = 0;
  let no = 0;
  for (const vote of latest.values()) vote.yes ? yes++ : no++;
  return { yes, no };
}
