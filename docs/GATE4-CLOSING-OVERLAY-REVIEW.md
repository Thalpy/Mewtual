# Gate 4: durable Closing overlay foundation

Status: user design review PASS at `d576af2`, 2026-09-14, with no required design changes.
The core/store implementation is complete for this foundation and awaits user implementation
review; no native overlay command is enabled. The 131 affected local tests, Clippy and three
isolated mutations pass as recorded in [HANDOVER](HANDOVER.md). The repository-wide ambient
dependency check still reports six baseline findings; full-repository acceptance remains open.
The review inspected source/design and did not execute overlay tests. Native exposure,
replay/disposition, provisional-preview overlays and Gate 4 acceptance remain outside its verdict.
The scheduling checkpoint `6b71d96`, with evidence
at `a40909e`, has passed review without required changes.

## Problem and next checkpoint

The design requires local edits while a checkpoint is waiting to open. Today ordinary Apply
correctly refuses Closing/Fault, and awaiting-tenure previews are read-only. A renderer buffer
does not provide the required vault-sealed save/restart guarantee.

The existing intent ledger cannot serve as an acceptance flag: ordinary Save persists its intent
before its signed source, so a failed Save can leave an intent that was never accepted. Current
replay deliberately returns `NoEvidence` for that case. Treating every pending entry as an
accepted overlay would turn an unsuccessful ordinary request into a later automatic edit.

The next implementation checkpoint should provide an explicitly accepted local branch for a
**known Closing document whose exact expected checkpoint can be constructed**. It covers Index
and Flipnote art operations through a core/store boundary, with durable restart and exact retry
tests. Actor scheduling, native exposure and automatic replay follow only after that boundary
has implementation evidence. This is a slice of the overlay requirement, not its completion.

This first checkpoint excludes a newcomer with only an unconfirmed preview. Its stored history,
expiry and attribution require a separate extension; a volatile preview cannot become a durable
base by passing its `epochId` to this API. It also excludes Fault, an unavailable expected seed,
unknown owner tenure, Create's multi-document transaction, signed repair and A-to-B-to-A authority.
Those cases retain their current behavior. An unavailable or over-budget local save must leave
editor work unsaved and visible; it must never report durable success.

## Reuse and the authority boundary

| Existing seam | Proposed use |
|---|---|
| `studio/epoch/settlement.rs` | Derive the exact expected seed from the checked Closing source, receipt and close. Source-version binding detects stale preparation. |
| `studio/admission.rs`, Index/frame writers and validators | Apply a domain operation on a detached local working graph with the same typed policy, semantic checks and exact size preflight. |
| `store/epoch_intents.rs` | Keep original envelopes and explicit overlay acceptance in one accounted, vault-sealed final record, under the same exclusive coordinator. |
| `store/epoch_recovery/inventory.rs` and creative references | Account final/temporary/replacement bytes and protect every retained base and operation CID after restart. |
| `studio/replay.rs` | Preserve the current distinction between signed historical evidence and a failed ordinary Save. Overlay replay needs a separate, explicit eligibility branch later. |
| Existing actor preparation/native custody | Reuse bounded preparation, lifecycle fences and invalidation when commands are subsequently connected. |

Introduce a distinct private-provenance `StudioClosingOverlayBasis`, built from the actual
source and a matching `StudioSettlementPlan` under checked current membership/tenure. It contains
the logical/channel scope, source physical ID and source-version fingerprint, exact receipt and
canonical expected seed. It grants permission to prepare local draft data only. Recheck the
same source and authority immediately before the first durable acceptance.

Do not install the separate successor, call `admit_local` on the Closing gate, sign/seal a shared
operation, publish a Registry pointer, modify the owner journal, retire an intent or prune source
history when accepting an overlay. Ordinary Apply retains its existing phase checks. No public
constructor accepts a caller-supplied receipt, tenure or projection as a substitute for the basis.

The detached local graph is an implementation detail of a separate overlay type. Reuse typed
writers and validators without constructing an editable `StudioEpoch`, forging an Open gate or
issuing a `VerifiedCheckpoint` from local bytes. Any markers used to validate that private graph
are draft bookkeeping only and are inaccessible to settlement/retirement. Return the base and
local effects with a distinct local-draft trust state, never an ordinary current-document view.

## Local record and limits

Use the existing `.intents` path, scope binding, sealing and budget ownership. Keep the current
`IntentLedger` wire encoding unchanged. Extend only its vault-local enclosing record:

