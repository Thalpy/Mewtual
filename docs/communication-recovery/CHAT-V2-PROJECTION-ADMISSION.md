# Default-closed v2 projection admission

7 November status: compilation and focused execution are permitted. The general Core
issuer passes ten production-verifier tests; its Draft/signing extension is now declared
and all 252 tests in the broader chat subset pass. This document's original projection mechanisms
remain a Core boundary, not a mounted owner host, durable negative journal or native/UI
adapter. General Proven Delete remains deliberately refused until paired durable custody
exists. [Current evidence](../HANDOVER.md) and
[general issuer](CHAT-V2-GENERAL-OPEN-ISSUER.md) supersede earlier paused-test notes.

## Why a separate boundary is necessary

ActionState::Valid, ordinary historical authority and exact Delete approval are monotone semantic
facts in the inactive core. They do not prove that an action may affect the current projection.
Raw actions, pages, snapshots and semantic causal knowledge remain available for retained history.
The existing projection API must be documented as semantic/audit-only, not reused as active-v2 UI.
The new guarded API defaults closed, including after snapshot restore even if semantic witnesses
were supplied. No decoded status/bool or current roster creates positive projection authority.

Each retained entry prepays fixed logical slots for one opaque projection-admission token and one
negative prior-Delete capsule. Future host fees must separately cover actual shared scope/control
allocations, page caches and worst retained overlap; sizeof(entry) alone is not a physical invoice.
No production activation/complete memory claim is made before that host exists.

## Original views, incarnations and point of use

One negative-only projection view belongs to this actual ledger instance. Beginning a new view
irrevocably closes the old one. Public view creation grants no positive token; only the later real
v2 verifier can construct the opaque token inside the chat boundary. The token binds exact realm,
action, full signer, Grant/Basis, Create target, Profile/Admission and original view identity.
Provisional tokens additionally observe that exact Admission's original OPEN lifetime. Removal
closes A1 OPEN in O(1), preserving accepted/reviewed effects and historical ordinary rights.
Creating A2 or a new scope with matching raw labels cannot renew an old observer.

Fork/proof loss closes the original governance view before asynchronous repair. A conservative
first implementation fences the whole view and rebuilds common-prefix positives in bounded turns.
It never chooses first arrival or resets/deletes semantic history. A new token requires the actual
current-governance semantic dependency verification, not merely an old cached Valid entry.

A projection captures the original view/revision BEFORE reading any body. Admission/token/capsule
changes that affect the result invalidate original read leases; accepted/reviewed tokens bind view
identity/support, not their issuance revision, so an unrelated admission close does not revoke them.
Final native publication must hold that
same original projection lease after its UI/registry guards, through synchronous reply resolution;
no await follows it. Once a reply is admitted, revocation cannot report completion before that
commit ends. A stale task cannot sample/adopt the newer revision after reading old plaintext.
Actual lock order is UI -> registry -> projection commit. Owner Drop MUST cancel without waiting
on that read barrier: a FnOnce callback can own/drop the ledger/view, also during unwind. A blocking
Drop would deadlock behind its own reader. Abandonment cancels future admission but does not claim
that already admitted replies finished.

Use nonblocking explicit close and a must-use completion object. It prevents new admission at once;
completion reports true only after a nonblocking commit-writer probe observes all admitted readers
finished. The active async owner retains/polls it instead of reporting Closed early or blocking an
Actor. Admission close keeps a negative-pending count until its completion barrier, then permits
fresh reads while preserving accepted tokens; abandoning that completion conservatively cancels
the whole view. Beginning a replacement view retains old-close completion and keeps the new view
negative-pending until the old admitted publications have finished. Dropping/unwinding any of these
owners/completions never waits on the barrier and never fabricates completed revocation.
Native integration and deterministic final-reply tests remain required beyond pure-core tests.

## Guarded results and effect filtering

The new result distinguishes Visible, Deleted, Held and IncompleteSuppressed. Visible/Deleted carry
explicit Proven versus Provisional certainty; a provisional Delete may hide text but does not claim
accepted-before-removal and cannot mint a prior-effect capsule. Ordinary
Visible returns only selected current text/metadata, not all historical edit bodies. Deleted and
suppressed results contain no message/edit text. Raw complete history stays in separate admin APIs.
Every Create, Edit and Delete needs its own live exact token; admitting an Edit cannot admit its
held Create. Held edits do not supersede visible edits; held deletes never enter effective tombstones.
Knowledge traversal may pass held semantic Join/ancestor records without releasing their bodies.
Semantic and projectable causal-head APIs must remain distinct and truthfully named.

