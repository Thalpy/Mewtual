# CatComs — Handover

Authoritative current-state document. Read this first, then
[`INTERFACES.md`](INTERFACES.md) (the API/seam schema) and
[`ARCHITECTURE.md`](ARCHITECTURE.md) (decisions + the adversarial-review fixes).

## Status (as of 2026-06-22)

- **Phases 0 → 6e-3c complete; 6e-3d (rendezvous discovery + eclipse-resistance) in
  progress — slices 1–5 of 9 done.** 151 tests passing.
- Both CRITICALs the 6e-3d design pass found are **closed and adversarially reviewed**:
  **A1** (the pre-existing bug where the gossip topics hashed the plaintext-invite
  `group_id`, so any invite-holder could read all topics) and **Sybil-C1** (the
  catch-up source-trust hole). See [`design-6e-rendezvous.md`](design-6e-rendezvous.md).
- Toolchain pinned **Rust 1.89.0** (`rust-toolchain.toml`; automerge 0.10 needs it).
- 9 library crates + 1 binary. The protocol layers are tested deterministically with
  N in-process nodes over an in-memory transport; the mesh is *additionally* tested
  over **real libp2p** — the memory transport (real swarms/Noise/req-resp), TCP
  loopback (real sockets, multi-process `serve`/`join`), a circuit relay, a DCUtR
  hole-punch upgrade, and a rendezvous register→discover.
