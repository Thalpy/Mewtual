# Next file milestone: whole-variant fallback

Read-only design checkpoint, 19 October 2026. **Implemented with focused checks and read-only re-review; final programme acceptance remains open.** Scoped durable Keep,
automatic partial resume and explicit Pause/Retry exist; this remaining Package 4 boundary
must not be represented as complete by those features.

## Ownership and representation

Use a closed V4 transfer record with one immutable, canonically ordered set of 1–4 compatible
exact encrypted manifests. Capture it from the original resolved listing, validating each whole
plaintext CID, exact manifest digest, plaintext chunk identity/size and derived ciphertext bounds.
Current index changes cannot replace saved candidates or supply new authority after unlisting.

Retain per-candidate verified chunk bits and deterministic bad-reference verdicts, a selected
working candidate, and a separate optional committed winner. Scope, transfer identity, original
listing, remaining execution allowance, Retry grant/receipt, user intent and Pause are global.
Changing candidates never refills any of them. A per-run tried mask prevents transient cycling;
restart retains the global finite allowance and persisted selection.

Resumable candidates remain private. Ordinary retained serving and the completed inventory use
only the committed winner. A Completed copy stays owned/served while an alternative is checked;
stopping, failing or pausing that check cannot demote, delete or replace its prior winner. Keep
the winner's checked verdict separate from the working candidate's verification. Only complete
ordered plaintext-CID verification can publish a new committed winner.

## Storage and compatibility

Reserve every admitted candidate's full chunk bounds plus both V4 replacement slots before
publishing its first intent. Conservative duplicate ciphertext charging is acceptable. This can
refuse a large multi-variant set that a single variant would fit; report that quota consequence
without silently dropping candidates. Switching selection does not refund surviving bytes.

V4 uses its own 400 KiB record bound: the working mirror plus four maximum 71,249-byte plans,
bitmaps, fixed journal fields and disk seal fit below 360 KiB. It permits at most 512 unique chunk leaves, bounded cleanup and two full V4 metadata slot charges.
Preserve V2/V3 canonical encodings, predecessor hashes and their existing 80 KiB slot charges.
Never raise a shared constant and retroactively reinterpret old reservations.

Old saved records remain exact-only unless a separately validated migration captures additional
candidates. They did not save alternatives that can be reconstructed after unlisting. No implicit
enrichment, new grant or release/recreate migration is permitted. A future enrichment would need
the exact original listing digest, reserve-before-publication and unchanged allowance semantics.
Legacy completed copies similarly retain their exact-only repair contract until an explicit
ownership-preserving migration exists. Old readers must refuse V4 without deleting its state.

## Runtime transition

Provide an internal storage transition equivalent to
`switch_variant(exact_run, expected_version, saved_candidate) -> fresh_run`. Validate first,
retire the old run, durably publish the new selection/verdict, then issue the fresh opaque owner.
Publication failure poisons execution and preserves quota. Actual old lower permits stay charged
until their real retirement; a new run cannot adopt their results or refund their work.

The actor carries the same native session/inline cancellation to the new run. Restart hashing,
chunk index, staging, provider selection and page cursors from the first chunk of that whole
candidate. Do not splice ciphertext or plaintext progress from different saved manifests.
Per-candidate durable bytes can be reverified when that candidate is selected again.

Missing file keys, stale scope/membership, local read faults and network failures are not
permanent bad-manifest verdicts. Missing keys must refuse admission before reservation/debit.
An exact authenticated ciphertext/key failure can exclude that candidate durably; failure of the
complete plaintext CID is common to equivalent candidates and blocks automatic execution.
Providerless automatic work may run only when one whole saved candidate is plausibly local,
not when a mixture of candidates covers the plaintext chunk positions.

## Required regressions

- A bad first wrapped key falls back to a saved healthy variant.
- Mid-file fallback resets hashing and commits one complete exact variant.
- Restart preserves selection, progress, allowance, grant and user intent.
- Stopping/failing an alternative check preserves the prior completed winner.
- An old lower cannot commit to the fresh run or release its retained permit early.
- Full quota, a malformed/fifth candidate, failed publication and unsupported readers fail closed.
- V2/V3 bytes and charges remain stable, including pending-successor reconciliation.
- Whole-local automatic eligibility rejects mixed per-chunk availability across candidates.

Indirect file service, external destination resume and native physical-network acceptance remain
separate boundaries. This checkpoint supplies none of their acceptance evidence. See the revision ledger for current
focused implementation checks; no full-programme completion is claimed.

## Current evidence

All40 retained-store cases and29 actual file-worker cases pass. The regressions include an
executed red/green first-key failure and completion quiet-deadline race, signed network responses,
full hash restart after unlisting and server/disk reopen, closed saved-set validation, all512
leaves, valid pending selection/commit recovery, exhausted/Pause/bad-verdict/Retry history,
whole-local eligibility and old-lower fencing. Re-review is clear. See the revision ledger for
exact logs and earlier failed attempts. This is not full-suite, physical power-loss or indirect
file-service acceptance.
