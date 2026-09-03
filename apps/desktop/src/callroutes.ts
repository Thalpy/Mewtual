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

/** Private IPv4 space (RFC 1918) plus the link-local block, as a candidate address. */
function isPrivateIpv4(a: string): boolean {
  const parts = a.split(".").map(Number);
  if (parts.length !== 4 || parts.some((n) => !Number.isInteger(n) || n < 0 || n > 255)) return false;
  const [x, y] = parts;
  return x === 10
    || (x === 172 && y >= 16 && y <= 31)
    || (x === 192 && y === 168)
    || (x === 169 && y === 254);
}

/**
 * Whether a gathered host candidate is worth sending to the far end.
 *
 * A desktop with virtualisation installed gathers host candidates on adapters that no remote peer
 * can ever reach: VirtualBox host-only (`192.168.56.x`) and WSL/Hyper-V vEthernet (`172.x`) both
 * showed up in the field on 2026-09-02. Signalling them costs the PEER a connectivity check per
 * dead address before ICE can settle, which is pure added latency on every call, and it also
 * tells them which virtualisation software is installed.
 *
 * The rule is deliberately narrow, because dropping a candidate that WOULD have worked is worse
 * than keeping a dead one: only a PRIVATE IPv4 host candidate on an interface that is not the
 * default route is suppressed. Such an address is reachable only from this machine's own virtual
 * networks. Anything we cannot judge is kept: reflexive and relay candidates (not host), an
 * mDNS `.local` name, IPv6, a public address, and every candidate at all when the native side
 * could not name a default route.
 */
export function shouldSignalHostCandidate(
  candidate: Pick<IceCandidateView, "type" | "address">,
  defaultRouteIpv4: string | null | undefined,
): boolean {
  if (candidate.type !== "host") return true;
  const route = defaultRouteIpv4?.trim();
  if (!route) return true; // no route known: never suppress on a guess
  const a = candidate.address?.trim();
  if (!a || a.endsWith(".local")) return true;
  if (!/^\d{1,3}(\.\d{1,3}){3}$/.test(a)) return true; // IPv6 or a hostname: not ours to judge
  if (a === route) return true;
  return !isPrivateIpv4(a);
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
