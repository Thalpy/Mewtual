# Gate 4 Agent 1: G4-A1-MAP, structured refusal reasons (design, revision 2)

Status: **proposal, design-reviewed once, nothing built.** It answers acceptance row G4-A1-MAP
(`GATE4-ACCEPTANCE.md`): "map runtime refusals to `StudioOverlayEligibility` /
`StudioOverlayManualReason` without collapsing distinct cases", with "store -> actor -> native
conversion coverage for every reason, including stale final delivery". Decisions that need Agent
2's agreement, because the types or files are theirs, are marked **[A2]**.

Revision 1's design review (Opus, static, 2026-10-07) returned **ready with changes**: three highs
(the handoff classification could not run where it was placed; a runtime reason in `eligibility`
would be untruthful; uncertain Save outcomes were lost on the `Err` path), four mediums and seven
lows. Section 9 maps each to its answer. The evidence is a read-only inventory of every Flow S and
Flow H refusal at `9c63bd6e`; its row ids (S1 to S26, H1a to H5, K1 to K13) are summarised in
appendix A.

## 1. What exists

- **The reason types** are Agent 2's (`catcoms-replication/src/studio/overlay/eligibility.rs`):
  `StudioOverlayEligibility { Transferable, Manual(StudioOverlayManualReason) }`, sixteen manual
  reasons, and `StudioOverlayUnconfirmedState`. Their contract is the **automatic handoff**: why
  it will refuse this draft, from durable state only; transient refusals are deliberately not
  classified (`store/epoch_studio/eligibility.rs:73-77`). Only the read-side classifier produces
  them, and the lifecycle row and the inspection carry them to native
  (`src-tauri/src/studio/lifecycle.rs:482-513`).
- **Every runtime refusal is a string.** Store errors are `AppError::Invalid("epoch studio: ..")`;
  the actor flattens control replies to `Result<_, String>` (`actor.rs:4392-4396`).
- **Distinct situations share one value** (appendix A, K1 to K11): `EpochScope` six ways at S1b,
  `stale_branch` six ways, `EpochClosed` both "a handoff is in flight" and "the source faulted",
  the probe merging `Imported` with `Unknown` tenure, and the successor check mapping eight manual
  reasons onto two errors.
- **The scheduled runtime throws reasons away**: the handoff records only a per-target backoff, and
  a refused S2 plan is dropped (K12, section 6).

## 2. The central decision: two vocabularies, one source of truth for each durable fact

`StudioOverlayManualReason` answers "why won't the automatic handoff take this draft". Most Save
refusals answer "why can't I author this now". So:

- **Flow H** refusals map onto the existing manual reasons where they coincide.
- **Flow S** refusals get a new type, provisionally `StudioOverlaySaveRefusal` **[A2: name and
  home]**. Where a Save refusal is about the **source or the tenure**, a durable fact the classifier
  also reports, it **embeds the manual reason** (`Durable(StudioOverlayManualReason)`), so the
  editor and the lifecycle row can never name one fact two ways (review M3: `Fault` was reused by
  value while the table invented `SourceFaulted`, and `OwnerChanged` was the classifier's
  `ReceiptChanged`). Save-only variants cover authoring facts only: the nonce, pixels, the preview,
  the branch, the rails.

## 3. Flow S: `StudioOverlaySaveRefusal`

Each variant carries an **action** rather than a class (review M4), extending the existing
`retry: "sameRequest"`: `sameRequest` (retry this exact request), `newTicket` (prepare again from
the current state), or `none` (cannot proceed until something else changes).

