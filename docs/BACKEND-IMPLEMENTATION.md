# Flipnote backend implementation checklist

This is an acceptance checklist, not a count of source files. UI layout, components and the
canonical Flipnote mockups remain user-owned. No item is complete merely because its core
helper exists. Audited on 2026-09-12 against `acdb7f8`; the active scope is **Flipnote and the P1
paths it requires**, not completion of the entire Creative Suite.

Frontend integration is tracked separately in [FLIPNOTE-UI-HOOKS](FLIPNOTE-UI-HOOKS.md): actual
native commands/events, retry rules and explicitly unavailable controls. Update it alongside
every bridge-facing slice; UI layout and rendering remain user-owned.

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

These are ordered integration milestones, not equal-sized percentages. Index/art milestones
through Gate 3 are implemented; wider sound/export coverage still belongs to Gate 6.
The first user-observable target is gate 2: **create, save, restart, reopen** through real backend
commands. Its Index/art native-path and reference-protection tests now pass; that art milestone
is implemented and the next active work is gate 4. Wider sound/export families remain gate 6
work, so this does not close every family in the seven full gates. Gate 1 begins with typed Studio
operations; it does not wait for P1 support
for unrelated document types. Tests and review accompany each slice, not only gate 7.

| Gate | Remaining work | Completion evidence |
|---|---|---|
| 1. Typed Flipnote documents | Rust StudioIndex/Flipnote domain-op validation, deterministic projection, conflict/Restore data and exact checkpoint preflight; frame, byte, sfx and patch caps. Unsupported linked-score behavior stays unavailable until gate 6, never silently accepted. | Tests exercise valid edits, malformed/cross-document operations, both concurrent delivery orders and cap boundaries through the real P1 gate. |
| 2. Durable one-device Save/Load | **Index/art milestone implemented:** accounted vault/lifecycle ownership, native commands, real PIX CIDs, sealed intents, conservative source/seed/recovery reference protection and three-state expiry. Extend these same seams to actual sound/export records in gate 6. No UI edits. | Actor/native create/edit/restart/reopen uses real CIDs. Failure cases preserve durable state. Fileshare unlisting/upload cleanup cannot delete referenced pixels; full scans/restart include superseded seed/history, pending intents and retained/staged recovery. Open edits remain provisional. |
| 3. Two-member collaboration and joining | **Index/art milestone implemented:** live sharing, automatic same-epoch repair, unopened saved-key service, keyed Registry/Studio checkpoint discovery and recovery-first Studio adoption through the existing actor/native worker. | Actual actors join after the fixture receipt, install Registry plus Index/art checkpoints, persist the Studio open tail and reopen. Provider restart needs no UI watch. Closing survives expired selection and restart; ordinary Read reestablishes the volatile watch, then no further action is needed. Existing cancellation/authority/durability tests and 70-op paging remain. The 8-MiB input/inventory and 256-KiB unrelated-cold rails remain; this is not arbitrary-size latency qualification. |
| 4. Rotation and recovery in the running app | **Active:** accounted Studio owner settlement, watched rotation, Registry pointer/tail maintenance, recovery List/Read/backup Export/Ack/Restore/Copy, settlement invalidations and conservative own-intent replay are connected, and the persisted eviction grace is enforced rather than waiting on Acknowledge. Remaining: running-app succession/signed fault repair and full-gate acceptance. | Focused crash/restart, solo three-rotation, Registry paging, Create after Index rotation, native recovery fences, deadline-promotion and replay/manual-disposition regressions pass, each guard confirmed to fail when removed. Full-gate acceptance and final suites remain pending. No pruning before receipt and durable recovery; manual recovery is not settlement. Backup Export is not `.pixa` (gate 6). |
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

