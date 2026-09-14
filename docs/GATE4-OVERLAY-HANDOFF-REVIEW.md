# Gate 4: atomic handoff of an accepted Closing overlay

Status: proposed core/store design, 2026-09-14; awaiting user adversarial review.
No implementation or native command is supplied by this checkpoint. The Closing overlay
foundation at `b1b0ec9` is accepted; the user's review of `65db6ac` closes OVERLAY-TEST-001.
See [the foundation record](GATE4-CLOSING-OVERLAY-REVIEW.md) and [HANDOVER](HANDOVER.md).

## Problem and bounded outcome

An accepted local branch currently survives restart, but cannot enter ordinary Apply or retire.
Removing that exclusion before its operations have been durably signed would recreate the failed
ordinary-Save ambiguity the foundation eliminated. Removing entries individually would also break
the saved branch: its decoder and ordered reconstruction require every annotated envelope.

The proposed next implementation transfers one complete branch into the **pristine, independently
verified installed successor named by its basis receipt**. It persists all signed operations in one
source replacement, then releases their overlay hold together. The original ordinary ledger entries
remain pending until the existing receipt/recovery rules settle or dispose them. This is shared
provisional work, not receipt finality or a delivery acknowledgement.

This first handoff deliberately has a narrow automatic eligibility rule. An Open source with
additional operations, a different seed/receipt, a nonadjacent successor, ambiguous authority or
insufficient capacity leaves the full branch retained for reconciliation/manual handling. An exact
retry of an already committed handoff is a different case and must tolerate subsequent source edits.
There is no partial automatic replay, synthesized signed recovery, silent rebase or implicit discard.
This design does not finish the user-facing overlay lifecycle or enable its Save button.

## Audited seams and required changes

| Current seam | Consequence for the handoff |
|---|---|
| [Core overlay](../crates/catcoms-replication/src/studio/overlay.rs) | `checked_entries` requires all annotations and full ledger envelopes. Keep the complete branch until transfer; do not delete a prefix. |
| [Ordinary replay](../crates/catcoms-app/src/studio/replay.rs) | `choose` requires exact historical signed evidence. Preserve `NoEvidence`; do not pass local draft data as `StudioRecovery`. |
| [Replay worker](../crates/catcoms-app/src/studio/receiver/replay.rs) | Its order is a dependency traversal of ordinary ids, and it calls ordinary Apply one operation per turn. It is not the overlay's saved sequence or a batch committer. Active/prepared overlay ids must be excluded explicitly before future worker integration. |
| [Studio store](../crates/catcoms-app/src/store/epoch_studio.rs) | Ordinary edit validates before its intent barrier, signs, then persists the source. Reuse those typed checks and the existing source writer in a separate private batch path. |
| [Intent persistence](../crates/catcoms-app/src/store/epoch_intents.rs) | Keep the same vault namespace, full record accounting and sync-only retry helper. No second operation ledger or extra quota pool. |
| [Retirement](../crates/catcoms-app/src/store/epoch_intents/retirement.rs) | Active/prepared annotations remain held. After a complete handoff, the unchanged envelopes become ordinary pending intents; handoff itself removes none. |
| [Recovery disposition](../crates/catcoms-app/src/store/epoch_studio/recovery_disposition.rs) | Requires actual signed historical envelopes. Local draft bytes cannot satisfy it. Manual overlay handling is still a separate obligation. |
| [Detached source preparation](../crates/catcoms-app/src/store/epoch_studio/preparation.rs) | Captures carry authenticated bytes and public context, not device/MLS secrets. Future batch preparation must preserve this restriction. |
| [Initial publication](../crates/catcoms-app/src/studio/publication.rs) | Existing Save advertises at most two packets. Do not put a 256-operation handoff into that path or enlarge its network window. |

## Eligibility and private preparation

The internal Server adapter reads actual membership, device identity and observed owner tenure
from the live sync instance. A request can identify the target and saved branch fingerprint; it
cannot provide author, seed, receipt, tenure, envelopes or an editable source. The store authenticates
the complete intent record and actual current source under the existing exclusive coordinator.

A new handoff requires all of these independently:

1. The branch belongs to the actual local author and complete server/group/type/logical/channel
   scope. Every saved sequence, original timestamp and full author/domain envelope validates.
