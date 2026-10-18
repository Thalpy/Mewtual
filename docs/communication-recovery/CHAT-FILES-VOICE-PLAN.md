# Chat file sharing and voice reliability plan

Prepared 17 October 2026; amended 18 October 2026 following external design review.
Status: proposed work; no runtime changes or acceptance results.
Initial source baseline: `f3596ac859293293701b0e5d2dd0339bd7b1f5fe`. Amendments checked against
`b2f26d0b208ca67952032cf44cbcbcc509b49033` and the current worktree, which contains unrelated
Studio, native and frontend changes. Preserve those changes and record the actual implementation
baseline before starting each package.

Improve ordinary member connections and apply the existing Flipnote recovery patterns to chat,
file sharing and voice. Members connected through an authorized bridge should exchange chat,
obtain available files and discover the same voice room. Audible voice additionally requires a
working media route. The first implementation priority is to reproduce and repair the bridged
voice-room failure, alongside the connection failures exposed by that reproduction.

This extends [the recovery programme](IMPLEMENTATION-PLAN.md), rather than replacing it.
Use its invariants and [status ledger](STATUS.md), the canonical [message trace](../MESSAGE-FLOW.md),
[file contract](../design-file-reliability.md) and [voice contract](../design-voice.md).
Older descriptions contain superseded behavior; check production call paths before treating
a documented gap as an unfixed defect.

Keep this programme's acceptance ledger separate from [Gate 7 acceptance](../GATE7-ACCEPTANCE.md).
That gate covers the creative-suite create/save/restart/share/join/rotate/recover/export flow
and production command/event/recovery handoff. Sharing a branch does not expand its completion
criteria to this communication programme.

## Intended behavior

| Surface | Required outcome | Evidence needed |
| --- | --- | --- |
| Connections | Recover through permitted direct or indirect paths without repeated invite codes or an always-online founder. | Asymmetric links, changed listeners, bridge replacement, sleep and restart scenarios. |
| Chat | Preserve accepted messages and exact retry identities; reconcile missing history in both directions through changing neighbors. | Exact message ID, original signed-operation/device attribution and content union after partitions and restarts. |
| Files | Discover usable providers, retry interrupted requested transfers and verify complete copies without conflating metadata with possession. | Exact manifest/chunk verification, provider failure and restart tests. |
| Voice room | Bridged members see the same fresh participant set and exchange setup signals. | A-B-C with A-C blocked, including late join and bridge replacement. |
| Voice audio | Establish an encrypted media path when direct ICE fails and an authorized reachable relay is available. | Native WebView2 audio and selected-candidate evidence under forced relay conditions. |

An admission helper, a replica storing history, a libp2p circuit relay and a WebRTC media relay
have different responsibilities. A successful join or chat exchange proves none of the other
three. Recovery requires a surviving authorized copy and a usable path; a current voice call
requires contemporaneous connectivity, not a time-separated store-and-forward chain.

## Existing mechanisms and first investigations

Source observations below identify inspection targets, not reproduced root causes.

- Chat already has durable desktop sends, sealed pending retries, bounded signed catch-up,
  neighbor sweeps, saved member routes and six-client recovery fixtures. Extend and test these
  paths instead of introducing a second operation ledger or a parallel discovery service.
- `crates/catcoms-app/src/actor/file_transfers.rs` already prepares and commits work on the actor
  while detached workers wait on the network. It retains attempt permits through real completion,
  uses paged reads and retries, and sets its attempt deadline above the transport timeout.
  The file design's earlier eight-second description is stale. Audit current constants and tests.
- File Keep has an explicit local retention policy and bounded durable reservations. Publishing
  a file index entry does not establish another member's custody. Automatic replication and
  remote possession receipts need their own policy and protocol decisions.
- In `App.svelte`, `broadcastOn` uses `onlineCallTargets` from `call-identity.ts`, filtering room
  signals by the online roster. `ChannelSync::send_call_signal` in `catcoms-sync/src/lib.rs`
  resolves one exact device to a peer and uses `notify`. It provides no general multi-hop
  room dissemination. Trace what online means and what the transport actually connects.
- `serve_call_signal`, `authenticate_request` and `kind_binds_requester_peer` enforce existing
  member/signature/freshness and transport-binding contracts. Forwarding cannot be implemented
  safely by relaxing a shared authentication check or re-signing the origin as the bridge.
