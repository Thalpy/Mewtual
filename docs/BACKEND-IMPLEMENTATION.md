# Flipnote backend implementation checklist

This is an acceptance checklist, not a count of source files. UI layout, components and the
canonical Flipnote mockups remain user-owned. No item is complete merely because its core
helper exists. As of 2026-09-08, the active scope is **Flipnote and the P1 paths it requires**,
not completion of the entire Creative Suite.

## Scope reset (2026-09-08)

The user paused games and asked to focus on Flipnote. Game-only avatar consent, avatar-profile
publication changes and play protocols are paused too. This does not remove existing security
checks or decide what consent a future game would require.

The active target retains Flipnote's canonical Art/Sound/Music contract: durable frames,
collaboration, advisory claims, recovery, per-frame sfx, linked scores and export. Work on
standalone Pictochat, chat/announcement doodles, knocks, games and GB cam is deferred. Shared
identity/channel work is included only where required for Flipnote claims or audio; it does not
authorize implementing those other products. The initial P1 consumers are the registry,
StudioIndex and StudioObject (flipnotes and linked scores), not PostReplies or every future type.

P1's existing receipt, recovery, authorization and capacity guarantees still apply. P2's new
expiry-enforcement pass remains non-blocking; accurate Flipnote reference enumeration and its
connection to any existing reclamation path are required before promising retained frames.

## Fixed delivery gates

These are ordered integration milestones, not equal-sized percentages. All seven remain open.
The first user-observable target is gate 2: **create, save, restart, reopen** through real backend
commands. Its Index/art native-path and reference-protection tests now pass; that art milestone
is implemented and the next active work is gate 3. Wider sound/export families remain gate 6
work, so this does not close every family in the seven full gates. Gate 1 begins with typed Studio
operations; it does not wait for P1 support
for unrelated document types. Tests and review accompany each slice, not only gate 7.

| Gate | Remaining work | Completion evidence |
|---|---|---|
| 1. Typed Flipnote documents | Rust StudioIndex/Flipnote domain-op validation, deterministic projection, conflict/Restore data and exact checkpoint preflight; frame, byte, sfx and patch caps. Unsupported linked-score behavior stays unavailable until gate 6, never silently accepted. | Tests exercise valid edits, malformed/cross-document operations, both concurrent delivery orders and cap boundaries through the real P1 gate. |
| 2. Durable one-device Save/Load | **Index/art milestone implemented:** accounted vault/lifecycle ownership, native commands, real PIX CIDs, sealed intents, conservative source/seed/recovery reference protection and three-state expiry. Extend these same seams to actual sound/export records in gate 6. No UI edits. | Actor/native create/edit/restart/reopen uses real CIDs. Failure cases preserve durable state. Fileshare unlisting/upload cleanup cannot delete referenced pixels; full scans/restart include superseded seed/history, pending intents and retained/staged recovery. Open edits remain provisional. |
| 3. Two-member collaboration and joining | **Cooperative saved-operation send/receive implemented** for Index/art; explicit bounded registry source preparation/reuse is now available. Still: runtime ownership/driving, automatic catch-up/gossip, keyed discovery/Studio seed installation and remote events. Fence whole-server snapshots and cancellation/authority changes. | Two members must exchange edits automatically; a newcomer must find epoch-0 and prepared rotated objects by logical key. Explicit send/durable-receive/reopen and warm registry page serving pass, but do not close scheduling, joining or all accepted-size latency evidence. |
| 4. Rotation and recovery in the running app | Drive owner receipts without needing another member's query; atomic sealing, recovery-first settlement, own-intent replay, owner succession, fault/repair and recovery actions/events for the active types. | Production-adapter scenarios cover rotation, restart, owner offline/return, excluded edits, Restore/Copy/Export, storage exhaustion and staged-snapshot warnings. No pruning before the receipt and durable recovery barriers. |
| 5. Collaborative frame claims | Required full-identity signalling and shared channel admission; bounded capability/session-bound claim, Ask and Pass messages with receiver-observed expiry. No game/avatar path or standalone drawing feature. | Two members observe advisory claim/Ask/Pass/expiry; collision, replay and disconnect tests pass. Claims never become edit locks. |
| 6. Sound and export | Linked-score typed operations/preflight/recovery, sfx/emoji patch sources, 64-patch union, deterministic valid-take export and byte-exact `.pixa` publication with durable export records. Cover the specified local GIF export contract without taking over UI design. | No-score and linked-score golden vectors, maximal accepted exports and malformed/over-cap rejection pass; exported bytes can be read back and validated. Playback-facing contracts preserve Deafen and membership teardown. |
| 7. Flipnote backend acceptance and UI handoff | Run the complete create/save/restart/share/join/rotate/recover/export flow through production adapters, including failure paths. Publish Markdown for the real commands, events, limits and recovery behavior. | Backend acceptance tests and mandatory suites pass, adversarial blocker/high findings are resolved, and every canonical UI dependency maps to a working command/event or explicitly user-owned rendering work. |