1. Records with no extension keep their existing exact encoding.
2. An optional, explicitly versioned extension follows the current scope and ledger fields.
   A reader accepts exactly the old complete form or the new complete form; unknown versions,
   trailing bytes, duplicate entries and noncanonical order reject. Old readers fail closed on
   the extension. Decoding an old ledger never synthesizes overlay acceptance.
3. The extension contains at most one branch: its local author, immutable basis descriptor,
   exact receipt/seed, and ordered accepted operation IDs with their original local timestamps.
   The full author/domain envelopes remain in the ledger. Each annotation must refer to an exact
   matching ledger entry and the branch's actual local author. It is not a second ledger of ops.
4. Record an increasing checked sequence for each newly accepted request; exact retries reuse
   their sequence and timestamp. Neither hash order nor the current wall clock reconstructs
   editing order. Exhaustion refuses new writes; it cannot wrap or renumber accepted requests.

The initial limits are one branch and at most **256 accepted overlay operations per logical
document**, a seed bounded by the existing **2 MiB** checkpoint limit, and at most **64 KiB** of
extension metadata excluding seed bytes. The encoded complete record must still fit the existing
`MAX_RECORD_BYTES` (`5 MiB + 1024`), including the ordinary ledger, seed and extension. Existing
10,000-intent / 4 MiB operation bounds and the **64 MiB vault-wide physical intent limit** remain
in force. These limits are simultaneous, not additional allowances. A large existing ledger or
seed may therefore refuse an otherwise valid overlay. Admission checks before acknowledgement.

Charge the ordinary per-server content budget, including sealed framing, unpublished siblings,
record slots and the complete replacement peak. Do not consume settlement/protocol reserves or
create an unscanned draft file/cache. Failed I/O invalidates both budget generations as it does
today. Extend full inventory and conservative creative-reference enumeration to include the
captured base's references as well as every pending operation; superseded draft pixels remain
protected while their intent is retained. Corrupt/unsupported records fail closed for reclamation.

## Acceptance and retry transaction

Run through the existing exclusive store/server coordinator, with no network wait while held:

1. Check the actual local member and complete target/channel scope. For a new branch, obtain the
   checked basis above. For an append, require that same still-eligible Closing basis and author;
   a new receipt, fault or source change requires revalidation and cannot silently rebase it.
2. Rebuild the detached graph from its exact seed and all accepted operations in saved sequence.
   Bound input and work before cloning/parsing. Apply the proposed operation using the existing
   typed writers, semantic policy, full-envelope checks and exact checkpoint/recovery preflight.
   An invalid operation changes no durable record. Retain the original authored timestamp for
   restart rather than rendering a different source record on each replay of the draft.
3. A nonce already present as an ordinary unannotated intent is **not** upgraded into an overlay,
   even if its body is equal. Return a distinct conflict/ambiguous-request result. For an existing
   overlay ID, require the same complete request and branch identity; changing any envelope byte
   fails. Do not allocate another sequence, replace the seed or reset another request's state.
4. Prepare the ledger entry and its acceptance annotation in the same replacement plaintext,
   preflight all limits, then seal and atomically persist through the existing writer and sync
   barrier. Only successful durable completion returns a local-saved acknowledgement. No ordinary
   source, recovery, owner or Registry write accompanies this transaction.
5. A retry after uncertain post-rename completion reloads the exact authenticated record and
   syncs it and its parent, without requiring replacement headroom at the cap. Membership and
   vault/session checks still apply. If the matching accepted entry already exists, a now-changed
   source does not erase that acknowledgement: return that saved local entry, without appending,
   rebasing or claiming current edit eligibility. A different request still fails.

Successful acknowledgement means this device has durably saved a local operation and its base.
It says nothing about delivery, inclusion, settlement or other members' views. Cancellation before
final delivery may suppress the result after persistence; a same-request retry must resolve that
uncertainty. A failed response is not proof that no bytes landed.

## Restart, transitions and the bounded first implementation

Restart reconstructs the exact local branch from the sealed base and ordered accepted envelopes.
Compare its complete projection to the pre-restart projection in tests, including frame positions,
conflicts, tombstones, attribution claims and pixel CIDs. Local draft history remains a local
draft if current membership/tenure or source eligibility has changed; reading it must never
re-create an Open source or a live network-preview capability. This does not add a new offline
export authorization path for removed members.