| variant | action | from | split needed first |
|---|---|---|---|
| `Durable(TenureUnknown \| TenureImported)` | none | S7 | none |
| `Durable(SourceMissing)` | none | S8 | none |
| `Durable(Fault)` | none | S9 | K4: split `prepare_settlement`'s faulted case (two forms) |
| `Durable(SourceNotClosing)` | none | S9 (phase not Closing: successor installed, rewound, restored) | K4 |
| `Durable(ReceiptChanged)` | newTicket | S9 `EpochAuthority` / `ReceiptConflict` | K5 |
| `NotMember` | none | S1, S26, ticket | none |
| `SourceAdopting` | sameRequest | S9 (`EpochClosed`, adopting) | K4 |
| `InstalledSource` | none | S10 | none (the budget-disagreement check beside it is `Err`) |
| `PreviewUnavailable`, `PreviewExpired`, `TailIncomplete`, `PreviewReplaced` | newTicket | S12, ticket | K10: `SyncError::Unauthorized` covers five provisional cases **[A2: sync layer]** |
| `MembershipChanged` | newTicket | S15 | none |
| `BasisChanged { during_detach }` | newTicket | S16, S25 | K3 |
| `TransferInProgress` | sameRequest | S17 (branch Prepared) | K4 |
| `OtherKindDraft` | none | S18 / K1(b), part of K2, `refuse_closing_draft` | K1, K2 (design 8.1's wording) |
| `LiveBranchSuperseded` | none | S18 / K1(c) | K1 |
| `StaleBranch` | newTicket | S20, S24 floor | K2 |
| `NonceConflict` | none | S4, S5, S24 | none |
| `OrdinaryIntentPending` | none | S6 | none |
| `BranchFull` | none | the per-branch rail | moved to S1b (section 6) **[A2: shared predicate]** |
| `PixelUnpublished`, `PixelInvalid`, `PixelMissing` | newTicket | S21, S25 | none |
| `UnconfirmedBranchesPerServer`, `UnconfirmedBytesVaultWide`, `ReferenceCapacity`, `StorageRefused` | none | the two 8.3 rails (`epoch_intents.rs:436`, `:445`), S22 cap, S24 ledger, S25 preflights | typed apart, not one string **[A2: the 8.3 refusal]** |
| `Retry` | sameRequest | S2, S11, S22 scan, S25 context changed | none |

Internal invariant mismatches (S3, S13, S14, S19, S23, the S25 mismatches, the mint's budget
disagreement) are **not** a refusal: they stay `Err`, so there is one channel for them, and their
text never reaches the refusal JSON (review M2).

**Carrier inside the app** (review M2). One new error variant,
`AppError::StudioRefused { reason, message }`, whose `Display` is today's message, so every
existing caller, log and test string is unchanged. Store sites construct it instead of
`invalid(..)`. Only the Save boundary converts it: `Err(StudioRefused { .. })` becomes
`Ok(Refused { reason, message })` on `StudioOverlaySaveVisit`, `StudioUnconfirmedSaveOutcome`
**[A2]**, the Closing equivalent, and the `BeginUnconfirmedOverlaySave` ticket. Nothing treats
`Ok` as saved today (the actor's `begin_delivery` already ignores the Save outcome), and
`unconfirmed_save_value` gains a `refused` state. Store and receiver code never re-derive a reason
from a string.

## 4. Flow H: classify in the worker, report beside eligibility

**Where classification runs** (review H1). The successor check runs in H2, on the worker, against
the source it just restored; its failure returns through `complete`, which has no store, so the
receiver cannot classify after the fact, and a later read could see a different source. So the
worker classifies, on the same restored source: replication's `overlay_successor_hold`
(`epoch/handoff.rs:321-371`, today `#[cfg(test)]` and pinned equal to `check_overlay_successor`)
is promoted, and H2 returns a typed refusal carrying its reason **[A2: shared replication code]**.
Fallback if Agent 2 prefers not to: classify only while `studio_handoff_is_current` still matches
the job's stamp, and record "unclassified" whenever the stamp moved or the hold returns nothing.

**Where it is reported** (review H2). Not in `eligibility`. That field stays classifier-only, so
the row and the inspection keep their "can never disagree" property (`lifecycle.rs:477-479`), and
`Transferable` keeps meaning what the classifier says. Beside it, a new field:

```
handoffAttempt: { state: "refused" | "backingOff", reason: ManualReason | "transient", retryAt }
```

- keyed to the H1 stamp (intent and source digests, tenure, MLS epoch), and **dropped as soon as
  that stamp is stale**, so it can never outlive the condition it reports;
- injected by the receiver into both the row and the inspection, in `StudioReceiver::control`'s
  fall-through, because `Server::studio_control_transaction` builds the row with no receiver
  state;
- bounded to the watch rail (16 targets) and pruned when a watch is evicted (review L).

The other splits: **K6** at the probe (`receiver/handoff.rs:605`): read
`server.observed_owner_tenure()` rather than `authoring_owner_tenure_start()`, so `Imported` and
`Unknown` stay apart, with no store signature change. **K8**: `NotCurrentAuthor` and
`ReceiptChanged` are reasons; the MLS-epoch, membership and race sub-cases are transient, matching
the classifier's mapping of the membership check (`eligibility.rs:191-196`). **K9**: H1b's
"author or basis mismatch" separates `NotCurrentAuthor` from a probe/start race.

## 5. Native and uncertain outcomes

**Stale final delivery and uncertain errors** (review H3). A Save can be durable and still come
back as a refusal by two routes: native withholds a complete `Ok` (cancellation, session lock, a
generation change; `studio.rs:217-262`), or the commit's write fails after a durable change and
returns `Err` (`unconfirmed.rs:183-199`; `UNCERTAIN_OUTCOME`, `CONTROL_REPLY_DROPPED`). Native
treats both as uncertain only for archive, release and dispose. Fix: route the Save command's
errors through a `classified`-style mapper, and set `durable_write` per outcome: **uncertain** for
`Saved`, `HandedOff`, `Disposed` and `Scheduled` (another visit can commit the parked plan);
**plain** for `Busy` and `Refused`. The Save has no `StudioDelivered`, so its withheld-delivery test
uses cancellation, a session lock or a generation change, not clock expiry.

`INTERFACES.md` gains the Save `refused` state, its reasons and actions, and `handoffAttempt`;
`lifecycle.rs`'s conversions are marked **[A2]**.

## 6. Two defects found on the way, fixed first

1. **K12, S2 refusals are dropped.** A failed plan parks nothing and clears the scheduled request
   (`catchup.rs:1438-1449`); the per-branch rail lives only in `append`, so S1b passes, recaptures
   and answers `Scheduled` again, for ever, paying an inventory scan, a PIX promotion and hold, a
   permit and a detached decode each time. Fix (sent to Agent 2 for their live path, mirrored in
   the Closing `save_overlay`): park the refusal keyed on the request fingerprint **and** the
   capture's intent digest, answer it only while that digest is current (a disposal that freed the
   cap must not be answered "refused"), expire it lazily without feeding `pending` or `wake_in`,
   and clear it wherever the scheduled request is cleared. Move the per-branch rail to S1b through
   one predicate shared with `append` **[A2]**.
2. **S1b refusals emit no `RefreshRequired`**, so after `InstalledSource` or `SourceNotClosing`
   the row can lag. Emit one on any refusal caused by a durable-state change (review M3).

## 7. Tests

Three layers rather than one end-to-end fixture per reason (about forty-five), which is
impractical (review):

1. **A store test per reason**, which is where a collapse is caught: each produces its exact
   variant, and a mutation re-merging each K split fails it.
2. **One real actor-and-native test per carriage path**: S1, S1b, a parked S2 refusal (using an
   S2-only refusal such as the ledger cap, since the rail moves to S1b), S3, the ticket,
   `handoffAttempt`, a withheld `Ok`, and an uncertain `Err`.
3. **An exhaustive native literal table** with a distinctness check, for both reason sets.

Mutations: each K split re-merged; each converted site reverted to `Err(invalid(..))`; K7
classified without the stamp while the source changes during H2; the parked refusal dropped
(asserting no second promotion or permit); the Save removed from `durable_write`.

## 8. Order and ownership

1. Agent 2 agrees the type, its name and home, the carrier, and the K7 choice (sections 2 to 4).
2. K12 and the `RefreshRequired` gap (section 6), and the uncertain-outcome mapper (section 5).
3. The Flow S splits in my store stages (K1 to K5), the `StudioRefused` carrier at each site, the
   Save-boundary conversion, and the ticket.
4. Flow H: the worker classification, `handoffAttempt`, K6, K8, K9.
5. Native fields and `INTERFACES.md`, then Agent 4's TypeScript union.
6. The tests in section 7.

**[A2] in one list:** the Save refusal type's name and home; `unconfirmed.rs` (carrier conversion,
K12 there); the replication oracle promotion (K7) and the shared rail predicate; K10's sync split;
the typed 8.3 rail refusals; the native `lifecycle.rs` fields.

