# Design; post-join steady-state rendezvous discovery

Status: **implemented; self-healing retry hardened 2026-08-21.** After joining, a member periodically re-registers itself
at the rendezvous under its rotation-aware namespaces and discovers + dials *other members* there;
so members re-find each other after a restart / address change with **no fresh invite**. Built on
the 6e-3d primitives (`rendezvous_namespaces`, the `DiscoveryPolicy`, PEX), driven by the per-server
**actor** through the generic transport.

See also: [`design-6e-rendezvous.md`](design-6e-rendezvous.md), [`design-rendezvous-ui.md`](design-rendezvous-ui.md).

## Implementation checklist

- [x] Periodic member-only rendezvous registration/discovery.
- [x] Authenticated bounded member PEX independent of rendezvous.
- [x] Vault-sealed, roster-reverified cross-session address cache.
- [x] Bounded redial backoff with jitter; disconnect and a newer signed address epoch bypass it.
- [x] Fold PEX records before the same pass's cached redial, so dynamic-IP updates are immediate.
- [x] Poll the route-selected IPv4/IPv6 interfaces on the discovery cadence; publish one fresh
  signed epoch and withdraw vanished raw routes without removing identical manual/mapping/relay
  ownership.
- [ ] Subscribe to native OS network-change events for lower latency; retain polling as the
  portable repair path and debounce event bursts into one signed epoch.
- [ ] Add an optional, tightly bounded previous-address-epoch grace window (one record, minutes,
  current routes first); never build an indefinite address history.
- [ ] Add authenticated reciprocal-dial signalling through an already connected member.
- [ ] Track bounded pairwise reachability evidence by address family and transport.
- [ ] Add SWIM-style indirect probes without equating suspicion with membership removal.
- [ ] Split topology maintenance into HyParView-like active/passive views and randomized promotion.
- [ ] Add CYCLON-like age-biased, source-diverse passive-view shuffles.
- [ ] Surface typed per-device health and safe personal/group actions in Connectivity.
- [ ] Add an explicit manual fallback-redial action for a completely isolated member.
- [ ] Make rendezvous renewal TTL-aware instead of re-registering every discovery tick.

## WebSocket boundary

WebSocket/WSS is useful as an additional **transport**, especially through enterprise HTTP proxies
or to a known relay/rendezvous endpoint on port 443. It is not a zero-server discovery or NAT-
traversal mechanism. A WebSocket starts as an outbound TCP connection to a listener that is already
reachable; two members whose routers both reject inbound traffic still cannot open one directly.
Hosting WSS on an ordinary member also adds certificate/name lifecycle without improving over the
same member's reachable libp2p TCP listener. Keep WSS for proxy-compatible infrastructure and
future browser interoperability, not as a substitute for reciprocal dialing, DCUtR, switchboards,
or relay circuits.

## Why this needs an actor + transport-trait change

