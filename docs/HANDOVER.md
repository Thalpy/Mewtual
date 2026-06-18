# CatComs — Handover

Authoritative current-state document. Read this first, then
[`INTERFACES.md`](INTERFACES.md) (the API/seam schema) and
[`ARCHITECTURE.md`](ARCHITECTURE.md) (decisions + the adversarial-review fixes).

## Status (as of 2026-06-18)

- **Phases 0 → 6d-1a complete.** Latest commit `89b5492`. ~110 tests passing.
- Toolchain pinned **Rust 1.89.0** (`rust-toolchain.toml`; automerge 0.10 needs it).
- 9 library crates + 1 binary; everything is tested deterministically **on one
  machine** (N in-process nodes over an in-memory transport).
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
| `catcoms-net` | libp2p `MeshService` realizing `MeshTransport` (gossipsub + request/response over Noise+yamux); tracing-instrumented. |
| `catcoms-sync` | `ChannelSync`: replication + membership over the transport — blinded topics, live gossip, catch-up, the network **join handshake**, and membership **commit propagation**. |
| `catcoms-log` | `tracing` subscriber init; `init_debug(debug, dir)` writes `debug_log_<ts>.txt`. |
| `bins/catcomsctl` | Dev CLI. `demo` runs the whole stack end-to-end; `--debug` writes a debug log. |

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
cargo run -p catcomsctl -- demo                 # found server -> invite -> join -> E2E chat -> converge
cargo run -p catcomsctl -- --debug demo         # + logs/debug_log_<ts>.txt
RUST_LOG=catcoms_sync=trace cargo run -p catcomsctl -- demo
```
The second bot (per the user) exercises the CLI; success = exit 0 + the `[OK]` line.
The demo runs both members **in one process** over the in-memory transport — it is
not yet two real OS processes (that needs the same join path over libp2p TCP; the
protocol already works, only CLI plumbing remains).

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
| 6d-1b | commit-catch-up recovery + cross-epoch op window | **next** |
| 6d-2 | concurrent-commit fork resolution + full proposal/commit linearization | planned |
| 6e | relay v2 + DCUtR, rendezvous, eclipse-resistance, blob-fetch padding | planned |
| 7 | end-to-end local integration over real sockets + security suite; multi-process `catcomsctl serve`/`join` | planned |
| 8 | product model + Tauri desktop UI (channels, fileshare browser, status, wiki) | planned |
| 9 | Android (Tauri 2 mobile): JNI keystore, foreground service, two-tier keys | planned |
| 10 | hardening: calendar, cover traffic, supply-chain attestation, metadata-index aging, security review | planned |

### Immediate next: 6d-1b
The membership-linearization design + review (verified vs openmls 0.8.1) is captured
in the workflow output — see the design-pass notes. 6d-1b should add, against the
**already-present data model** (no rewrite): a `KIND_COMMIT_CATCHUP` request, a bounded
`commit_log`, buffering out-of-order commits + ordered replay, **peer discovery for
catch-up** (retain `TransportEvent::Gossip.from`; there is no `DeviceId→PeerId` map
yet — the review flagged this), and a bounded/zeroized past-epoch channel-key window
so live ops crossing an epoch boundary still ingest (with auto-refetch on
`EpochUnavailable` rather than silent drop). Then 6d-2 does the hard part:
concurrent-commit fork resolution (openmls `clear_pending_commit` /
`fork_resolution::{readd,reboot}`; note `ExternalInit` recovery needs an
`external_pub` GroupInfo extension the config does **not** emit yet), the full
proposal/commit split with a designated committer packing replicated proposals, the
replicated InviteLedger, and **joiner-bound nonces**.

## Known limitations / deferred (the security-relevant ones)

- **Network admission is single-committer-only** (only the lowest-leaf-index member
  admits). Concurrent admits / fork resolution + cross-member single-use = 6d-2.
- **No commit-catch-up recovery yet** (6d-1b): a member that misses a membership
  commit under unreliable delivery can't yet self-heal; the 3-member test relies on
  reliable in-process delivery.
- **Live ops crossing an epoch boundary** drop (`EpochUnavailable`) until the
  past-epoch key window lands (6d-1b).
- **Persistent sealed MLS storage** is deferred: each device uses openmls's
  in-memory provider; tying group state to the Phase-1 `mls_seal_key` + SQLCipher
  (and the local metadata index) is platform/storage-phase work.
- **Metadata**: who-talks-to-whom, timing, group sizes, and (once DCUtR hole-punches)
  member IPs are the dominant residual — mitigated, not eliminated. See ARCHITECTURE §3.
- **Per-peer rate limiting / off-actor offload** of join work: a hardening follow-up.
- **`tracing` retrofit** for the earlier crypto/storage crates: deferred (user OK'd).

## Where the design/review outputs live

The big pre-implementation design pass and the per-protocol adversarial reviews ran
as background `Workflow`s; their structured output is under the session's
`tasks/<id>.output` and `tool-results/`. The load-bearing conclusions are distilled
into `ARCHITECTURE.md` §2 (initial corrections) and §4a/§4b (join + propagation), and
into the memory files. If continuing 6d-2, re-read the linearization design/review
(committer = lowest **leaf index** not device id; fork-recovery topology; nonce→joiner
binding).
