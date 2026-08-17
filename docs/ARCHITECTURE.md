# Mewtual; Architecture

This document is the in-repo source of truth for the design. It records the locked
decisions, the corrections that came out of an adversarial design review (run before any
code was written), the honest residual risks, and the phased build plan.

## 1. Locked decisions

| Area | Decision |
|------|----------|
| Stack | Rust core (shared `rlib`+`cdylib`) + React UI, packaged via **Tauri 2** to Linux/Windows/Android from one codebase. |
| Group crypto | **MLS (RFC 9420)** via `openmls`, ciphersuite `0x0003` (X25519 + ChaCha20-Poly1305 + SHA-256 + Ed25519), `PrivateMessage` wire format only. One MLS group == one server/connection. Per-**device** identity (a human with N devices = N leaves). |
| Channels | NOT separate groups; each channel/wiki/status/calendar derives an independent key via the MLS exporter secret + a canonical, injective `(doc_type, doc_id)` context. |
| Delivery | Encrypted append-only **CRDT logs** (`automerge`) synced P2P. Channels, wiki, status, calendar are all CRDT documents. |
| Networking | **rust-libp2p** (QUIC + TCP + WSS; *no WebRTC in v1*). Zero-knowledge **circuit-relay v2 + DCUtR** hole-punching and an authenticated rendezvous. Invites embed bootstrap multiaddrs. |
| Invites | Strictly **single-use, device-bound** (one device per invite); revocable/expirable. |
| Files | Content-addressed over **ciphertext**. Expiry default **1 month**, adjustable global → per-server → per-file; "expired" = evicted from cache / dropped from auto-share, still re-fetchable by CID. |

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

## 3. Honest residual risks

- **Metadata** is the dominant weakness: who-talks-to-whom, timing, group sizes, and; once
  two members hole-punch; each other's IP, are partly observable. Mitigated, not eliminated.
  No nation-state-grade metadata protection is promised.
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
