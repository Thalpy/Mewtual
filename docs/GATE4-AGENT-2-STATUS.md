# Gate 4 Agent 2 status

Owner: Agent 2, [overlay lifecycle, provisional local work and repeated tenure](GATE4-AGENT-HANDOFFS.md#agent-2-overlay-lifecycle-provisional-local-work-and-repeated-tenure).
Design of record: [GATE4-AGENT-2-DESIGN.md](GATE4-AGENT-2-DESIGN.md).
Review preamble: [preamble 2](GATE4-REVIEW-PREAMBLES.md#review-2-manualprovisional-overlay-lifecycle-and-repeated-tenure).

## Completion matrix, 2026-10-02

**This table is the current state.** Every section below it is a dated record; where one disagrees
with this table, this table wins. Built against the six goals of the assignment and the
whole-scope review of `510d0b54` (verdict: CHANGES REQUIRED, Agent 2 not complete).

| Goal | State | Evidence, and what is still missing | Owner |
|---|---|---|---|
| 1. Bounded inspection, export, copy, disposition | Implemented, not natively exposed | Two-visit inspect/export/archive/copy, D1-D6 disposal, release. Export, archive and copy preview now keep the job's permit and the actor's delivery fence through native conversion (`3168de6e`); an archive whose result cannot be delivered reports `outcome=uncertain`. The nine commands are unregistered (`510d0b54`). | Agent 2; registration Agent 4 |
| 2. Lossless work and PIX across refusal and restart | Implemented except one platform barrier | Archive codec, binding, release, collector, evidence-before-removal ordering (`6bd76c13`). **Open:** `sync_directory` is a no-op on `not(unix)`, so on Windows a preserving disposal cannot establish the archive's directory entry. | Persistence owner via Agent 4; Agent 2 shows disposal honours the result |
| 3. Recoverable stale, rewound and non-pristine branches, with actionable states (P2) | Classification implemented; first review fixed | `StudioOverlayEligibility` / `StudioOverlayManualReason`. Production classifies from each source record's HEADER (`overlay_successor_hold_in_vault`, `vault_holds_work`) and never restores, pinned by a zero-restore assertion; the structural hold, the full hold and `check_overlay_successor` agree on every fixture state. The store adds provenance, author, missing/unreadable source, an Index entry's missing Flipnote (`objectMissing`, agreeing with the real handoff in both directions), tenure and the live receipt owner. Lifecycle and inspection carry `eligibility` and `manualReason`. Agent 1's `StudioOverlayHold` was designed but never built; its runtime still refuses with strings. | Agent 2; Agent 1 maps its runtime refusals |
| 4. Durable local work on an awaiting-tenure preview | **Not implemented** (first slice built) | Design section 8 accepted. Built (replication and sync): 8.1 parts 1-2, part 3's re-parse at the mint (its detached-plan re-parse is app-side and unbuilt), and the reviewed mint design.
<br>- The exact seed bytes are retained and exposed only inside the scoped callback. This costs up to 2 MiB more per ready preview, and the true per-slot worst case is unmeasured.
<br>- `StudioUnconfirmedOverlayBasis` is minted only by `ChannelSync::mint_unconfirmed_overlay_basis` inside the live hint, which requires a complete tail and re-parses the retained seed against its receipt.
<br>- The fingerprint is domain-separated by provenance; the kind is not persisted, so a reload restores it from the record.
<br>- `new_admitted` refuses a provenance that disagrees with its basis.
<br>- `clippy.toml` and a source-scan test pin both hidden constructors.
<br>Missing, all app-side: "no installed source" under custody, the S3 re-entry, the 8.3 rails and the 8.7 save path. Both wait on Agent 1's structural decode exposing provenance and charged bytes, and on its provenance-parameterized Flow S. Also missing: 8.6 reconciliation, restart reconstruction through the store, and native results. Its absence is a Gate 4 gap, not a deferral. | Agent 2; Agent 1 for the two hand-offs |
| 5. Repeated owner tenure, rejoin, newcomers, legitimate progress | Implemented at the sync and receipt layer; not yet through the app actor | Leaf-aware tenure, v1 import, app seam, CORE-005 archived witness (`066a6533`), M-1 on the receive path with its own error (`0335262e`). **The product never rejoins with the same identity**: join and found both mint a fresh `MlsDevice`, so a returning owner is a new `DeviceId`. `returning::a_removed_owner_returns_as_a_new_device_with_a_new_tenure_everywhere` drives that form with real MLS and real receipts: A' lands in the vacated leaf, A' and every witness agree on the new start across restart, A' can author, its receipt verifies on witnesses and a newcomer, and A's first-tenure receipt and A's old key claiming the new tenure are refused everywhere. **Missing:** the same flow through the actor and a real Studio rotation. | Agent 2 |
| 6. Truthful native results and events | Partial | Results and settlement notices exist; no command beyond `studio_overlay_read` is registered; UI-hooks rows not applied. | Agent 2 contract; Agent 4 registration and rows |

| Prerequisite | State |
|---|---|
| P1 reviewed manual lifecycle | Implemented; review at `510d0b54` returned CHANGES REQUIRED, items above |
| P2 hold mapping | Implemented. Agent 1's two questions answered in `f158c17b` (`TenureImported`, `PreparedStuck`). Its re-review found no blocker and two mediums, both fixed in the next commit: a Prepared branch with Absent evidence now runs every active check H1 runs after returning it to active, and the tenure copy no longer claims waiting cures one reason but not the other (both end at the next observed owner transition). |
| P3 native results and events | Partial (goal 6) |
| P4 live-tenure contract | Implemented: `StudioOwnerTenure`, `require_owner_tenure`, CORE-005 witness |
| **P5** | **FALSE.** Native Save stays unregistered. |

The `510d0b54` review's evidence gaps are closed in `0335262e`: D4 against an earlier generation's
archive of identical work, D1 for a removed real author, the copy object probe at C3 and C4, and
M-1 on the receive path. Each was confirmed by disabling its guard.

## Current state (historical, superseded by the matrix above)

| Item | State |
|---|---|
| Design revision | 7. **Design ACCEPTED: adversarial PASS for (a), (b) and (c), no findings.** Revision 7 adds only two non-blocking refinements offered with the PASS. |
| Design verdict | PASS at `a6d8170f6ab1f0d46287808fc8051ac2a387521c`, 2026-09-16. (b) passed at revision 4, (a) and (c) at revision 6. **Design only**: no implementation, test, measurement or native exposure is accepted. |
| Original design base | `1bcb1bca204d721b848b17c0835faf931ae930e3` |
| Revision 1 head, reviewed | `a901f6b0f64df2b4ea9cc0221b64ac98276f582d` |
| Revision 2 head, reviewed as SEC-PAIR-001 | `21ca8fa93c07b8bb65a00bc0bbc555518f8a7132` |
| Revision 3 head, reviewed | `909720739b6c0a455d37761e8149d7eec21cb6f4` |
| Revision 4 base | `909720739b6c0a455d37761e8149d7eec21cb6f4` |
| Revision 4 head, reviewed | `37fa87753d32a2c4f5d1172cc865910a93d90fa1`. **Boundary (b) PASS.** |
| Revision 5 base | `37fa87753d32a2c4f5d1172cc865910a93d90fa1` |
| Revision 5 head, reviewed | `f2257b018d396a835529742c40d4b282bbc127d9` |
| Revision 6 base | `f2257b018d396a835529742c40d4b282bbc127d9` |
| Revision 6 head, **accepted** | `a6d8170f6ab1f0d46287808fc8051ac2a387521c` |
| Revision 7 head SHA | `7d98baf`. Refinements only; no reviewed decision changes. |
| Working checkout | `gate4-agent1-runtime`, main repository tree, by the user's decision that everything goes on one branch for now. **No separate worktree**: the shared `target` is ~138 GB and a second one was judged unaffordable at the time. What replaced it is a private `CARGO_TARGET_DIR` at `M:/catcoms-agent2-target`, taken because the other agents' test binaries hold `catcoms_app-*.exe` open and fail the link. Agent 4 should expect to move these two documents at integration. |
| Production code | **Written, for the archive family only.** The payload codec, the reference collector, the record writer with its accounting and sub-cap, the `archive_bytes` tally, and the release path. Nothing else in this scope exists. |
| Tests added | 26 in `epoch_studio::tests::rotation::overlay::archive`, 10 in `catcoms-replication`'s archive module, 2 in `epoch_intents::retirement`. |
| Cargo commands executed | Yes; see the mutation ledgers below. **Every run used `RUST_MIN_STACK=33554432`**, a workaround for a stack regression at HEAD and not a clean default-stack result. |
| Measurements | Still **none**. Every number in the design remains an existing bound read from source. The mutation ledgers are executed evidence, not measurements. |
| CI status | **None.** No reviewed commit has an attached GitHub check; all pass/fail evidence in this document is reported local evidence. |
| Native commands registered | **None by Agent 2.** `studio_overlay_read` remains the only registered overlay command, unchanged. |

**This table was stale and Agent 1 caught it.** It said production code: none, tests: none, cargo
commands: none, while `0287910`, `6e1551c`, `47bad73`, `2dba8fa`, `28bb73d` and `466a372` were all
already committed. It had not been revised since the design was accepted, because the implementation
progress was being appended further down the document instead. Two rows are worth separating
carefully, because they are different claims and only one was wrong:

- The **P1 to P4 rows** below saying "designed, accepted, unimplemented" are **correct**. The
  manual lifecycle, the hold-variant mapping, the native results and the tenure contract are not
  the archive plumbing, and none of them exists.
- **P5 remains FALSE.** Correcting this table is not permission to promote it. `studio_overlay_save`
  stays unregistered until the required implementation reviews are complete and this document says
  so explicitly.

## CORE-005 archived Observed-tenure witness, 2026-10-01

Agent 3's accepted CORE-005 contract ([authority follow-up](GATE4-AGENT-3-AUTHORITY-FOLLOWUP.md),
"Chosen mechanism and Agent 2 dependency") needed a seam extension in `catcoms-sync`, which is this
scope's. `94af29df` was a rejoin test, not that seam; Agent 3 was right to keep N17 blocked on it.

**Implemented (sync half):**

- `ArchivedOwnerTenure`: owner key, Observed start, `tenure_id(group id, key, start)` and
  retirement epoch. No public constructor, no wire form, no import path. One per group.
- Minted only in `OwnerTenure::applied`, when a contiguous step derives a new tenure and the one it
  ends was held as `Observed` at exactly the pre-step position. Imported, Unknown, gap, stale-position
  and same-owner steps neither mint nor replace. The departing key is captured in `Position` before
  the MLS call, because a Remove takes it out of the group.
- Persisted in the owner-tenure tail of the same sync snapshot, so it is atomic with MLS and current
  tenure. v3 is written only when a witness exists; every other state keeps its exact v2 bytes.
  Restore checks framing, the derived id (binding group, key and start) and
  `start < retired_at <= epoch`, and refuses the whole tail on failure. Legacy records have none.
- Reached only through `with_durable_owner_history`, under a `DurableOwnerSnapshot` that captures
  the witness with the successful save and is current only while the witness is unchanged.

**Not done here:** the app-side admission attestation, report admission, N49 and N50 as production
consumer tests. Those are Agent 3's. `retired_at` is range-checked but not bound by the derived id,
so an in-range corruption of that field is caught only by the vault's authentication of the whole
snapshot.

## Agent 1's registration prerequisites (its section 12.3)

**Authoritative statement: P5 is FALSE. `studio_overlay_save` must not be registered.**

*Table updated 2026-09-30 at `bbef5908`, in answer to Agent 1's question of whether it had gone
stale. It had: the table still read "no line of production code has been written for any of them",
which stopped being true at slice 6. It is now per-row, and no row has reached true.*

| Prerequisite | State | Where |
|---|---|---|
| P1 reviewed manual lifecycle: inspect, export, copy-into-current, explicit disposition, lossless across restart and refusal | **All four verbs implemented; NOT yet reviewed as a lifecycle.** Inspect pre-dates this scope; export, copy-into-current and explicit disposition are built end to end with the archive that makes the preserving arm reachable. What is missing for P1 is not code: "lossless across restart and refusal" is proven for disposal, archive and release by their own tests, and **not** proven for copy across a restart. The fable review of the copy work has not returned | design 6.1-6.6, 12 |
| P2 every `StudioOverlayHold` variant mapped to a user-visible actionable state | **Design accepted**, unimplemented | design 7, 11 |
| P3 truthful native results, events and UI-hooks rows | **Partially implemented.** Four of nine commands are registered and their views are truthful about provisionality, non-authority and required reconciliation. The UI-hooks rows are not updated and no row is published as available | design 11 |
| P4 live-tenure contract, over `verification_owner_tenure_start()` and `authoring_owner_tenure_start()` | **Substrate implemented; the contract is partly implemented and unverified.** Slice 7 built the mechanism. **CORRECTION, 2026-09-30:** an earlier version of this row said "no app authoring call site consults either accessor". That was wrong and I told Agent 1 so. **Nine non-test app call sites consult `authoring_owner_tenure_start()`** - three in `studio/overlay.rs`, four in `studio/receiver/handoff.rs`, two in `studio/receiver.rs` - and they follow A-1 exactly: the wrapper reads the `Option<u64>` and passes it through unchanged for the store to refuse at the stage that needs it. What is genuinely missing is the app seam V5 and V7 name: **`StudioOwnerTenure`, `Server::observed_owner_tenure()` and `Server::require_observed_owner_tenure()` do not exist**, so Agent 3 has nothing to take, and V7's app-boundary conversion has no anchor. V1's coverage of all nine named stages and V8's reachability are also unverified by me. N-T7b is undischarged | design 9.4 V1-V8, 9.3 part 5 A-1 |
| P5 explicit statement that P1-P4 are implemented and reviewed | **No** | this table |

This row is the single authoritative source for P5. It changes only after implementation exists and
review 2 returns PASS for the corresponding boundary.

**To Agent 1's framing directly: the landed slices are prerequisites, not the prerequisites.** Slice
6 implements one of P1's four verbs; slice 7 implements the substrate P4's contract will be written
over, not the contract. Reasoning from either row as FALSE remains correct today.

## Review history

| Date | Item | Verdict |
|---|---|---|
| 2026-09-15 | Design revision 1 (`a901f6b`) | **CHANGES REQUIRED on all three boundaries**, nine findings: 3 High, 6 Medium. Reviewer ran no Cargo commands and did not complete the `catcoms-sync/src/lib.rs` constructor/restore call-site trace. |
| 2026-09-15 | Design revision 2 (`21ca8fa`) | **CHANGES REQUIRED on all three boundaries**, as SEC-PAIR-001, five findings: 3 High, 2 Medium. Revision-1 findings 1, 2, 4, 5, 6 and 8 closed; 9 closed at the API level with a test-layer correction; 3 and 7 open in narrower forms. Reviewer ran no Cargo commands. |
| 2026-09-16 | Design revision 3 (`9097207`) | **CHANGES REQUIRED on all three boundaries**, two findings: 1 High, 1 Medium. All five SEC-PAIR-001 corrections accepted. Reviewer ran no Cargo commands. |
| 2026-09-16 | Design revision 4 (`37fa877`) | **(b) PASS.** (a) and (c) CHANGES REQUIRED for one Medium test and mutation gap. The revision-3 High tenure finding is closed at the design level. Reviewer ran no Cargo commands. |
| 2026-09-16 | Design revision 5 (`f2257b0`) | **(b) PASS remains.** (a) and (c) CHANGES REQUIRED for one new Medium finding introduced by revision 5's own accessor-removal hardening. The revision-4 finding is closed. Reviewer ran no Cargo commands. |
| 2026-09-16 | Design revision 6 (`a6d8170`) | **PASS for (a) and (c), no findings.** With (b)'s revision-4 PASS this accepts the whole design. Every finding from revisions 1 to 6 is closed at the design boundary. Two non-blocking refinements were offered and are adopted in revision 7. Reviewer ran no Cargo commands. |
| 2026-09-16 | Design revision 7 | Accepted refinements only: A-1's scope sentence and N-T7b's diagnostics. No reviewed decision changes. |

### Revision-5 re-review findings and their disposition

| # | Sev | Finding | Disposition |
|---|---|---|---|
| 1 | Med | The accessor-removal table moved tenure refusal ahead of legitimate retry and recovery paths: `save_studio_closing_overlay` and `handoff_studio_overlay` were classified as pure authoring, although their accepted store ordering acknowledges and recovers before requiring a tenure | **Corrected as invariant A-1**, generalised from the reviewer's per-wrapper table: an app wrapper reads `authoring_owner_tenure_start()` and passes the `Option<u64>` through; the store owns every refusal at the stage that needs it. The committed orderings are cited by line and verified, not taken from prose. V1 refined to "new authoring", V8 states the complementary reachability, N-T7b and M24c make it executable. |

### Revision-4 re-review findings and their disposition

| # | Sev | Finding | Disposition |
|---|---|---|---|
| 1 | Med | `Imported` was not mutation-isolated at the app authoring seam: N-T7 and M24 covered only `Unknown`, so an implementation could satisfy the sync-layer tests while mapping `Imported(S)` to `Known(S)` one layer up | **Corrected.** N-T7 leaves the retained set and runs the V1 matrix for both fail-closed values from a genuinely migrated fixture, asserting verification stays unaffected in the same run; M24 narrows to `Unknown`; M22g and M24b separately anchor the two halves of the app-boundary invariant; V7 states it. |
| n/a | note | `observed_owner_tenure_start`'s "independently observed" contract would become false for `Imported` | **Adopted and taken further:** the accessor is removed rather than repointed, so the compiler enumerates every call site; `verification_owner_tenure_start` and `authoring_owner_tenure_start` replace it with a per-site mapping recorded in design 9.3 part 5. |
| n/a | note | P4 cited V1-V5 | Corrected to V1-V7 in both documents. |
| n/a | decision | 16.1 product decision | **Adopted as (a)**, over this design's recommendation of (b): `Imported` ships fail-closed with no operator-adoption override, because (b) would create an authority-bearing escape hatch that defeats V6. |

### Revision-3 re-review findings and their disposition

| # | Sev | Finding | Disposition |
|---|---|---|---|
| 1 | High | v1 tenure snapshots could promote historically ambiguous tenure into `Known`: a stale `start` written by the old preserve branch, with the live leaf digest grafted on | **Corrected.** `start == Some(epoch)` proved to be exactly the promotable set; a bare downgrade of the rest rejected as worse than the risk; the value split by consumer through a new `Imported(u64)` state, sound for verification and fail-closed for authoring, with `prepare_receipt_head_snapshot` moved to a new authoring accessor and a snapshot flag preventing save/reload laundering (N-T6d, M22e, M22f). The residual product decision is question 16.1. |
| 2 | Med | Revision 3 still contained, in O2, the archive placement it declares impossible, plus two stale cross-references | **Corrected.** O2 rewritten so the design holds one archive placement; Agent 1's design recorded as user PASS; the pre-`Unmatched` shorthand removed from this note. |

### SEC-PAIR-001 findings and their disposition

| # | Sev | Finding | Disposition |
|---|---|---|---|
| 1 | High | The generation scheme had no legal first-acceptance state: every unknown identity returned `Stale`, including the legitimate first Save of the next generation | **Corrected.** `classify_request` is structural and basis-free and returns `Unmatched`, which is not a verdict; `admit_new_branch` resolves it into `New` or `Stale` at the authorizing stage against the derived expected-next identity. No new stage, no reserved generation, AG1-001 preserved (N17a, M10b, M10c). |
| 2 | High | The archive had no unambiguous physical identity in the Intents inventory | **Corrected, adopting the 16.1 answer.** `EpochRecordKind::DraftArchive`: distinct physical family, Intents accounting class, gated by `includes_intents()` (N19b, M28). Revision 2's placement is withdrawn as unrepresentable; audit fact A6 records why. |
| 3 | High | The archive's declared bound could not contain its own maximum payload | **Corrected.** Three constants derived from the actual field bounds, threaded through reader cap, family `sealed_cap`, authentication rail, budget, sub-cap and native bound, with a static assertion and maximal-shape tests (N19c, L3b). |
| 4 | High | The same-commit tenure residual was blocked only in the local commit builder | **Corrected.** M-1 moves to the receive side, into the existing pre-merge staged-commit inspection, stated over `DeviceId`. The invite ledger is demoted to honest-join admission (N-T6c, M22d). |
| 5 | Med | N14/M5 could not test a missing or wrong confirmation at the store layer | **Corrected.** Split by layer: N14n/M5n at the native adapter, N14/M5 at the store. |

### Revision 1 findings and their disposition

| # | Sev | Finding | Disposition |
|---|---|---|---|
| 1 | High | `Copied` disposal has no valid terminal representation: the mode required `copy`, `copy` was forbidden without `active`, and the transition cleared both | **Corrected by removing the cause.** Copy bookkeeping deleted from the durable record. The terminal manifest is self-contained and no rule of it refers to a live field. Positive encode/decode/reopen cases added for both modes (N11, N13), plus M1b, which makes the defect executable. |
| 2 | High | The copy proof could account for the wrong source work; projection-level copying is not envelope-level preservation | **Corrected twice.** Copy is no longer a disposal precondition, and `source_entry` is deleted: `restore::plan` derives `source_ops`. Preservation moves to a lossless draft archive. C-P states the loss plainly. |
| 3 | High | Repeated disposal erased the only defence against an old Save retry | **Corrected by a durable branch-generation namespace.** `branch_id` includes a monotonic generation. The precise rule is invariant I-I below, as corrected in revision 3: an unmatched identity is Stale **unless** it is exactly the derived next generation under fresh live authority (N17b, M3b, M10b). |
| 4 | Med | The unchanged inspection capture cannot supply copy's destination inputs | **Corrected.** Composite capture of both destination records under the same single permit, rechecked at preview and apply (N8b). The "only the rebuild function changes" claim is withdrawn. |
| 5 | Med | A structurally valid, non-replayable branch had no lossless export path | **Corrected.** Export and the archive are structural, not reconstruction-dependent, so a preserving disposal works for such a branch (N22). |
| 6 | Med | The preview mint had no way to obtain its seed-only bytes | **Corrected.** Retained verified seed bytes, scoped access, and a detached re-parse so the mint does not trust the retention. Memory accounted (L10). |
| 7 | High | L5's safety argument overlooked Unknown-tenure readers | **Corrected; the argument is withdrawn.** Agreement is now structural: `Position` gains the committer leaf identity and `applied` gains a discontinuity arm, plus a membership rule for the invisible residual (N-T6b, M22b, M22c). |
| 8 | Med | The native storage-refusal rule was false after a partial copy | **Corrected.** The partial-copy state no longer exists; a three-state outcome covers the sequence that does. |
| 9 | Med | Discard confirmation was required but absent from the schema | **Corrected.** A required typed token in both the Rust request and the native argument list. |
| n/a | n/a | Reviewer's N19 precision point on reference retention | **Accepted.** R3 and N19 rewritten: the archive retains the references, a destination copy does not. |

## Proposed API seams

Full signatures are in design section 5. Changes against revision 1 are marked.

| Seam | Kind | Consumer |
|---|---|---|
| `StudioOverlayProvenance` on the basis | core | Agents 1, 3 |
| **`branch_generation`, `branch_id`, `classify_request` returning `Unmatched`, and `admit_new_branch`** | core | Agent 1's Save classification at S1 and its authorizing stage at S1b |
| **`EpochRecordKind::DraftArchive`** as a distinct physical family in the Intents accounting class | store, **touches every match on that enum** | Agents 1, 3, 4 |
| **M-1 in `ServerGroup::process_incoming`'s pre-merge inspection** and in the local commit builder | mls, **authority-bearing, every member's receive path** | boundary (c) |
| v3 terminal `disposed` arm, `dispose`, extended `validate` (**no `copy` arm**, finding 1) | core | Agent 1, Agent 3 |
| Provenance guard on `prepare_handoff*` | core | Agent 1 |
| **`UnconfirmedStudioSeed::seed_bytes` and `ProvisionalStudioSeedUse.seed_bytes`** (new, finding 6) | core, sync | boundary (b) |
| **`ServerGroup::designated_committer_leaf()`** and the commit-builder remove-and-re-add refusal (new, finding 7) | mls, **authority-bearing** | boundary (c) |
| `Position.leaf`, the `applied` discontinuity arm, `OwnerTenure::joined`, the versioned snapshot tail | sync, **authority-bearing** | boundary (c) |
| `ServerStore::studio_overlay_lifecycle` | store, structural only | Agents 1, 4 |
| **`capture_studio_overlay_copy`, `studio_destination_is_current`** (new, finding 4) | store | Agent 2 |
| **Draft archive record: writer, reader, release, Intents-arm accounting and reference collection** (new) | store, **shared inventory work** | Agents 1, 3 |
| `dispose_studio_overlay_with_io` | store | Agent 2 only |
| `StudioInspectionPurpose` / `rebuild_for` (now structural for `Archive`) | store | Agent 1 |
| `restore::plan(.., &[&StudioProjection], .., PlanScope)` returning **`source_ops`** | app | Agent 2 |
| New `StudioControlAction` and response variants, now nine native commands | app, **central enum edit** | Agents 1, 3, 4 |
| `StudioSettlementState::{LocalDraftManual, LocalDraftDisposed}` | app, **shared enum** | Agent 1 |
| `StudioOwnerTenure`, `observed_owner_tenure`, `require_observed_owner_tenure` | app | Agents 1, 3 |
| Wider visibility for `ServerStore::read_studio_record` | store | Agent 3 |

## Invariants this scope owns

- **I-A.** Read-only operations change no durable byte.
- **I-B.** An annotated ledger entry leaves the ledger only through the explicit disposal
  transaction. Receipt-covered retirement keeps its overlay filter and stays incapable of removing
  one.
- **I-C.** Disposal writes its terminal manifest and removes the named entries in one sealed,
  accounted, atomic replacement. There is no state in which the entries are gone without their
  evidence. For a preserving disposal the archive is durable before that transaction begins.
- **I-C2 (new, finding 1).** The terminal manifest is self-contained: no field of it refers to
  `active`, `prepared` or any live field, so a disposed record is valid, re-encodes canonically and
  reopens with no branch present.
- **I-D.** An `Unconfirmed` branch can never reach handoff preparation, a signed source, a receipt,
  a tenure, publication, settlement, receipt-covered retirement, replay evidence or ordinary Apply.
  The guard is in core, not only in the app.
- **I-E.** Preview expiry, eviction, replacement, unwatch, lock, remount, membership change and
  restart remove the live preview and never the durably accepted draft; a retained draft never
  revives a preview.
- **I-F.** `StudioOwnerTenure::Unknown` **and `Imported`** are both fail-closed for every authoring,
  signing, repair-issuance, rotation and publication decision. A reused key, a Welcome, a hint, a
  candidate receipt's claim, a fresh owner proof's claim and the current group epoch are each
  insufficient to make it `Known`.
- **I-N (new, revision-5 finding 1).** Fail-closed means new authoring is refused, not that every
  path is refused. An app wrapper reads the authoring accessor and passes the `Option` through; the
  store refuses at the stage that needs a tenure. Under `Imported` and `Unknown`, an exact accepted
  Save retry, a completed-handoff acknowledgement and resolution of an already durable `Prepared`
  handoff all stay reachable. Without this, an indefinitely `Imported` single-owner server would
  turn a crash during Prepared into permanent data limbo.
- **I-M (revision-4 finding 1).** The app-level tenure conversion is lossy in one direction
  only: `Imported` never becomes `Known` at any layer, and `require_observed_owner_tenure()`
  succeeds for `Known` alone. The conversion and the accessor are separately anchored mutations,
  because the sync-layer tests cannot see a value laundered one layer up.
- **I-L (revision-3 finding 1).** A v1 tenure snapshot is promoted to leaf-aware `Observed`
  only when `start == Some(epoch)`. Any other `start` becomes `Imported`: still reported to
  verification, so a mismatched proof is still refused, and never reported to authoring. `Imported`
  survives save and reload as `Imported` and is promoted only by a leaf-aware observed transition.
- **I-G.** A returning owner in a new tenure observes a strictly different value from its earlier
  tenure, including across a same-commit membership discontinuity.
- **I-H (new, finding 2).** No count of copied items, and no `source_ops` value, ever establishes
  that a branch was preserved. Only a durable archive whose `content`, `branch`, `generation` and
  entry list match does.
- **I-I.** A request naming a branch identity the record does not know is refused as Stale unless it
  is exactly the derived next generation under freshly minted live authority, in which case it is a
  first acceptance. Forgetting acknowledgement evidence degrades to refusal, never to acceptance.
- **I-K (new, SEC-PAIR-001 finding 4).** No member merges a commit that both removes the pre-commit
  designated committer and adds the same `DeviceId`. The rule binds the receive path, not only the
  builder, and does not depend on the invite ledger, which no other member holds.
- **I-J (new, finding 6).** No durable unconfirmed branch is built from retained preview bytes
  without a detached re-parse that re-proves the receipt binding from first principles.

## Files this scope expects to touch

Leaves owned by Agent 2 (new): `catcoms-replication/src/studio/overlay/disposal.rs`,
`catcoms-app/src/store/epoch_intents/{disposal,archive}.rs`,
`catcoms-app/src/studio/lifecycle.rs`, `catcoms-app/src/studio/overlay/copy.rs`,
`apps/desktop/src-tauri/src/studio/overlay.rs`, plus test modules and
`.github/scripts/check-studio-overlay-lifecycle-mutations.py`.

Shared files, coordinated with Agent 4 before editing:
`catcoms-replication/src/studio/overlay.rs`, `overlay/handoff.rs`, `studio/provisional.rs`;
`catcoms-sync/src/registry_seed/provisional/seed.rs`, `owner_tenure.rs`, `lib.rs`;
`catcoms-mls/src/group.rs`;
`catcoms-app/src/store/epoch_intents.rs`, `.../epoch_intents/{inspection,retirement}.rs`,
`.../store/epoch_studio.rs`, `.../store/epoch_recovery/inventory.rs`;
`catcoms-app/src/studio/{restore,control,dispatch,inspection,settlement}.rs`;
`apps/desktop/src-tauri/src/{lib.rs,studio.rs}`.

Highest-risk shared items, in order: the inventory arm's archive record kind and its reference
collection; `catcoms-mls`'s leaf accessor and commit-builder rule; the `owner_tenure` snapshot
format change.

Documents owned by Agent 4 and **not** edited by this scope: `INTERFACES.md`,
`BACKEND-IMPLEMENTATION.md`, `HANDOVER.md`, `design-creative-suite.md`, `FLIPNOTE-UI-HOOKS.md`,
`GATE4-ACCEPTANCE.md`.

## Proposed UI-hooks update

Exactly design section 11: the extended `OverlayInspection` type with `branch`, `content`,
`generation`, `provenance`, `eligibility`, `manualReason`, `unconfirmedState`, `replayable` and
`archived`; the new `disposed` kind with `mode: "preserved" | "discarded"`; the nine-command table;
the three-state write outcome; the truthfulness rules, including that a copy count is never a
preservation claim; and the two new `SettlementState` values `localDraftManual` and
`localDraftDisposed`. Agent 4 applies it. No row may be published as available before the
corresponding command is registered.

## Implementation progress

Implementation started 2026-09-16, after the design PASS and once Agent 1's `DraftArchive` seam
landed. Everything on `gate4-agent1-runtime`.

### Slice 1: the draft archive payload codec (core)

| Item | State |
|---|---|
| `crates/catcoms-replication/src/studio/overlay/archive.rs` | New. `StudioDraftArchive`, `StudioOverlayProvenance`, `MAX_STUDIO_DRAFT_ARCHIVE_BYTES`. |
| Built from a **structural** branch plus its ledger | Yes: `from_branch` uses `checked_entries` and the ledger's envelopes, so a non-replayable branch archives exactly as well as a replayable one (design 6.5, finding 5 of SEC-PAIR-001). |
| `replayable` | A recorded label, not a gate. |
| Bound | `MAX_STUDIO_DRAFT_ARCHIVE_BYTES` derived in replication from the field maxima, which is the correct layer: the payload schema owns its own bound. Agent 1's app-side `MAX_DRAFT_ARCHIVE_PAYLOAD_BYTES` currently re-derives the same value independently. **Open item for slice 2:** tie them with a static assertion rather than leaving two derivations, and hand the collapse to Agent 4. |
| Tests | 6, in `studio/epoch/owner/tests/archive.rs`, reusing the real settlement `Fixture`. A synthetic basis would have needed a test-only constructor on `StudioClosingOverlayBasis`, which the overlay design forbids. |
| Evidence | `cargo test -j 1 -p catcoms-replication`: 212 + 14 + 25 + 8 passed, 0 failed, 0 ignored. Strict clippy `--all-targets -D warnings` clean. `cargo fmt --check` clean. |

**One mutation run, and it found a bad test of mine.** Deleting operation-CID collection from
`blob_cids` did **not** fail the reference test as first written: the replication fixture authors
only header edits, which carry no PIX reference, so both sides of the comparison were the base set
and the assertion was vacuous. The test is now scoped and labelled to the base half only, with an
in-test assertion that fires if the fixture ever gains a CID-bearing operation and silently
re-widens it. **The operation half is owed in slice 2**, against the app crate's Flipnote fixture
that publishes real PIX blobs, which is also where the collector plugs into the inventory arm and
where M28 can observe a missing CID. Deferred, not done.

A second mutation confirms a guard that no test previously reached: removing the
`entry.operation.id(&entry.author) != entry.id` check makes
`draft_archive_rejects_an_entry_id_that_does_not_bind_its_body` fail at its intended assertion and
nothing else; source restored byte for byte and all 6 pass again. Without that check an archive
could name accepted work it does not contain, and a preserving disposal would destroy the real
operation while claiming to have kept it.

### Slice 1 review and corrections

Adversarial review of slice 1 returned **CHANGES REQUIRED** with three Medium findings and one
Low, no Critical or High. All four are corrected; none required an architectural change.

| # | Finding | Correction |
|---|---|---|
| 1 | The non-replayable regression was **vacuous**. Every branch built through `append` is replayable by construction, because `append` calls `read` before accepting, so passing `replayable: false` for one of those proved nothing: an implementation that called `read` inside `from_branch` would have passed it. | The fixture is now C1-TEST-002's genuinely structural-but-not-replayable branch. **Mutation: adding `overlay.read` to `from_branch` fails that test at its intended assertion and nothing else**; restored byte for byte, reverified. |
| 2 | The entry-id check **did not prove what its comment claimed**. `DomainOp::id` hashes the logical key, author and nonce and **not the body**, so a different body under the same nonce keeps the same id; the swapped-id test only ever caught a changed identity. | The archive now **carries the envelope**, which does hash the body and is what the live branch compares in `checked_entries`, so a decoded archive stands on its own rather than on the provenance of whatever built it. A new test alters one character of a title in place, leaving every length, nonce and id untouched. **Mutation: removing the envelope comparison fails exactly that test while the swapped-id test still passes**, which is how the two are shown to cover different properties. `validate` also restores the author, doc-type and logical-key invariants. |
| 3 | The bound constants were **padding described as derived**. | Still padding, no longer unproven: a test differences two real encodings to recover the per-entry framing and fixed header, **proves the document maxima rather than assuming them**, extrapolates every variable field to its maximum and requires the result to fit. A future field, wider framing or larger maximum fails there instead of silently narrowing the payload bound. The envelope pushed the per-entry cost to exactly 128, so the constant moved to 160. |
| Low | `Unconfirmed` provenance was the untested wire branch, and it is the one the preview path will use. | Round-trip added, asserting the provenance is in the bytes and not merely in the returned value. |

Evidence after correction: `cargo test -j 1 -p catcoms-replication`, 217 + 14 + 25 + 8 passed,
0 failed, 0 ignored. Strict clippy and fmt clean, **scoped to this package** rather than the
workspace.

**Accepted sequencing constraint from that review, carried forward:** do not narrow Agent 1's
fail-closed `DraftArchive` reference arm until the real PIX-bearing operation-CID test and M28
both pass. That is I-5's precondition and it now has an external reviewer holding it too.

### Commit provenance on the shared branch: a recorded hazard

Slice 1 was committed locally as `f559889`. **That SHA does not exist on the remote, and no
longer exists in local history either.** A parallel session rebased the shared branch, and the
archive files were re-committed inside `0467e45`, which carries **Agent 1's** commit message and
also contains Agent 1 app changes. The file content survived intact, verified by diffing the
working tree against the remote, so nothing was lost; the attribution and the message were.

The effective slice-1 source boundary is therefore `3b6a4b4` to `c3a702a`, **contaminated by
concurrent Agent 1 work**. Shared history is not being rewritten to tidy this.

Consequence, adopted as a rule for the rest of this scope: a local "tree clean, only my files"
report is **not** sufficient evidence that a slice landed as its own commit. Every slice must be
followed by checking the **exact remote SHA and its file list**, and any review request must
name the boundary that actually reached the remote rather than the local commit. Agents on this
branch are committing one another's staged and uncommitted work; the remote history proves it.

Related, and smaller: `cargo fmt --all` on this shared checkout spans other agents' uncommitted
files. Formatting is now scoped with `-p`.

### Slice 2 (in progress): the operation-CID debt, paid

| Item | State |
|---|---|
| `crates/catcoms-app/src/store/epoch_studio/tests/rotation/overlay/archive.rs` | New. Two tests against the app's PIX-bearing Flipnote fixture. |
| The debt from slice 1 | **Paid.** `blob_cids` is now compared against the live branch's own sets on **both** halves, base and operations, assembled the way the inventory arm assembles them. |
| Non-vacuity | Asserted in the test: each operation CID must be present through an accepted operation and absent from the base, so the halves are provably separable and the comparison cannot degenerate the way its predecessor did. |
| Mutation | Deleting operation-CID collection from `blob_cids` fails at "the archive's reference set must equal the live branch's". **That is the exact mutation that survived slice 1.** Restored byte for byte, verified against HEAD, both tests pass again. |
| Evidence | `cargo test -j 1 -p catcoms-app --lib rotation::overlay::archive`, 2 passed. This file is fmt-clean. |
| **Not verified** | Crate-wide clippy and fmt for `catcoms-app`. Agent 1's uncommitted `catchup` work currently fails both; every fmt diff and the clippy error is in its files. Owed once their tree settles. |

I-5's precondition is now half met: the operation-CID test passes. M28 still requires the
collector, so Agent 1's fail-closed `DraftArchive` reference arm stays as it is.

### Slice 2 (continued): the archive writer, and a real bug the assertion caught

| Item | State |
|---|---|
| `ServerStore::write_studio_draft_archive_with_io` | Built. One archive per document; a second **different** one is refused rather than replacing the first; an exact retry takes the sync-only path so an uncertain write repeats at capacity without a second record. Payload-to-scope binding enforced at the writer as well as at the collector, so a misplaced archive is never created rather than merely never trusted. |
| Accounting | `EpochIntentBudget` gains an `archive_bytes` tally and `MAX_VAULT_DRAFT_ARCHIVE_BYTES` (16 MiB), a **share** of the 64 MiB class ceiling and never an addition. Abandoned archive temporaries occupy the sub-cap as they occupy the class total. Preflight before any reservation, both budgets poisoned before the first possible I/O, exactly as `write_prepared_intents` does. |
| I-4 | `epoch_mutation_guard` still does not exist; the two archive writers remain in Agent 1's I-4 participant list. |

**The static assertion fired on its first compile, and it was a real bug.** The design said slice 2
would tie my replication-owned bound to Agent 1's app-side copy with a static assertion. Written,
it immediately failed: the encoder's per-entry cost had grown by the 32-byte accepted envelope,
which an archive must carry because an operation id binds identity and not body, so the seam's copy
was **8 KiB below the encoder's real maximum** and would have refused a maximal archive at the
reader. Two derivations plus an assertion is strictly worse than one derivation, so the app copy is
deleted and the schema's own bound is the only one. This is the second time a constant mirrored for
convenience turned out to be wrong; the first was the mirrored test constants in slice 1.

**A design over-promise, corrected.** Section 6.5 says `write_draft_archive_for_test` "must be
deleted when `write_studio_draft_archive_with_io` lands". That is wrong for the same reason the M19
retirement was wrong: the fail-closed tests need to write payloads that are *not* valid archives,
which the production writer will never do. The helper survives, narrowed to that purpose; the valid
paths move to the real writer. Recorded here rather than silently kept.

### Slice 2 review of `0287910`: three Medium corrections

`f403a0a` **PASSED**: both collector fail-closed findings closed, the `seed()` accessor accepted,
the M19 reclassification accepted. `0287910` returned CHANGES REQUIRED.

| # | Finding | Correction |
|---|---|---|
| 1 | The archive sub-cap made `from_inventory` **fail**, which is self-locking: every accounted write needs a budget, so an over-cap vault would lose unrelated intent writes and the release that is the only way back under the cap | The check is removed from `from_inventory`. The class ceiling and the sub-cap differ in kind: one is a resource rail for a state this code cannot safely account for, the other an admission policy over a state that is perfectly accountable. Existing occupancy is grandfathered, growth refuses at admission. New regression proves an over-cap inventory still yields a usable budget, ordinary work continues, sync-only retry survives, growth refuses, and reduction restores admission. |
| 2 | The sub-cap subtracted `old` before the physical write, contradicting the design's own claim that the peak and temporaries count | Corrected to the physical peak, `archive_bytes + next`, with the `old -> next` move left to commit. **The unit test that blessed the wrong model is rewritten**: a near-cap same-size non-sync replacement must now refuse, because the staged replacement and the record it replaces exist at once. |
| 3a | The writer's *use* of the sub-cap was unanchored: the arithmetic test calls the helper directly and every writer test is far from 16 MiB | New test positions the tally near the cap with class headroom intact, so a refusal is attributable to the sub-cap alone. Note: the mutation the reviewer proposed, swapping in the ordinary class `preflight`, is **impossible** because that method is private to `epoch_intents`. The reachable mutation is passing `sync_only: true`, which skips the sub-cap; that fails the new test. |
| 3b | N19 still installed its **valid** archive with the fault-injection helper, so writer to collector was compositional rather than end to end | N19 now retains the typed archive and persists it through the production writer. Proved by mutation: truncating the scope the writer emits breaks N19, which it could not have detected before. |

Low items also done: the stale helper and module comments, which had already misled twice, and
the status "Not yet built" list. The retirement rule the reviewer extracted is recorded as **R-1**
in the design.

### Verified: `intent_class()` for `DraftArchive` is safe for the preservation guarantee

Agent 1 flagged this as the one decision it made on Agent 2's behalf that is not obviously
reversible, and asked for it to be checked rather than assumed. Checked in source:

- `intent_class()` governs exactly two things: budget accounting in `EpochIntentBudget::
  from_inventory`, which is what Agent 2 wants, and coverage gating in `storage_name`. It reaches
  **no deletion or retirement path**.
- Nothing retires a final record *by family*. Retirement in this codebase removes **ledger
  entries inside the intents record** (`remove_receipted`, `remove_to_manual_recovery`) through
  `retire_included_with_io`, which addresses `epoch_intent_path` alone. An archive is a separate
  record with no ledger entries, so intent retirement cannot reach it by construction.
- Cleanup unlinks only `RecoveryName::Temporary`. Its own comment: "Finals (including corrupt
  ones) and unrelated staging families are not ours to remove."

