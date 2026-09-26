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

export function persistenceWarning(result: SendMessageResult): string | null {
  if (!result.accepted) return "Message saved as a pending request in this vault. Its history save has not completed; use Retry pending messages to try again safely.";
  switch (result.persistence.status) {
    case "durable": return null;
    case "pending":
      return "Message accepted locally, but saving to this device has not completed. Keep the app open; sending it again could create a duplicate.";
    case "superseded":
      return "Message accepted locally, but this conversation closed before its save could be confirmed. Sending it again could create a duplicate.";
  }
}