The core still explicitly bounds scan/traversal work; it must return work refusal rather than a
partial projection labelled complete. Scalable cold-index/paged projection and automatic bounded
repair belong to the active host and remain separate gates. Point-of-use scope checks are fixed
runtime observations, with no arbitrary verifier callback or I/O per row.

## V2 Delete and local cold-negative custody: required next implementation

Current General V2 accepted-action issuance refuses Delete. The existing prior-Delete
capsule issuer uses legacy proof identity and cannot be relabelled as V2. A separate
own-Delete inclusion issuer must verify the actual current or frozen Admission, exact
signed dependencies/own target, record binding and inclusion. Its V2 support must carry
exact record/index/source provenance, never a parsed tip or status. Provisional Deletes
can hide text but cannot mint prior-Proven custody.

Both installation orders matter: Delete may arrive before its admitted Create. Core must
preflight ALL newly eligible exact V2 capsules in an opaque non-Clone mutation plan; Root
must durably save that exact negative batch before final positive token/capsule mutation.
Final application rechecks the same plan/view revision under original group then ledger
guards. A direct Create admission must not bypass required durable negatives. Each Delete
keeps its own fixed obligation, whether or not a renderer ever displayed its effect.

ChatSnapshot currently saves actions only and restores projection closed. Full governing
replay authenticates acceptance tips, not the complete local negative inventory. Incremental
Create/OPEN activation after restore can forget previously effective Delete custody unless
a selected durable negative-closure index/journal, or equivalent per-target closure, is
loaded first. Page EOF, a snapshot hash and the loaded-actions index do not prove that.
This coverage is the mounted LOCAL known-obligation generation, never globally unseen
remote Deletes. Fresh ordinary OPEN remains available after its complete local lookup;
unknown remote signed Deletes merge when they arrive.

Each selected negative row retains exact target/Delete/Create bindings and original signed
Delete/proof locators. Decoding negative DATA may only suppress: it cannot issue Proven
Deleted, OPEN, signing or clearing permission. Missing/corrupt/unsupported selected negative
state keeps that target closed. Clearing requires a separate exact-capsule complete-current
closure against the actual selected local inventory and governing source; missing proof,
removal, a lone Reject or a Boolean cannot erase an obligation. Persist the exact resolution
witness before runtime clearing and preserve the original row; proof loss suppresses again.

Required tests cover genuine current/frozen inclusion, both arrival orders, provisional
Create, minting without rendering, two independent Deletes, proof/view/source loss, stale
and cross-source resolution, cancellation during remint and restart with partial action/
proof loading. Root must test journal barriers, quota/refund, lost replies, corruption and
no body publication before complete local negative lookup. These production consumers and
the durable negative format are not implemented by the current Core issuer.

## Missing previously effective Delete evidence

A capsule is minted only from an exact genuinely verified and effectual accepted/reviewed Delete
and its admitted Create under the original view. Provisional/unapproved/raw Delete cannot mint it.
The capsule retains fixed exact binding/context/source identity, never a trusted was_deleted bool.
It is negative incomplete-proof custody, not a new approval, a verified tombstone, or an exemption
from deterministic common-prefix resolution. It must remain through view cancellation/proof loss.
Minting depends on eligible verified effect evidence, never on whether this replica rendered it.

If no current verified Delete is available and such custody is unresolved, return
IncompleteSuppressed without body text. A current real verifier may clear it only with an opaque
exact-capsule/target/current-view resolution that proves the complete required current effect
closure. A later signed Reject alone cannot retract a genuine finalized approval. Equal complete
verified evidence must eventually resolve to equal projection after a fork; never grandfather a
replica-local previously rendered branch. Durable restore must validate/preserve actual negative
closure through a critical format, not serialize a raw approval bit. That producer/journal remains
unimplemented in this core slice.

## Required tests and implementation limits

