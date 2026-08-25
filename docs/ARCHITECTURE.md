# Mewtual; Architecture

This document is the in-repo source of truth for the design. It records the locked
decisions, the corrections that came out of an adversarial design review (run before any
code was written), the honest residual risks, and the phased build plan.

## 1. Locked decisions

| Area | Decision |
|------|----------|
| Stack | Rust core (shared `rlib`+`cdylib`) + Svelte 5 UI, packaged via **Tauri 2** to Linux/Windows/Android from one codebase. |
| Group crypto | **MLS (RFC 9420)** via `openmls`, ciphersuite `0x0003` (X25519 + ChaCha20-Poly1305 + SHA-256 + Ed25519), `PrivateMessage` wire format only. One MLS group == one server/connection. Per-**device** identity (a human with N devices = N leaves). |
| Channels | NOT separate groups; each channel/wiki/status/calendar/moderation document derives an independent key via the MLS exporter secret + a canonical, injective `(doc_type, doc_id)` context. |
| Delivery | Encrypted **CRDT documents** (`automerge`) synced P2P. Chat logs are append-oriented; policy documents are not assumed append-only without protocol enforcement. |
| Networking | **rust-libp2p** (QUIC + TCP + WSS; *no WebRTC in v1*). Direct reachability uses stable ports plus best-effort **UPnP IGD, IPv4 PCP/NAT-PMP, and IPv6 PCP firewall pinholes**; zero-knowledge **circuit-relay v2 + DCUtR** hole-punching and authenticated rendezvous cover harder networks; **AutoNAT v2** performs scoped dial-back testing through explicitly enabled relay/rendezvous nodes. PCPv6 binds the exact global listener address to that interface's scoped default router, requests short leases for TCP and UDP/QUIC, and honors the router's assigned lifetime up to 24 hours. AutoNAT serving is experimental and off by default; its pre-socket guard enforces exact source/target matching, direct public-address shape, global/per-prefix/per-peer rate buckets and concurrency caps. Invites embed bootstrap multiaddrs. A 60-second `JoinReply` lets the inviter (or one explicitly authorized current-member helper) dial a joiner's validated public routes back. Separately, an opted-in current member can publish a two-minute signed **switchboard offer**; a fresh, explicitly labelled assisted invite may endorse up to three such members, and the joiner must consent before they are contacted after direct routes fail. A switchboard forwards only the admission exchange to the invite's named inviter and must catch up the exact MLS Add before becoming the joiner's first member path; it never signs/adopts the Welcome or becomes a general circuit relay. |
| Invites | Strictly **single-use, device-bound** (one device per invite); revocable/expirable. |
| Files | Content-addressed over **ciphertext**. Expiry default **1 month**, adjustable global → per-server → per-file; "expired" = evicted from cache / dropped from auto-share, still re-fetchable by CID. Health checks authenticate storage seals/CIDs and decrypt file refs; repair may overwrite a corrupt local record only with authenticated, CID-valid peer bytes. |
| Moderation | One group-bound, independently signed moderation document per server. Warnings attest to bounded snapshots; kick votes are advisory; only the owner-only MLS removal path changes membership. The log is not yet protocol-enforced append-only (threat-model R7). |
| Local continuity & backup | Drafts/read positions are bounded and vault-sealed. Backup is an opaque, non-overwriting copy of a freshly snapshotted sealed vault. Secret changes atomically rewrap the same DEK; automated restore is not implied. |

## 2. Corrections from the adversarial review (must hold in the implementation)

The naive "one group, every device commits, replay old ciphertext to latecomers" design
is broken. The load-bearing fixes:

1. **Proposal/commit split.** Concurrent MLS commits fork the group. Devices replicate
   *proposals* via the CRDT; a single designated committer per epoch packs them into one
   commit; deterministic fork-resolution (lowest `commit_hash`) + loser re-issues.
