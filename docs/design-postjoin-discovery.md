# Design; post-join steady-state rendezvous discovery

Status: **implemented; swarm-inspired repair completed 2026-08-28.** After joining, a member renews its registration
at the rendezvous under its rotation-aware namespaces and discovers + dials *other members* there;
so members re-find each other after a restart / address change with **no fresh invite**. Built on
the 6e-3d primitives (`rendezvous_namespaces`, the `DiscoveryPolicy`, PEX), driven by the per-server
**actor** through the generic transport.

See also: [`design-6e-rendezvous.md`](design-6e-rendezvous.md), [`design-rendezvous-ui.md`](design-rendezvous-ui.md).

Adversarial review follows [`ADVERSARIAL-REVIEW.md`](ADVERSARIAL-REVIEW.md).

## Implementation checklist

- [x] Periodic member-only rendezvous registration/discovery.
- [x] Authenticated bounded member PEX independent of rendezvous.
- [x] Vault-sealed, roster-reverified cross-session address cache.
- [x] Vault-sealed authenticated direct-join route for same-LAN close/reopen, kept local rather
  than publishing private addresses through PEX.
- [x] Bounded redial backoff with jitter; disconnect and a newer signed address epoch bypass it.
- [x] Fold PEX records before the same pass's cached redial, so dynamic-IP updates are immediate.
- [x] Poll the route-selected IPv4/IPv6 interfaces on the discovery cadence; publish one fresh
  signed epoch and withdraw vanished raw routes without removing identical manual/mapping/relay
  ownership.
- [x] Subscribe to native OS network-change events for lower latency; retain polling as the
  portable repair path and debounce event bursts into one signed epoch.
- [x] Require one canonical, terminal peer-bound multiaddr grammar for invite, rendezvous, PEX,
  switchboard, companion-grant, two-way reply, and cached member routes; remove the transport's
  bare discovery dial fallback.
- [x] Charge endpoint attempts (not peers) through one transient scheduler shared by every desktop
  server and untrusted pre-join/member-discovery path, with per-process, per-server, canonical
  Phase-0-peer, direct-socket or relay-circuit, IPv4 `/24`, and IPv6 `/48` limits on the injected
  monotonic clock. Parser-derived opaque endpoints prevent identity-domain aliases.
- [x] Close previous-address-epoch grace as a safety decision: a newer signed omission/withdrawal
  is authoritative, and both live and sealed caches discard the old route rather than probing a
  potentially reassigned dynamic IP.
- [x] Add authenticated, exact-descriptor reciprocal-dial signalling through an already connected
  proven member, with queued connected-only forwarding and shared endpoint limits.
- [x] Track bounded, session-only pairwise reachability evidence by address family and transport;
  keep aggregate connect/disconnect authoritative, handle concurrent relay/direct paths, and expire
  historical successes after 24 hours.
- [x] Add SWIM-style indirect probes without equating suspicion with membership removal.
- [x] Split topology maintenance into derived HyParView-like active/passive views and randomized
  logical promotion; never close a shared libp2p connection to meet a view target.
- [x] Add CYCLON-like local age-biased, source-diverse passive-view shuffles.
- [x] Surface typed per-device claimed-route health and safe personal/group actions in
  Connectivity/debug output without equating local unreachability with offline presence.
- [x] Add an explicit manual fallback-redial action with a monotonic anti-click cooldown while
  preserving the ordinary discovery policy and process-wide endpoint scheduler.
- [x] Make rendezvous renewal TTL-aware (renew at 75% of the bounded granted lease, with a bounded
  retry deadline while the grant is pending).

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
  wastes canonical, endpoint-metered attempts (bounded both locally and across the process).
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
- `endpoint_dials: EndpointDialScheduler`; an explicitly injected clone of the desktop's one
  process handle. The local policy ranks and reserves by endpoint; the shared scheduler is the
  final gate before socket submission. A shared denial is refunded to the local window because no
  attempt happened, while the process counters are charged only for routes actually granted.
