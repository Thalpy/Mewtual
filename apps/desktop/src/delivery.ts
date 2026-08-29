/**
 * What a delivery tick is allowed to claim (docs/design-delivery-states.md).
 *
 * There is no server to acknowledge a message. "Delivered to X" means X sent an authenticated
 * receipt for the exact change or authored a causally-descending change. Both are sound positive
 * evidence within the current roster; counts may fall when that roster changes. This must never
 * invent a negative: a red
 * "nobody is reachable" is a claim about the network, and silence is not that claim.
 *
 * Pure, so the honesty rules are testable without a webview.
 */

export type DeliveryVerdict =
  /** Still being written locally; it has not left this device. */
  | "pending"
  /** Sent, with nobody having proved they hold it yet. */
  | "waiting"
  /** At least one other member has proved it holds the message, but not the whole roster. */
  | "partial"
  /** Every other member of the roster has proved they hold it. */
  | "everyone"
  /** Nothing to send to, and nobody has it: it waits in the local doc for a peer to appear. */
  | "queued";

export type DeliveryEvidence = {
  /** Members other than me on the roster. */
  others: number;
  /**
   * Peers that have provably built on this message, or `null` when nothing has been reported for
   * it at all. The two are different: zero is a measurement, null is a missing one.
   */
  delivered: number | null;
  /** Peers reachable at the last report, or `null` when there has been no report. */
  reachable: number | null;
  /**
   * Whether the node had ANY transport peer connected at the last report; `null` when there has
   * been no report.
   *
   * This, not `reachable`, is what a red tick is allowed to rest on. `reachable` resolves live
   * connections to member fingerprints through signed peer records, so a peer whose record has
   * not arrived yet is absent from that count while ops gossip to it perfectly well; keying the
   * failure state off it announced a delivery problem on messages that had already been received.
   */
  anyPeer: boolean | null;
  /** The message has not been acknowledged by the local actor yet. */
  pending: boolean;
  /** This is my newest message in the log. */
  latest: boolean;
};

/**
 * The state a message is actually in, or `null` for "say nothing".
 *
 * Silence is a real answer and the right one twice over. Alone in a group there is nobody to
 * deliver to; and the actor tracks only its most recent own messages per channel and forgets them
 * on restart, so an older message has no evidence either way. Painting those "sending…" forever
 * would be a false claim about a message that arrived days ago.
 */
export function deliveryVerdict(e: DeliveryEvidence): DeliveryVerdict | null {
  if (e.pending) return "pending";
  if (e.others <= 0) return null; // alone here: nothing to deliver to
  if (e.delivered === null) {
    // Nothing reported for this message. Only the newest one is plausibly still in flight; for
    // anything older this is a forgotten record, not a stuck send.
    return e.latest ? "waiting" : null;
  }
  if (e.delivered >= e.others) return "everyone";
  if (e.delivered > 0) {
    // Evidence of arrival can never be overridden by a reading of the live network. A peer that
    // confirmed and then went offline still holds the message; this is what painted a delivered
    // message red every time a connection flapped.
    return "partial";
  }
  // Nobody holds it yet. Red needs a measurement saying the message cannot leave at all, and
  // "no member resolved to a live connection" is not that measurement.
  if (e.anyPeer === false) return "queued";
  return "waiting";
}

/** The gutter glyph for a verdict. */
export function deliveryGlyph(v: DeliveryVerdict): string {
  switch (v) {
    case "pending":
    case "waiting":
      return "◌";
    case "partial":
      return "~";
    case "everyone":
      return "✓✓";
    case "queued":
      return "✕";
  }
}

