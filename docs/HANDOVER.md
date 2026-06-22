# CatComs — Handover

Authoritative current-state document. Read this first, then
[`INTERFACES.md`](INTERFACES.md) (the API/seam schema) and
[`ARCHITECTURE.md`](ARCHITECTURE.md) (decisions + the adversarial-review fixes).

## Status (as of 2026-06-22)

- **Phases 0 → 7 COMPLETE; Phase 8 (product model + Tauri desktop UI) in progress.**
  6e-3d and Phase 7 (every networking + NAT-traversal path proven end-to-end over real
  TCP — direct/discovered/relayed/DCUtR — plus the consolidated security suite) are
  done. Phase 8 so far: the UI-facing **`catcoms-app`** product model + an async
  **event-stream actor** (8a/8b-1, fully test-gated), and a first **Tauri 2 + Svelte
  desktop app** (`apps/desktop`, 8b-2 … 8q) wired to the stack — found a server, mint a
  single-use invite, a second instance joins via paste, **multiple name-addressed
  channels** (with symmetric history catch-up + timestamps + auto-scroll), a live roster,
  **customizable member profiles** (name/color/font/animated effect/**avatar**, shared +
  converging), and a **fileshare browser**. Underneath, **8l** adds **content-addressed
  blob fetch over the mesh**, and **8m/8n** build avatars + fileshare on it (binaries
  travel by content address, fetched on demand — not inline in gossip). **8o–8q** make it a
  real client: **cross-network** founding/joining (LAN/advertised + **relay-circuit** NAT
  traversal, no port-forward) and **multi-server** (a Discord-style rail; several servers
  at once). **204 tests passing** (the GUI WebView is the one manually-verified surface;
  both halves compile). Rendezvous auto-discovery in the UI is the next
  network slice. See Known limitations.
- Both CRITICALs the 6e-3d design pass found are **closed and adversarially reviewed**:
  **A1** (the pre-existing bug where the gossip topics hashed the plaintext-invite
  `group_id`, so any invite-holder could read all topics) and **Sybil-C1** (the
  catch-up source-trust hole). See [`design-6e-rendezvous.md`](design-6e-rendezvous.md).
- The four security-critical closing slices (6–9) each passed an **adversarial-review
  workflow before commit**; one **blocking** finding was fixed in each of 3d-7 (a
  member-on-member PEX CPU-DoS — receive-side bundle uncapped) and 3d-8 (a variable-time
  cache integrity-tag compare — timing oracle); 3d-6 and 3d-9 were SOUND with cheap
  hardening folded in. The whole block ends with a **memory end-to-end test**: a joiner
  discovers the inviter at a zero-knowledge rendezvous and joins with **no hard-coded
  server address**.
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
| `catcoms-mls` | MLS group core (openmls 0.8): `MlsDevice`, `ServerGroup` (create/add/remove/process/epoch/`channel_secret`), single-use device-bound `InviteToken` (now `INVITE_DOMAIN` **v2**, carrying a signature-bound `rendezvous: Vec<String>`; `mint_invite_with_rendezvous`) + `InviteLedger`, `AddOutcome`, `designated_committer`. |
| `catcoms-replication` | Encrypted CRDT docs (automerge 0.10): inner-signed `SignedOp`, `SealedOp` (per-epoch channel-key sealing), `EncryptedDoc` (edit/ingest/catch-up). |
| `catcoms-storage` | Content-addressed `Cid` blob stores (mem + fs); per-file encryption (`FileRef`, per-file wrap nonce); `RetentionIndex` (3-scope expiry, GC with decorrelated eviction + `HolderOracle` probe). |
| `catcoms-net` | libp2p `MeshService` realizing `MeshTransport` (gossipsub + request/response over Noise+yamux). NAT traversal: relay-client + **circuit-relay-v2** + **DCUtR** hole-punch (`next_direct_upgrade()`). Standalone zero-knowledge infra: `build_relay_swarm`/`run_relay` and `build_rendezvous_swarm`/`run_rendezvous` (`RelayBehaviour`/`RendezvousBehaviour`). **Rendezvous client** in `MeshBehaviour`: `rendezvous_register`/`rendezvous_discover`; discovered records surface via `next_discovered()` (per-response capped) and are **never auto-dialed**. `add_external_address()` (register without a relay), `validate_rendezvous_addrs()` (reject circuit / require one `/p2p/` / distinct PeerIds). `connection_limits` on every swarm. Tracing-instrumented. |
| `catcoms-discovery` | **Pure** eclipse-resistance layer (no I/O, no ambient time/RNG). `DiscoveryPolicy` ranks discovered candidates into a bounded, Clock-paced/RNG-jittered **dial plan** (the only thing that decides what to dial): member-tag-verified → multi-rendezvous-corroborated → cache → junk-last, ≤1 root/rendezvous, roster-clamped, seq-freshness. Advisory `EclipseDetector` (D/R/S + hysteresis; never gates). Cross-session `AddressCache` (proven members, RNG-jittered eviction, BLAKE3 keyed integrity tag → tamper-detected on load; SQLCipher backing deferred). |
| `catcoms-sync` | `ChannelSync`: replication + membership over the transport. Blinded **member-only gossip topics keyed under `ns_secret_L`** that rotate on member removal (routing label `L`), grandfathered re-subscription window. The **join handshake** transfers the **routing state** (sealed, signature-bound). Membership **commit propagation**; **missed-commit recovery** with **signed catch-up responses** (nonce+epoch anti-replay) + a **two-pool peer model**; bounded zeroized **past-epoch key window**. Discovery surface: `rendezvous_namespaces()`, the pre-dial **member tag** (`membership_tag`/`verify_membership_tag`), **member PEX** (`PeerDescriptor`, `request_pex`/`publish_self_record`/`ingest_peer_record`/`known_peer_records`), the pre-join **`join_namespace()`**, and `transport()` (so the discovery/dial layer above can drive register/discover/dial). `SyncStats`. |
| `catcoms-app` | **Product model** — the UI-facing facade over the stack (so a GUI never touches MLS/automerge). `Server<T,R>` (found/join/open_channel/send_message/messages/members/invite), the canonical chat-message schema (`append_message`/`read_messages`), and the async **event-stream actor** (`spawn` → `ServerActor` commands + `AppEvent` stream: `ChannelUpdated`/`MembersChanged`/`Closed`). |
| `catcoms-log` | `tracing` subscriber init; `init_debug(debug, dir)` writes `debug_log_<ts>.txt`. |
| `apps/desktop` | **Tauri 2 + Svelte 5 desktop app** (its own cargo workspace, excluded from the root). A thin `#[tauri::command]` bridge (`src-tauri`) over the `catcoms-app` actor + a Svelte frontend. Found a server, #general, send/read. The WebView is the one manually-verified surface; `npm install && npm run tauri dev`. |
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
# Discovery: the server registers at a rendezvous under the invite's join_ns; the
# joiner discovers it there and joins with NO hard-coded server address:
cargo run -p catcomsctl -- serve --port 9000 --rendezvous /ip4/<rz-ip>/tcp/5000/p2p/<rz-id>
cargo run -p catcomsctl -- join                     # discover -> dial (via DiscoveryPolicy) -> join
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
| 6e-3d-6 | **`catcoms-discovery` `DiscoveryPolicy`** (pure: rank/clamp/dial-budget) + catch-up **nonce/epoch anti-replay** + pre-dial **membership tag**; reviewed SOUND | ✅ `2fedcd0` |
| 6e-3d-7 | **member PEX** (`KIND_PEX`, self-signed `PeerDescriptor`, responder-signed, capped/rate-limited); reviewed (blocking receive-cap DoS fixed) | ✅ `762ef63` |
| 6e-3d-8 | **advisory eclipse detector** (D/R/S, hysteresis, never gates) + **cross-session address cache** (tamper-detected); reviewed (blocking timing-oracle fixed) | ✅ `ca8493f` |
| 6e-3d-9 | **invite rewiring** (`rendezvous` vector, `INVITE_DOMAIN` v2) + pre-join **`join_ns`** + `serve --rendezvous`/`join` **discover→dial→join** end-to-end; reviewed SOUND | ✅ `f31a0c7` |
| 7a | **full-stack end-to-end over real TCP sockets** — founder binds an ephemeral loopback port; a fresh device dials it over real OS sockets, runs the MLS join, and converges | ✅ `798b50f` |
| 7b | **consolidated security suite** — threat-model → where-proven map + cross-layer scenarios (`an_eclipse_caution_never_gates_a_removal`, `a_removed_member_is_excluded_from_the_rotated_namespace`) | ✅ `ec5638e` |
| 7c | **rendezvous discovery bootstrap over real TCP** — joiner discovers the inviter under `join_ns` and joins with no hard-coded address, over OS sockets | ✅ `a168c1d` |
| 7d | **relayed full-stack join over real TCP** — server reachable only via a circuit relay; join + catch-up over the relayed connection (NAT traversal) | ✅ `0c2a6d8` |
| 7e | **DCUtR-upgraded full-stack path over real TCP** — a relayed join that hole-punches to a direct link (`next_direct_upgrade`), driven through a complete join + converge | ✅ `ff4c63f` |
| 8a | **`catcoms-app` product model** — UI-facing `Server` facade + canonical chat-message schema (the typed boundary the GUI is built against) | ✅ `1332051` |
| 8b-1 | **async event-stream actor** — `spawn(server)` → commands in / events out (ChannelUpdated, MembersChanged); the substrate the Tauri bridge drives | ✅ `c73929c` |
| 8b-2 | **Tauri 2 + Svelte desktop app** (`apps/desktop`) — found/open/send/read over the actor bridge; both halves compile (WebView manually verified) | ✅ `7c5f72e` |
| 8c | **invite + join in the desktop UI** — found mints a single-use invite (loopback bootstrap); a second instance pastes it, dials, joins, and converges (two instances can talk over real TCP) | ✅ `61f2ec3` |
| 8d | **multi-channel** — name-addressed channels (`catcoms-app::channel_id`); channel-list sidebar + "join #channel" + per-channel view/unread in the UI | ✅ `d2ec4d3` |
| 8e | **member roster + chat polish** — live Members panel (device-id fingerprints + "you"), own-message bubbles | ✅ `e77a33d` |
| 8f | **member profiles (backend)** — `DocType::Profile` (tag 9) + a shared per-server profile doc `{name,color,font,effect}` keyed by device fingerprint; messages now authored by fingerprint (name/style resolved from the author's profile at render time); actor seeds/serves/converges profiles | ✅ `bcf61db` |
| 8g | **profile editor + rich rendering** — "Your profile" editor (name, color, font, animated effect); roster + message authors resolve fingerprint → profile (rainbow colour-wave / wave / pulse); own-message keys on local fingerprint | ✅ `612965f` |
| 8h | **member avatars** — `Profile.avatar` (inline bytes in the profile doc, `MAX_AVATAR_BYTES` = 64 KiB; base64 across IPC); UI canvas-downscales to a 128px JPEG; circular avatars in roster + messages with an initials fallback | ✅ `9e9b878` |
| 8i | **per-channel history catch-up** — the bridge remembers the join peer (`catchup_peer`); opening any channel catches it up from that peer (joiner side), so ad-hoc channels show backlog, not just live. Asymmetry: a founder opening a joiner-created channel still relies on live gossip (superseded by 8j) | ✅ `ec3fc90` |
| 8j | **symmetric (any-peer) catch-up** — `ChannelSync::request_catchup_best` (+ `now_ms`) catches up from the best known peer (proven member, else any known peer); `Server::request_channel_catchup_any` + actor `CatchUpAny`; bridge `open_channel` uses it (dropped `catchup_peer`). Either side gets the backlog of a channel the other created | ✅ `479da21` |
| 8k | **chat UX polish** — messages carry a clock-stamped `ts` (canonical schema + `ChatMessage.ts`, stamped via `ChannelSync::now_ms`); UI shows HH:MM + auto-scrolls to newest | ✅ `f202530` |
| 8l | **content-addressed blob fetch over the mesh** — `ChannelSync` holds a `BlobStore`; `KIND_BLOB_FETCH` request/response (members-only, responder-signed/bound, 16 MiB cap, **per-requester rate limit** — folded from adversarial review since blob is the strongest amplifier); `put/get/has_blob`, `request_blob(_best)`. Re-hashes served bytes vs the requested CID before storing (no cache-poisoning). Foundation for large avatars + fileshare | ✅ `e0c3c8e` |
| 8m | **avatars over the blob layer** — the profile doc stores the avatar's `avatar_cid` (not inline bytes); `set_profile` puts the blob, `profiles()` resolves the CID against the local store, the actor proactively `fetch_missing_avatars` (always-try, since the holder-peer is often only known after the profile arrives) and re-emits. Public `Profile.avatar` (bytes) unchanged, so bridge/UI untouched | ✅ `5bc31f4` |
| 8n | **fileshare browser** — per-server file index (`DocType::FileIndex`): `add_file`/`files`/`download_file`/`open_files`/`request_files_catchup` + `FileEntry`; actor `AddFile`/`Files`/`DownloadFile`/`CatchUpFiles` + `FilesUpdated`; bridge base64↔CID-hex; UI "Files" panel (upload/list/download). Blobs plaintext-at-rest, members-only — `seal_file` encryption-at-rest + chunked transfer deferred | ✅ `66b06ce` |
| 8o | **cross-network founding/joining** — bind `0.0.0.0`; founder advertises a reachable address (LAN/public IP, `host:port`, or relay-circuit multiaddr) in the invite; joining dials **all** bootstrap addresses. Same-machine/LAN/port-forwarded internet all work. Pure `tcp_port`/`build_advertised` helpers unit-tested | ✅ `2ba19d3` |
| 8p | **multi-server** — bridge `AppState` is a `HashMap<u64, ServerEntry>` (each its own `Server`/actor); every command takes a `server` id, every event is tagged with it (`actor_of` clones the actor out so the registry lock is never held across an await); `found`/`join` return `{server, channel}` + register, `leave_server` shuts down. UI: a Discord-style server rail with per-server `ServerState` (channels/active/unread/invite/dot), active-server data loaded on switch + tagged events | ✅ `37fc6e7` |
| 8q | **relay-circuit founding** — `found_server` gains an optional `relay` multiaddr; dials it, reserves a circuit (`listen_on(relay/p2p-circuit)`), and puts the relayed address first in the invite (mirrors `catcomsctl serve --relay`). Joiner unchanged (8o dial-all handles a relayed bootstrap). Zero-config NAT traversal with a relay node, no port-forward | ✅ `5bf3970` |
| 8r/8s | **security-review hardening** — adversarial review of 8m–8q (no blocking findings); `fetch_missing_avatars` per-pass bounded (8r), `MemoryBlobStore` size-bounded (8s); + a desktop [User Guide](USER_GUIDE.md) | ✅ `0d0f3cf`/`173d168` |
| 8t | **status feed** — per-server post stream on `DocType::Status` (reuses the message schema): `open_status`/`post_status`/`statuses`/`request_status_catchup`; actor `PostStatus`/`Statuses`/`CatchUpStatus` + `StatusUpdated`; bridge + a "Status" UI panel | ✅ `64d3812` |
| 8u | **wiki** — one per-server `DocType::Wiki` doc (map page→body): `open_wiki`/`wiki_pages`/`read_wiki_page`/`write_wiki_page`/`request_wiki_catchup`; actor `WikiPages`/`ReadWikiPage`/`WriteWikiPage`/`CatchUpWiki` + `WikiUpdated` (full-map change tracking); bridge + a Chat/Wiki main-pane toggle (page list + editor, dirty-flag preserves in-progress edits) | ✅ `e6091aa` |
| 8v | **char-level wiki merge** — page bodies are automerge `Text` (`update_text` diff-splice); concurrent same-page edits merge char-by-char. + reworked a flaky file-convergence test to deterministic request/response catch-up (MemNetwork emits no `PeerConnected`, so gossip/peer-discovery timing was racy; the blob fetch itself is tested at the sync layer) | ✅ `1f61599` |
| 9 | **disk persistence + encryption-at-rest** — designed in [`design-persistence.md`](design-persistence.md). **9a** key vault (passphrase-sealed `Dek`→`KeyHierarchy`) ✅ `2bbae5d`-prev, **9b** sealing blob store (encrypted at rest, plaintext-CID-addressed) ✅ `2bbae5d`, **9c** snapshottable MLS state (`snapshot_server`/`restore_server`; the pivotal slice — adversarially reviewed) ✅, **9d** doc persistence (`EncryptedDoc::snapshot`/`restore`) ✅, **9e** sync-state assembly (`ChannelSync::snapshot`/`restore` — MLS+docs+routing+ledger+commit_log+peer_records into one `Zeroizing` blob, restored onto a fresh transport; adversarially reviewed) ✅, **9f** vault-sealed `ServerStore` (`servers/<id>.bin`+`registry.bin`, atomic, wrong-passphrase-safe) + `Server::snapshot`/`restore` + actor `Snapshot` command + the desktop **passphrase gate** (`unlock` reloads each server onto a fresh transport) and **save-on-mutation** — close/reopen the app, enter the passphrase, servers + history are back (read offline) ✅, **9g** re-dial persisted peers on reload (`peer_addrs_from_snapshot` → fresh mesh bootstrap; a reloaded joiner reconnects to stable-address peers) ✅, **9h** per-file encryption: **9h-a** wired `SealingBlobStore`/`FsBlobStore` per server (files+avatars persist + sealed at rest under `blob_key`) ✅, **9h-b** stable per-group file-wrap key minted at founding + bundled into the join transfer; `seal_file`/`open_file` so files are e2e ciphertext keyed by ciphertext CID (adversarially reviewed) ✅. **Phase 9 complete.** | **✅ done (9a–9h)** |
| 8… | rendezvous auto-discovery in the UI · chunked large-file transfer · last-copy-safe blob retention | planned |
| 9 | Android (Tauri 2 mobile): JNI keystore, foreground service, two-tier keys | planned |
| 10 | hardening: calendar, cover traffic, supply-chain attestation, metadata-index aging, **security review** (deeper adversarial scenarios land here) | planned |

### Earlier blocks (history)
6d-1b (missed-commit recovery + past-epoch key window) and 6d-2 (fork resolution +
the convergence-safe single-serializer membership model) are complete; their details
live in the commit messages and [`design-6d2.md`](design-6d2.md). The default config
stays **single designated committer** — the concurrent-committer / fork path is OFF
by default (`max_committer_rank=0`) pending **I1** (a wall-clock contest window can
converge honest nodes differently under async timing); re-read `design-6d2.md` before
touching it.

## 6e-3d — COMPLETE (rendezvous discovery + eclipse-resistance)

**Read [`design-6e-rendezvous.md`](design-6e-rendezvous.md)** — the 9-slice contract
from a 7-agent design+review workflow, plus the per-slice adversarial-review outcomes
(2b, 3d-5, and 3d-6…9 are all recorded there). All 9 slices are done.

**Phase 7** (end-to-end local integration over real sockets + consolidated security
suite) is **COMPLETE**. Every networking + NAT-traversal path is proven end-to-end over
**real TCP loopback sockets** (not just the libp2p memory transport): **direct**
(`tcp_e2e.rs`, 7a), **rendezvous-discovered** with no hard-coded address
(`tcp_rendezvous_e2e.rs`, 7c), **relayed** NAT-traversal (`tcp_relay_e2e.rs`, 7d), and a
**relayed→direct DCUtR upgrade** driven through a full join (`tcp_dcutr_e2e.rs`, 7e).
The consolidated security suite (`security.rs`, 7b) maps the threat model to where each
property is proven and adds the cross-layer scenarios (eclipse-never-gates-a-removal;
removed-member-excluded-from-the-rotated-namespace). Deeper adversarial scenarios are
deferred to **Phase 10** (the dedicated hardening + security-review phase). The next
focus is **Phase 8** — the product model + Tauri desktop UI.

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

**Eclipse-resistance (3d-6…9, done, each adversarially reviewed):**
- **3d-6** — pure `catcoms-discovery` `DiscoveryPolicy` (rank candidates → bounded,
  Clock-paced/RNG-jittered dial plan; ≤1 root/rendezvous; roster clamp; seq-freshness;
  the only thing that decides what to dial). Plus the pre-dial **membership tag** and
  the catch-up **nonce/epoch anti-replay** (closing the 3d-5 deferred items).
- **3d-7** — **member PEX** (`KIND_PEX`): members supply each other dialable
  `PeerDescriptor`s without a rendezvous; responder-signed, members-only, capped +
  rate-limited; entries are discovery candidates (never auto-promoted to the trusted
  catch-up pool).
- **3d-8** — advisory **`EclipseDetector`** (D/R/S + hysteresis; never gates) +
  cross-session **`AddressCache`** (proven members, tamper-detected load).
- **3d-9** — invite rewiring (signature-bound `rendezvous` vector, `INVITE_DOMAIN`
  v2), pre-join **`join_namespace`**, and `serve --rendezvous`/`join`
  **discover→dial→join** (DiscoveryPolicy-mediated) — verified by a memory end-to-end
  test (no hard-coded server address).

## Known limitations / deferred (the security-relevant ones)

- **Desktop networking: LAN/advertised works (8o); zero-config NAT traversal + dev/release
  build distinction.** The `apps/desktop` bridge binds all interfaces and the founder
  advertises a reachable address (LAN/public IP, `host:port`, or a relay-circuit multiaddr);
  joining dials every bootstrap address. So **same-machine** (blank), **LAN** (founder's
  LAN IP), and **internet via a port-forwarded public IP** all work.
  **Relay-circuit NAT traversal works (8q)** — paste a relay node's multiaddr when founding
  and the invite carries the relayed address (no port-forward for either peer). Still
  deferred: **rendezvous auto-discovery in the UI** — joining with *no* address in the
  invite, discovering the inviter at a zero-knowledge rendezvous (the CLI's `serve
  --rendezvous`/`join` already do this over real TCP; it is just not wired into the desktop
  `found`/`join` yet). Also: a
  **`cargo build` (debug) exe is a dev build** that loads the UI from the Vite dev server
  (`localhost:1420`) and shows "can't reach the page" on any machine without it — to
  distribute, build a release exe with the frontend embedded
  (`npm run build && npm run tauri build -- --no-bundle`; needs WebView2 on the target).
- **Blob store: size-bounded interim, not yet last-copy-safe (8l/8m/8n/8s).** From the
  8m–8q security review (no blocking findings; the slices reuse the reviewed-secure
  blob/MLS/Noise layers — `fetch_missing_avatars` is per-pass bounded, 8r). `MemoryBlobStore`
  is now **size-bounded** (`DEFAULT_BLOB_BUDGET` 128 MiB, **FIFO** eviction, 8s) so fetched
  avatars/files no longer grow without bound — but it is still **in-memory** (no
  persistence) and the FIFO eviction is **not holder-probe-aware** (it can drop the last
  copy of a blob; evicted blobs are re-fetchable only while a holder is online). Wiring the
  `catcoms-storage` retention engine (never-evict-last-copy + on-disk store) is the follow-up.
- **No disk persistence yet → per-file encryption-at-rest is premature (analysis, not
  implemented).** Full design + slice plan: [`design-persistence.md`](design-persistence.md)
  (pivotal constraint: openmls 0.8 has no group snapshot, so persistence needs a sealed
  on-disk `StorageProvider`; the keystore already has the at-rest primitives). The desktop
  app keeps everything **in RAM** (`MemoryBlobStore`, in-memory
  MLS/group state) — restart loses all servers. So there is currently **no disk "at rest"**
  to protect; the earlier "files plaintext at rest" note was about the in-memory cache, not
  disk. Per-file encryption-at-rest was investigated and **deliberately deferred**: with
  blobs in RAM, sealing them would put ciphertext next to the wrap-key in the same process
  memory (zero benefit) while requiring the riskiest change in the system — transferring a
  new stable key through the **join handshake**. The mechanics rule out shortcuts (ops are
  sealed **per-epoch**, catch-up **re-seals under the current epoch**, and there is **no
  stable group secret**), so the correct design is a **stable per-group file-wrap-key minted
  at founding and transferred at join like `RoutingState`**, used with
  `catcoms-storage::seal_file`/`open_file`. **Sequencing: disk persistence first** (blobs +
  group state to disk, the state sealed under the device keystore `catcoms-crypto::Dek`/
  `KeyHierarchy`), **then** per-file encryption as a paired, reviewed slice. Doing the
  encryption before persistence is security theater.
- **Network admission is single-committer-only** (only the lowest-leaf-index member
  admits). Concurrent admits / fork resolution + cross-member single-use = 6d-2.
- **Commit catch-up needs a peer that still holds the commit.** A member behind by
  more than a serving peer's `max_commit_log` window can't recover via commit
  catch-up — a full snapshot rejoin (deferred) is required; the gap is logged and a
  bad source is excluded, but exhausting all sources surfaces only a warning (a
  recovery *event* to the app is a follow-up).
- **Catch-up auth is now nonce-bound (6e-3d-6).** Catch-up *responses* are signed by a
  current member and bound to `(group_id, requester pubkey, req_ts, **nonce**, **epoch**,
  bundle)`, and requests carry a fresh signed timestamp + per-request RNG nonce — so a
  captured response cannot be replayed against a different request and the same-ms `ts`
  collision window is closed. Residual: there is still no *server-side* seen-nonce log,
  so a captured *request* can be re-sent within `MAX_REQUEST_AGE_MS` (60s) — harmless
  (the member just re-serves a freshly-signed bundle; the Noise transport confines it to
  the peer's own session). A full snapshot rejoin for a too-far-behind member is the
  remaining recovery gap.
- **6e-3d discovery is built (eclipse-resistance complete).** `DiscoveryPolicy`
  (ranked, budgeted dial plan), the pre-dial membership tag, member PEX, the advisory
  eclipse detector, and the cross-session cache are all in (6e-3d-6…9), and a joiner can
  bootstrap via `join_ns` with no hard-coded address. Residual / deferred (all
  availability-only, review-confirmed): the **cross-session cache is in-memory** (its
  SQLCipher persistence is storage-phase work); `catcomsctl serve --rendezvous`
  **registers once** and does not yet re-register before the rendezvous TTL lapses; the
  `join` path uses the **first** rendezvous only (no multi-rendezvous fall-through yet);
  and `--host` must be a raw IP. The net Actor never auto-dials; the dial decision and
  all bounds live in `catcoms-discovery`.
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