2. The canonical source exists, is Open, and is the adjacent physical successor selected by the
   saved receipt. Its installed opening receipt and seed exactly match the branch basis. The
   receipt also verifies against the live independently observed owner tenure; a local extension's
   receipt, a seed hash alone or a provisional preview cannot authorize this step.
3. The successor has zero current signed operations and its complete typed projection equals
   the projection parsed from the saved base. Both checks are required: equal displayed values
   cannot hide intervening history. The comparison includes source claims/conflicts and all
   Index/frame structure, not just the visible title or raster.
4. All accepted entries can be applied in original sequence to a separate candidate derived from
   that actual installed source, using the existing local policy, typed writer/validator, domain
   marker rules, signatures and exact admission/checkpoint/recovery limits at every step. Preserve
   original author, nonce, body and timestamp. A refusal anywhere discards the entire candidate.
5. Existing Index object-reference checks still apply. A PutObject must not manufacture an absent
   Flipnote or bypass Create's transaction; missing or unavailable referenced source holds the
   branch. Unsupported operations remain unavailable.
6. Preflight the complete proposed intent states and signed source replacement, including all
   framing, old finals, orphan siblings and replacement peaks, with ordinary content allowance.
   A branch that fits local storage may not fit a shared epoch; that is a retained hold, not success.

The candidate is private and grants no network publication or receipt authority. Before any write,
recheck actual source bytes/version, intent bytes/version, actor, group, owner, MLS epoch and tenure
under one exclusive custody window. A changed Closing basis is not silently renewed during handoff;
the saved original basis identifies the branch, while independent live installation authorizes the
destination. The handoff does not relax first/append source-version checks or exact local retry.

## Durable state and codec

Extend only the local enclosing intent record. Keep the existing ledger codec, outer extension tag
and original no-extension form. Accept inner overlay version 1 as Active. A new inner version 2 has
an optional Active/Prepared branch, an optional compact completed-transfer acknowledgement, and a
monotonic local retry floor. Old readers reject version 2 rather than ignoring its hold state.

| State | Retained content | Permitted next action |
|---|---|---|
| Active | Full existing basis, seed, ordered annotations and matching ledger entries | Exact local retry; eligible append; new checked handoff preparation |
| Prepared | The full Active data plus destination epoch/id, opening receipt hash, source-before fingerprint, branch/ledger digest and expected full-envelope and signed-operation hashes | Resolve this specific handoff before ordinary replay or replacement can erase its evidence |
| Completed acknowledgement | Original branch fingerprint, author, original per-request id/full-envelope hash/sequence/timestamp, destination epoch/id and handoff outcome; no seed or signed-authority type | Acknowledge exact saved requests as handed to shared history, without a new edit |

At most one Active/Prepared branch and one completed acknowledgement exist per logical document.
Each manifest has at most 256 entries. The original 2 MiB seed, combined 64 KiB extension metadata,
5 MiB + 1024 complete plaintext record, operation ledger, 64 MiB physical intent and ordinary server
limits remain simultaneous. The completed manifest consumes that existing metadata budget. Version 2
must bound counts/lengths before allocations and reject inconsistent states, duplicate ids, unknown
tags, scope/body mismatches and trailing data. It cannot turn a decoded Prepared flag into authority.

After transfer, the full ledger remains until ordinary retirement; the compact acknowledgement
does not require those entries to remain forever. It proves only a local prior handoff outcome.
A new active branch can coexist with the previous completed acknowledgement. A later completed
handoff replaces that acknowledgement and raises a persisted `minimum_new_basis_closed_epoch` to
its destination epoch. New branch admission requires a basis at or above this floor, with actual
current eligibility; it does not infer receipt/owner authority from the floor. Thus an old forgotten
retry cannot become a fresh branch after a rewind. Older unmatched requests return stale/unknown,
never success or a guessed new operation. Exhaustion rejects; the floor cannot wrap or reset when
the ordinary ledger empties. This local duplicate fence may conservatively hold drafts after a
rewind; a later reconciliation design must preserve the same protection.

The future save result must distinguish retained local draft from completed handoff. Do not fabricate
a `StudioLocalDraft` after dropping its base, or claim settlement from the completed acknowledgement.
No JSON shape or native method name is made callable by this proposal.

## Write order and interrupted handoff

All three barriers use the existing accounted atomic writers and sync behavior. No packets escape
the private candidate before the source and transfer record are durable.

