# Inactive typed chat core

29 October 2026. This isolated replication-only core is being validated. It has
no production authority issuer, DocType allocation, Root/native/UI dispatch,
encryption adapter, legacy conversion or activation. Production channel traffic
still uses the existing legacy path; **R6 is not closed by this core**.

## Identity and evidence

`chat::Realm` hashes an exact framed group ID, channel u128 and profile1. An
owner key, random cutover nonce, first Genesis/root binding or mutable directory
winner cannot choose another namespace. Authority/root statements are evidence
to be verified separately, not a first-arrival namespace selection.

Each canonical action signs that realm, its full32-byte Ed25519 signer, exact
basis/grant IDs, nonce16, presentation time, sorted unique causal parents and
typed body. Text is capped at64 KiB; one frame at128 KiB; one action has at most
64 explicit parents. A zero nonce is legal. Neither a writer sequence nor a
nonce is an exclusive first-arrival slot.

The ActionCID hashes canonical unsigned semantics. A separate envelope hash
identifies one signed encoding. Every signature variant is verified before
duplicate handling; valid variants represent one semantic action and the least
signature is its deterministic representative. Different signed bodies sharing
a nonce remain distinct retained actions. No author fingerprint, mutable author
field, carrier epoch or sealed ciphertext supplies the action's identity.

Create IDs are exact Create ActionCIDs. Edit/Delete/Join target that ID, and a
reply names another exact Create. Missing targets/dependencies stay pending;
they never retarget a legacy ID or another row with matching text.

## Authorization boundary

`VerifiedAuthorityBasis` is an opaque externally verified historical decision
bound to realm, basis, grant, full subject key, rights and selected causal-basis
coverage. There is **no production constructor or decoding path** for it. Wire
and snapshot decoding cannot turn a signature, label or stored `verified` bit
into permission. The sole unit-fixture constructor verifies real grant and
coverage signatures under an explicitly pinned fixture issuer.

Self-edit/delete compares the full Create signer key. A rejoined device with the
same key and a fresh valid ordinary grant can edit its older Creates; that grant
does not inherit another incarnation's DeleteAny right. A privileged delete
requires an explicit matching DeleteAny witness. Parent and target authorization
bases must be covered by the supplied witness; absent coverage stays pending.

This module does **not** decide native membership, root continuity, authority
succession/forks, revocation or whether a delayed old-context moderator action
should be authorized after demotion. Those privileged-policy questions remain
unresolved; no such user approval or production behavior is inferred here.
An MLS epoch-authenticator hash is a state identifier, not independent proof an
Add physically happened. Certificates would establish an authorization basis,
not prove physical action creation before removal. A complete production issuer
and historical authorization archive require separate reviewed contracts.

## Additive state and bounded work

Validly signed same-realm bodies are retained, including semantic forks, pending
dependencies and semantically invalid evidence. Rights/target checks determine
the projection; evidence is not deleted because a current roster changed.

The ledger's explicit caller limits bound action count and canonical envelope
bytes. Full capacity refuses new evidence without evicting old evidence. These
are logical limits, not a complete allocator/RSS or native funding invoice.
Each dependency evaluation examines at most65 labels; a wide dependent fanout
wakes one child per charged work unit. Iterative projection/heads scans have an
explicit allowance and return WorkLimit rather than a partial complete result.

Causally later authorized edits supersede their ancestors. Concurrent maximal
edits remain alternatives with a fixed greatest-CID display choice, independent
of timestamps and arrival order. Any authorized Delete is a persistent visible
tombstone; old Create/edit replay cannot resurrect it. Join only combines causal
knowledge and changes no text. More than64 heads are returned in full and can be
covered by bounded Join nodes; the ledger never silently truncates a frontier.

## Snapshot, paging and remaining durability work

An immutable `ChatSnapshot` retains all canonical signed action bodies and
authorization references in CID order. It stores no status or authority tokens.
Restore imports the evidence again and requires externally supplied action-bound
`(ActionId, VerifiedAuthorityBasis)` pairs, followed by explicit bounded resolution.
Selected coverage remains with its exact action even while a parent is missing;
unrelated actions under one grant do not share one capped coverage union. Conflicting
rights for the same grant are refused. The separate native
archive of certificate/context proof bytes is **not implemented here**. The300-
basis regression resupplies signed fixture witnesses; it is not300 MLS transitions
or proof that authorization evidence is durably archived beyond commit-log expiry.

Snapshots are plaintext at this API and must be sealed by a future native storage
adapter. Their capture/decode walks the configured inventory; it is not claimed
as a bounded network turn. CID paging runs over the immutable snapshot and binds
continuations to its exact hash. Pages cap records at64 and bytes at256 KiB. A bad
inner tail preserves an imported verified prefix; the caller may advance its
continuation only after the complete page succeeds. No legacy Automerge cursor,
receipt hash or advertised completeness is reinterpreted as an ActionCID.

The existing native durable send transaction is unchanged. New typed send/edit/
delete issuance, covering saves, publication custody and authenticated network
recovery are later integration work. Legacy history remains readable/unproven;
there is no automatic promotion into the new authenticated semantic namespace.

## Verification at this draft checkpoint

All17 focused tests pass (50.38s), including the parameterized20/50-writer,
five-bubble corpus. They use genuine deterministic Ed25519 fixtures, not real
MLS/native/network authority. Scoped Rust formatting and diff checks pass.

Independent actual-diff review found a HIGH restore failure from globally combining
unrelated actions' coverage and a MEDIUM malformed-count allocation hazard. Both are
corrected and re-review reports no remaining blocker/high. The70-target restore
regression failed before the fix and now passes; delayed-parent/wrong-grant and
short-header/billion-count controls pass. The dangerous old billion-count allocation
was source-verified, not executed. A separate long-reply projection regression failed
with WorkLimit before the traversal was bounded at the target Create, then passed.

Strict changed-crate Clippy passes. A broader affected-crate run was intentionally
stopped while unchanged Studio/registry stress cases ran, so the new durable
mutation draft could be compiled. Its partial output is not a
complete pass. Complete Root/native/frontend validation remains pending for the combined text-chat
change; focused results do not replace it. A later genuine alternate-nonce signature
vector passes its focused test (one test, 0.05s): real verification accepts both
signatures over identical semantics, opposite orders retain one deterministic
representative, signatures alone grant no authority and a corrupted duplicate cannot
replace valid evidence. Its read-only re-review is clear. Additional malformed-cursor
and wide-fanout cases remain explicit coverage follow-ups.
No executed result from earlier Main inputs covers this new core.