- Voice uses WebRTC in the webview. `iceServers` accepts personal and server-provided TURN.
  Existing libp2p traversal does not automatically establish the WebRTC media path. MLS-derived
  encoded-frame protection remains unimplemented according to the current voice security contract.

Relevant implementation seams include `catcoms-discovery/src/reconnect.rs`, native
`member_reconnect.rs`, `six_client_recovery.rs`, the app actor, sync catch-up/blob service,
`catcoms-storage/src/kept.rs` and `catcoms-mls/src/channel.rs`,
`voice-signaling.ts`, `call-identity.ts`, `callroutes.ts` and the call lifecycle in `App.svelte`.
The current worktree already modifies `App.svelte`; coordinate surgical changes with that work.

## Shared design rules

Reuse verified discovery, bounded pagination, retained obligations, capture then detached work
then live commit, and exact lifecycle revalidation from Flipnote. Keep document-specific
authorization and storage owners intact. Ordinary chat and file reads must not acquire an
online document-owner requirement.

Use one actor-owned recovery scheduling policy around existing domain owners. Consolidate timers
only after tracing them. Share route evidence, admission budgets and wakeups where appropriate;
keep chat obligations, file custody and ephemeral call sessions as distinct typed state.

1. Bind authority to authenticated group and full device identity. Numeric UI server IDs and
   short fingerprints are display/routing context, never authorization.
2. Keep membership epoch, document generation, actor instance, vault mount, account session,
   route evidence and call incarnation separate. Recheck relevant dimensions at commit/delivery.
3. Preserve original signed operations, manifest variants and operation IDs. Neither rendered
   equality nor a successful forward proves authorship, freshness, custody or convergence.
   Existing displayed-author/edit semantics retain the limitations in threat-model R6 until a
   separately scoped semantic-binding change closes them; recovery alone cannot close that gap.
4. Preserve dial permissions, private-route provenance and authenticated immutable group mode.
   A hint or helper statement cannot authorize private-address probing or a dedicated-to-P2P downgrade.
5. Bound bytes, attempts, fan-out, hops, replay tables, queued work and elapsed time per device,
   group and process. Keep cancelled lower-level work charged until actually released.
6. Reserve progress for membership/control, call setup and live traffic while backlog/blob work
   runs. Measure congestion and queue delay before choosing new numeric budgets.
7. Keep chat publication obligations and explicitly requested retained-file work durable.
   Expire room presence, SDP, ICE and audio. Never replay an old call after unlock or restart.
8. Keep private drafts, recovery archives and unaccepted Flipnote branches outside shared history.

## Implementation sequence

Each package delivers a small reviewable change with focused regressions and an updated evidence
ledger. Existing programme items CR02-CR08 and CR10 remain useful dependency labels; this plan
does not mark them complete.

Start with Package 0, narrow Package 1 fixes supported by reproductions, and Package 2's
room/session investigation. Run Package 5A's existing TURN feasibility experiment alongside
that early work; it need not wait for broader chat or file changes. Follow with Package 3 and
Package 4, then Package 5B implementation and Package 6 rollout after their design gates pass.

### Package 0 Reproduce failures and pin the baseline

Trace direct, reply and helper admission through saved routes, steady-state reconnect, chat
catch-up, provider selection, room broadcast, signal receipt and WebRTC creation. Identify
whether each field means current authenticated connection, claimed path or indirect reachability.
Distinguish a libp2p circuit connection from application forwarding through another member.

Extend the deterministic topology harness with A-B-C and forbidden A-C, a replaceable second
bridge D, fixed group membership initially, gossip loss, delayed replies and busy actor commands.
Reproduce missing voice presence separately from failed signalling and failed audio. Add the
narrowest failing Rust/TypeScript regressions before production fixes. Record a redacted native
support trace and baseline recovery latency, attempts, bytes and actor responsiveness.

Exit: each reported failure is reproduced or explicitly remains a hypothesis; current behavior
and preserved security invariants are pinned. No broad replacement is justified by flakiness alone.

### Package 1 Connection recovery and fair scheduling

Repair demonstrated route/discovery/lifecycle defects first. Preserve last-good authorized routes,
peer diversity and successful dial direction across restart. Rotate failed sources without tearing
down a useful indirect path to attempt a direct upgrade. Apply bounded backoff with injected time
and RNG, reset only on relevant new evidence, and prevent reciprocal retry storms.

