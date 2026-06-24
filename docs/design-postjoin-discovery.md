# Design — post-join steady-state rendezvous discovery

Status: **design approved, implementing.** After joining, a member periodically re-registers itself
at the rendezvous under its rotation-aware namespaces and discovers + dials *other members* there —
so members re-find each other after a restart / address change with **no fresh invite**. Built on
the 6e-3d primitives (`rendezvous_namespaces`, the `DiscoveryPolicy`, PEX), driven by the per-server
**actor** through the generic transport.

See also: [`design-6e-rendezvous.md`](design-6e-rendezvous.md), [`design-rendezvous-ui.md`](design-rendezvous-ui.md).

## Why this needs an actor + transport-trait change

The rendezvous verbs (register / discover / dial / `next_discovered`) are inherent on the concrete
`MeshService`, but the per-server **actor** owns the `Server` (and thus the transport's receivers)
after spawn — the bridge keeps only a write-only `MeshHandle`. Continuous discovery (and reading
`next_discovered`) must therefore live in the actor, which is generic over `MeshTransport`. So:

- **Extend `MeshTransport`** (catcoms-rt, libp2p-free) with the discovery verbs, using **opaque
  types** (`&str` namespaces, `&[u8]` rz-node ids, `String` addresses) and a new rt-native
  `DiscoveredPeer { peer: Vec<u8>, addresses: Vec<String>, namespace: String }`. **Default impls**
  make the in-memory transport inert: register/discover/dial/add_external return `Ok(())`;
  `next_discovered` returns `std::future::pending().await` (so its `select!` arm never fires — NOT
  `None`, which would busy-loop). `async_trait` supports default methods.
- **`MeshService`** implements them by mapping bytes↔`libp2p::PeerId` / `String`↔`Multiaddr`,
  delegating to its existing inherent methods (which the bridge still uses for found/join).

## The membership-tag is NOT used (it was never wired)

`routing_membership_tag`'s doc says the tag is "meant to ride as a registrant-signed synthetic
address in the libp2p PeerRecord, carried + verified in 6e-3d-9" — **but that carrying was never
implemented**: `net::Discovered` drops both the record `seq` and any tag, and register passes no
tag. So `Candidate.tag_verified` is **`false`** here. That is **safe**:
- `DiscoveryPolicy` never *drops* an unverified candidate — it only *ranks it last* (`tag_verified`
  is an additive score). So discovery still works.
- The `rendezvous_namespaces` are **member-only** (derived from the secret `ns_secret_L`); a
  non-member can't compute them, so registering/discovering under them already restricts to members.
- The **real gates are post-dial**: MLS group membership (a dialed non-member can't decrypt the
  channel), and `ingest_peer_record`/`request_pex` (membership-verified). A wrongly-dialed peer just
  wastes one dial (bounded by the policy budget).
Wiring the synthetic-address tag (for pre-dial eclipse hardening) is a documented follow-up.

## State + flow

**`ChannelSync` gains** (catcoms-discovery promoted from dev- to a real dependency):
- `rendezvous_nodes: Vec<(String, Vec<u8>)>` — (dialable rz multiaddr, rz libp2p-id bytes), set at
  found/join, **persisted** in the snapshot (after the item-3 roster fields, before `finish()`; new
  fields strictly after the peer records, so `peer_addrs_from_snapshot` is unaffected).
- `discovery: DiscoveryPolicy` (one long-lived per group) + `dialed_peers: HashSet<Vec<u8>>` (dedup,
  keyed on discovered peer bytes), both rebuilt fresh on restore (transient).

**Driver methods on `ChannelSync`** (called by `Server`, driven by the actor):
- `drive_discovery()` (async): for each rz node, for each `ns` in `rendezvous_namespaces(rz_node)` —
  `transport.rendezvous_register(ns, rz_node)` (advertise our external addrs under the member-only
  namespace) and `transport.rendezvous_discover(ns, rz_node)`.
- `next_discovered()` (async): delegate to `transport.next_discovered()`.
- `ingest_discovered(d)` (async): build a `Candidate { source: Rendezvous(rz_node), seq: 1,
  tag_verified: false }`, `DiscoveryPolicy::plan(roster = member_count)`, and for each planned addr
  whose peer isn't already in `dialed_peers`, `transport.dial(addr)`; mark dialed. After connecting,
  the existing PEX + membership verification take over.

**Driving it (timer in the bridge, off the deterministic-time seam).** The periodic timer can't
live in `crates/` — the ambient-dependency gate forbids `tokio::time::interval`/`sleep` there (all
real time must flow through the `Clock` seam, which is logical-time-only). So the **bridge**
(`apps/`, not gate-scanned) spawns a per-server `tokio::time::interval` that sends a fire-and-forget
`AppCommand::DriveDiscovery` every `DISCOVERY_INTERVAL_SECS` (60s); the actor handles it in its
existing `cmd` arm: `if has_rendezvous { drive_discovery(); drain next_discovered() under one
`tokio::time::timeout` (allowed) + a count cap }`. Putting the drain in the command handler avoids
the `select!` borrow conflict (`sync_once` is `&mut`, `next_discovered` is `&mut` — they'd clash as
concurrent `select!` arms; in the handler they run sequentially after the arm wins). The timer task
exits when the actor stops (the command send errors). In tests (in-memory transport, no rendezvous)
no `DriveDiscovery` is ever sent and `has_rendezvous` is false, so discovery is inert + deterministic.

**Bridge:** at found/join/reload, set the member's rendezvous config on the `Server` (founder: the
rz it registered at; joiner: the invite's rz). For the joiner to be *discoverable* it must also
**listen + advertise** (today the joiner binds no port) — so the join path binds a port,
`add_external_address`es it, and the actor's first tick registers it under the member-only
namespaces. The bridge's existing one-shot found/join `join_ns` registration is unchanged (it serves
the invite); steady-state adds the rotation-aware namespaces on top.