2. **Snapshot-only catch-up.** Never replicate raw old-epoch frames between members (it
   contradicts forward secrecy). Latecomers receive a self-contained Automerge snapshot
   re-sealed under the current epoch. `max_past_epochs` only covers in-session reordering.
3. **Inner per-op signature.** Every `LogEntry` is signed by the author's Ed25519 leaf
   over `(doc_id, deps, mls_epoch, payload, author_device)`, verified independently of MLS
   sealing; so re-sealing is a pure transport re-wrap and history cannot be forged/omitted.
4. **Invites bound in MLS.** The invitee's KeyPackage carries a `(GroupId, invite_nonce)`
   extension validated at `Add`; the leaf is reserved in the InviteLedger before commit.
   Stops cross-group KeyPackage replay and partition double-claim.
5. **Routing topic decoupled from content epoch.** Gossipsub topic + rendezvous namespace
   derive from a **separate metadata exporter label**, rotating only on member *removal*
   (with grandfathering); not on every commit.
6. **File crypto.** Per-file wrap nonce (no nonce reuse); re-wrap content keys under each
   new epoch so a removed member cannot lazily fetch post-removal files. Already-fetched
   files cannot be un-shared (documented limit).
7. **One key hierarchy.** Platform-wrapped root → one DEK → HKDF subkeys (SQLCipher DB key,
   openmls value sealing, blob-at-rest). On Android, **two tiers**: a non-auth-bound
   SERVICE key (background sync of ciphertext) + an auth-bound CONTENT key (decrypt
   plaintext for display). `SecureKeyStore` reports a tier enum; a downgrade forces a
   passphrase before the store opens.
8. **Also:** no device key ever leaves a device (multi-device is origin-rooted
   certificates at chain depth 1; `design-multi-device.md` v2; with origin-signed
   revocation; the earlier account-key cross-cert chain design was superseded and its
   module deleted); blob-fetch padding/quantization + per-session outer
   re-encryption; eclipse resistance (≥2 rendezvous + member peer-exchange + roster-size
   check + cached addresses); decorrelated cache eviction (jitter) + holder liveness probe
   + mandatory archive-pin role; deterministic byte-identical compaction at an all-acked
   low-watermark; resumable chunked anti-entropy under relay caps; panic-revoke (any
   sibling device removes another) and `mint_invite` gated behind the auth-bound key;
   per-server "relay-only / hide my IP" mode; pure-Rust crypto on the hot path;
   metadata-index aging/re-keying.
9. **Discovery egress is peer-bound and process-bounded.** Every peer/invite/switchboard/companion
   grant route
   accepted for dialing uses one canonical supported multiaddr grammar with a non-zero TCP or
   UDP/QUIC socket and a terminal `/p2p/<PeerId>` matching the signed/discovered transport
   identity. Untrusted records use IP literals; DNS and dangerous local/private/link-local,
   multicast and transitional ranges fail closed outside the explicitly local invite case. The
   sync classifier deliberately retains non-routed documentation/benchmark literals as
   deterministic test stand-ins; those can only consume bounded retry tokens. `DiscoveryPolicy`
   charges addresses rather than peers, then
   one desktop-owned `EndpointDialScheduler` applies monotonic per-process, per-server, canonical
   Phase-0-peer, attempt, and IPv4 `/24`/IPv6 `/48` caps before submission. The parser embeds that
   peer principal in each opaque endpoint so cache, rendezvous, and pre-join callers cannot select
   different accounting identities. A direct attempt is keyed by its physical socket, excluding
   the claimed terminal id; a relayed attempt is keyed as the authenticated relay/target circuit,
   while the relay's outer host remains bounded by the prefix and process caps. PeerId/sequence
   rotation therefore cannot reset a direct socket, prefix, server, or process bucket. The transport
   refuses a dial command with no terminal peer rather than falling back to an address-only socket
   dial. Two-way reply retries use the same scheduler for each new socket pass; their proof request
   may continue only over a connection that is live when the network actor handles it, without
   consulting the ordinary recent-peer redial cache.

