# CatComs 6e (3+) — Relay / NAT traversal design

The "relay-first" requirement (peers that can't directly connect reach each other
via zero-knowledge relays, then hole-punch to a direct link). Verified against the
**libp2p 0.56** source; this is the implementation contract. Staged so each slice
builds + tests on its own.

## Why this is its own careful block

Multi-process over TCP works today (6e-1/6e-2), but only when the joiner can dial
the server directly (same LAN / public IP / port-forward). The internet case —
both peers behind NAT — needs **circuit relay v2** (route through a third party)
and **DCUtR** (upgrade the relayed link to a direct one by simultaneous-open hole
punching). These touch the swarm's transport + behaviour composition and are
notoriously fiddly + hard to test, so they get a design pass and incremental,
separately-tested slices rather than one big change.

## libp2p 0.56 API (confirmed present in the registry source)

- `SwarmBuilder::…with_tcp(...)?.with_relay_client(noise::Config::new, yamux::Config::default)?`
  — wraps the base transport with the relay-client transport and hands the
  behaviour closure a `libp2p_relay::client::Behaviour` (builder/phase/other_transport.rs:104
  and relay.rs:47). Works after `with_other_transport` too, so the **memory** test
  swarm can be relay-capable identically to TCP.
- Relay **server**: `libp2p::relay::Behaviour` (the `relay` feature).
- **DCUtR**: `libp2p::dcutr::Behaviour` (the `dcutr` feature).
- **identify**: `libp2p::identify::Behaviour` — required so a client learns its
  observed external address (DCUtR needs it) and so the relay learns client addrs.
- **rendezvous**: `libp2p::rendezvous::{server,client}` (the `rendezvous` feature)
  — register/discover under a namespace; for authenticated peer discovery.

Add to `catcoms-net`'s libp2p features: `relay`, `dcutr`, `identify`, `rendezvous`.

## Swarm / behaviour changes

`MeshBehaviour` (currently `gossipsub` + `request_response`, `#[derive(NetworkBehaviour)]`)
gains, for **client** nodes (serve/join):
```
relay_client: libp2p::relay::client::Behaviour,   // from with_relay_client
dcutr:        libp2p::dcutr::Behaviour,
identify:     libp2p::identify::Behaviour,
```
Both `build_memory_swarm` and `build_tcp_swarm` switch to the `with_relay_client`
chain and construct these. The **relay server** is a *separate* swarm/behaviour
(`RelayBehaviour { relay: relay::Behaviour, identify, ping }`) with its own minimal
run loop — it never needs `MeshBehaviour` (it only forwards). The `Actor` learns to
handle `relay_client`/`dcutr`/`identify` events (mostly logging + surfacing
`PeerConnected` on a successful direct upgrade; reservation accepted/expired).

## Flows

1. **Reservation (server behind NAT).** `serve --relay <relay-multiaddr>`: after
   spawning, the server dials the relay and **listens** on
   `<relay-addr>/p2p/<relay-id>/p2p-circuit` (a `Command::Listen` added to the
   actor). The relay grants a reservation; the server's reachable address becomes
   the circuit address. The invite's `bootstrap` carries that circuit address.
2. **Circuit dial (joiner behind NAT).** `join`: dials the circuit address from the
   invite. libp2p routes the connection through the relay; `request_join` then runs
   exactly as today over that connection. **No protocol change above the transport.**
3. **Hole punch (DCUtR).** Once relayed, `identify` exchanges observed addresses and
   `dcutr` attempts a direct connection; on success traffic moves off the relay.
   Transparent to CatComs (still a `PeerConnected`).
4. **Discovery (rendezvous).** A `catcomsctl rendezvous` server; clients register
   under a per-server namespace (a blinded id) and discover peers — replaces hard
   coding bootstrap addresses. Eclipse-resistance (≥2 rendezvous + peer-exchange +
   roster-size check, ARCHITECTURE §2.8) layers on here.

## Staging (each slice compiles + tests independently)

- **6e-3a — relay-capable swarm + relay server.** Add the features; switch the
  swarm builders to `with_relay_client` + identify + dcutr; add `build_relay_swarm`
  + a relay run loop + `catcomsctl relay --port`. **Test:** a relay node + a client
  that **reserves** a slot (over the memory transport) — assert the reservation is
  accepted and the client gets a circuit listen address.
- **6e-3b — circuit dial end-to-end.** `serve --relay` reserves and advertises the
  circuit address; `join` dials it. **Test:** relay + server + joiner over the
  memory transport, server reachable *only* via the circuit address (no direct
  addr advertised), joiner completes `request_join` + catch-up through the relay.
- **6e-3c — DCUtR hole punch.** Wire dcutr/identify; surface the upgraded direct
  connection. **Test:** assert a direct connection forms after a relayed one (a
  contrived two-listener setup; real NAT can't be unit-tested, so assert the
  upgrade event path, not true NAT traversal).
- **6e-3d — rendezvous discovery + eclipse resistance.** Rendezvous server +
  client registration/discovery under blinded namespaces; ≥2 rendezvous, member
  peer-exchange, roster-size sanity check.

## Test strategy / honest limits

Real NAT traversal can't be unit-tested in-process. The slices test the
*mechanisms* over the libp2p **memory** transport: reservation accepted, a
connection routed through the relay (server advertises only its circuit address),
the DCUtR upgrade event path. End-to-end behind real NATs is a manual / staging
check. Relays stay **zero-knowledge** (Noise+MLS ciphertext only) — the relay never
sees plaintext, which is already true of every byte CatComs sends.

## Residuals / deferred

- Relay **rate-limiting / reservation caps** + relay selection/rotation (hardening).
- DCUtR **failure fallback** (stay relayed) UX + metrics.
- Metadata: a relay sees who-relays-through-it and timing (ARCHITECTURE §3) —
  mitigated (≥2 relays, cover traffic later), not eliminated.
