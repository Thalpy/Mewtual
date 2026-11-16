# Source-absent first initialization

This continuation closes one negative kernel path. It does not activate native Chat V2, provide
the scalable owner archive, reconstruct lost private KeyPackage state or complete native startup.
No compile, tests or build ran for this revision because the user paused those commands.

`prepare_owner_evidence_restore(group, signer)` now returns a closed `UnexposedInitialization`
variant when the actual mounted OwnerEvidence vault has that exact Fresh+Initializing row, no
registered live/pending/current canonical writer, and no canonical or staged source. This check
precedes root-pin decoding because process loss can happen before that pin was installed. The
opaque result owns its original admission/mount/nonce/revision; it has no Server, transport,
snapshot, authoring binding or raw-source getter. Capturing it changes no durable state.

Its `retry` performs the actual negative transition under admission -> canonical pool -> optional
head -> synchronous original Store custody. The same guards cover root/leaf directory durability,
both exact fixed-file absence checks and the covering retirement save. Any `.bin` or `.replace.tmp`
is retained, including unsupported bytes. A live, pending or current head wins over apparent disk
absence. A later Current promotion or changed original mount/nonce/provenance refuses. An exact
already-started Retired save can finish after its publication observer was revoked, including a
saturating `u64::MAX` revision. The sealed identity record remains non-evicting; only an empty
no-file runtime head slot can be released after successful negative save.

Before canonical writer registration, a genuinely private creator can exist while its issued row
is Initializing. Negative recovery may win in this interval. The canonical reservation therefore
rechecks its ORIGINAL preparation and creation observers after acquiring the pool guard, before
any head insertion. It cannot register a writer from the revoked preparation. A deterministic
regression holds the actual method after its early checks, completes real retirement, and then
releases the old creator. A created Current head is tested in the opposite ordering.

Startup inventory intentionally changes its old cleanup contract: valid bounded canonical stage
names now reserve/charge their logical head and preserve their bytes. Deleting a stage before
checking absence would make the negative classifier discard possible retained state. The canonical
writer's original-candidate replacement helper is unchanged. Exact stage-only automatic repair is
a separate open path; an orphan stage is not treated as authenticated or durable by inventory.

The authored regressions use actual founder/Initializing persistence, crash/reopen, both canonical
rename outcomes, stale Current promotion, same-vault new physical mount, directory and retirement
write faults, stage-before-startup ordering, the real creator admission seam, exact scalar exhaustion,
legacy refusal and negative progress at the complete canonical-slot limit. They are unexecuted.

The native automatic registry/restart adapter remains open. After a post-rename negative save and
process loss, existing exact lineage resolution reports the old nonce Retired. A generic Retired
record does not prove that it was never Current; missing history must not be relabelled as a complete
empty ledger or a stronger "never exposed" fact. The host must handle closed lineage separately
from canonical body restore and preserve truthful evidence-unavailable status. No raw missing file
or numeric alias can create a replacement admission.
