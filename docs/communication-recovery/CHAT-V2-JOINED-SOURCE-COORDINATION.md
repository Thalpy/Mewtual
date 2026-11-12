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