Runtime ownership and storage admission are prerequisites within gates 2-4, not optional polish
after exposing writes. A successful small local-save test does not close shared editing or P1.
Gate 3 can test discovery against an explicitly prepared receipted fixture; automatic receipt
production closes only at gate 4. Gate 5 supplies the reviewed identity/channel prerequisites
before collaborative claims ship; art persistence need not wait for the whole C4 Draw product.

Gate 1 substeps (not extra product gates):

- [x] Static IndexOp/FlipnoteOp body codec, complete-envelope checks and shared frontend vectors.
- [ ] Deterministic Studio projections with stable ids, ordering, conflict and deletion evidence.
      StudioIndex and the Flipnote art/frame subset now have read-only Automerge projections.
      Sound/score/export projection support remains separate; unsupported state rejects.
- [ ] Causal Automerge-change validation and aggregate admission through the actual P1 gate.
      `StudioTarget` now supplies Index/art writers and signed P1 edit/ingest with exact
      prospective checkpoint AND whole-version recovery preflight, in epoch zero and verified
      checkpoint epochs. Sound/score/export families remain fail-closed pending gate 6 support.
- [ ] Exact checkpoint-size preflight and typed checkpoint/recovery representation.
      Complete for Index/art: canonical compact seeds, full typed recovery, original attribution,
      bounded conflicts, exact aggregate encoding and omission of old ops from successive seeds.
      The checkbox covers all Flipnote families; audio/export representation remains pending.

Progress reports name the gate, the observable behavior proved, tests/review evidence and the
remaining blocker. The older 25%/65% figures are retired: they estimated broad foundations,
not usable Flipnote integration or time remaining. Do not replace them with a new guessed figure.
Do not expand P1's guarantees or implement a deferred feature without asking the user. A
performance/refactor slice must identify the failing gate and the acceptance evidence it enables;
micro-optimization alone is not a reason to postpone Studio integration.

## Completed-work ledger: reuse before adding

Audited against this branch's committed history through `dda1fad` (2026-09-08). This groups
the P1/Flipnote `feat` and `perf` commits from `57e51ad` onward, plus the original P1 commit
`a67e284` and the performance probe. It is not a repository-wide release changelog: unrelated
voice, files, release and user-owned UI work is not marked as Flipnote progress. Commit subjects
are discovery aids, not proof of completion; the current contracts and limitations below govern.

Use this ledger for **what exists and where to extend it**, [INTERFACES](INTERFACES.md) for
the exact API contract, and [HANDOVER](HANDOVER.md) for test/review evidence and historical
limitations. The file column names primary entry points, not every touched test or supporting
file. `git show --stat <commit>` and `git show --name-status <commit>` provide the full change map.
An implemented helper or cooperative adapter does not close an end-to-end delivery gate.

### Existing P1 foundations and registry adapters

Paths below use `rep/` = `crates/catcoms-replication/src/`, `app/` = `crates/catcoms-app/src/`,
`sync/` = `crates/catcoms-sync/src/`, and `native/` = `apps/desktop/src-tauri/src/`.