Audit event-driven wakeups and periodic fallback after admission, peer/path change, newly acquired
history, failed persistence, unlock and network resume. Retain a single owner for each obligation
and ensure quiet networks still retry. Check relay reservation renewal and helper departure
without assuming every relay candidate is reachable or consented.
Persisted route successes are ranking hints; only fresh authenticated evidence establishes
present-time reachability.

Exit: direct/reply/helper scenarios recover in both restart orders and after bridge loss where
another permitted path survives. Heavy valid file/history work cannot strand control or another
server. Agree measured latency and bandwidth limits before accepting the package.

### Package 2 Repair voice room discovery and signalling

Build on Package 0 immediately; do not wait for all chat/file enhancements. Model room presence
and media connectivity separately. Replace direct-online-only eligibility with authenticated,
bounded room dissemination or scoped subscriptions through permitted neighbors. Choose the
smallest compatible mechanism after comparing existing gossip and forwarding infrastructure.
Targeted SDP/ICE needs a separately reviewed indirect route; sending announcements to every
roster entry alone is insufficient.

Before introducing a new envelope, specify original issuer, intended recipient or room, group,
call incarnation, request identity, sequence, expiry, membership/policy basis and integrity
protection. Preserve endpoint authentication independently of the helper's transport identity.
Specify signalling confidentiality and who may observe SDP/ICE address material; do not broaden
existing visibility accidentally. Keep old direct authentication intact.
Audit the existing signal's recipient and replay binding explicitly: `KIND_CALL_SIGNAL` is
currently outside `kind_binds_requester_peer`. Reusing its signature alone cannot establish the
new forwarding envelope's recipient, call-incarnation or duplicate-rejection guarantees.

Distinguish stable room identity, each participant's fresh join incarnation, and negotiation
generation within that join. Authenticate and pin the signalling protocol mode for the exact
endpoint session. A session established under the upgraded protocol must never be mutated by
legacy `bye`, heartbeat, offer, answer or ICE ingress. Supporting legacy direct peers requires
explicitly separate state/admission rules; it cannot supply a fallback writer into protected state.
Bridge changes, failed transport or missing/stripped capability fields cannot lower that protection.
A restart ends the old call session and requires fresh authenticated negotiation; it cannot
restore the old session with weaker checks or silently discard a retained protocol requirement.

Use fresh expiring self-announcements and challenge/session evidence where current liveness is
claimed. A forwarded old heartbeat cannot renew presence merely because it arrived recently.
Bound duplicate suppression and forwarding, prevent loops and storm amplification, and retain
per-origin limits across helper/connection changes. Reconcile missing setup state; handle lost,
duplicate and reordered offers/answers/ICE with negotiation and call-generation fences.

Leave, lock, group removal, room switch and actor replacement revoke pending effects. An old
bye cannot dismiss a newer join. A peer's own signed state cannot authorize another member's
presence, microphone, playback or recording consent. Update native mocks and UI tests as needed.

Fence queued RTC callbacks and asynchronous continuations by the exact peer object and call/join/
negotiation generation, not just the peer identity. Queue `ontrack`, connection-state and ICE
callbacks and work suspended in `setRemoteDescription`; replace the peer or call before releasing
them. They must not attach audio, send obsolete SDP or remove the replacement peer.

Compatibility regressions must deliver each legacy signal type through the old handler into an
upgraded session, including a different recipient, helper B replaced by D, same-channel leave/
rejoin and missing capability fields. None may renew protected presence or mutate negotiation.

Exit: A and C discover the same room through B, negotiate through B with A-C transport blocked,
and recover through D without phantom participants, stale audio, duplicate peer objects or
cross-room state. Signal submission remains distinct from remote receipt and media connection.

### Package 3 Close demonstrated chat recovery gaps

Retain the durable send barrier and exact sealed retry identities. Extend existing signed,
bidirectional catch-up across multiple providers, with fair paging and continuation ownership.
Discover channels created during absence; audit unopened-document retention and onward service
so an intermediate member can serve newly acquired admissible history without opening its UI.

Preserve locally accepted work during remote recovery, handle missing dependencies and unavailable
history explicitly, and audit membership/control recovery before data recovery. Choose typed
rejoin/removal/fork outcomes rather than retrying forever. Any historical-author semantic binding
or partitioned-membership change needs its own security design and compatibility review.

