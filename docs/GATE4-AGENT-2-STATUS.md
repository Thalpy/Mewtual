# Gate 4 Agent 2 status

Owner: Agent 2, [overlay lifecycle, provisional local work and repeated tenure](GATE4-AGENT-HANDOFFS.md#agent-2-overlay-lifecycle-provisional-local-work-and-repeated-tenure).
Design of record: [GATE4-AGENT-2-DESIGN.md](GATE4-AGENT-2-DESIGN.md).
Review preamble: [preamble 2](GATE4-REVIEW-PREAMBLES.md#review-2-manualprovisional-overlay-lifecycle-and-repeated-tenure).

## Current state

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
| Working checkout | main repository tree. Revision 2 was committed on branch `gate4-agent1-runtime`, which a parallel Agent 1 session had checked out; the user asked for no branch change. The design content is branch-independent, but Agent 4 should expect to move these two documents when the branches are integrated. No separate worktree yet; one is taken before any production edit. |
| Production code | **None written.** |
| Tests added | **None.** |
| Cargo commands executed | **None.** No local or CI run exists for this scope. |
| Measurements | **None.** Every number in the design is an existing bound read from source, not an observation. |
| Native commands registered | **None by Agent 2.** `studio_overlay_read` remains the only registered overlay command, unchanged. |

## Agent 1's registration prerequisites (its section 12.3)

**Authoritative statement: P1 to P4 are DESIGNED AND THE DESIGN IS REVIEWED, but NONE OF THEM IS
IMPLEMENTED. P5 is FALSE. `studio_overlay_save` must not be registered.**

P1's wording is "a reviewed manual lifecycle". The design of that lifecycle is now reviewed and
accepted; the lifecycle itself does not exist. P5 asks whether P1 to P4 are **implemented** and
reviewed, and no line of production code has been written for any of them.

| Prerequisite | State | Where |
|---|---|---|
| P1 reviewed manual lifecycle: inspect, export, copy-into-current, explicit disposition, lossless across restart and refusal | **Design accepted**, unimplemented | design 6.1-6.6, 12 |
| P2 every `StudioOverlayHold` variant mapped to a user-visible actionable state | **Design accepted**, unimplemented | design 7, 11 |
| P3 truthful native results, events and UI-hooks rows | **Design accepted**, unimplemented; no row is published as available and no command is registered | design 11 |
| P4 live-tenure contract, over `verification_owner_tenure_start()` and `authoring_owner_tenure_start()` | **Design accepted**, unimplemented | design 9.4 V1-V8, 9.3 part 5 A-1 |
| P5 explicit statement that P1-P4 are implemented and reviewed | **No** | this table |

This row is the single authoritative source for P5. It changes only after implementation exists and
review 2 returns PASS for the corresponding boundary. Revision 1 returned CHANGES REQUIRED on all
three boundaries, so the design itself is not yet accepted.

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

### Built so far

The payload codec, the reference collector that narrows Agent 1's fail-closed arm under I-5, the
archive record writer with its accounting and sub-cap, and the archive tally on
`EpochIntentBudget`.

### Not yet built

The archive release path, the disposal transaction (which is the writer's first production
caller), the v3 record arms, the composite copy capture, the lifecycle classifier, the tenure
work and every native command.

**Sections above this point are an append-only ledger and are dated.** Where an earlier entry
says something is not yet built, read it as the state at that entry's date, not as current
state; this pair of lists is the current one.

## Test and CI evidence

| Item | State |
|---|---|
| Focused core/store/actor suites | Not written, not run |
| Mutation script and restored regressions | Not written, not run |
| `studio-overlay` `lifecycle` job | Proposed in design 17.3; not added to any workflow |
| GitHub run URLs and checkout SHAs | None |

Nothing in this scope has been executed. Any later claim of a pass must name the exact command, the
executed test count, the ignored cases, the run URL and the actual checkout SHA.

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
5. **Two seam artefacts retire with this design's implementation.** Agent 1's M19 is superseded by
   M28 when the collector lands, with a different assertion; and `write_draft_archive_for_test`, the
   `cfg(test)` hand-sealer that exists only because the family has no writer, is deleted when
   `write_studio_draft_archive_with_io` lands, with the seam's regressions repointed at the real
   writer.
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