So a final archive has exactly one deleter: the release path, which does not exist yet. That is
the correctness dependency already recorded above, now with its full weight: grandfathering makes
release the only route back under the sub-cap, and this check confirms nothing else will ever
remove an archive on its own.

The one behaviour that *is* wanted from `intent_class()` on the cleanup side is that an abandoned
archive **temporary** is reclaimable, which it is, and which the sub-cap's orphan accounting
already assumes.

### Answered to Agent 1: intent write frequency is high by design, and user-paced

Agent 1 warned that intent writes rotate `intent_generation` store-wide and asked whether Agent
2's lifecycle writes intents at high frequency, offering to bound it on their side. Checked in
source before answering, rather than from memory:

`save_overlay`'s path (`epoch_intents/overlay.rs`) takes **one** operation and performs **one**
`write_prepared_intents`, which rewrites the whole intents record. So each appended op is one
full-record rewrite and one store-wide generation rotation, and copying N items is N of each.

That is **deliberate**, not an accident to optimise away: C2', L2 and N6 state that a bulk copy is
the user issuing C3/C4 per item, with no batch command and no batch atomicity, so that a partially
completed copy is a coherent set of individually complete copies. Batching them would be an
unreviewed reversal of an accepted decision, so it will not be done as a performance fix.

The mitigating fact for Agent 1's probe: each item is a distinct user action, so the rate is
bounded by human interaction, not by a machine loop. No path in this design rotates
`intent_generation` in an automated loop.