The Gate 4 implementation checkpoints are recorded through `39ceb76`, checked against HEAD
`acdb7f8` on 2026-09-12. The per-commit rows below stop at `b6f137b`; the five Gate 4 commits
(`9799c6f`, `cbed5b7`, `ccddd23`, `dba52e5`, `39ceb76`) are described in the active-slice section
rather than as ledger rows. This groups the P1/Flipnote `feat` and `perf` commits from `57e51ad` onward, plus the
original P1 commit `a67e284` and the performance probe. It is not a repository-wide release changelog: unrelated
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
| Initial publication from ordinary actor/native Save / 3 | `cafb221` | [app/studio/publication.rs](../crates/catcoms-app/src/studio/publication.rs), [app/studio.rs](../crates/catcoms-app/src/studio.rs), [app/actor.rs](../crates/catcoms-app/src/actor.rs), [native/studio.rs](../apps/desktop/src-tauri/src/studio.rs), [actor Save regressions](../crates/catcoms-app/src/studio_exchange/tests/actor_save.rs) | Successful Create/Apply retains at most two actual store-returned packets and attempts existing one-shot publication under the same native/source custody and one aggregate two-second budget. Local Save survives send refusal/cancellation. No duplicate save or outbox; automatic watched receive, remote events, retry/catch-up and newcomer discovery remain open. |
| Bounded automatic receive and remote updates / 3 | `4a6c3a7` | [app/studio/receiver.rs](../crates/catcoms-app/src/studio/receiver.rs), [native/studio.rs](../apps/desktop/src-tauri/src/studio.rs), [app/actor.rs](../crates/catcoms-app/src/actor.rs), [inventory.rs](../crates/catcoms-app/src/store/epoch_recovery/inventory.rs), [native two-member regressions](../apps/desktop/src-tauri/src/studio/tests/receiver.rs) | Recent-target watches, one paced native worker, same Ready/lease/typed ingest and incarnation-fenced remote events work through ordinary commands. Its initial LOCAL 256 KiB whole-vault rail is superseded by the warm/cold split below; the inbox, coordinator, pacing and fail-closed pause remain. |
| Authenticated inventory-validation reuse / 3 | `ee67ad2` | [inventory/cache.rs](../crates/catcoms-app/src/store/epoch_recovery/inventory/cache.rs), [inventory.rs](../crates/catcoms-app/src/store/epoch_recovery/inventory.rs), [epoch_studio.rs](../crates/catcoms-app/src/store/epoch_studio.rs), [receiver regression](../crates/catcoms-app/src/studio_exchange/tests/receiver.rs), [profiling harness](../crates/catcoms-app/src/store/epoch_registry/tests/performance.rs) | Existing scanner memoizes pure validated footprints in a 64-entry mount-local LRU after fresh full-wrapper authentication/digest matching. Small targets receive beside warm larger unrelated histories under 8 MiB read / 256 KiB cold rails. Dense ~4.9-MiB inventory measured 10,646 ms cold versus 11/11/11 ms warm; full accounting/reference checks remain. Its cold-only active target policy is superseded only for the one prepared source below. This LRU is not a document cache, storage owner or receiver. |
| One owned active Studio source / 3 | `0d74190` | [epoch_studio/source.rs](../crates/catcoms-app/src/store/epoch_studio/source.rs), [app/studio.rs](../crates/catcoms-app/src/studio.rs), [receiver.rs](../crates/catcoms-app/src/studio/receiver.rs), [source regressions](../crates/catcoms-app/src/store/epoch_studio/tests/source.rs), [two-member regression](../crates/catcoms-app/src/studio_exchange/tests/receiver.rs), [Studio probe](../crates/catcoms-app/src/store/epoch_studio/tests/performance.rs) | The mounted store moves one verified source through warm read/receive, with actual full-wrapper/context/budget checks and the existing typed ingest/durable save. Index refresh preserves art. A 6,939-op Studio fixture received three new durable edits without a full restore; diagnostic cold 142,337 ms / warm 192/193/184 ms, with exclusions in the performance report. Encoded slot cap 8 MiB, cold source cap 256 KiB; not all-watch service or a heap promise. Reconnect/catch-up and newcomer discovery/seed installation remain; do not rebuild the live path. |
| Studio same-epoch operation pages / 3 | `86ed32a` | [rep/studio/epoch/catchup.rs](../crates/catcoms-replication/src/studio/epoch/catchup.rs), [shared page engine](../crates/catcoms-replication/src/registry_epoch/catchup.rs), [store/epoch_studio/receive.rs](../crates/catcoms-app/src/store/epoch_studio/receive.rs), [source.rs](../crates/catcoms-app/src/store/epoch_studio/source.rs), [page regressions](../crates/catcoms-app/src/store/epoch_studio/tests/pages.rs) | Reuses the existing bounded prefix/cursor algorithm, with a separate Studio domain/channel binding and unchanged Registry golden. Warm-only saved-source serving and all-or-none typed page admission use the same owned graph/save/flush path. 70-op distinct-member Index/art and >256-KiB provider/receiver tests pass; failure can leave only old or whole-new durable state. No transport route, provider lifecycle/rates, cursor owner or reconnect driver is added. Next: bind these to authenticated transport and existing native custody; no new paging engine/store is needed. |