1. Persist Prepared while retaining the complete branch and ledger. Failure returns no handoff
   success; uncertain completion invalidates both accounting views and requires reconciliation.
2. Persist the complete candidate signed source in one atomic replacement. There is no durable
   signed prefix and no per-operation removal from the overlay. Failure leaves Prepared/full draft.
3. Authenticate and flush the actual source, require every manifest entry as an exact current signed
   envelope **and the same complete signed-operation digest**, then replace Prepared with the compact
   completed acknowledgement. The signed digest binds the delta and original timestamp/provenance
   that an author/domain envelope alone does not necessarily encode. Keep the same complete
   pending ledger; only its overlay exclusion is removed. Verify conservative source-plus-intent CID
   enumeration protects all base and operation references before releasing the base. Only then may
   the adapter report handoff and normal shared-history delivery become eligible.

| State observed after reopen | Required resolution |
|---|---|
| Active, original source | No handoff happened. Recompute eligibility normally. |
| Prepared, exact recorded source-before, none of the branch's ids in its current signed log | Revalidate and reproduce the exact recorded signed-operation manifest, or durably return to Active without claiming handoff. A candidate with different signed bytes cannot overwrite the pending manifest or count as that attempt. |
| Prepared, same destination physical epoch with changed source, no branch ids and no conflicting same-id envelope | The candidate is stale. After authenticating the complete retained signed log, durably return to Active with the full draft; any next attempt starts fresh. Do not block unrelated source progress indefinitely on a candidate that never became current. |
| Prepared, all exact envelopes and signed-operation hashes in the named destination's current signed log | Flush source, complete step 3 without reapplying; source may now be Closing or contain later operations. This resolves saved evidence rather than authorizing new edits. |
| Prepared, partial matches, conflicting body/signed digest, missing source or different physical epoch | Retain the full branch, report a hold and do not guess completion, reapply a prefix or clear the record. |
| Completed acknowledgement after uncertain final sync | Sync the authenticated record without allocating replacement headroom; return the exact stored outcome. |

The Prepared state is a **source replacement fence**. Every path that can replace/prune that target
source must resolve it first: ordinary owner rotation, frozen-owner takeover, receipt/page installation,
adoption and any future repair path. Ordinary incoming operations and receipt/Fault recording can
continue if they retain the full signed log. They cannot erase the destination while Prepared remains.
This is a per-document hold, never a server-wide stop. All-exact completion can release the hold even
in Closing/Fault, with current membership and full source/manifest checks; it does not clear Fault.
No normal writer or an arbitrary supplied id list may bypass the fence. The implementation review
must enumerate actual replacement entry points, including restart paths, rather than relying on a
check only in the background owner worker. Missing/corrupt intent state must fail that fence closed.

The current write convergence point is `save_studio_source_reusing` in `store/epoch_studio.rs`,
also reached by `save_studio_source`. Destructive destination selection currently occurs through
`checkpoint_successor` in `store/epoch_studio/rotation.rs` and `adopted_successor` in
`store/epoch_studio/adoption.rs`; frozen-owner takeover uses the rotation coordinator. The
`receive.rs` page path and `source.rs` packet path update the current physical source and must
preserve its signed evidence. Install the fence at coordinator entry and at this shared write
boundary, with a private checked handoff capability for the candidate write, not a caller boolean.
Do not reject every ordinary rewrite just because an overlay exists: Active permits installation
of its expected successor, and Prepared permits evidence-preserving ingest/sealing.

Prepared must also be resolved before serving or publishing operations from its destination,
including generic page/current-tail service and ordinary retry sending after restart. Checking only
the new handoff caller would let the existing source services advertise a batch between steps 2
and 3. Retry servicing may attempt resolution under normal custody or return a per-document hold;
it cannot spend a received hint as evidence or wait on network while retaining vault custody.
Returning to Active with no matching ids is safe only while this publication fence and the shared
write fence have prevented a committed batch from being exposed and subsequently erased.

At every crash point there is either a complete local branch, or a durably signed source plus the
ordinary pending ledger. The final transition may release the redundant local base; it never retires
an operation. Existing recovery-first receipt settlement owns eventual ordinary intent removal.

## Runtime and UI obligations before exposure

