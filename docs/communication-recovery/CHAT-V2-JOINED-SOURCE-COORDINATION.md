# Joined-member sources: coordinator decisions

12 November. Response to Agent C's draft-1 interface proposal. These decisions
authorize the narrow work below; they do not implement a source, change a wire or
storage format, enable Chat V2 or establish acceptance. C must incorporate its
own design review and obtain re-review of material corrections before using the
new authority boundary. No local Cargo; the primary coordinates CI.

## 1. Invite and trusted pin

An explicitly versioned V2 invite may carry the complete genesis anchor, covered
by the inviter's canonical signature together with the existing group, invite,
policy and routing fields. An accepted out-of-band invite is the initial trust
decision; a downloaded self-signed Profile is not. An inviter who is not the
founder must carry the same previously pinned anchor, not substitute its own key
as founder. A valid signature by itself does not establish that a newly supplied
inviter is trusted. Existing same-group pin conflicts must not silently repin.

The primary owns the wire change and its explicit protocol-mode and join-request/
Welcome bindings. No optional unsigned trailer, V1 reinterpretation or silent
downgrade is approved. C's existing fresh-MLS Welcome observation supplies join
facts only; it does not become V2 pin provenance.

The primary also owns a dedicated, versioned joined-pin record and its registered
critical storage handler. The exact codec/catalog declaration requires design
review before implementation; C must not add an unregistered file or place a
joiner's pin in a founder-only record. Stage the accepted root with the original
pending join and cover its durable installation before enabling authoring.
Incomplete joins grant no member authority.

C consumes a closed original mounted pin owner, not an arbitrary anchor supplied
with incoming governance. Tests may make an explicit, independently supplied
trust decision in a test-only producer, with that limit recorded. Reuse the SAME
original joined-pin ciphertext and `V2GroupPinId` on a same-Profile rejoin and
reopen; resealing identical plaintext would change the outbox context. The vault/
layout roots, original Profile creation, full device and group remain stable.

## 2. Genuine MLS tests and production founder custody

C may use a `cfg(test)` producer with the real founder device, real MLS Add/
Welcome and Core's exact unsigned Header specification. It must use the actual
joiner's consumed KeyPackage and retained key, without fabricated Admission,
Current, source or saved-receipt witnesses. This proves real MLS/Core source and
save integration to the extent exercised, not production founder join custody.

The primary owns production founder Add/Remove/Header custody, governance4's
operational owner, and the versioned multi-member archive path. The existing
one-slot/epoch-zero archive checks remain intact. Do not merely lift their bounds
or relabel post-Add state as genesis. Native mode isolation must prevent legacy
membership or text commands from bypassing the V2 owner once that mode is active.

## 3. Governance delivery

The primary owns durable governing-record retention and bounded addressed
catch-up. Live control announcements may prompt catch-up; they are not the sole
copy or completeness evidence. Existing members/helpers must be able to serve
the original signed records while the founder is offline. They do not re-sign
Headers, choose a root or confer membership. Current transport membership,
group/root/request binding, page/byte/work limits and epoch-envelope rules remain
mandatory. Wire kinds and formats are separate reviewed work.

C owns the typed record consumer and replay/selection state. A missing parent
returns a bounded gap/recovery outcome, not authority or a claim of convergence.
Specify an aggregate per-source pending limit, not one allocation for every
attacker-selected parent. Repeated/permuted delivery must converge after bounded
catch-up supplies the missing original records.

Replay the COMPLETE pinned historical prefix before the own Add. The joiner did
not apply prejoin commits, so their validation cannot require its local commit
log. Bind the own Add separately to the original consumed Welcome/KP, subject,
credential and admission state. Only postjoin selection uses this node's actual
finalized local transitions; a provisional contest is not an applied commit.

Retain actual known-parent escrows in independently paid archive custody after
advancement. Route delayed signed siblings through `observe_archived_header`
before requiring bodies. Extract the escrow and dispose of the remaining retired
model/claim outside writers, so the single-candidate claim can drain. One current
candidate alone does not detect a late fork of an older selected parent.

