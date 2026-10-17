# Chat file sharing and voice reliability plan

Prepared 17 October 2026. Status: proposed work; no runtime changes or acceptance results.
Source baseline: `f3596ac859293293701b0e5d2dd0339bd7b1f5fe`, with pre-existing uncommitted
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

Use fresh expiring self-announcements and challenge/session evidence where current liveness is
claimed. A forwarded old heartbeat cannot renew presence merely because it arrived recently.
Bound duplicate suppression and forwarding, prevent loops and storm amplification, and retain
per-origin limits across helper/connection changes. Reconcile missing setup state; handle lost,
duplicate and reordered offers/answers/ICE with negotiation and call-generation fences.

Leave, lock, group removal, room switch and actor replacement revoke pending effects. An old
bye cannot dismiss a newer join. A peer's own signed state cannot authorize another member's
presence, microphone, playback or recording consent. Update native mocks and UI tests as needed.

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

For explicitly requested Keep/download work, persist a versioned transfer intent and verified
progress through existing sealed storage/reservations. Resume only after current membership,
scope, exact manifest and disk state are revalidated. Preserve total attempt/deadline bounds;
partial pages and provider swaps cannot reset them indefinitely. Add bounded whole-variant
fallback without mixing encrypted variants into a fictitious retained manifest.
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

### Package 5 Decide and implement voice media bridging

Room/signalling fixes do not complete this package. First prove the existing personal/server TURN
path under forced relay ICE and validate credentials, expiry, candidate selection and reconnect
in the native webview. TURN provides an ICE relay mechanism; it does not supply room discovery.
See [RFC 8656](https://www.rfc-editor.org/rfc/rfc8656.html) and
the [WebRTC specification](https://www.w3.org/TR/webrtc/).

Write an architecture decision comparing a reachable consented member-hosted TURN service with
an encrypted media/datagram overlay over existing member links. Test feasibility on WebView2,
UDP-blocked networks and representative NATs. A member who bridges chat may be unable to expose
a TURN allocation; a loopback relay address is not remotely reachable. Account for TCP head-of-line
delay if evaluating media on reliable mesh streams. Do not select a new dependency before its
licence/advisory/build review and a latency/loss prototype.

Prefer end-to-end DTLS-SRTP relay semantics for the first media experiment. Any design that
terminates peer encryption or forwards decoded audio requires a separately implemented and
reviewed frame-encryption design before claiming member-opaque media. MLS-derived key exposure
alone is insufficient. Specify membership revocation for already-open calls and call/epoch changes.

Bind relay authorization to the group, exact endpoints and live call; require explicit hosting
consent, bounded allocations/bandwidth and expiring credentials. Prevent an open proxy, private
network scanning and cross-group credential reuse. Specify relay health, failover, stop/leave
cleanup and an honest failure state if no usable relay exists. Do not record or persist audio.

Exit: real clients on forced non-direct paths can hear each other; a third member carries only
encrypted media under the selected design. Relay loss triggers bounded recovery through an
eligible alternative. Measure end-to-end latency, loss/jitter, bandwidth and resource ceilings.
Keep this gated until physical-network and audible-device evidence exists.

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