The proposed implementation checkpoint is core/store only, invoked explicitly by tests. It must not
run a 256-operation synchronous batch in the live actor. Before enabling background/native use,
prepare typed candidate work under bounded shared process custody, retaining permits through worker
cancellation and accounting input/output simultaneously. Device and MLS secrets stay with the actor.
Signing/finalization must have a measured bounded custody cost or be split into cancellable bounded
turns with private candidates and final source/version rechecks; timeouts cannot free live workers.
Do not cache several complete mutable sources or introduce an unaccounted overlay preparation pool.

After durable handoff, use existing authenticated current-tail discovery/paging for delivery. Preserve
the at-most-two initial Save packets and existing per-turn/network limits; do not queue one giant
overlay message. Rebinding, invalidations and completed replay keys must include the new intent
generation so saving a draft on an already watched epoch cannot be missed by the ordinary cache.

Before UI Save is exposed, finish and review manual inspection/copy/export/disposition of retained
branches, stale/rewound-base reconciliation, finite native request/lock/server-switch custody and
PIX reference tests through actual cleanup. Unsupported automatic cases must be visible and recoverable.
Never label a retained local draft as shared, or a shared pending intent as receipted. A newcomer with
only an awaiting-tenure preview remains outside this base format; its durable evidence/lifetime needs
the separately reviewed extension already required by the foundation.

## Required implementation evidence

| Case | Independent assertion |
|---|---|
| Index and Flipnote handoff | Real verified installed seed; complete ordered branch with original attribution/timestamps; one durable signed source replacement; ledger still pending; local hold removed only afterward |
| Authority and eligibility | Forged/old-owner receipt, Unknown tenure, synthetic epoch zero, wrong channel/physical id, nonadjacent head, extra history with equal displayed values, and a preview cannot mint the private plan |
| Whole-branch validation | Deliberately reverse nonce hash order; dependent insert/replace/remove and Index object checks; a late invalid/over-cap entry leaves all canonical source bytes unchanged |
| Crash matrix | Fail before/after each Prepared/source/completed write and every sync; reopen, resolve actual record state, preserve full work and avoid duplicate signing/effects |
| Replacement fence | Exercise each real rotation/takeover/adoption/receive replacement path after interrupted source commit; it must complete transfer or retain the source, including after restart |
| Publication fence | Generic current-tail/page service and ordinary retry sends cannot expose the Prepared batch after restart; all-exact resolution releases the hold only after the final durability barrier |
| False completion | Missing/partial/conflicting envelope, seed marker, matching projection or receipt hint cannot release Prepared or erase the base |
| Retry and bounds | Full-cap sync-only completed retry; different body/author/basis cannot acknowledge; old-reader rejection, malformed manifests, combined budgets and replacement/orphan peaks |
| Old retry after rollover | Replace a completed manifest with a later one, then rewind/reopen; an old request cannot create a branch below the persisted floor |
| References and lifecycle | Base-only, pending, superseded and removed-frame PIX remain enumerable across both source and intent transitions; exact retry does not require released draft data |
| Ordinary behavior | Existing failed-Save `NoEvidence`, signed replay, retirement and manual recovery tests still pass; the handoff cannot delete ordinary entries in a mixed ledger |

Use isolated mutations for completion, replacement fencing, ordering, retry floor and quota guards
where an earlier failure could mask the intended check. Require executed assertion failures and
byte-exact restoration. Record core/store checks separately from the later actor/native evidence.

## Message for adversarial design review

```text
Please review docs/GATE4-OVERLAY-HANDOFF-REVIEW.md as the next Gate 4 design checkpoint.
The foundation is accepted and OVERLAY-TEST-001 is closed. No handoff implementation exists yet.

Challenge the pristine verified-successor eligibility rule, atomic whole-branch source write,
Prepared replacement fence across every restart/installation path, all-exact completion proof,
transfer to ordinary pending intents, bounded acknowledgement/retry floor, and reference/budget
accounting. Check whether every crash outcome retains recoverable work without duplicate edits
or invented owner/receipt authority. Compare the proposed reuse with the linked source seams.

Return PASS or numbered findings with severity and concrete required changes. Distinguish defects
in this core/store handoff from explicitly deferred actor scheduling/native/manual/reconciliation,
provisional-preview and repeated-owner-tenure work. A PASS accepts this design only, not Gate 4.
```
