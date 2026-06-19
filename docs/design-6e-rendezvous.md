# CatComs 6e-3d — Rendezvous discovery + eclipse-resistance

Implementation contract, synthesized from a 7-agent design+adversarial-review workflow
(3 independent designs → 3 attackers → synthesis) and **verified against the real
libp2p-rendezvous 0.17.1 API and the repo source**. Read this before building any 6e-3d
slice. Companion to [`design-6e-relay.md`](design-6e-relay.md) (relay/DCUtR, done).

## Why this block grew

"Rendezvous discovery + eclipse-resistance" (ARCHITECTURE §2.8) cannot be shipped honestly
without also fixing two things the review found in the **existing** code:

- **A1 (CRITICAL, pre-existing).** `channel_topic`/`control_topic` (catcoms-sync lib.rs ~399,411)
  hash the **raw `group_id`**, and `group_id` travels in **plaintext** in `InviteToken`
  (invite.rs ~121). So **any invite-holder — including a non-member — can compute every
  gossip topic today** and subscribe to the membership-commit and op fan-out. Blinding the
  rendezvous namespace while the paired topic stays invite-derivable is pure theatre (§2.5
  literally requires "topic AND namespace derive from the same label, rotate together").
  → Re-keyed from `ns_secret_L` in **slice 6e-3d-2**. Not deferred.
- **Sybil-C1 (CRITICAL).** "Discovery is a hint; junk just fails the handshake" is **false at
  the catcoms-sync layer**: `remember_peer` runs on raw `PeerConnected` (a Noise handshake,
  not group membership), `pick_catchup_peer` draws from that set, and catch-up **responses**
  are unauthenticated (only requests are). Auto-dialing discovered records therefore poisons
  the catch-up source set with un-handshaked Sybils and stalls convergence — an availability
  eclipse with no group member involved. → Two-pool peer model + signed responses +
  member-verifiable discovery tag + no auto-dial + global dial budget (slices 5/6).

The net effect: 6e-3d is a **9-slice mini-phase**, each independently compiling and
memory-transport tested.

## Namespace + topic derivation (the core construction)

openmls `export_secret` is **current-epoch only**, and `ServerGroup` wraps a live `MlsGroup`
with no epoch history — so the rotation-on-removal secret history must be **built in
catcoms-sync**, not read from MLS. New state in `ChannelSync`:

- `DocType::Routing = 8` in catcoms-wire (`from_tag` arm + golden test — both required).
- `ServerGroup::routing_metadata_secret(device) -> [u8;32]` = `metadata_secret(device,
  DocType::Routing, 0)` (current epoch only; all openmls can give).
- A replicated removal counter `L: u64` and a bounded `BTreeMap<u64, Zeroizing<[u8;32]>>`
  retaining `{L-2, L-1, L}` — **two** grandfather windows (a single window self-eclipses a
  member two removals behind during rapid removals; review A2).

**Snapshot point (the fix for "can't reuse the snapshot machinery"):** at the local
`commit_remove_now` **and** on the inbound Remove-apply path, **after** the epoch advances
(so the secret is the *post-removal* epoch a removed member can never export — fixes C2),
capture `routing_metadata_secret()` into slot `L`, then `L += 1`. This is a **new** snapshot
alongside the existing content-key `snapshot_epoch_keys()` at the same site. The inbound path
must detect a Remove: `process_incoming` currently returns `Incoming::CommitApplied` for all
commits — extend it to inspect `staged.remove_proposals()` (confirmed available, openmls
staged_commit.rs:617) **before** merge and surface "a removal happened" so `L` advances
identically on every member.

Derivation (pure; no ambient time/RNG):

```
ns_secret_L = routing_metadata_secret @ the epoch produced by the L-th Remove   // member-only, post-removal
ns_bytes    = BLAKE3_keyed(key = ns_secret_L, "catcoms/rendezvous/ns/v1" ‖ group_id ‖ L_be8 ‖ rz_peer_id_i)
namespace_i = "catcoms1-" ‖ base32_nopad_lower(ns_bytes[..20])                  // ≤41 ASCII, well under 255
```