Exit: exact-union recovery survives gossip loss, provider changes, quiet retries, offline sends,
bridge restart and solo reopen. Local save, replica custody, recipient delivery and read status
have separate evidence. Add remote receipts only under a reviewed durable-write contract.

### Package 4 File recovery and authorized bridge service

Separate file-index reconciliation from actual encrypted chunk availability. Extend bounded
provider discovery and explicit-read routing so a requested file can be obtained across an
authorized member bridge when no direct provider path exists. Reuse existing blob authentication,
manifest identity, ciphertext CID checks, range/page bounds and cancellation accounting.

Specify whether a bridge serves its own retained copy or forwards an opaque request to a live
provider. The latter needs end-to-end response binding, connected-only forwarding, request/byte
budgets, loop prevention and no arbitrary address/URL fetches. An advertised holder is a candidate;
verify bytes and treat expiry as loss of evidence, not authority to delete a local copy.

Durable resume requires a reviewed retained-store state transition and migration first.
`KeptBlobStore::load` currently deletes `pending` under its exclusive lease. `begin_keep` puts
incomplete transfers there; `abort_keep` deletes them; `forget_kept` renames a released copy
there before cleanup. A journal above this store cannot preserve bytes that startup deletes,
and preserving all legacy `pending` directories would risk resurrecting a deliberate release.
The actor's `ReadJob::fail` path also aborts Keep work, including admission when permits are busy.
Separate transient suspension from an explicit durable cancellation before retaining partial work.

The storage owner must reconcile versioned state and journal/reservation ownership under the
exclusive lease, before admission or serving. Specify atomic ordering and crash recovery for:

| Durable state | Permitted recovery |
| --- | --- |
| Resumable transfer | Revalidate authority, exact manifest and durable bytes; obtain fresh runtime ownership before resuming. |
| Completed copy | Verify the committed result and retire stale transfer intent idempotently. |
| Cancelled or deleting copy | Continue bounded cleanup; never resume fetching or restore serving eligibility. |
| Damaged or inconsistent metadata | Retain conservative quota charges and expose blocked/manual recovery; do not infer ownership from a path. |

Treat legacy `pending` as cleanup-only because it cannot distinguish an interrupted fetch from
a release. Define new-format readers, migration barriers and old-client/downgrade behavior before
writing resumable state; unsupported readers must not destructively reinterpret that state.
An upper layer must not race mount cleanup. Incomplete bytes remain non-serving, and cleanup
failure cannot refund surviving storage or multiply reservations after restart.

For explicitly requested Keep/download work, persist the transfer intent, verified progress and
execution-budget state through that revised sealed storage contract. Resume only after current
membership, scope, exact manifest and disk state are revalidated. Distinguish the durable user
request from a bounded execution window: the proposed default on budget exhaustion is a durable
paused state requiring explicit user retry to grant a new bounded window. Restart, newly available
providers and route changes cannot replenish an exhausted budget. Persist the retry grant and
remaining budget conservatively before issuing work; interruption must not erase charged work.
Specify nonpersistable runtime-deadline handling so reopen cannot silently extend the allowance.
Transient capacity pressure may suspend unfinished work without discarding partial bytes, while
explicit cancellation/release takes the cleanup path. Partial pages and provider swaps cannot
reset execution bounds indefinitely. Add bounded whole-variant fallback without mixing encrypted
variants into a fictitious retained manifest.
Resuming an external download also revalidates its destination, current local write authority
and no-overwrite policy; a saved path is not permission to replace a newly created file.

Automatic redundant copies are a separate policy stage: define local opt-in/disclosed group
policy, quota, desired replica count, retention/expiry, consent revocation, offline deletion and
garbage-collection rules before enabling it. This plan preserves existing explicit Keep semantics.
Remote custody receipts must name exact content and retention scope after a successful durable
barrier. Never promise availability beyond surviving copies, keys and reachable eligible holders.

Exit: file reads and opted-in retained transfers survive provider failure, bridge changes and
restart; corrupt/removed providers, manifest replacement, full disk and cancellation fail safely.
Bulk work cannot monopolize chat/control/call setup. UI distinguishes metadata, partial bytes,
verified local copy, remote custody evidence and no currently usable provider.