### Shared-checkout conditions during slice 2

The `catcoms-app` test build broke and recovered twice inside a single verification pass, from
Agent 1's in-flight `receiver/catchup.rs`: first a type mismatch, then a missing
`queue_overlay_for_test`. Nothing of Agent 2's was involved. Verification had to be retried
rather than trusted on first result.

An isolated worktree was considered to get a stable build and **rejected**: `target` measures
**138 GB**, so a second target directory is not affordable on this machine, which is the same
constraint the handoffs record about exhausted RAM and disk. The alternative of stashing Agent
1's file was also rejected: reverting a peer's work mid-edit could corrupt their session. The
working answer is to retry and to report which signals could not be obtained.

### Slice 3: the release path, and a stack regression found on the way

`release_studio_draft_archive_with_io` landed at `47bad73`. The design left its signature open;
what slice 3 fixed, and why, is in the commit message and the function's own doc comment. The four
decisions worth repeating here: `expected_content` binds the destruction to the archive the user
was shown, decode happens before destroying, `verify_record` runs before the unlink, and **both
budgets are closed afterwards with a reconcile required**, because release cannot be expressed as
a replacement and hand-subtracting the freed bytes would be a second representation of occupancy
maintained beside the inventory's.

Seven tests, one per guard. The load-bearing one is
`releasing_an_archive_makes_its_sole_references_reclaimable`, the mirror of N19: one vault shows
the pixels pinned because of the archive and reclaimable again because it was released. It is the
only test that proves release reaches the scanner rather than merely returning `Ok`.

