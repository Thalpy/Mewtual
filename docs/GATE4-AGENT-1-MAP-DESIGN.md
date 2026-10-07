# Gate 4 Agent 1: G4-A1-MAP, structured refusal reasons (design, revision 1)

Status: **proposal, not reviewed, nothing built.** It answers acceptance row G4-A1-MAP
(`GATE4-ACCEPTANCE.md`): "map runtime refusals to `StudioOverlayEligibility` /
`StudioOverlayManualReason` without collapsing distinct cases", with "store -> actor -> native
conversion coverage for every reason, including stale final delivery". Several decisions below
need Agent 2's agreement, because the reason types are theirs; they are marked **[A2]**.

The evidence is a read-only inventory of every Flow S and Flow H refusal at `9c63bd6e`
(2026-10-07). File and line references below are to that commit.

## 1. What exists

- **The reason types** are Agent 2's, in `catcoms-replication/src/studio/overlay/eligibility.rs`:
  `StudioOverlayEligibility { Transferable, Manual(StudioOverlayManualReason) }`, sixteen manual
  reasons, and `StudioOverlayUnconfirmedState`. Their contract is about the **automatic handoff**:
  "why the handoff will refuse this draft, from durable state only"; transient refusals "say
  retry" and are deliberately not classified (`store/epoch_studio/eligibility.rs:73-77`). They
  are produced only by the **read-side** classifier `studio_overlay_eligibility` and
  `overlay_successor_hold_in_vault`, and reach native JSON through the lifecycle row and the
  inspection (`src-tauri/src/studio/lifecycle.rs:482-513`).
- **Every runtime refusal is a string.** Store errors are `AppError::Invalid("epoch studio: ..")`;
  the actor flattens every control reply to `Result<_, String>` (`actor.rs:4392-4396`), and native
  passes the string on. No Save or handoff refusal produces a reason today.
- **Distinct situations share one value** (the inventory's K1 to K11). The worst: `EpochScope` at
  S1b covers six cases (another channel's record, the other kind of draft naming the live branch,
  a live branch opened on a superseded basis, an Unconfirmed basis for another target, a basis
  rewound below the floor, an S2 fingerprint mismatch); `stale_branch` covers six; `EpochClosed`
  covers "a handoff is in flight" (retry) beside "the source faulted" (permanent); the handoff
  path merges `Imported` with `Unknown` tenure through an `Option<u64>`, and its successor check
  collapses seven manual reasons into `EpochClosed`.
- **The scheduled runtime throws reasons away.** The H1 probe, signing, commit and detached
  completion record only a per-target backoff (`receiver/handoff.rs`); a refused S2 plan is
  dropped (`receiver/catchup.rs:1429-1450`), which with a deterministic refusal makes the Save
  answer `Scheduled` indefinitely (sent to Agent 2 as a likely HIGH; section 5).

## 2. The central decision: two kinds of reason, not one

`StudioOverlayManualReason` answers "why won't the automatic handoff take this draft". Most Save
refusals answer "why can't I author this now", and stuffing them into the manual enum would break
its one-reason-per-Manual-draft contract and its native literal set. So:

- **Flow H** refusals map onto the **existing** manual reasons, where the inventory shows they
  coincide exactly (`ObjectMissing`, `NotReplayable`, `PreparedStuck`, `SourceMissing`,
  `Unconfirmed`, `TenureUnknown`, `TenureImported`, the seven successor reasons, `NotCurrentAuthor`,
  `ReceiptChanged`, `SourceUnreadable`). Transient handoff refusals stay unclassified, as the
  contract already says.
- **Flow S** refusals get a **new** type, provisionally `StudioOverlaySaveRefusal` **[A2: name,
  home and owner]**, which reuses a manual reason by value only where the two meanings are
  identical (the tenure pair, `SourceMissing`, `Fault`), and otherwise has its own variants
  (section 3). Every variant is classed as **user-actionable**, **retry** or **internal**.

## 3. Flow S: proposed `StudioOverlaySaveRefusal`

| variant | class | from (inventory row) | split needed first |
|---|---|---|---|
| `NotMember` | user | S1, S26 | none |
| `Tenure(Unknown \| Imported)` | user | S7 | none |
| `SourceMissing` | user | S8 | none |
| `SourceFaulted` | user | S9 (`EpochClosed` on a faulted source) | K4: split `prepare_settlement`'s faulted case from adopting / not Closing |
| `ClosingEnded` | user | S9 (successor installed) | K4 |
| `OwnerChanged`, `ReceiptChanged` | user | S9 `EpochAuthority` / `ReceiptConflict` | K5 |
| `InstalledSource` | user | S10 (Unconfirmed beside a source) | none |
| `PreviewUnavailable`, `PreviewExpired`, `TailIncomplete`, `PreviewReplaced` | user / retry | S12 | K10: `SyncError::Unauthorized` covers five provisional cases (sync layer, Agent 2) |
| `MembershipChanged` | retry | S15 | none |
| `BasisChanged { during_detach: bool }` | user | S16, S25 | K3: S1b-stale versus changed during the detach |
| `TransferInProgress` | retry | S17 (`EpochClosed`: branch Prepared) | K4 |
| `OtherKindDraft` | user | S18 / K1(b), part of K2 | K1, K2 (design 8.1 requires "a draft of the other kind holds this document") |
| `LiveBranchSuperseded` | user | S18 / K1(c) | K1 |
| `StaleBranch` | user | S20, S24 floor | K2: cross-kind and wrong-body cases split out |
| `NonceConflict` | user | S4, S5, S24 `IntentConflict` | none |
| `OrdinaryIntentPending` | user | S6 | none |
| `BranchFull` | user | S24 `EpochBound` (64 / 256) | move the rail to S1b (section 5) |
| `PixelUnpublished`, `PixelInvalid`, `PixelMissing` | user | S21, S25 | none |
| `StorageRefused { reason }` | user | S22 cap, S24 ledger, S25/H5 preflights | align with Agent 2's planned 8.3 rail refusal **[A2]** |
| `Retry` | retry | S2, S11, S22 scan, S25 context changed | none; one variant, the message keeps the detail |
| `Internal` | internal | S3, S13, S14, S19, S23, S25 mismatches | none; keeps the string, never user-facing copy |

**Carriage.** The actor's reply channel is `String`, and changing it is a wide edit. The
precedent is `StudioRepairOutcome::StorageRefused`, an **Ok** outcome. So a refusal travels as
`Ok(Refused(reason, message))` on `StudioOverlaySaveVisit` and on `StudioUnconfirmedSaveOutcome`
(and the Closing equivalent) **[A2: your outcome type]**, with the string kept for logs. `Err`
remains for faults the caller cannot act on.

## 4. Flow H: reasons through the scheduled runtime

The handoff has no caller to answer; its refusals matter to the lifecycle row, which already shows
`eligibility`. Proposal: the receiver keeps the **last classified refusal per target** beside the
backoff it already records, and the lifecycle row prefers the classifier's own reason but shows
the runtime's when the classifier says `Transferable` and the runtime has just been refused (the
gap between "would transfer" and "was refused"). Splits needed:

