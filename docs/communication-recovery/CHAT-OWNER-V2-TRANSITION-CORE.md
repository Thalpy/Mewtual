# Exact public body-transition math (source checkpoint; validation paused)

The prior public state/header/body codecs check signatures and data binding. The new bounded
math derives an exact body result from the preceding mathematical state. Source is authored
under `owner_state::transition`; compilation/tests/builds remain paused
and all ten authored regressions are UNRUN. Read-only design and final actual-diff reviews are
clear for this DATA-only boundary. This is not executed acceptance, a selected governor or
production activation.

## First slice and trust boundary

Implement PUBLIC replay mathematics for Founder/Add/Append/Remove, with at most64 CURRENT slots,
each slot's OWN selected tip HEADER identity and exact frontier. Current-slot bounds do not limit
lifetime admissions or original history. No raw-current-image restore constructor may bypass
sequential derivation from genesis. Unchanged slots retain their own selected tip, not whichever
global header most recently advanced another admission.

This mathematics cannot prove root provenance, a selected prefix, private MLS execution, complete
cross-batch accepted lookup, mandatory control payment, current Review cases or an original native
publication domain. Its objects/results MUST be named DATA, never Verified/Current/approved. A
generic public math API must be unmistakably untrusted analysis data, not a root constructor.
The source-bound Sync creator bridge is DEFERRED; it must later seed only through actual
`FrozenFounder`/original node validation. The recipient/cold governor constructor remains absent until actual opaque
archive-root/current-prefix custody is available. A downloaded self-signed Profile cannot create
that future selected-source wrapper. These APIs issue no positive token and attach no publisher.

The agreed first API is public `owner_state::transition::ReplayMath` DATA. Its initializer takes
an explicit typed Profile, Founder Admission, header1 and image and checks complete consistency;
that caller root remains an UNTRUSTED trust decision, never an `ArchiveRootCapture`. There is no
generic current-image initializer. `derive_successor(&self, header, body, image)` returns immutable
candidate `StepData`: the next math object plus bounded append-node or removal-cut DATA. It never
advances the original object, chooses a fork winner or publishes. Unsupported Review returns a
dependency refusal; no callback/bool can substitute cold case/accepted lookup. A narrow actual
creator Sync bridge is deferred; no substitute host trait/source wrapper exists here.
Parent owns archive layout/storage/source work; Factory owns destination/catalog. Existing
`owner_state` gets only the reviewed child-module declaration, with no codec/byte changes.

## Exact derivation

Every input signature representation is verified before duplicate/conflict handling. The expected
group/Profile/full original issuer/tenure context is supplied separately from candidate headers,
never inferred from whichever candidate arrived. For the actual creator bridge it comes from the
actual genesis producer; generic math expectations remain DATA and confer no trust. Genesis
requires exact Profile/Founder-body/header/state binding and one context-empty slot/frontier.

For a successor require the exact current header CID as parent, checked sequence+1, supported
unchanged issuer/tenure/Profile/group, exact body kind/CID and valid actual signature. Derive the
whole expected canonical state: exact context, parent, previous signed state hash, sequence, kind,
body CID, MLS epoch and ALL ordered current slots. Compare the whole received state and signed
result hash to that derivation. Valid self-signatures or matching changed rows alone are insufficient.

* Add accepts only an Add-origin Admission body, with before epoch equal to current MLS epoch and
  checked after=before+1. Full subject must not already be current; current capacity must be<64.
  Install exactly the new incarnation/body CID, empty frontier and tip BODY==Admission, then set
  its selected tip HEADER only after verifying the actual new Admission header. Every old slot
  and selected tip stays exact. Rejoining a removed full signer creates a new Add incarnation;
  no old frozen Admission becomes open again. Recipient public replay relies on the pinned
  finalizer's attestation; actual producer observation/selected KP is a separate signing boundary.
* Append addresses one exact ACTIVE Admission context. Its previous tip HEADER and old count/root
  must match that admission's OWN internally retained frontier/source. No caller frontier label
  can replace it. Apply canonical1..64
  leaves through actual bounded tree math, compare exact new count/root, then update only that
  slot's log tip BODY and selected HEADER. Epoch and every unrelated slot/tip remain unchanged.
  This math does not establish cross-batch uniqueness or exact original action acceptance.
  The repeated-leaf regression deliberately demonstrates that a mathematically valid second
  append is not proof of protocol idempotency; only the real complete accepted lookup can do that.