**Mutation evidence: COMPLETE, and it found a real gap.** Eight mutations, each restored
byte-exact from git and followed by a passing restored run:

| # | Mutation | Tests that failed |
|---|---|---|
| 1 | delete the `expected_content` comparison | `release_refuses_an_archive_other_than_the_one_it_names` only |
| 2 | delete the `verify_record` call | `release_refuses_a_record_its_budget_does_not_know` only |
| 3 | missing archive returns `Ok(())` | `release_refuses_when_no_archive_is_preserved` only |
| 4 | delete `budget.invalidate()` | `release_closes_both_budgets_so_the_next_write_must_reconcile` only |
| 5 | move `before_unlink` after the removal | `release_consults_hooks_on_both_sides_of_its_unlink` only |
| 6 | skip the unlink entirely | **three** tests |
| 7 | let an undecodable payload proceed | `release_refuses_an_archive_it_cannot_decode` only |
| 8 | delete the `document()` check | `release_refuses_an_archive_naming_another_document` only |

Two of these are worth keeping in mind rather than just counting:

- **6 is not a guard mutation** and is not expected to fail one test. It deletes the operation
  itself, and three tests independently notice the archive surviving. That is the right shape for
  the core operation; only guards owe a single-test failure.
- **8 did not exist until the mutation pass demanded it.** Deleting the document check originally
  failed nothing, because the content comparison catches the cases the other tests happen to
  build. The guard is reachable on its own: an archive for B sealed into A's record would be
  released by a user who confirmed a release for A, destroying B's only preserved evidence while
  the dialog, the scope and the content all agreed. The test now builds that vault and passes B's
  real content, so the document claim is the only thing that can refuse it. This is the third time
  in this scope that a mutation has converted a plausible-looking test set into a real one.