| Automatic same-epoch Studio catch-up / 3 | `443c5f0` | [sync page exchange](../crates/catcoms-sync/src/registry_catchup/studio.rs), [app cursor owner](../crates/catcoms-app/src/studio_exchange/pages.rs), [receiver runtime](../crates/catcoms-app/src/studio/receiver/catchup.rs), [detached source preparation](../crates/catcoms-app/src/store/epoch_studio/preparation.rs), [native idle wake](../apps/desktop/src-tauri/src/studio.rs), [actor regressions](../crates/catcoms-app/src/studio_exchange/tests/reconnect.rs) | Kind 23 shares Registry queue/rates/capacity; detached authenticated attempts and the existing native worker repair missed watched edits without more user actions. Both real actors can request and serve concurrently. Save-before-cursor, cancellation, channel/MLS supersession, fairness and cold-art/small-Index regressions pass. Reuses the previous two rows rather than replacing their store/page algorithm. Unopened-key service, current-owner checkpoint discovery and Studio adoption remain Gate 3. |

| Cooperative Studio checkpoint discovery/adoption / 3 | `b6f137b` | [shared scopes](../crates/catcoms-sync/src/checkpoint_exchange.rs), [head attempts](../crates/catcoms-sync/src/receipt_head/detached.rs), [seed attempts](../crates/catcoms-sync/src/registry_seed/detached.rs), [Studio installer](../crates/catcoms-app/src/store/epoch_studio/adoption.rs), [source service](../crates/catcoms-app/src/store/epoch_studio/discovery.rs), [app custody](../crates/catcoms-app/src/studio_exchange/discovery.rs), [joined-member regressions](../crates/catcoms-app/src/studio_exchange/tests/discovery.rs) | Kinds 24/25 reuse the existing head/seed engine, all its budgets and owner-proof rules. Source Closing/Fault, typed recovery and separate successor use the same store/gate/inventory, not a new replication system. Ordinary Studio restart v1 is unchanged; adoption v2 and the Registry-only lineage ceiling are regression-tested. Explicit discovery/install/tail/reopen works after real endpoint bootstrap; service/runtime orchestration follows in the closure below. |

### Gate 4 active slice: bounded recovery hold and slot-order-proof replay (`39ceb76`)

Gate 4 has five committed implementation checkpoints, all on this branch and all ancestors of HEAD:

| Slice | Commit | What it added |
|---|---|---|
| Owner settlement preparation | `9799c6f` | Typed owner close/receipt core adapter and recovery-bound successors |
| Owner rotation and recovery inspection | `cbed5b7` | Registry pointer/tail maintenance in the idle worker; native recovery List/Read/Export/Ack |
| Recovery controls, replay and settlement events | `ccddd23` | Per-item Restore/Copy, separately retryable pointer restoration, own-intent replay, `settlement-changed` |
| Frozen-owner succession foundations | `dba52e5` | Shared installed-opening inheritance checks; Studio/Registry frozen-source takeover and exact-journal restart through the accounted store; paced replay follow-ups. This is not running-app succession acceptance. |
| Bounded recovery hold, slot-order-proof replay | `39ceb76` | **Current slice.** Enforces the eviction grace and removes a slot-ordering dependency from replay |

