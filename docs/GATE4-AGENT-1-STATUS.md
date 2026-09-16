# Gate 4 Agent 1 status: local Save and automatic handoff runtime

Owner: Agent 1 ([assignment](GATE4-AGENT-HANDOFFS.md#agent-1-local-save-and-automatic-handoff-runtime)).
Proposal: [GATE4-AGENT-1-DESIGN](GATE4-AGENT-1-DESIGN.md), revision 4, **accepted (PASS) at
`25ce89bb705dfc228c7b32874788ebd062e6fcf4`, 2026-09-15, design boundary only**.
Review preamble: 1. Current entries override older ones.

**The design boundary is complete. Implementation has started** on branch
`gate4-agent1-runtime`, from `5a899c2`. Progress and executed evidence are in "Implementation
progress" below. The bounded implementation review uses the request in design 18.3, and must not be
sent until the scope it names has evidence.

### Implementation sequencing, revised from design 15

Design 15 proposed I-4 and C-3 first because they are the largest shared-seam items. That ordering
is changed, deliberately and for stated reasons: C-1 is self-contained, removes the dominant cost
that every other stage depends on, and does not collide with Agents 2 and 3, who are editing the
same store files. I-4 and C-3 bound the commit visit's custody but block no product behaviour, and
they are the highest-conflict changes, so they land last. The per-item verdict request in design
18.3 is unchanged; only the order is.

| Order | Item | State |
|---|---|---|
| 1 | C-1 structural decode, with C-2's digest fences and the R4 replay exclusion | **landed; reviewed PASS; no-replay boundary covered by N24; R4 selection covered by N25/M9** |
| 2 | C-4 transient reference holds, with the I-3 transfer | **landed; seam reviewed PASS; transfer implemented and mutation-proven; dead-code markers removed** |
| 3 | The runtime: Flows S, H and R, admission, scheduling, commit seams | **Flow S store seams landed: capture, detached plan, stamp-checked commit, with N12(a). Actor admission, scheduling and Flows H/R not started** |
| 4 | I-4 and C-3 | not started |

## Checkpoints

| Date | Checkpoint | Base | Head | Kind | Verdict |
|---|---|---|---|---|---|
| 2026-09-15 | Design revision 1 | `a052f78b62a549702686a8741932f1d2f8c98773` | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | design, docs only | **REQUEST CHANGES**: AG1-001 to AG1-005, AG1-TEST-001 |
| 2026-09-15 | Design revision 2 | `ac12822f04337b3e388618f81ce4a4b29d1e9b87` | `56198de80e4942fd1612feff5d9d07f2f9cced7a` | design, docs only | **REQUEST CHANGES**: AG1-004 **closed**; residuals on the other five |
| 2026-09-15 | Design revision 3 | `56198de80e4942fd1612feff5d9d07f2f9cced7a` | `1bcb1bca204d721b848b17c0835faf931ae930e3` | design, docs only | **REQUEST CHANGES**: AG1-001, AG1-002, AG1-003, AG1-005 **closed at the design boundary**; AG1-TEST-001 open, P3 |
| 2026-09-15 | Design revision 4, N31 correction | `1bcb1bca204d721b848b17c0835faf931ae930e3` | `25ce89bb705dfc228c7b32874788ebd062e6fcf4` | design, docs only | **PASS**: AG1-TEST-001 **closed**; bounded runtime design accepted, no new finding |
| 2026-09-15 | PASS recorded | `25ce89bb705dfc228c7b32874788ebd062e6fcf4` | `5a899c2` | docs only | n/a; records the verdict and the next checkpoint's request |
| 2026-09-15 | C-1 and C-4 implementation | `5a899c2` | `d67e649`, `7ed6302` | bounded implementation, PR #27 | **PASS by source inspection** for the C-1 decoder, the C-2 digest changes, the R4 filter and the C-4 seam; no production defect found. Two coverage findings: **C1-TEST-002** (P2) and **R4-TEST-001** (P3). C-1 evidence not a completed checkpoint. |
| 2026-09-15 | C1-TEST-002 correction | `7ed6302` | `4d09869` | test only | **PASS**: C1-TEST-002 **closed**; R4-TEST-001 still open |
| 2026-09-15 | R4-TEST-001 correction | `4d09869` | `079e59a` | test plus one cfg(test) helper | N25 and M9; pushed, awaiting the reviewer's source inspection to close |
| 2026-09-15 | I-3 protection transfer | `079e59a` | `65e77d1` | bounded implementation | Mechanism **PASS**; **I3-001** (P2) opened: media admission ran before retry classification |
| 2026-09-16 | I3-001 correction | `65e77d1` | `b7df00b` | bounded implementation | **PASS**: I3-001 **closed** |
| 2026-09-16 | Flow S staged seams and N12(a) | `7b9cf3e` | uncommitted working tree | bounded implementation | First real detached window; runtime not acceptable until I-4/C-3 lands where H7 spans an inventory |

Working checkout: `M:\Git (local)\CatComs`, branch `Create-suite-2`. **Other agents are working in
this same checkout**: Agent 3's design landed at `7efc9c2` and Agent 2's documents are present
untracked. All four Agent 1 passes touch only these two files, and any commit must be
pathspec-scoped to them. Implementation must move to a separate branch or worktree.

## Finding ledger

Closure at the design boundary is not implementation acceptance: every mechanism below still needs
code and executed evidence, and the reviewer said so explicitly for each one.

| Finding | Severity | Status | Where |
|---|---|---|---|
| AG1-001 | P2 | **Closed at the design boundary** (revision-3 review). Accepted and completed retries no longer require media admission: S0 is split into common request validation, classification, and new-authoring-only media admission, and S1a is terminal and sync-only. | Design 6.2, 6.3, 14 N30, M16 |
| AG1-002 | P2 | **Closed at the design boundary**, subject to I-4's stated implementation audit. `inventory_generation` replaces the invalid two-token assumption; the reviewer confirmed the underlying facts about the recovery and owner writers and about budget entry. | Design 9.2, R10, 14 N17, M20, M21 |
| AG1-003 | P2 | **Closed at the design boundary.** I-3 establishes ordinary protection before potentially durable I/O and before transient ownership is released. | Design 8.3, R7, 14 N12, M14 |
| AG1-004 | P2 | **Closed** at revision 2. Prepared alone no longer prohibits nondestructive access; Agent 2 still owns the concrete lifecycle. | Design 12.1 |
| AG1-005 | P3 | **Closed at the design boundary.** Admission depends on actual owners, not an actor-side strong reference awaiting cleanup. | Design 5.5, 7.1, 14 N14, M3 |
| AG1-TEST-001 | P3 | **Closed at the design boundary** (revision-4 review). The residual was that N31 required only `remaining() > 0`, which a visit deferring on the priority gate without signing also satisfies, so both the unchanged and the mutated implementation could pass. Revision 4 adds a positive signing precondition (`after < before`, `after > 0`, exact expected count derived from production `remaining()`), a deterministic injected-clock seam, authoritative work staged only after slice selection, and independent per-limit preconditions, with M5 split into M5a and M5b. The reviewer confirmed the production basis: `remaining()` delegates to the pending queue and `sign_next` removes exactly one item only after the signature succeeds, so the delta counts **successfully produced** signatures. | Design 7.3, 14.1 "N31 in full", 14.2 M5a/M5b |

## Audit claims corrected across revisions

| Claim | Correction |
|---|---|
| Ordinary Studio reads always reconstruct a retained branch (rev 1) | Conditional on the source wrapper's required-metadata link byte plus a retained Active or Prepared branch. The unconditional costs are `checked_epoch_replay_state` and the Intents arm of every five-family scan. |
| "About four" reconstructions in the synchronous commit (rev 1) | Withdrawn; path dependent and possibly higher. |
| C-1's moved call sites are "projection-free" (rev 1) | Corrected to branch-replay-free: the reference path retains `base_blob_cids`, which calls `base.graph()` and performs typed seed work. |
| C-1 justified by AEAD sealing (rev 1) | Replaced by invariant I-1, a requirement on the writers. Authentication proves origin and integrity, not typed admission, and not every `write_prepared_intents` call follows a fresh `append`. |
| The digest fence is "strictly stronger" (rev 1) | Withdrawn. The existing decoder already requires canonical re-encoding equality. C-2's justification is cost only. |
| Intents is the one uncached family (rev 2) | Wrong. `inventory_cache` admits only `Registry` and `Studio`, and only when not collecting references, so `Recovery`, `OwnerReceipts` and `Intents` are all uncached. The surviving point is that the Intents arm is the one whose uncached cost scales with a retained branch. |
| `write_prepared_intents` performs an old-record read (rev 2) | Wrong. It consumes a supplied `old: Option<u64>`. C-2 changes its **callers** (`persist_handoff_intents`, `write_studio_overlay_intent`, `save_studio_closing_overlay_with_io`). |
| C-3 is a mechanical refactor (rev 2) | Wrong. It is a semantic consistency change; cross-visit resumption is sound only because of I-4. |
| H1 capture is strictly fresher (rev 2) | Withdrawn. Earlier capture can become stale while H2 runs; what is guaranteed is that a stale authority cannot authorise a signature or a commit. |

## Facts established by the revision-3 audit

Verified in the code at the design base, relied on from revision 3 onward, and confirmed
independently by the revision-3 reviewer:

- The only rotations of an inventory-relevant token are `epoch_intents.rs:479`, `:510`,
  `epoch_intents/retirement.rs:199`, `:233` (`intent_generation`),
  `epoch_recovery/cleanup.rs:94` (`studio_generation`), `:155` (`intent_generation`, coverage
  dependent), and `epoch_studio.rs:168` and `:203` (`studio_generation`, on budget mint and on
  budget **entry**).
- `update_epoch_recovery_accounted_with_writer` and `update_epoch_owner_state_with_writer` reserve,
  write and commit real record replacements and rotate neither token. This is AG1-002's
  counterexample, confirmed.
- `enter_studio_budget_scope` rotates `studio_generation` on every entry, so reusing it as C-3's
  token would also make a parked cursor die on unrelated Studio activity.
- `inventory_cache` is consulted only for `Registry` and `Studio`, and only when not collecting
  references.
- `write_prepared_intents` consumes a supplied `old` and rotates `intent_generation` on both
  branches.
- `StudioOverlay::encode_vault` itself calls `checked_entries`, which is why M8 had to move.
- `Protection::unknown()` drops `pins`; `install()` replaces the known set; `ProtectedBlobs::delete`
  consults only that set. Both halves of AG1-003, before and after the durable write, follow from
  this.

## Dependencies

| Dependency | Current state | Effect if unmet |
|---|---|---|
| Core signing split `e65bfd8` | **Unreviewed.** Both reviewers inspected its interfaces without granting a PASS. | Design 6.1 and 9 are bound to `handoff_authority`, `prepare_handoff_detached`, `sign_next` and `finish`. Contingency in design 16. |
| Agent 2 manual overlay lifecycle | Not started. | Native Save stays unregistered and absent from FLIPNOTE-UI-HOOKS. Prerequisites P1 to P5, design 12.3, now including `ReferenceCapacity` and `InventoryUnstable` holds. |
| Agent 2 live-tenure contract | Not started. | The runtime binds `tenure` as an opaque `u64` and needs "equal value implies the same continuous tenure" (P4). |
| Agent 2 copy contract | Not started. | Design 12.2 lists the requirements the revision-2 reviewer attached to copy-while-Prepared, including that a different channel label for the same Flipnote object is not an independent destination. |
| Agent 3 signed repair | **Design revision 1 landed at `7efc9c2`.** Its section 13.1 accepts I-4, names the owner record write, the recovery stage and the successor write as the three of its writers that must rotate the token, states its design is unaffected if I-4 does not land, and confirms it adds no competing source writer and no second preparation pool. It asks that `save_studio_source_checked`'s `handoff` parameter shape be preserved; this design preserves it. | The coordinated verdict on I-4 now has both sides on record. The exhaustive choke-point audit remains an implementation-review obligation. |
| Agent 4 shared seam, enum, registration and workflow edits | Not started. | Design 15 lists every central edit. I-4 is now the largest and highest-risk item. |

## Proposed API seams

Full signatures are in [the design](GATE4-AGENT-1-DESIGN.md) section 5. Summary:

- Core: `StudioOverlay::decode_vault_structural`, `StudioOverlayState::decode_vault_structural`
  (C-1).
- Store: `capture_studio_overlay`, `studio_overlay_is_current`, `studio_overlay_structural`,
  `commit_studio_overlay_state`, `studio_closing_basis`, `VerifiedPersistedSource`,
  `hold_creative_transient` and `CreativeHold` (C-4), `epoch_mutation_guard` and
  `inventory_generation` (I-4), `begin_epoch_storage_scan` / `step_epoch_storage_scan` /
  `finish_epoch_storage_scan` and `EpochStorageCursor` (C-3); `StudioOverlayStamp`,
  `StudioOverlayCapture`, `StudioOverlayWork`, `StudioOverlayPlan`, `StudioOverlayPlanned`,
  `StudioOverlayHold`, `StudioOverlayCapture::plan`.
- App runtime: `OverlayAdmission` (weak-handle bookkeeping), `OverlayRuntime`, `OverlayJob`,
  `OverlayStage`, `OverlayOwnership`, `StudioBackgroundJob::{OverlayPlan, OverlayAssemble}`,
  `StudioBackgroundResult::{OverlayPlanned, OverlayAssembled, OverlayCancelled}`,
  `studio_overlay_live_hold`, `studio_overlay_transfer_hold`.
- Control: `StudioControlAction::{BeginOverlaySave, PrepareOverlaySave, FinishOverlaySave}`,
  `StudioControlResponse::{OverlaySaveBasis, OverlayAcknowledged, OverlaySavePreparation,
  OverlaySaved}`, `StudioOverlayAcknowledgement::{LocalDraft, Handoff}`.
- Settlement: `StudioSettlementState::{LocalDraftRetained, LocalDraftHandedOff}`.
- Native (designed, **not registered**): `studio_overlay_begin`, `studio_overlay_save`, plus
  `transferState: "completed"` on the existing `studio_overlay_read` result.

**None of these exist yet.** A proposed name is not an implementation.

## Invariants this scope must not weaken

1. Prepared, whole Source, Completed, with actual evidence rechecked at every barrier. No durable
   signed prefix, no per-entry retirement, no bypass cache.
2. The complete original pending ledger survives handoff; only the overlay exclusion is removed.
3. HANDOFF-001 complete-target comparison before any completed acknowledgement, source lookup or
   sync reservation.
4. HANDOFF-002's authenticated inventory dependency **and**, separately, `check_handoff_references`
   (R9).
5. The Prepared source-replacement fence and the publication hold, including after restart.
6. First local acceptance derives from the actual Closing source, its matching saved signed close
   and independently observed tenure; ordinary failed Apply stays `NoEvidence`.
7. One shared four-slot `registry_catchup::preparation_pool()`, plus **I-2**: one live per-actor
   overlay admission, proved by a live `Arc` and never by a flag some path must clear.
8. **I-1**: first acceptance fully validates the branch; every later writer preserves the
   already-checked identity and evidence.
9. **I-3**: a job-owned reference hold is released only after ordinary conservative protection
   covers the same references.
10. **I-4**: `inventory_generation` rotates before any five-family durable mutation, including
    temporary siblings and unlinks, and before its first possible I/O.
11. Device and MLS secrets never leave the actor; a detached worker owns authenticated plaintext,
    public context, its permit, its admission token and any transient reference hold only.
12. Completed publication uses ordinary tail and page service; the initial Save window stays at two
    packets.
13. Every refusal, hold, cancellation and capacity failure retains the complete branch.

## Implementation progress

Branch `gate4-agent1-runtime`, based on `5a899c2`. Nothing is merged and no native command is
registered.

### C-1 structural decode, with C-2 and R4

Production changes:

| File | Change |
|---|---|
| `crates/catcoms-replication/src/studio/overlay.rs` | `StudioOverlay::{decode_vault, decode_vault_structural}` over a shared `decode_vault_inner(.., replay)`. The structural path keeps the bounds, version tag, nested seed and metadata bounds, target derivation, receipt and ledger scope equality, `checked_entries` and the canonical `encode_vault == bytes` comparison; it skips only `read`. |
| `crates/catcoms-replication/src/studio/overlay/handoff.rs` | The same pair on `StudioOverlayState`, threading `replay` to the nested branch decoder. |
| `crates/catcoms-app/src/store/epoch_intents.rs` | `EpochIntentState::{decode, decode_structural}`; `read_epoch_intent_record_structural`; `load_epoch_intents_structural`; `read_scoped_intent_plain` made visible in the store. `checked_epoch_replay_state` and `prepare_epoch_intent_with_io` now read structurally; `flush_checked_epoch_intents` takes only the physical size (C-2). |
| `crates/catcoms-app/src/store/epoch_intents/{overlay,retirement}.rs` | Retirement reads structurally; the overlay writer takes only the physical size (C-2). |
| `crates/catcoms-app/src/store/epoch_studio/handoff.rs` | `check_studio_handoff_write`, `check_studio_handoff_publication` and `check_studio_intent_link` read structurally. The commit's unchanged fence compares the complete authenticated plaintext digest and physical size instead of decoding and re-encoding (C-2); `persist_handoff_intents` takes only the size. |
| `crates/catcoms-app/src/store/epoch_recovery/inventory.rs` | The Intents arm decodes structurally. Reference collection still uses `base_blob_cids`, which keeps its typed seed work, and HANDOFF-002's dependency check is untouched. |
| `crates/catcoms-app/src/store/epoch_studio/recovery_disposition.rs`, `studio/control.rs` | Pending-count readers use the structural load. |
| `crates/catcoms-app/src/studio/replay.rs` | **R4**: `studio_replay_evidence` now excludes annotated ids from `own` explicitly, rather than relying on `choose` returning `NoEvidence` for them. Ordinary failed-Save `NoEvidence` is unchanged. |

New regressions, in `crates/catcoms-replication/src/studio/epoch/owner/tests/handoff.rs`:

- `studio_overlay_structural_decode_matches_full_decode_and_keeps_its_entry_checks`: for Index and
  Flipnote at 1 and 4 accepted entries, both decoders produce byte-identical canonical encodings and
  equal target, Prepared, Completed, floor, basis, author and projection; a ledger missing one
  annotated entry is refused by both; trailing and truncated records are refused.
- `studio_overlay_structural_decode_rejects_a_wrong_acceptance_sequence`: patches only the accepted
  entry's big-endian sequence inside the **branch** encoding, asserting first that the field really
  held 1 and that the length did not change.

### Executed evidence for C-1

| Check | Result |
|---|---|
| `cargo check -j 1 -p catcoms-replication --lib`, then `-p catcoms-app --lib` | Both clean. |
| `cargo test -j 1 --config profile.test.package.catcoms-replication.debug=0 -p catcoms-replication --lib studio` | **104 passed, 0 failed**, 63.22 s, before the new tests were added. |
| `... --lib studio_overlay_structural` | **2 passed, 0 failed**, 12.99 s. |
| M8 mutation, `entry.sequence != index as u64 + 1` weakened to `&& false` in the shared `checked_entries` | See below. |
| Restored source, rerun | Diff confirms only the C-1 additions remain; **2 passed, 0 failed**, 12.70 s. |
| `cargo test -j 1 --config profile.test.package.catcoms-app.debug=0 -p catcoms-app --lib studio_overlay -- --test-threads=1` | **37 passed, 0 failed, 2 ignored** (the opt-in profiles), 978.39 s. Log `logs/gate4-a1-c1-overlay.log`. Covers the overlay foundation and the whole handoff transaction, including the crash matrix, the replacement and publication fences, retirement, migration and the reference dependencies. |
| `cargo clippy -j 1 -p catcoms-replication -p catcoms-app --lib --tests -- -D warnings` | Clean, 37.67 s. |
| `cargo fmt --all -- --check` | Clean after reformatting two assertions in the new test. |

**The M8 mutation caught a defect in my own first test.** Against the mutant the sequence test
still passed, because it patched the last 88 bytes of the enclosing version-2 **state** record,
whose tail is the completed-acknowledgement section rather than the branch's entry. The record was
being rejected by an unrelated check, so the fixture proved nothing: exactly the masking class the
reviewer raised three times. The test now patches the **branch** encoding and asserts the field it
targets held 1 beforehand and that the length is unchanged. Against the mutant it then fails at its
intended assertion, "structural decode accepted sequence 2 for the first accepted entry"; after
byte-exact restoration it passes. This is why the guard-breaking step is not optional.

### C-4 transient reference holds

`crates/catcoms-app/src/store/creative_references.rs`: `Protection` gains a `transient` table of
job-owned holds keyed by a `Weak` owner. `unknown` and `install` leave it untouched and a complete
scan may subtract only from `pins`, so a reference no durable record names yet survives the scan
that would otherwise reclaim it. `ProtectedBlobs::delete` reaps dead owners and consults the
transient table before the installed set, under the same guard it already holds through unlink, so
a live hold protects even while durable protection is unknown. `hold_creative_transient` checks the
owner rail and the shared `MAX_CREATIVE_REFERENCES` rail **before** installing anything, and
refuses rather than marking the store unknown. `CreativeHold` releases on drop.

The existing accepted test
`transient_preholds_survive_budget_scans_but_only_complete_reference_scans_can_unpin` already
asserts the hazard this closes: after a complete scan that no durable record informed, the orphan
CID deletes. Two new regressions:

- `job_owned_transient_holds_survive_a_complete_scan_and_release_with_their_owner`: the CID
  survives two complete scans while held, an unrelated orphan still deletes, and the CID becomes
  deletable the moment the owner drops. That last assertion is the point: it is why I-3 must
  install the ordinary conservative holds before the owner can disappear.
- `transient_hold_exhaustion_refuses_admission_without_disturbing_existing_protection`: the owner
  rail refuses one too many, protection stays known, unrelated reclamation still works, and
  releasing one owner readmits exactly one.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-app --lib creative_references -- --test-threads=1` | **9 passed, 0 failed**, 3.34 s, including the seven pre-existing tests. |
| M13 mutation, `transient_holds(..) && false` in `ProtectedBlobs::delete` | Fails at "a live job-owned hold did not protect its reference"; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

### I-3, the protection transfer

C-4's seam now has its production consumer and the four `#[allow(dead_code)]` markers are gone.

`save_studio_closing_overlay_with_io` takes a job-owned `CreativeHold` over the operation's pixel
references immediately after the operation is decoded and before anything durable happens. The
guard lives to the end of that scope, so it is released only after the write attempt returns,
on success, on error and on unwinding alike. `write_studio_overlay_intent` already installed the
two ordinary conservative holds before its write; that call site is now documented as the second
half of the transfer, because it is what makes dropping the transient owner safe.

The ordering the accepted design requires, as implemented:

```
decode and bound the operation
  -> hold_creative_transient(operation pixels)      // job-owned, covers the in-flight window
  -> basis / exact-retry / eligibility checks
  -> hold_creative(base CIDs) + hold_creative_operation(new operation)   // the transfer
  -> write attempt
  -> drop CreativeHold                              // scope end, after the attempt returns
```

Two regressions in `store/epoch_studio/tests/rotation/overlay.rs`:

- `studio_overlay_acceptance_transfers_pixel_protection_before_its_write`: a complete reference
  scan runs **before** the acceptance and installs a known set excluding the new CID, so the
  assertion cannot pass through fail-closed unknown protection. After the Save succeeds and every
  owner is gone, and before any further scan, the CID is still retained, while an unreferenced
  orphan still reclaims and protection is still known. That control is what distinguishes a real
  transfer from a blanket refusal.
- `studio_overlay_uncertain_acceptance_still_protects_its_pixels`: two injected failures, one where
  the record really lands and only the caller's result is lost, and one that fails leaving a
  temporary sibling. Both leave the pixels protected, because the transfer runs before the write
  attempt and does not depend on its outcome.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-app --lib studio_overlay -- --test-threads=1` | **39 passed, 0 failed, 2 ignored**, 1237.73 s. Log `logs/gate4-a1-i3-overlay.log`. The 37 pre-existing overlay and handoff tests plus the two new ones. |
| `cargo test ... --lib creative_references` | **9 passed, 0 failed**, 3.26 s. |
| M14, removing only the two ordinary holds from `write_studio_overlay_intent` | Fails at "an accepted operation's pixels were reclaimable before the next scan". `git diff` confirms the restored file differs from `079e59a` only by the new comment, so the two calls are byte-identical. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean, with no `allow(dead_code)` remaining in `creative_references.rs`. |

### I3-001: media admission moved behind classification

The first placement of I-3's transient hold was wrong: it ran before `completed_retry` and
`exact_retry`, so an already accepted request could be refused by new-authoring reference
admission when the shared rails were full. That is the coupling AG1-001 closed, reintroduced.

The hold now sits after classification and only on the new-authoring path:

```
decode and bound the operation
  -> enter budget, read structural state
  -> completed_retry            -> acknowledge and return
  -> exact_retry                -> flush-only, no media admission
  -> ordinary-intent collision  -> refuse
  -> ONLY IF genuinely new authoring:
       operation_blob_cid, hold_creative_transient
       fresh Closing eligibility and basis
       ordinary I-3 holds, write attempt
  -> drop the transient owner after the attempt returns
```

The ordinary-collision check is hoisted to the same classification stage; the writer keeps its own
copy as defence in depth, which the reviewer explicitly permitted.

`studio_overlay_exact_retry_is_acknowledged_without_media_admission` accepts a pixel-bearing
operation, fills every job-owned hold slot with unrelated live work, proves a further hold is
refused, then retries the exact accepted request. It requires the retry to succeed, to return the
same accepted draft, to leave the durable records unchanged, and to leave the live hold count
unchanged. It calls the store directly rather than through the fixture helper so the failure names
this boundary instead of unwrapping. A `#[cfg(test)] live_transient_holds_for_test` counter makes
"took no hold" an observation rather than an inference from a successful result.

Moving the hold back above classification fails it at "an accepted retry was refused by media
admission: creative reference scan incomplete, unsupported or over bound"; the restored source
passes. The first attempt at this mutation surfaced as a generic `.unwrap()` inside the fixture
helper, which is why the retry now goes through the store API directly.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-app --lib studio_overlay -- --test-threads=1` | **40 passed, 0 failed, 2 ignored**, 995.49 s. Log `logs/gate4-a1-i3001-overlay.log`. |
| I3-001 mutation, taking the hold unconditionally | Fails at "an accepted retry was refused by media admission"; restored source passes. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

### Flow S staged seams, and N12(a)

`store/epoch_studio/overlay_capture.rs` adds the three stages the design specifies:

- `capture_studio_overlay_save` (custody): authenticates the bounded intent record **without
  decoding it**, captures the stamp (mount, numeric server, complete target, document, actor and
  key, designated owner, MLS epoch, and the record's authenticated plaintext digest with its
  physical size, absence explicit), and takes ownership of the caller's media hold.
- `StudioOverlayCapture::plan` (detached): the expensive stage. Full `EpochIntentState::decode`,
  which reconstructs any retained branch, then `append`, which replays it again with the new
  operation. It owns authenticated plaintext, public context, the basis and the hold; no store,
  Server, device key, MLS secret or writer, and it writes nothing. It repeats the
  ordinary-collision check rather than trusting a decision taken against bytes it cannot see.
- `commit_studio_overlay_save` (custody): re-mints the Closing basis from actual durable state and
  requires the same fingerprint, checks stamp equality, runs the I-3 transfer, then performs one
  accounted intent write. The hold is released only when it returns.

`save_studio_closing_overlay_with_io` now **composes those three inline**, so there is one
algorithm rather than a batch path that can drift from the scheduled one. The classification order
established by I3-001 is preserved exactly: completed retry, exact retry and ordinary collision all
precede media admission, and the exact-retry path returns before any capture.

| Check | Result |
|---|---|
| `cargo test ... --lib studio_overlay -- --test-threads=1`, after the refactor and before the new tests | **40 passed, 0 failed, 2 ignored**, 1123.67 s. Behaviour equivalence for the staged composition. Log `logs/gate4-a1-flows-overlay.log`. |
| `... --lib studio_overlay_detached` | **2 passed, 0 failed**, 22.52 s. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

**N12(a) is now real.** `studio_overlay_detached_acceptance_survives_a_complete_scan_between_capture_and_commit`
installs a known pin set excluding the new CID, captures, runs `plan` with custody genuinely
released, and then completes a full reference scan and attempts protected deletion **while the job
is detached**. The pixels survive; an unreferenced orphan still reclaims, so the scan is complete
and usable rather than fail-closed. After the commit, with every owner gone and before any further
scan, the ordinary holds keep them. Dropping the hold before the capture, so the detached job
carries none, fails the test at "a complete scan reclaimed pixels held by a detached acceptance".
That is a fixture-side check of the assertion's discriminating power, not an isolated production
mutation.

`studio_overlay_detached_plan_is_refused_when_the_record_changed` covers the other half: a plan
derived from superseded bytes is refused at the stamp comparison, records are unchanged and the
accepted branch is undisturbed.

**What this does and does not cover.** The transfer and its ordering are real and mutation-proven.
Those particular tests prove the transfer on the composed path; N12(a) above now proves the
detached window itself.

### C1-TEST-002: N24 and the unconditional-replay mutation

The revision-4 reviewer found that nothing proved the structural decoder actually skips the replay.
Every branch built through `append` is replayable by construction, so the equivalence and
wrong-sequence tests would both keep passing if `out.read(ledger)?` ran unconditionally: valid
branches replay successfully and malformed ones fail before the replay. The central behavioural
claim of C-1 was unprotected.

`studio_overlay_structural_decode_accepts_a_branch_the_full_decoder_cannot_replay` closes it. The
branch names a sound-effect operation the typed writer does not support. `checked_entries` never
decodes an operation body, so the record is completely consistent structurally: the entry is in the
ledger, authored by the basis author, with the exact envelope hash, sequence 1 and canonical
re-encoding. The fixture obtains a canonical accepted single-entry encoding for a supported
operation, then retargets that entry's id and envelope, which occupy fixed-width fields at offsets
4 and 40 of the 88-byte entry, so the record stays canonical. It asserts the unsupported operation
is genuinely unacceptable through `append`, and that the unmodified canonical record still replays,
so the refusal comes from the retargeted entry and not from the encoding.

Required outcome, proven:

| Decoder | Result |
|---|---|
| `decode_vault_structural` | succeeds |
| `read` and `decode_vault` | refuse |

**The mutation is part of C-1's required evidence, not an ordinary regression.** Replacing
`if replay { out.read(ledger)?; }` with an unconditional `out.read(ledger)?;` makes N24 fail at its
structural-success assertion, while the other two structural tests **still pass** exactly as the
reviewer predicted. Byte-exact restoration confirmed by `git diff` reporting no change against
`d67e649`; all three then pass.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-replication --lib studio` | **107 passed, 0 failed**, 98.36 s. |
| Unconditional-replay mutation | N24 fails at its intended assertion; the other two pass, confirming they cannot cover this boundary. Restored source: 3 passed. |
| `cargo clippy -j 1 -p catcoms-replication --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

### R4-TEST-001: N25 and M9

`studio_replay_evidence_excludes_accepted_overlay_ids_and_keeps_ordinary_own_intents` asserts on
the **actual `ReplayEvidence.own` set** produced by the production path, with the control the
reviewer specified: an accepted overlay id must be absent while a comparable unannotated own
intent, authored by the same device and pending in the same ledger, is present. The test first
establishes that both entries are pending and that only one is annotated, so the assertion is about
selection rather than about the ledger's contents.

The fixture builds the real state rather than a synthetic one: a founded `Server` with its own
store, an installed source filled with real large signed operations to reach the production
rotation threshold, a real seal, a real accepted Closing-overlay entry through
`save_studio_closing_overlay`, the source warmed through the existing detached preparation, the
pristine successor installed, and then an ordinary Apply on that successor for the control.

| Check | Result |
|---|---|
| `cargo test ... -p catcoms-app --lib studio_replay -- (default threads)` | **10 passed, 0 failed**, 17.88 s. |
| M9 mutation, removing only `&& !intents.is_overlay(id)` | Fails at "an accepted overlay id reached ordinary replay selection"; `git diff` confirms byte-exact restoration; restored suite passes. |
| `cargo clippy -j 1 -p catcoms-app --lib --tests -- -D warnings`, `cargo fmt --all -- --check` | Clean. |

One production file gained a **`#[cfg(test)]`** helper,
`ServerStore::install_sealed_studio_successor_for_test`. `rotate_studio_owner` refuses an
already-sealed source because it seals inside its own transaction, and the post-seal successor
install that the store's own rotation fixtures perform uses private items that `crate::studio`
cannot reach. The helper runs exactly that existing sequence. It is compiled out of production
builds, adds no callable surface and grants no authority; the alternative was widening real
production visibility for a test.

### Not yet done for C-1

The C-1 call-site table in design 5.1 is implemented but has no test asserting that no moved call
site needs a projection, and the app-side suites beyond `studio_overlay` have not run.

## Touched files

Design passes, on `Create-suite-2`: `docs/GATE4-AGENT-1-DESIGN.md`,
`docs/GATE4-AGENT-1-STATUS.md` only.

Implementation, on `gate4-agent1-runtime` only: the production and test files listed under
"Implementation progress". No shared contract document, no workflow, no native command registration
and no frontend file has been changed, and nothing is merged to `Create-suite-2`. The remaining
planned files are in design 5 and 15.

## Executed checks

| Command | Result |
|---|---|
| `git log --oneline`, `git status --short` | Revision 4 starts from `7efc9c2` (Agent 3's design), which contains revision 3 at `1bcb1bc`. Agent 1's two files were unmodified by `7efc9c2`; Agent 2's documents are present untracked. Revision 4 is uncommitted. |
| `git fetch origin Create-suite-2` | Revision 1 and 2 passes both found `origin/Create-suite-2` equal to the local head. |
| `grep` over `docs/GATE4-AGENT-3-DESIGN.md` sections 11 and 13.1 | Agent 3 accepts I-4, names its three affected writers, asks that `save_studio_source_checked`'s `handoff` parameter shape be preserved, and confirms no competing source writer or second pool. |
| `gh pr view 26 --json state,reviews,title,headRefName` | Open, `Create-suite-2`, `reviews: []`. Verdicts on this design were delivered outside GitHub's submitted-review endpoint, so its emptiness is not evidence that no review happened; `e65bfd8` is called unreviewed because HANDOVER and the core note agree, not because of that endpoint. |

Through the four design passes no Cargo command was executed. Implementation execution is recorded
under "Executed evidence for C-1" above; every run there used `-j 1` with the per-package test
debug override and no concurrent Cargo work, as the shared machine requires.

**No measurement exists yet.** Quoted performance numbers still come from the existing
[P1-PERFORMANCE](P1-PERFORMANCE.md) debug-profile observations. Design 13's eight measurements are
outstanding, including the C-1 before-and-after comparison that would quantify what the structural
decode actually saves: the code is in, the number is not.

## Proposed UI-hooks update (for Agent 4, not yet applicable)

Do **not** apply until design 12.3's prerequisites pass and the implementation checkpoint passes
review 1. Until then FLIPNOTE-UI-HOOKS must keep saying durable overlay Save is unavailable.

Under "Available now", after the existing `studio_overlay_read` block:

```ts
studio_overlay_begin({ server, channel, object? })
  -> { v: 1; kind: "eligible"; basis: string; accepted: number }
   | { v: 1; kind: "ineligible"; reason: string }

studio_overlay_save({ server, channel, object?, basis, nonce, body })
  -> { v: 1; kind: "local-draft"; channel: string; object: string | null;
       basis: string; accepted: number; alreadySaved: boolean }
   | { v: 1; kind: "acknowledged-handoff"; channel: string; object: string | null;
       basis: string; accepted: number; epoch: string; epochId: string };
```

Accompanying prose:

- `basis` is required and is the original authoring identifier: take it from
  `studio_overlay_begin` for a new branch or from `studio_overlay_read` for an existing one, and
  keep `(basis, nonce, body)` byte-stable across retries. It is an identifier, not authority.
- There is no timestamp field. The actor supplies it, and a retry with a fresh actor timestamp is
  still an exact retry.
- A request that matches no retained entry and whose `basis` is not the currently eligible one
  returns a stale error. It never becomes new work.
- `acknowledged-handoff` records that this exact request was previously transferred into the named
  local destination epoch. It is not delivery, receipt or settlement; after legitimate retirement it
  does not assert that those operations are still in the current signed source or the pending
  ledger, and it does not assert that the referenced frame pixels are still held locally.
- Retrying an already accepted save never requires the referenced PIX bytes to still exist. Saving a
  **new** frame operation does: publish the frame PIX first, and a missing blob refuses the save
  without changing anything.
- Neither result carries `content`. Refresh with `studio_overlay_read`.
- Save is available only for a Closing document whose exact expected checkpoint can be constructed
  from the actual source, its saved signed close and the observed owner tenure. Fault, an
  unavailable seed, unknown tenure and an unconfirmed preview refuse and keep unsaved editor work
  visible; a refusal is never a durable save.
- Automatic transfer happens in the background once a verified eligible successor is installed. It
  sends no packets of its own; peers receive the operations through ordinary paging. A large branch
  completes over several background turns while the app stays responsive.
- `studio_overlay_read` gains `transferState: "completed"`. `kind: "absent"` keeps its meaning.
- Both commands share the latest-view request fence with ordinary reads; run them sequentially for
  the same target. One overlay operation is live per server at a time, and capacity exhaustion,
  storage-reference capacity and an unstable vault inventory all return retryable errors.

## Next actions

1. Finish C-1's evidence: complete the `studio_overlay` app run, then the broader app Studio suite,
   then strict Clippy for both crates. C-1 touches `checked_epoch_replay_state`,
   `prepare_epoch_intent_with_io`, retirement, the inventory scan and replay selection, so the
   regression set is every Studio path, not only the overlay suite.
2. Build N24: a hand-assembled two-entry branch whose operations are individually valid on the base
   but invalid in sequence, so the structural decoder accepts it and the full decoder refuses. Until
   that exists, C-1's stated validation boundary is asserted by argument rather than by a test.
3. Add the R4 regression (N25): assert annotated ids are absent from `studio_replay_evidence`'s
   `own` set, and the matching M9 mutation.
4. Then C-4, then the runtime, then I-4 with C-3. See the revised sequencing table above.
5. Produce design 13's measurements as each item lands; C-1's before-and-after is the first one and
   is cheap, since the opt-in profile already exists.
6. Confirm with Agent 2 the prerequisites P1 to P5, the two-hold contract (design 12.1) and the copy
   requirements (design 12.2); give Agent 4 the central edit list in design 15. Agent 3's side of
   the I-4 coordination is already on record at `7efc9c2`, and their "unaffected if I-4 does not
   land" clause is an integration alternative, not an opt-out from a deployed cursor's discipline.
7. Keep native Save unregistered and out of FLIPNOTE-UI-HOOKS until Agent 2's manual lifecycle
   passes its own review and their status note says so.
8. Do not send the design 18.3 implementation review until the scope it names has real evidence. A
   partial branch is not a checkpoint.