### Slice 3 adversarial review: all four findings addressed, and two mutations that survived

The review of `47bad73` + `d023a9e` returned CHANGES REQUIRED with one High and three Mediums.
Every one was correct. Fixed at `2dba8fa` and after.

**High: the release token did not identify the archive.** `content()` is the *branch's* content
identity, supplied by the caller and stored verbatim, so two archives of one branch that differ
only in `replayable`, `provenance` or `generation` share it. Release-then-write is the only way to
replace an archive, which makes the dangerous sequence ordinary rather than exotic: read A, A is
released by another valid action, B is archived for the same branch, and the queued confirmation
for A destroys B. New `StudioDraftArchive::archive_id`, a derive-key digest over the canonical
payload, hashed over the body rather than the sealed record so re-sealing identical evidence does
not change the identity a user was shown.

**Medium: the storage budget closed too late.** The failure that matters returns early - unlink
succeeds, parent sync fails - so a closure at the end of the happy path never ran. Both budgets
now close before the first destructive step. Mutation 10 (move it back) fails only the new
after-unlink test, which is direct evidence the finding was real.

**Medium: release cannot honour the exact-retry contract.** Recorded as an explicit exemption in
design 12.1, option (a): uncertainty resolves by reconcile-and-re-read, not by resending. Option
(b), a release tombstone, was rejected: a new durable record family with its own accounting,
bound, reference rules and eviction question, built solely to preserve a retry slogan for the one
operation whose purpose is to remove records. The exemption also lives in the function's doc
comment, because a rule that lives only in a design document is one the next caller's author will
not read.

**Medium: the invalidation rails were not independently anchored.** Now closed, in two rounds.

| Rail | State |
|---|---|
| storage-budget closure across a destructive failure | anchored, mutation 10 |
| `inventory_generation` rotation | anchored, mutation 13 |
| over-cap release remediation | anchored, mutation 14 |
| `intent_generation` rotation | anchored, mutation 12b |
| `intents.begin_write()` | redundant hardening, deliberately unanchored |

The first attempt failed and the failure is worth keeping. Mutations 11 and 12 removed each intent
rail and **failed nothing**; removing **both** still failed nothing. Every probe I had written
wrote a record whose map entry the release had changed, so `preflight`'s
`records.get(&id) != old` refused before the generation was ever compared and hid both rails
behind it. Two tests that claimed to isolate the rails were renamed to say what they actually
prove.

I then recorded the rotation as test debt for the disposal slice, on the grounds that isolating it
needed a multi-document fixture this module lacked. **The re-review disagreed and was right.** No
second Studio source is needed: a second logical document in the same group plus the ordinary
`prepare_epoch_intent` path is enough, because A's release does not touch B's intent record, so
B's map entry still matches disk and the rotation is the only remaining fence. That is
`a_stale_intent_budget_cannot_write_another_document_after_a_release`, and deleting the rotation
now fails it and only it.

The lesson is the one worth carrying: "this needs an expensive fixture" was an assumption about
the *probe*, not a fact about the code, and it went unchallenged because the honest negative
result felt like enough diligence on its own. Documenting a gap is not the same as establishing
that the gap is hard to close.

`intents.begin_write()` remains deliberately unanchored. With the rotation in place no stale
intent budget can preflight, so a surviving mutation there implies no safety regression. It is
kept as redundant hardening, consistent with `write_prepared_intents`' discipline, and the
re-review explicitly did not require an anchor for it.

The mutations ran against a private `CARGO_TARGET_DIR` (`M:/catcoms-agent2-target`): the shared
workspace target is contended by the other agents' test binaries, whose long runs hold
`catcoms_app-*.exe` open and fail the link with `LNK1104`. A ten-minute retry loop was not enough;
a private target dir took the cycle to about 35 seconds. This is the opposite of the recorded
shared-target hazard, which is about a *shared* dir poisoning a cache.

**A pre-existing stack regression, reported to Agent 1, not caused by this slice.** While verifying
slice 3, `store::epoch_studio` began aborting with exit `0xffffffff` and no panic message. The
first comparison appeared to implicate slice 3 (three crashing runs with the changes, one passing
run without), and that was **wrong**: the passing run predated Agent 1's C-3 commits, and the tree
moved between the two samples because another agent's uncommitted `epoch_studio/tests.rs` was
present for one and not the other. Re-running the comparison after the tree settled reproduced the
crash **at HEAD with slice 3 stashed**, which is what actually attributes it.

The cause is stack exhaustion, not logic:

| Suite | default stack | `RUST_MIN_STACK=32 MiB` | `RUST_MIN_STACK=128 MiB` |
|---|---|---|---|
| `rotation::overlay::archive` | aborts after 15/17 | 17 passed | - |
| `rotation::overlay::handoff` | aborts | aborts | 37 passed, 2 ignored |

Every test passes in isolation, so it is cumulative depth rather than one deep test. The handoff
suite needing somewhere between 32 MiB and 128 MiB of thread stack is the signal worth acting on,
and it appeared with the C-3 parked-cursor work. Agent 2 is not fixing this: it is Agent 1's area.
Slice 3's own evidence was taken at `RUST_MIN_STACK=33554432`, and **that is a workaround, not a
result** - any later claim that this scope is green must say which stack size it used.

### Slice 4A: the disposal manifest and the v3 extension arm

The replication half of the disposal transaction, at `e6f4a0d`. The store transaction enforcing
D1-D6 is next; this is the state rebuild it calls.

What is worth recording beyond the commit message:

- **The manifest can coexist with a live branch.** `validate` deliberately does **not** require
  `active` to be absent when `disposed` is set. A new branch started after a disposal is exactly
  the `Disposed` classification case, where a request is checked against the retained
  acknowledgement. Requiring absence would have looked tidier and broken that.
- **The version byte is evidence, not a stamp.** 3 only when a disposal is present, so every vault
  that never had one re-encodes byte-identically and keeps passing its own canonical re-encode
  check. Both halves are enforced on decode: v3 must carry a manifest, v2 must not. Without the
  second half the byte would be advisory and a v2 record with trailing bytes would decode as a
  disposal.
- **`dispose` returns the removed ids** rather than letting the store recompute them. Two
  derivations of "which ids went away" is how an entry ends up charged to a branch that no longer
  exists.
- **No test-only constructor was added.** The transfer-hold case drives the real preparation path
  to get a genuine `Prepared` state; the unreplayable case states its property on the manifest
  builder, where the structural-entries guarantee actually lives, rather than inventing a way to
  wrap a bare overlay into a state.
- **`branch_content` is a production accessor, not a test hook.** A disposal request has to carry
  the content the user saw, so something has to expose it; making callers re-derive the hash is the
  second representation that lets a request name work the user never saw.