## 3. Honest residual risks

- **Metadata** is the dominant weakness: who-talks-to-whom, timing, group sizes, and; once
  two members hole-punch; each other's IP, are partly observable. Mitigated, not eliminated.
  AutoNAT additionally reveals a candidate address and probe timing to its configured observer;
  a positive is scoped to that server/address/moment and is not a universal reachability claim.
  No nation-state-grade metadata protection is promised.
- Automatic router mapping and IPv6 firewall pinholes intentionally expose the stable TCP and UDP/QUIC libp2p listeners to
  the internet when the local gateway grants a lease. Noise still authenticates/encrypts libp2p
  sessions and connection limits cap strangers, but a hostile/local gateway can deny mapping or
  report an unusable address. PCPv6 may also disclose a device-specific/privacy address, and a
  granted pinhole does not prove the host or upstream firewall permits traffic. A mapping is
  therefore only a candidate until AutoNAT calls that exact address back.
- Two-way reply codes and member switchboards improve signalling/routing, but cannot create a path
  from nothing. The first two mutually unreachable users still need a public IPv6/manual/router
  mapping, a configured relay, or a reachable third party. Reply punching requires both apps'
  60-second sessions to overlap and is primarily useful for QUIC; TCP simultaneous-open is not
  claimed. Switchboard offers disclose the helper's stable identities/candidate addresses to the
  invite recipient and disclose the joiner's IP/timing to the helper. Signed candidates are not
  proof of address ownership or reachability.
- Post-join discovery is self-healing only while some route survives: the desktop polls a bounded
  set of peers with authenticated PEX, retries cached/current signed address epochs with monotonic
  exponential backoff and jitter, and resets that delay on a new signed epoch or connection
  lifecycle. Native route/interface notifications trigger a debounced re-sample of the kernel's
  route-selected IPv4/IPv6 sources, while the roughly-minute pass remains a fallback. A changed
  sample republishes one address epoch; exact route ownership prevents raw-interface removal from
  withdrawing an identical mapping/manual/relay route. It intentionally does not merge
  withdrawn public IPs forever because an ISP can reassign them. Route signatures and matching
  peer ids still do not prove ownership of an IP/port before the bounded first packet is sent.
  Scheduler counters are transient and reset with the process; they charge submitted attempts, not
  actor-confirmed socket creation or completion. Duplicate suppression, cancellation, and failed
  command delivery can therefore over- or under-account relative to actual sockets. A relay circuit
  has its own attempt key so unrelated targets at one relay do not starve each other, but the shared
  outer relay socket is not separately leased at the exact-socket scope; it is bounded only by the
  relay-host prefix and process caps. Libp2p's pending-outgoing cap is still per swarm, so this slice
  does not yet provide a process-wide actor-consumed in-flight lease. A fully isolated device whose
  current address is unknown to every peer still needs
  out-of-band signalling, rendezvous/relay infrastructure, or a reachable member; swarm sampling
  cannot manufacture a route from no contact.
- Pairwise route evidence is a bounded, session-only refinement of aggregate transport liveness.
  Libp2p reports a sorted/deduplicated IPv4/IPv6/DNS/memory + TCP/QUIC/WebSocket/circuit snapshot
  after connection edges, including relay-to-direct DCUtR upgrades and partial closes. The sync
  layer accepts detail only while `PeerConnected` remains live, caps custom transports again,
  retains historical success for at most 24 monotonic hours, and never serializes it. Admission
  paths that temporarily inspect pushed proof/Welcome requests coalesce any lifecycle
  edges they dequeue into a bounded final-state handoff, which the new sync owner adopts once;
  ordinary pre-owner connection waits use a non-consuming watch. Connectivity exposes typed
  claimed-peer health/actions, refreshes on path changes even when aggregate presence
  is unchanged (plus a bounded visible-view refresh for time-derived expiry), and says “not
  connected here,” never “offline.” Dial counters describe scheduler submissions/cooldown, not
  unobserved per-address failures, and IPv6 candidate shape is not presented as an outbound-route
  test. A
  signed peer record is still a self-asserted device-to-transport binding, so this evidence is not
  proof that the member controls that live transport key or is reachable from another network.
