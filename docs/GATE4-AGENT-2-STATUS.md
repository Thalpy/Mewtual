# Gate 4 Agent 2 status

Owner: Agent 2, [overlay lifecycle, provisional local work and repeated tenure](GATE4-AGENT-HANDOFFS.md#agent-2-overlay-lifecycle-provisional-local-work-and-repeated-tenure).
Design of record: [GATE4-AGENT-2-DESIGN.md](GATE4-AGENT-2-DESIGN.md).
Review preamble: [preamble 2](GATE4-REVIEW-PREAMBLES.md#review-2-manualprovisional-overlay-lifecycle-and-repeated-tenure).

## Current state

| Item | State |
|---|---|
| Design revision | 2, **awaiting re-review** |
| Original design base | `1bcb1bca204d721b848b17c0835faf931ae930e3` |
| Revision 1 head, reviewed | `a901f6b0f64df2b4ea9cc0221b64ac98276f582d` |
| Revision 2 base | `a901f6b0f64df2b4ea9cc0221b64ac98276f582d` |
| Revision 2 head SHA | _pending: the commit that adds revision 2; fill before sending the review request_ |
| Working checkout | main repository tree, branch `Create-suite-2`. No separate worktree yet; one is taken before any production edit. |
| Production code | **None written.** |
| Tests added | **None.** |
| Cargo commands executed | **None.** No local or CI run exists for this scope. |
| Measurements | **None.** Every number in the design is an existing bound read from source, not an observation. |
| Native commands registered | **None by Agent 2.** `studio_overlay_read` remains the only registered overlay command, unchanged. |

## Agent 1's registration prerequisites (its section 12.3)

**Authoritative statement: P1 to P4 are DESIGNED ONLY, and the design has not yet passed review.
P5 is FALSE. `studio_overlay_save` must not be registered.**

| Prerequisite | State | Where |
|---|---|---|
| P1 reviewed manual lifecycle: inspect, export, copy-into-current, explicit disposition, lossless across restart and refusal | Designed, unimplemented, **design not yet accepted** | design 6.1-6.6, 12 |
| P2 every `StudioOverlayHold` variant mapped to a user-visible actionable state | Designed | design 7, 11 |
| P3 truthful native results, events and UI-hooks rows | Designed, including the corrected three-state write outcome | design 11 |
| P4 live-tenure contract for `observed_owner_tenure_start()` | Designed | design 9.4 V1-V5 |
| P5 explicit statement that P1-P4 are implemented and reviewed | **No** | this table |

This row is the single authoritative source for P5. It changes only after implementation exists and
review 2 returns PASS for the corresponding boundary. Revision 1 returned CHANGES REQUIRED on all
three boundaries, so the design itself is not yet accepted.

## Review history

| Date | Item | Verdict |
|---|---|---|
| 2026-09-15 | Design revision 1 (`a901f6b`) | **CHANGES REQUIRED on all three boundaries**, nine findings: 3 High, 6 Medium. Reviewer ran no Cargo commands and did not complete the `catcoms-sync/src/lib.rs` constructor/restore call-site trace. |
| 2026-09-15 | Design revision 2 | Request prepared; head SHA pending. Three separable verdicts requested again. |

### Revision 1 findings and their disposition

| # | Sev | Finding | Disposition |
|---|---|---|---|
| 1 | High | `Copied` disposal has no valid terminal representation: the mode required `copy`, `copy` was forbidden without `active`, and the transition cleared both | **Corrected by removing the cause.** Copy bookkeeping deleted from the durable record. The terminal manifest is self-contained and no rule of it refers to a live field. Positive encode/decode/reopen cases added for both modes (N11, N13), plus M1b, which makes the defect executable. |
| 2 | High | The copy proof could account for the wrong source work; projection-level copying is not envelope-level preservation | **Corrected twice.** Copy is no longer a disposal precondition, and `source_entry` is deleted: `restore::plan` derives `source_ops`. Preservation moves to a lossless draft archive. C-P states the loss plainly. |
| 3 | High | Repeated disposal erased the only defence against an old Save retry | **Corrected by a durable branch-generation namespace.** `branch_id` includes a monotonic generation; an unknown id returns Stale, never a new acceptance (N17b, M3b, M10b). |
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
| **`branch_generation`, `branch_id`, `classify_request`** (new, finding 3) | core | Agent 1's Save classification |
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
- **I-F.** `StudioOwnerTenure::Unknown` is fail-closed for every authoring, signing,
  repair-issuance, rotation and publication decision. A reused key, a Welcome, a hint, a candidate
  receipt's claim, a fresh owner proof's claim and the current group epoch are each insufficient to
  make it `Known`.
- **I-G.** A returning owner in a new tenure observes a strictly different value from its earlier
  tenure, including across a same-commit membership discontinuity.
- **I-H (new, finding 2).** No count of copied items, and no `source_ops` value, ever establishes
  that a branch was preserved. Only a durable archive whose `content`, `branch`, `generation` and
  entry list match does.
- **I-I (new, finding 3).** A request naming a branch identity the record does not know is refused
  as Stale. Forgetting acknowledgement evidence degrades to refusal, never to acceptance.
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

## Test and CI evidence

| Item | State |
|---|---|
| Focused core/store/actor suites | Not written, not run |
| Mutation script and restored regressions | Not written, not run |
| `studio-overlay` `lifecycle` job | Proposed in design 17.3; not added to any workflow |
| GitHub run URLs and checkout SHAs | None |

Nothing in this scope has been executed. Any later claim of a pass must name the exact command, the
executed test count, the ignored cases, the run URL and the actual checkout SHA.

## Open questions carried to the re-review

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