- `local_reconnect_routes: Vec<(PeerId,String)>`; transient in `ChannelSync`, restored from the
  desktop's version-3 `ServerNet`. Its adjacent durable `ReconnectPolicy` is `Disabled`,
  `AuthorizedPeer(peer)`, or `LegacyPending`; a stored row is valid only for the authorized peer.
  Direct admission authorizes only the named inviter and makes one bounded best-effort PEX request
  before the first post-join snapshot, so reload normally has the inviter's signed transport claim.
  Helper/reply/switchboard admission is explicitly disabled. Every retry requires an exact, unique
  current roster claim for that peer, raw literal-IP TCP/QUIC shape, and the shared endpoint
  scheduler. The hint is never gossiped and never mixed into the public-only `AddressCache`/PEX
  candidate set.
  On each discovery cadence, `AuthorizedPeer` may refresh only that peer. `LegacyPending` may
  promote once only when there is exactly one other member and one unique live signed peer claim;
  that captured route must be private/loopback. This excludes public infrastructure and
  ambiguous/helper-bearing groups. An empty observation never deletes the old hint merely because
  the remote app is currently closed.

**Driver methods on `ChannelSync`** (called by `Server`, driven by the actor):
- `drive_discovery()` (async): reconnect/discover on every cadence but register each exact
  rendezvous-node/namespace key only when its lease is absent or due. A successful grant schedules
  renewal at 75% of its bounded TTL; a pending grant gets a short retry deadline.
- `next_postjoin_discovery_event()` (async): select between `transport.next_discovered()` and
  `transport.next_registered()` without busy-looping an inert transport.
- `ingest_discovered(d)` (async): build a `Candidate { peer: canonical_transport_peer,
  source: Rendezvous(rz_node), freshness: Transport(canonical_transport_peer), seq,
  tag_verified: false }`, apply the retry deadline, then call
  `DiscoveryPolicy::plan(roster = member_count)`. Only policy-granted dials receive retry state;
  budget-deferred candidates remain eligible. Every surviving address must be canonical and end
  in the discovered record's exact `/p2p/<PeerId>`; public IP classification is applied
  separately. After connecting, the existing PEX + membership verification take over.