| Reusable work / gates | Commits | Primary files | Implemented boundary; still to connect |
|---|---|---|---|
| P1 records, gates, checkpoints and registry / 1-4 | `a67e284`, `57e51ad` | [rep/epoch.rs](../crates/catcoms-replication/src/epoch.rs), [rep/checkpoint.rs](../crates/catcoms-replication/src/checkpoint.rs), [rep/registry.rs](../crates/catcoms-replication/src/registry.rs) | Signed operations, owner receipts/faults, bounded recovery and verified seeds are existing core machinery. Extend typed consumers; do not rebuild finality. |
| Recovery persistence, inventory, capacity and temp cleanup / 2, 4 | `40d8aa6`, `4a4b603`, `efaffc4`, `b7c6af2` | [app/store/epoch_recovery.rs](../crates/catcoms-app/src/store/epoch_recovery.rs), [app/store/epoch_recovery/](../crates/catcoms-app/src/store/epoch_recovery/), [app/store/epoch_budget.rs](../crates/catcoms-app/src/store/epoch_budget.rs) | Sealed transitions and replacement-space accounting exist. They are not the Studio runtime settlement driver or recovery UI commands. |
| Durable owner decisions and intent ledgers / 2, 4 | `5262409`, `17e6694`, `fc6d32e` | [app/store/epoch_owner.rs](../crates/catcoms-app/src/store/epoch_owner.rs), [app/store/epoch_intents.rs](../crates/catcoms-app/src/store/epoch_intents.rs) | Existing persist-before-publish journal, bounded local intents and inventory. Studio already reuses these storage foundations; no second journal is needed. |
| Checked registry restart, ingress, seals and exact local retries / 2-4 | `6655c2c`, `e26a3ae`, `b497e30` | [rep/registry_epoch.rs](../crates/catcoms-replication/src/registry_epoch.rs), [app/store/epoch_registry.rs](../crates/catcoms-app/src/store/epoch_registry.rs) | Owned registry source and accounted durable writes exist. Registry-specific semantics are not interchangeable with Studio validators. |
| Receipt-bound settlement and recovery-first installation / 4 | `a09eb1e`, `6d55c25`, `9bc9ec4` | [rep/registry_epoch/settlement.rs](../crates/catcoms-replication/src/registry_epoch/settlement.rs), [app/store/epoch_registry/recovery.rs](../crates/catcoms-app/src/store/epoch_registry/recovery.rs), [app/store/epoch_registry/installation.rs](../crates/catcoms-app/src/store/epoch_registry/installation.rs) | Explicit registry settlement saves recovery before replacement and retires only covered intents. Automatic Studio settlement/actions remain gate 4. |
| Durable replay and one-shot publication / 3-4 | `0d053dc`, `eea698d`, `cd6270e`, `fc508f1` | [app/store/epoch_registry/replay.rs](../crates/catcoms-app/src/store/epoch_registry/replay.rs), [app/registry_replay.rs](../crates/catcoms-app/src/registry_replay.rs), [sync/registry_publication.rs](../crates/catcoms-sync/src/registry_publication.rs), [transport.rs](../crates/catcoms-rt/src/transport.rs) | Checked saved-intent replay and driver-acknowledged send exist. Caller still owns scheduling/lifecycle; send admission is not delivery or settlement. |
| Watched registry gossip / 3 | `fc777ef` | [app/registry_ingress.rs](../crates/catcoms-app/src/registry_ingress.rs), [sync/registry_ingress.rs](../crates/catcoms-sync/src/registry_ingress.rs) | Opt-in authenticated receive with durable admission. Not automatic Studio gossip. |
| Paged catch-up and durable receiver continuation / 3 | `cce0528`, `81fb91b`, `3a8928c` | [rep/registry_epoch/catchup.rs](../crates/catcoms-replication/src/registry_epoch/catchup.rs), [app/registry_catchup.rs](../crates/catcoms-app/src/registry_catchup.rs), [sync/registry_catchup.rs](../crates/catcoms-sync/src/registry_catchup.rs) | Bound cursors, authenticated exchanges and save-before-advance exist for registry. Explicit bounded source preparation/reuse now supplies version/authority fences; runtime driving remains. |
| Observed tenure, keyed receipt heads and expected-hash seed fetch / 3-4 | `1cb161b`, `4084c50`, `2a40b2b` | [sync/owner_tenure.rs](../crates/catcoms-sync/src/owner_tenure.rs), [app/registry_head.rs](../crates/catcoms-app/src/registry_head.rs), [app/registry_seed.rs](../crates/catcoms-app/src/registry_seed.rs) | Cooperative authenticated registry discovery exists; fetching alone installs nothing. Reuse the checked handles, not raw proof fields, for runtime joining. |
| Recovery-first adoption of discovered registry checkpoints / 3-4 | `a765bdb`, `9f779d1` | [rep/registry_epoch/adoption.rs](../crates/catcoms-replication/src/registry_epoch/adoption.rs), [app/store/epoch_registry/adoption.rs](../crates/catcoms-app/src/store/epoch_registry/adoption.rs), [app/registry_seed.rs](../crates/catcoms-app/src/registry_seed.rs) | Explicit installer preserves the source and recovery before replacement. Studio installation and automatic newcomer orchestration remain open. |
| Owner rotation and reply-handoff completion / 4 | `fd2943f`, `022bbc9` | [app/store/epoch_registry/owner.rs](../crates/catcoms-app/src/store/epoch_registry/owner.rs), [app/store/epoch_owner.rs](../crates/catcoms-app/src/store/epoch_owner.rs), [app/registry_head.rs](../crates/catcoms-app/src/registry_head.rs) | Durable exact owner decisions and checked reply-channel completion exist. Quiet/solo progress and Studio orchestration remain; handoff is not remote delivery. |
| Measured restore-query optimization / 3 | `c6092c1` (probe), `db979dd` | [rep/doc.rs](../crates/catcoms-replication/src/doc.rs), [rep/registry.rs](../crates/catcoms-replication/src/registry.rs), [app/store/epoch_registry/tests/performance.rs](../crates/catcoms-app/src/store/epoch_registry/tests/performance.rs) | Redundant history queries were removed without relaxing checks. Before source reuse, dense service exceeded deadlines; reuse the [cold/warm measurements](P1-PERFORMANCE.md), not a claim that every accepted shape's latency is solved. |
| Prepared registry page sources / 3 | `cf7c8f4` | [app/registry_catchup.rs](../crates/catcoms-app/src/registry_catchup.rs), [store/page_source.rs](../crates/catcoms-app/src/store/epoch_registry/page_source.rs), [rep/registry_epoch/catchup.rs](../crates/catcoms-replication/src/registry_epoch/catchup.rs), [regressions](../crates/catcoms-app/src/registry_catchup/tests/preparation.rs) | Split capture/worker/install, four process-wide cancellation-safe slots and exact full-record version fences exist. Dense warm pages measured 36–45 ms after 12,658-ms preparation. Reuse this job/source path; runtime driving, native lifecycle/snapshot custody and automatic Studio joining remain open. |