- A **fully compromised device** exposes its current keys and plaintext; PCS only heals
  *after* the device is removed.
- **Already-fetched files cannot be un-shared.**

## 4. Engineering seams (every layer is written against these)

- **`Clock`** (in `catcoms-rt`): all time flows through it; no layer calls the OS clock
  directly. Enforced by `scripts/check-no-ambient.sh` in CI.
- **`MeshTransport`** (in `catcoms-rt`): pub/sub + addressed request/response. Node logic
  runs over the deterministic in-memory `MemNetwork` in tests and over rust-libp2p in
  production, unchanged.
- **Canonical wire format** (`catcoms-wire`): length-prefixed, fixed-width, injective
  encoding; the basis of collision-free key-derivation contexts.

## 4a. Network join handshake (6c)

A fresh device joins over the wire by sending its invite-bound KeyPackage to the
inviter (request/response); the inviter validates and returns a Welcome. Adversarial
review (run before commit) hardened it:

- The joiner's KeyPackage init key is **not secret**, so a malicious inviter/relay
  could otherwise add it to a group *they* control and return a valid Welcome. The
  invite therefore carries the **inviter's public key**; the joiner authenticates
  the invite (`verify_self`), the admitter **signs the Welcome**, and the joiner
  verifies that signature against the invite's inviter and re-checks `group_id`.
- Only the invite's **named inviter admits** over the network (so the joiner can
  authenticate the response). `serve_join` runs cheap invite checks *before* the
  expensive KeyPackage validation, and oversized control requests are dropped.

## 4b. Membership commit propagation (6d-1a)

The Add commit (previously discarded) is now captured and fanned out on a per-group
**control topic**; every member applies it via `process_incoming` and advances to
the same epoch; so a multi-member join converges and a non-admitting member can
decrypt the new joiner's ops. A design+adversarial-review pass (verified against
the openmls 0.8.1 source) showed the "safe by construction" claim was only *assumed*,
so safety is **enforced**: only the **designated committer** (lowest leaf index) may
admit, which prevents concurrent commits from forking the epoch chain. Tested with a
3-member join + a non-committer-admit rejection.

## 4c. Missed-commit recovery + past-epoch key window (6d-1b)

A member that misses a control-topic commit (unreliable delivery / brief offline)
self-heals: an out-of-order commit is buffered (`pending_commits`), a new
`KIND_COMMIT_CATCHUP` request fetches the gap from any member's bounded `commit_log`,
and the missed MLS commits are **replayed in epoch order** through `process_incoming`.
Every retained buffer/queue is hard-bounded so an untrusted peer cannot force
unbounded allocation; a forged future commit is gap- and size-capped and fails MLS
verification at apply time (its only residual cost is one deduped catch-up request).
Separately, a bounded **past-epoch channel-key window** (`snapshot_epoch_keys` before
each advance → `Zeroizing` keys, evicted past `max_past_epochs`) lets an op sealed
just before an epoch boundary still decrypt (`ingest_with_key`, inner signature still
verified) instead of being dropped as `EpochUnavailable`; deeper gaps fall back to
auto-queued document/commit catch-up. Peer discovery is by remembering inbound
`Gossip.from`/`Request.from` (no `DeviceId→PeerId` directory yet).