Keep existing semantic-only regression showing approval-free semantic restore can expose text;
add guarded restore tests proving that raw semantic result never opens v2 projection. Cover
default closure; accepted/provisional removal separation; exact-action/context/mount/view binding;
A1/A2 and stale replies; held Create/Edit/Delete; held knowledge traversals; genuine signature
variants; immediate view cancellation under full entry quota; before/after final commit orderings;
callback-captured owner Drop/unwind without deadlock and abandoned incomplete close;
accepted Delete proof loss and unapproved/provisional capsule refusal; exact negative-resolution
binding; foreign/obsolete scope refusal and equal complete evidence. Test-only verified fixtures
must check real signed data and remain clearly distinct from the actual production verifier.

No public token constructor, wire decoder or positive status setter is added. The later active
governing/Profile/delegation verifier is the only production issuer. No cold store, durable proof
closure, native command, frontend result or live dispatch is claimed by this proposal.

## Implemented core checkpoint; tests not executed

`ChatLedger::begin_projection_view` returns only an UNBOUND core negative owner and default-closed view. Its
original revoker, admission owners/revokers and must-use `ProjectionCloseCompletion` never wait in
Drop. Explicit completion uses only `try_write`; a pending admission count prevents new body
publication while old readers drain. All views share the actual ledger's barrier, so nested
replacement cannot skip an older publication through an intermediate pending view. Replacement
owners retain one original close completion rather than a recursively growing parent chain.
Preparing-close unwind publishes an original retryable negative phase and releases its unreturned
handle count before cancelling the view. Arithmetic exhaustion never wraps. Poisoned barriers
remain closed for positive work; an exclusive poisoned guard can still prove negative drain.
An old admission may also reuse its SAME original view's completed barrier witness. It cannot
adopt another view's status or wait indefinitely on unrelated later-view readers after its own
view has already proved drain. Pending registration/completion still occurs exactly once.

The new `guarded_projection` / `projectable_causal_heads` results expose their output only through
`with_publication` under the original view/revision and contributing provisional scope observers.
Visible contains the selected current text and original Create timestamp, with no alternatives.
Deleted has explicit certainty and no text. Held/IncompleteSuppressed contain no text; a default-
closed Held/empty-heads result carries no positive lease or certainty. The real native adapter
still needs UI/registry reply ordering for negative results as well as the body publication gate.

`admit_projection` accepts only an opaque token for already Valid semantic evidence and takes the
actual nonblocking writer before output-revision changes. Busy/work refusal leaves entries and
leases unchanged. Equivalent current proof retries preserve original support/revision. There is
no production token constructor or complete-resolution issuer. Historical authorization/import
revisions are deliberately distinct from this output revision: future governing verification must
fence its actual complete dependency inventory before issuing/caching current support.

Every Delete has its OWN already-paid fixed capsule slot. A Proven Delete with an admitted Create
is eligible even if that Create is Provisional; displayed certainty still reflects both actions.
Capsules mint atomically on positive installation, never from a rendered flag. Admitting a Create
preflights the bounded scan for all newly eligible Deletes before mutation. Pending/Invalid tokens
refuse, so later semantic advance cannot activate an uncapsuled cached token. Exact complete-
current resolution addresses each original capsule separately; eligible current Proven support is
re-capsuled rather than exposing a future proof-loss gap. No capsule/status is added to raw
ChatSnapshot bytes. Critical durable negative closure and the actual governing producer remain open.

Thirty-one regressions are authored in `chat::projection_admission::tests`; none have run. They
cover signed exact fixtures, restored default closure, held effects/Join traversal, A1/A2 and
Provisional/Proven separation, independent Delete obligations, complete-resolution binding,
timestamp/work limits, actual admitted-reader drain, concurrent duplicate close, nested replacement,
callback self-close and captured-owner Drop/unwind, preparing unwind, full entry quota, genuine
signature variants, retry idempotency, common-evidence convergence, original-barrier reuse, poison
and arithmetic exhaustion. Test-only signed current-
effect/complete-closure assertions exercise capability binding; they do not implement the absent
production governance verifier. Primary owns all deferred focused/full/native validation.

## Message-local work, without authority from an index

The ledger derives a sorted message-candidate index at successful original-action import, after
signature/scope/duplicate/capacity checks. Every retained action has exactly one membership under
its target Create, or under its OWN CID for a Create. Reply Creates never join their reply target's
candidate set. Pending and semantically invalid evidence is indexed too, so consumers retain their
existing independent validity/current-token/body checks. The index supplies neither acceptance nor
effect authority. Restore rebuilds it through the same import path; no index bytes enter snapshots,
pages, action IDs or signatures. There are at most max_actions memberships and message keys, not
a claim that logical labels equal physical BTree memory or a complete native invoice.

