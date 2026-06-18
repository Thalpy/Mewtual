# CatComs

A peer-to-peer, serverless, end-to-end-encrypted, invite-only group communications
system. Mental model: **Signal + federation**.

- **Encrypted groups** — each "server/connection" is its own MLS (RFC 9420) group with
  an independent key schedule; per-device identity; forward secrecy + post-compromise
  security.
- **Serverless mesh** — nodes find and sync with each other over rust-libp2p with
  zero-knowledge relay/rendezvous nodes for NAT traversal. Relays only ever route
  Noise+MLS ciphertext.
- **Eventually-consistent** — channels, the wiki, status posts and calendar events are
  all encrypted append-only CRDT documents; offline members catch up on reconnect.
- **Content-addressed files** — images/audio kept locally for inline embeds; large/old
  files expire from cache (default 1 month, adjustable) but stay re-fetchable on demand
  by content address.
- **One codebase, three targets** — a shared Rust core packaged via Tauri 2 to Linux,
  Windows and Android.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the full design, the security
corrections from the adversarial review, and the phased roadmap.

## Status

Early construction, built **block-by-block with tests gating each phase**.

| Phase | Block | State |
|------:|-------|-------|
| 0 | Workspace, `Clock`/`Transport` seams, canonical wire format, CI | done |
| 1 | Device identity + unified key hierarchy | done |
| 2 | MLS group core (local) | done |
| 3 | Single-use device-bound invites | done |
| 4 | CRDT replication (inner-signed ops, snapshot catch-up) | done |
| 5 | Storage & retention | done |
| 6 | Real mesh (libp2p, relay, discovery) | planned |
| 7 | End-to-end local integration | planned |
| 8 | Product model + Tauri desktop UI | planned |
| 9 | Android | planned |
| 10 | Hardening + calendar | planned |

## Layout

```
crates/
  catcoms-wire    canonical, injective, length-prefixed wire encoding
  catcoms-rt      core runtime seams: Clock + RNG + MeshTransport (+ in-memory test impls)
  catcoms-crypto  identity (content-addressed ids, device-cert chains, revocation,
                  roster), and the unified key hierarchy / tiered key store
  catcoms-mls     MLS group core (openmls): one group per server, pinned ciphersuite,
                  ServerGroup lifecycle, and per-channel key derivation
  catcoms-replication  encrypted CRDT documents (automerge): inner-signed ops,
                  per-epoch sealing, and snapshot catch-up for late members
  catcoms-storage  content-addressed blob stores, per-file encryption (per-file wrap
                  nonce), and the retention engine (3-scope expiry, GC with
                  decorrelated eviction + holder probe, refetchable on eviction)
```
More crates (`catcoms-net`, `catcoms-core`, `catcoms-harness`) land with their phases.

## Build & test

```sh
cargo build
cargo test --all
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
bash scripts/check-no-ambient.sh   # no ambient time/RNG outside the seam crate
```

Fuzzing (nightly):

```sh
cargo +nightly fuzz run roundtrip -p catcoms-wire-fuzz   # from crates/catcoms-wire/fuzz
```