## Scope / deferred
- **Record seq — DONE (follow-up):** the discovered record's signed `seq` is now surfaced
  (`Discovered`/`DiscoveredPeer`/`Candidate`), so the `DiscoveryPolicy`'s anti-replay freshness is
  live (was inert under the placeholder `seq=1`).
- **`EclipseDetector` surfacing — DONE (follow-up):** `ChannelSync::observe_eclipse` feeds the
  hysteretic detector (R = roster, D = reachable member peers + self, S = distinct rendezvous roots);
  the actor emits `EclipseChanged{caution}` on a change and the UI shows an advisory banner. Strictly
  advisory — never gates dialing/messaging/membership.
- **Pre-dial membership-tag verification — still deferred:** carrying the per-namespace
  `routing_membership_tag` as a synthetic address in the libp2p PeerRecord (the only libp2p-level
  path) is invasive across 5 layers for marginal value — `tag_verified=false` is safe (the
  `DiscoveryPolicy` never *drops* an unverified candidate, only ranks it; the member-only namespace +
  MLS + PEX are the gates). `AddressCache` cross-session persistence: deferred.
- **Re-registration cadence:** a fixed interval (re-register every tick) rather than TTL-driven; a
  TTL-aware schedule is a refinement.

## Security
Discovery only *surfaces* candidates; the `DiscoveryPolicy` alone decides dials (budget-bounded,
≤1 trust-root per rendezvous). A dialed non-member cannot read the channel (MLS) or pass
`ingest_peer_record` (membership-signed). The member-only namespace secrecy + the post-dial
membership gate are the load-bearing checks; the review focuses on the trait extension (no busy-loop
/ no event-stealing), the actor arms (cancellation-safety), the dedup set (unbounded growth), and
that a hostile rendezvous can't make a member dial unboundedly (policy budget).