**Current slice (`39ceb76`), two fixes.** First, the seven-day eviction grace was never enforced:
`EpochRecoveryAction::AdvanceTime` had no production call site, so every settlement and
installation path returned `RecoveryPending` for as long as a persisted warning existed, and a
document needing a third recovery slot stayed Closing until somebody pressed Acknowledge - which
is precisely what the grace exists to bound.
[`store/epoch_recovery.rs`](../crates/catcoms-app/src/store/epoch_recovery.rs) now exposes
`advance_due_epoch_recovery_with_writer`, which promotes a staged version whose *persisted*
deadline has passed under the caller's own accounting and writer custody, and writes nothing before
then so an idle owner pass cannot churn the record or restart the grace. All four paths that hold
the same shape share it: Studio settlement
([epoch_studio/rotation.rs](../crates/catcoms-app/src/store/epoch_studio/rotation.rs)) and
checkpoint adoption ([epoch_studio/adoption.rs](../crates/catcoms-app/src/store/epoch_studio/adoption.rs)),
plus Registry installation and adoption
([epoch_registry/installation.rs](../crates/catcoms-app/src/store/epoch_registry/installation.rs),
[epoch_registry/adoption.rs](../crates/catcoms-app/src/store/epoch_registry/adoption.rs)).
Acknowledge now only brings the eviction forward, which is what
[FLIPNOTE-UI-HOOKS](FLIPNOTE-UI-HOOKS.md) tells the UI to show.

Second, replay screened the mutable register in every retained and staged version, but the element
*birth* only in the first version found. Retained slots are newest-first and the staged slot is
newer than both while sorting last, so the newest evidence was never the one checked and slot order
decided a safety question. In [studio/replay.rs](../crates/catcoms-app/src/studio/replay.rs),
`selected()` now returns provenance, `choose()` requires every version carrying the envelope to
agree on it, and `agreed_predecessor()` derives the explicit after-edge the same way. Disagreement
becomes Manual, the existing bounded disposition. One element id has one register, so this is a
determinism fix rather than a recovered wrong write. Five regressions cover both, each confirmed to
fail when its own guard is removed: deadline promotion on the frozen-owner and settlement paths
with an injected writer that fails if any recovery write happens inside the grace, and contested
frame births and object creations held Manual in both slot orderings, with the rival birth
deliberately the losing one so the extra evidence is the only difference.

Remaining for Gate 4: running-app succession, signed fault/repair, and full-gate acceptance.

#### Gate 4 progress audit (2026-09-12)

Most rotation/recovery plumbing is connected; the gate remains unaccepted. The rows below
describe observable boundaries, not equal portions of work or a percentage of time remaining.
The table audits the implementation checkpoints; fresh test evidence follows it. Full-gate suites and
acceptance have not been rerun or closed by this audit.

| Area | Current boundary | Evidence still needed |
|---|---|---|
| Ordinary owner rotation | Watched Index/art rotation, durable decisions, recovery-first installation and solo installed-head completion are connected. | Full production-adapter acceptance across owner absence, partitions and lifecycle failures. |
| Registry maintenance | Derived pointers, current-tail paging and Create after Index rotation are connected. | Include these in succession/restart acceptance; preserve per-bucket Fault isolation. |
| Recovery and own-intent replay | Seven native recovery commands, settlement invalidations, conservative replay/manual disposition and persisted eviction deadlines are connected. | Final combined acceptance and remaining fairness/backpressure/cold-source follow-ups in HANDOVER. |
| Owner succession | `dba52e5` already supplies core and store takeover of a frozen source, exact decision retries and whole-source recovery. | Exercise the actor/idle worker after a witnessed owner transition, including Open and Closing sources, restart and a post-succession joiner. |
| Signed fault/repair | ReceiptRepair v2 and bounded receipt-book loser screening have protocol regressions. | Durable repair issuance/application, recovery-before-replacement, distribution, owner-journal handling and runtime exit from Fault. Restore/Copy does not supply these. |
| Remaining UI state and gate acceptance | Current phase/recovery invalidations exist; every current view is provisional. | Persisted Closing overlays, provisional old-owner newcomer reads and specialized tenure/repair observations still need integration evidence. Then run the complete gate scenarios, required suites and user-provided adversarial review. |

**Current test slice (review fixes awaiting re-review):** two tests in
[studio_exchange/tests/succession.rs](../crates/catcoms-app/src/studio_exchange/tests/succession.rs)
pass through the actor Ready/lease and idle worker after an observed MLS owner transition and
restart. One preserves an ordinary own Save in an Open epoch; the other refuses Save in the
old owner's Closing source and requires its full historical projection in recovery. Both require
the new owner's durable receipt, Open successor and Registry pointer, then reopen the vault and
check them again. The only added helper prepares an eligible old-owner close over the existing
source fixture. The successor receipt and installation are produced by the existing runtime.

