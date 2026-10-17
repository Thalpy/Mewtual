// draw-wiring.ts: how one peer connection's `draw` data channel is bound to the call's claim
// session (Gate 5). App.svelte and the browser WebRTC check (scripts/claims-webrtc-check.mjs)
// both use exactly this, so the event wiring the check exercises over real data channels is the
// wiring the app ships, not a copy of it.

import {
  DRAW_CHANNEL_ID,
  DRAW_CHANNEL_LABEL,
  type DrawChannel,
  type DrawClaimsSession,
} from "./draw-claims.ts";

/** The parts of RTCDataChannel this wiring touches (a real channel in the app, a fake in tests). */
export type DrawDataChannel = {
  readonly readyState: string;
  send(data: string): void;
  close(): void;
  onopen: ((ev: Event) => unknown) | null;
  onmessage: ((ev: MessageEvent) => unknown) | null;
  onclose: ((ev: Event) => unknown) | null;
};

export type DrawPeerConnection = {
  createDataChannel(label: string, init: RTCDataChannelInit): DrawDataChannel;
};

export type DrawEdge = Readonly<{ dc: DrawDataChannel; channel: DrawChannel }>;

export type DrawEdgeHooks = Readonly<{
  /** False once this peer generation was replaced or the call ended. */
  live: () => boolean;
  /** The session changed something a view shows: rebuild the Studio snapshot. */
  changed: () => void;
}>;

/**
 * Create the negotiated, ordered `draw` channel (id 8) on `pc` and mint its capability for the
 * authenticated `peer`, synchronously, before the first offer, so nothing renegotiates and no
 * callback can run first. Every callback closes over this exact session and capability, so one
 * from a replaced edge or an ended call changes nothing. An older build has no channel 8: our
 * frames are discarded there and it sends none. Null when no channel could be made.
 */
export function openDrawEdge(
  pc: DrawPeerConnection,
  claims: DrawClaimsSession,
  peer: string,
  hooks: DrawEdgeHooks,
): DrawEdge | null {
  let dc: DrawDataChannel;
  try {
    dc = pc.createDataChannel(DRAW_CHANNEL_LABEL, { negotiated: true, id: DRAW_CHANNEL_ID, ordered: true });
  } catch {
    return null; // no draw channel on this edge: claims simply do not flow to or from it
  }
  // `true` only when the frame was queued: a closing channel refuses, and a full send buffer throws
  // OperationError. Either way the session keeps the frame owed to this edge and retries it on the
  // next tick, so an Ask or a Pass is never silently lost.
  const channel = claims.openEdge(peer, (raw) => {
    if (dc.readyState !== "open") return false;
    dc.send(raw);
    return true;
  });
  if (!channel) {
    dc.close();
    return null;
  }
  // Each callback rebuilds the snapshot only when the session changed something visible: a refused
  // or no-op frame (a flood, an auto-muted peer) costs no view work.
  const run = (op: () => unknown) => {
    const before = claims.rev;
    op();
    if (claims.rev !== before) hooks.changed();
  };
  dc.onopen = () => {
    if (hooks.live()) run(() => claims.edgeReady(channel));
  };
  dc.onmessage = (e) => run(() => claims.receive(channel, e.data));
  dc.onclose = () => run(() => claims.closeEdge(channel));
  return { dc, channel };
}

/**
 * The peer connection's state moved. Claims never outlive a dead link: a peer whose connection
 * leaves "connected" stops showing as drawing at once, and what it still sends is held, not shown,
 * until this side sees the link "connected" again. Recovery shows its claims that are still live,
 * applies what was held (so a Pass sent during the gap lands), and tells it whatever it missed
 * from me. A link that never recovers stays that way (the app restarts ICE on the same connection)
 * until the peer is removed or the call ends.
 */
export function drawConnectionState(
  claims: DrawClaimsSession,
  peer: string,
  state: RTCPeerConnectionState,
  edge: DrawEdge | null | undefined,
  changed: () => void,
): void {
  const before = claims.rev;
  if (state === "connected") {
    if (edge && edge.dc.readyState === "open") claims.edgeReady(edge.channel);
  } else if (state !== "new" && state !== "connecting") {
    claims.peerUnreachable(peer);
  }
  if (claims.rev !== before) changed();
}