- Local-only repo (`git init`'d, no remote). Commits are linear on `main`,
  one per phase/block.

## What CatComs is

A peer-to-peer, serverless, end-to-end-encrypted, invite-only group comms system —
"Signal + federation". Each "server/connection" is its own MLS (RFC 9420) group;
per-device identity; encrypted CRDT documents (channels/wiki/status/calendar)
replicated over a libp2p mesh with zero-knowledge relays. Targets Linux, Windows,
Android via a Rust core + (eventually) Tauri 2.

The four locked decisions and the pre-implementation adversarial-review fixes are
in [`ARCHITECTURE.md`](ARCHITECTURE.md) §1–§2 — **read them; they constrain everything.**

## Crate map

| Crate | Responsibility |
|-------|----------------|
| `catcoms-wire` | Canonical, injective, length-prefixed codec; domain-separated key-derivation contexts (`DocType`, `exporter_context`). |
| `catcoms-rt` | The **seams**: `Clock`, RNG (`OsCryptoRng`/`CryptoRngCore`), and `MeshTransport` (pub/sub + request/response) with an in-memory `MemNetwork`/`Hub` for tests. |
| `catcoms-crypto` | Content-addressed `DeviceId`/`UserId`; Ed25519 device/account keys; device-cert chains + `Roster`; the unified key hierarchy (`Dek`→HKDF subkeys), XChaCha20 `seal`/`unseal`, tiered `SecureKeyStore`. |
| `catcoms-mls` | MLS group core (openmls 0.8): `MlsDevice`, `ServerGroup` (create/add/remove/process/epoch/`channel_secret`), single-use device-bound `InviteToken` + `InviteLedger`, `AddOutcome`, `designated_committer`. |
| `catcoms-replication` | Encrypted CRDT docs (automerge 0.10): inner-signed `SignedOp`, `SealedOp` (per-epoch channel-key sealing), `EncryptedDoc` (edit/ingest/catch-up). |
| `catcoms-storage` | Content-addressed `Cid` blob stores (mem + fs); per-file encryption (`FileRef`, per-file wrap nonce); `RetentionIndex` (3-scope expiry, GC with decorrelated eviction + `HolderOracle` probe). |
| `catcoms-net` | libp2p `MeshService` realizing `MeshTransport` (gossipsub + request/response over Noise+yamux). NAT traversal: relay-client + **circuit-relay-v2** + **DCUtR** hole-punch (`next_direct_upgrade()`). Standalone zero-knowledge infra: `build_relay_swarm`/`run_relay` and `build_rendezvous_swarm`/`run_rendezvous` (`RelayBehaviour`/`RendezvousBehaviour`). **Rendezvous client** in `MeshBehaviour`: `rendezvous_register`/`rendezvous_discover`; discovered records surface via `next_discovered()` and are **never auto-dialed**. `connection_limits` on every swarm. Tracing-instrumented. |
| `catcoms-sync` | `ChannelSync`: replication + membership over the transport. Blinded **member-only gossip topics keyed under `ns_secret_L`** that rotate on member removal (the routing label `L`), with a grandfathered re-subscription window. The network **join handshake** now also transfers the **routing state** (sealed, signature-bound) so joiners derive the same topics/namespaces. Membership **commit propagation**; **missed-commit recovery** with **signed catch-up responses** + a **two-pool peer model** (untrusted candidates vs verified `member_peers`); a bounded zeroized **past-epoch key window**; `rendezvous_namespaces()`; and `SyncStats`. |
| `catcoms-log` | `tracing` subscriber init; `init_debug(debug, dir)` writes `debug_log_<ts>.txt`. |
| `bins/catcomsctl` | Dev CLI. `demo` runs the whole stack end-to-end (in-process); `serve`/`join` run it across **real OS processes over TCP** (optionally `serve --relay`); `relay` and `rendezvous` run the zero-knowledge infra nodes; `recover` drives the 6d-1b miss-and-heal path; `--debug`/`--stats`. |

## Build / verify ritual (run before every commit)

```sh
cargo build --all
cargo clippy --all-targets --all-features -- -D warnings   # must be clean
cargo fmt --all -- --check                                  # must be clean
cargo test --all                                            # all green
bash scripts/check-no-ambient.sh                            # ambient-dependency gate
```

PowerShell helper to sum test results (Windows dev box):
```pwsh
$out = cargo test --all 2>&1 | Out-String
([regex]::Matches($out,"(\d+) passed")|%{[int]$_.Groups[1].Value}|measure -sum).Sum
```
(`-match "FAILED"` is a false positive — it case-insensitively matches "0 failed".)

## Dev loop

```sh
cargo run -p catcomsctl -- demo                 # in-process: found -> invite -> join -> E2E chat -> converge
cargo run -p catcomsctl -- recover --stats      # 6d-1b: a member misses a commit and self-heals
cargo run -p catcomsctl -- --stats demo         # print per-node SyncStats counters
RUST_LOG=catcoms_sync=trace cargo run -p catcomsctl -- demo
# Real multi-process over libp2p (terminals 1+2; add --host <ip> to cross machines):
cargo run -p catcomsctl -- serve --port 9000 --invite-file invite.txt
cargo run -p catcomsctl -- join  --invite-file invite.txt
# NAT traversal + discovery infra (each runs until Ctrl-C):
cargo run -p catcomsctl -- relay --port 4000        # zero-knowledge circuit relay
cargo run -p catcomsctl -- rendezvous --port 5000   # zero-knowledge rendezvous
```
`demo` runs both members in one process over the in-memory transport; `serve`/`join`
run the *same* join + catch-up path across **separate OS processes over real libp2p
TCP** (verified, incl. through a relay).

## Working conventions (important — keep doing these)

- **Block by block, test-gated.** Build a coherent block, make all tests pass + the
  full ritual clean, commit, then continue. Efficacy over speed.
- **No ambient time/RNG.** All time flows through `Clock`, all randomness through an
  injected `CryptoRngCore`. The only sanctioned OS sources are
  `catcoms-rt/src/{clock,rng}.rs`; `scripts/check-no-ambient.sh` enforces it (extend
  its allowlist deliberately).
- **Adversarial-review workflows for security-critical protocol code.** This caught
  a genuine **HIGH** in the network join (group-substitution) and corrected the
  membership-linearization design against the openmls source. Pattern: design/build,
  then run a `Workflow` of hostile reviewers (crypto / DoS / guarantee-preservation /
  distributed-systems), fold findings in, then commit. Don't skip it for membership /
  admission / key-handling changes.
- **Commit messages** end with the `Co-Authored-By: Claude Opus 4.8 (1M context)`
  line. Use `git commit -F <file>` for messages containing `==`/quotes (PowerShell
  here-strings mangle them).
- **Memory**: `~/.claude/projects/.../memory/` holds durable facts; this repo's
  `docs/` holds the detailed handover. Keep both current.

## Roadmap & status

| Phase | Block | State |
|------:|-------|-------|
| 0 | workspace, `Clock`/`Transport` seams, wire format, CI | ✅ `016e3a0` |
| 1 | device identity + unified key hierarchy | ✅ `e5a292e` |
| 2 | MLS group core (local) | ✅ `196bf14` |
| 3 | single-use device-bound invites | ✅ `30bd856` |
| 4 | encrypted CRDT replication | ✅ `7db8efd` |
| 5 | storage & retention | ✅ `b797ffc` |
| 6a | libp2p `MeshService` over the seam | ✅ `0949151` |
| 6b | channel sync over the mesh + tracing | ✅ `c947021` |
| — | `catcomsctl` CLI + debug-file logging | ✅ `91c53d3` |
| 6c | network join handshake (inviter-authenticated) | ✅ `61d990c` |
| 6d-1a | membership commit propagation (single committer) | ✅ `89b5492` |
| 6d-1b | commit-catch-up recovery + ordered replay + past-epoch key window | ✅ `16a6427` |
| 6d-2a (1/2) | signed commit records + authorize-by-signature gate | ✅ `09d4cc5` |
| 6d-2a (2a) | MLS staged-commit primitives (stage/merge/abort) | ✅ `e577a11` |
| 6d-2a (2b) | sync-layer fork resolution (commit_id tie-break + contest window) | ✅ `939eb41` |
| 6d-2a (2c) | two-phase staged-Add join (provisional Welcome push) + review fixes I2–I6 | ✅ `42f9b7f` |
| 6d-2b (1) | **single-serializer remove** (members *request*; the designated committer alone commits) — the convergence-safe model, on by default | ✅ `63ac788` |
| 6d-2b (2) | **all-members apply-time Add-binding validation** (every member rejects an Add not bound to this group / its own leaf key) | ✅ `0a1a276` |
| 6d-2b (3…) | by-value proposal batching · history-derived single-use · committer-decoupled admission | planned |
| 6e (1) | **full stack over real libp2p** — join handshake + encrypted catch-up over `MeshService` (Noise + request/response) | ✅ `f1d2713` |
| 6e (2) | **multi-process `catcomsctl serve`/`join` over TCP** — two OS processes, real sockets, verified | ✅ `73904f1` |
| 6e (3a) | **relay infrastructure** — relay-capable swarm (relay-client + DCUtR + identify) + relay server; a client reserves a circuit slot | ✅ `84827b1` |
| 6e (3b) | **end-to-end through a relay** — `catcomsctl relay`; `serve --relay` reserves + advertises the circuit address; `join` dials it. Verified across 3 real processes | ✅ `2f196b1` |
| 6e (3c) | **DCUtR hole-punch** — a relayed link auto-upgrades to a direct one; the upgrade is surfaced via `MeshService::next_direct_upgrade()`. TCP-loopback test asserts the upgrade event path | ✅ `7173e5e` |
| 6e-3d | **rendezvous discovery + eclipse-resistance** — 9 slices; design contract in [`design-6e-rendezvous.md`](design-6e-rendezvous.md) (`bd5e0d1`) | in progress (1–5/9) |
| 6e-3d-1 | per-removal routing secret `ns_secret_L` + `rendezvous_namespaces()` (rotate on removal; removed-member exclusion) | ✅ `eb6a952` |
| 6e-3d-2a | routing-state **transfer on join** (sealed, epoch-keyed) so joiners converge on topics/namespaces | ✅ `cb5c168` (prep `837bd4f`) |
| 6e-3d-2b | **re-key gossip topics from `ns_secret_L`** (member-only, rotate on removal) — **closes A1**; adversarially reviewed | ✅ `1d9e3f2` |
| 6e-3d-3 | zero-knowledge **rendezvous server** + `catcomsctl rendezvous` | ✅ `924df09` |
| 6e-3d-4 | **rendezvous client** in `MeshBehaviour` (register/discover, surfaced, **no auto-dial**) + `connection_limits` | ✅ `fe1ed2a` |
| 6e-3d-5 | **signed catch-up responses + two-pool peer model** — **Sybil-C1** source-trust; adversarially reviewed | ✅ `726691b` |
| 6e-3d-6…9 | DiscoveryPolicy (pre-dial membership tag + capped dial budget) · member PEX · advisory eclipse detector + cache · invite rewiring + end-to-end | planned (security-critical; each adversarially reviewed) |
| 7 | end-to-end local integration over real sockets + security suite | planned |
| 8 | product model + Tauri desktop UI (channels, fileshare browser, status, wiki) | planned |
| 9 | Android (Tauri 2 mobile): JNI keystore, foreground service, two-tier keys | planned |
| 10 | hardening: calendar, cover traffic, supply-chain attestation, metadata-index aging, security review | planned |

### Earlier blocks (history)
6d-1b (missed-commit recovery + past-epoch key window) and 6d-2 (fork resolution +
the convergence-safe single-serializer membership model) are complete; their details
live in the commit messages and [`design-6d2.md`](design-6d2.md). The default config
stays **single designated committer** — the concurrent-committer / fork path is OFF
by default (`max_committer_rank=0`) pending **I1** (a wall-clock contest window can
converge honest nodes differently under async timing); re-read `design-6d2.md` before
touching it.

## 6e-3d — current focus (rendezvous discovery + eclipse-resistance)

**Read [`design-6e-rendezvous.md`](design-6e-rendezvous.md) before continuing** — the
9-slice contract from a 7-agent design+review workflow, plus the per-slice
adversarial-review outcomes (2b and 3d-5 are recorded there).

**Goal:** members find each other with no hard-coded bootstrap addresses, and an
attacker cannot isolate (eclipse) a member. Everything is built on a per-removal
routing secret `ns_secret_L`:

- **Foundation (3d-1/2a/2b, done).** `ns_secret_L` is snapshotted at each member
  **removal** (counter `L`), retained `{L-2,L-1,L}` in `ChannelSync`. The blinded
  gossip topics **and** rendezvous namespaces both derive from it (keyed BLAKE3), so a
  non-member can't compute them and they **rotate on removal** (forward secrecy for
  routing metadata). Because the secret is epoch-specific, the **join handshake
  transfers it** (sealed, bound into the inviter signature) so every member — founder,
  joiner, *post-removal* joiner — derives identical topics. Re-keying the topics
  **closed the pre-existing A1 CRITICAL**.
