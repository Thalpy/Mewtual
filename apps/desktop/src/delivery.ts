/**
 * What a delivery tick is allowed to claim (docs/design-delivery-states.md).
 *
 * There is no server to acknowledge a message. "Delivered to X" can only mean "X provably built on
 * the op", which the sync layer already knows, so every state here is evidence-based and the counts
 * only rise. The one thing this must never do is invent a negative: a red "nobody is reachable" is
 * a claim about the network, and not having heard anything yet is not that claim.
 *
 * Pure, so the honesty rules are testable without a webview.
 */

export type DeliveryVerdict =
  /** Still being written locally; it has not left this device. */
  | "pending"
  /** Sent, with nobody having proved they hold it yet. */
  | "waiting"
  /** Some of the reachable members have proved they hold it. */
  | "partial"
  /** Every currently-reachable member has proved they hold it. */
  | "reachable"
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
    if (e.reachable !== null && e.reachable > 0 && e.delivered >= e.reachable) return "reachable";
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
    case "reachable":
      return "✓";
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
    case "reachable":
      return "d-ok";
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
    case "waiting":
      return "sending…";
    case "partial":
      // Phrased against the roster when nothing is reachable, because "1 of 0 reachable" is not a
      // sentence. The message is held by somebody either way, which is the part that matters.
      return e.reachable && e.reachable > 0
        ? `delivering · ${delivered}/${e.reachable} peers`
        : `held by ${plural(delivered, "peer")}`;
    case "reachable":
      return `delivered · ${plural(delivered, "peer")}`;
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
        ? "Sent. No delivery report yet; confirmations arrive as members build on the message."
        : `Sent: no confirmations yet from ${plural(e.reachable, "reachable member")}. Silent receipt isn't visible; the count only rises.`;
    case "partial":
      return e.reachable && e.reachable > 0
        ? `Delivering: ${delivered} of ${e.reachable} reachable confirmed (${e.others} members in total). Members confirm by building on the message.`
        : `Held by ${plural(delivered, "member")}, of ${e.others}. Nobody is reachable right now, so the rest catch up when they reconnect.`;
    case "reachable":
      return `Delivered to all ${plural(e.reachable ?? 0, "reachable member")} (${delivered}/${e.others} confirmed overall). Confirmation is proof-based: silent receivers may also have it.`;
    case "everyone":
      return `Delivered to everyone: all ${plural(e.others, "other member")} proved they hold this message.`;
    case "queued":
      return "No peers reachable: queued; it gossips automatically when members reconnect. Not lost.";
  }
}

/**
 * Merge a delivery report into what is already known, keeping the proved count from regressing.
 *
 * Confirmations are evidence: once a peer has built on a message it has it, and a later report
 * that happens to see fewer holders (a reset sync record, a peer that dropped) has not unproved
 * anything. Reachability is genuinely live and is taken as reported.
 */
export function mergeDelivery(
  previous: { delivered: number; reachable: number; any_peer: boolean } | undefined,
  next: { delivered: number; reachable: number; any_peer: boolean },
): { delivered: number; reachable: number; any_peer: boolean } {
  return {
    delivered: Math.max(previous?.delivered ?? 0, next.delivered),
    reachable: next.reachable,
    any_peer: next.any_peer,
  };
}