- **Per-rendezvous diversification** (`rz_peer_id_i` folded in): each rendezvous sees a string
  unique to itself, so two colluding rendezvous can't join logs on an identical namespace.
- **Same `ns_secret_L`/`L` re-key the topics** (slice 2): replace `BLAKE3("catcoms/topic/v1" ‖
  raw group_id ‖ …)` with `BLAKE3_keyed(ns_secret_L, "catcoms/topic/v2" ‖ group_id ‖ L ‖ type ‖ id)`
  (control topic v3 likewise). Both rotate on removal with the same `{L}` grandfather window.
- `ChannelSync::rendezvous_namespaces(rz_peer_id) -> Vec<String>` returns `[current]` at L=0,
  `[current, prev]` for L≥1 (dedup). Register under / discover across the window so a member
  who hasn't yet processed the latest Remove is still found, while a **removed** member is
  excluded the instant the Remove lands.

## Invite changes

Bump `INVITE_DOMAIN` (hard cutover; pre-release). The namespace is **never** in the invite
(secret-derived, post-join, rotates on removal). `InviteToken` gains `rendezvous: Vec<String>`
(≥2 **direct** `/ip4|dns/.../tcp/.../p2p/<rz-id>` infra multiaddrs; reject `/p2p-circuit`;
distinct PeerIds). The distinct-PeerId check catches only accidental-duplicate misconfig, **not
adversarial collusion** — so rendezvous are credited as **≤1 trust root** in the eclipse layer
regardless. `bootstrap: Vec<String>` is retained but shrinks to an optional single inviter
seed; length-prefix both vectors in the signing payload (round-trip + tamper tests).