* Remove addresses the exact active incarnation/full subject. Match actual current epoch and
  checked+1, that slot's selected tip HEADER and full count/root frozen frontier. Remove only
  that current slot, retain every other tip, and return the exact cut as DATA for future archive
  retention. Removing the current issuer is unsupported. A missing/frozen incarnation cannot
  append/remove again. Historical cuts/decisions are not deleted just because hot slots omit them.

Review and exact retry are not guessed from an absent hot row. They require selected complete
cold case/action/acceptance lookup and immutable original outcomes. An initial Reject cannot undo
known finalized acceptance/approval; reviewed permission cannot modify a frozen LOG cut. Until
those real dependencies exist, this slice must refuse Review rather than create a fake lookup
witness, callback-returned was_accepted bool or finite lifetime cache that silently forgets facts.

## Sequence/control and contradiction integration that remains mandatory

Pure mathematics can reject u64 overflow but cannot certify prepaid remaining control space.
Actual signing/install must hold the original control account: ordinary work must leave checked
sequence space for EVERY already-issued cut/review/retirement obligation. Byte/row/journal/header
escrow and original operation retry custody likewise precede signing. No raw requested count or
"reserved=true" input substitutes for a real reserve. This production gate stays explicit.

The actual pinned governor must authenticate a known parent BEFORE body loading. Two different
valid successor semantic CIDs under that SAME pinned context/known parent/sequence establish
negative contradiction even when either body is missing/invalid. Header-invalid or foreign-context
data cannot poison the source. Equivalent valid signature variants are one semantic child. Unknown
parents stay incomplete and cannot allocate an unbounded candidate/fork cache.

That negative consumer must retain the actual original source/root/scope and shared
`ProjectionGroupView` revoker together, cancel that group in O(1) before asynchronous repair, and
retain/poll the original close completion before reporting drain. It must never look up a newer
view or close another equal-label group's barrier. Durable two-header contradiction escrow and
complete common-prefix repair are required; body failure never erases the headers. The pure math
slice cannot manufacture this bound negative wrapper from decoded labels or accept a caller's
arbitrary group revoker. Until actual root/prefix/cold dependencies exist it performs no such
governance mutation, positive issuance or contradiction-state clearing.

## Bounded hot objects and tests

Preflight current slots<=64 before copying. Each hot vector requests capacity64 explicitly so
inserting the64th item cannot silently grow a33-slot allocation to66/128. A slot carries the
public136-byte image data, its own32-byte tip header and fixed64-entry frontier. There is no
parent-object Arc chain or whole-history cache. Define a conservative1MiB logical policy invoice
and a compile-time named-layout bound (must fit that policy), counting these simultaneous objects:

* TWO64-slot hot-vector allocations at `size_of(HotSlot)` plus TWO `ReplayMath` fixed objects and
  four max256-byte Profile/header group payloads.
* FOUR max9244-byte state buffers for source/candidate/received/revalidation overlap, plus one
 64-slot public-image scratch vector at `size_of(CurrentAdmissionSlot)`.
* Up to191 `CreatedNode` records and TWO fixed `PreparedAppend` objects/frontiers, separately
  from the frontiers already included in hot slots. A removal's fixed cut/frontier output is
  included in the same checked worst-case scratch invoice, never an unbounded closed-admission map.
* TWO64-leaf packed/input allocations at `size_of(AcceptanceLeaf)`, one max3572-byte append
  encoded body, and eight complete Profile/header/Admission/Removal encoded bounds for bounded
  input/semantic/signature/genesis/prior-parent/body-ID scratch and fixed result bookkeeping.

This is deliberately conservative about overlap and layout. It does not claim allocator/RSS,
crypto implementation/SDK/private MLS heaps, a global pool debit or a live quota. The actual host
must pay source+candidate and input/output ownership before calling; the founder64KiB allowance
cannot substitute. Cold pages/lookup, accepted action bodies/ordinary semantic work, control
journals and lower/publication retention remain separate. The pure math checks every bounded
input before candidate cloning and leaves the original cursor unchanged on any error.

Authored UNRUN regressions should cover genuine signed interleavings across multiple admissions,
unchanged own-tip preservation, bad parent/context/signature/body/result fields, same-key rejoin,
frozen append/removal, issuer removal, maximum current64 with unbounded lifetime turnover,
tree/count/sequence boundaries and transactional refusal. Tests must explicitly distinguish public
finalizer-attestation replay from actual MLS producer observations and Data math from a selected
governor. Later integration additionally needs missing/invalid-body signed forks, old equal-label
scope/group replacement, exhausted ordinary load with controls progressing, cold exact acceptance/
Review completeness and native barrier/journal crash ordering. None is claimed by pure math.
