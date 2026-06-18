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
| 6a | Mesh transport: libp2p `MeshService` over the seam (gossipsub + req/resp) | done |
| 6b | Channel sync over the mesh (live gossip + catch-up); + diagnostics (tracing) | done |
| 6c | Network join handshake (inviter-authenticated, single-use over the wire) | done |
| 6d | Relay v2 + DCUtR, rendezvous, eclipse-resistance, proposal/commit linearization | planned |
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
  catcoms-net     libp2p MeshService realizing the MeshTransport seam (gossipsub +
                  request/response over Noise+yamux)
  catcoms-sync    ChannelSync: replicate encrypted CRDT docs over any MeshTransport
                  (blinded topics, live gossip, request/response catch-up)
  catcoms-log     tracing/diagnostics init for binaries and tests (RUST_LOG) +
                  toggleable debug-to-file (debug_log_<timestamp>.txt)
bins/
  catcomsctl      dev CLI driving the whole stack (`demo`), with --debug file logs
```
More crates (`catcoms-core`, `catcoms-harness`) land with their phases.

Diagnostics use the `tracing` facade; binaries/tests install a subscriber via
`catcoms_log::init()` / `init_test()`. Filter with e.g.
`RUST_LOG=catcoms_net=debug,catcoms_sync=trace`.

## Try it

`catcomsctl demo` composes the whole stack in one process: Alice founds a server,
mints a single-use invite, Bob redeems it and joins the MLS group, both open a
channel over the mesh, and exchange end-to-end-encrypted chat that converges.

```sh
cargo run -p catcomsctl -- demo                       # full end-to-end demo
cargo run -p catcomsctl -- --debug demo               # + writes logs/debug_log_<timestamp>.txt
cargo run -p catcomsctl -- --debug --log-dir /tmp demo
```

Diagnostics use the `tracing` facade. `--debug` writes a verbose
`debug_log_<timestamp>.txt`; console verbosity is set with `RUST_LOG`
(e.g. `RUST_LOG=catcoms_sync=trace cargo run -p catcomsctl -- demo`).

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