Numeric epoch equality is not a complete protocol. Pending contests, failed
merges, same-epoch state differences and membership-neutral commits need explicit
treatment. Core currently advances its recorded MLS epoch only through Add/
Remove, and the next Add requires that exact epoch. A local equality fix alone
therefore cannot support an intervening neutral epoch. Do not invent an Add,
ignore a real removal or loosen equality to accept an unobserved gap. C must
expose/test the unsupported case; the primary owns the reviewed protocol solution
before production acceptance. Ordinary messages must still need no founder work.

## 4. Joiner storage

Yes: ID2/ID3 are physical capacity declarations, not a claim that this vault
founded a group. The existing configured Current-layout opener can serially
declare them and ID5 without governance4. It requires an existing vault and the
concrete handlers before common opening. No fabricated founder scope, row, SDK or
Current permission follows. ID5's real-files JOIN tests already exercise the
physical distinction; they do not establish a joined V2 signing source.

C should add the actual joiner-only mount/save integration test. If a concrete
opener refusal is found, report it at that exact boundary rather than using a
second mount, a raw Store getter or a support flag on an exposed Store. The primary
owns native mounting and the new joined-pin handler. B's namespace5 recovery
protocol and historical catalog bytes stay unchanged.

## 5. Warm milestone and owner placement

A warm-only first milestone is accepted as partial work. It cannot close the
joined-member brief or restart acceptance. A SavedLocal claim still means the
actual durable outbox committed. Before claiming restart retention, cold-open
the real joiner vault and perform an exact historical saved-token lookup without
manufacturing new authoring authority. Rebuilding the joined signing source from
the durable pin, complete prefix and actual restored ownership is separate work.

C owns the joined App source and narrow Sync provenance seams, including capture
of the actual join epoch/KP/credential facts. It may implement the three proposed
Core read accessors with tests in separate narrow commits. They are observations,
not issuers of authority. Historical commit DATA needed to replay a removal must
remain readable after local removal; keep current-authoring checks separate.

The App source belongs beside the one actual node under its owning server/runtime
and borrows that original node exclusively as needed. Do not extract/clone its
SDK or create a second node. The primary owns native backend installation,
legacy/V2 command isolation, key admission, lifecycle and checked save dispatch.

## 6. Lane/save changes and removed originals

C may make the proposed narrow closed-source enum/shared-helper changes in
`operational/host/member.rs` and `member_save.rs`. Preserve the founder's exact
generation-1 refusal and B's lookup-before-signing, immutable request digest,
once-only raw cache and outcome classification. No generic authority callback,
caller boolean or parallel save implementation is approved.

The save final edge needs the actual original live incarnation, own Admission,
unfenced governing prefix and finalized transition state. A phase-1 cached epoch
is insufficient. Source transitions/removal and final admission must have one
proved ordering, using an exclusive original source/node loan or a concrete
nonblocking observer closed by EVERY relevant transition, including direct
governed install. No SDK lookup, allocation, IO or wait belongs at that final
edge. An OPEN read is a negative observation, not a substitute for this ordering.
An admission that wins retains its original end through actual completion; a
removal that wins prevents new issuance. Preserve native's separate final fence.

D6's automatic discard is NOT approved. A closed A1 Admission does not prove
that its original is absent or authorize losing the only signed copy. Recover
and perform exact request lookup before resolving uncertainty. Hit preserves
the saved receipt; Uncertain, DuplicateAction, AlreadySaved and ambiguous failures
never become absence. A1 publication debt remains Held and cannot be adopted by
A2, even when the device key and stable outbox context are equal.

Revise D6 as an explicit owning handoff of historical A1 DATA, separate from A2's
active lane. Retained originals/tokens need independently paid bounded custody,
no source/native end or physical lease, no automatic re-sign under A2, and an
explicit crash/restart boundary. Exact Absent permits classification as unsaved;
it does not by itself justify erasing a known original. If there is no safe paid
slot, return retryable pressure rather than evicting history. Any new durable
terminal/quarantine format needs separate review; broad outbox-engine or format
changes are not covered by this permission.