**Recorded debt from this slice:** `put_target`/`get_target` are now defined **twice**, in
`handoff.rs` and `archive.rs`, and verified byte-identical. That is one fact stored twice - the
same shape as two findings already raised in this scope. Disposal deliberately did **not** add a
third copy (it imports handoff's), but collapsing archive's onto the shared pair is outstanding and
should be its own small commit so it is independently reviewable.

### Slice 4B reordered, and a gap in 4A found while planning it

**The disposal store transaction is blocked on the lifecycle classifier, so the classifier goes
first.** D3 requires `request.branch` to equal the **durable** `branch_id()`, which is
`H(domain, basis fingerprint, branch_generation)` (design 6.6). `branch_generation` has no durable
home yet: it arrives with `admit_new_branch` in the classifier slice. Without it, D3's branch half
could only be enforced against a generation supplied by the caller, which is not a check at all -
a request would be authorising itself. Building an authorization rule with a known hole in it, even
a documented one, is worse than building the prerequisite first.

**The gap in slice 4A:** design 5.1's extension encoding lists `u64 branch_generation`, and the v3
arm I just landed does **not** carry it. That is my omission, not a design change. It is currently
free to fix, because nothing writes a v3 record outside its own tests: there is no production
caller of the disposal transaction and no native command. So `branch_generation` folds into the v3
arm during the classifier slice rather than minting a v4. **This must happen before any production
path can write a v3 record**, or the format is released incomplete and the fix costs a version.

Three invariants from design 5.1 that the classifier slice owes, recorded here so they are not
rediscovered: `disposed.generation <= branch_generation`; `branch_generation >= 1`; and
`active.generation > disposed.generation`, which is implied by `branch_id` construction and must
also be asserted rather than assumed.

**Also landed for 4B, independently:** `IntentLedger::remove_disposed`. A third name over the same
`remove_ids` mechanism, added deliberately because in that type the name *is* the assertion and both
existing removals assert something a disposal cannot. `remove_receipted` claims the entries are
proven final; nobody ever accepted them. `remove_to_manual_recovery` claims the bounded recovery
policy owns the remaining copy; for a discarded draft no copy remains by the user's explicit
instruction, and for a preserved one the copy is a draft archive, not a recovery record. Reusing
either would have put a false claim at the call site.

### Fable adversarial reviews, round 1: slice 4A and the typed reader

Two reviews, at the user's instruction to review each commit.

**Slice 4A (`6eeca185`): CHANGES REQUIRED, nine findings, all correct.** Addressed at `85b394ae`.
The one behavioural defect: design 5.1 lists three validation additions and I implemented two. The
missing rule is that **no id may occur in both terminal manifests**, and the reviewer byte-crafted a
record proving `completed_retry` and `disposed().contains()` can both answer truthfully for the same
id - the state `classify_request` says cannot exist. `dispose` retaining `completed` is what makes
the coexistence possible, so the rule belonged in this slice. My comment saying "the same two rules
the transferred manifest obeys" was accurate about what was copied and silent about what was
dropped.

The rest were honesty and test-quality failures, each real:

- The v3 layout is narrower than design 5.1's (no `branch_generation`, no provenance byte, mandatory
  rather than optional disposal block), and a comment asserted a contract I already intended to
  change. It now says the layout is **provisional**.
- `dispose` records `branch`, `generation` and `provenance` verbatim and cannot corroborate any of
  them - a Closing branch could be labelled `Unconfirmed` - and the doc comment claimed the rebuild
  was validated. Stated plainly now.
- **Three tests passed for the wrong reason.** The backward-compatibility test asserted one byte, and
  a symmetric trailing-byte change to the v2 layout left everything green; it now assembles the whole
  expected sequence. The retired-set oracle could not distinguish the branch's ids from every pending
  id; a never-appended intent now separates them. And test 7 justified using the manifest builder by
  claiming a state around a bare overlay needed a test-only constructor - **false**, and worth
  recording as the second time I have asserted a fixture was expensive without checking:
  `decode_vault_structural` produces one through production code.

**The typed reader (`b0b28dbb`): PASS, three Lows.** Addressed at `e5bd98d9`. The valuable one: the
sealed-scope comparison and the trailing-bytes check had no test, and the scope one carries a real
hazard - `seal` takes no AAD, so nothing binds an archive's ciphertext to its filename except the
plaintext prefix, and `LogicalDocument` equality excludes the local `server`, so the document binding
does not cover a cross-slot copy.

Writing that test corrected an assumption of my own: **a reference scan is not defended by this
guard.** It refuses earlier, by the inventory's filename-against-authenticated-scope rule. My first
version asserted the scan beside the addressed readers, which would have read as coverage of one
guard while exercising another. They are now two tests named for what actually refuses each.

### Process fix: commit before mutating

`git checkout -- <file>` ate uncommitted work **three times** this session, most expensively the six
review fixes to `handoff.rs`, which had to be reapplied from scratch. Mutation testing requires a
committed baseline; the restore step is not compatible with holding unrelated edits in the same
file. **Commit the fix, then mutate, then restore.** Both rounds above now follow that order.

### Slice 5 and its review: the classifier, and a High worth understanding

The lifecycle classifier landed at `8ceb23bd`; the disposal store transaction (D1-D6) at the same
time in the app crate. The fable review of the classifier returned **CHANGES REQUIRED with a genuine
High**, fixed at `a4156a5a`.

**The High: the generation was caller-supplied and there were two minting paths.**
`StudioOverlayAdmission` is a public enum with a public field, so any caller can build
`New { generation }` for any value, and `new_admitted` stored whatever it was handed. The reviewer
demonstrated both consequences: a skipped generation became durable, and **fabricating generation 1
on a post-disposal vault produced a live branch sharing the disposed branch's identity**, after
which `classify_request` answered `Active` for a branch that had been destroyed. That is exactly the
failure the namespace exists to prevent, reached with no tampering at all.

Separately, `append` minted a branch whenever `active` was `None` and did **not** increment - so the
first Save after a transfer or a disposal reused the old generation and inherited its identity. That
one is on the ordinary product path.

Both now route through `next_generation()`, one definition used by all three callers. The increment
had been written in one place and *trusted* in another, which is the same two-representations shape
that has produced most of the findings in this scope.

Worth recording the decision on `append`: **refusing was the wrong fix.** The first Save after a
handoff is an ordinary thing for a user to do and must work; incrementing makes it correct rather
than impossible, and the id it produces is the one `begin` would have offered the client.

**The same wrong assertion, twice.** Writing these tests I twice asserted that a disposed branch's
id should classify as `Unmatched` on a later state. It should not: the retained manifest still
legitimately acknowledges it, and that is what the manifest is *for*. The property is that it must
not resolve to the **live** branch. The code was right both times.

**Stated as untested, with the reason.** Two of `validate`'s generation rules -
`disposal.generation <= branch_generation`, and a live branch beside a disposal being strictly later
- have no test. Both need a record carrying a manifest *and* a mismatched generation, and the
manifest is a variable-length block after the field, so reaching them means byte surgery that
restates the layout or a test-only setter on the state. For decode-path defences that no production
path can violate, an honest gap beats either. The `>= 1` rule *is* tested, because the no-disposal
v3 shape puts the generation in the last ten bytes and needs no guesswork.

I also abandoned a first attempt at those tests that had drifted into offset arithmetic with dead
helpers and a `bump_generation_for_test` mutator - the back door this scope has avoided throughout.
Deleting it was the right call.

### Built so far

*Current as of 2026-09-30, through `bbef5908`.*

The payload codec, the reference collector that narrows Agent 1's fail-closed arm under I-5, the
archive record writer with its accounting and sub-cap, the archive tally on `EpochIntentBudget`,
**the archive release path** (slice 3), **the disposal manifest with its v3 extension arm** (slice
4A), the **typed archive reader**, `IntentLedger::remove_disposed`, **the lifecycle classifier**
(`branch_generation`, `provenance`, `branch_id`, `classify_request`, `admit_new_branch`,
`new_admitted`, the completed v3 layout) and **the disposal store transaction** with D1-D6.

Since then: **the tenure mechanism** (slice 7) - `OwnerTenure::joined`, the `Position` leaf
identity, `ObservedOwnerTenure`, the `catcoms-mls` receive-side M-1 rule, the v1 migration via
`Imported`, and the `verification_owner_tenure_start()` / `authoring_owner_tenure_start()` split;
**the mutation harness** `.github/scripts/check-studio-overlay-lifecycle-mutations.py` and the
`studio-overlay` `lifecycle` CI job; and **four of the nine native commands**.

### Native commands: nine of nine

Against the design's own list (design 6.2). No command on this list enables Save, and
`studio_overlay_save` remains unregistered.

| Command | State |
|---|---|
| `studio_overlay_read` | Pre-existing. The section 11 extension **is** applied at `bfce900e`, minus `eligibility`/`manualReason`/`unconfirmedState`, which are P2's `StudioOverlayHold` and do not exist in the tree, and `archived`, which `studio_overlay_lifecycle` answers from the record that holds it |
| `studio_overlay_lifecycle` | **Landed** `35236b0b`, reshaped by review at `d92f8980` |
| `studio_overlay_export` | **Landed** `9be5a6e7` |
| `studio_overlay_archive` | **Landed** `d92f8980`. Before it, the Preserve arm of disposal was unreachable from the UI: nothing outside `cfg(test)` could create an archive, so D4 always refused |
| `studio_overlay_archive_read` | **Landed** `35236b0b` |
| `studio_overlay_archive_release` | **Landed** `35236b0b` |
| `studio_overlay_copy_preview` | **Landed** `c4ce337d` |
| `studio_overlay_copy_apply` | **Landed** `c4ce337d`, its test corrected at `0b7b6fcc` |
| `studio_overlay_dispose` | **Landed** `35236b0b`, payload reshaped in `bbef5908` |

**One command exists that the design does not list: `studio_overlay_archive_export`.** It exports
the canonical envelope of an already-preserved archive record, where `studio_overlay_export`
exports the *live* draft. Both emit the same `p1-studio-draft-archive-v1` payload through one
helper, and a test asserts the two agree byte for byte. Recorded here rather than left as a silent
addition.

### Not yet built

**V1-V8, the live-tenure contract itself** -
see the P4 row above for why the slice 7 mechanism is not that contract.

One design deviation, recorded rather than left silent: design 5.3 lists
`StudioInspectionPurpose::CopyPlan(choice)`, meaning the copy plan would be produced by the
inspection rebuild. It is not. The plan is produced by `StudioOverlayCopyCapture::plan`, which owns
both captures, because the copy plan needs the **destination's** bytes and those are not in the
inspection capture at all. `StudioInspectionPurpose` exists with `Draft` and `Archive` only. The
behaviour the design asked for is present; the seam it named is in a different place, for the same
reason 5.2 itself was rewritten.

### The copy review: one High, and what is still outstanding

A fable adversarial review of the copy work returned CHANGES REQUIRED. It ran its mutations in a
detached worktree rather than my tree, which is the right way to do it and the fix for the hazard
recorded below.

**Addressed at `d4531b17`:**

- **High. A same-document Index `Object` copy could publish a dangling entry.** Recovery refuses
  exactly this and says why; copy reached `restore::plan` from the detached worker, which has no
  store and cannot probe, and neither C3 nor C4 put the probe back. The Save path has no equivalent
  guard - `local_policy` checks only `MAX_INDEX_OBJECTS`, `index::prepare` only decodes, and
  `check_index_object_sources` runs only at handoff. An Index branch retained across Closing whose
  object was later cleaned up gave `Ready`, applied, and a durable entry naming a source that does
  not exist. The probe now runs at C3 (downgrade) and C4 (refuse).
- **Medium. `source_ops` over-reported.** An `IndexRegister` with no explicit edit falls back to the
  creating operation's source, so a bare `PutObject` reported one id three times while claiming to
  name exactly what it resolved. Deduplicated in order. Its test could not see it because the
  fixture had no `SetTitle`/`SetExpiry`.
- **Medium (part).** The destination channel was checked at C1 and C4 but not C3.

**Outstanding, not yet addressed:**