Required crash/lifecycle regressions: reservation created before intent, intent committed before
reservation, chunk durable before progress, completed-copy rename before journal retirement,
and release rename before cleanup. Also exhaust the budget then restart with a healthy provider;
release old worker results after cancellation, release, remount or a replacement transfer for
the same file; and fail cleanup or corrupt metadata without gaining free quota. Test permit
exhaustion preserves a resumable reservation without admitting uncharged network work.

### Package 5A Establish voice media feasibility early

Room/signalling fixes do not complete this package. First prove the existing personal/server TURN
path under forced relay ICE and validate credentials, expiry, candidate selection and reconnect
in the native webview. TURN provides an ICE relay mechanism; it does not supply room discovery.
See [RFC 8656](https://www.rfc-editor.org/rfc/rfc8656.html) and
the [WebRTC specification](https://www.w3.org/TR/webrtc/).

Run this spike alongside Packages 0 and 2 using the existing TURN configuration, without adding
a new relay service or encryption claim. Record the shipping WebView2 version, selected relay
candidates, audible results, reconnect behavior and unavailable-path diagnostics. A failure is
evidence for the architecture decision, not permission to relax endpoint authentication.

Exit: the existing path's feasibility and limits are recorded separately from new protocol,
storage and relay acceptance. No automatic member-hosted relay is enabled by this experiment.

### Package 5B Decide and implement member media bridging

Write an architecture decision comparing a reachable consented member-hosted TURN service with
an encrypted media/datagram overlay over existing member links. Test feasibility on WebView2,
UDP-blocked networks and representative NATs. A member who bridges chat may be unable to expose
a TURN allocation; a loopback relay address is not remotely reachable. Account for TCP head-of-line
delay if evaluating media on reliable mesh streams. Do not select a new dependency before its
licence/advisory/build review and a latency/loss prototype.

Define server membership, active call participation and relay authorization as separate roles,
then trace which role can obtain or derive every key. The intended confidentiality boundary
excludes a relay-only server member from media keys. Prefer end-to-end DTLS-SRTP with transparent
relay semantics. Forwarded signalling must bind the original authenticated endpoint to its SDP
and DTLS certificate fingerprint; an authenticated helper cannot substitute its own certificate.

`ServerGroup::media_secret` derives the same key for all server members. That exporter key cannot
exclude a relay-only member, even if frames are encrypted. Any architecture terminating endpoint
encryption requires a separately implemented and reviewed participant-scoped frame-encryption
and key-distribution mechanism that excludes nonparticipating relays before making this claim.
Plaintext forwarding cannot satisfy it. MLS-derived key exposure alone is insufficient.
Specify future-media access after call leave separately from server removal: a room departure
need not advance the server's MLS epoch. Include participant removal, relay replacement and
rejoin; do not claim revocation of plaintext or keys already disclosed to a prior participant.

Bind relay authorization to the group, exact endpoints and live call; require explicit hosting
consent, bounded allocations/bandwidth and expiring credentials. Prevent an open proxy, private
network scanning and cross-group credential reuse. Specify relay health, failover, stop/leave
cleanup and an honest failure state if no usable relay exists. Do not record or persist audio.

Name the server-side enforcement point for application scope. Standard TURN permissions concern
IP addresses within allocations and ignore the CreatePermission port; they do not establish
group/device/call authorization. Credential naming alone cannot implement this policy
([RFC 8656 sections 9 and 10](https://www.rfc-editor.org/rfc/rfc8656.html#section-9)). Test a
valid authorized credential against unrelated destinations, cross-group use, allocation refresh
after call end and continued use after learned revocation. Existing third-party TURN support
does not establish these stronger guarantees for a new member-hosted service.

Define what each alternative route carries besides audio: the current peer connection also
supports authenticated instrument/jam and drawing channels. Specify supported companion traffic,
what remains unavailable and the UI behavior. An audio-only prototype cannot count as acceptance
for a regression in those channels, consent or their authenticated capabilities.

Exit: real clients on forced non-direct paths can hear each other. Give relay-only B its complete
legitimate server MLS state, relay credentials and all relay-visible traffic, but no active call
participation. The architecture/key-ownership trace must establish why these inputs do not yield
endpoint media keys; observing ciphertext alone is insufficient. Test call leave, participant
removal, relay replacement and rejoin against the specified future-media access policy. Relay
loss triggers bounded recovery through an eligible alternative. Measure end-to-end latency,
loss/jitter, bandwidth and resource ceilings. Keep this gated until physical-network and
audible-device evidence exists.

### Package 6 Combined acceptance and rollout

Extend the six-client fixtures across chat, files and voice signalling with shared topology
controls. Include partition/reunion, reversed restart order, source loss, duplicate/reordered
traffic, route changes, background call during server switching, lock/unlock, removal and load.
Check no forbidden direct edge is silently created. Keep actual audio acceptance separate from
mocked WebRTC or single-process transport evidence.

Add versioned wire/storage fixtures and mixed-version refusal/fallback behavior before exposing
new envelopes, receipts or transfer journals. Roll out independently: connection fixes, room
discovery/signalling, chat/file enhancements, then media relay. Disabling new recovery must retain
accepted work; it must not disable existing authorization or downgrade authenticated group mode.
Pin upgraded signalling mode at the endpoint-session boundary throughout rollout and rollback;
an enabled legacy handler cannot mutate that session when new routing is disabled.

Name these combined scenarios and exact oracles in the communication-recovery ledger:

| Scenario | Required oracle |
| --- | --- |
| A-B-C with A-C forbidden | The harness rejects forbidden connections and records that recovery used only allowed edges. |
| B relays without joining the room | A and C discover each other; B gains no participant tile, microphone or recording authority. |
| Independent joins by A and C | Stable room discovery converges while participant join incarnations remain distinct. |
| B disappears and D survives | Original identities and replay accounting survive; obsolete B routes retire without weakening authentication. |
| History/blob saturation | Control, current chat, call setup and another group meet agreed progress/latency limits. |
| A gives history to B, then both stop | C recovers from B's reopened durable state without a hidden live snapshot or test-only save. |
| Provider returns after transfer exhaustion and restart | The durable paused/manual-retry policy holds; restart supplies no new budget. |
| No usable media relay | Presence, signalling progress and unavailable audio remain distinct in native state and UI. |

Provide redacted diagnostics for route class, recovery stage, outstanding obligation age, retries,
provider failure, queue delay and media candidate class. Exclude message/file contents, keys,
TURN secrets, SDP, raw ICE/private addresses and stable cross-group tracking identifiers.

## Review and verification gates

Before changing protocol boundaries, obtain read-only design review of forwarding authority,
freshness, confidentiality, membership recovery, budgets, file retention and voice encryption.
After each implementation package, review the actual diff and surrounding lifecycle/enforcement
paths according to AGENTS.md. Resolve blocker/high findings and record lower findings; request
re-review when a fix materially changes the boundary.

Add deterministic unit regressions next to the existing Rust and TypeScript tests, then use
real TCP topology tests, independent process restart tests, and native physical-network/audio
acceptance for the claims those layers establish. Pin recovery deadlines and service-latency
targets after Package 0 measurements; do not treat a retry being scheduled as successful recovery.

For every code handoff run the required complete suites:

```sh
cargo test --all --all-features
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
npm --prefix apps/desktop test
```

For root Rust changes also run formatting, strict Clippy, `scripts/check-no-ambient.sh` and
`cargo deny check`. For native changes run its separate `cargo check`. For frontend changes run
`npm --prefix apps/desktop run check` and `npm --prefix apps/desktop run build`. Run `test:flows`
when the change can reach send/friend acceptance, and build/run `test:startup` after changes to
setup, runtime/thread placement or process spawning. Use the visual fixture for relevant UI
evidence and real native clients for WebView/media behavior.

Update architecture, interfaces, threat model, handover and per-feature evidence when their
contracts change. This planning-only addition does not require runtime suites; implementation
acceptance remains outstanding for every package above.

## Design review amendments

The 18 October external review identified three medium-priority plan amendments: retained-store
resume migration, media-key audience, and isolation of upgraded sessions from legacy ingress.
They are incorporated above as prerequisites and explicit regression criteria. The review also
supplied TURN authority, queued RTC effects, companion-channel and combined-topology acceptance
details, and separated early media feasibility from gated implementation. Package 0 and narrow
reproduction-backed fixes remain the starting point. These dispositions accept planning changes;
they do not close implementation, security review or native acceptance obligations.
