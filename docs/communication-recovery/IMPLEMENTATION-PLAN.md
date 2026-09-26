# Mewtual: robust communication and recovery implementation plan

**Version:** 1.0  
**Prepared:** 26 September 2026  
**Repository:** `Thalpy/Mewtual`  
**Target branch:** `gate4-agent1-runtime`  
**Inspected baseline:** `48f906962b37ed71a967c04ef4978d019ff43586`  
**Status:** Proposed implementation and acceptance plan. Not an implementation PASS or a completed security review.

> **Core objective:** A member should not need to connect directly to every other member. The application should automatically use whichever permitted connections exist to recover authenticated membership state, exchange missing history in both directions, retain unfinished work, and make newly acquired history available to the next peer.

This plan incorporates the reconnect/reply-code problem, mesh history reconciliation, automatic bridges, overlap with flipnote epoch work, and a strict distinction between peer-to-peer and dedicated groups. It proposes implementation contracts and work packages, not a replacement for the existing replication system.

The branch head was checked during preparation. Selected production sources and the Gate 4 handoffs were inspected; a complete lower-level transport audit and runtime reproduction were **not** performed. Source observations are distinguished from proposed behaviour below. All implementation tests in this document are requirements, not claimed results.

## Start here

**For implementation:** read the decisions and invariants, then use section 14 as the work queue and section 15 as the acceptance checklist. Start with CR00–CR04: reproduce the failure, make persistence truthful, unify recovery scheduling, authenticate group mode and make successful reply-assisted admission restart-safe. Follow with two-way reconciliation and automatic bridges; dedicated enforcement is a separate, non-downgradable release boundary.

**For review:** sections 5–12 contain the proposed authority, storage, protocol and lifecycle contracts. Section 16 identifies the specific new boundaries that need independent design acceptance before exposure. The baseline observations are linked to exact source locations in section 18.

## Contents

