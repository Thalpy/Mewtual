# Gate 4 Agent 2 status

Owner: Agent 2, [overlay lifecycle, provisional local work and repeated tenure](GATE4-AGENT-HANDOFFS.md#agent-2-overlay-lifecycle-provisional-local-work-and-repeated-tenure).
Design of record: [GATE4-AGENT-2-DESIGN.md](GATE4-AGENT-2-DESIGN.md).
Review preamble: [preamble 2](GATE4-REVIEW-PREAMBLES.md#review-2-manualprovisional-overlay-lifecycle-and-repeated-tenure).

## Current state

| Item | State |
|---|---|
| Design revision | 1, **awaiting adversarial review** |
| Design base SHA | `1bcb1bca204d721b848b17c0835faf931ae930e3` |
| Design head SHA | _pending: the commit that adds the design; fill before sending the review request_ |
| Working checkout | main repository tree, branch `Create-suite-2`. No separate worktree is in use yet; one will be taken before any production edit. |
| Production code | **None written.** |
| Tests added | **None.** |
| Cargo commands executed | **None.** No local or CI run exists for this scope. |
| Measurements | **None.** Every number quoted in the design is an existing bound read from source, not an observation. |
| Native commands registered | **None by Agent 2.** `studio_overlay_read` remains the only registered overlay command, unchanged. |

## Agent 1's registration prerequisites (its section 12.3)

**Authoritative statement: P1 to P4 are DESIGNED ONLY. P5 is FALSE. `studio_overlay_save` must not
be registered.**

| Prerequisite | State | Where |
|---|---|---|
| P1 reviewed manual lifecycle: inspect, export, copy-into-current, explicit disposition, lossless across restart and refusal | Designed, unimplemented, unreviewed | design 6.1-6.4, 12 |
| P2 every `StudioOverlayHold` variant mapped to a user-visible actionable state | Designed | design 7, 11 |
| P3 truthful native results, events and UI-hooks rows | Designed | design 11 |
| P4 live-tenure contract for `observed_owner_tenure_start()` | Designed | design 9.4 V1-V5 |
| P5 explicit statement that P1-P4 are implemented and reviewed | **No** | this table |

This row is the single authoritative source for P5. It changes only after implementation exists and
review 2 returns PASS for the corresponding boundary.

## Proposed API seams

Full signatures are in design section 5. Summary, for Agents 1, 3 and 4 to agree before consuming:

| Seam | Kind | Consumer |
|---|---|---|
| `StudioOverlayProvenance` on the basis, `StudioOverlayState::provenance()` | core | Agents 1, 3 |
| v3 extension arms: `disposed`, `copy`, provenance tag; `dispose`, `record_copy`, `disposed_retry` | core | Agent 1 (Save classification), Agent 3 |
| Provenance guard on `prepare_handoff`, `prepare_handoff_detached`, `prepared_manifest` | core | Agent 1 |
| `ServerStore::studio_overlay_lifecycle` -> `StudioOverlayLifecycle` | store, structural only | Agents 1, 4 |
| `ServerStore::dispose_studio_overlay_with_io`, `record_studio_overlay_copy_with_io` | store | Agent 2 only |
| `StudioInspectionPurpose` / `StudioInspectionCapture::rebuild_for` | store, extends the accepted capture | Agent 1 |
| `restore::plan(.., history: &[&StudioProjection], .., PlanScope)` | app | Agent 2 |
| `StudioControlAction::{OverlayLifecycle, ExportOverlay, FinishOverlayExport, PrepareOverlayCopy, FinishOverlayCopyPreview, ApplyOverlayCopy, DisposeOverlay}` and their responses | app, **central enum edit** | Agents 1, 3, 4 |
| `StudioSettlementState::{LocalDraftManual, LocalDraftDisposed}` | app, **shared enum** | Agent 1 |
| `StudioOwnerTenure`, `Server::observed_owner_tenure`, `require_observed_owner_tenure` | app | Agents 1, 3 |
| `OwnerTenure::joined` and the `ChannelSync::new_joined` call site | sync, **authority-bearing** | Agents 1, 3 |
| Native `studio_overlay_{lifecycle,export,copy_preview,copy_apply,dispose}` | native | Agent 4 registers |

## Invariants this scope owns

- **I-A.** Read-only operations change no durable byte. Inspect and export never clear `Prepared`,
  retire an entry, advance a floor, release a reference hold or record that they occurred.
- **I-B.** An annotated ledger entry leaves the ledger only through the explicit disposal
  transaction of design 6.4. Receipt-covered retirement keeps its existing overlay filter and stays
  incapable of removing one.
- **I-C.** Disposal writes its bounded manifest and removes the named entries in one sealed,
  accounted, atomic replacement of the intent record. There is no state in which the entries are
  gone without their evidence.
- **I-D.** A branch whose provenance is `Unconfirmed` can never reach handoff preparation, a signed
  source, a receipt, a tenure, publication, settlement, receipt-covered retirement, replay evidence
  or ordinary Apply. The guard is in core, not only in the app.
- **I-E.** Preview expiry, eviction, replacement, unwatch, lock, remount, membership change and
  restart remove the live preview and never the durably accepted draft; a retained draft never
  revives a preview.
- **I-F.** `StudioOwnerTenure::Unknown` is fail-closed for every authoring, signing, repair-issuance,
  rotation and publication decision. A reused key, a Welcome, a hint, a candidate receipt's claim,
  a fresh owner proof's claim and the current group epoch are each insufficient to make it `Known`.
- **I-G.** A returning owner in a new tenure observes a strictly different value from its earlier
  tenure.

## Files this scope expects to touch

Leaves owned by Agent 2 (new): `catcoms-replication/src/studio/overlay/disposal.rs`,
`catcoms-app/src/store/epoch_intents/{disposal,archive}.rs`,
`catcoms-app/src/studio/lifecycle.rs`, `catcoms-app/src/studio/overlay/copy.rs`,
`apps/desktop/src-tauri/src/studio/overlay.rs`, plus their test modules and
`.github/scripts/check-studio-overlay-lifecycle-mutations.py`.

Shared files, to be coordinated with Agent 4 before editing:
`catcoms-replication/src/studio/overlay.rs` and `overlay/handoff.rs`;
`catcoms-app/src/store/epoch_intents.rs`, `.../epoch_intents/{inspection,retirement}.rs`;
`catcoms-app/src/studio/{restore,control,dispatch,inspection,settlement}.rs`;
`catcoms-sync/src/{owner_tenure.rs,lib.rs}`;
`apps/desktop/src-tauri/src/{lib.rs,studio.rs}`.

Documents owned by Agent 4 and **not** edited by this scope: `INTERFACES.md`,
`BACKEND-IMPLEMENTATION.md`, `HANDOVER.md`, `design-creative-suite.md`, `FLIPNOTE-UI-HOOKS.md`,
`GATE4-ACCEPTANCE.md`.

## Proposed UI-hooks update

Exactly design section 11: the extended `OverlayInspection` type with `branch`, `provenance`,
`eligibility`, `manualReason`, `unconfirmedState` and `copiedEntries`, the new `disposed` kind, the
six-row command table, the truthfulness rules, and the two new `SettlementState` values
`localDraftManual` and `localDraftDisposed`. Agent 4 applies it; it is not applied here and no row
may be published as available before the corresponding command is registered.

## Test and CI evidence

| Item | State |
|---|---|
| Focused core/store/actor suites | Not written, not run |
| Mutation script and restored regressions | Not written, not run |
| `studio-overlay` `lifecycle` job | Proposed in design 17.3; not added to any workflow |
| GitHub run URLs and checkout SHAs | None |

Nothing in this scope has been executed. Any later claim of a pass must name the exact command, the
executed test count, the ignored cases, the run URL and the actual checkout SHA.

## Review history

| Date | Item | Verdict |
|---|---|---|
| 2026-09-15 | Design revision 1 | Request sent / pending. Three separable verdicts requested: (a) manual lifecycle, stale bases and tenure integration; (b) preview-local-work extension; (c) the `OwnerTenure::joined` correction. |

## Open questions carried to review

Design section 16: the disposal trade against P1's recovery-before-removal; v3 extension versus a
separate record; the soundness and boundary of the `OwnerTenure::joined` inference and its L5
residual; seed-only versus bounded-tail unconfirmed bases; the unconfirmed rails; and the bound on
cross-document copy.

## Known limits recorded so far

Design section 15, L1-L8: no export or copy for a structurally valid but non-replayable branch;
per-item copy with no batch atomicity; the 64 KiB metadata ceiling against two full manifests;
`Discarded` destroys the bodies; the same-commit remove-and-re-add tenure residual; legacy-snapshot
and unobserved-gap owners staying Unknown; the seed-only unconfirmed base; and no import path.