The user-provided source review of `d7ea514` requested two assertion fixes, without identifying
a production defect. SUC-001 now checks the requested title, new-owner attribution, nonce and
operation id independently of the returned view, then checks the saved exact envelope and its
pending intent before settlement. SUC-002 now requires the actor's `EpochClosed` error string
and immediately compares the physical document id, phase, operation count, projection and whole
pending-intent journal before/after refusal, explicitly excluding the rejected operation.
Both revised cases passed locally. Three temporary mutations then failed at the intended new
assertions: replacing Open Apply with Read failed the requested-title check; substituting an
unrelated Closing error failed the exact-error check; moving edit validation after intent
persistence failed the unchanged-journal check. The mutation runner restored the test and store
source files byte-for-byte. Logs: `logs/gate4-succession-mutation-*.log`. On the restored tree,
`cargo test --locked -j 4 -p catcoms-app --lib studio_actor_new_owner -- --nocapture` passes
both cases (25.07 seconds; `logs/gate4-succession-review-final-tests.log`), root formatting
passes, and `cargo clippy --locked -j 4 -p catcoms-app --tests -- -D warnings` passes
(`logs/gate4-succession-review-clippy.log`). The broader suite results below belong to the
original checkpoint; this assertion-only correction does not claim a fresh full-gate run.

The transition uses the existing staged-Remove protocol as a **test fixture**, then restores
the strict single-committer configuration before the Studio actor runs. Production owner-transfer
policy is unchanged. These are header-only Flipnote fixtures, epoch zero to checkpoint one;
they do not establish post-succession PIX availability. Index takeover, inheritance from an
already installed checkpoint, A-to-B-to-A runtime behavior, a post-succession joiner and signed
repair remain outstanding. These two passes do not close running-app succession.
The final vault reopen follows completed takeover and orderly actor shutdown; interruption
inside the new owner's installation and editing through a newly restored successor actor are
not covered. The isolated actor/verifier pair supplies no new multi-peer convergence evidence.

Original `d7ea514` evidence: `cargo test --locked -j 4 -p catcoms-app --lib studio_actor_new_owner -- --nocapture`
passes both cases; log: `logs/gate4-succession-focused.log`. Frontend tests pass 1,189/1,189,
root formatting and `cargo clippy --locked -j 4 -p catcoms-app --tests -- -D warnings` pass.
The broader `cargo test --locked -j 4 -p catcoms-app --lib studio_ -- --test-threads=4` run
passes 138 tests, with one existing opt-in profiling test ignored (710.89 seconds;
`logs/gate4-audit-studio-tests.log`). The ambient-dependency gate fails on four pre-existing
`Instant::now()` calls in native `media_decode.rs` (333, 426,
444, 504), outside this diff. Full-gate acceptance remains pending. Rust 1.89.0 and the standalone
Windows build tools/SDK are installed for local tests; application builds are left to GitHub
at the user's request. The next boundary is user-provided re-review of the assertion fixes.

#### Landed on the way: Studio owner settlement preparation (`9799c6f`)

Studio has the typed core adapter in
[owner.rs](../crates/catcoms-replication/src/studio/epoch/owner.rs) and
[settlement.rs](../crates/catcoms-replication/src/studio/epoch/settlement.rs), with regressions in
[owner/tests.rs](../crates/catcoms-replication/src/studio/epoch/owner/tests.rs). This reuses P1's
close validation, receipts, dependency projection and the existing Studio checkpoint/recovery
codecs. It does not replace the Registry implementation or add another finality protocol.

Implemented: immutable owner close/receipt preparation and exact restart/resume; typed seed from
the selected closure; complete included/excluded envelopes; recovery for excluded content AND
compactor omissions; exact-source-fenced construction of a separate successor. Seven focused
tests cover Index/art, later edits, restart, same-tenure retries, A-to-B-to-A succession, epochs
above Registry's product ceiling, malformed/stale plans, deletions and fifth conflict values.
Existing cap regressions also check the actual compactor's omission predicate.

The work areas that followed it, in order (areas within Gate 4, not new gates). All four are now
committed; the wording below is kept because it says what each area actually covers:

1. **Committed (`cbed5b7`):** this adapter is connected to the existing accounted owner journal,
   recovery-first store installation and exact covered-intent retirement.
   The Index/art crash matrix exercises actual source/journal/recovery/intent/successor writers,
   recovery eviction acknowledgement, reopen, retry and unchanged-file durability boundaries.
2. **Committed (`cbed5b7`); full-gate acceptance verification still open:** the existing idle worker
   rotates watched owner documents, completes installed-head availability without a second member's
   query, refreshes their Registry pointers and receives Registry open-tail pages. Solo owner
   rotation across three restarts, two-member actual Registry paging, large-page inventory
   continuity and per-bucket Fault isolation have focused regressions. These are extensions of the
   existing worker, journal, page protocol and inventory cache, not replacement systems.
3. **Replay connected (`ccddd23`), then made slot-order independent (`39ceb76`):** paced
   own-envelope replay checks all retained snapshots and current state, orders stable-id
   dependencies and holds competing mutable edits. Unsafe choices can use the user-approved
   recovery-first manual disposition; missing evidence stays pending. Contested births now require
   agreement across every version carrying the envelope rather than trusting the first slot found.
   Remaining: running-app succession and signed fault/repair.
4. **Connected (`cbed5b7` for List/Read/Export/Ack, `ccddd23` for Restore/Copy, pointer restoration
   and settlement events; grace enforcement in `39ceb76`):** native recovery List/Read/backup Export
   and exact eviction Ack share the existing actor/vault/session custody. Per-item Restore/Copy and
   separately retryable pointer restoration pass focused tests. Settlement invalidations now cross
   actor/native guards, and the eviction countdown is enforced rather than decorative.
   [FLIPNOTE-UI-HOOKS](FLIPNOTE-UI-HOOKS.md) documents the tested callable names and limitations;
   it is checkpointed at the same commit as this document.

No automatic Studio rotation or new native hook is claimed by the first core slice.
Full root/native/frontend suites, root formatting and Clippy, native check, ambient-dependency
and diff checks passed. Final read-only adversarial review found no remaining findings; the
HANDOVER entry records commands, evidence and two focused coverage follow-ups for integration.

Gate 4 integration files, committed but not yet a completed Gate 4 or a released native contract:
`store/epoch_studio/{rotation,registry}.rs`, `studio_exchange/rotation.rs`,
`studio/receiver/catchup/{rotation,registry_runtime}.rs`, and the existing Registry
receive/provider and owner adapters (`cbed5b7`). Tests live beside those modules. No UI component
or canonical mockup changed. Remaining succession/repair and full acceptance are required before
Gate 4 is complete. Replay/Restore additions reuse `studio/{replay,restore,settlement}.rs`,
`studio/receiver/replay.rs`, the existing accounted intent/recovery writers and native custody
(`ccddd23`), with `store/epoch_recovery.rs` and `studio/replay.rs` corrected in `39ceb76`.

Gate 4 acceptance also must exercise **Create after Index rotation**: the current Create adapter
now uses its existing two-write/intent path against the actual current locally checked Index;
the new object still begins at epoch zero. Its actor regression passed. No new
creation or persistence subsystem was introduced.

### Keeping this ledger useful

Gate 3 Index/art closure (`5f24262`, 2026-09-09; extends `b6f137b`, does not replace it):

| Reused implementation | Added integration | Files |
|---|---|---|
| Existing bounded head/seed/page queues and debt | Exact prepaid unopened-key interests; no implicit UI subscriptions | [sync/epoch_service.rs](../crates/catcoms-sync/src/epoch_service.rs), existing families' `service.rs` |
| Existing native worker, Ready/vault lease and Studio source | Owner-lifecycle snapshot, automatic discovery, Closing retries, source service and watch-safe remote updates | [receiver/catchup.rs](../crates/catcoms-app/src/studio/receiver/catchup.rs), [discovery](../crates/catcoms-app/src/studio/receiver/catchup/discovery.rs) |
| Existing Registry prepared page source and inventory LRU | Prepared head/seed service, fixed cache-slot expiry, associated large-Registry footprint preparation | [registry_catchup.rs](../crates/catcoms-app/src/registry_catchup.rs), [runtime adapter](../crates/catcoms-app/src/studio/receiver/catchup/registry.rs), [source footprint](../crates/catcoms-app/src/store/epoch_registry/page_source.rs) |
| Existing typed adoption and save-before-cursor | Actual post-checkpoint newcomer Index/art, unopened provider restart, Closing restart and larger-data regression | [unopened tests](../crates/catcoms-app/src/studio_exchange/tests/unopened.rs), [pool lifetime regression](../crates/catcoms-app/src/registry_catchup/tests/preparation.rs) |