### Flipnote-specific work already integrated

| Reusable work / gates | Commits | Primary files | Implemented boundary; still to connect |
|---|---|---|---|
| Real PIX publication and bounded fetch / 2, 6 | `19e75d4` | [app/creative.rs](../crates/catcoms-app/src/creative.rs), [native/creative_blobs.rs](../apps/desktop/src-tauri/src/creative_blobs.rs), [sync/lib.rs](../crates/catcoms-sync/src/lib.rs) | Actor/native `publish_pix` and `request_blob_bounded` exist. Use these for real CIDs; do not add another publication primitive. Export packaging remains gate 6. |
| Closed operation codec and shared vectors / 1 | `f77b8f9` | [rep/studio.rs](../crates/catcoms-replication/src/studio.rs), [rep/studio/patch.rs](../crates/catcoms-replication/src/studio/patch.rs), [shared vectors](../crates/catcoms-replication/tests/fixtures/studio-ops-v1.json) | Static Index/Flipnote bodies, full envelope checks and patch validation exist. A recognized audio/export body is not an implemented materializer or writer. |
| Deterministic Index/art projections / 1 | `c383cde`, `8718328` | [rep/studio/index.rs](../crates/catcoms-replication/src/studio/index.rs), [rep/studio/frames.rs](../crates/catcoms-replication/src/studio/frames.rs) | Stable ordering, conflicts/deletions and cap flags exist. Sound/score/export projection is the remaining extension, not a reason to rebuild art. |
| Causal Index/art mutation validation / 1 | `20d4c17`, `327883c` | [rep/studio/index/change.rs](../crates/catcoms-replication/src/studio/index/change.rs), [rep/studio/frames/change.rs](../crates/catcoms-replication/src/studio/frames/change.rs) | Signed-actor records, causal targets/origins and exact predecessor checks exist. All consumers must retain these callbacks. |
| Typed checkpoints, recovery and P1 admission / 1, 4 | `9902e49` | [rep/studio/admission.rs](../crates/catcoms-replication/src/studio/admission.rs), [rep/studio/snapshot.rs](../crates/catcoms-replication/src/studio/snapshot.rs), [rep/studio/recovery.rs](../crates/catcoms-replication/src/studio/recovery.rs) | Index/art exact seed/recovery preflight, verified checkpoint edits and bounded rotations are core-tested. Running-app receipt/settlement/recovery control remains gate 4. |
| Accounted Index/art vault Save/Reopen / 2 | `7fbd683` | [rep/studio/epoch.rs](../crates/catcoms-replication/src/studio/epoch.rs), [app/store/epoch_studio.rs](../crates/catcoms-app/src/store/epoch_studio.rs) | Complete signed sources, sealed intents, exact retries, five-family inventory and restart exist. Reuse this owned source rather than a parallel persistence format. |
| Actor/native local Save/Reopen and local events / 2 | `1a0ad9d` | [app/studio.rs](../crates/catcoms-app/src/studio.rs), [app/actor.rs](../crates/catcoms-app/src/actor.rs), [native/studio.rs](../apps/desktop/src-tauri/src/studio.rs) | Five Studio commands, lifecycle custody and real PIX save/restart tests exist. Local/provisional results are not shared edits; UI adaptation is user-owned. |
| Reference protection at existing cache deletion paths / 2 | `bf1b64b` | [rep/studio/references.rs](../crates/catcoms-replication/src/studio/references.rs), [app/store/creative_references.rs](../crates/catcoms-app/src/store/creative_references.rs), [app/store/epoch_recovery/inventory.rs](../crates/catcoms-app/src/store/epoch_recovery/inventory.rs) | Saved art, seed/history, intents and retained/staged recovery hold their pixels. Reuse the shared guard/enumerator; expiry enforcement and actual export-record coverage are not included. |
| Cooperative saved-operation exchange / 3 | `dda1fad` | [app/studio_exchange.rs](../crates/catcoms-app/src/studio_exchange.rs), [sync/studio_exchange.rs](../crates/catcoms-sync/src/studio_exchange.rs), [two-member tests](../crates/catcoms-app/src/studio_exchange/tests.rs) | Saved-only own send, bounded authenticated watches/inbox and durable typed receive work in both directions, including reopen. Automatic runtime scheduling, Studio source ownership, catch-up/discovery and remote UI events remain open. |

