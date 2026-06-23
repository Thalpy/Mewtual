# Design — rendezvous auto-discovery in the desktop UI

Status: **design approved, implementing.** Lets a joiner join a server with **no hard-coded
inviter address** — the invite points at a zero-knowledge rendezvous, the founder registers there,
and the joiner discovers → dials → joins. The entire mechanism is already built + tested one layer
below the bridge (`crates/catcoms-sync/tests/tcp_rendezvous_e2e.rs` is the canonical recipe); this
is the product/UI wiring.

See also: [`design-6e-rendezvous.md`](design-6e-rendezvous.md), [`HANDOVER.md`](HANDOVER.md).

## Why bridge-level (the architecture decision)

The rendezvous verbs (`rendezvous_register` / `rendezvous_discover` / `dial` / `next_discovered` /
`next_registered` / `add_external_address`) are **inherent methods on the concrete `MeshService`**,
NOT on the `MeshTransport` trait that `Server<T>`/`ChannelSync<T>` are generic over. The desktop
**bridge already holds the concrete `MeshService`** while founding/joining (it calls
`MeshService::new_tcp` + dials before constructing the `Server`, and `spawn()` only moves the
`Server` into the generic actor *after*). So the orchestration lives in the bridge, pre-spawn —
**no `MeshTransport` extension, no generic-actor refactor.** This mirrors the e2e test exactly.

`MeshService` is not `Clone` (it owns `Mutex<Receiver>`s), but every **write** verb routes through a
clonable `cmd_tx: mpsc::Sender<Command>`. So we add a small **`MeshHandle`** (a clone of `cmd_tx` +
the local `PeerId`) exposing the fire-and-forget verbs (`rendezvous_register`, `dial`,
`add_external_address`). The bridge keeps one per founded server so it can **register a fresh
on-demand invite under its new namespace after the Server has been spawned into the actor** (the
read side — `next_registered` confirmation — stays with the actor's `MeshService`; fresh-invite
registration is fire-and-forget, which is fine because registration is internally deferred+flushed
once an external address exists, established at found time).

## The namespace is nonce-keyed (why every invite must register)

