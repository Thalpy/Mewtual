// The server operator's join log.
//
// Pure formatting over what the bridge hands up (`get_join_attempts`), so the copy a user reads
// (and the plain text they paste into a conversation to get help) can be tested without a network,
// a webview or a running server.
//
// The backend deliberately sends stable outcome **ids** and no prose: the wire answer to a
// rejected joiner stays opaque, and the sentence an operator reads belongs with the rest of the
// app's copy, here.

/// One inbound join attempt, exactly as `get_join_attempts` serialises it.
export type JoinAttempt = {
  /// Milliseconds since the epoch, on the serving node's clock.
  at: number;
  /// The stable outcome id.
  outcome: string;
  /// Whether the joiner got in (or is on their way in).
  admitted: boolean;
  /// The requesting peer, as a short hex prefix.
  peer: string;
  /// The invite nonce prefix: what an operator matches against the invite they sent.
  nonce: string;
};

/// What an operator is told about one outcome: a short label for the list, and the next action.
export type OutcomeCopy = {
  /// The list label, kept short enough to sit in a narrow column.
  label: string;
  /// What it means and what to do about it. This is the whole point of the surface.
  note: string;
  /// Rendering class: a success, a plain refusal, or something worth a second look.
  tone: "ok" | "refused" | "alarm";
};

/// The copy for every outcome the backend can emit. Ids are stable across releases; a new one
/// appearing here before the frontend knows it is handled by `describeOutcome`.
export const OUTCOME_COPY: Record<string, OutcomeCopy> = {
  admitted: {
    label: "admitted",
    note: "Joined. They are a member of this server now.",
    tone: "ok",
  },
  relayed: {
    label: "relayed to owner",
    note:
      "You minted this invite as an admin, so the join was passed to the owner to commit. It completes when the owner is next online.",
    tone: "ok",
  },
  staged: {
    label: "staged",
    note: "The admission is waiting on the commit contest to resolve, then the joiner is let in.",
    tone: "ok",
  },
  undecodable: {
    label: "not a join request",
    note:
      "Something reached this node that was not a join request at all. Nothing to do: an unrelated peer, or a scanner.",
    tone: "refused",
  },
  "wrong-group": {
    label: "wrong server",
    note:
      "The invite is for a different server. They probably pasted an invite from somewhere else.",
    tone: "refused",
  },
  "not-this-inviter": {
    label: "wrong device",
    note:
      "The invite names another device as the inviter, and only that device can admit. Whoever sent the invite has to be online, or send one of yours instead.",
    tone: "refused",
  },
  "bad-signature": {
    label: "invite altered",
    note:
      "The invite failed its own signature check: it was edited in transit, truncated on the way through a chat app, or forged. Send a fresh one.",
    tone: "alarm",
  },
  expired: {
    label: "expired",
    note: "The invite had run out. Mint a new one and send that.",
    tone: "refused",
  },
  revoked: {
    label: "revoked",
    note: "This invite was revoked before it was used. Mint a new one if you still want them in.",
    tone: "refused",
  },
  "already-used": {
    label: "already used",
    note:
      "Invites are single-use and this one had already been redeemed. Mint a second invite; one invite cannot admit two people (or the same person twice, e.g. after a reinstall).",
    tone: "refused",
  },
  "not-authorized": {
    label: "not an admin",
    note:
      "This device minted the invite but is not an admin here any more, so it refused to relay rather than leave the joiner waiting forever. Ask the owner to invite them.",
    tone: "refused",
  },
  "admission-failed": {
    label: "admission failed",
    note:
      "Every check passed and the admission itself still failed. This one is a bug or a malformed client: turn on the debug log in Settings, reproduce, and share the file.",
    tone: "alarm",
  },
};

/// The copy for an outcome id, with a safe fallback so a backend that gains a new outcome shows
/// the id rather than an empty row.
export function describeOutcome(id: string): OutcomeCopy {
  return (
    OUTCOME_COPY[id] ?? {
      label: id || "unknown",
      note: "This build does not have copy for that outcome yet.",
      tone: "refused",
    }
  );
}

/// A deterministic UTC stamp for pasted text. Local time is right on screen and wrong in a
/// paste: the person being asked for help is usually in another timezone.
function stamp(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "unknown time";
  return new Date(ms).toISOString().replace("T", " ").replace(/\.\d+Z$/, "Z");
}

/// The join log as plain text, for pasting into a conversation.
///
/// Every line carries the three things a second pair of eyes needs: when, what happened, and
/// which invite it was about.
export function formatJoinLog(attempts: JoinAttempt[]): string {
  if (!attempts.length) {
    return "Mewtual join log: no inbound join attempts recorded this session.";
  }
  const lines = attempts.map((a) => {
    const c = describeOutcome(a.outcome);
    const nonce = a.nonce ? `invite ${a.nonce}` : "invite unknown";
    const peer = a.peer ? `peer ${a.peer}` : "peer unknown";
    return `${stamp(a.at)}  ${c.label.padEnd(18)}  ${nonce}  ${peer}`;
  });
  return [
    `Mewtual join log (${attempts.length} attempt${attempts.length === 1 ? "" : "s"}, newest first)`,
    ...lines,
  ].join("\n");
}

// Preserve the existing frontend import surface while connectivity callers migrate to their
// dedicated module incrementally.
export {
  type Connectivity,
  type ConnectivityStatus,
  type DiagStep,
  automaticMappingUnavailable,
  connectivityReadout,
  connectivityStatus,
  formatConnectivity,
  reachabilityEventAffectsReport,
  reachabilitySummary,
  switchboardEventRefreshDecision,
  withOrderedConnectivity,
  withOrderedRefreshedInvite,
  withRefreshedInvite,
} from "./connectivity.ts";
