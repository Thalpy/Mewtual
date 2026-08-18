# Mewtual; Handover

Authoritative current-state document. Read this first, then
[`INTERFACES.md`](INTERFACES.md) (the API/seam schema) and
[`ARCHITECTURE.md`](ARCHITECTURE.md) (decisions + the adversarial-review fixes).
[`THREAT-MODEL.md`](THREAT-MODEL.md) tracks what a modified ("hacked") client can/can't do;
the protocol- vs honest-client-enforced boundary and the hardening backlog.

## Status (as of 2026-08-15)

- **Phases 0 → 10 COMPLETE. The live work is the desktop client's real-time layer;
  group voice (phases 1–3 shipped; see [§ Voice](#voice-group-calls)).**
  The protocol stack (0–7) is done, and every networking + NAT-traversal path is proven
  end-to-end over **real TCP sockets**; direct / rendezvous-discovered / relayed /
  DCUtR-upgraded; plus the consolidated security suite. **Phase 8** built the UI-facing
  **`catcoms-app`** product model + async **event-stream actor** and the **Tauri 2 +
  Svelte desktop app** (`apps/desktop`): a multi-server rail, name-addressed channels
  with symmetric catch-up, a live roster with presence, profiles/avatars, a **fileshare
  browser** over **content-addressed blob fetch** (chunked, resumable-by-chunk), a status
  feed, a wiki, DMs + friends, and the whole chat product layer (search, edit/delete,
  reactions, replies, @mentions, pins, cross-server inbox). **Phase 9** added **disk
  persistence + encryption-at-rest** (passphrase vault → sealed per-server snapshots:
  close the app, reopen, enter the passphrase, everything is back). **Phase 10** was the
  UI/product overhaul + roles & permissions. Since then: **group voice**, and two
  networking QoL slices; best-effort **UPnP** (an auto-reachable founder: no relay, no
  port-forward, `bf28db9`) and **persistable relay/rendezvous identities** (a stable peer
  id, so invites embedding an infra multiaddr survive a restart, `f317c5c`).
  **2026-08:** the **desktop UI overhaul** (11u–11w): tokens-first "operator terminal"
  redesign with full user customisation, **server livery + shared icons**
  (`DocType::Livery`), the **verify-identity** surface, **channel topics**, and the
  delivery-states design (D1–D3 in progress).
  **11x: wiki history + edit review + nested pages.** The wiki doc gained three more
  NUL-reserved keys beside `\u{0}meta` (older peers merge them blind): `\u{0}hist`
  (per-page revisions: author fp, ts, full-body snapshot, kind edit/approve/auto/reject/
  rollback/delete/rename), `\u{0}pending` (member edits awaiting review) and `\u{0}cfg`
  (`review_days`, 0=off, ≤30, owner/admin-set). With review on, a member's save queues;
  owner/admin approve/decline; **auto-acceptance at the deadline is READ-TIME** (effective
  `wiki_map()` overlays expired pendings; nobody races to apply in the CRDT) and is folded
  into stored history by the next direct write under a **deterministic rev id (= pending
  id)** so concurrent folds converge. Member delete/rename are gated while review is on;
  pending bodies count toward `wiki_pinned_cids`. Sidebar: `/`-separated names render as a
  collapsible tree (`wikitree.ts`); history browser + review queue diff via `linediff.ts`
  (Myers). New `Server` methods: `wiki_history`, `wiki_pending_edits`, `wiki_review_days`,
  `set_wiki_review_days`, `approve_wiki_edit`, `reject_wiki_edit`, `restore_wiki_page`;
  `write_wiki_page` now returns `queued: bool`, mirrored through actor + bridge
  (`get_wiki_history`/`get_wiki_pending`/`get_wiki_review_days`/`set_wiki_review_days`/
  `approve_wiki_edit`/`reject_wiki_edit`/`restore_wiki_page`).
  Also **wiki infoboxes** (`infobox.ts`, pure + unit-tested): a `{{Infobox …}}` block is lifted
  by `renderWiki` **before** either converter runs and rendered with that page's own inline
  renderer, so one syntax serves both markdown and wikitext pages; it emits a `<table
  class="wiki-infobox">` (only `colspan` was added to the sanitizer allow-list). An image field
  accepts **only** the `![alt](cid:HEX)` marker: a bare content address would be invisible to
  the never-decay scan and could expire while the card still referenced it.
  **221 tests passing** as of 2026-07 (+ livery/icon/topic suites since; the GUI WebView
  is the one manually-verified surface; both halves compile). See Known limitations.
- Both CRITICALs the 6e-3d design pass found are **closed and adversarially reviewed**:
  **A1** (the pre-existing bug where the gossip topics hashed the plaintext-invite
  `group_id`, so any invite-holder could read all topics) and **Sybil-C1** (the
  catch-up source-trust hole). See [`design-6e-rendezvous.md`](design-6e-rendezvous.md).
- The four security-critical closing slices (6–9) each passed an **adversarial-review
  workflow before commit**; one **blocking** finding was fixed in each of 3d-7 (a
  member-on-member PEX CPU-DoS; receive-side bundle uncapped) and 3d-8 (a variable-time
  cache integrity-tag compare; timing oracle); 3d-6 and 3d-9 were SOUND with cheap
  hardening folded in. The whole block ends with a **memory end-to-end test**: a joiner
  discovers the inviter at a zero-knowledge rendezvous and joins with **no hard-coded
  server address**.
- Toolchain pinned **Rust 1.89.0** (`rust-toolchain.toml`; automerge 0.10 needs it).
- 11 library crates + 1 binary (+ `apps/desktop`, its own workspace). The protocol
  layers are tested deterministically with
  N in-process nodes over an in-memory transport; the mesh is *additionally* tested
  over **real libp2p**; the memory transport (real swarms/Noise/req-resp), TCP
  loopback (real sockets, multi-process `serve`/`join`), a circuit relay, a DCUtR
  hole-punch upgrade, and a rendezvous register→discover.
- Local-only repo (`git init`'d, no remote). Commits are linear on `main`,
  one per phase/block.

## What Mewtual is

A peer-to-peer, serverless, end-to-end-encrypted, invite-only group comms system;
"Signal + federation". Each "server/connection" is its own MLS (RFC 9420) group;
per-device identity; encrypted CRDT documents (channels/wiki/status/calendar)
replicated over a libp2p mesh with zero-knowledge relays. Targets Linux, Windows,
Android via a Rust core + (eventually) Tauri 2.

The four locked decisions and the pre-implementation adversarial-review fixes are
in [`ARCHITECTURE.md`](ARCHITECTURE.md) §1–§2; **read them; they constrain everything.**

## Crate map

| Crate | Responsibility |
|-------|----------------|
| `catcoms-wire` | Canonical, injective, length-prefixed codec; domain-separated key-derivation contexts (`DocType`, `exporter_context`). |
| `catcoms-rt` | The **seams**: `Clock`, RNG (`OsCryptoRng`/`CryptoRngCore`), and `MeshTransport` (pub/sub + request/response) with an in-memory `MemNetwork`/`Hub` for tests. |
| `catcoms-crypto` | Content-addressed `DeviceId`; Ed25519 device keys; multi-device pairing primitives (`PairingRequest`/SAS/`DeviceCertificate` group-bound/`DeviceRevocation`/`MasterHandoff`; v2 origin-rooted, depth-1; the v1 account-key chain module was deleted); the unified key hierarchy (`Dek`→HKDF subkeys), XChaCha20 `seal`/`unseal`, tiered `SecureKeyStore`. |
| `catcoms-mls` | MLS group core (openmls 0.8): `MlsDevice`, `ServerGroup` (create/add/remove/process/epoch/`channel_secret`), single-use device-bound `InviteToken` (now `INVITE_DOMAIN` **v2**, carrying a signature-bound `rendezvous: Vec<String>`; `mint_invite_with_rendezvous`) + `InviteLedger`, `AddOutcome`, `designated_committer`; **`media_secret(call_id)`** (MLS exporter under `MEDIA_EXPORTER_LABEL` → the per-call voice key). |
| `catcoms-replication` | Encrypted CRDT docs (automerge 0.10): inner-signed `SignedOp`, `SealedOp` (per-epoch channel-key sealing), `EncryptedDoc` (edit/ingest/catch-up). |
| `catcoms-storage` | Content-addressed `Cid` blob stores (mem + fs); per-file encryption (`FileRef`, per-file wrap nonce); `RetentionIndex` (3-scope expiry, GC with decorrelated eviction + `HolderOracle` probe). |
| `catcoms-net` | libp2p `MeshService` realizing `MeshTransport` (gossipsub + request/response over Noise+yamux). NAT traversal: relay-client + **circuit-relay-v2** + **DCUtR** hole-punch (`next_direct_upgrade()`), plus best-effort **UPnP/NAT-PMP** (`upnp` behaviour) that auto-opens a router port and surfaces the public address via `next_external_addr()`; folded into the invite so a peer can connect directly with no relay when the router cooperates (`None` promptly on no/CGNAT gateway). Standalone zero-knowledge infra: `build_relay_swarm`/`run_relay` and `build_rendezvous_swarm`/`run_rendezvous` (`RelayBehaviour`/`RendezvousBehaviour`). **Rendezvous client** in `MeshBehaviour`: `rendezvous_register`/`rendezvous_discover`; discovered records surface via `next_discovered()` (per-response capped) and are **never auto-dialed**. `add_external_address()` (register without a relay), `validate_rendezvous_addrs()` (reject circuit / require one `/p2p/` / distinct PeerIds). `connection_limits` on every swarm. Tracing-instrumented. |
| `catcoms-discovery` | **Pure** eclipse-resistance layer (no I/O, no ambient time/RNG). `DiscoveryPolicy` ranks discovered candidates into a bounded, Clock-paced/RNG-jittered **dial plan** (the only thing that decides what to dial): member-tag-verified → multi-rendezvous-corroborated → cache → junk-last, ≤1 root/rendezvous, roster-clamped, seq-freshness. Advisory `EclipseDetector` (D/R/S + hysteresis; never gates). Cross-session `AddressCache` (proven members, RNG-jittered eviction, BLAKE3 keyed integrity tag → tamper-detected on load; SQLCipher backing deferred). |
| `catcoms-sync` | `ChannelSync`: replication + membership over the transport. Blinded **member-only gossip topics keyed under `ns_secret_L`** that rotate on member removal (routing label `L`), grandfathered re-subscription window. The **join handshake** transfers the **routing state** (sealed, signature-bound). Membership **commit propagation**; **missed-commit recovery** with **signed catch-up responses** (nonce+epoch anti-replay) + a **two-pool peer model**; bounded zeroized **past-epoch key window**. Discovery surface: `rendezvous_namespaces()`, the pre-dial **member tag** (`membership_tag`/`verify_membership_tag`), **member PEX** (`PeerDescriptor`, `request_pex`/`publish_self_record`/`ingest_peer_record`/`known_peer_records`), the pre-join **`join_namespace()`**, and `transport()` (so the discovery/dial layer above can drive register/discover/dial). Voice: **`KIND_CALL_SIGNAL`** (authenticated members-only push of an *opaque* SDP/ICE payload; signed, freshness-bound, `from` = the verified signer; **not** deduped, FIFO-bounded) + `media_key`. `SyncStats`. |
| `catcoms-app` | **Product model**; the UI-facing facade over the stack (so a GUI never touches MLS/automerge). `Server<T,R>` (found/join/open_channel/send_message/messages/members/invite), the canonical chat-message schema (`append_message`/`read_messages`), and the async **event-stream actor** (`spawn` → `ServerActor` commands + `AppEvent` stream: `ChannelUpdated`/`MembersChanged`/`Closed`; voice adds `MediaKey`/`SendCallSignal` commands + a `CallSignal` event). |
| `catcoms-log` | `tracing` subscriber init; `init_debug(debug, dir)` writes `debug_log_<ts>.txt`. |
| `apps/desktop` | **Tauri 2 + Svelte 5 desktop app** (its own cargo workspace, excluded from the root). A thin `#[tauri::command]` bridge (`src-tauri`) over the `catcoms-app` actor + a Svelte frontend; the whole product surface (rail/channels/files/status/wiki/DMs/profiles) plus the **WebRTC voice mesh** (all media-plane code is frontend). The WebView is the one manually-verified surface; `npm install && npm run tauri dev`. |
| `bins/catcomsctl` | Dev CLI. `demo` runs the whole stack end-to-end (in-process); `serve`/`join` run it across **real OS processes over TCP** (optionally `serve --relay`); `relay` and `rendezvous` run the zero-knowledge infra nodes (`--identity <file>` persists the keypair for a **stable peer id across restarts**, so invites embedding the address keep working); `recover` drives the 6d-1b miss-and-heal path; `--debug`/`--stats`. |

## Build / verify ritual (run before every commit)

```sh
cargo build --all
cargo clippy --all-targets --all-features -- -D warnings   # must be clean
cargo fmt --all -- --check                                  # must be clean
cargo test --all                                            # all green
bash scripts/check-no-ambient.sh                            # ambient-dependency gate
```

For a change that touches the desktop app (its own cargo workspace; the root
`--all` does **not** cover it), also run, in `apps/desktop`:

```sh
npm run check          # svelte-check: must be 0 errors / 0 warnings
npm run build          # vite build
cargo check --manifest-path src-tauri/Cargo.toml   # the bridge half
```

PowerShell helper to sum test results (Windows dev box):
```pwsh
$out = cargo test --all 2>&1 | Out-String
([regex]::Matches($out,"(\d+) passed")|%{[int]$_.Groups[1].Value}|measure -sum).Sum
```
(`-match "FAILED"` is a false positive; it case-insensitively matches "0 failed".)

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
# ...for a DEPLOYED node, persist the identity so its peer id (and every invite that
# embeds its multiaddr) survives a restart:
cargo run -p catcomsctl -- relay --port 4000 --identity relay.key
# Discovery: the server registers at a rendezvous under the invite's join_ns; the
# joiner discovers it there and joins with NO hard-coded server address:
cargo run -p catcomsctl -- serve --port 9000 --rendezvous /ip4/<rz-ip>/tcp/5000/p2p/<rz-id>
cargo run -p catcomsctl -- join                     # discover -> dial (via DiscoveryPolicy) -> join
```
`demo` runs both members in one process over the in-memory transport; `serve`/`join`
run the *same* join + catch-up path across **separate OS processes over real libp2p
TCP** (verified, incl. through a relay).

## Working conventions (important; keep doing these)

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
- **Commit messages** end with a `Co-Authored-By: Claude <model> (1M context)` line
  (history is `Opus 4.8`; use whichever model actually did the work). Use
  `git commit -F <file>` for messages containing `==`/quotes (PowerShell here-strings
  mangle them).
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
|; | `catcomsctl` CLI + debug-file logging | ✅ `91c53d3` |
| 6c | network join handshake (inviter-authenticated) | ✅ `61d990c` |
| 6d-1a | membership commit propagation (single committer) | ✅ `89b5492` |
| 6d-1b | commit-catch-up recovery + ordered replay + past-epoch key window | ✅ `16a6427` |
| 6d-2a (1/2) | signed commit records + authorize-by-signature gate | ✅ `09d4cc5` |
| 6d-2a (2a) | MLS staged-commit primitives (stage/merge/abort) | ✅ `e577a11` |
| 6d-2a (2b) | sync-layer fork resolution (commit_id tie-break + contest window) | ✅ `939eb41` |
| 6d-2a (2c) | two-phase staged-Add join (provisional Welcome push) + review fixes I2–I6 | ✅ `42f9b7f` |
| 6d-2b (1) | **single-serializer remove** (members *request*; the designated committer alone commits); the convergence-safe model, on by default | ✅ `63ac788` |
| 6d-2b (2) | **all-members apply-time Add-binding validation** (every member rejects an Add not bound to this group / its own leaf key) | ✅ `0a1a276` |
| 6d-2b (3…) | by-value proposal batching · history-derived single-use · committer-decoupled admission | planned |
| 6e (1) | **full stack over real libp2p**; join handshake + encrypted catch-up over `MeshService` (Noise + request/response) | ✅ `f1d2713` |
| 6e (2) | **multi-process `catcomsctl serve`/`join` over TCP**; two OS processes, real sockets, verified | ✅ `73904f1` |
| 6e (3a) | **relay infrastructure**; relay-capable swarm (relay-client + DCUtR + identify) + relay server; a client reserves a circuit slot | ✅ `84827b1` |
| 6e (3b) | **end-to-end through a relay**; `catcomsctl relay`; `serve --relay` reserves + advertises the circuit address; `join` dials it. Verified across 3 real processes | ✅ `2f196b1` |
| 6e (3c) | **DCUtR hole-punch**; a relayed link auto-upgrades to a direct one; the upgrade is surfaced via `MeshService::next_direct_upgrade()`. TCP-loopback test asserts the upgrade event path | ✅ `7173e5e` |
| 6e-3d | **rendezvous discovery + eclipse-resistance**; 9 slices; design contract in [`design-6e-rendezvous.md`](design-6e-rendezvous.md) (`bd5e0d1`) | in progress (1–5/9) |
| 6e-3d-1 | per-removal routing secret `ns_secret_L` + `rendezvous_namespaces()` (rotate on removal; removed-member exclusion) | ✅ `eb6a952` |
| 6e-3d-2a | routing-state **transfer on join** (sealed, epoch-keyed) so joiners converge on topics/namespaces | ✅ `cb5c168` (prep `837bd4f`) |
| 6e-3d-2b | **re-key gossip topics from `ns_secret_L`** (member-only, rotate on removal); **closes A1**; adversarially reviewed | ✅ `1d9e3f2` |
| 6e-3d-3 | zero-knowledge **rendezvous server** + `catcomsctl rendezvous` | ✅ `924df09` |
| 6e-3d-4 | **rendezvous client** in `MeshBehaviour` (register/discover, surfaced, **no auto-dial**) + `connection_limits` | ✅ `fe1ed2a` |
| 6e-3d-5 | **signed catch-up responses + two-pool peer model**; **Sybil-C1** source-trust; adversarially reviewed | ✅ `726691b` |
| 6e-3d-6 | **`catcoms-discovery` `DiscoveryPolicy`** (pure: rank/clamp/dial-budget) + catch-up **nonce/epoch anti-replay** + pre-dial **membership tag**; reviewed SOUND | ✅ `2fedcd0` |
| 6e-3d-7 | **member PEX** (`KIND_PEX`, self-signed `PeerDescriptor`, responder-signed, capped/rate-limited); reviewed (blocking receive-cap DoS fixed) | ✅ `762ef63` |
| 6e-3d-8 | **advisory eclipse detector** (D/R/S, hysteresis, never gates) + **cross-session address cache** (tamper-detected); reviewed (blocking timing-oracle fixed) | ✅ `ca8493f` |
| 6e-3d-9 | **invite rewiring** (`rendezvous` vector, `INVITE_DOMAIN` v2) + pre-join **`join_ns`** + `serve --rendezvous`/`join` **discover→dial→join** end-to-end; reviewed SOUND | ✅ `f31a0c7` |
| 7a | **full-stack end-to-end over real TCP sockets**; founder binds an ephemeral loopback port; a fresh device dials it over real OS sockets, runs the MLS join, and converges | ✅ `798b50f` |
| 7b | **consolidated security suite**; threat-model → where-proven map + cross-layer scenarios (`an_eclipse_caution_never_gates_a_removal`, `a_removed_member_is_excluded_from_the_rotated_namespace`) | ✅ `ec5638e` |
| 7c | **rendezvous discovery bootstrap over real TCP**; joiner discovers the inviter under `join_ns` and joins with no hard-coded address, over OS sockets | ✅ `a168c1d` |
| 7d | **relayed full-stack join over real TCP**; server reachable only via a circuit relay; join + catch-up over the relayed connection (NAT traversal) | ✅ `0c2a6d8` |
| 7e | **DCUtR-upgraded full-stack path over real TCP**; a relayed join that hole-punches to a direct link (`next_direct_upgrade`), driven through a complete join + converge | ✅ `ff4c63f` |
| 8a | **`catcoms-app` product model**; UI-facing `Server` facade + canonical chat-message schema (the typed boundary the GUI is built against) | ✅ `1332051` |
| 8b-1 | **async event-stream actor**; `spawn(server)` → commands in / events out (ChannelUpdated, MembersChanged); the substrate the Tauri bridge drives | ✅ `c73929c` |
| 8b-2 | **Tauri 2 + Svelte desktop app** (`apps/desktop`); found/open/send/read over the actor bridge; both halves compile (WebView manually verified) | ✅ `7c5f72e` |
| 8c | **invite + join in the desktop UI**; found mints a single-use invite (loopback bootstrap); a second instance pastes it, dials, joins, and converges (two instances can talk over real TCP) | ✅ `61f2ec3` |
| 8d | **multi-channel**; name-addressed channels (`catcoms-app::channel_id`); channel-list sidebar + "join #channel" + per-channel view/unread in the UI | ✅ `d2ec4d3` |
| 8e | **member roster + chat polish**; live Members panel (device-id fingerprints + "you"), own-message bubbles | ✅ `e77a33d` |
| 8f | **member profiles (backend)**; `DocType::Profile` (tag 9) + a shared per-server profile doc `{name,color,font,effect}` keyed by device fingerprint; messages now authored by fingerprint (name/style resolved from the author's profile at render time); actor seeds/serves/converges profiles | ✅ `bcf61db` |
| 8g | **profile editor + rich rendering**; "Your profile" editor (name, color, font, animated effect); roster + message authors resolve fingerprint → profile (rainbow colour-wave / wave / pulse); own-message keys on local fingerprint | ✅ `612965f` |
| 8h | **member avatars**; `Profile.avatar` (inline bytes in the profile doc, `MAX_AVATAR_BYTES` = 64 KiB; base64 across IPC); UI canvas-downscales to a 128px JPEG; circular avatars in roster + messages with an initials fallback | ✅ `9e9b878` |
| 8i | **per-channel history catch-up**; the bridge remembers the join peer (`catchup_peer`); opening any channel catches it up from that peer (joiner side), so ad-hoc channels show backlog, not just live. Asymmetry: a founder opening a joiner-created channel still relies on live gossip (superseded by 8j) | ✅ `ec3fc90` |
| 8j | **symmetric (any-peer) catch-up**; `ChannelSync::request_catchup_best` (+ `now_ms`) catches up from the best known peer (proven member, else any known peer); `Server::request_channel_catchup_any` + actor `CatchUpAny`; bridge `open_channel` uses it (dropped `catchup_peer`). Either side gets the backlog of a channel the other created | ✅ `479da21` |
| 8k | **chat UX polish**; messages carry a clock-stamped `ts` (canonical schema + `ChatMessage.ts`, stamped via `ChannelSync::now_ms`); UI shows HH:MM + auto-scrolls to newest | ✅ `f202530` |
| 8l | **content-addressed blob fetch over the mesh**; `ChannelSync` holds a `BlobStore`; `KIND_BLOB_FETCH` request/response (members-only, responder-signed/bound, 16 MiB cap, **per-requester rate limit**; folded from adversarial review since blob is the strongest amplifier); `put/get/has_blob`, `request_blob(_best)`. Re-hashes served bytes vs the requested CID before storing (no cache-poisoning). Foundation for large avatars + fileshare | ✅ `e0c3c8e` |
| 8m | **avatars over the blob layer**; the profile doc stores the avatar's `avatar_cid` (not inline bytes); `set_profile` puts the blob, `profiles()` resolves the CID against the local store, the actor proactively `fetch_missing_avatars` (always-try, since the holder-peer is often only known after the profile arrives) and re-emits. Public `Profile.avatar` (bytes) unchanged, so bridge/UI untouched | ✅ `5bc31f4` |
| 8n | **fileshare browser**; per-server file index (`DocType::FileIndex`): `add_file`/`files`/`download_file`/`open_files`/`request_files_catchup` + `FileEntry`; actor `AddFile`/`Files`/`DownloadFile`/`CatchUpFiles` + `FilesUpdated`; bridge base64↔CID-hex; UI "Files" panel (upload/list/download). Blobs plaintext-at-rest, members-only; `seal_file` encryption-at-rest + chunked transfer deferred | ✅ `66b06ce` |
| 8o | **cross-network founding/joining**; bind `0.0.0.0`; founder advertises a reachable address (LAN/public IP, `host:port`, or relay-circuit multiaddr) in the invite; joining dials **all** bootstrap addresses. Same-machine/LAN/port-forwarded internet all work. Pure `tcp_port`/`build_advertised` helpers unit-tested | ✅ `2ba19d3` |
| 8p | **multi-server**; bridge `AppState` is a `HashMap<u64, ServerEntry>` (each its own `Server`/actor); every command takes a `server` id, every event is tagged with it (`actor_of` clones the actor out so the registry lock is never held across an await); `found`/`join` return `{server, channel}` + register, `leave_server` shuts down. UI: a Discord-style server rail with per-server `ServerState` (channels/active/unread/invite/dot), active-server data loaded on switch + tagged events | ✅ `37fc6e7` |
| 8q | **relay-circuit founding**; `found_server` gains an optional `relay` multiaddr; dials it, reserves a circuit (`listen_on(relay/p2p-circuit)`), and puts the relayed address first in the invite (mirrors `catcomsctl serve --relay`). Joiner unchanged (8o dial-all handles a relayed bootstrap). Zero-config NAT traversal with a relay node, no port-forward | ✅ `5bf3970` |
| 8r/8s | **security-review hardening**; adversarial review of 8m–8q (no blocking findings); `fetch_missing_avatars` per-pass bounded (8r), `MemoryBlobStore` size-bounded (8s); + a desktop [User Guide](USER_GUIDE.md) | ✅ `0d0f3cf`/`173d168` |
| 8t | **status feed**; per-server post stream on `DocType::Status` (reuses the message schema): `open_status`/`post_status`/`statuses`/`request_status_catchup`; actor `PostStatus`/`Statuses`/`CatchUpStatus` + `StatusUpdated`; bridge + a "Status" UI panel | ✅ `64d3812` |
| 8u | **wiki**; one per-server `DocType::Wiki` doc (map page→body): `open_wiki`/`wiki_pages`/`read_wiki_page`/`write_wiki_page`/`request_wiki_catchup`; actor `WikiPages`/`ReadWikiPage`/`WriteWikiPage`/`CatchUpWiki` + `WikiUpdated` (full-map change tracking); bridge + a Chat/Wiki main-pane toggle (page list + editor, dirty-flag preserves in-progress edits) | ✅ `e6091aa` |
| 8v | **char-level wiki merge**; page bodies are automerge `Text` (`update_text` diff-splice); concurrent same-page edits merge char-by-char. + reworked a flaky file-convergence test to deterministic request/response catch-up (MemNetwork emits no `PeerConnected`, so gossip/peer-discovery timing was racy; the blob fetch itself is tested at the sync layer) | ✅ `1f61599` |
| 9 | **disk persistence + encryption-at-rest**; designed in [`design-persistence.md`](design-persistence.md). **9a** key vault (passphrase-sealed `Dek`→`KeyHierarchy`) ✅ `2bbae5d`-prev, **9b** sealing blob store (encrypted at rest, plaintext-CID-addressed) ✅ `2bbae5d`, **9c** snapshottable MLS state (`snapshot_server`/`restore_server`; the pivotal slice; adversarially reviewed) ✅, **9d** doc persistence (`EncryptedDoc::snapshot`/`restore`) ✅, **9e** sync-state assembly (`ChannelSync::snapshot`/`restore`; MLS+docs+routing+ledger+commit_log+peer_records into one `Zeroizing` blob, restored onto a fresh transport; adversarially reviewed) ✅, **9f** vault-sealed `ServerStore` (`servers/<id>.bin`+`registry.bin`, atomic, wrong-passphrase-safe) + `Server::snapshot`/`restore` + actor `Snapshot` command + the desktop **passphrase gate** (`unlock` reloads each server onto a fresh transport) and **save-on-mutation**; close/reopen the app, enter the passphrase, servers + history are back (read offline) ✅, **9g** re-dial persisted peers on reload (`peer_addrs_from_snapshot` → fresh mesh bootstrap; a reloaded joiner reconnects to stable-address peers) ✅, **9h** per-file encryption: **9h-a** wired `SealingBlobStore`/`FsBlobStore` per server (files+avatars persist + sealed at rest under `blob_key`) ✅, **9h-b** stable per-group file-wrap key minted at founding + bundled into the join transfer; `seal_file`/`open_file` so files are e2e ciphertext keyed by ciphertext CID (adversarially reviewed) ✅. **Phase 9 complete.** | **✅ done (9a–9h)** |
| 10 | **desktop UI / product overhaul** ([plan](../../.claude/plans/moonlit-puzzling-karp.md)). **10a** tabbed nav (Chat·Files·Status·Wiki·Profile) + Settings overlay + invite placement, **10b** rich-text renderer (`render.ts`: `marked` + DOMPurify; `[[links]]`/`:emoji:`/`![cid embeds]` tokens; sanitizer allows no media/raw-HTML; placeholders only), **10c** fileshare folders (`FileEntry.path`, traversal-safe) + chat drag-drop media embeds + media resolver (builds `<img/video/audio>` in code from CID-verified blobs) **+10e** status media; *adversarially reviewed, 0 security findings*, **10d** wiki overhaul (markdown render + Read/Edit + `[[links]]` nav + backlinks via `get_wiki_map` + media + in-app help), **10f** custom emoji via the `emoji/` fileshare folder (picker + `:code:` render + Settings manage), **10g** notification sounds (Web Audio chime, Settings toggle), **10h** roles/permissions (`MemberRoles` doc tag 6; owner = MLS designated committer; admin grants + role-gated `mint_invite`; Settings→Server role manager); *adversarially reviewed; enforcement is honestly documented as **policy-layer/advisory** (admin grants forgeable by a modified client; owner-signed grants + committer-side join re-check are the named follow-up)*. All 16 user UI requests delivered. | **✅ done (10a–10h)** |
| 8… | **✅ rendezvous auto-discovery in the UI**; found registers at a zero-knowledge rendezvous; a joiner pasting that invite is discovered there and joins with **no hard-coded address** ([`design-rendezvous-ui.md`](design-rendezvous-ui.md), reviewed). **✅ chunked large-file transfer**; a file splits into chunks (each its own content-addressed blob) described by a `FileManifest`; the per-blob 16 MiB cap now bounds only a chunk (whole-file cap 256 MiB), the blob rate limit became a per-requester **bytes-budget**, and download reassembles + verifies the whole-file plaintext cid ([`design-chunked-transfer.md`](design-chunked-transfer.md), reviewed). **✅ post-join steady-state discovery**; after joining, a member periodically re-registers/discovers at the rendezvous under its rotation-aware namespaces and dials other members (re-finds the group after a restart, no fresh invite); `MeshTransport` extended (libp2p-free, default-inert verbs), a per-server bridge timer drives `AppCommand::DriveDiscovery` (real-time off the deterministic-time seam), persisted rz config ([`design-postjoin-discovery.md`](design-postjoin-discovery.md), reviewed). **✅ dedup-safe blob GC** (delete now reclaims a deleted file's orphaned chunk blobs, keeping any chunk another file references), **✅ download progress** (per-chunk `DownloadProgress` events → a UI progress bar; the whole-buffer-IPC/non-blocking-actor refactor stays deferred) **+ a Downloads tab** (per-server, newest-first list of queued/downloading/done/failed transfers + a "clear finished" action; shows the **live provider**; the signed responder that actually served each chunk, surfaced authenticated via `request_blob_best_provider` (the responder signs the request-bound, content-verified blob response, so the fingerprint is unspoofable), falling back to the uploader as the source), **+ file-browser availability** (each file is colour-coded by local availability; `●` on this device / `◐` partial _h/t_ / `○` downloadable / `○` no peers online; via a new `files_view` that counts held chunks per file + a cheap reachable-peer flag, zero network cost; refreshed on tab-open / files-updated / post-download), **+ channel viewer is now chat-only** (the channel list hides outside Chat; the roster stays), **+ live member presence** (`ChannelSync` now keeps an accurate `connected_peers` set; `PeerDisconnected` was previously dropped; surfaced as roster online dots + an "N online" count via `connected_member_fingerprints`, which matches each member by **its own** signed `peer_id` so a forged record can't steal another's presence; the availability hint's `has_peers` now uses this live set, fixing the staleness), **+ per-member presence detail** (the frontend tracks observed connect/disconnect transitions to show "Online · 5m" / "Last seen 5m ago" in the roster tooltip, member menu, and an inline last-seen; durations only for transitions actually witnessed this session, refreshed by a 60s tick), **+ DMs + friends (phase 1)**; a DM is a 2-person server flagged `is_dm` (a backward-compatible registry trailing block; the signed invite + network path are unchanged, so per-server unlinkability is preserved); a DMs circle on the rail opens a DM-home (friends/DM list + the conversation reusing the chat view), **New DM** founds a DM + surfaces its invite as a friend code, **Add friend** redeems a pasted code ([`design-dms-friends.md`](design-dms-friends.md), reviewed; no protocol/security change). **✅ phase 2: friends-list sortings**; a DMs-only `message_stats`/`dm_stats` (count + timestamps + distinct active days, no message text) drives sorting the friends list by **recent** / **most active** (msgs ÷ active days) / **reconnect** (volume × silence) / **A–Z**, with a per-DM last-message hint (reviewed). **✅ phase 3: in-band "Add friend"**; a roster action on an *online* member founds a DM and delivers its invite over the shared server via a new authenticated `KIND_DM_INVITE` request (membership+signature+freshness, like PEX; `from` = the verified signer, unforgeable; payload opaque/inert, validated only on accept; queue bounded+deduped+transient). The recipient sees a pending friend request (DMs-circle badge + a list) and accepts with one click; offline targets fall back to the friend code (reviewed: auth/no-spoof + inert-payload + DoS-bound all hold). **DMs + friends complete.** **✅ non-blocking download**; a large download no longer freezes the server actor: the bridge fetches the file **one chunk per actor command** (`file_download_plan` + `fetch_file_chunk`), so the actor returns to its loop between chunks and interleaves messages/sync; it reassembles, emits per-chunk progress, and verifies the whole-file content address bridge-side (reviewed; equivalent + integrity undiminished). **✅ eclipse `D` accuracy**; `observe_eclipse`'s reachable-devices now uses the live `connected_member_fingerprints` instead of the monotonic `member_peers`, so it stops under-warning after a node loses its peers. **✅ in-channel message search**; Ctrl+F (or the 🔍 header button) opens a search bar over the active conversation's messages; matches are highlighted, Enter/Shift+Enter (or ↑/↓) step through them scrolling each into view, with an _n / m_ counter; closes on Esc or a channel/server switch (frontend-only). **✅ advanced search**; the same bar gained a filter panel (Ctrl+Shift+F, or the "Filters (n)" toggle): **from** a member · **after**/**before** a local-day date · **has** image/video/audio/file/link (embeds classified by the fileshare index's MIME via `safeMime`, so a non-media or not-yet-indexed cid reads as a plain attachment) · **is** reply/has-replies/pinned/edited/mentions-me/from-me · **reactions** any/mine/a specific emoji; all AND-combined and usable with an *empty* query, plus a **sort** (oldest/newest/author A–Z/most reactions) that orders both the ↑/↓ stepping and a new click-to-jump result list (first 50 rendered, count disclosed). The match cursor is *clamped* rather than reset so a filter edit or an incoming message can't strand the highlight; the author/emoji pickers are lazy deriveds over the loaded messages, and the media regex only runs when a has-filter is active (frontend-only; still scoped to the loaded backlog). **✅ advanced search, round 2**; search became **server-wide**: an **In** scope (this channel / all channels / a specific one) builds a *corpus* of `{channel, index, message}` hits, fetching each non-open channel once via the existing `get_messages` into a snapshot dropped when the search closes (the open channel always reads the live `messages`, so it can't go stale). A hit in another channel is reached by clicking it or stepping onto it: `switchTo` gained a `keepSearch` flag (and is now `async`, awaiting `refresh`) so the jump lands by **message id** in the freshly-loaded channel, with the outgoing channel snapshotted first so its hits don't blink out mid-switch; *refining* a query never jumps channels, only ↑/↓ and clicks do. **From** and a new **Mentions** filter are member **typeaheads** (roster ∪ corpus authors, ↑/↓/Enter/click, emptying the box drops the filter); mentions match the `@[Name]` marker via the shared `mentionName` normalizer, so a since-renamed member matches under the name they were mentioned by. Also: **Today/7d/30d** date shortcuts, **case-sensitive** + **whole-word** match modifiers (the latter bounds on non-word-or-edge, since `\b` misbehaves on a punctuation-edged query), a **most replies** sort, and reply counts computed over the *corpus* (not just the open channel) so "has replies" and that sort stay right server-wide. Result rows carry the channel; the sorts key on timestamp rather than corpus position, since a multi-channel corpus is grouped by channel. **✅ edit + delete your own messages**; messages now carry a stable random `id` (list indices are unstable under CRDT merges); a member can edit (inline, with an "(edited)" tag) or delete its own messages via `Server::edit_message`/`delete_message` (a soft own-author gate; honest-client-only, the documented R6 residual since message content isn't authenticated). The change-detector switched from message-count to a content signature so an edit (count unchanged) refreshes everyone (reviewed: CRDT ops merge-safe, no empty/stale op). **✅ message moderation**; owner/admin can delete *any* member's message (not just their own; edit stays own-only), honest-client gated like file deletion (R6); offered in servers, not DMs. **✅ jump-to-unread**; a per-`server:channel` read mark (localStorage) renders a "New messages" divider + an "↑ New" jump button; the mark advances to the latest once seen. **✅ emoji reactions**; toggle a reaction on any message (quick-picker + right-click "React…"); chips show counts and highlight your own. Stored as flat scalar keys `"<emoji>\x1f<fp>"=true` written **directly on the message map** (no sub-object), so concurrent reactors write distinct keys that all survive a merge; no concurrent-create loss for *any* message, legacy included (5-lens adversarial review → this superseded an earlier pre-created-container design that still lost reactions on old-client-authored messages; a two-replica fork/merge convergence test pins the invariant; emoji validated at the trust boundary). The content signature folds reactions so a peer's reaction refreshes everyone. **✅ reply / threading**; reply to any message (right-click → "Reply" or the composer banner); messages carry an immutable `reply_to` parent-id (written only when it's a reply, so plain messages stay key-clean; no concurrency hazard, it's set once at creation), rendered as a clickable parent-quote that jumps to + flashes the original (degrades to "original message" if the parent isn't loaded). `Server::send_reply` threads it; `send_message` stays a 2-arg delegate (no test churn). Reviewed: sound, backward-compatible, all dangling/lifecycle paths degrade gracefully. **✅ @mentions + reply notifications**; type `@` for a member autocomplete that inserts an `@[Name]` marker (frontend-only; mentions ride in message text, no CRDT change), rendered as a highlighted chip via a new `marked` tokenizer (DOMPurify-sanitized) with a stronger self-highlight; a sidebar `@` badge marks any active-server channel with an unseen message that mentions you or replies to one of yours (scoped to the active server, where your per-server identity is known; cleared on read). Insertion + detection share a `mentionName` normalizer so odd names round-trip. Reviewed: no XSS, the mid-fetch server-switch race guarded, name-based matching is best-effort by design. **✅ custom-emoji reactions**; the reaction picker also offers the server's custom `:name:` emoji (the `emoji/` fileshare folder), and reaction chips render a custom emoji as its image (graceful `:name:` text fallback where the emoji file isn't held); backend unchanged (it already accepts any emoji string). **✅ cross-server inbox**; a dedicated rail icon (📥) opens its own screen listing every message that @-mentions you or replies to one of yours, across **all** servers/DMs, newest first, each showing who/where/when + a one-click jump (with unseen highlighting + a rail badge). Backend-driven: `Server::inbox` scans each server's channels in-process and resolves author names (per-server identity); the bridge `get_inbox` aggregates under a lock-free actor snapshot; a 1.5s-debounced reload keeps it live. The backend reuses the UI's exact `@[Name]` normalization (`normalize_mention_name`) so detection matches insertion. Reviewed: no blocking, the marker-normalization divergence + jump-to-unlisted-channel + timer-leak all fixed. **✅ reply-count thread affordance** (a "💬 N replies" chip under any message that has replies, jumping to the first) **+ distinct mention chime** (a brighter rising triad when a message mentions/replies to you, vs the two-note chime for ordinary messages; wired into both the open channel and the per-channel scan). **✅ message pinning**; owner/admin can pin/unpin any message (honest-client gated, R6); a 📌 marks pinned messages inline and a header "📌 N" opens a panel listing them with jump-to/unpin. Stored as a `pinned` flag **directly on the message map** (merge-safe like the reactions design; concurrent pins of different messages can't conflict, a pin/unpin race is clean LWW); the change-detector folds it so a peer's pin refreshes everyone; an idempotent guard avoids a redundant op. Reviewed: ship, no blocking. **✅ rich composer**; `||spoiler||` tags (a new `marked` tokenizer rendering a blacked-out span revealed on click, DOMPurify-allowlisted), a composer **formatting toolbar** (bold/italic/strike/code/spoiler that wrap the selection, + Ctrl+B/Ctrl+I), and **per-channel drafts** (in-memory: switching channels/servers preserves what you'd typed, cleared on send). **✅ message-action UX fix**; edit/picker were rendering on every legacy (empty-id) message because `editingId/reactionPickerFor === ""` matched `m.id === ""`; now gated on a truthy id, plus a Discord-style hover toolbar (react/reply/⋯-more) on each message. **✅ bug fixes (user-reported):** profile name/styling reverted on reload because `spawn` *unconditionally* re-seeded the profile from the founding display name; now seeds only when absent, so a restored profile survives (regression-tested); `saveProfile` also keeps the rail label in sync when it was still tracking your name. Inline media (status/chat embeds + custom emoji) vanished after a tab switch because the resolution `$effect` didn't track `view` (tab switch destroys+recreates the DOM with fresh, unresolved placeholders); now re-resolves on `view` change (cheap; the embed cache holds the decrypted bytes). The file-info preview no longer hangs on "Loading preview…" forever; a failed fetch now surfaces "preview unavailable". Composer: emoji button moved right, the inline formatting toolbar replaced by a Settings → Message-formatting help section (Ctrl+B/I kept). **✅ emoji/sticker size**; custom emoji can be created at a chosen size (Emoji/Medium/Large/Sticker, capped 160px), encoded as a `~<px>` suffix in the emoji's filename so it's shared with everyone (no backend change); inline `:code:` renders at that size, reactions/pickers stay small. **✅ profile cards + customisation**; clicking a member's avatar/name opens a profile card (avatar, styled name, role, a self-set **description/bio**, an Add-friend button for online members); the Profile gained `description` + `bubble` fields (CRDT, additive/backward-compatible). The **message bubble** is now customisable per member (color/gradient presets), applied to that author's messages; the value is sanitized (colors/gradients only, no CSS injection) and the description renders as escaped text. **✅ discovery record-seq surfacing** (real anti-replay freshness) **+ advisory `EclipseDetector` surfacing** (isolation banner; never gates). Remaining: pre-dial membership-tag verification (deferred; invasive synthetic-address carry, marginal value) · AddressCache persistence · true streaming download · TTL-aware re-registration | rendezvous + chunking + post-join discovery + final polish **done**; rest planned |
| 10+ | roles hardening: **owner-only member removal PROTOCOL-enforced** (`request_remove` rejects a non-owner; the committer ignores any inbound remove request not from the owner; THREAT-MODEL R1 closed) + **owner/admin file deletion** (`delete_file` role-gated; reviewed); DONE. **✅ Functional admin invites (Option C, owner-serialized)**; an admin broadcasts a signed `CTRL_ADD_REQUEST`; the **owner alone** runs the MLS Add (single committer → no fork) + relays a re-signed Welcome (joiner verification unchanged); offline-queued until the owner is online ([`design-admin-invites.md`](design-admin-invites.md), reviewed, no blocking findings). **✅ Replay-proof grant revocation (THREAT-MODEL item 3)**; authoritative admin set is **owner-local** (`ChannelSync::admin_roster`, persisted); the admission gate reads it (a malicious member can't write it), the CRDT `roster` is owner-signed display-only ([`design-grant-revocation.md`](design-grant-revocation.md), reviewed). UI now lets **admins mint invites**. Remaining: file-delete protocol gate (low stakes) · sticky/transferable ownership · blob GC after delete. Do **not** enable `max_committer_rank ≥ 1`. See [`THREAT-MODEL.md`](THREAT-MODEL.md). | **✅ done** |
| 10++ | **embed-persistence fix** + **file info pane** + **feedback button**: inline image/emoji embeds vanished after a restart/HMR; the resolve `$effect` ran before `{@html}` committed its placeholders and never re-ran; fixed with `tick()` (+ a dev-HMR `unlock` guard against duplicate actors). Clicking a file opens an **info pane** (preview · local-availability via `file_available`/`has_blob` · uploader/size/type/folder/cid · Download · owner/admin Delete). A 💬 rail button composes a bug/feature report to the clipboard (serverless, so copy-and-share). | ✅ `dd44446`, `8d9e371` |
| 10+++ | **chat layout polish**; chat is edge-to-edge (no bordered box / distinct background, trimmed channel padding; bubbles float on the app background) and the bubble presets were re-picked dark enough for white text (+ a text-shadow on custom bubbles). Frontend-only | ✅ `43457ff` |
| **11** | **GROUP VOICE**; E2E real-time audio, design in [`design-voice.md`](design-voice.md). See [§ Voice](#voice-group-calls) | ✅ 11a–11e (phases 1–3 of the design; phase 4 planned) |
| 11a | **voice phase 1; crypto + signalling core.** `MEDIA_EXPORTER_LABEL` + `ServerGroup::media_secret(call_id)` derive a 32-byte per-call key from the MLS exporter at the current epoch (every member derives it **locally**, never on the wire; distinct calls → distinct keys), surfaced `ChannelSync::media_key` → `Server::media_key` → actor `MediaKey` → bridge `call_media_key`. New authenticated push `KIND_CALL_SIGNAL` (= 8) mirroring `KIND_DM_INVITE`; members-only, Ed25519-signed, freshness-bound, `from` = the verified signer; payload **opaque** to the core, **not** deduped (every ICE candidate must arrive), FIFO-bounded (`MAX_PENDING_CALL_SIGNALS`). Actor drains per loop → `CallSignal` event → bridge `call-signal` (base64) | ✅ `bd483b5` |
| 11b | **voice phase 2; WebRTC mesh + call UI.** Full mesh (`RTCPeerConnection` per pair, no server in the media path → DTLS-SRTP is end-to-end); SDP/ICE ride the 11a authenticated push, so the DTLS fingerprints **can't be MITM'd**. Protocol: start → "ring" online members; accept → "hello" → existing participants "offer"; "answer"/"ice" per edge; "bye" tears one down (a newcomer auto-meshes with everyone). UI: header 📞 Call, a floating call bar (participant avatars, mute, leave) surviving channel/tab switches, an incoming prompt | ✅ `52a64e2` |
| 11c | **voice NAT traversal**; configurable ICE servers: STUN on by default (hole-punch across most home NATs), optional TURN (relays still-SRTP-encrypted audio when no direct path exists; TURN can't decrypt). User-editable in Settings → Calls, persisted locally (blank STUN = LAN-only). Call bar shows live status ("connecting…" / "N connected" / "check NAT/TURN"). Note: **signalling still rides the mesh**, so the members must already be mesh-connected | ✅ `1e2a698` |
| 11d | **channel-scoped voice rooms + presence + notifications**; a room is per **channel** (the channel id doubles as call id **and** media-key id); participants heartbeat ("voice-ping"), everyone tracks `{server:channel → {fp: lastSeen}}` with a staleness timeout + cleanup tick, so each channel shows a live "🔊 N in voice" pill and the header reads "Join voice (N)". A room you're *not* in going active raises a banner + chime, gated by a per-server "notify me of voice calls" toggle. Frontend-only | ✅ `b93164e` |
| 11e | **server-provided TURN**; the operator sets one TURN endpoint (Server settings) that rides the invite as a `.turn.<b64json>` suffix, stripped by the joiner before the bare hex reaches `join_server` and stored per-server in localStorage; `iceServers()` merges it with the user's personal STUN/TURN. Frontend-only, **no protocol change**: TURN is a non-secret hint (media is E2E DTLS-SRTP, so a hostile TURN relays only ciphertext or the call falls back), so it needs no signing and doesn't touch invite crypto | ✅ `7492f92` |
| 11n | **networking QoL**; best-effort **UPnP/NAT-PMP** (`upnp` feature + `MeshBehaviour.upnp`): the router auto-opens the listen port, we `add_external_address` (so identify + rendezvous advertise it) and surface it via `next_external_addr()`, signalling `None` promptly on `GatewayNotFound`/`NonRoutableGateway` so a waiter short-circuits. `found_server` waits ≤4s for it when the user left advertise **and** relay blank, folding `/ip4/<public>/tcp/<port>/p2p/<id>` into the invite; so the very first invite is directly dialable with **no relay and no port-forward**. Limits: founder-only, router-dependent, useless behind CGNAT (deploy a relay instead); untestable in CI (needs a live router). Plus `build_relay_swarm_with_key`/`build_rendezvous_swarm_with_key` + `catcomsctl relay/rendezvous --identity <file>`; a **stable peer id** across restarts (previously every launch minted a fresh identity, silently invalidating every already-shared invite) | ✅ `bf28db9`, `f317c5c` |
| 11u | **desktop UI overhaul; tokens-first "operator terminal" redesign + user customisation.** A declared **token layer** replaced ~90 hardcoded hexes (the old CSS referenced `var(--accent, …)` etc. but *never declared them*; two palettes shipped at once); default preset **Nightshade** (purple-shifted slate) + `aurum`/`verdant`/`garnet`/`slate`, semantic colours have fixed jobs in every theme (green=presence, gold=mentions, red=danger). Reskin: flat **timestamp-gutter** message log (day dividers; grouping never crosses midnight), mono micro-labels, dedicated member column (online/offline groups, role abbrevs), global **status bar** (node/peers/vault/rendezvous/transfers/own-id), squircle rail with hand-drawn **SVG line-icons** (emoji chrome fully replaced; found+fixed `.call-start`'s green never applying under `button.ghost` specificity; the 📎 stays by request). Nav: **surfaces strip** atop the content column + **contextual sidebar** (chat=channels · wiki=pages · files=folders+actions · transfers=clear) killing the wiki double-sidebar; **Ctrl+K quick switcher**. Customisation: Settings → Appearance (preset, accent override, compact density, terminal-chrome scanlines, flatten-bubbles, flat-icons) persisted in `catcoms.appearance`; Discord-style **name styles** (gradient/neon effects, script/caps fonts, swatch picker; `fxClass` now **sanitizes peer-supplied effect ids** before they reach a class attribute); default unicode emoji under the server set in the picker. Frontend-only | ✅ `ba8c20a`, `4ff7b99` |
| 11v | **server livery + shared server icon** ([`design-livery.md`](design-livery.md)); owner/admin publishes a colour scheme members inherit; per-server user opt-out. `DocType::Livery = 10`, one CRDT doc per server mirroring the Profile path end to end (lazy open, doc sync + **snapshot catch-up**, generic persistence); writes owner/admin-gated at the same policy layer as roles; sizes capped (`MAX_LIVERY_*`); values stored opaquely and **validated client-side** (preset allow-list, `#rrggbb`, colour-token allow-list; recolor-only by construction, semantics untouchable, no URL-shaped values). Precedence: user per-server opt-out > livery > own appearance. **Server icon** rides the same doc (additive `icon` key, 64 KiB cap, own `set_server_icon` command; `set_livery` is a read-modify-write that preserves it); rail shows it live via `livery-changed`, viewers can prefer monograms ("flat server icons"). Round-trip + icon-survival tests | ✅ `9d128fb`, `1bea14a` |
| 11w | **verify dialog + channel topics.** Out-of-band **identity verification** surface (the eclipse banner's "verify a member out of band" finally has UI): both fingerprints in read-aloud 4-char groups, explicit wording, **local-only** verified marks (`catcoms.verified.<server>`, never gossiped, no crypto weight) → ✓ in roster/profile. **Channel topics**: there is *no backend channel registry* (id = BLAKE3(name), the list is frontend-local), so the topic is a ROOT **LWW scalar in the channel's own doc**; replicates/seals/catches-up like messages, 256-byte cap, **any member** may set (channels are open-create; a topic is content), rides `channel-updated`; header click-to-edit UI. Tests: topic round-trip/cap/multibyte, two-node convergence with a non-owner writer, sealed-store reload. Delivery-states design written ([`design-delivery-states.md`](design-delivery-states.md)): sync-derived (`their_heads` ⊇ op hash), **no read receipts by design**; D1–D3 in progress | ✅ `1757732`, `3c19d07` |
| 11x | **delivery states + channel topics + member badges.** *Delivery* ([`design-delivery-states.md`](design-delivery-states.md)): neither design route existed (no automerge sync protocol; gossip + full-log pull), so delivery is **signed causal evidence**: a member counts once it authored a change descending from yours (`edit_tracked` returns the change hash at author time; `holders_of` answers a batch in one pass; counts are lower bounds that only rise, silent receipt invisible; **no read receipts by design**). Actor keeps a bounded id→hash map, emits `delivery-changed` (≤1/s/channel); UI: gutter ticks `✕ ◌ ~ ✓ ✓✓` (red only for "no peers reachable"; `--info` blue joins the fixed semantic set) with honest hover copy. *Topics*: no channel registry exists (id = BLAKE3(name)) so the topic is a ROOT LWW scalar in the channel's own doc (256 B, any member, `channel-updated`), click-to-edit in the header. *Badges*: `DocType::Badges = 11`, owner/admin-assigned `fp → {label, color}` chips (roster/profile/role-manager + inline editor); role names reserved backend-side, ignored client-side. Badges re-key to user ids under [`design-multi-device.md`](design-multi-device.md) M3 | ✅ `5b97524` `77f184d` `3c19d07` `1bc07ed` `790b5a2` |
| 11y | **safe livery customisation + events/news + event refs + unlock minigames.** *Customisation* ([`design-livery-customisation-safety.md`](design-livery-customisation-safety.md); raw HTML/CSS is RCE/overlay-phishing in a `csp: null` WebView; **rejected**, incl. for profiles): radius/font/pattern as **catalog ids** in the existing bounded tokens map (client validates per key) + **custom cursor** as inline re-encoded image bytes (own `set_server_cursor`; `set_livery`/icon/cursor mutually preserving, test-pinned; read-side deep validation incl. a minimum-opaque-area anti-griefing floor; `, auto` fallback always). *Events*: the reserved `DocType::Calendar = 4` finally lands; status-path mirror, any-member create, author/owner-admin delete, ⧗ surface (Ctrl+7) + sidebar next-5; **news feed** = inbox Mentions｜News toggle aggregating upcoming events + recent status posts across servers client-side (wiki joins once saves carry timestamps). *Event refs*: `[title](event:ID)`; the "+" picker's fourth kind, seam-tested against the renderer grammar. *Unlock minigames*: Passphrase (recommended) ｜ Spell (24 glyphs, indexed) ｜ Melody (pitch-class piano: on-screen, DAW home row, **Web MIDI**); every method encodes to a scheme-prefixed string into the **unchanged vault KDF**, with a live entropy meter (red <28 / gold <44 / green ≥44 bits). CSP hardening for the WebView remains a named follow-up | ✅ `f253930` `e10515c` `c9a4e66` `e16efc4` `405e896` `5e73e5e` |
| 11z | **multi-device M1+M2; pairing primitives + the grant ceremony** ([`design-multi-device.md`](design-multi-device.md) v2.2; **adversarially reviewed pre-commit**, BLOCKING findings fixed). Model per owner review: the **origin device is the identity root** (no account key; chain depth 1; master transferable-not-distributable via monotonic `MasterHandoff`), one device per single-use grant. M1 (`f6b7386`): `PairingRequest` / 6-digit **SAS** (domain-separated BLAKE3, bias < 2⁻⁴³) / `DeviceCertificate` + `DeviceRevocation` (carry-the-pubkey verification mirroring `InviteToken::verify_self`; names reject control/bidi/zero-width). M2 (`fe618d3`): the **offline-first paste ceremony**; begin → read (backend stores THE pending ceremony; **mint takes no blob**; TOCTOU closed, the human gate exists backend-side) → SAS-gated popup (pre-mint comparator = **device code**, SAS = post-delivery check; **scope disclosed**; decline **burns the nonce**) → passphrase-sealed all-server bundle (vault primitives verbatim + distinct HKDF label; ≥ 8-char transport passphrase; the sealed bundle is the only object ever linking per-server identities) → open (every cert verified FOR this device; certs **group-bound in the signed payload**). Per-server signing via a narrow `SignDeviceCert` actor command; keys never surface. Dead v1 account-key `cert.rs` (709 lines, zero users) deleted; `/v2` domains prevent cross-verify. M3 (admission via the owner-serialized queue) in progress | ✅ `e44bfe3` `cd8e300` `f6b7386` `fe618d3` |
| 11z-2 | **multi-device M3–M6; admission, attribution, revocation, carry channels** ([`design-multi-device.md`](design-multi-device.md); M3 **adversarially reviewed**, 3 BLOCKING findings fixed). **M3+M4** (`ba8a8d1`): a companion joins by presenting its group-bound `DeviceCertificate` through the owner-serialized add queue (`CTRL_DEVICE_ADD`, single committer → no fork); the owner-signed `Devices` doc (`DocType::Devices`) gives every member the companion→origin map for attribution; UI nests companions under their member with a mono device tag, owner device panel, "join granted servers" flow. Review fixes: the `Devices` doc entry now carries the **owner's signature** (a certificate proves an origin *wanted* a device, not that the group *admitted* it; an unsigned entry can't poison the depth-1 gate or spoof attribution); the relay path **authenticates before republishing** onto the control topic; a **per-origin device cap** bounds owner-executed Adds; asymmetric freshness; the invite self-gate treats an unreadable roster as "relay, let the owner decide". **M5** (`cca00bb`): `revoke_device` (origin-signed `DeviceRevocation`, owner-enforced MLS Remove, honoured only when the origin matches the companion's *registered* origin so A can't evict B's device) + `remove_member` **cascades** to a kicked member's companion leaves. **M6** (`bcd5e17`): QR + a hand-rolled acoustic FSK modem carry pairing blobs (and invites), both unit-tested. **Deliberate deferral:** `MasterHandoff`'s primitive is committed but inert; consuming it (per-group master state + monotonic seq) is future work; the common flows don't need it. | ✅ `ba8a8d1` `cca00bb` `bcd5e17` |
| 11z-3 | **melody lock, engraved; chords + note values + playback** (client-only; the vault KDF stays untouched). The melody minigame graduates from a note-name chip list to a real **grand staff** rendered from `melody.ts` (pure, unit-tested): diatonic step placement (C♯ shares C's line and carries an accidental), auto-ledger lines with middle C in the gap, second-interval head offsets, stem direction per staff, note-head shape/flag by duration, chord symbols over the bar, and a viewBox that grows to whatever register the tune reaches. Input becomes **held**, not triggered: keys sustain while down (on-screen, home row, and Web MIDI note-off is now handled), overlapping notes collapse into **one chord event**, and hold time quantises to eighth/quarter/half/whole. **`melody:v2` → `v3`**; `60+64+67.2-62.0` (chord tones joined with `+`, ascending and de-duplicated so fingering order cannot fork the secret; the `.N` duration class is omitted *entirely* when rhythm is off, so the two modes can never collide). v2 joins v1 in retirement: a vault sealed under either must be re-entered under a scheme this build can still produce. Rhythm is **opt-out** (persisted locally) because it is the one setting that can lock a correct player out of a vault with no recovery path; the entropy model stays deliberately pessimistic (+2 bits per extra chord tone, +1.5 for a duration class). Also: **1–7** jump register and z/x blip the new bottom C, ▶ playback of the *recorded* durations with the sounding event lit on the staff, and the piano keys are finally ivory and ebony. | ✅ |
| 11z-4 | **wiki overhaul; per-page md/wikitext, auto-contents, page tools.** The wiki grows Wikipedia's bones without losing the friendly path: each page declares a **render format**; `md` (default) or `wiki` (a MediaWiki-wikitext subset); toggled per page in the editor and **shared with every member** (stored in the wiki CRDT under a reserved NUL-prefixed root meta key `"\u{0}meta"`, a Map so it's *invisible to older readers* whose `read_wiki_map` only materializes `Text` values; name validation now rejects NUL-prefixed/>120-char pages). Backend: `wiki_meta`/`set_wiki_page_format`/`delete_wiki_page`/`rename_wiki_page` (rename = copy+delete; a concurrent edit on the old key loses, documented) + actor commands with error replies + 4 bridge commands; the wiki change-detector compares **bodies and formats** (a toggle is body-invariant); meta reads/deletes span **all conflicting meta maps** (`get_all`) since two members lazily creating the map concurrently would otherwise silently drop one side's formats; merge test pins it. Renderer: new pure `wikitext.ts` (no DOM; node-testable; `== headings ==`, `'''bold'''`/`''italic''`, `* / #` nested lists, `; :` definitions, `{\| \|}` tables + caption, leading-space pre, `<nowiki>`, http(s)-only external links, `{{templates}}` inert) emitting the **same placeholder vocabulary** as the markdown path (render.ts now imports its token builders; byte-identical by construction, same DOMPurify allow-list +`caption`/`dl`/`dt`/`dd`), fuzzed 60k hostile inputs; **piped `[[Page\|label]]` links everywhere** (chat/status/wiki); `#REDIRECT [[Target]]` + `__TOC__`/`__NOTOC__` helpers. UI (matched to the operator-terminal chat chrome): article view with title rule, **auto-Contents box** (3+ headings, hierarchical numbering, hide/show), hover-a-heading **section-edit jumps**, redirect-following with a "Redirected from" notice, backlinks as **What links here**; editor gains the md/wiki switch, a format-aware toolbar (B/I/H2/H3/link/lists/table, Ctrl+B/I/S), **live side-by-side preview**, and per-page in-memory **drafts** (following a link no longer discards unsaved edits); page header rename + two-step delete; sidebar `wt` badge. 124 frontend + 79 catcoms-app tests green | ✅ |
| 11z-5 | **sigil lock; multi-factor magic circle; `spell:v1:` RETIRED** (client-only; the vault KDF stays untouched). The spell minigame is **deleted**; `spell:v1:` can no longer be produced, so a vault sealed under it must be re-entered under a surviving scheme, the same retirement contract as melody v1/v2; `UnlockMethod` is now `"pass" \| "sigil" \| "melody"`. Its replacement is one screen, freely re-editable factors assembling into a single SVG magic circle: a **path over a fixed 19-node lattice** (centre + inner 6 + outer 12; **indices frozen forever**, geometry cosmetic; hard-snap hit-testing; disjoint catch discs *smaller than the node art* give dead-zone hysteresis, pinned by `min-spacing > 2·CATCH_R`; order + direction significant, multi-stroke via pointer-lift with `_` separators), optional **per-node colour marks** (4 variants cycling on tap, fully independent of the path; tap-vs-drag split by a pure `classifyGesture`; one node within `TAP_SLOP` = mark, a second node = stroke, a long one-node wander = nothing; each variant a distinct **shape** as well as hue for colour-blind users; keyboard `C` cycles), a **focus-emoji SET** (select/deselect toggle, ≤ 8; each element codepoint-encoded; lowercase hex `-`-joined, never catalog index; NFC, VS16 + skin tones ALWAYS stripped, ZWJ kept; the set is **canonically sorted + de-duplicated** so toggle order can't fork the secret; the `normalizeEvent` lesson; `+`-joined like chord tones), and a **magic word** (NFC + trim, case preserved, **length-prefixed** so a delimiter inside the word can't collide two secrets). Wire format **`sigil:v1:<path>:<colours-19>:<emoji-set>:<len>:<word>`**; colours a fixed-width 19-digit 0–3 field, all-zeros encoded literally; `""` when path/emoji/word missing (marks optional). ⚠ **v1 amended IN PLACE** (emoji field became a set, colour field inserted); legal only because sigil:v1 never sealed a committed vault; **any test vault sealed with the earlier working-tree build will not reopen.** The ring inscription leaks NOTHING: a **constant-count** rune band (42, full circumference; per-character runes leak length, tiled repeats leak it via the period) derived from `(session seed ⊕ fnv1a(word))`, reseeded per mount, so typing reshuffles it visibly but a photo recovers nothing; opt-in "show my word". Entropy stays pessimistic (2.5 bits/hop; Android-pattern bias; 6 first emoji + 3/extra cap 15; same popular head, correlated picks; word 4+2/char cap 20; marks 1.5/non-default node cap 12). The **cat summon (≤900 ms) runs concurrently with the KDF** and aborts on failure; the particle rAF loop stops on unlock/teardown/`visibilitychange`; `prefers-reduced-motion` skips both. `sigil.ts` pure + node-tested (42 tests) mirroring `melody.ts` | ✅ |
| 11z-6 | **fileshare pass; upload dedup · circulation expiry + wiki pin · Properties · toasts · wiki "+" picker** (follow-up to 11z-4 after owner feedback: "attaching doesn't seem to work", wanted toasts, the chat "+" in the wiki, no-reupload on same hash, wiki files exempt from decay, expiry + used-in + right-click Properties). *Attach fix + toasts* (`074a058`): the wiki attach pipeline was sound but **silent**; the marker appended off-screen at the textarea bottom with no confirmation; embeds now insert **at the caret** (edit-mode auto-switch, focus restored) and a **toast stack** (info/ok/err, in-place morphing) narrates every upload path (wiki/chat/status/files) + wiki save/rename/delete/format, failures loud with the real error. The composer's **"+" insert picker became a shared snippet** with an `insertTarget` routing to the chat or wiki caret; it opens from the wiki toolbar (drops down; composer's drops up) alongside a relocated 📎. *Upload dedup* (`5d2dc0e`): `add_file` computes the plaintext cid first; same name+normalized-folder → idempotent no-op returning the existing cid; different name/folder → one new index entry carrying the twin's encoded `FileManifest` **verbatim** (same ciphertext-cid chunks + wrapped keys → zero new storage; ciphertext identity can never dedup since seal_file is randomized per chunk). Backend-only `delete_file_at(cid, path)` unlists one listing (public `delete_file` still unlists all); dedup-safe GC composes (shared chunks survive until the last listing goes). Tests prove blob inventory unchanged AND manifests byte-identical. *Expiry + pin + usage* (`8461d8b`): `FileEntry.expires: FileExpiry{Unrecorded\|Never\|At(ms)}` (doc key `"exp"`: absent=legacy, automerge **explicit Null**=keep-forever, int=deadline; three states survive a merge), stamped `now+30d` on both add paths (dedup relistings stamp fresh); `set_file_expiry` per-listing (uploader/owner/admin R6 gate, proven-as-gate in tests); `wiki_pinned_cids()` **derived** from live wiki bodies (both `](cid:` and `](file:` grammars); un-pins when the page drops the embed; `file_usage(cid)` inbox-style scan → wiki pages + status/chat counts. **HONEST SCOPE: metadata + surfacing only; `RetentionIndex` is still unwired, nothing evicts yet**; `wiki_pinned_cids` carries a MUST-consult note for the future retention GC. UI: Properties gains **Circulates until** (pinned → forever → date `· in 30 days` → `not recorded`) + plain-language non-deletion note, **Used in** (clickable wiki pages, chat/status counts), **Keep forever** toggle, **right-click → Properties on any embed** (chat/status/wiki share the context path), Files-tab 📌 on pinned rows. Known tradeoffs doc-commented: dedup'd listing inherits first upload's mime; per-device index view; dedup of never-downloaded content lists chunks held elsewhere. 91 catcoms-app + 150 frontend tests | ✅ `074a058` `5d2dc0e` `8461d8b` + UI in tree |
| 11z-7 | **360 server space (orbit view); client-only.** A memory-palace overlay over the rail (mockup-approved on the shared design canvas): servers hang as billboards on a sphere around a rotation-only camera (yaw wrap + pitch clamp ±60°, `space.ts` pure + node-tested: project/unproject round-trip, lasso capture, group-offset carry, defensive store parse). **No WebGL**: the backdrop is a CSS 3D cube (4 SVG walls + floor/ceiling, tokens-only so presets/accent recolor the room; `den` ships the sleeping mascot on the windowsill) and icons are JS-projected onto a flat layer with the **same focal length** (`spaceF`, window-derived) so the layers never drift. Gestures: drag looks (grab semantics), **press-and-hold grows a lasso** from the cursor (one gesture for 1..N servers; capture → the constellation rides the aim as angular offsets → click drops; capture-phase click swallow keeps the drop from opening a server; pointer capture starts only when a drag/lasso commits so plain clicks reach the buttons), tap opens (`switchServer` folds the view), right-click → return-to-tray. **Tray** = hold `[T]` (keyup/blur-safe) or pinned via the hotkey chip: unplaced/new servers wait there; tap flies one to the reticle. **State reads**: unread/dot = breathing accent glow, mentions keep the gold rail badge, hover pulses in the server's **livery accent** (`--sp-a`, fetched per server on open) + name label. Backdrops: `den`/`ridge`/`void` presets + **custom equirect 2:1 image** (canvas-downscaled data URL; v1 shows equirect quarters flat on the cube walls, near-field distortion accepted). Persistence `catcoms.space` localStorage, **per-device by design** (like desktop icon positions); Settings · Appearance gains backdrop tiles + custom upload + forget-placements. Ctrl+O toggles; Escape chain releases carry → pinned tray → view; lock closes it. All motion behind `data-motion` + `prefers-reduced-motion`. 216 frontend tests | ✅ in tree |
| 12 | Android (Tauri 2 mobile): JNI keystore, foreground service, two-tier keys | planned |
| 13 | hardening: calendar, cover traffic, supply-chain attestation, metadata-index aging, **security review** (deeper adversarial scenarios land here) | planned |

### Earlier blocks (history)
6d-1b (missed-commit recovery + past-epoch key window) and 6d-2 (fork resolution +
the convergence-safe single-serializer membership model) are complete; their details
live in the commit messages and [`design-6d2.md`](design-6d2.md). The default config
stays **single designated committer**; the concurrent-committer / fork path is OFF
by default (`max_committer_rank=0`) pending **I1** (a wall-clock contest window can
converge honest nodes differently under async timing); re-read `design-6d2.md` before
touching it.

## 6e-3d; COMPLETE (rendezvous discovery + eclipse-resistance)

**Read [`design-6e-rendezvous.md`](design-6e-rendezvous.md)**; the 9-slice contract
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
deferred to the dedicated hardening + security-review phase (**13** in the table above;
it was numbered "10" before the UI overhaul took that number).

**Goal:** members find each other with no hard-coded bootstrap addresses, and an
attacker cannot isolate (eclipse) a member. Everything is built on a per-removal
routing secret `ns_secret_L`:

- **Foundation (3d-1/2a/2b, done).** `ns_secret_L` is snapshotted at each member
  **removal** (counter `L`), retained `{L-2,L-1,L}` in `ChannelSync`. The blinded
  gossip topics **and** rendezvous namespaces both derive from it (keyed BLAKE3), so a
  non-member can't compute them and they **rotate on removal** (forward secrecy for
  routing metadata). Because the secret is epoch-specific, the **join handshake
  transfers it** (sealed, bound into the inviter signature) so every member; founder,
  joiner, *post-removal* joiner; derives identical topics. Re-keying the topics
  **closed the pre-existing A1 CRITICAL**.
- **Discovery (3d-3/4, done).** A zero-knowledge rendezvous **server** (`catcomsctl
  rendezvous`) and a **client** in `MeshBehaviour`: register a signed peer record under
  a blinded namespace, discover others. Discovered records are **surfaced, never
  auto-dialed**; the dial decision (and eclipse-resistance) lives a layer up.
- **Source trust (3d-5, done).** Commit catch-up responses are now **signed** by the
  responder's MLS leaf key, bound to the request; a **two-pool** model separates
  untrusted candidates from verified `member_peers`. **Closed Sybil-C1.**

**Eclipse-resistance (3d-6…9, done, each adversarially reviewed):**
- **3d-6**; pure `catcoms-discovery` `DiscoveryPolicy` (rank candidates → bounded,
  Clock-paced/RNG-jittered dial plan; ≤1 root/rendezvous; roster clamp; seq-freshness;
  the only thing that decides what to dial). Plus the pre-dial **membership tag** and
  the catch-up **nonce/epoch anti-replay** (closing the 3d-5 deferred items).
- **3d-7**; **member PEX** (`KIND_PEX`): members supply each other dialable
  `PeerDescriptor`s without a rendezvous; responder-signed, members-only, capped +
  rate-limited; entries are discovery candidates (never auto-promoted to the trusted
  catch-up pool).
- **3d-8**; advisory **`EclipseDetector`** (D/R/S + hysteresis; never gates) +
  cross-session **`AddressCache`** (proven members, tamper-detected load).
- **3d-9**; invite rewiring (signature-bound `rendezvous` vector, `INVITE_DOMAIN`
  v2), pre-join **`join_namespace`**, and `serve --rendezvous`/`join`
  **discover→dial→join** (DiscoveryPolicy-mediated); verified by a memory end-to-end
  test (no hard-coded server address).

## Voice (group calls)

Shipped `bd483b5` → `7492f92`; contract in [`design-voice.md`](design-voice.md)
(design phases 1–3 are in, phase 4 is not). **All media-plane code is frontend**
(`apps/desktop/src/App.svelte`); the Rust core only derives a key and relays opaque,
authenticated signalling.

**How it works today**
- **Rooms are per channel.** The channel id doubles as the call id *and* the media-key
  id, so "join #general's voice" is unambiguous. Presence heartbeats give each channel a
  live "🔊 N in voice" pill; a room going active raises a per-server-gated banner + chime.
- **Media plane:** a full **WebRTC mesh** in the webview; one `RTCPeerConnection` per
  other participant, no server in the path, so DTLS-SRTP is genuinely end-to-end. Mesh
  economics cap the useful size at **~8** (uplink = (n−1) × ~32 kbit/s Opus).
- **Signalling:** SDP/ICE over `KIND_CALL_SIGNAL`; members-only, signed,
  freshness-bound, `from` = the verified signer. Because signalling is authenticated,
  the **DTLS fingerprints can't be MITM'd**. Payload is opaque to the core; not deduped
  (every candidate must land); FIFO-bounded. **Signalling rides the existing mesh**, so
  two members must already be mesh-connected (i.e. chat works between them) before a
  call can be set up; STUN/TURN only fixes the *media* path.
- **Media key:** `media_secret(call_id)` off the MLS exporter at the current epoch;
  every member derives the identical key locally, it is **never sent on the wire**, and
  distinct calls are domain-separated. Test-pinned (`members_derive_the_same_e2e_media_key`).
- **NAT:** STUN by default, optional personal TURN (Settings → Calls), plus a
  **server-provided TURN** the operator sets once and every invitee inherits.

**Pending / honest gaps**
1. **The MLS-keyed frame layer (SFrame / Encoded Transform) is NOT implemented.** The key
   is derived and exposed to the webview (`call_media_key`), and the frontend does not yet
   call it; today's E2E property comes from mesh DTLS-SRTP + un-MITM-able signalling.
   That holds while media is peer-to-peer or TURN-relayed (a TURN sees only SRTP
   ciphertext); it would **not** hold behind an SFU, which is exactly what the frame layer
   is for. Don't describe voice as "MLS-encrypted media" until this lands.
2. **Design phase 3 remainder:** re-derive the media key on an **MLS epoch change**
   (using the bounded past-epoch window for in-flight frames) and VAD/DTX. A long call
   spanning a membership change currently keeps its original epoch's key.
3. **Design phase 4:** move media onto the libp2p relay/DCUtR fabric so calls need no
   third-party STUN/TURN (the ethos-consistent transport).
4. **Ring-at-start only**; a member who comes online mid-call isn't rung (the channel
   presence pill covers most of this in practice).
5. **No adversarial-review workflow is recorded for the voice slices.** 11a added
   protocol surface (a new `KIND` + an MLS exporter label); the working conventions call
   for a hostile review on that class of change. Worth running before voice is "done".

## Known limitations / deferred (the security-relevant ones)

- **Desktop networking: every path is wired; what's left is router luck + the
  dev/release build distinction.** The `apps/desktop` bridge binds all interfaces and the
  founder advertises a reachable address (LAN/public IP, `host:port`, or a relay-circuit
  multiaddr); joining dials every bootstrap address. So **same-machine** (blank), **LAN**
  (founder's LAN IP), and **internet via a port-forwarded public IP** all work;
  **relay-circuit NAT traversal** (8q) needs no port-forward on either side;
  **rendezvous auto-discovery** works in the UI (join with *no* address in the invite) and
  **post-join steady-state discovery** re-finds the group after a restart with no fresh
  invite; **UPnP** (11n) can make the founder directly reachable with no relay at all.
  Residual: UPnP is **founder-only and router-dependent** (CGNAT or a UPnP-disabled
  gateway still needs a deployed relay), DCUtR hole-punching still needs a relay to
  coordinate, and a **`cargo build` (debug) exe is a dev build** that loads the UI from the
  Vite dev server (`localhost:1420`) and shows "can't reach the page" on any machine
  without it; to distribute, build a release exe with the frontend embedded
  (`npm run build && npm run tauri build -- --no-bundle`; needs WebView2 on the target).
- **Blob store: persistent + sealed, but not yet last-copy-safe (8l–8s, 9h).** The
  desktop attaches a per-server on-disk **`SealingBlobStore`** once the vault is unlocked
  (9h-a), so files/avatars survive a restart and are encrypted at rest under `blob_key`;
  deleting a file reclaims its orphaned chunk blobs while keeping any chunk another file
  references (dedup-safe GC). `MemoryBlobStore` remains the pre-unlock/test path and is
  size-bounded (`DEFAULT_BLOB_BUDGET` 128 MiB, **FIFO**, 8s). What's still missing is the
  `catcoms-storage` **retention engine**: eviction is not holder-probe-aware, so it can
  drop the **last copy** of a blob (re-fetchable only while some holder is online), and
  there is no disk quota/expiry enforcement on the sealing store.
- **Persistence + encryption-at-rest are DONE (Phase 9); the residuals are recovery
  ergonomics.** [`design-persistence.md`](design-persistence.md) is the design; it shipped
  as 9a–9h: a passphrase-sealed key vault, per-server snapshots (MLS + docs + routing +
  ledger + commit log + peer records) written atomically under the vault, an on-disk
  sealing blob store, and a stable per-group **file-wrap key** minted at founding +
  transferred in the join handshake (so files are e2e ciphertext keyed by ciphertext CID).
  Residual: **no passphrase change/recovery path** (lose it and the servers are
  unreadable; there is no escrow by design, but there is also no re-key flow), and a
  corrupted/partial snapshot surfaces as a load failure rather than a repair.
- **Network admission is single-committer-only** (only the lowest-leaf-index member
  admits). Concurrent admits / fork resolution + cross-member single-use = 6d-2.
- **Commit catch-up needs a peer that still holds the commit.** A member behind by
  more than a serving peer's `max_commit_log` window can't recover via commit
  catch-up; a full snapshot rejoin (deferred) is required; the gap is logged and a
  bad source is excluded, but exhausting all sources surfaces only a warning (a
  recovery *event* to the app is a follow-up).
- **Catch-up auth is now nonce-bound (6e-3d-6).** Catch-up *responses* are signed by a
  current member and bound to `(group_id, requester pubkey, req_ts, **nonce**, **epoch**,
  bundle)`, and requests carry a fresh signed timestamp + per-request RNG nonce; so a
  captured response cannot be replayed against a different request and the same-ms `ts`
  collision window is closed. Residual: there is still no *server-side* seen-nonce log,
  so a captured *request* can be re-sent within `MAX_REQUEST_AGE_MS` (60s); harmless
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
- **Persistent sealed MLS storage** landed in 9c (`snapshot_server`/`restore_server`
  over openmls's in-memory provider, the whole sync state sealed under the vault). What
  remains from the original note: **SQLCipher** backing (for the address cache + a local
  metadata index) rather than sealed flat files.
- **Metadata** is the dominant residual: who-talks-to-whom, timing, group sizes, the
  member IPs a DCUtR upgrade reveals to the peer, and; now; a **rendezvous** node
  learning `namespace ↔ IP ↔ timing` for the registration TTL (a higher-value target
  than a relay; querying ≥2 rendezvous doubles the operators who see it). Per-rendezvous
  namespace diversification removes the cross-operator join key; rotation-on-removal
  limits long-term linkage but leaks a removal-cadence signal. **Voice widens this**: ICE
  reveals each participant's IP to every other participant (inherent to a P2P mesh), and
  a **STUN/TURN operator** learns `IP ↔ call timing ↔ duration` (a TURN sees only SRTP
  ciphertext, but it sees *that* you called and for how long); the default public STUN
  is a third party the rest of the system deliberately avoids, so privacy-sensitive
  deployments should point Settings → Calls at their own (or blank it for LAN-only).
  Mitigated, not eliminated (≥2 relays/rendezvous, cover traffic, staying relayed).
  Relays/rendezvous only ever see Noise+MLS ciphertext / opaque namespaces.
  See ARCHITECTURE §3.
- **Per-peer rate limiting / off-actor offload** of join work: a hardening follow-up.
- **`tracing` retrofit** for the earlier crypto/storage crates: deferred (user OK'd).

## Where the design/review outputs live

Design passes and adversarial reviews run as background `Workflow`s; their structured
output is under the session's `tasks/<id>.output`. The load-bearing conclusions are
distilled into the design docs and the memory files; **read the design doc for the
block you're touching**:
- `ARCHITECTURE.md` §1–§2; the four locked decisions + the initial corrections;
  §4a/§4b; join + commit propagation; §3; honest residual risks.
- [`design-6d2.md`](design-6d2.md); fork resolution / single-serializer membership
  (committer = lowest **leaf index**; the **I1** gate keeping concurrent committers off).
- [`design-6e-relay.md`](design-6e-relay.md); relay-v2 + DCUtR.
- [`design-6e-rendezvous.md`](design-6e-rendezvous.md); rendezvous discovery +
  eclipse-resistance: the 9-slice contract and the recorded per-slice adversarial-review
  outcomes (A1/2b and Sybil-C1/3d-5) with their deferred follow-ups.
- [`design-rendezvous-ui.md`](design-rendezvous-ui.md) ·
  [`design-postjoin-discovery.md`](design-postjoin-discovery.md); the same discovery
  machinery wired into the desktop client (found/join with no address; steady-state
  re-registration after a restart).
- [`design-persistence.md`](design-persistence.md); the Phase 9 vault/snapshot/at-rest
  slice plan (9a–9h).
- [`design-chunked-transfer.md`](design-chunked-transfer.md); chunked large-file
  transfer (`FileManifest`, per-requester bytes budget, whole-file CID verification).
- [`design-dms-friends.md`](design-dms-friends.md); DMs as 2-person servers + the
  in-band friend-request path (`KIND_DM_INVITE`).
- [`design-admin-invites.md`](design-admin-invites.md) ·
  [`design-grant-revocation.md`](design-grant-revocation.md); owner-serialized admin
  invites and the owner-local authoritative admin set.
- [`design-voice.md`](design-voice.md); **the active block**: E2E group voice
  (media key, `KIND_CALL_SIGNAL`, the WebRTC mesh, and the phase-4 transport plan).
  Cross-check it against [§ Voice](#voice-group-calls); the doc describes the frame
  layer that is **not** built yet.