`join_namespace(group_id, invite_nonce, rz_peer)` is keyed by the **invite nonce** (6e unlinkability:
a rendezvous operator can't link invites/joins to a group). So each single-use invite has its own
`join_ns`, and the founder must register under the `join_ns` of *each* invite it wants discoverable —
not once per group. Hence the `MeshHandle` for on-demand fresh invites.

## Flows

### Found (founder registers)
1. Build the `MeshService` (as today: listen + optional relay). If a **rendezvous address** is given:
   `validate_rendezvous_addrs(&[rz])` → `RendezvousTarget{addr, peer}`.
2. `mesh.dial(rz.addr)`; wait for `PeerConnected(phase0_peer_id(rz.peer))`.
3. Learn the bound addr (`next_listen_addr`), `mesh.add_external_address(bound_or_advertised_addr)`
   (so the deferred registration can flush).
4. Mint with `Server::mint_invite_with_rendezvous(nonce, expiry, bootstrap, vec![rz.addr])` (the rz
   addr is bound into the inviter signature — can't be stripped).
5. `join_ns = join_namespace(group_id, nonce, rz.peer)`; `mesh.rendezvous_register(join_ns, rz.peer)`;
   `await next_registered` (with a timeout — best-effort).
6. `Server::found(mesh, …)`; spawn; **keep `mesh.handle()` + the rz addrs in `ServerEntry`** and
   **persist the rz addrs** in `ServerRecord`.

### Join (joiner discovers — no hard-coded address)
1. Decode the invite. If `invite.rendezvous` is non-empty → discovery path; else the current
   bootstrap-dial path.
2. `validate_rendezvous_addrs(&invite.rendezvous)`; build `MeshService::new_tcp(None, &[rz.addr…])`;
   wait connected to each rz.
3. For each rz: `join_ns = join_namespace(invite.group_id, invite.invite_nonce, rz.peer)`;
   `rendezvous_discover(join_ns, rz.peer)`; collect `next_discovered` (bounded by a timeout).
4. Build a `catcoms_discovery::Candidate` per discovered record (`source = Rendezvous(rz.peer)`,
   `tag_verified=false` — pre-join), run ONE `DiscoveryPolicy::plan(...)` → `PlannedDial`s. **The
   policy decides what to dial; never auto-dial off `next_discovered`.**
5. The inviter id = `phase0_peer_id(discovered.peer)`. `dial` the planned addresses (and the invite's
   `bootstrap` addrs as extra direct candidates for the same peer); wait connected to the inviter.
6. `Server::join(mesh, …, inviter, &invite)` (unchanged handshake).

### Fresh on-demand invite (owner/admin)
`mint_invite_fresh` → new actor command `MintInviteWithRendezvous{nonce, expiry, bootstrap,
rendezvous}` (the persisted rz addrs) → returns the invite. The bridge then registers the new
`join_ns` via the stored `MeshHandle` (`rendezvous_register`, fire-and-forget). Falls back to the
plain bootstrap-only invite if the server has no rendezvous.

### Reload
`reload_one` rebuilds a `MeshService`. If the persisted `ServerRecord` carries rz addrs: dial +
add_external_address + register under the persisted (latest) invite's `join_ns` (best-effort,
pre-spawn), and keep a fresh `MeshHandle`.

## UI
- **Found form → Network `<details>`:** a "Rendezvous address (optional)" input next to advertise/
  relay, pre-filled from a saved default.
- **Settings → Network:** a persisted "default rendezvous address" (localStorage, like the sound
  toggle) so it isn't retyped.
- **Status:** a transient phase line — "registering at rendezvous…" on found, "discovering…" /
  "connecting…" on join — surfaced from the busy state (the commands are one-shot; the existing
  `busy` spinner text becomes phase-aware).
- Join needs **no input** — it reads `invite.rendezvous`.

## Scope / deferred (noted, not in this slice)
- **Periodic TTL re-registration:** a single registration's granted TTL (libp2p default ≫ the 1h
  invite life) covers an invite's usefulness, so we register once per invite. A long-lived server
  that outlives the TTL silently leaves discovery until its next fresh invite / reload — a
  re-register timer is a follow-up (would need the actor to own a re-register tick).
- **Post-join steady-state discovery** (rotation-aware `rendezvous_namespaces`, re-finding members
  after restart), **`EclipseDetector` surfacing**, **`AddressCache` persistence**, **PEX**, and
  **multi-rendezvous-root corroboration** beyond feeding all `invite.rendezvous` into one `plan()`.
- The rendezvous/relay **infra nodes** themselves are member-operated (`catcomsctl rendezvous`) — a
  deployment concern, not client code.

## Review outcome

Adversarially reviewed — **no blocking defects**; the no-auto-dial invariant holds (only
`DiscoveryPolicy::plan` decides what to dial) and `request_join`'s Welcome-signature + group-id
check fails closed against a malicious rendezvous (no wrong-group join). Folded SHOULD-FIX items:
loopback addresses are advertised to a rendezvous **only** when nothing else is reachable (so a
shared namespace isn't polluted with an unreachable record — `external_addrs`); a flaky reload that
can't re-register a discovery-enabled invite now **drops** that invite (the rail prompts a fresh,
self-registering one) rather than presenting one that won't resolve; plus clarifying comments
(circuit auto-promotion; the pre-join `seq` placeholder).

## Security
The dial decision stays with `DiscoveryPolicy` (never auto-dial). The joiner can't verify a member
tag pre-join (no group secret yet), so first-join eclipse-resistance is bounded by trusting the
invite-named rendezvous + the bound `join_ns`'s secrecy; the **`request_join` Welcome-signature
check fails closed after dialing** a wrong peer (the inviter signature is bound to the invite). The
rz addrs are signed into the invite (`mint_invite_with_rendezvous`), so a relay can't substitute
them. This wiring inherits the reviewed 6e security model; the adversarial review focuses on the
new glue (policy correctly applied, no auto-dial, timeouts bounded, the `MeshHandle` can't be
abused, fresh-invite registration races).
