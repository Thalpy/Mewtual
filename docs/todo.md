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
| 8f | **member profiles (backend)** — shared `DocType::Profile` doc (name/color/font/effect), messages authored by device fingerprint | done |
| 8g | **profile editor + rich rendering** — customize name/color/font/animated effect; roster + messages resolve fingerprint → profile | done |
| 8h | **member avatars** — small inline display pictures (canvas-downscaled, capped); circular avatars in roster + messages, initials fallback | done |
| 8i | **per-channel history catch-up** — a joiner catches up the backlog of any channel it opens (not just #general) from the peer it joined through | done |
| 8j | **symmetric (any-peer) catch-up** — either side pulls the backlog of a channel the other created (best known peer) | done |
| 8k | **chat UX polish** — message timestamps (clock-stamped) + auto-scroll to newest | done |
| 8l | **content-addressed blob fetch over the mesh** — `KIND_BLOB_FETCH` (members-only, signed, capped, rate-limited); the foundation for large avatars + fileshare | done |
| 8m | **avatars over the blob layer** — profile carries the avatar's CID, not inline bytes; fetched on demand over the mesh | done |
| 8n | **fileshare browser** — per-server file index + upload/list/download (bytes via the blob mesh); Files panel in the UI | done |
| 8o | **cross-network founding/joining** — bind all interfaces + advertise a reachable address (LAN/public IP); joining dials all bootstrap addresses | done |
| 8p | **multi-server** — a Discord-style server rail; be in several servers at once (each its own group/channels/roster/profiles/files) | done |
| 8q | **relay-circuit founding** — reserve a circuit on a relay node so NAT'd peers connect with no port-forward (zero-config NAT traversal) | done |
| 8r/8s | **security-review hardening** — adversarial review of 8m–8q (no blocking findings); bounded avatar fetching + size-bounded blob store; [User Guide](docs/USER_GUIDE.md) | done |
| 8t | **status feed** — a per-server post stream (announcements/activity) + Status panel in the UI | done |
| 8u/8v | **wiki** — per-server collaborative pages (name→body map doc) + Chat/Wiki view toggle (page list + editor); page bodies are automerge `Text`, so concurrent edits merge **char-by-char** (8v) | done |
| 9 | **disk persistence + encryption-at-rest** — [designed](docs/design-persistence.md), **9a–9h done**: key vault, sealing blob store, snapshottable MLS state, doc + whole-server sync-state persistence, vault-sealed `ServerStore`/registry, the desktop passphrase-gate + reload-on-startup, peer re-dial, and e2e per-group file encryption (9c/9e/9h-b adversarially reviewed). Close/reopen the app, enter your passphrase → servers + history are back (read offline), sealed at rest | done |
| 10 | **desktop UI / product overhaul** — **10a–10h done**: tabbed nav + Settings overlay; a sanitized markdown renderer (`marked`+DOMPurify) with `[[wiki links]]`, `:emoji:`, and `![cid embeds]`; fileshare **folders** + drag-drop **media embeds** in chat/status (built in code from CID-verified blobs); a **wiki** overhaul (markdown, links, backlinks, media, in-app help); **custom emoji**; **notification sounds**; and **owner/admin roles** + a server-settings role manager. 10c & 10h adversarially reviewed; roles enforcement is documented as policy-layer (cryptographic hardening is a named follow-up) | done |
| 11 | Desktop expansion: discovery UI, chunked transfers, DMs/friends, events/news, search/inbox, delivery states, multi-device, wiki history/review, livery and voice surfaces | done/current |
| 12a | Signed moderation plane: event timeline, range warn/delete, collapsible warning evidence, kick cases/votes and owner-only resolution | done; full suites + antagonist gates passed 2026-08-20, R7 disclosed |
| 12b | Durable history UX: vault-sealed composer drafts/read positions plus safe legacy read-mark migration | done; full suites passed 2026-08-20 |
| 12c | Storage health/repair and connectivity assistant in the server sidebar, with storage repeated in Transfers | done; full suites passed 2026-08-20 |
| 12d | Backup & Recovery centre: coherent encrypted export now; locked staged verification/import/rollback remains a named security gate | partial; export in tree, restore deferred |
| 12e–12g | Notification controls (parallel review), voice completion (after user test), channel governance (later design) | queued/deferred |
| 13 | Android (Tauri 2 mobile): keystore, foreground service and two-tier keys | planned |
| 14 | Hardening: last-copy-safe retention, recovery import, AutoNAT, voice completion, supply-chain attestation and independent security review | planned |

The active, evidence-based status for the current development pass is maintained in
[`DEVELOPMENT-SWEEP.md`](DEVELOPMENT-SWEEP.md). It separates existing-but-unverified work from
work that has not been implemented and records contradictory older directions without silently
choosing between them.

Features/things in stack:
1. Draw a waveform or shared visualizer for MIDI/instruments in voice chat (fun; decide whether
   this stays a toy or becomes a small collaborative music surface).
2. Add better MIDI settings: ~~device routing~~, velocity curve, ~~sustain handling~~ and
   per-instrument receive controls. Settings → Devices now carries the controller panel (live
   device list, per-port routing, message monitor, panic, setup/troubleshooting help) and the
   pedal is handled for the call instrument. Velocity is parsed and shown in the monitor but not
   yet mapped to loudness, and per-instrument receive controls are untouched.
3. Explicit settings gaps: vault re-key, keybind remapping, notification granularity, and the
   protocol-backed Security / Server Nodes pages.
4. The agreed experimental queue — **Campfire rooms, Promote conversation to wiki, Memory
   Keepers, Ciphertext mailboxes, Guardian recovery, Two-way proximity invites, Community time
   capsules, and Trust constellations** — is tracked with per-feature antagonist questions and
   test/documentation gates in [`feature-implementation-tracker.md`](feature-implementation-tracker.md).

---

## Reachability / connectivity work (2026-08-19)

Triggered by a field failure: a remote user redeeming an invite got "timed out connecting to
the server". Full design, defect list and status board in
[`design-zeroconf-reachability.md`](design-zeroconf-reachability.md); section 1c there is the
authoritative per-defect board, this is the summary.

**Done.** The invite path (identity and port now persist, reload re-advertises, UPnP given a
real window, IPv6 and QUIC, invite signature checked before dialling), peer exchange and the
address cache wired end to end (which also fixed presence, cross-session re-dial and a
permanent false eclipse alarm), both deployment-blocking CRITICALs on the relay and rendezvous
nodes, a product-layer integration test suite, and the joiner control-topic bug that made
later members invisible to earlier ones.

**Every defect P1 to P14 is now worked to a conclusion** (2026-08-19). Nine closed outright.
P3 deferred by decision. P5, P6 and P10 partial with their gaps named. P9 **closed as a
decision**: its premise was false, since the membership tag is keyed on the same secret as the
rendezvous namespace and so never defended the attacker P8 was filed about. The per-defect board
in [`design-zeroconf-reachability.md`](design-zeroconf-reachability.md) section 1c is
authoritative; section 11 carries the loose ends that are not defects.

**Higher ladder status.** Stable direct ports, UPnP/PCP/NAT-PMP plus PCPv6 firewall pinholes, scoped AutoNAT v2, the live
Connectivity assistant, 60-second two-way replies and opt-in member admission switchboards now
exist. The AutoNAT server has pre-socket exact-source target policy plus peer/prefix/node limits,
but remains experimental/off by default because metadata, egress and same-NAT port probes remain.
mDNS, recurring/pairwise reachability, concurrent rung racing, the guided port-forwarding wizard,
general circuit switchboards/monthly host budgets, hosted mode and the public DHT remain. One item
is operational rather than code: **Mewtual starts with zero owned servers**, so a bootstrap node has
to be provisioned before rung 4 means anything.

**Partially implemented: the connectivity UI.** Onboarding and Settings share the mockup's live
status line, readout and diagnosis over real mapping/AutoNAT evidence, plus one-time reply help,
standing switchboard opt-in/status and direct-first joiner consent. The create-server mode,
Advanced redesign, automatic escalation, one-time helper approval popups and onboarding/wiki
questionnaire remain.

Working rule carried out of this: **adversarial review happens per slice, before the commit
that lands it.** Batching it at the end is how unreviewed work reached `main` twice.

## Two-client automated acceptance test

**Context (2026-08-26).** The diagnostics methodology explains a failure after it happens, but it
does not itself prove that two independently running clients can complete the user journey. The
repository already has the right fast foundation. See
[`TWO-CLIENT-TESTING.md`](TWO-CLIENT-TESTING.md) for the commands that work today and the proposed
operator contract for the packaged two-process harness:

- `crates/catcoms-app/tests/product_e2e.rs` drives the same `ServerActor`/`AppEvent` product surface
  as the Tauri bridge, deterministically over the in-memory mesh;
- `crates/catcoms-app/tests/tcp_product_e2e.rs` runs two real libp2p nodes over OS TCP sockets and
  verifies join, chat convergence, presence and disconnect;
- the lower-level sync suite separately covers direct, rendezvous, relay and DCUtR paths.

These are product integration tests, but they do not launch two packaged desktop processes, load
two independent vaults, or cross the webview/IPC boundary. Keep that distinction explicit rather
than calling the existing suite a complete desktop acceptance test.

**TODO — fast discrete two-client scenario (normal CI).** Extend the real-socket product test, or
add a neighbouring `two_client_acceptance.rs`, with one named scenario and a small reusable `Node`
harness. It should use ephemeral ports and temporary stores, bound every wait, and assert visible
outcomes rather than log text:

1. Alice founds a server and mints a fresh invite; Bob redeems it.
2. Each sends a uniquely identified message and the other observes it.
3. Transfer a small deterministic file and compare its bytes/CID at Bob.
4. Stop Bob, assert Alice observes the disconnect, then start Bob from the same sealed store.
5. Assert rediscovery/reconnection and catch-up without minting another invite.
6. Export each node's bounded diagnostic report only on failure, labelled Alice/Bob and with the
   scenario seed, phase and exact commit in the test artifact.

Do not use fixed sleeps as success criteria, share a vault/store between nodes, assert prose log
messages, or run both discovery passes simultaneously. The existing product tests document why
simultaneous PEX passes can make both actors wait for their request deadlines.

**TODO — two-process desktop smoke test (nightly/release gate).** Add a Windows-first harness that
launches two copies of the built application with separate temporary app-data roots and drives a
minimal stable automation surface. Prefer purpose-built, debug/test-only commands addressed by
semantic operation (`found`, `mint_invite`, `join`, `send`, `wait_for_message`, `shutdown`) over
screen coordinates. The automation surface must be impossible to enable in a release build unless
the explicit test feature/configuration is present. Capture both process exit codes, screenshots,
startup logs and privacy-filtered diagnostics on failure. Run this after building the desktop, not
in the root Rust matrix; the desktop is a separate Cargo workspace and CI currently does not build
it there.

Start with join + bidirectional text + clean shutdown. Add restart/catch-up next, then one small
file. Relay/rendezvous/NAT fault injection belongs in separate scenarios so the basic smoke test
stays quick and failures identify one boundary. A real cross-network canary remains useful, but it
is monitoring infrastructure rather than a deterministic pull-request test.

## Endpoint scheduler follow-up hardening

**Context (P-fixes adversarial re-review, reviewed `a165e31`, rechecked at `e896dec`).** Canonical
transport principals, connected-only reciprocal proof requests, separate relay-circuit attempt
keys and exact WebSocket-path parsing resolved or explicitly accepted the three MEDIUM findings.
The present scheduler deliberately accounts dial-command submissions, not confirmed socket starts.

- **TODO:** replace scalar `DiscoveryPolicy::refund_endpoint_budget(count)` with an opaque,
  single-use reservation/permit tied to the exact planned endpoint.
- **PARTIAL:** exact single-use permits now reach the network actor's `Submitted`/`Suppressed`
  decision. Duplicate/already-connected suppression and failed command delivery refund by dropping
  the uncommitted permit. A distinct post-flush `started` acknowledgement and cancellation after
  actor acceptance remain.
- **TODO:** separately lease a genuinely new outer relay socket while keeping logical relay-circuit
  attempts distinct, so shared relays neither collapse unrelated peers nor evade exact-socket
  accounting.
- **TODO:** add a process-wide in-flight/concurrency lease across swarms; libp2p's current pending
  outgoing limit is per swarm.
- **TODO:** add actor-level suppression, cancellation and new-outer-relay regression tests. Preserve
  the existing connected-only proof test and canonical cache/rendezvous identity test.

Until these land, documentation and diagnostics must continue to say **bounded dial-command
submissions**, not claim a transport-wide bound on actual socket starts.

## Field test, 2026-08-19: first successful internet P2P session

Two machines, different networks, over the internet. **Text, image and audio transfer all
worked.** The founder's router cooperated with UPnP, so there was no relay, no port forward and
no manually entered address: the zero-config direct path (rung 0b) carried a real session. This
is the first end-to-end confirmation that the invite path works for a remote user, which is the
failure this whole workstream started from.

Issues observed, in the user's words, to be worked after the outstanding P-defects:

1. **An existing (pre-update) server's invites did not work; a newly created server's did.**
   Suspected cause: a server founded before the identity-persistence work re-keys once on first
   launch, and its `advertise`/`relay` settings were never persisted so they are lost. The
   `get_invite` self-heal (`67a32b7`) should now cover the address half. **Needs verifying
   against a genuinely pre-existing server.** The user's suggestion, worth considering on its
   own merits: require an explicit "create new invite" before any invite code is shown, so a
   stale one can never be copied.
2. **Voice would not connect** over the internet, while text and files did. Expected shape: the
   media plane is a WebRTC mesh in the webview and does not ride the libp2p transport at all, so
   UPnP on the libp2p port buys it nothing and it needs STUN, or TURN under symmetric NAT. See
   `design-voice.md` phase 4 (move media onto the relay/DCUtR fabric).
3. ~~**No upload progress, and an upload appeared to finish and then hang.**~~ Fixed. Uploads
   became first-class in the Transfers tab, and the hang (an upload stuck at 10% taking the whole
   app with it) turned out to be a second, deeper fault: the entire file crossed the IPC bridge as
   one base64 string and was then sealed inside a single actor command, so both the webview and
   the server actor were occupied for the whole transfer. Both halves are now streamed; see
   [`design-chunked-transfer.md`](design-chunked-transfer.md) § Streaming.
4. **Messages do not clearly indicate when they have been sent.** The delivery-states design
   (D1-D3) is listed as in progress and is what this needs.
5. **In a DM you appear as the person you are DMing** (identity/label bug in the DM view). The
   user was also unsure the DM itself worked, so this needs reproducing before diagnosis.
6. **Debug output is needed** to make any of the above diagnosable by a user rather than by
   reading source. Being built now: a per-server join-attempt log with distinct outcomes, an
   opt-in `tracing` log with its path shown in the UI (the desktop app currently initialises no
   subscriber at all, so every warning it logs is discarded), and a connectivity panel on the
   create/join screens.