## 9. Revision 1's design review, answered

| finding | answer |
|---|---|
| H1 the after-the-fact classification has no store, no type and no stable bytes | 4: classify in the H2 worker on the same source (promote `overlay_successor_hold`), or a stamp-gated fallback recording "unclassified" |
| H2 a runtime reason in `eligibility` is untruthful and untestable | 4: `eligibility` stays classifier-only; a separate stamp-keyed `handoffAttempt` |
| H3 uncertain outcomes lost on the `Err` path | 5: a `classified` mapper for Save errors; `durable_write` per outcome |
| M1 K12 fix needs a staleness key; the test contradicted the rail move | 6.1; 7 layer 2 uses an S2-only refusal |
| M2 no store-to-receiver carrier; `Internal` doubled a channel; the ticket uncovered | 3: `AppError::StudioRefused`, no `Internal`, the ticket included |
| M3 one durable fact two names; no `RefreshRequired` on S1b refusals | 2: `Durable(manual reason)`; 6.2 |
| M4 missing and mis-scoped cases; class column mixed "new ticket" with "cannot proceed" | 3: `SourceAdopting`, `SourceNotClosing`, the two typed rails, the budget check as `Err`, an action column |
| L appendix missing; count mismatch; K6 location; per-target maps unbounded; K8 direction; INTERFACES; privacy | appendix A; eight successor reasons; K6 at the probe; bounded to the rail; K8 stated; INTERFACES in 5; no new disclosure, internal text kept out of JSON |