The rendezvous verbs (register / discover / dial / `next_discovered`) are inherent on the concrete
`MeshService`, but the per-server **actor** owns the `Server` (and thus the transport's receivers)
after spawn; the bridge keeps only a write-only `MeshHandle`. Continuous discovery (and reading
`next_discovered`) must therefore live in the actor, which is generic over `MeshTransport`. So:

- **Extend `MeshTransport`** (catcoms-rt, libp2p-free) with the discovery verbs, using **opaque
  types** (`&str` namespaces, `&[u8]` rz-node ids, `String` addresses) and a new rt-native
  `DiscoveredPeer { peer: Vec<u8>, addresses: Vec<String>, namespace: String }`. **Default impls**
  make the in-memory transport inert: register/discover/dial/add_external return `Ok(())`;
  `next_discovered` returns `std::future::pending().await` (so its `select!` arm never fires; NOT
  `None`, which would busy-loop). `async_trait` supports default methods.
- **`MeshService`** implements them by mapping bytes↔`libp2p::PeerId` / `String`↔`Multiaddr`,
  delegating to its existing inherent methods (which the bridge still uses for found/join).

## The membership-tag is NOT used (it was never wired)

`routing_membership_tag`'s doc says the tag is "meant to ride as a registrant-signed synthetic
address in the libp2p PeerRecord, carried + verified in 6e-3d-9"; **but that carrying was never
implemented**: `net::Discovered` drops both the record `seq` and any tag, and register passes no
tag. So `Candidate.tag_verified` is **`false`** here. That is **safe**:
- `DiscoveryPolicy` never *drops* an unverified candidate; it only *ranks it last* (`tag_verified`
  is an additive score). So discovery still works.
- The `rendezvous_namespaces` are **member-only** (derived from the secret `ns_secret_L`); a
  non-member can't compute them, so registering/discovering under them already restricts to members.
- The **real gates are post-dial**: MLS group membership (a dialed non-member can't decrypt the
  channel), and `ingest_peer_record`/`request_pex` (membership-verified). A wrongly-dialed peer just
  wastes one dial (bounded by the policy budget).
**Update (2026-08-19): the synthetic-address wiring is now closed as a decision, not a follow-up.**
The libp2p `PeerRecord` cannot carry it (`register` takes addresses from the swarm-global external
set and mints `seq` itself), forcing it through `add_external_address` would broadcast a
group-linked token over `identify`, and no call site could act on it anyway; the pre-join path
that ranks several candidates holds no group secret, and the post-join path plans one candidate at
a time. Reasoning in full: the P9 row of `design-zeroconf-reachability.md` § 1c. What the eclipse
detector needed from it is served by roster-backed confirmation instead (the P8 row).

## State + flow

**`ChannelSync` gains** (catcoms-discovery promoted from dev- to a real dependency):
- `rendezvous_nodes: Vec<(String, Vec<u8>)>`; (dialable rz multiaddr, rz libp2p-id bytes), set at
  found/join, **persisted** in the snapshot (after the item-3 roster fields, before `finish()`; new
  fields strictly after the peer records, so `peer_addrs_from_snapshot` is unaffected).
- `discovery: DiscoveryPolicy` (one long-lived per group) + a bounded transient dial-retry map.
  Retry entries use monotonic exponential backoff with jitter, are cleared by a successful
  connection/disconnection lifecycle, and are bypassed immediately by a newer signed peer-record
  sequence. This replaces the old process-lifetime `dialed_peers` set, under which one failed dial
  permanently suppressed that member until this app restarted.

**Driver methods on `ChannelSync`** (called by `Server`, driven by the actor):
- `drive_discovery()` (async): for each rz node, for each `ns` in `rendezvous_namespaces(rz_node)`;
  `transport.rendezvous_register(ns, rz_node)` (advertise our external addrs under the member-only
  namespace) and `transport.rendezvous_discover(ns, rz_node)`.
- `next_discovered()` (async): delegate to `transport.next_discovered()`.
- `ingest_discovered(d)` (async): build a `Candidate { source: Rendezvous(rz_node), seq,
  tag_verified: false }`, apply the retry deadline, then call
  `DiscoveryPolicy::plan(roster = member_count)`. Only policy-granted dials receive retry state;
  budget-deferred candidates remain eligible. After connecting, the existing PEX + membership
  verification take over.

**Driving it (timer in the bridge, off the deterministic-time seam).** The periodic timer can't
live in `crates/`; the ambient-dependency gate forbids `tokio::time::interval`/`sleep` there (all
real time must flow through the `Clock` seam, which is logical-time-only). So the **bridge**
(`apps/`, not gate-scanned) spawns a per-server `tokio::time::interval` that sends a fire-and-forget
`AppCommand::DriveDiscovery` every `DISCOVERY_INTERVAL_SECS` (60s); the actor handles it in its
existing `cmd` arm: `if has_rendezvous { drive_discovery(); drain next_discovered() under one
`tokio::time::timeout` (allowed) + a count cap }`. Putting the drain in the command handler avoids
the `select!` borrow conflict (`sync_once` is `&mut`, `next_discovered` is `&mut`; they'd clash as
concurrent `select!` arms; in the handler they run sequentially after the arm wins). The timer task
exits when the actor stops (the command send errors). In tests (in-memory transport, no rendezvous)
no `DriveDiscovery` is ever sent and `has_rendezvous` is false, so discovery is inert + deterministic.

**Bridge:** at found/join/reload, set the member's rendezvous config on the `Server` (founder: the
rz it registered at; joiner: the invite's rz). For the joiner to be *discoverable* it must also
**listen + advertise** (today the joiner binds no port); so the join path binds a port,
`add_external_address`es it, and the actor's first tick registers it under the member-only
namespaces. The bridge's existing one-shot found/join `join_ns` registration is unchanged (it serves
the invite); steady-state adds the rotation-aware namespaces on top.

## Scope / deferred
- **Record seq; DONE (follow-up):** the discovered record's signed `seq` is now surfaced
  (`Discovered`/`DiscoveredPeer`/`Candidate`), so the `DiscoveryPolicy`'s anti-replay freshness is
  live (was inert under the placeholder `seq=1`).