/** The CSS class for a verdict; the colour is the only other thing the tick carries. */
export function deliveryClass(v: DeliveryVerdict): string {
  switch (v) {
    case "pending":
      return "d-pending";
    case "waiting":
      return "d-wait";
    case "partial":
      return "d-part";
    case "everyone":
      return "d-all";
    case "queued":
      return "d-none";
  }
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** The spelled-out receipt, under the newest own message. */
export function deliveryLabel(v: DeliveryVerdict, e: DeliveryEvidence): string {
  const delivered = e.delivered ?? 0;
  switch (v) {
    case "pending":
      return "saving…";
    case "waiting":
      // The local write is complete. A receipt is best-effort over a live route, so calling this
      // state "sending" would still make a completed local send look stuck during disconnection.
      return "sent · awaiting confirmation";
    case "partial":
      // We have holder identities and a separate reachable count, but not their intersection.
      // Comparing the cardinalities would let an offline holder stand in for an unconfirmed live
      // peer, so this claim deliberately says only what the positive evidence proves.
      return `held by ${plural(delivered, "peer")}`;
    case "everyone":
      return `delivered · all ${plural(e.others, "member")}`;
    case "queued":
      return "queued · no peers reachable";
  }
}

/** The hover explanation. Factual: what was proved, never "read". */
export function deliveryTip(v: DeliveryVerdict, e: DeliveryEvidence): string {
  const delivered = e.delivered ?? 0;
  switch (v) {
    case "pending":
      return "Saving this message locally…";
    case "waiting":
      return e.reachable === null
        ? "Sent. No authenticated delivery receipt or later causal proof has arrived yet."
        : `Sent: no confirmations yet from ${plural(e.reachable, "reachable member")}. Receipts are best-effort; confirmations can change when the current roster changes.`;
    case "partial":
      return `Held by ${plural(delivered, "other member")}, of ${e.others}. Confirmation is an authenticated receipt or later causal proof; it does not identify whether those holders are connected now.`;
    case "everyone":
      return `Delivered to everyone: all ${plural(e.others, "other member")} proved they hold this message.`;
    case "queued":
      return "No peers reachable: queued; it gossips automatically when members reconnect. Not lost.";
  }
}

/**
 * Replace the previous delivery report with the actor's current-roster snapshot.
 *
 * Receipt evidence is durable inside one roster, but the backend deliberately filters holders
 * through the current roster. Keeping a numeric maximum here would let a removed member's count
 * stand in for a different current member and could falsely produce "everyone". Identity-keyed
 * evidence (or a roster revision) would be needed before this count could safely be monotonic.
 */
export function mergeDelivery(
  _previous: { delivered: number; reachable: number; any_peer: boolean } | undefined,
  next: { delivered: number; reachable: number; any_peer: boolean },
): { delivered: number; reachable: number; any_peer: boolean } {
  return { ...next };
}

export type DeliveryReport = {
  id: string;
  delivered: number;
  reachable: number;
  any_peer: boolean;
};

export type DeliverySnapshot = {
  revision: number;
  states: readonly DeliveryReport[];
};

export type DeliverySnapshotView = {
  revision: number;
  reports: Readonly<Record<string, DeliveryReport>>;
};

/**
 * Reconcile a complete actor snapshot without allowing a delayed query to overwrite a newer
 * event. Revisions are issued by the sole-owner Rust actor, so they order both IPC paths.
 */
export function replaceDeliverySnapshot(
  previous: DeliverySnapshotView,
  snapshot: DeliverySnapshot,
): DeliverySnapshotView {
  if (snapshot.revision <= previous.revision) return previous;
  const next: Record<string, DeliveryReport> = {};
  for (const state of snapshot.states) {
    next[state.id] = { id: state.id, ...mergeDelivery(undefined, state) };
  }
  return { revision: snapshot.revision, reports: next };
}

/**
 * Clear an unavailable actor's rows only if no newer query/event landed while the failed request
 * was in flight. Keep the accepted revision so a delayed older event cannot repopulate the view.
 */
export function clearDeliverySnapshotAfterFailedQuery(
  current: DeliverySnapshotView,
  requestedAtRevision: number,
): DeliverySnapshotView {
  if (current.revision !== requestedAtRevision) return current;
  return { revision: current.revision, reports: {} };
}
