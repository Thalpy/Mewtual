/** Pure UI rules for the native two-way join-reply lifecycle. */

export const JOIN_REPLY_PREFIX = "mewtual-reply-v1:";
export const JOIN_REPLY_REPLACEMENT_ERROR =
  "a different joiner is already using this invite's reply window; confirm replacement only if you intended to switch people";

/** Only the backend's explicit key-conflict result may reveal the destructive replacement action. */
export function joinReplyNeedsReplacement(error: unknown): boolean {
  return String(error) === JOIN_REPLY_REPLACEMENT_ERROR;
}

export function joinReplyCandidateLabel(count: number): string {
  const safe = Number.isFinite(count) ? Math.max(0, Math.trunc(count)) : 0;
  return `${safe} direct listener route${safe === 1 ? "" : "s"}`;
}

export function joinReplyIsExpired(expiresAtMs: number, nowMs: number): boolean {
  return !Number.isFinite(expiresAtMs) || nowMs >= expiresAtMs;
}

export type AssistedJoinAction = "preview" | "direct" | "switchboard";

/** The first action on an assisted code can only reveal consent; it cannot dial a helper. */
export function assistedJoinAction(
  previewMatchesCode: boolean,
  switchboards: number,
  consented: boolean,
): AssistedJoinAction {
  if (!previewMatchesCode && switchboards > 0) return "preview";
  return consented && switchboards > 0 ? "switchboard" : "direct";
}

/** Ignore an older same-server status completion and every completion for a switched-away view. */
export function withOrderedSwitchboardStatus<T>(
  current: T,
  activeServer: number | null,
  requestedServer: number,
  refreshed: T,
  generation: number,
  latestGeneration: number,
): T {
  return activeServer === requestedServer && generation === latestGeneration
    ? refreshed
    : current;
}
