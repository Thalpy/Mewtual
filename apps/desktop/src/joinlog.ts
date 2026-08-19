// The server operator's join log, and the connectivity report on the create/join screens.
//
// Pure formatting over what the bridge hands up (`get_join_attempts`, `get_connectivity`), so
// the copy a user reads (and the plain text they paste into a conversation to get help) can be
// tested without a network, a webview or a running server.
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

/// One step of a founding/joining attempt, as `get_connectivity` serialises it.
export type DiagStep = {
  at: number;
  kind: string;
  target: string;
  detail: string;
  /// "ok" | "failed" | "unknown".
  status: string;
};

/// What `get_connectivity` returns.
export type Connectivity = {
  action: string;
  subject: string;
  at: number;
  server: number;
  advertised: string[];
  upnp: string;
  steps: DiagStep[];
  last_error: string;
};

/// The honest answer to "am I reachable from the internet".
///
/// AutoNAT is not implemented (`docs/design-zeroconf-reachability.md`, rung 0c), so nothing in
/// this app can have a peer dial it back, and no amount of local information settles the
/// question. A public address from UPnP or a reserved relay circuit is evidence and is reported
/// as evidence; it is never reported as a verdict.
export function reachabilitySummary(c: Connectivity | null): {
  verdict: string;
  detail: string;
} {
  const upnp = c?.upnp ?? "";
  const relayed = (c?.advertised ?? []).some((a) => a.includes("p2p-circuit"));
  const publicUpnp = upnp.startsWith("/");
  if (relayed) {
    return {
      verdict: "reachable through a relay",
      detail:
        "A relay circuit is reserved, so joiners can reach this node through the relay even behind NAT. Whether they can also reach it directly is untested.",
    };
  }
  if (publicUpnp) {
    return {
      verdict: "probably reachable directly",
      detail: `Your router opened a port and reported ${upnp}. That is good evidence, not proof: nothing here can dial this machine from outside to check.`,
    };
  }
  return {
    verdict: "unknown",
    detail:
      "This app cannot test whether it is reachable from the internet: that needs a peer willing to dial back (AutoNAT), which is not implemented yet. No public address was obtained, so a joiner on another network may not be able to reach you; a relay or a rendezvous address is the way around that.",
  };
}

/// The connectivity report as plain text, for pasting.
export function formatConnectivity(c: Connectivity | null): string {
  if (!c || !c.action) {
    return "Mewtual connectivity: nothing has been founded or joined this session.";
  }
  const out: string[] = [];
  out.push(
    `Mewtual connectivity report (${c.action}${c.subject ? ` ${c.subject}` : ""}, ${stamp(c.at)})`,
  );
  const reach = reachabilitySummary(c);
  out.push(`Reachable from the internet: ${reach.verdict}`);
  out.push(`  ${reach.detail}`);
  out.push(`UPnP: ${c.upnp || "not attempted"}`);
  out.push(
    c.advertised.length
      ? `Addresses this node advertises (${c.advertised.length}):`
      : "Addresses this node advertises: none",
  );
  for (const a of c.advertised) out.push(`  ${a}`);
  out.push(c.steps.length ? "What the attempt did:" : "What the attempt did: nothing recorded");
  for (const s of c.steps) {
    const target = s.target ? ` ${s.target}` : "";
    out.push(`  [${s.status}] ${s.kind}${target}: ${s.detail}`);
  }
  out.push(c.last_error ? `Last error (verbatim): ${c.last_error}` : "Last error: none");
  return out.join("\n");
}