No new replication format, persistence owner, UI component or owner-issuance protocol was added
by this closure. Registry checkpoint bootstrap is deliberately narrower than its current tail;
Gate 4 must implement pointer publication/refresh and Registry tail receive, plus automatic
rotation/succession/replay/recovery controls. Existing unrelated cold-inventory and retained-source
rails remain; this milestone does not promise bounded latency for every accepted history shape.

Verification: full root and native Cargo test suites and all 1,144 frontend tests passed.
Root formatting, all-target/all-feature Clippy with warnings denied, native Cargo check and the
ambient-dependency gate passed. Final read-only adversarial review found no remaining
blocker/high/medium in this boundary. See the latest HANDOVER entry for commands and evidence.

Related runtime foundations (reuse references, **not extra Flipnote progress**):

| Existing work | Commit | Extend here |
|---|---|---|
| Numeric-server snapshot ordering and incarnation checks | `c3bd3b2` | Native `persist_captured` / `persist_lock_for` in [lib.rs](../apps/desktop/src-tauri/src/lib.rs). Studio already holds the same persistence guard; do not add another snapshot owner. |
| Detached prepare/worker/current-state completion pattern | `5cf2f56` | [actor/file_transfers.rs](../crates/catcoms-app/src/actor/file_transfers.rs) and the existing actor completion arm. Reuse the lifecycle pattern for future long read-only jobs; do not park the actor through registry reconstruction. |

The 2026-09-08 reuse audit found no confirmed accidentally reimplemented completed feature in
the compared registry/Studio store and exchange commits. They share P1 admission, budgets,
reconciliation and one-shot transport, with different typed validators. Similar adapter shapes
are maintenance duplication, not proof of interchangeable finished features. `cf7c8f4` replaces
the slow per-page restore path and removes its old forwarding method; it still uses the same
core page provider. The earlier regression-counter finding concerned test precision, not a
second paging protocol. This is a scoped audit, not a whole-history guarantee of zero rework.

