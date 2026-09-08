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
commands. Gate 1 begins with the missing typed Studio operations; it does not wait for P1 support
for unrelated document types. Tests and review accompany each slice, not only gate 7.

| Gate | Remaining work | Completion evidence |
|---|---|---|
| 1. Typed Flipnote documents | Rust StudioIndex/Flipnote domain-op validation, deterministic projection, conflict/Restore data and exact checkpoint preflight; frame, byte, sfx and patch caps. Unsupported linked-score behavior stays unavailable until gate 6, never silently accepted. | Tests exercise valid edits, malformed/cross-document operations, both concurrent delivery orders and cap boundaries through the real P1 gate. |
| 2. Durable one-device Save/Load | Accounted vault/lifecycle ownership for these types; actor/native create/list/read/apply commands; publish real PIX blobs before frame records; sealed intents; frame/export/recovery CID enumeration and expiry metadata. No UI edits. | An actor/native-path test creates and edits a Flipnote, restarts the backend, and reads identical frames from real CIDs. Crash/storage-refusal cases preserve the last durable state. Saved open-epoch edits are labelled provisional, not receipted. |
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
      The epoch-zero StudioIndex semantic callback is tested through signed P1 edit/ingest,
      including causal target/predecessor attacks and rollback. Frame validation and production
      aggregate admission remain; no typed write adapter substitutes reader bounds for preflight.
- [ ] Exact checkpoint-size preflight and typed checkpoint/recovery representation.

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
Epoch-zero `validate_index_change` now supplies the Index semantic callback: canonical signed-actor
record binding, causal target existence (including overflow), immutable evidence and exact register
predecessors. Signed gate tests prove rejection leaves the document/log/gate unchanged. It is not
a production adapter: frame causal validation, exact checkpoint/recovery encoding and P1 preflight
remain gate-1 work; sound/score/export projections must land before those families can be admitted.
Checkpoint-epoch Index validation refuses pending its actual typed seed format.
No Studio live write path is installed and no gate is closed.
The fixture's numeric-only expiry view still needs an explicit absent/null/timestamp adapter at
gate 2; zero remains a timestamp and must never be used as a Never sentinel.

At `db979dd` on `Create-suite-2`, native `publish_pix` and `request_blob_bounded` are wired through
the actor. They publish/fetch immutable bytes, not a Studio document. `studio-store.ts` still
uses an in-memory object/blob map and placeholder CIDs; Rust Studio materializers and
create/list/read/apply commands are missing. P1 has tested protocol/store and cooperative
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