### Keeping this ledger useful

Before a slice, find its row, read the current entry points and nearby tests, then name the missing
integration or failing acceptance case. Extend the existing path unless a concrete incompatibility
requires replacement. After a slice, add its commit, primary files and verified capability or update
the matching row; keep the outstanding boundary explicit. Record relevant `fix`, `test`, refactor
and non-conventional commits too when they change that boundary, not just `feat`/`perf` titles.
If work supersedes an earlier approach, mark which implementation replaces it instead of leaving
two apparently active solutions. Use the full diff for file history; do not duplicate INTERFACES
or infer percentages from commit counts. **Next remains gate 3, not another foundation pass.**

## Current evidence

Gate 1 now has the static Rust `studio::IndexOp`/`FlipnoteOp` codec: the closed operation set,
complete-envelope bounds/scope, full-identity creator binding and validated jam patch hashes.
Shared Rust/TypeScript byte vectors pin canonical encoding and three-state expiry. This is a
schema substep only. The subsequent read-only `studio::StudioIndexProjection` now materializes
the channel's object list from actual Automerge state: stable-id ordering, smallest-op-id
creation conflicts, Automerge rename/expiry winners, provenance-bearing tombstones, and explicit
overflow beyond 64 visible objects. It checks all live concurrent values, including hidden and
deleted content, and has primitive/byte reader bounds. This does not authenticate record claims.
The subsequent `FlipnoteFrameProjection` reads stable insertion-node order, pixel replacements,
title/fps registers and provenance-bearing deletions. It retains every live alternative/hidden
frame and flags the 999-frame / 8 MiB over-cap suffix without fetching blobs. Same-gap op-id
ordering respects captured placement; deleted/losing insertions remain ordering anchors.
It rejects sound/score/export state rather than return an incomplete successful view.
`validate_index_change` supplies the Index semantic callback: canonical signed-actor
record binding, causal target existence (including overflow), immutable evidence and exact register
predecessors. Signed gate tests prove rejection leaves the document/log/gate unchanged.
The art `validate_frame_change` likewise checks exact record
mutations and derives insertion origins from the sender's causal view, retaining hidden anchors.
Signed tests cover ordering, concurrent deletion, false origin claims and unchanged state on
rejection. Both now accept actual typed checkpoint baselines at the sender's causal frontier.
`StudioTarget` connects these validators and exact seed/recovery encoding to existing P1 gated
edits/ingest. Core tests cover edit/checkpoint/owner verification/reopen/concurrent edit, forty
rotations without history growth, actual seed-hash golden vectors, 999-frame/8 MiB and 64-object
concurrent overflow, 1024 conflict fields, and bounded complete recovery. This reuses existing P1
signing, gates, rollback and seed installation; it adds no finality protocol.
Sound/score/export projections must land before those families can be admitted. Historical-view
work is bounded but not latency-qualified for production scheduling. The explicit Index/art
actor/native path below is now installed; no full product gate is closed.
Gate 2 now has `StudioEpoch` and accounted `ServerStore` Index/art Save/Load. The store journals
the exact intent, persists the full signed source/gate/receipts, then returns prepared ciphertext.
Restart, exact retry, both crash barriers, corrupted files, storage-ceiling refusal and persisted
fault tests exercise the actual vault adapter. A real promoted PIX blob plus its saved frame CID
also survives reopening. The subsequent actor/native tests now prove this through real commands.
Five-family inventory includes Studio files and their temporary copies; no new budget or finality
protocol replaces the existing P1 foundations.
Gate 2 now exposes native `studio_create`, `studio_list`, `studio_read`, `studio_apply` and
`studio_apply_index`. Actor and native tests create a Flipnote, publish canonical PIX bytes,
save its real CID, restart from the command-saved server snapshot and reopen identical state.
Exact retries do not duplicate edits or overwrite later titles. Create writes the object then
the index; an index refusal can leave an unlisted object, and retrying the exact request completes
it. Current Create targets epoch zero; rotated-index creation awaits the gate 4 installer.
The Ready handshake transfers the sole live Server and mounted vault to a finite blocking worker,
retaining persistence/UI/registry guards even if the invoke or actor is cancelled. Busy fences
refuse without awaiting the actor. No snapshot success or shared publication is fabricated.
Views retain conflicts, tombstones, overflow and full identities, and say `publication: "local"`
and `provisional: true`. Expiry is explicitly unrecorded/never/at, including timestamp zero.
Local `studio-updated` events are forwarded; automatic remote edits/settlement events are not.
`ServerStore::creative_pinned_cids()` now reuses an opt-in complete five-family inventory to
derive byte-liveness holds from whole signed Studio histories, verified seed-only baselines,
pending intents and retained/staged typed recovery. It includes hidden/deleted/conflicting and
sequentially replaced pixels while their evidence remains held. This is conservative physical
protection, not a circulation-expiry pass. Its 65,536-reference rail refuses reclamation rather
than silently truncate; it adds no replicated cap or durable pin journal.
All same-mount persistent blob handles share a deletion guard outside the kept-copy adapter.
Writes add holds before I/O; uncertain writes retain them. Only a generation-current complete
scan can remove holds. Unknown/corrupt/unsupported/partial metadata keeps bytes; restored P1
mounts may defer GC until a successful Studio-triggered or explicit scan. Real unlist/upload
cleanup regressions prove protection while unrelated known-unreferenced cache bytes still delete.
The native transaction refreshes before pre-holding/reading new PIX bytes; its later accounting
scan cannot erase that unpublished hold. No UI change or automatic expiry engine is included.
Gate 3 now has explicit `watch_studio_epoch`, `send_saved_studio_once` and `receive_studio_step`
adapters over the existing transport and accounted store. Tests use two distinct members joined
through invites, real PIX Save, durable receiver admission and reopen, exact duplicate retry,
missing dependencies, stale watches/mounts, cancellation and a receipt arriving after queueing.
Only already-saved own operations can be sent; this is not another local edit/publication path.
There is no automatic scheduler, remote UI event or implicit blob fetch. The detailed API/bounds
are in [INTERFACES](INTERFACES.md#studio-operation-exchange-gate-3-cooperative-indexart-adapter).
Gate 3 also has split registry source preparation: capture authenticated bytes, rebuild on a
worker without store/Server borrows, then install only if runtime/mount/member/job generation and
exact saved record still match. Four process-wide slots include cancelled workers and retained
results. Warm pages recheck the full saved record without replaying it; cold/stale pages require
local preparation. Receipt faults invalidate caches even when the source operations are unchanged.
**Next implementation target: gate 3 runtime driving using these existing split jobs and exchange
adapters, then automatic catch-up/discovery and remote events.** Do not rebuild their inbox,
saved-only send, persistence or preparation paths. No automatic service may hold actor/vault
locks through reconstruction. The [performance report](P1-PERFORMANCE.md) separates cold
preparation from warm serving; neither larger deadlines nor skipped validation are used.
The one-device art Save/Reopen/reference-protection milestone is now implemented. Sound/export
writers and their actual record coverage remain gate 6, not another prerequisite to art progress.
The user-owned frontend must adapt these documented results instead of the fixture's numeric-only
expiry and in-memory blob map; no UI source or canonical mockup was changed.

At `db979dd` on `Create-suite-2`, native `publish_pix` and `request_blob_bounded` are wired through
the actor. They publish/fetch immutable bytes, not a Studio document. `studio-store.ts` still
uses an in-memory object/blob map and placeholder CIDs; the new native commands provide the
durable Index/art replacement, but the frontend has not been switched over. P1 has tested protocol/store and cooperative
registry discovery/settlement adapters, but not automatic production orchestration for Studio.
The earlier dense probe spent about 11 seconds rebuilding each source per page. That work now
belongs to explicit preparation, not each warm page; runtime integration and broader accepted-size
measurements remain separate from the cache primitive.
HANDOVER and `P1-PERFORMANCE.md` retain the individual commits' evidence.

## P1 foundation inventory (not the active delivery order)

The inventory below preserves completed foundation work and wider platform gaps. References to
all managed types are broad-design backlog; the seven gates above close only Flipnote's consumers.

- [x] Signed domain operations, epoch gates, close candidates, owner receipts and fault model.
- [x] Deterministic registry checkpoint materialization and projection preflight.
- [x] Vault-sealed registry, intent, receipt and recovery records, bounded inventory and reserves.
- [x] Recovery-first durable settlement and restart paths; included-only intent retirement.
- [x] Cooperative durable-intent replay and driver-acknowledged one-shot publication.
- [x] Opt-in authenticated registry gossip with durable receive admission.
- [x] Bounded durable registry operation pages with provider-local authenticated cursors.
- [x] Authenticated registry page request/response with bounded cooperative source serving.
- [x] Bounded receiver continuation that saves complete pages before advancing; joined-member
      divergence, duplicate, cancellation, rotation and uncertain-storage regressions.
- [ ] Keyed receipt-head and expected-seed discovery, including a newcomer after rotation.
      Keyed authenticated registry head queries and checked durable owner-selection proofs are
      implemented cooperatively. Kind-22 expected-seed fetch now retains exact typed bytes
      behind a fresh kind-21/runtime/MLS/owner-bound handle. Joined members exercise both routes
      and the installed vault source. The explicit accounted registry installer now saves the
      selected receipt/full source before optional seed work, typed whole-version recovery before
      replacement, and never retires intents. Restart, warning retarget, crash boundaries and
      an actual queued old-epoch page are covered. A fresh watch/pass catches up the successor.
      Fetch alone still creates no receiver files. The installer rechecks fresh context, mount,
      high-water and recovery; raw mutable
      `ReceiptHeadAnswer.proof: Some` is not an admission permit or proof the seed is available.
      Independently observed owner-tenure evidence is saved with MLS; unknown tenure must
      not authorize a fresh head proof by copying a restored receipt's claimed tenure. The
      proof publisher uses explicit local snapshot preparation and source/journal barriers. Legacy or
      newly joined owners may stay Unknown; do not substitute the current MLS epoch.
      This milestone remains open for automatic runtime ownership, other managed document types
      and production newcomer acceptance; cooperative registry integration is not the whole feature.
- [ ] Runtime ownership, scheduling, cancellation, vault lifecycle and complete storage accounting.
      Whole-server snapshot publication must share the native numeric-server persistence ordering
      or an exact-incarnation fence: an older asynchronously captured snapshot must not overwrite
      newer P1 MLS/tenure evidence. The separate store mutex alone does not establish this ordering.
      Release source profiling is recorded in `P1-PERFORMANCE.md`: indexed restore queries improve
      byte-heavy pages; a valid 8,002-small-op source previously took roughly 11 seconds per saved
      page (baseline about 13), exceeding request deadlines. Explicit bounded worker reconstruction
      and source reuse now supply version/authority fences. Runtime driving remains open; cold
      preparation and warm serving are measured separately, and slot/rate caps do not prove latency.
- [ ] Owner receipt issuance, succession, fault/repair and settlement driven end to end.
      Explicit registry owner rotation now derives an eligible close/seed from its checked source,
      journals that exact close with the receipt, and seals/installs recovery-first under a durable
      MLS/tenure snapshot permit. Restart resumes the same choice even after later Open edits.
      Kind-21 serving now records exact completion after an accepted local reply-channel handoff
      of a fresh owner proof (not transport-driver admission or delivery), allowing the next eligible
      rotation. Quiet/solo owner progress still needs orchestration because this path requires a query.
      Automatic driving, durable repair and all managed document types remain acceptance work.
- [ ] Recovery listing, Restore/Copy/Export actions and settlement events over the actor/bridge.
- [ ] Multi-peer, restart, partition, capacity and owner-offline acceptance scenarios through the
      production adapters rather than direct calls to protocol helpers.

## Wider Creative Suite backlog (deferred except the Flipnote subset above)

- [ ] C0 foundations: finish publication/retention and authoritative local avatar state;
      full-identity signalling and shared data-channel admission. Existing bounded PIX
      publication/fetch commands are implemented; the whole foundation is not yet complete.
- [ ] StudioIndex/StudioObject domain validators, projections, caps and P1 adapters.
- [ ] Studio actor/native commands, registry discovery and typed update/settlement events.
- [ ] Score and flipnote export, patch-union validation and referenced-blob enumeration.
- [ ] Announcement replies and chat doodle persistence/attachments.
- [ ] Non-visual draw/claim/replay, emoji-sound, ring and play protocol/state-machine contracts.
- [ ] Remaining backend/media support explicitly required by the creative design, with UI-only
      work separated from protocol/codec/export work when each slice is audited.
- [ ] Backend acceptance tests for the frontend's ten dependencies and documented honest limits.

## Completion and delivery rule

Each code slice gets focused regressions, a read-only adversarial review of the actual diff,
resolution of blocker/high findings, and all checks required by AGENTS.md. Verified slices are
committed; periodic non-force pushes require the outstanding destination approval for
`Thalpy/Mewtual`, branch `Create-suite-2`.
Unrelated work is preserved. This checklist and HANDOVER record actual progress and gaps.

The final UI implementation guide will list real commands, schemas, events, lifecycle/recovery
rules, limits and examples. It will distinguish production-ready backend paths from UI work
still to be built. Remaining material product/security choices require the user's direction;
they are not filled in by claiming a narrower definition of “100%”.