- **Discovery (3d-3/4, done).** A zero-knowledge rendezvous **server** (`catcomsctl
  rendezvous`) and a **client** in `MeshBehaviour`: register a signed peer record under
  a blinded namespace, discover others. Discovered records are **surfaced, never
  auto-dialed** — the dial decision (and eclipse-resistance) lives a layer up.
- **Source trust (3d-5, done).** Commit catch-up responses are now **signed** by the
  responder's MLS leaf key, bound to the request; a **two-pool** model separates
  untrusted candidates from verified `member_peers`. **Closed Sybil-C1.**

**Remaining (3d-6…9, planned, security-critical → each adversarially reviewed):**
- **3d-6 `DiscoveryPolicy`** (a new pure `catcoms-discovery` crate): the **pre-dial
  membership tag** (deferred from 3d-5 — the "where the tag rides" question resolves
  here) + a Clock-paced **capped dial budget** + cross-rendezvous union/freshness. Also
  absorbs the 3d-5 review's deferred hardening — a per-request **nonce** + an **epoch
  bind** in the catch-up transcripts (anti-replay) and the candidate-flood / cold-start
  fix.
- **3d-7** member **PEX** · **3d-8** advisory **eclipse detector** + cross-session
  cache · **3d-9** invite rewiring (`rendezvous: Vec<String>`, INVITE_DOMAIN bump) +
  end-to-end + pre-join `join_ns`.