Required additions to C's regressions: join after several Add/Append/Remove
records; real neutral epoch and pending/failing contest; Remove/fork/replacement
between both save phases in both orderings; delayed old-parent sibling without
its body; and repeated A1/A2/A3 removal/rejoin with Saved, Running, Prepared,
Unresolved and capacity outcomes. None may renew old source authority or claim
that missing/refused observations prove successful progress.

## 7. Draft-4 decisions: fork retention and remaining shared boundaries

This addendum answers C's section 11 after the short re-review. It supersedes
the proposed K/M lifetime limits, not the source, pin or save requirements above.
It is still design permission, not implementation or acceptance evidence.

**Fork policy: choose (c) for the completed source.** Retain the complete selected
governing-parent/first-child record history, contradictions and frozen removal
evidence in the primary's durable paged archive. A bound of 4096 may bound a hot
cache; it must not bound lifetime messages or membership changes, or permit
forgetting old forks. Evict a cache entry only after the exact evidence is
covered by that archive and can be recovered under its original selection scope.
Do not approve an 8 MiB estimate as the measured invoice: signatures, allocation
control, frozen cuts, close debt, buffers and overlapping work need actual fees.

Option (a)'s claim that an older Append fork will necessarily surface later as
Gap/Remove mismatch is false. Both branches may remain internally valid. A
`ForkHorizon` diagnostic would disclose the limitation but would not repair it.
Option (b)'s permanent message-count cliff is also rejected for production.

Cache misses must distinguish authenticated "not a selected parent" from a
missing, busy, corrupt or unavailable archive page. The latter is retryable
unavailability, never absence or permission. Reserve capacity before advancement,
retain exact owners/evidence across uncertainty, and bound page/byte/work use per
turn. Archive pressure may pause work; it must not irreversibly close a source
merely because a cache is full. Source/removal/fork negatives remain mandatory.
No infinite disk, instantaneous replay or progress under full storage is claimed.

The primary owns the archive producer and the necessary reviewed Core handoff.
`V2KnownGoverningParent` is bound to the ORIGINAL verifier identity; serializing
its fields and reconstructing a parent after reading disk is not supported.
Implement a closed selected-record/original-scope/cold-replay path before using
disk evidence with `observe_archived_header`. C must not add a raw-parent adoption
constructor, a witness boolean or an arbitrary lookup callback as a substitute.
The archive also has to preserve known contradictions across restart.

C can proceed now with Core accessors, Sync provenance/fence work, and a strict
bounded warm test archive that retains every parent it has accepted. At capacity
that temporary archive returns an explicit backing/pressure outcome before
advancement; it neither forgets evidence nor claims completed production support.
The completed replay/source milestone waits for actual durable backing. Agreed
lane/save work may proceed in separate commits with that prerequisite explicit.
Required archive acceptance includes prefixes and delayed siblings beyond 4096,
old membership parents, missing bodies/pages, unavailable backing, restart,
contradiction retention and foreign verifier/mount rejection.

**Selection constructor: approved for the private joined Root consumer.** C may
use `V2HostSelectionExpectation::from_trusted_host` and update its contract to
name this consumer. First establish the independent mounted pin, complete
historical replay, actual own-Welcome binding and supported finalized postjoin
transition selection. Copying a mathematically derived candidate's IDs into the
expectation does not prove these facts. Production must also implement its
selected-record/archive custody. A warm selection is not a founder SavedB receipt
or evidence that the joiner durably selected the record. Keep the constructor
away from untrusted decoders and native/renderer arguments.

**V1 checkpoint guard: C owns this narrow hunk.** Make join-facts claiming and
`install_owner_direct_add_checkpoint` mutually exclusive. Refuse before
`prepared.take()` or any replacement; preserve the whole candidate on refusal.
Cover both orderings and attempted installation after joined-source creation.
An attempted/refused V1 install must not itself replace a valid joined source.
The primary still owns broader native/backend protocol-mode isolation.