Before a slice, find its row, read the current entry points and nearby tests, then name the missing
integration or failing acceptance case. Extend the existing path unless a concrete incompatibility
requires replacement. After a slice, add its commit, primary files and verified capability or update
the matching row; keep the outstanding boundary explicit. Record relevant `fix`, `test`, refactor
and non-conventional commits too when they change that boundary, not just `feat`/`perf` titles.
If work supersedes an earlier approach, mark which implementation replaces it instead of leaving
two apparently active solutions. Use the full diff for file history; do not duplicate INTERFACES
or infer percentages from commit counts. **Gate 4 is active, reusing the completed Gate 3 paths.**

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
it. Gate 4 now selects the actual current Open Index epoch inside that same transaction, so
creating a new object after Index rotation works; the new object still starts in epoch zero.
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
Those cooperative calls alone supply no scheduler, remote UI event or implicit blob fetch; the
automatic runtime described below adds scheduling and events, not blob fetch. The detailed API/bounds
are in [INTERFACES](INTERFACES.md#studio-operation-exchange-gate-3-cooperative-indexart-adapter).
Gate 3 also has split registry source preparation: capture authenticated bytes, rebuild on a
worker without store/Server borrows, then install only if runtime/mount/member/job generation and
exact saved record still match. Four process-wide slots include cancelled workers and retained
results. Warm pages recheck the full saved record without replaying it; cold/stale pages require
local preparation. Receipt faults invalidate caches even when the source operations are unchanged.
**Next implementation target: Gate 4 rotation and recovery runtime.** Unopened-provider service,
keyed discovery and Studio seed installation now reuse the completed page/native/source machinery.
Authenticated transport, cursor/retry ownership and reconnect scheduling are already integrated.
The typed page engine and atomic vault adapters now exist; do not rebuild their inbox,
saved-only send, persistence or preparation paths. Automatic work must not repeat unrestricted
history reconstruction under actor/vault locks. The initial receiver uses a conservative local
work rail and pauses. One verified active source can now be reused within its encoded-input
rail; cold dense preparation remains expensive and does not get a larger request deadline.
The [performance report](P1-PERFORMANCE.md) separates cold
preparation from warm serving; neither larger deadlines nor skipped validation are used.
Initial actor/native Save now attempts bounded one-shot publication directly from successful
durable output under the same lease, without another save/retry pass. Local/provisional remains
the acknowledgement; failed sharing never erases Save. Automatic recent-target receive now reuses
the SAME native lease and typed store, and emits remote updates only after accepted persistence.
The native two-member test includes real frame CIDs, both edit directions, paced receive and
restart. At most 16 recent targets are watched; missed edits on those targets now catch up
automatically. Closed/unwatched objects still need discovery. The scanner memoizes pure Registry/Studio footprint validation in a
mount-local 64-entry LRU, matching freshly authenticated complete wrapper bytes on every hit.
Its LOCAL rails are 1024 directory entries, 64 records, 8 MiB authenticated P1 bytes and 256 KiB
cold validation across the vault. Normal Save/full scans warm it; Read warms its actual Studio
record, not unrelated histories. Missing/corrupt/changed records and reference enumeration cannot
use stale metadata. The mounted store also retains one owned verified active Studio unit, moved
rather than cloned into another slot. Warm views and receive reauthenticate its exact full wrapper;
receive also checks current context and the fresh inventory/budget before normal typed ingest/save.
Index/list refreshes preserve opened art while keeping only verified Index footprint metadata.
The slot permits up to 8 MiB encoded input, not heap; large watched sources now prepare outside
the actor under the shared four-process-slot pool. Small cold Index gossip keeps the bounded
ingest path so it cannot evict warm art. Unrelated cold inventory still obeys its original rail.
Storage/admission failures pause until explicit access; network/context supersession retries.
This does not lower document acceptance caps or authorize skipped validation. The pause event
is bridged but its warning UI is user-owned. The actual dense Studio probe measured 142,337 ms
cold reconstruction and 192/193/184 ms warm inventory/ingest/save, with exclusions documented in
[P1-PERFORMANCE](P1-PERFORMANCE.md). Cold first-open and local Save can remain slow. Recovery of
missed packets and automatic Index/art newcomer joining are implemented under those rails.
The new Studio page wrapper shares Registry's existing walk and constants, adding only typed
scope/channel cursor binding. Registry's published cursor golden remains unchanged. Cooperative
store serving refuses a cold/stale source, and batch receive validates an entire page under the
same owned gate before one durable write/flush. Invalid input saves no prefix; write-after-rename
uncertainty permits only old or whole-new state and requires reconciled exact retry. Saved results
provide counts/frontier, not cursor ownership or finality. Distinct-member 70-op Index/art tests
and real >256-KiB prepared provider/receiver tests exercise these seams. Additive kind 23 now
shares Registry's request queue, rates and capacity. Detached attempts, current mount/runtime
wrappers and the native five-second idle wake drive automatic paced retries without another
user action. Two real actors repair independently missed edits in both directions. Original
heads/seed remain fixed until a pass finishes; no page result installs a checkpoint or retires
an intent. Unopened-key service and keyed discovery now extend this same bounded runtime.
The one-device art Save/Reopen/reference-protection milestone is now implemented. Sound/export
writers and their actual record coverage remain gate 6, not another prerequisite to art progress.
The user-owned frontend must adapt these documented results instead of the fixture's numeric-only
expiry and in-memory blob map; no UI source or canonical mockup was changed.

At `db979dd` on `Create-suite-2`, native `publish_pix` and `request_blob_bounded` are wired through
the actor. They publish/fetch immutable bytes, not a Studio document. `studio-store.ts` still
uses an in-memory object/blob map and placeholder CIDs; the new native commands provide the
durable Index/art replacement, but the frontend has not been switched over. P1 has tested protocol/store and cooperative
registry discovery/settlement adapters, with automatic Studio joining now integrated. Automatic
owner rotation, Registry tail/pointer publication and recovery orchestration remain Gate 4.
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