## Known limitations / deferred (the security-relevant ones)

- **Network admission is single-committer-only** (only the lowest-leaf-index member
  admits). Concurrent admits / fork resolution + cross-member single-use = 6d-2.
- **Commit catch-up needs a peer that still holds the commit.** A member behind by
  more than a serving peer's `max_commit_log` window can't recover via commit
  catch-up — a full snapshot rejoin (deferred) is required; the gap is logged and a
  bad source is excluded, but exhausting all sources surfaces only a warning (a
  recovery *event* to the app is a follow-up).
- **Catch-up auth is replay-bounded, not replay-proof.** Catch-up *responses* are now
  signed by a current member and bound to the request (6e-3d-5), and requests carry a
  fresh signed timestamp — but both still rest on `req_ts` uniqueness within
  `MAX_REQUEST_AGE_MS` (60s), not a per-request nonce. A per-request **nonce** + an
  **epoch bind** in both transcripts land in **6e-3d-6** (the only effect of the
  current gap is a *transient* mis-preference; applied records are independently
  committer-authorized, so no junk state). The Noise transport confines requests to the
  peer's own session in production.
- **6e-3d discovery (in progress).** Discovery infrastructure (rendezvous server +
  client) is in, but the **eclipse-resistance policy is not yet built**: discovered
  records are surfaced but the **pre-dial membership tag, the capped dial budget, member
  PEX, and the advisory eclipse detector are 6e-3d-6…8**. Until then an untrusted-
  candidate connect-flood can evict an honest peer from the bounded candidate set on a
  cold-start node (availability only — the two-pool model keeps trust intact). See
  [`design-6e-rendezvous.md`](design-6e-rendezvous.md) for the full residual list.