- **K6**: the probe and `start_studio_handoff` take `Option<u64>` tenure; carry
  `StudioOwnerTenure` so `Imported` and `Unknown` stay apart.
- **K7**: `check_overlay_successor` returns `EpochClosed` / `EpochAuthority` for nine distinct
  reasons. Either it returns the reason **[A2: shared replication code]**, or the refusal site
  calls `overlay_successor_hold_in_vault` to classify after the fact. The second keeps the change
  in my files, at the cost of a second read on a refusal path; preferred unless Agent 2 objects.
- **K8**: `live_authority` / `check_live` `EpochAuthority`: `NotCurrentAuthor` and
  `ReceiptChanged` are reasons; MLS-epoch, membership and race sub-cases are transient.
- **K9**: H1b's "author or basis mismatch" string merges `NotCurrentAuthor` with a probe/start race.

## 5. Two defects this work found, fixed before the mapping

1. **S2 refusals are dropped** (K12). A deterministic plan refusal (the per-branch rail in
   `append`, the ledger cap, the floor) leaves nothing parked, and the caller's retry is captured
   and answered `Scheduled` again, for ever. Fix: park the refusal for the scheduling request
   (bounded like a parked plan) and answer that request's next visit with it; and move the
   per-branch rail to S1b so the cheap stage refuses first. Agent 2's live Unconfirmed path first
   (their file); I mirror it in the Closing `save_overlay` with its two known fixes.
2. **Stale final delivery reports a durable Save as a refusal.** Native's final fence
   (`src-tauri/src/studio.rs:222-256`) withholds an expired or cancelled delivery; only
   `OverlayArchived` is marked durable (`durable_write`) and so answered "uncertain, retry
   exactly". A withheld `UnconfirmedOverlaySaved` (and the future Closing Save) comes back as a
   plain refusal although the work is saved. Fix: mark the Save responses durable too. Not live
   yet (the command is unregistered), so it can land with the mapping.

## 6. Tests the row asks for

For every reason: one test that produces it in the store, carries it through the receiver and the
actor's control reply, and asserts the native JSON (`eligibility` / `manualReason` for Flow H, the
new refusal field for Flow S), plus a mutation per split showing the cases no longer collapse.
Plus: a withheld Save delivery answered as uncertain; the S2 rail refusal reaching its caller
within two visits with no re-capture.

## 7. Order and ownership

1. Agent 2 agrees section 2 and 3's type, name and home, and the outcome carriage.
2. K12 and the stale-delivery fix (section 5), each with its regression.
3. The Flow S splits that are mine (K1, K2, K3, K4, K5 in my store stages), the type's
   construction at each site, and the carriage through the receiver.
4. Flow H: K6, K7 (by after-the-fact classification), K8, K9, the per-target last reason, the
   lifecycle-row rule.
5. Native fields (`src-tauri`), then Agent 4's TypeScript union.
6. The coverage tests, one per reason.

Open for review: whether the lifecycle row should ever show a runtime reason over the
classifier's (section 4), and whether `Retry` should be split by cause.
