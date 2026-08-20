export type PeerRecovery = "create" | "resend-offer" | "resend-answer" | "restart-ice" | null;

/** A channel id is not globally unique: every server can have (for example) `general`. */
export function isCurrentVoiceRoom(
  callServer: number | null,
  callChannel: string,
  signalServer: number,
  signalChannel: string,
): boolean {
  return callServer !== null && callServer === signalServer && !!callChannel && callChannel === signalChannel;
}

/**
 * Decide how a room heartbeat should repair an incomplete peer edge. Heartbeats are durable
 * reconciliation, not presence-only decoration: the one-shot hello or any SDP reply may be lost.
 */
export function heartbeatRecovery(input: {
  currentRoom: boolean;
  hasPeer: boolean;
  connectionState?: string;
  signalingState?: string;
  localDescriptionType?: string;
}): PeerRecovery {
  if (!input.currentRoom) return null;
  if (!input.hasPeer) return "create";
  if (input.connectionState === "connected") return null;
  if (input.connectionState === "failed" || input.connectionState === "disconnected") {
    return "restart-ice";
  }
  if (input.signalingState === "have-local-offer" && input.localDescriptionType === "offer") {
    return "resend-offer";
  }
  if (input.signalingState === "stable" && input.localDescriptionType === "answer") {
    return "resend-answer";
  }
  return null;
}

export const MAX_BUFFERED_ICE_PER_PEER = 64;

/** Keep trickled ICE bounded while SDP is still in flight; retain the newest candidates. */
export function bufferIce<T>(queued: readonly T[], candidate: T): T[] {
  return [...queued, candidate].slice(-MAX_BUFFERED_ICE_PER_PEER);
}