## Appendix A. The inventory, by row

S-rows are Flow S sites in `store/epoch_studio/overlay.rs` (O) and `overlay_capture.rs` (C);
H-rows Flow H. Line numbers at `9c63bd6e`.

| id | site | refusal |
|---|---|---|
| S1 | O:323 | not a current member |
| S2 | O:335 | stale budget / inventory needs reconciliation |
| S3 | O:351-354 | `EpochScope`: record under another channel label |
| S4 | O:386-391 | `IntentConflict`: transferred op id, different bytes |
| S5 | O:408 | `IntentConflict`: live-branch op id, different bytes |
| S6 | O:432-437 | op pending as an ordinary intent |
| S7 | C:575 | tenure Imported / Unknown |
| S8 | C:576-577 | Closing source missing |
| S9 | C:581-584 | `prepare_closing_overlay`: `EpochAuthority`, `EpochClosed` (adopting, faulted, not Closing), `ReceiptConflict` |
| S10 | C:605-608 | Unconfirmed beside an installed source |
| S11 | C:592-616 | probe I/O, parent not a directory, budget disagreement |
| S12 | C:617 | the preview mint's own error (lost, expired, incomplete, replaced) |
| S13 | C:619-620 | `EpochScope`: Unconfirmed basis for another target |
| S14 | C:622-641 | basis for another group, device, or without Unconfirmed provenance |
| S15 | C:642-649 | minted under another membership |
| S16 | O:461-463 | `basis_changed` |
| S17 | O:480-482 | `EpochClosed`: live branch Prepared |
| S18 | O:483-485 | `EpochScope`: the live branch's basis is not the fresh one |
| S19 | O:488-493 | `admit_new_branch`: `EpochScope` / `EpochBound` |
| S20 | O:496 | `stale_branch` |
| S21 | C:399-405 | frame PIX unpublished, mis-declared, invalid, wrong size |
| S22 | C:409-411 | promotion failed; reference scan incomplete or over bound |
| S23 | C:475-492 | media for another request; no current owner |
| S24 | C:266-290 | S2: decode, another channel, ledger cap, branch mismatches, `append` (`EpochScope` floor and fingerprint, `EpochClosed`, `IntentConflict`, `EpochBound` branch full, `EpochAuthority`) |
| S25 | C:722-812 | S3: kind mismatch, mismatches, context changed, re-mint, PIX gone, base missing, write errors |
| S26 | O:286-288 and the ticket | another channel, tenure, op bound, unknown channel, Closing draft on a preview, not a draft Save, not a member |
| H1a-H1g | `store/epoch_studio/handoff.rs`, `handoff_capture.rs` | no live branch; author or basis mismatch; Prepared resolution; no tenure; unavailable Flipnote; authority mint; capture source missing |
| H2 | `handoff_capture.rs:281-303`, replication `preparation.rs` | channel changed; Unconfirmed provenance; successor check; local policy; `IntentConflict` |
| H3 | `handoff_capture.rs:200-205` | live context moved |
| H4 | `handoff_capture.rs:230-232` | signing incomplete; `finish` |
| H5 | `handoff.rs:302-442` | context changed; destination missing; base blob release; Index objects; capacity; intent source changed |

Collapses: K1 `EpochScope` (six cases), K2 `stale_branch` (six), K3 `basis_changed` (S1b versus
during the detach), K4 `EpochClosed` (in flight, adopting, faulted, not Closing), K5 Closing-mint
`EpochAuthority`, K6 the probe's tenure, K7 the successor check (eight reasons onto two errors),
K8 `live_authority` / `check_live`, K9 H1b, K10 `SyncError::Unauthorized`, K11 the creative hold
string, K12 the dropped S2 refusal, K13 native's final-delivery check.