Raw and guarded projection/heads and atomic Create capsule preflight scan only those candidates.
For the fixed selected Edit/head set, ancestor expansion uses one shared visited set, initially
EMPTY rather than seeded with roots. A selected root is superseded only upon reaching it through
an actual ancestor edge, independent of CID/root order. Each root starts its own bounded parent
stack; every pushed and popped edge spends work even when that ancestor was already visited.
Create remains a stop boundary, and held Join metadata remains traversable without releasing an
effect. Raw heads retain their direct-parent semantics; guarded heads retain transitive filtering
through held metadata. Work exhaustion returns an explicit refusal, not partial heads/effects.

Five additional semantic regressions and three guarded regressions are authored/unexecuted. They
cover a96-edit chain within a linear edge-work allowance, unrelated messages/reply Creates,
reverse import and byte-identical restore, rejected signatures/quota/duplicates, retained pending/
invalid candidates, shared Join diamonds and atomic capsule creation/proof-loss suppression.
These are deterministic allowance assertions, not a wall-time benchmark or 20/50-peer acceptance.

## Shared original group lifetime

`ProjectionGroup` owns one bounded exact group identity and ONE actual shared publication gate.
Its `begin_view` returns an original negative `ProjectionGroupView`; cancelling that lifetime
immediately closes future positive work for every attached channel without scanning channel rows.
Drop never waits or reports completion. Explicit must-use `ProjectionGroupCloseCompletion` polls
that original gate's exclusive writer and therefore cannot acknowledge before admitted callbacks
or positive token/capsule mutations finish. Poison may prove negative drain through its real
exclusive guard but never opens a replacement. No generation counter wraps and replacements retain
no recursive parent chain: all share the same flat gate, so a pending middle view cannot hide an
older reader from the next replacement. Unrelated mounted groups have independent barriers.

`ChatLedger::begin_projection_view_in` checks exact full group identity and original group liveness
BEFORE replacing its existing channel view. It captures the supplied original Arc, never a later
lookup of a group with matching labels. Cancelled/pending/poisoned group views refuse. Creating
that view is not a root/Profile/tenure selection or a positive authorization. The future production
issuer MUST refuse unbound `begin_projection_view` views and bind the actual verified original
governing source; raw bytes or this lifecycle owner cannot supply that verification.

Final synchronous commit order is outer native UI -> registry -> ORIGINAL GROUP -> ledger.
`with_publication` first refuses already obsolete callers, then takes group/ledger readers,
validates original view/revision/scope inside both and retains them throughout the callback.
No post-callback check retracts an admitted reply. Token installation and exact capsule resolution
likewise retain the group reader before the ledger writer. All probes are nonblocking; no await
belongs inside the callback or either mutation. Capture original group/view cancellation custody
before actor/body waits. Looking up a newer view afterward cannot renew an old result.

Capsule installation prepares its whole per-message plan before final original-token/liveness/
revision admission. Resolution freezes its exact replacement before the same final view/revision
check. If cancellation wins BEFORE that admission, no positive metadata changes. If it wins AFTER,
the admitted operation installs the PRECOMPUTED effect and retains both guards until done: it
must not re-read later liveness and silently turn an eligible Proven Delete remint into None.
Group close completion covers these mutations as well as body callbacks. These ordering rules
were corrected during actual-diff adversarial review; no execution is claimed during the pause.

Eleven additional regressions are authored and UNRUN (nine group lifecycle/mutation tests and
two group bound/poison tests). Real signed channel fixtures cover multi-channel cancellation,
before/after admitted publication and capsule installation, both exact-resolution orderings with
and without current Proven support, nested replacement, equal-label remount, foreign refusal
without channel mutation, owner Drop/self-close/unwind and independent unrelated-group progress.
Worker completion waits are explicitly bounded before joining. The total projection admission
regression count is42; five separate semantic message-work regressions are also unexecuted.
This adds no production governor, physical control-memory invoice, durable negative journal,
native/UI command or activation. Those remain required before Chat V2 can be enabled.