The first implementation has **no automatic promotion or removal of annotated entries**. Extend
ordinary Apply to refuse an annotated operation ID unless a later, private overlay-promotion
path authorizes that exact envelope. Knowing its nonce must not bypass the local-only boundary.
Extend existing store retirement/manual-disposition code to hold annotated entries, including attempts
to retire a mixture of ordinary and overlay entries, until an explicit overlay-aware transition
is implemented. Ordinary intents keep their existing behavior. A seed marker, equal projected
value, source installation, export or eviction acknowledgement cannot erase local acceptance.

This temporary hold is acceptable for an internal foundation with no native command. It is not
acceptable as the final user-facing overlay lifecycle. Before enabling durable overlay Save in
native, implement and review all of the following:

- The actor/native local-save and draft-read result, bounded preparation ownership and lifecycle
  checks. A future explicit local-overlay request is distinct from ordinary Apply; command names
  and JSON are not callable contracts until entered in `FLIPNOTE-UI-HOOKS.md` as implemented.
- Automatic own-intent replay only into an independently verified, installed Open source.
  Recheck complete current signed envelopes and typed conflicts/deletions/creation dependencies
  on each step. A changed base requires conservative reconciliation or visible manual handling;
  a retained unconfirmed preview never supplies owner authority. Keep ordinary `NoEvidence`.
- Ordered replay, pause on another rotation/fault, and crash recovery between signed-source save
  and local-branch disposition. Preserve the original nonce/body/author and enough base/order
  evidence for later entries when an earlier entry is settled. Removing a ledger entry while its
  annotation or descendants still need it is invalid; the foundation deliberately holds it.
- A bounded manual inspection/copy/export/disposition path for work that cannot safely replay.
  An unsent local branch cannot be passed off as `StudioRecovery` signed historical evidence.
  No automatic expiry or pressure eviction of the sole accepted local copy.
- A separately reviewed extension for local work based on an awaiting-tenure preview, including
  what is persisted for restart, finite tail provenance, aggregate bounds and fresh authority
  before replay. This proposal does not change the existing preview eviction/lifetime contract.

## Required evidence for the foundation

| Case | Required assertion |
|---|---|
| Index and Flipnote Closing save/restart | Actual checked expected seed; exact local projection and envelope/order after reopen; canonical source, gate, recovery, owner and Registry bytes unchanged. |
| Uncertain write | Pre-write, post-rename and sync failures do not return success; exact retry establishes durability without duplicate effect or sequence. |
| False acceptance | An unannotated intent left by a failed ordinary Save stays unannotated and `NoEvidence`; annotation-only/missing-entry records reject. |
| Identity and eligibility | Wrong channel/source/version/author, nonmember, Unknown tenure, Fault and an unconfirmed preview cannot obtain a first-acceptance basis. Each fixture must pass earlier checks. |
| Replay order and semantics | Nonce hashes deliberately sort opposite the authored order; insert/replace/remove and dependent inserts still reconstruct in saved order. Invalid typed operations never become acknowledged entries. |
| Resource accounting | Existing ledger plus overlay reaches each combined limit; replacement peak and orphan temporaries count; both stale budget tokens reject after failed I/O; exact retry at full cap remains possible. |
| Pixel retention | Base-only, pending and superseded-operation CIDs survive cleanup and reopen; malformed extension prevents a successful incomplete reference scan. |
| Transitions | Installation, receipt-covered retirement and manual-disposition attempts cannot drop annotations or their required entries. A changed source allows exact saved-entry retry but refuses a new append on the old basis. |

Use actual store records and existing failure-injection seams. Mutate individual acceptance,
scope, ordering and accounting guards where a negative test could otherwise pass for an earlier
error; require the intended executed assertion failure and byte-for-byte restoration. Run affected
core/store suites and Clippy locally. Actor/native integration and its GitHub checks belong to the
following checkpoint; do not report them as covered by foundation tests.

## Review request

Review this design's bounded core/store checkpoint before implementation. Focus on accidental
promotion of a failed ordinary Save, a forged or stale basis, retry acknowledgement after source
change, unsigned draft content reaching authority-bearing types, codec downgrade behavior,
aggregate storage/reference accounting, and loss of ordered draft evidence during retirement.
Return PASS or numbered findings with severity, separating blockers to this foundation from the
explicitly outstanding native/replay/provisional/tenure/repair work. A PASS clears this design
checkpoint; it does not claim that the implementation or Gate 4 has passed.
