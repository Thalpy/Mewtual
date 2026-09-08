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
| 3. Two-member collaboration and joining | Automatic runtime catch-up/gossip, keyed discovery and seed installation for registry and Studio documents; typed update events. Fence whole-server snapshots and cancellation/authority changes. Solve repeated dense source restoration with bounded off-executor preparation/reuse before enabling automatic serving. | Two members exchange edits automatically; a newcomer finds an epoch-0 document and a prepared rotated document by logical key. Tests cover stale/cancelled work and accepted-size histories within the request lifecycle, without relaxing validation/deadlines. |
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
**Next implementation target: gate 3's two-member Index/art edit exchange and newcomer path.**
The one-device art Save/Reopen/reference-protection milestone is now implemented. Sound/export
writers and their actual record coverage remain gate 6, not another prerequisite to art progress.
The user-owned frontend must adapt these documented results instead of the fixture's numeric-only
expiry and in-memory blob map; no UI source or canonical mockup was changed.

At `db979dd` on `Create-suite-2`, native `publish_pix` and `request_blob_bounded` are wired through
the actor. They publish/fetch immutable bytes, not a Studio document. `studio-store.ts` still
uses an in-memory object/blob map and placeholder CIDs; the new native commands provide the
durable Index/art replacement, but the frontend has not been switched over. P1 has tested protocol/store and cooperative
registry discovery/settlement adapters, but not automatic production orchestration for Studio.
The latest saved-source probes still show about 11 seconds for a dense registry page, beyond
the current request deadlines. This is an integration blocker, not completed performance work.
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
      byte-heavy pages, but a valid 8,002-small-op source still takes roughly 11 seconds per saved
      page (baseline about 13), exceeding request deadlines. Bounded off-executor reconstruction
      and source reuse need version/authority fences before automatic scheduling; fixed memory/rate
      caps alone do not prove latency. This prerequisite remains open.
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