| Finding | What |
|---|---|
| M2 | No test catches a *changed* destination recovery record. The implementation is correct; the guard is unanchored, and a mutation comparing `is_some()` equality survives |
| M3 | No test exercises a **successful** copy apply or the exact retry. The reviewer verified the behaviour is right in its own worktree; an equivalent test is not committed |
| M4 | C1' ("refused while a transfer hold exists on the destination") is refused only at C4, so a preview returns `Ready` and the apply fails. Not a correctness hole, but the design says C1 |
| L2 | A pre-existing vacuous test (`studio_restore_preview_fingerprint_tracks_provenance...`) asserts only `is_err()` and fails at "missing recovery title" - the same class I fixed elsewhere |
| L4 | The exact-retry shortcut reports **any** prior `(nonce, body)` this device saved into the destination as a copy `Applied {already_saved:true}`, including an ordinary non-copy Save. No write; a misreport of kind. Re-confirmed by the correctness review; nonce replay itself is safe, since an old nonce with a different body hits `IntentConflict` |
| Low (new) | `probe_copy_object` surfaces a wrong-object-channel record as an error rather than a `MissingTarget` hold, so at C3 it is a preview failure. Matches recovery's existing behaviour, which is why it is not being changed unilaterally |
| Low (new) | The tenure seam has two doors: `require_observed_owner_tenure` is expected-dead while nine sites read `authoring_owner_tenure_start()` directly. Correct today, but V5/V7's anchoring is only worth what the callers make it |
| L5 | The same-document copy path is untested, because this fixture's document is Closing. N6's "no byte of the branch's record" is verified only cross-document |

### The comprehensive review: verdict, and the verification debt cleared

Two comprehensive adversarial reviews were run over the whole scope at `288bb30c`, one on
correctness and one on test evidence. The correctness review returned **CHANGES REQUIRED, no High**,
and confirmed the hard invariants directly: P5 holds (ten `studio_overlay_*` commands registered,
none a save, and no composition writes into a Closing branch); evidence precedes removal; both
confirmations are unforgeable and non-transferable; the detached worker carries no live authority;
and M-1's unreachability claim survives attack.

> **Two claims in that paragraph are now false; corrected 2026-10-02.** M-1 is **reachable**: any
> existing member can send one commit with an inline Remove of the committer and an inline Add of
> the same `DeviceId`, and with M-1 disabled a witness merges it
> (`m1_tests::a_witness_refuses_one_commit_that_removes_the_committer_and_re_adds_its_device_id`,
> `0335262e`). M-1 is the only defence and now refuses with its own `InviteError::CommitterReAdded`.
> And the nine lifecycle and copy commands are **not registered** since `510d0b54`; only
> `studio_overlay_read` is.

**It also executed the two test sets I could not**, in a worktree that excludes the other agent's
uncommitted work:

| Run | Result |
|---|---|
| `studio::tenure` (`23465a17`) | **3 passed** |
| `studio::restore` (`d4531b17`) | **8 passed** |
| `cargo test -p catcoms-app --lib studio::` | **222 passed**, 0 failed, 5 ignored |
| desktop `--lib studio::` | **45 passed**, 0 failed |

So both commits' claims now hold, and the "compiled but unrun" caveat in their messages is
discharged.

Fixed in response: the `branch_content` defect below, the row-6 retry arm now comparing `accepted`
as D3 does, and D4 comparing `target` and `provenance` explicitly rather than relying on the
document binding to imply them.

### `content` was a property of the document, not of the branch

The review's Medium 1, and the one with a user-visible consequence. `branch_hash` folded
`ledger.encode()` into the value, so an ordinary intent belonging to nobody's branch changed the
identity of a branch nobody had touched. An archive written at one moment then stopped satisfying D4
at the next, and a preserving disposal refused with "the preserved archive is for a different
branch" **while holding an archive of exactly that branch**. The only recourse was to release
verified evidence and archive again. Reachable in the design's primary same-document copy case,
where the copy itself lands the intent that breaks it.

`branch_content_hash` covers the branch alone, under its own derive key. `Prepared.branch` keeps the
document-wide hash deliberately: a transfer hold is a signing commitment against a state and should
go stale on any change to it.

### P1 BLOCKER: the generation namespace is built and unwired

Found by the comprehensive correctness review, and it is the most important thing in this document.

`classify_request`, `admit_new_branch` and `new_admitted` have **zero non-test callers anywhere in
`crates/catcoms-app/src`**. The Save seam carries `basis` only (`studio/receiver.rs`,
`store/epoch_studio/overlay.rs`) and never a `branch`, so the two-stage classification design 6.6
specifies is never consulted by production code.

**CORRECTED: the short trace this row used to give was wrong, and a review caught it.** It said
"dispose G1, admit G2, deliver a delayed G1 request" and claimed the request appends onto G2. It does
not. The retained G1 disposal manifest still remembers G1's operation ids, and `validate` refuses any
state where an id appears both in the live branch and in the retained manifest - so the overlap is
rejected. That existing defence is real and I had written past it. My own replication test
(`an_old_generation_request_is_stale_after_the_namespace_has_moved_on`) uses the correct sequence; the
prose here did not.

The reachable trace needs the manifest to be **replaced**, because `dispose` overwrites the previous
`disposed` record and does not advance the basis floor:

1. Accept G1 containing operations X; dispose G1.
2. Accept G2 containing **disjoint** operations Y on the still-eligible basis.
3. Dispose G2 - which replaces G1's retained manifest.
4. Deliver a delayed G1 request naming an operation from X.

Now the id is not pending, not in the retained manifest, and not a completed or exact retry, while
the basis fingerprint still matches. With a Save seam that carries only `basis` and no branch
identity, there is nothing left to distinguish the delayed request from new work. That is the case
6.6 says must return `Stale`.

It is not reachable today, for one reason only: `studio_overlay_save` is unregistered. That makes it
a **P5 and P1 blocker rather than a live defect**, and it is the honest reason P1 cannot be called
reviewed:

> "lossless across restart and refusal" cannot be claimed for a lifecycle whose rollover defence
> has no production caller.

The integration is Flow S's: S1 must carry `branch` and call `classify_request`, S1b must call
`admit_new_branch`. That is Agent 1's seam to wire; stating the obligation is mine, and this entry
is that statement. I have not reached into the Save ordering to do it, for the same A-1 reason the
tenure refusals are not mine either.

### The evidence review: two more vacuous tests of mine, and one gap still open

The evidence review's lens is "does the evidence prove what it claims", and it found three more.

**1. The `Imported` preserve regression was vacuous, and it guarded a High.** The step in
`owner_tenure_v1_snapshots_promote_only_the_provably_safe_shape` passed `Position::of(group)` as
`before`, so `applied` returned at its own `if after == before` no-op guard and never reached the
flag line at all. Mutating `self.imported = self.imported && !computed && start.is_some()` to
`self.imported = false` left **all eleven sync tests green**. That flag is the `computed` fix from an
earlier review - the one that stopped an unverifiable v1 tenure laundering itself into a fully
observed one on its next member add - so the fix was real and its anchor was not. Fixed by using a
synthetic `before` one epoch back with the same owner and leaf, which is the shape the preserve arm
actually takes.

**2. The `RefreshRequired` notice had no test.** `DisposeOverlay`, `ReleaseOverlayArchive` and
`FinishOverlayArchive` are in the receiver's `changing` list and nothing named them as emitters, so
the list could have lost any of them silently. Anchored now, with every action in the new test
deliberately **failing**: the notice follows the action rather than its outcome, because the case
that matters is a release whose unlink succeeded and whose parent sync did not.

**3. STILL OPEN: `probe_copy_object` has no test.** The H1 fix from `d4531b17` - the one that stops a
copy publishing an Index entry naming an object that does not exist - is unanchored. Forcing it to
`Ok(true)` leaves all five desktop copy tests green, and the app crate has no store-backed copy
fixture at all. The fix is believed correct and is modelled directly on recovery's equivalent, but
**believed correct is what every vacuous test on this list also was**. It needs either an Index
destination fixture in the desktop tests or a store-backed copy fixture in the app crate.

That is now the single largest known hole in this scope's evidence, and it guards a durable write.

### Preserving-disposal crash ordering: ordering CLOSED, platform barrier OPEN

**Update.** A second re-review separated a real barrier from the order it runs in; fixing the
Windows primitive alone would not have proved the ordering. The ordering is now this transaction's:
after D4 matches and before anything is removed, disposal hands the on-disk archive back to the
writer, whose exact-retry branch performs a guarded sync-only repair. A failure refuses with nothing
removed. Anchored by two hook-observed tests and a tenth harness entry; skipping the barrier makes the
disposal succeed and record `Preserved` with no durable archive, which is the defect demonstrated.

Still open: `sync_directory` is `Ok(())` on `not(unix)`, so on Windows the repair establishes file
contents and not the directory entry. Shared primitive; decision above this scope.

*Original entry follows.*

### (Superseded) OPEN, and not an evidence gap: the preserving-disposal crash ordering

The most serious item in this scope, and the only one that is a missing *guarantee* rather than a
missing test. Raised by an external review, disputed by me, and the review was right.

D4 authenticates and decodes the archive and syncs nothing. I argued that was safe because the archive
and intent records share a `servers/` directory and the replacement's `atomic_write` ends in
`sync_directory` on that parent, so one barrier covers both. That fails twice:

1. **`sync_directory` is `Ok(())` on `not(unix)`.** On Windows there is no parent barrier at all.
   `fs::rename` does not supply one: the pinned toolchain's `MoveFileExW` does not request
   write-through. My own note about cfg-gated blindness on this host says exactly why I should have
   checked this before claiming closure.
2. **"If the fsync fails, neither is durable" is not a property of `fsync`.** A failed flush means
   completion is not guaranteed, not that nothing persisted - and this family's own tests already
   treat a post-rename sync failure as **committed, not rolled back**.

What holds is narrower than the preservation guarantee this scope claims: on Unix, a *successfully
completed* replacement makes both namespace changes durable. Interrupted executions, and every
execution where the barrier is a no-op, are not covered.

**Not fixed here, deliberately.** `sync_directory` is shared by every record family, so the choice is
above this scope: implement a real Windows barrier, refuse a preserving disposal before removal where
none can be provided, or narrow the stated guarantee. Whoever owns persistence should decide. The
`debug_assert` at the D4 site is kept only for co-location and now says so.

### The evidence audit: "every guard has a failing mutant" was FALSE

A full per-test audit ran 25 hand mutations over this scope. Its central result overturns a claim
this document made:

> **"Every guard has a failing mutant" is false at the store disposal layer.** Six guards there can
> be deleted with the whole 105-test overlay suite still green.

They survive because a *later* guard refuses the same input first. That is not redundancy being
harmless: it means each of these can be deleted, or silently stop working, and nothing will say so.

| Guard | Mutation that survives | Why it is masked | Severity |
|---|---|---|---|
| `disposal.rs:72-77` D1 membership | delete the check | the stranger in its test is also not the author, so D1-authorship refuses | **High** |
| `disposal.rs:221-229` D4 content/branch/generation | delete the triple | `matches_branch` refuses instead | **High** |
| `copy.rs:143-165` `probe_copy_object` | force `Ok(true)` | nothing else checks it; no fixture reaches it | **High** |
| `disposal.rs:81-83` D1 target derivation | delete | document binding refuses | Medium |
| `disposal.rs:161` D2 store-level transfer hold | delete | replication's own `dispose` refuses | Medium |
| `disposal.rs:173` D3 content, store level | delete | replication `dispose` refuses with `IntentConflict`; only the **desktop** message test catches it | Medium |
| `epoch_draft_archive.rs:191` record bound | delete | nothing; 32/32 still pass | Medium |
| `disposal.rs:256` D6 ledger-count mismatch | delete | no fixture reaches it | Low, defensive |
| `copy_capture.rs:368-369` `stamp.server`/`target` | delete both | document derivation and a `None` read refuse | Low, redundant |