- **`EclipseDetector` surfacing; DONE (follow-up):** `ChannelSync::observe_eclipse` feeds the
  hysteretic detector (R = roster, D = reachable member peers + self, S = distinct rendezvous roots);
  the actor emits `EclipseChanged{caution}` on a change and the UI shows an advisory banner. Strictly
  advisory; never gates dialing/messaging/membership.
- **Pre-dial membership-tag verification; closed as a decision (2026-08-19), not deferred:**
  carrying the per-namespace `routing_membership_tag` as a synthetic address in the libp2p
  PeerRecord is not merely invasive, it is unbuildable as designed and would be a new disclosure,
  and nothing above could use it (P9 in `design-zeroconf-reachability.md` § 1c).
  `tag_verified=false` is safe and permanent (the `DiscoveryPolicy` never *drops* an unverified
  candidate, only ranks it; the member-only namespace + MLS + PEX are the gates). The desktop now
  seals `AddressCache` beside the server snapshot and re-verifies every row on load; SQLCipher
  remains a storage-engine refinement.
- **Re-registration cadence:** a fixed interval (re-register every tick) rather than TTL-driven; a
  TTL-aware schedule is a refinement.
- **Ordinary interface churn; polling DONE, native notification deferred:** every discovery pass
  re-samples the kernel's route-selected IPv4/IPv6 sources. A changed set updates external-address
  ownership, the live bootstrap, Connectivity, and one new signed peer-record epoch before PEX in
  that same pass. Exact ownership prevents a disappearing raw GUA from removing an identical live
  PCPv6/manual route. Native platform notifications would reduce the current roughly one-minute
  detection latency; they must be debounced and polling must remain the portable repair path.
- **Pairwise reachability and reciprocal dial signalling:** not yet represented. A connected member
  can distribute signed records, but it does not yet carry a bounded, authenticated “please dial
  this member's fresh candidate now” signal or report which address/transport worked from its own
  vantage point.

## Address-history policy

The newest self-signed `PeerDescriptor` is authoritative and should normally carry every currently
valid IPv4 and IPv6 TCP/QUIC route together. The cache therefore keeps that complete newest record;
it does **not** union every historical IP forever. A withdrawn dynamic public address may be
reassigned to another subscriber, and continuing to probe it leaks timing/IP metadata even though
Noise will reject the wrong transport identity.

A future history layer may retain at most one preceding signed record for a short grace period, try
it only after the current epoch's routes fail, and discard it immediately on an explicit expiry or
mapping withdrawal. That is a recovery hint, never an address the current record is claimed to
endorse. Until that lifecycle exists, replacing the old record is safer and more truthful.

## Swarm-inspired next layer

The existing structures already resemble a small churn-tolerant overlay:

- **active view:** authenticated entries in `connected_peers`;
- **passive view:** roster-checked signed records in `AddressCache`;
- **periodic shuffle:** bounded PEX requests, currently randomized within trust tiers;
- **failure suspicion:** a missing connection/PEX answer affects presence and retry choice, never
  MLS membership.

The next coherent extension is HyParView/CYCLON-like behavior: on disconnect, promote randomized
passive candidates; exchange small age-biased record samples; retain source diversity; and age
unconfirmed candidates without deleting the member. SWIM-style indirect probes can then ask two
active members whether they can reach a suspect peer. “Unreachable from me” must remain distinct
from “offline” and must never remove a member.

The product-facing model should be typed per device: `connected_direct`, `connected_relay`,
`reachable_via_member`, `retrying`, `no_current_route`, last successful contact, candidate families,
and the next safe actions. Personal actions include reopening/forwarding the stable port or sharing
a fresh reply code; group actions include enabling a reachable switchboard or configuring a
relay/rendezvous node. No single peer's failed probe is a global health verdict.

## Security
Discovery only *surfaces* candidates; the `DiscoveryPolicy` alone decides dials (budget-bounded,
≤1 trust-root per rendezvous). A dialed non-member cannot read the channel (MLS) or pass
`ingest_peer_record` (membership-signed). The member-only namespace secrecy + the post-dial
membership gate are the load-bearing checks; the review focuses on the trait extension (no busy-loop
/ no event-stealing), the actor arms (cancellation-safety), the dedup set (unbounded growth), and
that a hostile rendezvous can't make a member dial unboundedly (policy budget).
