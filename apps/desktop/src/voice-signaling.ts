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

/**
 * What a peer says about itself: the two mute states, which video slot they are filling, and the
 * jam-take coordination pair. `rec` is what THEY are doing (0 none, 1 arming/paused: asking the
 * room, 2 recording); `rc` is whether they consent to being recorded. Both are honest-client
 * signals re-sent on every heartbeat; an old build sends neither, which correctly reads as "not
 * recording, has not consented", so a mixed-version room can never start a take that build
 * cannot see.
 */
export type PeerState = {
  mic: boolean;
  inst: boolean;
  vid: number;
  rx: 720 | 1080 | 1440 | 2160;
  rec: 0 | 1 | 2;
  rc: boolean;
};

/**
 * Fold a peer's broadcast state into what we already hold for them.
 *
 * `vid` (0 none, 1 camera, 2 screen) is the field that has to survive a partial message. It is
 * announced once, on the data channel, when the video starts, while `mic` and `inst` are re-sent
 * by every five-second room heartbeat. Reading an ABSENT `vid` as "no video" therefore let the
 * next heartbeat retract a screen share that was still sending frames: the sender saw its own
 * live preview and believed it was sharing, while every viewer had already torn the picture down.
 * Absence is not a claim. Only a number in the message changes what we believe.
 */
export function mergePeerState(
  prev: PeerState | undefined,
  msg: { mic?: unknown; inst?: unknown; vid?: unknown; rx?: unknown; rec?: unknown; rc?: unknown },
): PeerState {
  const receiveHeight =
    msg.rx === 720 || msg.rx === 1080 || msg.rx === 1440 || msg.rx === 2160
      ? msg.rx
      : prev?.rx ?? 1080;
  return {
    mic: msg.mic === 1,
    inst: msg.inst === 1,
    vid: typeof msg.vid === "number" ? msg.vid : prev?.vid ?? 0,
    rx: receiveHeight,
    // Like mic: freshly claimed by every message, so absence (an old build, or a build that
    // stopped sending it) reads as the safe zero, never as a sticky earlier claim.
    rec: msg.rec === 1 ? 1 : msg.rec === 2 ? 2 : 0,
    rc: msg.rc === 1,
  };
}

/** The two things a video slot can be carrying. They differ in what they cost on the wire. */
export type VideoKind = "cam" | "screen";

/**
 * What one peer's video may spend, per second.
 *
 * A screen share carries text and edges, which fall apart at a bitrate a face survives, so the
 * two are not interchangeable. The cap has to be re-applied every time the slot changes hands:
 * cam and screen swap through one sender, and a screen share left on the camera's budget arrives
 * as mush. Both numbers are per PEER, and in a mesh every peer gets its own encode.
 */
export const VIDEO_BITRATE: Readonly<Record<VideoKind, number>> = {
  cam: 500_000,
  screen: 1_200_000,
};

/** Every direction an RTP transceiver can be in, including the terminal one. */
export type SlotDirection = "sendrecv" | "sendonly" | "recvonly" | "inactive" | "stopped";

/**
 * Open the send half of a video slot, leaving the receive half exactly as it is. `null` means
 * "already right, do not touch it": assigning a direction that has not changed is wasted, and on
 * a stopped transceiver it throws.
 */
export function directionSending(current: SlotDirection): SlotDirection | null {
  if (current === "recvonly") return "sendrecv";
  if (current === "inactive") return "sendonly";
  return null; // already sending, or stopped and past reviving
}

/**
 * Close the send half, leaving the receive half alone. Dropping to recvonly/inactive is what
 * tells the far end the track is gone, so their tile clears instead of freezing on a last frame.
 */
export function directionIdle(current: SlotDirection): SlotDirection | null {
  if (current === "sendrecv") return "recvonly";
  if (current === "sendonly") return "inactive";
  return null; // already not sending, or stopped
}

/**
 * How to fill this peer's one video slot when a camera or screen share starts.
 *
 * There is deliberately only ever ONE video m-line per peer: cam and screen swap through the same
 * sender, so only the first video a peer ever sends renegotiates. Stopping a video used to call
 * removeTrack, which retires the transceiver for good, because addTrack will not reuse one that
 * has sent before. Every stop/start cycle therefore bolted another video section onto the SDP,
 * and the sender it left behind no longer matched the "has a video track" test that was supposed
 * to find it. Stopping now parks the slot instead; this decides whether a parked one can be
 * revived, and what direction it has to go back to.
 */
export function videoSlotPlan(slot: { hasSender: boolean; direction: SlotDirection }): {
  action: "add" | "reuse";
  direction: SlotDirection | null;
} {
  if (!slot.hasSender || slot.direction === "stopped") return { action: "add", direction: null };
  return { action: "reuse", direction: directionSending(slot.direction) };
}

export const MAX_BUFFERED_ICE_PER_PEER = 64;

/** Keep trickled ICE bounded while SDP is still in flight; retain the newest candidates. */
export function bufferIce<T>(queued: readonly T[], candidate: T): T[] {
  return [...queued, candidate].slice(-MAX_BUFFERED_ICE_PER_PEER);
}