An adversarial review (background `Workflow`) hardened this before commit. The
load-bearing fix: the **catch-up serve endpoints are members-only**. A requester
proves current membership by signing `("catcoms/catchup-auth/v1" ‖ group_id ‖ kind ‖
body ‖ requester_pubkey ‖ timestamp)` with its MLS leaf key; the server serves only
if that key content-addresses a current member, the timestamp is fresh, and the
signature verifies; so an outsider cannot harvest a group's id, member device ids,
or history from these endpoints. (Residual: within-freshness-window replay of a
captured signed request, closed by the Noise transport in production; a server nonce
challenge or authenticated-peer binding is the full fix, with 6e.) Also folded in:
hard response-size bounds on the *serving* side, `committer_device` validated against
the designated committer on the inbound apply path, and explicit caps on every
recovery buffer/queue.

**Still deferred, with the data model already in place (no rewrite):**
- **6d-2**; concurrent-commit fork resolution + the full RFC 9420 proposal/commit
  split (designated committer packs replicated proposals; deterministic lowest-hash
  tie-break; openmls `clear_pending_commit` rollback / `fork_resolution` heal),
  plus the replicated InviteLedger (single-use across members) and joiner-bound
  nonces. Until then network admission is single-committer only.
- Per-peer rate limiting / off-actor offload of join work.

## 4d. Product operations and moderation plane

The desktop keeps operational observations separate from protocol claims. **Storage health** walks
the current file index and validates every manifest and referenced chunk through the storage seal,
CID and file-layer key. One report per server is cached for the process session and augmented with a
deduplicated category/pin/largest-file inventory; category local-byte totals remain estimates, while
the verified ciphertext total is exact. **Repair** is an explicit network action: it re-fetches only
missing or unreadable content from authenticated members, re-runs verification, and replaces that
cache. **Connectivity assistant**
reports the live peer/path evidence already available to the node; it is diagnostic and never a
proof that every remote member or future network path is reachable.

Moderation history lives in `DocType::Moderation` (stable tag 14, document id 0), not in a chat
channel. Each event/vote has a canonical Ed25519 signature binding its semantic fields and group id.
The app also binds the signer device to its certified member origin and interprets authority through
the current owner-signed role state. Warning evidence therefore survives a live message edit/delete
and is attributable to the moderator who observed it. It does not retroactively authenticate the
message author, prove historical role state, or prevent a modified member from deleting a CRDT root
entry; those are the explicit R7 residual. A vote can never invoke membership mutation. The owner
must resolve a case, and removal reuses the protocol-enforced MLS Remove flow.

Frontend continuity state (currently composer drafts and per-channel read marks) is stored as a
bounded vault-sealed record rather than browser plaintext. An offline backup first snapshots live
actors and persists the registry, then copies the already-sealed vault tree to a new destination
without following links or overwriting an existing backup. Restore remains deliberately staged work:
it needs a locked-vault import path, full verification, atomic swap and rollback before it can be
safe to expose. Export creates another permanent offline guessing surface and leaves filesystem
metadata visible. Changing the live secret atomically rewrites only the DEK wrapper; it cannot revoke
an older export and never rotates the server/blob encryption keys.

## 5. Roadmap (test-gated, block by block)

0. Workspace + `Clock`/`Transport` seams + canonical wire format + CI/lint gate.
1. Device identity + unified key hierarchy (`SecureKeyStore` tiers; desktop impls).
2. MLS `ServerGroup` (local, no network) + channel-key derivation.
3. Invites (MLS-bound, single-use, device-bound) + InviteLedger.
4. Replication engine: inner-signed ops, proposal/commit split, snapshot catch-up,
   deterministic compaction.
5. Storage & retention: CID blockstore, 3-scope expiry, clock-injected GC, file crypto.
6. Real mesh: libp2p, relay-v2/DCUtR, rendezvous, scoped gossipsub, eclipse resistance.
7. End-to-end local integration over real sockets + consolidated security suite.
8. Product model + Tauri desktop UI (channels, fileshare browser, status, wiki).
9. Android (Tauri 2 mobile): JNI keystore, foreground service, two-tier keys.
10. Hardening: calendar, cover traffic, supply-chain attestation, security review.