**The D4 row is the one to fix first.** Its own comment says the generation compare is the only
thing that catches an archive of a *previous generation* whose entries and content are identical -
and that case has no test at all. Of the three Highs, it is the one where the masking guard does not
cover the same ground.

Two more vacuous tests of mine, beyond those already recorded:

- `a_preserving_disposal_refuses_an_archive_for_another_branch` proves `matches_branch`, not the
  metadata triple it is named for.
- `d3_refuses_a_wrong_branch_a_wrong_content_and_a_wrong_count_separately` is not "separately" for
  the content case; only the desktop test's message assertion distinguishes it.

**And a caveat about the harness itself:** it runs each mutation with `--exact`, so siblings never
execute and the script does **not** check isolation. Isolation rests on the hand-runs behind each
entry. `release-scope-binding`'s expected string also matches both assertions in its test, and the
typed reader fires first, so the release path's own scope refusal is never executed under that
mutation.

The audit's full UNVERIFIED list - archive A4-A13, copy_capture M15-M17, desktop D2-D8, restore
P1-P12, tenure T1-T3, sync S1-S8, replication R1-R11 - is the queue for whoever picks this up,
each with the exact mutation to apply.

### Verified at `a0803080`

Run in a detached worktree, because the main tree cannot link (see below).

| Suite | Result |
|---|---|
| `cargo test -p catcoms-sync --lib owner_tenure` | **11 passed** |
| `cargo test -p catcoms-replication --lib studio::` | **136 passed** |
| `cargo test -p catcoms-app --lib studio` | **317 passed**, 6 ignored |

That discharges the "compiled but unexecuted" caveat on `3cb073bf`, `ed8ab0a8`, `1c1a454d` and
`a0803080`. The independent audit separately measured 222 app / 45 desktop at `288bb30c` and ran the
mutation harness end to end: **9 detected, 9 restored runs passing**.

### Blocked: the app crate cannot link

`cargo test -p catcoms-app` has been failing for an extended stretch on **another agent's in-flight
work**, not mine: `crates/catcoms-app/src/actor/file_transfers.rs` references
`catcoms_sync::BlobPageOutcome`, `catcoms_sync::MIN_BLOB_PAGE` and `catcoms_rt::REQUEST_TIMEOUT_MS`,
none of which exist yet in those crates.

`cargo check -p catcoms-app --lib --all-targets` is clean, so everything below compiles, including
the test targets. What has **not** run:

- the `studio::tenure` seam tests (`23465a17`)
- the corrected `studio::restore` tests and the H1 fix (`d4531b17`)

Both commits say so in their own messages. I have not claimed a pass for either and will report the
result when the tree links rather than assume it.

### Mutation testing found three vacuous tests I wrote

All three passed while proving nothing about the guard they named, and all three were caught by
deleting the guard and watching the test stay green:

1. A cross-document copy pointed at an **empty** foreign projection, asserting only `is_err()`. It
   was failing at "missing recovery title", not at the scope check. With the check deleted the plan
   comes back `Ready` - a foreign group's content copied into this document.
2. An apply substituting the literal `"not what was proposed"` as a body. Refused while decoding
   the operation.
3. The same case substituting a **frame** body from a second preview. Refused by the blob rail with
   "publish the frame PIX before saving its reference".

The fix in each case was to make the input reach the guard: a foreign document with a real title of
its own, and a substitute body that is an independently valid title operation. With the body guard
deleted that body is now written - `contentSaved: true`, a renderer previewing one value and saving
another.

Asserting each refusal's **message** is what exposed the second wrong claim in the same test: a
mismatched `epochId` never reaches the echo check at all, because the preflight that establishes the
destination is the Open epoch the proposal was built for fires first. Three guards that all refuse
is not three guards that each refuse for their own reason.

### Findings from the fable adversarial reviews, and what they cost

Two rounds on the native surface. The second is worth recording because of *how* it landed: the
reviewer did not argue that a test was weak, it **deleted two D3 guards and showed all eleven of my
tests still passed**. Every case in `a_live_branch_survives_every_refused_disposal` used
`mode: preserve` against a fixture with no archive, so D4 refused all four whichever D3 check was
removed. The test was green for a reason unrelated to what it claimed. Fixed by sending the first
three as confirmed discards, which have nothing left to stop them, and by asserting each refusal's
own message; the reviewer's exact mutation now fails, and only in that test.

Three more, all real:

- The lifecycle view **mixed branch-scoped and document-scoped facts without binding them**. An
  archive outlives the branch it preserved and a retained disposal of generation N sits beside a
  live N+1. As bare presence flags a renderer would report the user's current work as preserved
  when the archive is evidence for work they already disposed of. Every branch-scoped fact now
  carries its branch and generation.
- **No destructive control action emitted a `RefreshRequired` notice.** Only `Acknowledge` and
  `RestorePointer` were classified as changing, so a dispose that rewrote the intent record, or a
  release that unlinked and then failed its parent sync, left the renderer showing state that is
  gone.
- Release's **uncertain outcome was indistinguishable from a clean refusal** at the IPC boundary,
  although section 12.1 requires such a caller to reconcile rather than resend.
  `AppError::CommittedButNotDurable`'s Display prefix is now published as
  `catcoms_app::UNCERTAIN_OUTCOME` and documented as a contract.

Two defects were found by writing tests rather than by review: the lifecycle view did not publish
the branch content digest a disposal has to echo, so the command surface was complete and
unusable; and serde's internally tagged representation let a **unit** variant swallow every other
field, so `{"kind":"preserve","confirmation":"destroy-local-draft"}` deserialised silently.

**A process cost worth recording.** I let a review agent with checkout permissions run while I had
uncommitted work in its scope, and its byte-exact restore - correct behaviour on its part - took
the whole of `lifecycle.rs`. That is the fifth time `git checkout --` has destroyed uncommitted
work in this scope, and the first time it was not my own hand. The rule stands and now has a second
half: **commit before mutating, and commit before letting anything else mutate.**

**Sections above this point are an append-only ledger and are dated.** Where an earlier entry
says something is not yet built, read it as the state at that entry's date, not as current
state; this pair of lists is the current one.

## Test and CI evidence

*Current as of 2026-09-30, through `bbef5908`. Local runs on this Windows host, no GitHub run
URLs yet.*

| Item | State |
|---|---|
| `cargo test -p catcoms-app --lib studio::` | **211 passed, 0 failed, 5 ignored** at `bbef5908`, `RUST_MIN_STACK=33554432` |
| `cargo test --lib studio::` in `apps/desktop/src-tauri` | **36 passed, 0 failed** at `bbef5908` |
| `cargo clippy -p catcoms-app --lib --all-targets` | Clean at `bbef5908` |
| `cargo clippy --lib --all-targets` (desktop) | Clean at `bbef5908` |
| Mutation script and restored regressions | Script written (9 mutations, 4 crates). **Not yet re-run end to end on a quiet tree** |
| `studio-overlay` `lifecycle` job | Added to `.github/workflows/studio-overlay.yml`. No run URL observed yet |
| GitHub run URLs and checkout SHAs | None |

`RUST_MIN_STACK=33554432` is a **workaround, not a result**: without it these suites abort with
`0xffffffff` from stack exhaustion. The cause is not in this scope.

Any later claim of a pass must name the exact command, the executed test count, the ignored cases,
the run URL and the actual checkout SHA.

## Implementation prerequisites, in order

The design is accepted; implementation has not started. Before it does:

1. **`EpochRecordKind::DraftArchive`: LANDED** at Agent 1's `705d44b`, option (a), seam only, no
   writer, no guard, I-4 not pulled forward. Agent 2 verified two claims in source rather than
   accepting them: `collect_creative_references` structurally refuses a narrow or partly consumed
   scan, and the archive reference arm fails closed, scoped to reference scans only. Both are
   recorded as inherited guarantees in design 6.5, with invariant I-5 requiring the fail-closed arm
   to be **narrowed, not deleted**, when the collector replaces it. Agent 1 informs Agent 3.
2. **The archive writers must appear in I-4's participant list.** `write_studio_draft_archive_with_io`
   and `release_studio_draft_archive_with_io` join the recovery, owner, Registry, Studio source,
   intent and cleanup writers that rotate nothing today. Until I-4 lands, the archive writers are in
   the same position as every other five-family writer. This is a tracked obligation on both sides,
   not a commit-message note.
3. **A-1's precondition: REVERIFIED TWICE, and the second one mattered.** Agent 1's FS-002 at
   `5a024a7` did move `tenure.ok_or_else` earlier in `save_studio_closing_overlay_with_io`. A-1
   survived, because it moved to just after the ordinary-collision check rather than above the retry
   branches, but that is the exact drift A-1 exists to catch and it happened within days. Agent 1
   has since bracketed the Save ordering with its own assertions on both sides. Handoff is stronger
   still: `resolve_studio_handoff_with_io` takes no tenure parameter, so the L11 limbo case cannot
   arise from statement-level drift at all. N-T7b remains Agent 2's and remains the only end-to-end
   proof.
4. **Branch placement: settled.** Everything stays on `gate4-agent1-runtime` for now, by the user's
   decision. Agent 4 may still relocate these documents at integration.
5. **CORRECTED, and this entry is the example R-1 exists to prevent.** This item used to claim two
   seam artefacts retire with this design's implementation: that M19 is superseded by M28 when the
   collector lands, and that `write_draft_archive_for_test` is deleted when
   `write_studio_draft_archive_with_io` lands. **Both claims were wrong, and design rule R-1 was
   written because of them.** Each reasoned from *implementation succession* while the artefact
   also provided *unique state reachability*: M19 asserts a property M28 does not, and the
   `cfg(test)` hand-sealer is the only way to reach a malformed-archive state the real writer
   cannot produce, so it is retained as **fault injection** with its doc-comment rewritten to say
   so. Neither is retired. Before any future entry in this document says "X retires when Y lands",
   it must answer R-1's five questions first.
6. The shared central edits listed above stay coordinated with Agent 4: the control and dispatch
   enums, `StudioSettlementState`, the inventory family and the `catcoms-mls` receive rule.

## Open questions

None. Design 16 records every question as answered, including the 16.1 product decision, which is
option (a): `Imported` ships fail-closed with no operator-adoption override.

## Superseded: open questions carried to the re-review

Design section 16: archive placement (a second record kind in the Intents family versus its own
inventoried family); archive cardinality; whether the generational identity fully closes finding 3
and whether Stale is acceptable for a retry older than the retained manifest; the leaf-digest field
set and whether the commit-builder membership rule is necessary; and preview seed retention versus a
bounded reconstruction API.

## Known limits recorded so far

Design section 15, L1-L10. Changed since revision 1: L1 now covers only copy planning and the typed
projection view, because export, archiving and a preserving disposal are structural; L5 is
superseded by the section 9.3 correction and is replaced by two testable obligations; L9 and L10 are
new, covering archive cardinality and the retained preview seed bytes.