1. [Decisions and scope](#1-decisions-and-scope)
2. [Baseline findings and existing building blocks](#2-baseline-findings-and-existing-building-blocks)
3. [Safety and progress invariants](#3-safety-and-progress-invariants)
4. [Architecture and flipnote reuse](#4-architecture-and-flipnote-reuse)
5. [Authenticated group mode and migration](#5-authenticated-group-mode-and-migration)
6. [Durable state and identity contracts](#6-durable-state-and-identity-contracts)
7. [Reconnection and lifecycle recovery](#7-reconnection-and-lifecycle-recovery)
8. [Bidirectional history reconciliation](#8-bidirectional-history-reconciliation)
9. [Automatic bridges and store-and-forward](#9-automatic-bridges-and-store-and-forward)
10. [Membership, key and document recovery](#10-membership-key-and-document-recovery)
11. [Durability, acknowledgements and retention](#11-durability-acknowledgements-and-retention)
12. [Resource limits and dedicated-mode enforcement](#12-resource-limits-and-dedicated-mode-enforcement)
13. [UI, diagnostics and operational behaviour](#13-ui-diagnostics-and-operational-behaviour)
14. [Implementation work packages](#14-implementation-work-packages)
15. [Acceptance and adversarial test matrix](#15-acceptance-and-adversarial-test-matrix)
16. [Integration, rollout and review process](#16-integration-rollout-and-review-process)
17. [Definition of done](#17-definition-of-done)
18. [Source references](#18-source-references)

---

## 1. Decisions and scope

### 1.1 Product decisions to implement

| Decision | Required consequence |
|---|---|
| P2P does not require a stable member, founder, coordinator or dedicated service. | Discover, forward, replicate and retry automatically through available members. Optional infrastructure may improve reachability but is not a correctness dependency. |
| Members should not configure bridges or exchange new codes during normal recovery. | Joining a P2P group establishes the disclosed, bounded member-assistance policy. Assistance is automatic while participating. |
| Indirect communication is a valid working connection. | Do not block messages or reconciliation while attempting a direct connection upgrade. |
| Histories can be incomparable. | Exchange differences in both directions and consult multiple sources. No highest-count or highest-hash winner. |
| Existing signed operations and Automerge documents remain canonical. | Reuse the signed catch-up path. Do not introduce another chat ledger or deduplicate by rendered text. |
| Flipnote mechanisms should be shared where their contracts fit. | Reuse verified discovery, persistence, bounded preparation, lifecycle fencing and pagination; retain document-specific authorization. |
| Ordinary P2P chat must not require an online document owner. | Do not attach chat to the owner-receipt lifecycle merely to reuse epoch code. |
| Dedicated mode is a different security contract. | Persist and authenticate the mode; enforce current admission at the designated service boundary; never silently downgrade to P2P. |
| A signature proves attribution, not freshness, completeness or durable custody. | Verify what each piece of evidence actually establishes; keep those states separate. |
| Work must survive ordinary interruptions. | Persist accepted operations and recoverable publication obligations; replace failed routes and sessions rather than discarding work. |

### 1.2 Delivery guarantee

For each authorized document, a durably accepted, admissible operation within the agreed retention scope should eventually reach eligible replicas when a usable **time-ordered sequence of exchanges** exists, the required content and verification material survive those exchanges, and the scheduler receives sufficient execution and resources.

This is not a promise that every member is simultaneously online, that arbitrary membership forks merge, or that data can be recovered after its last holder deletes it. A malicious bridge controlling every available path can withhold traffic. Complete isolation with no usable or discoverable contact cannot be repaired by guessing an address.

These limits must not become reasons to disable useful P2P forwarding. When temporarily isolated, preserve local work, report the limitation accurately, and continue bounded automatic recovery.

### 1.3 Explicit non-goals for the first release

Do not build a global consensus protocol, an unrestricted public proxy, a custom cryptographic transport, a second message store, a new mandatory rendezvous service, or a Merkle/IBLT optimization before the existing reconciliation path is correct. Do not add anonymous cross-group relaying by default. Do not change flipnote ownership, manual disposition, repair authority, native Save exposure or Gate 4 acceptance as a side effect.

Mode conversion after group creation is a separate migration feature. The first release supports authenticated mode selection and explicit legacy migration, not opportunistic runtime mode changes.

## 2. Baseline findings and existing building blocks

### 2.1 Source observations

| Observation at the inspected commit | Implementation implication | Evidence |
|---|---|---|
| `reconnect_policy_after_admission` authorizes the named inviter only for ordinary direct admission; reply and switchboard paths return `Disabled`. | Add a reviewed, common post-admission member relationship, rather than merely deleting the security condition. | [R1] |
| Authenticated route capture checks membership, endpoint grammar, peer binding and route bounds. | Preserve these checks and the distinction between route evidence and permission to dial. | [R1] |
| `ServerNet` retains per-group transport identity, port, peer-record sequence reservations, proven routes and a bounded pending recovery candidate. | Extend this model without regenerating identities or overwriting healthy routes during a failed attempt. | [R2] |
| The inspected persistence function logs snapshot/save failures and returns without a structured result; it correctly retires tickets only after a successful write. | Preserve ticket/incarnation fencing, but propagate outcomes to callers and retain retry obligations. Do not falsely describe an unsuccessful save as durable. | [R3] |
| `export_catchup_since` compares requester heads; `export_catchup_page` traverses the signed log with bounded output and a provider-specific continuation. | Build orchestration around the existing implementation. Test unknown heads, duplicate-only pages, interruption and provider changes. | [R4] |
| Catch-up exports retained signed operations re-sealed under the current epoch. | Separate operation identity from ciphertext identity and verify original authorship when an intermediary serves history. | [R4] |
| Registry and Studio discovery share handles/slots and verify mount/server provenance on completion. | Reuse the custody and provenance pattern; do not create a competing worker pool or injected trusted-selection path. | [R5] |
| Gate 4 handoffs explicitly divide runtime, lifecycle/tenure, repair and integration ownership. | Agree seams with those owners and preserve independent review boundaries. | [R6] |

The prior conversation also identified an eager peer-record exchange restricted to the direct-join finalization path, warm-unlock versus cold-reload differences, and potentially ambiguous catch-up outcomes. Reproduce and trace these at the implementation baseline before treating them as independently verified defects. Do not use earlier review prose as a substitute for inspecting the current call graph.

### 2.2 Evidence register required before implementation

Create `docs/communication-recovery/BASELINE.md` containing the actual checked-out SHA, current wire/storage versions, relevant feature flags, tested platforms, and a trace of:

- direct, reply and helper admission through durable finalization;
- send/receive through signing, authorization, persistence, publication and UI delivery;
- warm lock/unlock, logout/account teardown, cold restart and sleep/resume;
- control/key recovery, peer-record discovery and both directions of channel catch-up;
- reopened/unopened document service and history retention.

Classify each finding as **source-confirmed**, **reproduced**, **hypothesis**, or **existing invariant to preserve**. Record test commands and results. A green helper test is not a running-app reproduction.

## 3. Safety and progress invariants

These invariants are the review contract. New code must identify which ones it implements or relies on.

| ID | Invariant |
|---|---|
| I01 | Accepted local work is not silently lost, replaced by a remote snapshot, or deleted because transport failed. |
| I02 | A persisted or custody acknowledgement refers to content that crossed the specified durable write barrier. |
| I03 | One logical operation retains the same retry identity; at-least-once transmission has idempotent effects. |
| I04 | A route hint, successful socket, member authorization and history authority are distinct facts. |
| I05 | Only live authenticated evidence upgrades a candidate to a proven connection strategy; direction is preserved. |
| I06 | Known membership removals, policy pins and authoritative document transitions cannot be rolled back by stale advertisements. |
| I07 | Reconciliation is bidirectional and multi-source; equal counts, empty responses and numeric hash ordering never establish completion. |
| I08 | Newly acquired admissible history becomes eligible for onward service and triggers neighbour reconciliation. |
| I09 | A waiting recovery obligation has an owner, a wake-up condition and a bounded scheduled retry where appropriate. |
| I10 | A replacement session, mount, actor, policy or document generation cannot consume a stale completion. |
| I11 | P2P ordinary communication needs no permanent host and no online owner receipt for each chat operation. |
| I12 | Dedicated groups cannot be downgraded through failure, omitted fields, old clients or forwarded requests. |
| I13 | Network-triggered work is bounded in bytes, CPU, storage, fan-out and duration, including work retained after cancellation. |
| I14 | Forwarding does not transfer authorship, authorize key disclosure or promote unverified history. |
| I15 | Retention coverage, missing blobs, unresolved dependencies and unavailable authorization are explicit states, not false convergence. |
| I16 | Private drafts, recovery archives and unaccepted branches are not automatically advertised as shared history. |
| I17 | Chat integration does not bypass flipnote Prepared/source fences, tenure evidence, repair authority, source custody or manual lifecycle. |
| I18 | Group-scoped identities, descriptors and budgets do not create a new cross-group tracking or probing surface. |

## 4. Architecture and flipnote reuse

### 4.1 One coordinator, existing domain owners

Place orchestration behind the existing server actor. Prefer focused leaf modules over further expansion of central files.

```text
Native lifecycle and UI commands
              |
              v
Server actor: recovery coordinator
  |-- connectivity recovery and route selection
  |-- membership/control recovery adapter
  |-- document inventory and reconciliation scheduling
  |-- durable publication/custody tracking
              |
      Existing implementations
  catcoms-sync / catcoms-net / catcoms-discovery
  signed operations / Automerge / document-specific gates
  sealed store / epoch inventory / bounded preparation
```

These are logical responsibilities, not four independent services with competing retry loops. Audit and consolidate current timers before adding another one. Transport owns sockets; the actor owns scheduling and live authority; the store owns durable commits; document adapters own semantic admission. Native/UI code translates commands and truthful results but does not become the source of network state.

**Suggested new module names are proposals**, to be reconciled with current structure:

| Proposed location | Responsibility |
|---|---|
| `crates/catcoms-app/src/recovery/` | Coordinator, obligations, scheduling, document adapters, typed outcomes. |
| `crates/catcoms-sync/src/recovery/` | Bounded protocol sessions and integration with existing catch-up/control paths. |
| Existing discovery/net modules | Candidate evidence, reverse-connect requests, relay maintenance, shared dial budgets. |
| Existing store modules, with focused leaves | Versioned policy/recovery records and commit barriers; no parallel vault. |
| `apps/desktop/src-tauri/src/` | Lifecycle entry points, structured native results and UI snapshots. |
| `docs/communication-recovery/` | ADRs, baseline, schema, status and test evidence. |

Do not reserve numeric wire tags, choose a new crypto envelope, or add a public native command solely from the illustrative names in this document. Those are reviewed boundary changes.

### 4.2 Reuse matrix

| Flipnote/Registry mechanism | Reuse | Do not import accidentally |
|---|---|---|
| Verified head/checkpoint discovery | Provider evidence, selection freshness and source failover. | A peer hint as canonical authority. |
| Paginated source exchange | Bounded work and explicit continuation ownership. | Another provider's cursor or an obsolete source generation. |
| Capture / detached preparation / live commit | Heavy work outside exclusive custody; exact revalidation on return. | Worker-owned signing keys or an uncharged complete-source cache. |
| Shared preparation slots and inventory | Existing resource accounting, reference holds and generation checks. | A separate chat pool that multiplies process memory. |
| Pending intents and recovery-first transitions | Preserve obligations across crash/refusal. | Reinterpreting a flipnote local intent as a publishable chat operation. |
| Runtime/mount/session fencing | Prevent stale completion application and stale native delivery. | A single underspecified generation token for every identity. |
| Document lifecycle and signed repair | Consume verified outcomes when recovering that document type. | Making ordinary chat depend on owner-issued receipts or inventing a parallel repair path. |

The existing shared discovery handles demonstrate that this kind of reuse is already an architectural pattern. [R5]

### 4.3 Epochs and generations stay typed

Keep separate types for MLS membership/key epoch, document generation/checkpoint lineage, owner tenure, authenticated policy version, transport session, actor instance, vault mount, native account session and store inventory generation. Preserve current types where available.

The complete scope of a detached job must identify the actual account/mount, numeric server slot and group, document type/logical identity, document generation, policy/control basis, relevant owner tenure and originating request. Recheck precisely the dimensions that authorize its action, not a guessed subset. Source content stamps must describe the real bytes/metadata used, not only a rendered projection.

Reuse accepted Gate 4 seams; coordinate new cross-domain changes with the existing runtime, lifecycle/tenure, repair and integration owners. Their status documents may contain historical entries: read the latest applicable checkpoint before consuming an API. Design acceptance is not implementation acceptance. [R6], [R7]

## 5. Authenticated group mode and migration

### 5.1 Group mode is not a device role

Introduce an authenticated policy with two modes, conceptually `PeerToPeer` and `Dedicated`. An always-on member does not change a P2P group into a dedicated group. A relay does not automatically become an admission authority.

The policy binds:

- group identity and schema/version;
- immutable mode for this release;
- forwarding/discovery and history-service rules;
- dedicated authority identities and service requirements, where applicable;
- minimum required protocol capabilities and policy update authority;
- the history/retention policy reference.

Use the existing authenticated group-control model to authorize this record; obtain design review for any new authority requirement. Bind the policy digest into invitations and authenticated protocol negotiation, and persist the highest verified applicable policy state. Store UI projections separately from the authoritative record.

Do not trust a caller-supplied mode, a provider's role claim, an arbitrary larger version, or the presence/absence of configured relay addresses. Policies from incompatible control branches need explicit control recovery, not last-write-wins selection.

### 5.2 Behaviour by mode

| Behaviour | Peer-to-peer | Dedicated |
|---|---|---|
| Member assistance | Automatic, bounded, group-scoped forwarding and replication. | Only service paths explicitly authorized by the dedicated policy. |
| Normal recovery contact | Any useful reachable member; no inviter dependency. | Current admission through the designated service boundary. |
| Direct edges | Optional optimization. | Limited by service policy; do not expose member endpoints by default. |
| Stale membership knowledge | Permit bounded recovery based on locally verified evidence; acknowledge freshness limits. | Require current service admission before ordinary history/inventory access. |
| Infrastructure unavailable | Retain work and continue permissible mesh recovery. | Retain work; no automatic P2P downgrade. |
| Ordinary chat authority | No online owner requirement. | Follow the dedicated policy without granting the service message authorship. |
| Revocation | Enforced as verified evidence reaches each peer. | Enforced against current service state at relevant boundaries. |

Dedicated mode need not give the service plaintext keys. However, a server that is merely an optional encrypted relay cannot enforce stronger access rules while unrestricted peer-history paths remain enabled.

### 5.3 Legacy data and mixed versions

Persist a distinction between **authenticated mode**, **unresolved legacy policy**, and **unsupported policy**. Missing policy must not silently mean P2P.

For a legacy group, preserve existing readable local content and existing supported behaviour while presenting its policy as unverified. Do not enable the new automatic forwarding surface or claim dedicated guarantees until a reviewed upgrade is authenticated through the group's actual existing governance. A temporarily offline owner need not block routine operation of an already authenticated P2P group; legacy policy establishment is a separate one-time boundary.

New dedicated groups require capable clients at admission. Old clients that cannot enforce dedicated restrictions cannot participate through a compatibility fallback that restores unrestricted P2P service. Unknown mandatory fields/capabilities fail closed for the affected operation, without destroying local work.

Migration must preserve existing transport seeds, device identity, message IDs, receipt evidence and pending work. Write schema fixtures for every currently supported store version. Back up or retain the last valid sealed representation until an upgrade is durably committed. Interrupted upgrades must reopen consistently, not as an accidental different mode.

## 6. Durable state and identity contracts

Use existing types and storage where possible. The following are logical records, not final Rust definitions.

### 6.1 `RecoveryObligation`

Key an obligation by group and document/scope plus purpose: discover membership, reconcile history, publish accepted work, refresh route evidence or obtain a referenced blob. Store its durable basis, requested coverage, retained operation identifiers, latest meaningful progress, and terminal/manual-action reason where applicable.

Do not persist live socket handles, worker handles, an `in_flight = true` flag that survives forever, or another process's timer instant. On reopen, reconstruct resumable obligations from durable content and policy. Provider-bound cursors are disposable unless their validity can be freshly established.

Coalesce obligations by exact scope. A new local or remote change updates the target generation; it must not disappear behind an older completed request.

### 6.2 `PeerReachabilityEvidence`

Retain the authenticated member-to-transport binding, exact signed descriptor identity, permitted strategy, connection direction, scope, freshness/expiry, last verified success and classified failures. Direct, reverse-initiation and relay strategies are distinct.

Recheck membership/uniqueness, canonical terminal peer binding, address policy and shared dial budget at use time. Record learned-from provenance separately from target-signed evidence. A third party's assertion is not target authentication. Keep private/LAN observations local unless an existing explicitly authorized path permits their disclosure.

Never cache an inbound ephemeral source port as a proven listening port. Never replace a proven route with the address that happened to be attempted before a different path succeeded. Extend the bounded model rather than retaining every address forever. The existing `ServerNet` identity/sequence and candidate-promotion protections are starting points. [R1], [R2]

### 6.3 `DocumentSummary`

A signed summary identifies the protocol version, group/policy, document identity/generation, relevant control basis, sorted canonical durable heads, coverage/checkpoint information and sender session/sequence. It summarizes **dependency-complete, durably retained state**, not merely received hashes or visible text.

For compatible scopes, compute an equality digest over canonical metadata and heads using the existing approved hash primitives and a distinct domain separator. The digest is a cheap change detector; the heads/dependency graph and bounded inventory requests perform actual difference discovery. A fixed-size ordinary hash cannot reveal an unbounded missing-message set.

Keep presence outside the replicated chat ledger. Cache summaries when committed state changes rather than rehashing the entire archive on every heartbeat. Protect summaries inside the appropriate authenticated/encrypted group or recovery session; do not publish public chat-history fingerprints in address discovery.

### 6.4 `ReplicationReceipt`

A receipt binds an authenticated device, group/document, exact operation IDs or a precisely scoped frontier/manifest, acknowledgement class, retention coverage and request/session context. A receipt is the signer's claim; it does not prove an honest disk or permanent retention.

Distinguish received, durably stored, endpoint-delivered and read. The publication loop may stop aggressive retransmission after its configured custody goal, but ordinary anti-entropy must continue serving retained operations. Never delete the final local copy merely because one peer signed a custody claim.

### 6.5 Runtime outcomes

Provide typed results such as `Progress`, `InSyncWithTarget`, `NoUsablePeer`, `NeedsControlEvidence`, `NeedsAuthorizedRejoin`, `StorageBlocked`, `RateLimitedUntil`, `CoverageUnavailable`, `UnsupportedPolicy`, `CancelledStaleContext` and `PermanentInvalidInput`.

A zero operation count is not a result class. Map these into native/UI states without turning every condition into a generic connection failure.

## 7. Reconnection and lifecycle recovery

### 7.1 Model separate progress dimensions

Avoid a single `Connected` boolean or a single sequential state machine that stalls all work behind one unavailable peer. Track connectivity, control readiness, per-document reconciliation and pending publication separately.

```text
Connectivity: no path -> discovering -> indirect/direct path -> retrying
Control:      checking -> compatible | evidence needed | rejoin needed | removed
Document:     local -> inventory -> reconciling -> target satisfied | unavailable
Publication:  durable pending -> offered -> custody acknowledged -> endpoint delivered
```

A document can be locally readable while network recovery is incomplete. An indirect path can be useful while direct dialing fails. A missing blob in one channel must not block control messages or unrelated chat.

### 7.2 Idempotent recovery entry point

Add or consolidate an actor-owned `ensure_recovery(scope, reason)` operation. It coalesces work and schedules the next bounded turn. It does not spawn an unbounded task per event.

Trigger it on authenticated member connection, successful admission, cold reload, warm unlock, network/interface change, relay loss/expiry, newly accepted descriptor, relevant control advancement, newly committed history, service capacity becoming available and periodic repair ticks.

Each scheduled obligation must declare its next event and/or deadline. Inbound request service and completion handling must remain runnable while outbound requests are pending. Do not wait for a network response while holding exclusive actor/vault custody. Cancellation leaves the durable obligation pending and releases resources only when the worker/result actually relinquishes them.

Use deterministic clock/RNG injection for tests. Use monotonic time for running retry deadlines and checked signed wall-time limits for credentials. Reconstruct deadlines conservatively on restart; a clock rollback must not grant unlimited credential lifetime or a retry storm.

### 7.3 Lifecycle rules

| Lifecycle event | Required behaviour |
|---|---|
| Warm UI lock/unlock | Preserve the existing approved lock semantics. Reuse healthy actors where that is the current contract; invalidate UI deliveries, reconcile on unlock and rebuild UI snapshots. Do not implicitly expand networking or source service while locked. |
| True logout/account switch/unmount | Cancel and fence old-account work, revoke services according to existing policy and reconstruct obligations only in the appropriate later mount. Never send under the next account's context. |
| Cold process restart | Restore identity/policy/durable state, reserve and seal fresh sequence space before publication, rebuild transports, renew infrastructure state and schedule recovery. |
| Sleep/resume or network change | Revalidate connections and local interface generation; expedite one bounded recovery pass, then return to backoff. |
| Actor/task failure | Surface the failure. Restart only through a reviewed restore boundary with durable state, not by silently reusing a potentially inconsistent live object. |
| Clean shutdown | Attempt bounded persistence and stop work cleanly; do not rely on shutdown succeeding for correctness. |

Audit warm unlock and cold restore as separate production paths. An ordinary unlock should neither duplicate healthy actors nor depend on a future chat message to restart recovery.

### 7.4 Common admission finalization

After direct, reply-assisted or authorized helper admission:

1. Verify the final member-to-transport identity binding and current applicable group policy.
2. Exchange permitted signed reachability/member records on the already working path.
3. Establish continuing reconnect permission under the P2P or dedicated policy, separately from the temporary invite/reply authority.
4. Retain the successful strategy and direction, with endpoint proof and validity limits. Keep other proven strategies.
5. Durably commit membership/reconnect state, or retain a crash-recoverable finalization obligation if the store layout requires multiple writes.
6. Report membership success and restart-safety accurately; schedule control/inventory/history exchange.

No additional per-connection prompt is required for automatic in-group assistance already disclosed by P2P mode. A temporary helper credential does not become an unrestricted standing relay permission.

If membership commits before descriptor exchange finishes, restart must resume finalization. It must not require repeating admission or manufacture another membership. Keep established communication working after a local persistence error, but warn that restart recovery is not yet safely recorded.

Test each participant restarting immediately, in both orders, with the original failed direction still blocked. This is the first end-to-end regression to land.

### 7.5 Recovery strategy and route replacement

Try permitted strategies using evidence and fairness, not an inflexible ladder that waits on every failed socket:

- Reuse existing authenticated paths first, including indirect paths.
- Try bounded previously proven direct candidates and fresh signed descriptors through the common scheduler.
- Ask connected members for newer target records or forward a reverse-connect request.
- Renew/use permitted relay routes and perform direct upgrades only without sacrificing a working path.
- Use permitted local discovery or already configured discovery services where present; no mandatory stable service for P2P.

Reverse-connect requests identify the target and the exact accepted descriptor/version or digest. They must not carry arbitrary caller-supplied sockets into a new dialing API. Preserve current-topic/control-label checks, member binding, replay high-water and shared destination budgets. Verify the existing recovery-trigger design before assigning new tags or limits.

Tie failures to the candidate and evidence version. A fresh valid descriptor can reopen a previously stale candidate; changing an untrusted sequence must not reset aggregate abuse budgets. Retain proven fallback routes when refresh yields no usable observation. Clear or demote authority-dependent candidates on removal, descriptor replacement, policy changes and expiry.

When a member connection can carry history, use it even if it cannot supply a better direct route. Optimize for useful information flow, not a fully connected graph.

## 8. Bidirectional history reconciliation

### 8.1 Summary and inventory protocol

Use authenticated summaries as an inexpensive trigger. Do not order hashes or counts. Two replicas can have the same count and entirely different missing operations.

```text
A: {a, b, d}        B: {a, c, d}
A requests c.      B requests b.
Both preserve a, b, c, d, subject to operation admission rules.
```

Reconcile control/policy evidence and a **document directory** as well as known channels. A client must discover channels created while it was offline. Do not assume the Studio Registry already indexes ordinary chat channels; verify and extend the appropriate existing channel inventory through an adapter.

Directory exchange is scoped to authorized visibility. Knowing a group exists does not entitle a requester to its complete channel list. Group-level roots may identify differing buckets; detailed inventories remain paginated and access controlled.

For each compatible document, exchange durable heads and retention/lineage information. Matching heads indicate matching change-graph state only under the required completeness checks; they do not by themselves prove that signed source envelopes, checkpoint evidence or attachment bytes are retained. Compare those inventories separately where needed.

Wide frontiers must be paginated or conservatively represented as incomplete. Never silently truncate and call the summary complete. The existing provider-log pager is a fallback that progresses despite incomplete requester heads. [R4]

### 8.2 Pairwise exchange algorithm

For each selected neighbour/document pair:

1. Authenticate scope, policy, requester/provider identity and required control context.
2. Capture both sides' target summaries. Negotiate supported formats and bounded budgets.
3. Use existing heads-based export and paginated signed-operation transfer for A-to-B.
4. Schedule the complementary B-to-A exchange independently; one successful pull does not satisfy both directions.
5. For each page, check request/provider/source-generation binding, size and operation identity; verify author evidence and document semantics through the appropriate adapter.
6. Resolve causal dependencies or retain a bounded explicit dependency gap. Do not advertise dependency-incomplete heads as committed state.
7. Commit accepted operations and relevant recovery progress before sending a durable acknowledgement.
8. Continue while the provider-specific position or verified target coverage advances. Duplicate-only or empty pages may still advance a valid cursor.
9. Verify the captured target is satisfied. Exchange a fresh summary and coalesce additional work caused by concurrent writes.
10. Mark the pair reconciled with that target, not globally complete. Notify other neighbours about newly committed information.

Use a captured upper bound or equivalent immutable source target so a busy writer cannot make an individual pass chase an infinite tail. On compaction or source replacement, invalidate its cursor and negotiate again. Retain already acquired operations; do not start by clearing the local document.

The baseline `export_catchup_page` bounds emitted bytes but can scan already-held history while advancing. Audit **CPU/scanned-operation bounds** as well as output bounds, including repeated closure calculation. A small reply must not conceal an unbounded actor-blocking scan. Also audit single-operation oversize handling: make permanent oversize refusal explicit or use an already approved large-object transfer, rather than repeatedly requesting an impossible page. [R4]

### 8.3 Completion and progress rules

A pass succeeds only when its compatible captured target is verified and durably available, or a documented coverage/checkpoint alternative satisfies the request. Local unique history must remain retained and eligible for the reverse pass.

An empty page with no meaningful cursor advance and an unsatisfied target is not success. Neither are timeout, connection establishment, an optimistic Bloom-filter match, or a peer's unsupported claim to have everything.

If a peer repeats no-progress responses, suspend that attempt within a bounded policy, select another source and retain the unmet obligation. A legitimate busy peer can give a bounded retry hint; cap untrusted retry times so it cannot suppress all other sources indefinitely.

A same-size or equal-head state must still refresh when relevant control evidence, coverage, pending acknowledgements or document inventory changes. Epoch/checkpoint incompatibility goes to the appropriate recovery adapter, not ordinary union logic.

### 8.4 Multi-peer fairness

Track reconciliation by group, document, provider and compatible source generation. Prefer likely useful sources, but periodically consult alternatives rather than permanently choosing the fastest peer.

Use bounded per-document and global concurrency, round-robin or deficit-based fairness, and jittered periodic repair. Consult peers with different reachable neighbourhoods where known without collecting unnecessary global topology.

If A and B reconcile, then B learns new history from C, B's local revision change must schedule A-B again. This trigger is essential for transitive convergence. Simulate rings, lines, stars, changing bridges and partitions; do not accept pairwise-only tests as whole-mesh evidence.

### 8.5 Hash/sketch optimization boundary

The first implementation uses existing frontiers and pagination. A checksum detects difference, not its direction. No IBLT or Merkle replacement is required for initial correctness.

Later optimization may add content-addressed inventory buckets or a set-reconciliation sketch only with measured benefit, version negotiation, bounded decoding, explicit failure handling and a correct paginated fallback. Such a sketch discovers IDs; it does not replace operation fetching, signatures, authorization, dependency recovery or durable storage.

Automerge also offers a native pairwise sync protocol, but its documented reliable/ordered channel assumptions do not justify replacing Mewtual's signed-operation envelope with raw sync traffic. Any such change needs a separate compatibility/security review. [E2]

## 9. Automatic bridges and store-and-forward

### 9.1 Three roles, no permanent bridge identity

| Role | Carries or retains | Does not imply |
|---|---|---|
| Member discovery helper | Scoped signed reachability evidence and reverse-connect requests. | Permission to trigger arbitrary outbound probes. |
| Live transport/application forwarder | Opaque authorized traffic across existing links. | Permission to decrypt private documents or mint authorship. |
| Authorized history replica | Original signed operations plus required verification material and declared retained blobs. | A guarantee of permanent storage or authority over the document. |

A device can perform several roles and change roles as conditions change. P2P membership enables disclosed bounded assistance, not public relay service for strangers.

### 9.2 First implementation path

For ordinary shared group chat, make neighbour replication the correctness path: A exchanges authenticated operations with B; B retains and serves them to C. Use existing gossip for prompt delivery where appropriate, followed by anti-entropy for repair. Verify the existing gossip forwarding and duplicate behaviour rather than layering a second unconditional rebroadcast loop on top.

For recovery/control requests whose target is not a direct neighbour, add or extend a bounded group-scoped forwarding envelope over **existing authenticated connections**. The original end-to-end request identifies its issuer, target/scope, request identity, expiry, response budget, policy/control reference and approved message class. Endpoints authenticate and authorize the contents; an intermediary is not required to decrypt chat history or establish the newest membership state merely to carry a permitted opaque request.

Do not assume libp2p Circuit Relay supplies arbitrary multi-hop application routing. Its reservation protocol explicitly restricts accepting reservations over already relayed connections. Use it for supported live circuits; use bounded application forwarding and neighbour replication for longer chains. [E3]

Preserve existing end-to-end cryptographic envelopes. If the current protocol cannot safely address opaque recovery responses to a stale-key endpoint, define that specific envelope and key binding for review before enabling it. Do not improvise raw-key export, plaintext recovery or a custom unaudited tunnel.

### 9.3 Forwarding mechanics

Bind immutable request identity/scope and maximum lifetime into the origin-authenticated envelope. Intermediate routing metadata carries bounded hop state and a short-lived reverse path; every hop enforces its own budget. Duplicate suppression keys on the original authenticated request, not just the most recent neighbour's packet ID.

Use expanding but capped forwarding breadth, per-origin and aggregate quotas, deadlines and bounded response sizes. A hop count alone is not a malicious-loop defense: an attacker can tamper with mutable routing fields, so receiver-side deduplication, expiry and aggregate budgets remain mandatory.

Do not broadcast full histories to find a provider. Forward a small scoped interest, obtain a useful provider/evidence response and perform bounded transfers. Avoid revealing private inventories to all intermediaries. Reverse-connect hints must still enter the shared dial scheduler at the eventual dialing endpoint.

Request state need not survive a bridge restart. Accepted history and the end user's outstanding obligation do. If a response path disappears, retry with a new session/provider while preserving already committed operations.

### 9.4 Unopened channels and asynchronous contacts

A device is not a useful history bridge merely because its UI knows a channel exists. Separate service/retention eligibility from the currently open tab.

Within disclosed P2P storage/resource policy, retain and serve authorized subscribed channel histories even when unopened. Advertise actual coverage and blob possession. Use existing unopened-source and inventory patterns without fabricating a UI watch. [R5]

Test a temporal path: A sends to B; B commits; A exits; B restarts; C later connects. C must recover the admissible retained history from B without A. Repeat with several successive bridges and with the final recipient returning after key-state changes.

Opaque ciphertext storage is a separate capability from decrypting history replication. A holder without decryption authority can retain/forward only appropriately protected artifacts; it cannot re-seal inaccessible content into a new epoch. The recovery guarantee therefore includes availability of the necessary authorized source/evidence, not merely some ciphertext somewhere.

### 9.5 Bridge loss and malicious participants

Disconnecting a bridge should invalidate its live routes and requests, not group membership or local history. Try other neighbours fairly. Do not classify ordinary network failures as permanent misconduct.

An authorized malicious replica can withhold content or lie about custody. Validate all supplied data and seek independent sources where available. A device that already knows content can copy it elsewhere; neither P2P nor dedicated mode can revoke already learned plaintext.

Do not make a dishonest majority a substitute for verification. Multiple agreeing providers are useful availability evidence, not authority to bypass signed document/control rules.

## 10. Membership, key and document recovery

### 10.1 Recovery order without circular dependencies

Provide a narrow authenticated recovery path that can carry admissible control evidence before the endpoint can decrypt current application traffic. Separate:

```text
Proof of device identity / permitted historical relationship
                |
        Bounded control recovery
                |
Verified compatible membership and key state, or authorized rejoin
                |
       Document discovery and history
```

A transport handshake alone does not authorize current keys. A historical credential can justify a bounded P2P recovery attempt under local policy, not unconditional current document access. Dedicated mode additionally requires its current service admission contract.

Return typed outcomes for incremental catch-up, unavailable control evidence, authorized rejoin needed, verified removal and incompatible/forked control state. The UI must not loop indefinitely on a generic reconnect message.

### 10.2 Membership forks and old author evidence

MLS commits are not CRDT chat changes. Authenticate control-state ancestry and use the application's reviewed sequencing/conflict mechanism, rather than choosing the largest epoch integer or importing a peer's complete current secret state. RFC 9420 explicitly leaves conflicting commit resolution to the application. [E1]

Record the exact existing policy for admitting historical operations from an author who is no longer a member. A valid signature plus a claimed old timestamp does **not** prove an operation existed before removal. Nor does a malicious current member re-encrypting it prove its original admission.

Reuse verifiable historical admission/receipt/causal-control evidence where the application has it. Distinguish re-serving previously verified retained history from admitting a newly presented operation under obsolete authority. If required evidence is missing, keep the candidate out of canonical state and seek evidence through a bounded path; preserve accepted local work for its existing manual recovery route.

This boundary is a mandatory design/audit gate, not an invitation to invent owner receipts for every chat message. Where the P2P policy accepts locally valid state during a partition, document the resulting freshness limitation explicitly. Do not claim immediate global revocation or malicious-fork convergence without a mechanism that provides it.

### 10.3 Long absences, rejoin and unavailable keys

Try compatible incremental control recovery first. If required state no longer exists, use the existing authorized rejoin mechanism or implement its missing bounded integration after design review. A checkpoint can restore document content but cannot manufacture missing MLS secrets.

Preserve original signed operation identity when an authorized source re-seals retained history under a usable current epoch. This is already supported by document catch-up. [R4] Do not distribute obsolete group secrets to make recovery convenient.

Removed-and-readded members/devices must not inherit an earlier tenure's publication authority. Unpublished local work stays visible and recoverable, but is not automatically re-signed or republished into a different authorization context.

On restored backups or cloned stores, detect what can be detected and require a safe current-session/key recovery path before fresh signing/publication where necessary. Do not promise perfect rollback detection from purely local restored data. Prevent reuse of cryptographic sending state and identity sequence space through the actual persistence/rejoin design.

### 10.4 Flipnote recovery remains document-specific

For Studio/Registry, consume existing typed checkpoint, tenure and repair results. A peer serving already-issued evidence need not be its issuer, but serving evidence cannot extend what it proves. Preserve `Unknown`/provisional states until the required evidence arrives.

Do not enable native Save, reinterpret a Closing overlay, dispose of a DraftArchive, or choose a repair winner from chat synchronization. Retain recovery-first source replacement and exact original-envelope identity. A logical match or same rendered projection is not permission to discard a branch. [R6]

## 11. Durability, acknowledgements and retention

### 11.1 Sending transaction

Use one stable caller retry token bound to account/group/document and payload identity. A repeated token with a different payload is an explicit conflict, not a second operation.

Prepare and authorize the operation using existing signing custody. Commit the operation, the necessary MLS/authoring state and its publication obligation atomically, or through an existing reviewed crash-recoverable protocol that has equivalent recovery behaviour. Only then report `SavedLocally` and publish.

Do not put a signed operation on the network and only later attempt the first durable write of its sender state. Such ordering can produce vanished local messages or unsafe retry state after a crash. Do not silently roll a signing ratchet backwards on failure.

If an in-memory operation exists but persistence fails, retain it while possible and return a precise non-durable outcome. Preserve its retry identity. Never tell the user to blindly send a second copy. Once a durable obligation exists, reload reconstructs its publication even if shutdown never ran.

### 11.2 Receiving transaction

Validate scope, operation identity, signatures, membership/document semantics and resource admission before applying. Commit accepted operations, dependency status, relevant key/control state and onward-service eligibility consistently.

Only acknowledge **durable custody** after the corresponding commit. An early transport acknowledgement, if retained, must be labelled as receipt only. If the durable acknowledgement is lost, the sender retries the same ID and the receiver returns the existing durable result without another semantic application.

Coalesce disk writes using existing snapshot tickets where safe, but preserve the rule that a caller's success covers a snapshot taken after its change. Change `persist_server` and its caller contracts to surface success/failure/staleness; preserve existing incarnation checks. [R3]

### 11.3 Cross-file consistency

The current store separates server snapshots and network records. [R2] Identify which transitions require coordinated durability: admission, mode pinning, identity sequence reservation, control/key updates and publication acceptance.

Use existing store transaction patterns or a small reviewed journal with explicit prepared/committed recovery rules; do not describe unrelated atomic renames as an atomic multi-file transaction. Test crash after every barrier, including data flush, rename and directory durability where supported. Persisted pending finalization must make partially completed admission recoverable without repeating membership.

### 11.4 Retention, compaction and attachments

Separate current document state, full operation history, required verification/control material, tombstones, attachment content and private/local recovery archives. Each needs a declared retention rule.

For the first release, do not introduce history-destructive chat compaction as an implicit recovery optimization. Use current retention boundaries, expose them, and add a separately reviewed compaction protocol if required. A checkpoint representing the current projection alone is not necessarily an archive of deleted/edited message history or original author evidence.

A partial replica advertises `SinceCheckpoint`, explicit available ranges/buckets, or another exact bounded coverage description. It may report `Unavailable` for missing older content without inducing an endless retry loop. Changing coverage invalidates affected cursors and replica claims.

Preserve deletion semantics and the evidence required to prevent stale replicas resurrecting deleted state. Do not delete the last local accepted branch due to an export, acknowledgement, pressure eviction or membership change unless the existing authorized lifecycle permits it.

Track attachment/PIX references separately. History can be text-complete while a blob is pending; communicate that distinction. Reuse existing holds, inventory and cleanup admission rather than bypassing blob protection from the chat bridge path.

## 12. Resource limits and dedicated-mode enforcement

### 12.1 One budget boundary

All direct, discovered, reverse-request, relay-assisted and manually initiated dials must use the existing shared endpoint scheduler. All heavy preparation must use the accepted bounded shared capacity. A forwarded request does not get a new independent allowance.

Charge work to authenticated origin where available, immediate connection, group, destination/address prefix and device/service aggregate. Account for CPU and scans, not just payload bytes; queued, running and ready results all consume resources. Small signed requests can still be expensive.

Do not weaken existing caps merely to fit a new protocol. R0 must inventory actual current limits. Additive defaults below are test-profile proposals, not calibrated production facts or permission to exceed an existing stricter bound.

| Proposed initial behaviour | Constraint |
|---|---|
| One active reconciliation session per document/provider pair. | Coalesce new targets instead of spawning duplicates. |
| Rotate across providers with bounded global/per-group concurrency. | Existing shared pool permits remain the hard resource limit. |
| One automatic target dial decision per group recovery pass, with a small validated candidate set. | Respect stricter existing candidate/endpoint caps and aggregate scheduler grants. |
| Capped jittered backoff with periodic repair. | Test profiles use injected time; production values require failure/latency measurements. |
| Bounded summary, inventory page and forwarding-interest formats. | Negotiate downward; unknown/oversize input fails before expensive parsing. |
| Finite request lifetime and duplicate cache. | Expiry, per-origin and aggregate budgets still bound replay after cache eviction. |
| Fair service classes for control, current chat and backlog/blob recovery. | Reserve progress for each; neither a backlog nor valid control flooding monopolizes service. |

Before production exposure, publish a checked-in limits table with actual bytes, request rates, concurrent work, retry/expiry windows, retained memory, store quotas and accounting keys. Tests must consume those constants, not copied magic values. Existing reviewed recovery-trigger limits remain in force until an explicit reviewed change.

### 12.2 Dedicated admission

Unauthenticated callers receive only a minimal bounded admission exchange. They must not get channel inventories, history summaries, member endpoints or arbitrary recovery work. Avoid distinguishing sensitive membership/history states in public refusal responses.

Require current device-bound authorization before ordinary service. Capabilities or sessions must bind group/policy, requester key, allowed request classes, limits and expiry; the receiver checks current revocation, not just an unexpired signature. Recheck revocation/policy at safe boundaries during long transfers, and stop newly forbidden data when the service learns a removal.

Forwarded dedicated requests preserve the original requester and undergo the same checks. An admitted helper cannot launder an unauthenticated origin into service or trigger arbitrary outbound scans. Dedicated policy disables member endpoint distribution and P2P history fallback unless a specific reviewed service path authorizes them.

A malicious client can continue sending packets. The guarantee is that sustained probing yields no additional protected service or unbounded resource consumption, not that a public listener becomes impossible to contact.

### 12.3 Sustained abuse and service failover

Persist bounded security-critical revocation/replay state and longer-horizon abuse accounting needed for the dedicated contract. Do not let service restart, a different helper, request-ID rotation or self-generated identity reset all effective limits. Anonymous accounting must be bounded and privacy-conscious; avoid an unlimited database of failed identities.

Multiple dedicated instances need coordinated authoritative revocation and the relevant aggregate enforcement. Per-instance limits alone are not a service-wide guarantee. Define the safe behaviour when that state is unavailable: refuse the affected sensitive service rather than silently reverting to historical credentials. Preserve authenticated authority rotation; never trust an arbitrary replacement server because the original is down.

Do not globally punish an entire shared network for one client's behaviour without proportionate limits and recovery. Do not permit remote peers to create permanent bans on innocent targets through forged failures.

### 12.4 P2P freshness tradeoff

P2P peers can enforce only the control evidence they have received. Bounded assistance under a locally verified older membership view is an explicit availability choice. Once newer valid evidence is known, do not roll it back or downgrade encryption to satisfy a stale peer.

This tradeoff does not remove signature, scope, canonical-address, replay, resource or document-semantic checks. Transporting an opaque request and releasing current content remain separate decisions.

## 13. UI, diagnostics and operational behaviour

### 13.1 Mode and state presentation

Show an authenticated mode badge in group settings and join preview. A legacy/unverified mode is visibly different. Use explanatory text along these lines:

> **Peer-to-peer:** Members' devices help deliver and retain messages while participating. Availability and the spread of membership changes depend on which members can communicate.

> **Dedicated:** Designated services control access and recovery. Service outages can interrupt communication; this group does not switch to peer-to-peer automatically.

Expose local availability, useful network path, history recovery and publication separately. Suggested states include `Saved locally`, `Waiting for a connection`, `Connected through members`, `Recovering history`, `Reconciled with available peers`, `Older history unavailable`, `Storage needs attention` and `Authorized rejoin required`.

Avoid displaying one peer's durable storage acknowledgement as recipient delivery or read status. Do not show an old forwarded heartbeat as proof that its author is currently online. Session/challenge freshness belongs to presence, not the chat operation ledger.

On unlock/remount, obtain authoritative native snapshots and reconcile event gaps. Do not rely on replaying every suppressed UI event, and do not let an old response repopulate the next account/session.

### 13.2 Safe diagnostics

Record structured events for recovery scheduling, strategy/direction, authentication stage, selected evidence version, retry classification, page/cursor progress, missing dependencies, durable commit, coverage refusal and policy enforcement.

Include local diagnostic correlation IDs, server/actor incarnation and redacted group/document references as supported by the existing safe logger. Do not add raw private addresses, group keys, message bodies, recipient lists or stable cross-group tracking IDs. Normal users should see a useful explanation without understanding a topology diagram.

Required metrics include pending durable work, age of oldest unmet obligation, retry/no-progress counts, useful peer count, recovered operations/bytes, store failures, retained/queued memory and control/chat service latency under backlog. A zero count is not automatically a healthy state when discovery or admission is blocked.

## 14. Implementation work packages

Use the `CR` prefix for this programme. These checkpoints are **not** replacements for the existing Gate 4 gates or agent numbers. Each package lands with implementation, focused tests, evidence and documented limits; new authority/wire/persistence contracts require the existing independent design-review process before implementation. Shared test IDs may have package-specific unit/contract variants; their complete production integration case is required at CR11, not assumed passed by an earlier helper test.

### 14.1 Dependency and ownership map

| Package | Deliverable | Depends on | Primary implementation area |
|---|---|---|---|
| CR00 | Baseline, regression harness, seam/limit inventory and ADRs | None | Cross-cutting evidence and integration |
| CR01 | Authenticated mode, capability negotiation and migration | CR00 design approval | Group control, wire, store, native projection |
| CR02 | Truthful persistence and durable publication obligations | CR00; approved persistence contract | Actor, store, native send/receive |
| CR03 | Actor-owned recovery scheduling and lifecycle fences | CR00; CR02 obligation interface | Actor and lifecycle integration |
| CR04 | Common durable admission and strategy-aware reconnect | CR01–03 | Desktop admission, discovery and net state |
| CR05 | Member-assisted route discovery and reverse connection | CR01, CR03–04 | Discovery, sync and shared dial scheduler |
| CR06 | Scoped inventories and bidirectional reconciliation | CR01–03; reviewed control adapter | Sync, replication and store |
| CR07 | Automatic forwarding, unopened service and bridge failover | CR05–06 | Mesh transport/application forwarding |
| CR08 | Long-absence control recovery and epoch integration | CR01–03; existing tenure/repair seams | MLS/control adapters and document lifecycle |
| CR09 | Dedicated service admission and sustained-abuse controls | CR01–03; service architecture | Service boundary and forwarded request checks |
| CR10 | UI continuity, mode explanation and safe diagnostics | CR03–09 interfaces | Native/frontend |
| CR11 | Adversarial integration, migrations and staged release | All applicable packages | Integration/acceptance owner |

CR06 and CR08 should develop against the same agreed control-readiness interface rather than inventing separate membership recovery. CR09 can progress in parallel with mesh work after mode and admission contracts are fixed. UI layout work can use typed mocks, but those mocks are not backend acceptance evidence.

**Critical first sequence:** CR00 -> CR02 -> CR03 -> CR04, with CR01 reviewed and available before the new reconnect permissions are enabled. This directly addresses false durability, warm/cold recovery and reply-code restart failure before larger mesh enhancements.

**No premature exposure:** new automatic network behaviour remains disabled until its authorization, limits and tests are integrated. Feature flags cannot override an authenticated dedicated policy.

### CR00 — Baseline and boundary designs

Create the baseline register described in section 2 and a deterministic harness with fake clock, controlled network edges, independently sealed stores, crash injection and captured production events. Add the reported direct-fails/reply-succeeds/immediate-restart case before fixing it. Record whether it fails and at which stage.

Write reviewable ADRs for: group mode/legacy upgrade; durable send/receipt semantics; forwarding/reverse-connect authority; historical-operation admission and long-absence recovery. Include the actual wire/store version changes, accounting keys, maximum shapes, trust assumptions and failure outcomes—not only high-level intent.

Inventory current timers, worker pools, native commands, record versions, recovery protocols, channel registry, current-topic checks and endpoint scheduler. Resolve whether the existing project already supplies each proposed seam. Obtain current status from the Gate 4 owners; list unavailable prerequisites rather than calling them completed.

**Exit evidence:** pinned baseline and test ledger; reviewed new contracts; a reproducible original failure or an explicit non-reproduction with traces; no production behaviour change hidden in the harness.

### CR01 — Authenticated mode and negotiation

Implement the mode policy, authenticated digest binding, persisted pin and typed native projection. Bind it into admission and protocol negotiation using reviewed existing governance. Add all supported legacy-version fixtures and interrupted-migration recovery.

Reject dedicated-policy downgrade attempts through missing fields, old peers, stale invitations, policy-version spoofing and helper forwarding. Separate dedicated authorities from ordinary peer roles. Do not let a local checkbox rewrite remote group security policy.

**Exit evidence:** T01–T05 and T39–T40; native join preview reflects verified versus unresolved policy; byte-level negative fixtures pass earlier signature/codec checks where applicable and isolate the intended policy guard.

### CR02 — Durability and stable retry identity

Trace and change sending so the logical operation and publication obligation have a safe commit barrier. Make persistence outcomes explicit through actor/native commands. Preserve snapshot coalescing and incarnation checks. Apply equivalent receiving/custody rules and receipt replay for a lost acknowledgement.

Define the precise store commit protocol, including required crypto/control state and multi-file transitions. Inject failures at each barrier; preserve non-durable in-memory work with honest status while avoiding duplicate retries.

**Exit evidence:** T06–T12; a snapshot/network write failure cannot produce a durable success; retrying one logical send creates one semantic operation; crash/reopen reconstructs pending publication without requiring clean shutdown.

### CR03 — Recovery coordinator and lifecycle integration

Consolidate current recovery triggers into actor-owned coalesced obligations. Add bounded timers, source fairness, exact completion context and explicit outcomes. Maintain service of inbound requests while outbound recovery is waiting.

Wire warm unlock, true teardown, cold reload, sleep/resume, network changes and task failure according to existing lifecycle policy. Rebuild UI state from snapshots separately from network recovery. Add cancellation/resource accounting and zero-external-event progress tests.

**Exit evidence:** T13–T18 and T47; repeated triggers create one bounded obligation; stale jobs cannot cross accounts/documents; quiet-network retries run; heavy recovery cannot starve current chat/control or another server.

### CR04 — Successful admission becomes restart-safe

Implement common finalization for direct, reply and authorized assisted admission. Record the real connection direction, exchange signed descriptors and persist continuing permission under the authenticated mode. Separate pending finalization from completed restart safety.

Preserve transport identities, sequence reservations and last proven alternatives. Do not turn reply tokens, inbound ephemeral ports or helper consent into unlimited future dialing. Add crash recovery for admission completed before `.net`/snapshot finalization.

**Exit evidence:** T19–T23. With the failed direction forcibly blocked, both participants reconnect in each immediate-restart order without a new code exchange whenever a usable permitted path remains. A failed refresh does not erase existing healthy evidence.

### CR05 — Automatic member assistance for routes

Extend existing signed peer discovery and bounded reverse-connect machinery. Reuse exact descriptor hashes/versions, current labels, endpoint validation, replay state and aggregate dial budgets. Preserve a working indirect path while exploring alternatives.

Wire interface/descriptor/relay changes to route invalidation and bounded retry. Renew real relay reservations rather than trusting persisted reservation-shaped addresses. Provide explicit `NoUsablePath` without discarding pending work.

**Exit evidence:** T24–T27 and the route-specific variants of T43–T44. Dedicated prolonged-probing coverage completes at CR09. Relayed/reverse requests cannot become an arbitrary socket-probing API. Provider/identifier rotation cannot reset all effective budgets. A recovered path can carry useful history even when direct upgrades fail.

### CR06 — Inventory and two-way multi-source reconciliation

Implement authorized directory/summary exchange, explicit source targets and dependency-complete progress tracking. Drive existing signed paginated export in both directions. Bind continuations to provider/runtime/source generation and reset only source-specific state on failover.

Ensure accepted pages become durable before custody acknowledgement and onward advertisement. Bound scanned work and large-frontier handling. Add fair alternate-source selection and trigger neighbours when newly learned history changes the local revision.

Keep attachment availability, private drafts and historical retention separate. Do not rewrite document authorization or invoke generic apply paths for epoch-managed documents.

**Exit evidence:** T28–T35 and T48–T50. Equal-size divergent histories converge; a stale first source cannot hide a useful second source; duplicate-only pages advance; no accepted unique local work disappears.

### CR07 — Bridge communication and asynchronous replication

Make ordinary P2P member assistance automatic under the verified mode. Integrate live forwarding without duplicating existing gossip, bounded scoped request routing and unopened authorized source service. Track actual retention/custody instead of relying on UI watches.

Implement failover for lost bridges and response paths. Validate original issuer and end-to-end payload at endpoints; limit loops, amplification and aggregate resource use at every hop. Do not require direct non-neighbour edges for acceptance.

**Exit evidence:** T36–T38 and T43–T46. Forced line/ring topologies, temporal contacts and bridge restart pass with independently sealed stores. Removal of a bridge leaves pending obligations recoverable through alternatives.

### CR08 — Long-absence and epoch-aware recovery

Connect the coordinator to reviewed membership/control recovery and authorized rejoin. Specify and test all supported past-state windows. Preserve history authenticity during current-epoch re-sealing and use the actual historical-admission policy.

Consume existing Registry/Studio checkpoint, tenure and signed-repair results through typed adapters. Exercise repeated-owner tenure, incompatible sources, missing receipts, Prepared state, unpublished branches and source replacement. Coordinate shared-file changes with the existing Gate 4 integration owner.

**Exit evidence:** T41–T42 and T51–T55. Long absence has a truthful terminal or resumable outcome; no obsolete authority becomes new publication authority; no native Save/lifecycle bypass is introduced. An unavailable prerequisite remains an explicit release blocker for the affected path.

### CR09 — Dedicated-mode enforcement

Implement the current-admission service boundary, requester-bound scopes, revocation checks during service and persistent bounded security state. Enforce identical policy for direct and forwarded requests and coordinate relevant limits across dedicated instances.

Ensure a service outage does not expose P2P discovery or history serving. Test long-lived probing with virtual time, server restart, origin/helper rotation and malformed-but-bounded input. Keep public refusal responses minimally informative.

**Exit evidence:** T39–T45 and T56. Expired/revoked/unauthenticated callers cannot progressively extract protected inventories/history or trigger unbounded work. Dedicated fallback remains disabled even under simulated outages.

### CR10 — Native/UI states and diagnostics

Integrate typed outcomes and mode disclosure. Separate saved, replicated, delivered and read states. Rebuild state on unlock, handle event gaps and enforce native final-delivery/session fences. Add actionable status for unavailable history, storage failure and authorized rejoin.

Emit privacy-safe diagnostics tied to durable operations and recovery stages. Provide a reproducible support trace for the original reply/restart case without including messages, keys or raw private addresses.

**Exit evidence:** T14, T17, T22 and T57–T58; UI reflects authoritative native state and never reports a queued or merely forwarded message as recipient delivery.

### CR11 — Combined acceptance and rollout

Run the full matrix at one integrated SHA using real actors, transports and independent stores; add physical two-/three-device transport tests and supported-platform CI. Repeat after restoring any mutation-tested guard. Record unrun tests as unrun.

Exercise old/new client and store combinations. Run churn and quiet-network soak tests and report raw timing/resource evidence with fixture sizes and conditions. Verify feature rollback preserves records, pending work and policy pins.

**Exit evidence:** section 17 satisfied for the declared release scope. Request separate integration acceptance; do not self-declare Gate 4 complete from this programme's test results.

## 15. Acceptance and adversarial test matrix

Every test must name its production entry point, topology, store layout, invariant, exact assertion and evidence command. Test IDs below describe required cases, not existing test function names.

### 15.1 Mode, durability and lifecycle

| ID | Scenario | Required assertion |
|---|---|---|
| T01 | New P2P and dedicated groups | Authenticated mode survives invite, join, restart and UI reload. |
| T02 | Missing, unknown or forged mode/capability | No silent dedicated downgrade; legacy state is explicit. |
| T03 | Migration from every supported old record version | Identity, history and pending work remain unchanged; interrupted migration recovers. |
| T04 | Larger but unauthorized policy version or incompatible branch | Cannot replace the pinned verified policy. |
| T05 | Local role/config changed to look dedicated | Does not change group mode or authorization. |
| T06 | Same send token repeated before/after restart | One operation and one logical publication obligation. |
| T07 | Same retry token, different payload/scope | Explicit conflict; no hidden second send. |
| T08 | Disk full/write/flush/rename failure | No false durable result or premature ticket retirement. |
| T09 | Crash at each send commit/publication barrier | Accepted work survives; crypto sending state is safe; no duplicate semantic effect. |
| T10 | Receiver crashes before/after commit; acknowledgement lost | Sender retries safely; durable receiver returns the same custody fact. |
| T11 | Snapshot and network/policy records commit partially | Consistent reopen or explicit resumable finalization; no false restart safety. |
| T12 | Old backup/cloned store resumes | Safe recovery before fresh publication where required; no claim of perfect local rollback detection. |
| T13 | Repeated warm unlock and repeated recovery triggers | Healthy actor reused; obligations/tasks remain bounded. |
| T14 | Events suppressed during lock | Native snapshot restores history/unreads without waiting for new traffic. |
| T15 | True logout/account switch during network/worker completion | Old context cannot publish, persist or update new-account UI. |
| T16 | Sleep/resume and interface changes | Candidate freshness rechecked and one bounded expedited pass occurs. |
| T17 | Actor/native instance replaced while persistence waits | Old snapshot/result cannot overwrite the replacement. |
| T18 | Network quiet after a failure | Scheduled recovery progresses without incoming traffic or user interaction. |

### 15.2 Reconnect, history and bridges

| ID | Scenario | Required assertion |
|---|---|---|
| T19 | Direct dial fails; reply succeeds; each participant restarts | Proven direction/strategy persists; no new manual code while a usable route exists. |
| T20 | Both participants restart immediately after admission | Finalization resumes; no duplicate membership or lost route authority. |
| T21 | Callback arrives from an ephemeral port | Port is not promoted to a listening endpoint. |
| T22 | Admission succeeds but descriptor/state persistence fails | Connection can remain useful; UI does not claim restart safety. |
| T23 | No fresh route observed or recovery candidate fails | Last good evidence is retained; pending candidate expiry stays bounded. |
| T24 | Endpoint/port changes; old descriptor replayed | Fresh authenticated record helps recovery; stale replay cannot override it. |
| T25 | NAT asymmetry; reverse initiation works | Reverse request uses approved descriptor/scheduler and preserves indirect traffic. |
| T26 | Relay disconnect/reservation expiry/restart | Reservation is re-established before advertisement as usable. |
| T27 | All cached routes fail but an existing mesh neighbour remains | Discover/help/reconcile through that neighbour rather than waiting on the inviter. |
| T28 | Incomparable histories with equal counts | Both valid differences are retained and exchanged. |
| T29 | Unknown/wide heads and missing causal dependencies | No false completeness; bounded fallback progresses. |
| T30 | Many duplicate-only or empty-but-advancing pages | Cursor progresses beyond duplicate prefix and eventually reaches missing data. |
| T31 | Provider/source generation changes mid-transfer | Old cursor rejected; accepted content retained; new negotiation succeeds. |
| T32 | Continuous writes during backlog transfer | Captured target completes; later changes schedule another pass. |
| T33 | Fast first provider stale; slower provider has unique operations | Fair source selection obtains the missing history. |
| T34 | New/unopened channel and missing attachment | Directory discovers channel; source service is not UI-dependent; blob state remains distinct. |
| T35 | Same final projection but different edit/delete/reaction history | Required operations/evidence reconcile without resurrection or metadata loss. |
| T36 | A-B-C and longer line, all non-neighbour edges blocked | Live updates and history reach the ends without direct connections. |
| T37 | A gives B history, A exits, B restarts, C arrives later | Durable store-and-forward succeeds without A. |
| T38 | Bridge disappears; alternate temporal path appears | Obligations survive and resume; no pinned-provider deadlock. |

### 15.3 Security, epoch compatibility and resource safety

| ID | Scenario | Required assertion |
|---|---|---|
| T39 | Dedicated outage; peer offers legacy/P2P fallback | Pending work survives; forbidden fallback is not enabled. |
| T40 | Old client joins a dedicated group | Required capability enforcement prevents bypass. |
| T41 | P2P partition during removal, then valid removal learned | Local freshness tradeoff is explicit; learned removal cannot be rolled back. |
| T42 | Signed, backdated operation from a removed author | Admission follows verifiable policy, not the timestamp or signature alone. |
| T43 | Helper/origin/request-ID rotation and looping forwarding | Deduplication and effective aggregate budgets remain bounded. |
| T44 | Forged/signed malicious endpoints, private IPs, ambiguous peer binding | No unauthorized socket probing or cross-group descriptor reuse. |
| T45 | Dedicated probing over simulated days and service restarts | No progressive protected information disclosure or unbounded accounting/state. |
| T46 | Bad signatures, false heads, withheld pages, lying custody claims | No forged history/false completion; alternatives considered without deleting last local copy. |
| T47 | Heavy valid backlog plus cancellation and repeated UI work | Slots remain charged correctly; control/current chat/other-server progress remains fair. |
| T48 | Frontier/log size at accepted maxima; tiny reply after huge scan | CPU/scan work is bounded and yields; no actor monopoly hidden by byte caps. |
| T49 | Operation exceeds transfer budget; corrupt/truncated page | Explicit bounded outcome; no infinite retry of impossible work. |
| T50 | Retention differs or source compacts during transfer | Coverage refusal/renegotiation is truthful; deleted state is not resurrected. |
| T51 | Absence exceeds all retained key/control windows | Incremental recovery or authorized rejoin outcome; never silent raw-key adoption. |
| T52 | Conflicting MLS/control branches and repeated owner A-B-A | No largest-epoch shortcut; actual control/tenure evidence governs. |
| T53 | Flipnote Prepared/Closing/stale/provisional branch during chat recovery | Original ledger, archive and reference protections remain intact. |
| T54 | Same group, different document; stale mount/intent/inventory generation | Full-scope fences isolate the actual job and reject stale commits. |
| T55 | Signed repair unavailable or invalid | No generic chat merge chooses a repair winner or releases retained work. |
| T56 | Dedicated service failover while revocation/accounting is stale | Consistent authority or explicit refusal; instance hopping is not a bypass. |
| T57 | Forged/replayed presence and relayed storage acknowledgement | No false live presence, endpoint delivery or read status. |
| T58 | Exported diagnostics and remounted frontend | No sensitive payload/identity leak; state aligns with authoritative native snapshots. |

### 15.4 Test implementation layers

**Deterministic state-machine tests:** injected time/RNG, precise packet loss/reordering/duplication, denied edges, source churn, crash points and store faults. Generate multiple histories with valid dependencies; after a finite churn period, hold a cooperative authorized path available and assert eventual convergence of the admissible retained set. Test bidirectional edges and asymmetric initiation separately.

**Production integration tests:** real actors, authenticated transport, independently sealed stores and native conversion. A ready-cache injection does not establish discovery. A `Vec` handoff does not establish network bridging. Assert forbidden edges were never connected, not merely that no direct dial was requested in the fixture.

**Platform/real-device tests:** cold restart, warm lock, actual interface/NAT changes and independent devices. Include a three-member bridge test with the end-to-end direct path genuinely unavailable. Do not equate a loopback-only success with Internet reachability.

**Adversarial tests:** make negative fixtures satisfy preceding codec/signature/member checks so the intended guard is reached. For critical policy, stale-context and commit-order guards, execute a targeted mutation: one intended assertion must fail, restore source byte-for-byte, and rerun the passing regression. Build failure or zero selected tests is not mutation evidence.

**Soak and resource tests:** model many groups/channels, large histories, partial replicas and repeated restart/churn. Measure queue/capacity bounds, retained bytes and fair service progress, not only average throughput. Run both active and completely quiet periods. Pin profile constants and include raw output rather than reporting unrepeatable absolute timing claims.

## 16. Integration, rollout and review process

### 16.1 Work isolation and existing owners

Use separate worktrees and branches. The repository's Gate 4 handoff explicitly requires this, and its status records document shared-index contamination. Do not run broad reset/stash/cleanup or mutation harnesses in another agent's mutable checkout. Do not rewrite load-bearing historical commits to tidy the branch. [R6], [R7]

Suggested implementation lanes are **policy/admission**, **network/reconnect**, **replication/durability**, **UI/diagnostics** and **integration/tests**. These are work areas, not replacements for the existing numbered agents.

| Existing Gate 4 ownership | Coordination rule |
|---|---|
| Agent 1: runtime/store capture and commit/signing custody | Consume the agreed source/preparation/commit seams; no competing signing loop or pool. |
| Agent 2: manual lifecycle, provisional work and live tenure | Preserve lifecycle/tenure evidence; coordinate any shared invalidation or archive changes. |
| Agent 3: signed repair | Consume reviewed repair results; do not add another repair issuer or winner-selection path. |
| Agent 4: integration, shared dispatch/exports/docs/native registration and final evidence | Centralize overlapping wiring and request combined acceptance at a pinned SHA. |

Keep new leaves independently testable; make unavoidable central dispatch/exports/native changes in identifiable integration commits. Do not expose an unfinished native command merely to make imports compile. Preserve the existing UI mockup/adapters and use native Rust state as the authority. Existing Gate 4 and Gate 5 boundaries remain unchanged. [R6]

### 16.2 Review checkpoints

Every checkpoint report should contain:

```text
Package and scope:
Exact base/head SHA:
Source-confirmed changes:
New/changed wire, storage or authority contracts:
Dependencies consumed, with their accepted checkpoint:
Invariants and test IDs covered:
Exact commands, selected test counts and results:
CI links and tested platforms:
Targeted mutation result and restoration evidence:
Unrun tests / known limitations / blockers:
Proposed documentation and UI-contract changes:
Requested verdict: design / bounded implementation / combined integration
```

Do not infer a PASS from permission to continue, a document status label or green unrelated CI. Do not silently reopen a previously accepted invariant; report an actual demonstrated regression and its scope.

### 16.3 Local and CI checks

Use the repository's existing workflows and feature matrix, resolved at the implementation SHA. The following are starting commands, **not executed results** and not substitutes for native/platform CI:

```bash
cargo fmt --all -- --check
cargo test --locked -p catcoms-replication -j 1
cargo test --locked -p catcoms-sync -j 1
cargo test --locked -p catcoms-app -j 1
cargo clippy --locked --workspace --all-targets -j 1 -- -D warnings
```

Record required features and platform prerequisites when selecting the actual commands. On the constrained Windows development machine, keep Cargo work serial and use the established per-package settings as necessary; use CI for required native/platform builds rather than installing an unrelated toolchain or hiding a skipped native test. Shared workflows and lockfiles remain integration-owned. [R6]

### 16.4 Staged rollout

**Stage A — Baseline and local correctness.** Land reproductions, structured outcomes and safe persistence improvements. Keep new wire/admission behaviour disabled until its reviewed contract is ready.

**Stage B — Authenticated policy and immediate reconnect repair.** Land migration/negotiation and common admission finalization. Exercise real reply-assisted restart recovery before claiming the original incident fixed.

**Stage C — Compatible-client P2P mesh recovery.** Enable two-way reconciliation and automatic bridges only for authenticated eligible groups/clients after loop/resource and end-to-end tests pass. Do not label this a dedicated guarantee.

**Stage D — Dedicated enforcement.** Enable dedicated creation/service only after current admission, anti-downgrade, sustained-abuse and service-failover checks pass. There is no permissive fallback release.

**Stage E — Combined release.** Run the whole supported matrix, publish actual limitations and review the integrated SHA. Protocol negotiation and store versions must match release notes.

These stages describe feature exposure, not a requirement to delay all independent work. Package dependencies and accepted seams govern parallel development.

### 16.5 Rollback and disablement

A feature kill switch may stop new automatic attempts or select an already permitted compatible protocol. It must not erase accepted operations, remove publication obligations, lower a policy pin, reactivate an obsolete credential or turn a dedicated group into P2P.

Keep supported old-format readers/migration backups according to the reviewed store policy. Do not downgrade a new store into an old representation that loses author evidence or pending work. After rollback, UI status must distinguish paused recovery from completed delivery.

### 16.6 Decisions requiring concrete resolution before exposure

The plan makes the product choices, but these code-level contracts must be written and reviewed at CR00/CR01 rather than guessed by an implementation agent:

| Contract | Required resolution | Blocking scope |
|---|---|---|
| Authenticated mode binding | Exact governance proof, invite/wire representation and legacy upgrade rule. | New automatic permissions and dedicated mode. |
| Durable acceptance | Exact store/crypto-state transaction and recovery barriers. | Truthful saved/custody guarantees. |
| Forwarded recovery envelope | Existing crypto/key binding, permitted classes and budget/replay fields. | New opaque recovery forwarding. |
| Historical operation admission | Existing verifiable evidence, treatment of removed authors and explicit P2P freshness limits. | Expanded history imports. |
| Long-absence control recovery | Supported incremental path, unavailable-state outcome and authorized rejoin integration. | Full offline/restart recovery guarantee. |
| Shared epoch APIs | Current reviewed runtime, tenure and repair seams, including unavailable dependencies. | Affected Studio/Registry recovery paths. |
| Concrete resource profile | Actual constants, accounting keys and accepted maximal shapes. | Production network exposure. |

No implementation package may replace one of these with a generic `trusted: true`, `epoch >= current`, an unchecked network-supplied address or an assumed existing API. Routine internal implementation choices within already accepted contracts do not need repeated approval.

## 17. Definition of done

The declared communication-recovery release is ready for acceptance only when all applicable statements below have evidence at one integrated SHA:

- [ ] The reply-assisted admission/restart regression passes with the originally failing direction still unavailable, and no new manual code is needed while a usable permitted path exists.
- [ ] Warm unlock, true logout/account switch, cold restart and sleep/resume each have independent production-path coverage.
- [ ] Saved/custody claims follow tested durable barriers; retries preserve operation identity; unfinished work survives crashes.
- [ ] Histories exchange in both directions, consult multiple sources, retain local unique operations and make progress through large/duplicate-heavy transfers.
- [ ] Forced non-direct and temporal bridge topologies recover live updates and retained backlog without a permanent host, founder or online chat owner.
- [ ] New/unopened channels, missing dependencies, retained evidence, partial coverage and blobs have explicit recovery/service behaviour.
- [ ] Long absences, removals/rejoins and incompatible control/document state produce correct verified outcomes instead of numeric-epoch shortcuts or endless retries.
- [ ] P2P assistance is automatic and bounded; learned revocations, message authenticity and resource checks remain enforced.
- [ ] Dedicated policy survives migration, outage, old-client negotiation, forwarding and service restart without downgrade or requester laundering.
- [ ] Dedicated prolonged-probing tests show bounded service/resource exposure, including effective enforcement across its actual service topology.
- [ ] Existing flipnote runtime, source custody, tenure, repair, archive/reference and native Save boundaries remain intact.
- [ ] UI and diagnostics distinguish local durability, replica custody, recipient delivery, presence and synchronization limits.
- [ ] Storage/wire migration, supported platforms, resource maxima and rollout/rollback have executed evidence; unrun checks are not marked passed.
- [ ] The required independent reviews are complete for the scope claimed. This acceptance does not automatically close Gate 4 or authorize Gate 5.

**Implementation priority:** preserve accepted work; establish the real successful reconnect strategy; drive existing signed history exchange fairly in both directions; propagate that history through changing neighbours; enforce the distinct dedicated boundary. Optimize history summaries only after those behaviours are demonstrated.

## 18. Source references

Repository references are pinned to the inspected commit, not the moving branch. They support the stated baseline observations and integration constraints; proposed modules, protocols, tests and defaults are not claims about current implementation.

| Reference | Source and relevant scope |
|---|---|
| [R0] | Pinned repository commit and baseline. |
| [R1] | Desktop reconnect route selection, admission policy, identity uniqueness and candidate capture; `lib.rs`, lines 4890–5070. |
| [R2] | `ServerRecord`, `ServerNet`, reconnect policy and sequence reservation; `store.rs`, lines 1–250. |
| [R3] | Snapshot persistence outcomes/ticket retirement and address-cache persistence; desktop `lib.rs`, lines 3230–3385. |
| [R4] | Signed ingestion and current-epoch catch-up export, frontier comparison, provider-specific paging; `doc.rs`, lines 900–1130. |
| [R5] | Shared Registry/Studio discovery handles, mount/server fencing and unopened source service; `studio_exchange/discovery.rs`, lines 1–170. |
| [R6] | Existing Gate 4 agent handoffs: isolation, review, custody, lifecycle/repair ownership and integration gates. |
| [R7] | Agent 1 status/history: implementation boundaries and documented shared-checkout hazards. Later applicable status entries govern current implementation readiness. |
| [E1] | IETF RFC 9420, especially protocol overview, sequencing of state changes and security considerations. Used for MLS/control constraints, not as proof of Mewtual's implementation. |
| [E2] | Automerge Rust sync documentation. General protocol context only; validate actual APIs against the repository's lockfile before changing dependencies or calling APIs. |
| [E3] | libp2p Circuit Relay v2 specification: reservation expiry/connection requirements, resource limits and non-recursive reservation constraint. Not an implementation audit. |

[R0]: https://github.com/Thalpy/Mewtual/commit/48f906962b37ed71a967c04ef4978d019ff43586
[R1]: https://github.com/Thalpy/Mewtual/blob/48f906962b37ed71a967c04ef4978d019ff43586/apps/desktop/src-tauri/src/lib.rs#L4890-L5070
[R2]: https://github.com/Thalpy/Mewtual/blob/48f906962b37ed71a967c04ef4978d019ff43586/crates/catcoms-app/src/store.rs#L1-L250
[R3]: https://github.com/Thalpy/Mewtual/blob/48f906962b37ed71a967c04ef4978d019ff43586/apps/desktop/src-tauri/src/lib.rs#L3230-L3385
[R4]: https://github.com/Thalpy/Mewtual/blob/48f906962b37ed71a967c04ef4978d019ff43586/crates/catcoms-replication/src/doc.rs#L900-L1130
[R5]: https://github.com/Thalpy/Mewtual/blob/48f906962b37ed71a967c04ef4978d019ff43586/crates/catcoms-app/src/studio_exchange/discovery.rs#L1-L170
[R6]: https://github.com/Thalpy/Mewtual/blob/48f906962b37ed71a967c04ef4978d019ff43586/docs/GATE4-AGENT-HANDOFFS.md
[R7]: https://github.com/Thalpy/Mewtual/blob/48f906962b37ed71a967c04ef4978d019ff43586/docs/GATE4-AGENT-1-STATUS.md
[E1]: https://www.rfc-editor.org/rfc/rfc9420.html
[E2]: https://docs.rs/automerge/latest/automerge/sync/index.html
[E3]: https://github.com/libp2p/specs/blob/master/relay/circuit-v2.md