- **A forged future `CommitRecord`** on the control topic is bounded (gap +
  buffer caps, deduped catch-up) and fails MLS verification at apply time. Per-peer
  rate limiting + exponential catch-up backoff are a hardening follow-up.
- **Persistent sealed MLS storage** is deferred: each device uses openmls's
  in-memory provider; tying group state to the Phase-1 `mls_seal_key` + SQLCipher
  (and the local metadata index) is platform/storage-phase work.
- **Metadata** is the dominant residual: who-talks-to-whom, timing, group sizes, the
  member IPs a DCUtR upgrade reveals to the peer, and — now — a **rendezvous** node
  learning `namespace ↔ IP ↔ timing` for the registration TTL (a higher-value target
  than a relay; querying ≥2 rendezvous doubles the operators who see it). Per-rendezvous
  namespace diversification removes the cross-operator join key; rotation-on-removal
  limits long-term linkage but leaks a removal-cadence signal. Mitigated, not eliminated
  (≥2 relays/rendezvous, cover traffic, staying relayed). Relays/rendezvous only ever
  see Noise+MLS ciphertext / opaque namespaces. See ARCHITECTURE §3.
- **Per-peer rate limiting / off-actor offload** of join work: a hardening follow-up.
- **`tracing` retrofit** for the earlier crypto/storage crates: deferred (user OK'd).

## Where the design/review outputs live

Design passes and adversarial reviews run as background `Workflow`s; their structured
output is under the session's `tasks/<id>.output`. The load-bearing conclusions are
distilled into the design docs and the memory files — **read the design doc for the
block you're touching**:
- `ARCHITECTURE.md` §1–§2 — the four locked decisions + the initial corrections;
  §4a/§4b — join + commit propagation; §3 — honest residual risks.
- [`design-6d2.md`](design-6d2.md) — fork resolution / single-serializer membership
  (committer = lowest **leaf index**; the **I1** gate keeping concurrent committers off).
- [`design-6e-relay.md`](design-6e-relay.md) — relay-v2 + DCUtR.
- [`design-6e-rendezvous.md`](design-6e-rendezvous.md) — **the active block**:
  rendezvous discovery + eclipse-resistance, the 9-slice contract, and the recorded
  per-slice adversarial-review outcomes (A1/2b and Sybil-C1/3d-5) with their deferred
  follow-ups.