**Driving it (timer in the bridge, off the deterministic-time seam).** The periodic timer can't
live in `crates/`; the ambient-dependency gate forbids `tokio::time::interval`/`sleep` there (all
real time must flow through the `Clock` seam, which is logical-time-only). So the **bridge**
(`apps/`, not gate-scanned) spawns a per-server `tokio::time::interval` that sends a fire-and-forget
`AppCommand::DriveDiscovery` every `DISCOVERY_INTERVAL_SECS` (60s); the actor first tries any
sealed local reconnect route even when no rendezvous is configured, then handles rendezvous work
in its existing `cmd` arm: `if has_rendezvous { drive_discovery(); drain next_discovered() under one
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
the invite); steady-state adds the rotation-aware namespaces on top. The bridge also owns one
`EndpointDialScheduler` and installs clones before eager cached redial. Invite bootstraps,
invite-supplied rendezvous seeds, rendezvous results, and consented switchboard candidates cross
the same final untrusted-discovery boundary. Trusted operator-infrastructure connections retain
their separate validator and lifecycle rather than being claimed here.

## Scope / deferred
- **Record seq; DONE (follow-up):** the discovered record's signed `seq` is now surfaced
  (`Discovered`/`DiscoveredPeer`/`Candidate`), so the `DiscoveryPolicy`'s anti-replay freshness is
  live (was inert under the placeholder `seq=1`). Freshness is signer-scoped: cached member
  descriptors use `Device(device_id)` while rendezvous records use `Transport(peer_id)`; both may
  merge into the same canonical transport dial target without either sequence pinning the other.
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
- **Re-registration cadence: DONE:** discovery remains periodic, while exact node/namespace
  registrations renew from the server-granted TTL. The transport event carries no request id, so
  commands remain serialized per key with a bounded retry deadline rather than claiming arbitrary
  concurrent-request correlation.
- **Ordinary interface churn; DONE:** one process-wide native monitor consumes Windows route/IP
  notifications, Linux/Android netlink, and Apple/BSD route notifications. Callback bursts are
  coalesced before every running server re-samples the kernel's route-selected IPv4/IPv6 sources.
  A changed set updates external-address ownership, the live bootstrap, Connectivity, and one new
  signed peer-record epoch before PEX in that same pass. Exact ownership prevents a disappearing
  raw GUA from removing an identical live PCPv6/manual route. The roughly-minute discovery poll
  remains active as the portable repair path if platform monitoring fails or misses an event.
- **Pairwise reachability and post-join reciprocal repair: DONE.**
  The invite `JoinReply` proof retry now has a connected-only network command and cannot implicitly
  redial from the ordinary recent-peer cache after the endpoint scheduler refuses a pass. A
  connected member
  can distribute signed records, and each device now retains bounded local path evidence for
  records that claim a live transport. The repair layer now carries a bounded authenticated
  “please dial this exact current descriptor” signal and signed, exact-attempt-bound evidence that a
  helper currently does or does not hold a proven path to that claimed transport. It deliberately
  does not claim which advertised socket worked or that the human is online.

## Reciprocal-dial design after adversarial review

Targeted member-control gossip is rejected. Pubsub forwarding occurs below the application-level
authorization handler, so a removed member that still knows a grandfathered topic could cause a
stale subscriber to relay a dial request. Gossip would also disclose attempted peer pairs and timing
to every subscriber.

The zero-server design instead uses a small **addressed, current-member helper protocol**. If A
cannot reach B but remains connected to C, A asks C to deliver a request to B. C catches up and
revalidates its current roster before forwarding; C never dials A and does not carry the resulting
connection. B validates the request and then dials A's already accepted current signed record. The
ordinary discovery loop continues A's own bounded retries. This does not claim general TCP
simultaneous-open and cannot repair a completely partitioned group with no surviving path.

The request contains references to the exact canonical hashes and sequences of A and B's already
accepted descriptors, not embedded addresses or replacement descriptors. An equal sequence with a
different hash is equivocation and is rejected. A descriptor replacement, route withdrawal, member
removal, session change, shutdown, or server deletion cancels the corresponding pending intent.

Every compatible current member provides this small baseline control verb; it is not an opt-in
hosting role. The requester-authenticated frame binds the group, requester and target devices, both
exact descriptor hashes/sequences/peers, a random attempt id, and a short expiry. A currently
connected helper may forward it only when it holds proven live paths to both claimed peers, and
adds its own signature over the original frame and source/target peers. Helpers never invent or
substitute routes. B rechecks current roster, exact descriptors, helper proof, signature, replay,
rate, and expiry regardless of which helper delivered it.

Accepted work becomes a reference-only actor intent. Immediately before dialing, the actor resolves
A's current record again and sends at most two direct peer-bound candidates (at most one IPv4 and
one IPv6, TCP or QUIC) through the shared endpoint scheduler. There is no relay or bare-address
fallback inside this direct repair verb. The
scheduler accounts for every endpoint at pair, device, address/prefix, server, and process scopes;
new address epochs and native network-change events refresh records but never directly trigger a
reciprocal-dial storm.

Handlers authenticate and enqueue connected-only pushes; probes, results, reciprocal forwards and
deliveries each arrive on separate actor turns, so no sole-owner actor waits for a remote repair
response. Cancellation removes pending exact-descriptor intents on replacement/removal, but the
current transport seam cannot recall a dial already submitted. An old client that does not implement the verbs times out
as `unknown`, never as `offline`. The UI derives presence and route claims from typed evidence; a
transport connection alone does not prove which route, family, descriptor epoch, or helper path
worked. This reciprocal protocol is deliberately **not** a dual signature by the device and libp2p
keys, so it does not upgrade `binding=self_asserted`.

### Baseline participation versus optional hosting

Every online, current, compatible member participates in the group mesh. This baseline is part of
joining the P2P network and is not disabled by opting out of hosting. It includes maintaining the
member's own outbound connections, distributing authenticated group traffic and signed address
epochs over already established paths, bounded PEX and health responses, and forwarding small,
addressed, authenticated reciprocal-dial control requests when the member is already on a viable
path. These duties must remain tightly bounded so an ordinary member cannot be conscripted into
arbitrary dialing or meaningful third-party bandwidth use.

Opt-in roles begin where the device accepts additional exposure or resource cost: accepting
pre-member traffic, serving as a standing switchboard, carrying another pair's application traffic,
opening a public relay/rendezvous/AutoNAT service, or accepting materially higher bandwidth/storage
budgets. Opting out of those roles therefore does not turn a member into a passive consumer; it only
prevents silent promotion into a public listener or general relay. A user can always cease all
participation by disconnecting or leaving the group.

Implementation status:

- [x] enforce one canonical direct-route grammar with a mandatory terminal `/p2p/<PeerId>` matching
  the signed descriptor and remove every discovery bare-address dial path;
- [x] meter endpoints rather than peers through a process-wide scheduler shared by existing
  untrusted discovery paths, using parser-derived canonical transport principals; reciprocal work
  reuses it;
- [x] add a typed, fail-closed, direct-only peer-bound batch transport API;
- [x] implement bounded session/replay state, addressed queued helper forwarding, pending-intent
  cancellation, and deterministic topology-aware A-to-C-to-B tests; and
- [x] complete another adversarial review of each later implementation slice before handoff (final
  re-review: no remaining blocker/high/medium/low finding in this boundary).

## Address-history policy

The newest self-signed `PeerDescriptor` is authoritative and should normally carry every currently
valid IPv4 and IPv6 TCP/QUIC route together. The cache therefore keeps that complete newest record;
it does **not** union every historical IP forever. A withdrawn dynamic public address may be
reassigned to another subscriber, and continuing to probe it leaks timing/IP metadata even though
Noise will reject the wrong transport identity.

A previous-address grace layer was rejected after adversarial review: this wire format cannot tell
ordinary omission from an intentional withdrawal strongly enough to keep probing safely. The
newest record is therefore authoritative. A zero-route replacement removes the previous live and
sealed cache row, and cache reload accepts a row only when its descriptor equals the current epoch.
This may lose a still-valid route an honest client accidentally omitted, but avoids probing an IP
after dynamic reassignment. Any future grace design needs an explicit signed route-transition or
withdrawal object; it must not infer permission from history alone.

## Swarm-inspired repair layer

The existing structures already resemble a small churn-tolerant overlay:

- **active view:** authenticated entries in `connected_peers`;
- **passive view:** roster-checked signed records in `AddressCache`;
- **periodic shuffle:** bounded PEX requests, currently randomized within trust tiers;
- **failure suspicion:** a missing connection/PEX answer affects presence and retry choice, never
  MLS membership.

The implemented HyParView/CYCLON-like layer derives a bounded active view from currently connected,
session-proven member paths and a passive view from current roster-checked descriptors. A disconnect
logically promotes one randomized passive descriptor plus the departed repair target, but never
closes a shared transport connection to enforce an overlay degree. PEX increments saturating local
record ages, resets age on a newer descriptor, records distinct authenticated PEX sources locally,
and serves age-biased/source-diverse bounded samples. Age and source claims never arrive from wire.

SWIM-style probes ask at most two active helpers about their present proven path to one exact target
descriptor. An unanswered pending probe expires as unknown. One signed positive becomes `reachable_via_member`; two distinct
signed negatives become `suspected_unreachable`. Neither changes local connection health, presence,
or MLS membership. A positive helper can carry the reciprocal request described above.

The product-facing model is now typed per device record: `claimed_peer_connected_direct`,
`claimed_peer_connected_relay`, `claimed_peer_connected_other`,
`claimed_peer_dial_cooling_down`,
`claimed_peer_dial_eligible`, `claimed_peer_has_no_route`, and `no_peer_record`, plus current path
families/transports, a time-bounded last successful path, candidate families/transports, and typed
personal/member/group actions, plus separate `reachable_via_member` / `suspected_unreachable`
evidence, helper-response count/age, and exact-descriptor reciprocal-pending state. No single peer's
failed probe is a global health verdict.

`claimed_peer_dial_cooling_down` means a policy-approved dial batch was submitted and its scheduler
deadline has not elapsed. The transport acknowledges actor submission but does not report a later
per-address connection result, so the counter is not labelled as a completed attempt or failure. Likewise,
IPv6-only candidates are shown as a clue: an advertised/public IPv6 observation is not an outbound
route test and cannot prove why a connection did not open. Current switchboards remain bounded
**admission-only** forwarders; they are not offered as a repair action for already-joined members.

The `claimed_peer_` prefix and `self_asserted` binding are load-bearing. A member signs the
`PeerDescriptor` containing its transport id, but the current protocol does not prove that its
device key controls that libp2p key. Active path observations can therefore refine the route named
by the member's record, but cannot prove the person/device is online. The implemented reciprocal
repair does not bind both keys; a future explicit dual-key proof is required before product wording
may remove that qualification.

## Security
Discovery only *surfaces* candidates; the `DiscoveryPolicy` ranks dials (budget-bounded,
≤1 trust-root per rendezvous), then the process scheduler applies the final endpoint limits. A
signed route must use the supported canonical TCP/QUIC or circuit grammar, name an allowed IP
literal, and terminate in the exact advertised peer id; signatures still do not prove ownership
of that public socket. A dialed non-member cannot read the channel (MLS) or pass
`ingest_peer_record` (membership-signed). The member-only namespace secrecy + the post-dial
membership gate are the load-bearing checks; the review focuses on the trait extension (no busy-loop
/ no event-stealing), the actor arms (cancellation-safety), bounded scheduler state, and that a
hostile rendezvous/member cannot turn peer or sequence rotation into unbounded endpoint traffic.
