/** Acceptance and local durability are separate from peer delivery receipts. */
export type SendMessageResult = {
  accepted: boolean;
  message_id?: string;
  replayed?: boolean;
  persistence:
    | { status: "durable" }
    | { status: "pending"; reason: "snapshot_failed" | "store_unavailable" | "write_failed" }
    | { status: "superseded" };
};

/**
 * What one press of Send (or Post, for an announcement) came to, for a caller that has to report
 * it: the clip maker's "post now". The composer's text cannot tell these apart: a sent message's
 * draft is kept when it was edited meanwhile, even back to the same text (its revision moved), and
 * a failed one is put back (found in review).
 * - `refused`: nothing was submitted (a guard, or the session moved on before dispatch);
 * - `failed`: not sent, and nothing will send it by itself: it sealed no identity, or its
 *   fresh identity is blocked from automatic retry by a definite refusal of its first attempt;
 *   the app shows why. An announcement that threw is `failed` too, though with no retry token it
 *   may have been posted before the error came back;
 * - `sent`: accepted and saved on this device;
 * - `uncertain`: may have gone, or will by itself (accepted without a durable save, a throw whose
 *   sealed identity stays pending and retries quietly, a throw after the request was dispatched
 *   even when Pending messages has since recovered or stopped it, or one taken over by a lock or
 *   the pending-message work). Sending it again could create a duplicate.
 */
export type SendOutcome = "refused" | "failed" | "sent" | "uncertain";

/** Classify this reply without letting a retry refusal erase a prior unknown outcome. */
export function sendOutcome(result: SendMessageResult, priorMayHaveGone = false): SendOutcome {
  if (result.accepted && result.persistence.status === "durable") return "sent";
  // Refused because the conversation moved on: never authored, and held for review with its
  // automatic retry paused. This is absence evidence for this attempt only: a lost predecessor
  // could have committed before the conversation/registry changed.
  if (!result.accepted && result.persistence.status === "superseded" && !priorMayHaveGone) return "failed";
  return "uncertain";
}

/** Only submission rejection may restore the composer. Refresh failure cannot undo acceptance. */
export async function sendAndRefresh(
  submit: () => Promise<SendMessageResult>,
  refresh: () => Promise<void>,
): Promise<{ result: SendMessageResult; refreshError?: unknown }> {
  const result = await submit();
  try {
    await refresh();
    return { result };
  } catch (refreshError) {
    return { result, refreshError };
  }
}

export function persistenceWarning(result: SendMessageResult, priorMayHaveGone = false): string | null {
  if (!result.accepted && result.persistence.status === "superseded") {
    if (priorMayHaveGone) return "Automatic retry is paused because the conversation changed. An earlier attempt may have sent this message; check the conversation before sending it again.";
    return "This pending message needs review because the conversation changed. Automatic retry is paused; move it to a draft to start a new send.";
  }
  if (!result.accepted) return "Message saved as a pending request in this vault. Its history save has not completed; it will retry automatically while unlocked.";
  switch (result.persistence.status) {
    case "durable": return null;
    case "pending":
      return "Message accepted locally, but saving to this device has not completed. Keep the app open; sending it again could create a duplicate.";
    case "superseded":
      return "Message accepted locally, but this conversation closed before its save could be confirmed. Sending it again could create a duplicate.";
  }
}