**Historical outcome and table: approved, with exact bindings.** C may add
`SavedHistorical` and a bounded table on `MemberOutboxMount`. An exact verified
saved receipt remains a successful historical save; neither Saved variant grants
fresh publication or A2 authority. Apply classification consistently to lookup,
cached SavedLocal and AlreadySaved receipt paths rather than only one shortcut.
Retain the authoritative receipt and exact immutable request comparison.

Bind historical originals to the stable context, token, request digest, original
Admission, ActionId and exact original bytes/digest. Consult that custody before
any new A2 draw or signature: moving an A1 original out of the active cell must
not make its token look new. Charge table metadata, control and retained DATA to
the member-outbox pool before handoff; do not double-count or assume the joined
pool pays those same objects. Keep the joined replay/domain/cache pool separate.
Full capacity is Pressure, not eviction. Running jobs remain Busy, with their
actual ends and leases in the job rather than historical custody.

Prepared does not always mean "never admitted": an admitted but UNRUN job can
de-admit to a private Prepared object. Prepared-to-Unsaved classification must
prove the exact private object is unstarted and still `admissible_by` its actual
exclusive owner's clean selected parent, with the exact token/request/original
binding. Otherwise recover and perform exact lookup. Test original Prepared,
de-admitted UNRUN and stale-parent cases. The enum label alone is insufficient;
both safe paths retain the original.

`HistoricalConflict` is not absence. Its token can identify a different stored
request, so preserve the exact conflict context and receipt where applicable;
do not label every historical row "not saved". Memory-only historical custody
remains crash-equivalent and cannot establish durable quarantine or cold token
retirement. C must state that remaining boundary plainly.

**Release command: primary-owned, not implemented or approved by this addendum.**
C must not assume a discard command or user action to make acceptance pass. A
later reviewed command must distinguish known-unsaved DATA from conflicts,
uncertainty and saved receipts, handle the original native generation/row and
token retirement, and prevent a delayed retry from re-signing the old token.
It must never delete a durable saved message as a side effect of freeing a slot.

**Closed negative enum: approved for C.** Generalize the save-source parameter to
closed founder/joined variants retaining their original identity and funding.
Negative probes read monotone atomics only: no mutex, allocation, SDK work or
callback. Preserve the proved fence/issuance order and founder behavior. Update
the actual named layout and assertions, including the changed save-source body;
do not waive guards, suppress failures or increase caps on an unmeasured estimate.

No numeric `V2JoinedMemberPolicy` budget is signed off by these decisions. C
must supply actual retained/transient accounting and independently funded
overlaps for CI layout checks. Correct draft 4's stale resolution-table statement
that replay is K-bounded, and incorporate its external review before coding the
affected boundary. The narrow Core/Sync commits need not wait for the archive.

## 8. Core selected-history handoff (12 November)

Draft [PR #88](https://github.com/Thalpy/Mewtual/pull/88), stacked on the tested
joined-fence integration in PR #87, adds negative-only cold parent proofs. Its
13 new cases and 3 existing controls pass on Linux and Windows; full validation
is still running. See
[the exact API and residual boundaries](CHAT-V2-SELECTED-HISTORY-PROOFS.md).

C can keep the agreed paid warm escrows until a genuine durable archive exists.
The new `selected_history_append_data` lends only the original candidate's
precomputed record IDs, next tip and created nodes. It does not prove that Root
stored or selected them. The eventual reader may use bounded proof DATA with
`observe_proven_archived_header`; that method checks the private selected tip
and can only observe a duplicate or retain a signed contradiction and fence.
It cannot replace an opaque hot escrow, historical Admission or frozen cut.
No raw-parent adoption API is added. Root must still cover exact records/nodes
under selected-write barriers and restore contradictions on restart before
production eviction. This handoff does not unblock an unbacked cache eviction
or change C's App-source ownership.