**Cold-start (honest):** a joiner can't compute the group namespace before joining (no exporter
secret yet). Slice 9 adds a pre-join `join_ns = BLAKE3_keyed(HKDF(invite_nonce),
"catcoms/join-rz/v1" ‖ group_id ‖ rz_peer_id_i)` so the inviter registers pre-join and the
joiner discovers it via ≥2 rendezvous — removing the single-seed SPOF. ≥2 named admitters
(so one offline inviter doesn't brick the invite) touches the §4a Welcome path → deferred.

## Eclipse-resistance design

| Mechanism | What it stops |
|-----------|---------------|
| **Single global `DiscoveryPolicy`** (pure, in a new `catcoms-discovery` crate) is the *only* thing that decides what to dial. The net Actor **never auto-dials** — it surfaces every signed `PeerRecord` on a never-dropping `next_discovered()` queue. A global, Clock-paced, RNG-jittered **dial budget** (shared across rendezvous+PEX+cache) caps junk at B dials/window. | Auto-dial source-set poisoning; dial-concurrency exhaustion. |
| **Member-verifiable registration tag**, checked **before dialing**: `tag = BLAKE3_keyed(ns_secret_L, rz_peer_id ‖ L ‖ record.seq)`. A discoverer (member, holds `ns_secret_L`) drops any record whose tag doesn't verify. | Leaked/guessed-namespace Sybil floods → one rejected hash, **no dial**. A just-removed member's L-1 tag is rejected by discoverers past the Remove. |
| **Two-pool peer model** in catcoms-sync: `discovered_peers` (untrusted) ≠ `member_peers` (only peers whose members-only RESPONSE is signed by a roster key). `remember_peer` stops running on raw `PeerConnected`; `pick_catchup_peer` draws only from `member_peers`. Catch-up/PEX responses signed under new `CATCHUP_RESP_DOMAIN`/`PEX_RESP_DOMAIN`, verified vs `contains_device`. | Sybil-C1 (un-handshaked peers serving recovery). |
| **Member PEX** (`KIND_PEX`, 5th RR kind) wrapped in the existing `authenticate_request` membership proof; entries are each peer's **own** signed `PeerRecord`; responder-signed; capped + rate-limited; issued only to `member_peers`. | Single-rendezvous omission (members supply peers without any rendezvous); outsider IP harvesting. |
| **Roster-size detector, ADVISORY-ONLY.** `R=member_count()` (local, unforgeable), `D`=distinct roster DeviceIds with a live handshake this session, `S`=distinct **trust roots** (all rendezvous ≤1; each PEX-vouching member =1; cache counts only via the live re-proof it enables). `suspect = R>floor && (D-1)/(R-1)<min_reach && S<min_sources`, on the Clock, after a grace window, with hysteresis. Raises WARN/CAUTION on `next_eclipse_warning()`. **Never blocks messaging; never gates a Remove.** | Eclipse by isolation (surfaced, not silently resolved). Weaponizing the detector to block a Remove (H3) — it can't. |
| **Cross-session address cache** (SQLCipher), signed `PeerRecord`s for **proven** members only; CryptoRng-jittered eviction; tamper-detected on load; a hit counts toward `S` only after a fresh live re-proof; freshness off the registrant's **own** signed seq, not the server-asserted TTL (a colluding server lies about TTL — A5). | First-contact eclipse on a returning node; stale-record replay pinning. |
| **`ConnectionLimits`** on `MeshBehaviour`, `RelayBehaviour`, `RendezvousBehaviour` (none exist today, H6); reserved outbound dial slots for member/cached/corroborated peers; per-PeerId token bucket on the rendezvous server's register/discover (H5). | Transport-level slot/dial exhaustion. |

### Honest residuals

- **All rendezvous colluding at once:** no rendezvous-sourced honest peer arrives; falls back to
  member PEX (needs ≥1 honest live member) + cache (needs a non-stale prior contact). Total
  isolation (all rendezvous hostile AND every cached peer gone AND no honest member live) is
  **undecidable from inside the node** — surfaced as a persistent CAUTION + "verify out-of-band".
- **Rendezvous independence can't be proven** — distinct-PeerId is a misconfig check; two
  secretly co-operated rendezvous remain an operator-trust assumption (credited ≤1 root).
- **Metadata** is the dominant residual (§3): each rendezvous learns `namespace_i ↔ IP ↔ timing`
  for the TTL (2–72h — a queryable directory, a *higher*-value target than a relay); querying ≥2
  doubles the operators who see it. Diversification removes the cross-operator join key; rotation
  limits long-term linkage but leaks a **removal-cadence** signal to a persistent observer —
  blunted (jittered TTL across [2h,72h]), not erased; accepted trade.
- **Two colluding in-group members** can present S≈2 and partially evade the advisory detector
  (it's a hint, not a proof).
- **First-contact** still depends on a reachable inviter via the seed until slice 9's `join_ns`.

## Slices (each compiles + memory-transport tests on its own)

| Slice | Deliverable | Gate test |
|-------|-------------|-----------|
| **6e-3d-1** | `DocType::Routing` (+from_tag+golden) · `routing_metadata_secret` · extend `Incoming` to flag Removes · replicated `L` + `{L-2,L-1,L}` metadata-secret store in ChannelSync · `rendezvous_namespaces()`. **Pure, no swarm.** | All members converge on identical `L`/namespace; namespace changes after Remove but **not** after Add/Update/post; grandfathered for 2 windows; a **removed** member can compute the pre- but not post-removal namespace; distinct groups never collide. |
| **6e-3d-2** | Re-key `channel_topic`+`control_topic` from `ns_secret_L` (A1 fix; domains v2/v3; rotate on removal w/ grandfather window). | An invite-holding **non-member** cannot compute any topic; two members agree; topic changes after Remove; mid-window node still receives on the previous control topic. |
| **6e-3d-3** | Rendezvous **server**: `RendezvousBehaviour {server, identify, ping, connection_limits}` + `build_rendezvous_swarm`/`build_memory_rendezvous_swarm` + `run_rendezvous` (per-PeerId token bucket on Clock) + `catcomsctl rendezvous --port`. Server `Config` caps. | A throwaway client given an external addr registers; assert client `Registered` AND server `PeerRegistered`. |
| **6e-3d-4** | Rendezvous **client** into `MeshBehaviour` (+connection_limits); explicit `add_external_address` on first confirmed external addr → flush deferred registers; `Command::RendezvousRegister/Discover` + MeshService methods + never-dropping `next_discovered()`/`next_registered()`. **Actor never dials.** Re-registration on injected Clock. | A registers (via circuit reservation), B discovers; discovered peer_id == A.local, addrs non-empty, peer_id matches signer; assert **no dial** issued on discovery. |
| **6e-3d-5** | Member-verifiable discovery tag verified pre-dial · two-pool model (`discovered_peers`/`member_peers`) · stop `remember_peer` on raw `PeerConnected` · `pick_catchup_peer` member-only · sign catch-up **responses** (`CATCHUP_RESP_DOMAIN`). | Valid-tag record dialed; junk-tag dropped pre-dial; un-handshaked peer never enters `member_peers`; unsigned/wrong catch-up response rejected, no promotion. |
| **6e-3d-6** | `catcoms-discovery`: pure `DiscoveryPolicy` (union, ≤1 root/rendezvous, Clock+RNG dial budget, multi-page cookie re-sort honest-first, freshness off signed seq, roster clamp). Ranks only — never gates messaging. | Single rendezvous can't dominate dial order; corroborated peer first; 500-under-roster-4 flood clamped; stale-seq dropped; cache-only peer still offered; junk ranks last. |
| **6e-3d-7** | Member PEX (`KIND_PEX`) over the members-only auth channel; entries = peers' own signed records; responder-signed; capped/rate-limited; feeds DiscoveryPolicy. | M1 PEXes M2, learns M3; non-member PEX rejected; junk reply raises raw discovered but not distinct member_peers; reply verified. |
| **6e-3d-8** | Advisory eclipse detector (D/R/S, hysteresis, `next_eclipse_warning()`, never gates) + cross-session SQLCipher cache (proven members only, RNG-jittered eviction, seq persisted). | Big roster + all-attacker → suspect after grace; small group → never; diverse partition → not suspect; single-source low → suspect; a legit Remove applies under CAUTION; session-2 reaches a cached member past a hostile rendezvous; tampered row rejected. |
| **6e-3d-9** | Invite rewiring (`rendezvous: Vec<String>`, INVITE_DOMAIN bump) + `serve --rendezvous`/`join` discover→dial→request_join + pre-join `join_ns`. End-to-end. | Invite round-trip+tamper; same-PeerId-twice fails; circuit-in-rendezvous rejected; end-to-end memory: joiner discovers inviter via `join_ns`, dials, joins with **no hard-coded server addr**, then discovers another member. |

Security-critical slices (5, 6, 7, 8, 9) get an **adversarial-review workflow before commit**,
per the project convention.

## Open questions to resolve in-flight

1. **catcoms-net under `check-no-ambient.sh`** — libp2p brings its own timers; the crate may
   already be allowlisted. If not, Clock-driven re-registration (slice 4) must route through the
   sync layer's Clock. **Verify before slice 4.**
2. **libp2p 0.56 circuit reservation → external address:** confirm whether a granted reservation
   auto-populates the swarm external-address set or the Actor must `add_external_address(circuit)`
   explicitly (contract assumes explicit; safe either way).
3. **Where the registration membership tag physically rides:** `libp2p_core::PeerRecord` is a
   fixed signed envelope with no free slot. Confirm the rendezvous `Registration` can carry the
   tag without breaking signature verification — else encode it into a derived **sub-namespace**
   the discoverer recomputes.
4. **Inbound Remove classification** — confirmed feasible (`staged.remove_proposals()`); slice 1
   must advance `L` identically on the inbound apply path, not just the local committer.
5. **Detector defaults** (grace 30s / `min_reach` 0.20) are guesses; tune against memory-transport
   timings + a staging run; surface as `EclipseConfig`, not hard-coded.
