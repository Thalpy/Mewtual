/** Pure rules for the call plane's router-mapped ICE routes. */

/** What the native side reports for one granted media-port mapping. */
export type MappedPort = { ip: string; port: number; mechanism: string; confirmed: boolean };

/** The slice of RTCIceCandidate these rules read, so tests need no RTCPeerConnection. */
export type IceCandidateView = {
  type: string | null;
  protocol: string | null;
  address: string | null;
  port: number | null;
  foundation: string | null;
  component: string | null;
  priority: number | null;
  sdpMid: string | null;
  sdpMLineIndex: number | null;
  usernameFragment: string | null;
};

/**
 * Only a host UDP candidate names a socket a router mapping would reach: srflx/relay already
 * traversed something, a TCP candidate's port is not the mapped UDP socket, and a candidate
 * with no port names nothing.
 */
export function mappableIcePort(c: Pick<IceCandidateView, "type" | "protocol" | "port">): boolean {
  return c.type === "host" && c.protocol === "udp" && !!c.port;
}

/**
 * How a host candidate's own address constrains router mapping. A mic-granted page gets real
 * IPs in its candidates, so an IPv4 literal is passed to the native side as a claim it checks
 * against the default-route interface. An mDNS-obfuscated `.local` name (or a missing address)
 * proves nothing and maps permissively: an extra dead candidate is harmless, and this is
 * deliberately NOT an active liveness probe, because Windows Firewall rejects unsolicited
 * inbound UDP to the webview with the same ICMP a dead socket produces and a probe vetoed
 * every mapping in the field. An IPv6 or hostname candidate is skipped outright: the mapped
 * socket is IPv4.
 */
export function mappingAddressPolicy(address: string | null | undefined): {
  map: boolean;
  claim: string | null;
} {
  const a = address?.trim();
  if (!a) return { map: true, claim: null };
  if (a.endsWith(".local")) return { map: true, claim: null };
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(a)) return { map: true, claim: a };
  return { map: false, claim: null };
}

/**
 * The hand-built server-reflexive candidate carrying a router-granted public socket. The
 * receiving side treats it like any other remote candidate, so old builds interoperate; its
 * connectivity checks land on the mapped port and the router forwards them from any source.
 *
 * The foundation is suffixed so ICE never treats it as redundant with the host candidate it
 * shadows; priority sits just below the host candidate's so a working direct LAN path still
 * wins; rport carries the local port, which is real (the mapping targets it) and reveals
 * nothing the host candidate didn't already.
 */
export function routerMappedCandidate(
  c: IceCandidateView,
  ext: { ip: string; port: number },
): RTCIceCandidateInit {
  const component = c.component === "rtcp" ? 2 : 1;
  const priority = Math.max(1, (c.priority ?? 1694498815) - 1);
  return {
    candidate: `candidate:${c.foundation ?? "rmap"}R ${component} udp ${priority} ${ext.ip} ${ext.port} typ srflx raddr 0.0.0.0 rport ${c.port ?? 0}`,
    sdpMid: c.sdpMid,
    sdpMLineIndex: c.sdpMLineIndex,
    usernameFragment: c.usernameFragment,
  };
}

/**
 * The call bar's one-line status. On failure it says which half of the path is missing: with a
 * router-mapped route on offer this side did its part and the block is on the peer's side;
 * without one, this side has no direct route either and the honest ask is TURN or a router
 * that maps.
 */
export function callBarStatus(
  participants: number,
  connected: number,
  anyFailed: boolean,
  mappedRoutes: number,
): string {
  if (participants === 0) return "waiting for others…";
  if (connected === participants) return `${participants} connected`;
  if (!anyFailed) return `${connected}/${participants} · connecting…`;
  return mappedRoutes > 0
    ? `${connected}/${participants} connected · direct route offered; their side may need TURN`
    : `${connected}/${participants} connected · no direct route; set a TURN server or allow router mapping`;
}
