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
| 6d-1a | Membership commit propagation (single designated committer) — multi-member join converges | done |
| 6d-1b | Missed-commit recovery (commit catch-up + ordered replay, peer discovery) + past-epoch key window | done |
| 6d-2 | Fork resolution + single-serializer membership (convergence-safe; concurrent-committer path fenced off until I1) | done |
| 6e-1/2 | Full stack over real libp2p; multi-process `serve`/`join` over TCP | done |
| 6e-3a/b/c | Circuit relay v2 (reserve + dial-through) + DCUtR hole-punch (NAT traversal) | done |
| 6e-3d-1/2 | Per-removal routing secret + member-only rotating gossip topics + join-time transfer (closes the pre-existing topic-disclosure bug) | done |
| 6e-3d-3/4 | Zero-knowledge rendezvous server + client (register/discover, no auto-dial) | done |
| 6e-3d-5 | Signed catch-up responses + two-pool peer model (catch-up source trust) | done |
| 6e-3d-6 | `catcoms-discovery` `DiscoveryPolicy` (ranked, bounded dial plan) + catch-up nonce/epoch anti-replay + pre-dial membership tag | done |
| 6e-3d-7 | Member PEX (`KIND_PEX`): members supply each other dialable signed peer records (members-only, responder-signed, capped/rate-limited) | done |
| 6e-3d-8 | Advisory eclipse detector (D/R/S, hysteresis, never gates) + cross-session address cache (tamper-detected on load) | done |
| 6e-3d-9 | Invite rewiring (`rendezvous` vector, `INVITE_DOMAIN` v2) + pre-join `join_ns` + `serve --rendezvous`/`join` discover→dial→join (no hard-coded address) | done |
| 7a | **Direct** full-stack join over **real TCP sockets** (MLS join + channel converge over OS sockets) | done |
| 7b | Consolidated **security suite** (threat-model map + cross-layer adversarial scenarios) | done |
| 7c | **Rendezvous-discovered** join over real TCP (no hard-coded server address) | done |
| 7d | **Relayed** full-stack join over real TCP (NAT traversal; server reachable only via a relay) | done |
| 7e | **DCUtR-upgraded** full-stack path over real TCP (relayed join hole-punches to a direct link) | done |
| 8a | `catcoms-app` **product model** (UI-facing `Server` facade + canonical message schema) | done |
| 8b-1 | async **event-stream actor** (commands in / events out) | done |
| 8b-2 | **Tauri 2 + Svelte desktop app** (`apps/desktop`): found → #general → send/read | done |
| 8c | **invite + join in the UI** — two app instances talk over real TCP (found → copy invite → paste/join) | done |
| 8d | **multi-channel** — IRC-style name-addressed channels + channel-list sidebar | done |
| 8e | **member roster + chat polish** — live Members panel + own-message bubbles | done |
| 8… | discovery/relay in the UI · multi-server · fileshare · status · wiki | planned |
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
                  (blinded ns_secret_L topics, live gossip, request/response catch-up,
                  member PEX, the pre-join join_ns, membership tags)
  catcoms-discovery  pure eclipse-resistance: DiscoveryPolicy (ranked, bounded dial
                  plan; the only thing that decides what to dial), advisory eclipse
                  detector, cross-session address cache (no I/O, no ambient time/RNG)
  catcoms-app     the UI-facing product model: Server facade, canonical chat-message
                  schema, and the async event-stream actor (commands in / events out)
  catcoms-log     tracing/diagnostics init for binaries and tests (RUST_LOG) +
                  toggleable debug-to-file (debug_log_<timestamp>.txt)
bins/
  catcomsctl      dev CLI driving the whole stack (`demo`), with --debug file logs
apps/
  desktop         Tauri 2 + Svelte 5 desktop app over the catcoms-app actor bridge
                  (its own cargo workspace; `npm install && npm run tauri dev`)
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
cargo run -p catcomsctl -- demo                       # full end-to-end demo (in-process)
cargo run -p catcomsctl -- recover --stats            # 6d-1b: miss a membership commit and self-heal
cargo run -p catcomsctl -- --debug demo               # + writes logs/debug_log_<timestamp>.txt
cargo run -p catcomsctl -- --stats demo               # print per-node SyncStats diagnostics
```

**Over real networking** (two OS processes, real libp2p TCP):

```sh
# Terminal 1 — found a server, write an invite, and serve:
cargo run -p catcomsctl -- serve --port 9000 --invite-file invite.txt
# Terminal 2 (or another machine, with --host <server-ip>) — join it:
cargo run -p catcomsctl -- join --invite-file invite.txt
```

The joiner dials the server over TCP, does the Noise + MLS join handshake, and
catches up the encrypted channel — the whole stack between separate processes.

**Through a relay** (NAT traversal — the joiner never connects to the server
directly; a zero-knowledge relay forwards ciphertext between them):

```sh
# Terminal 1 — a relay (prints its dialable /ip4/.../tcp/4000/p2p/<id>):
cargo run -p catcomsctl -- relay --port 4000
# Terminal 2 — serve, reserving a circuit slot on the relay:
cargo run -p catcomsctl -- serve --relay /ip4/<relay-ip>/tcp/4000/p2p/<relay-id>
# Terminal 3 — join (dials the relayed circuit address from the invite):
cargo run -p catcomsctl -- join
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
