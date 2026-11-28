# Chat V2 shared history beyond the current archive limit

The native received archive currently accepts up to 2,048 selected records
and 256 MiB for its one mounted scope. It enumerates physical files to enforce both limits.
Its relay retains an in-memory list proportional to the archive and can copy
overlapping generations for different peers. Exact history also replays transitive dependencies
under the hot/ancestry limits. Raising the record limit alone would increase
memory and scan time while leaving those other limits in place.

The archive is additive. A received operation that passes the selected group,
channel, Admission, signature and dependency checks keeps its complete
original. Edits, Deletes and Join operations change the derived view. Their
individual signed records remain readable in the authorized audit. A
separate received review receipt can display an explicitly approved post.
The ordinary archive must prove an exact healthy absence before allowing
that receipt to supply the display.

## Storage and compatibility

Keep the original signed bundle and stable vault/group/Profile/tenure/channel
scope, including its stable creation identity. Replace the bounded directory inventory with a versioned critical
archive descriptor and an authenticated, scope-bound selector. The selector
contains exact index roots and account totals. An authenticated empty
selector is required to prove emptiness. Missing or malformed required
root, path, record or directory must hold the affected read or write.

Use the existing bounded private-index traversal and exact rewrite checks.
Index exact ActionIds separately from Create targets. A target directory
points to bounded pages of its revisions and can paginate Create targets.
An exact Admission index supports the separate removed-post inventory.
A general hash of a composite key cannot supply a same-target range.

Before a write, reserve the complete maximum path and immutable output work
and check physical quota. Select one authenticated successor after holding
and syncing the exact original bundle, index pages and bounded WAL. Keep
the old and new selector identities and exact original in the WAL proof.
An interrupted writer can select the same covered values. Retry uses the
same ActionId and exact canonical original once covered. Incomplete or
contradictory evidence stays held. Reclaim only derived pages after proving
they are no longer selected, and account for pages still held by readers.
Signed originals remain in the archive.

Use a critical companion descriptor with its own authenticated tag and
marker domain. Bind the vault/root, the exact existing ID8 entry digest,
and the indexed layout. Preserving ID8 also preserves the descriptor and
marker keys used by the existing review journals. The first new vault
selects the companion using an exact one-entry declaration before its
first native row. Directory presence alone supplies no format authority.

A fresh v2 mount needs operational exact append and point reads, target
audit, and the existing projection and catch-up behavior before it is
selected by default. The catalog preserves both the original ID8 entry and its exact companion
through later declarations. A later fresh A2 can bind the resulting Current as its
original frame.

New binaries must explicitly recognize the new descriptor. Prove the
predecessor refuses it before any archive cleanup or mutation. A migration
from the first layout scans its at-most-2,048 entries in paid, restartable
batches, under an immobile original scope. It checks the exact selected
selector to bundle relationship and copies each corresponding original
or retains an exact link to that original. Keep the existing format and
its originals intact through the first migration increment. Switch
authority only after the full verified census and new roots are covered.
This cannot reconstruct a pair already absent from both old selectors
and bundles before migration.

Same-vault rejoin persists a commitment to the complete encrypted catalog
frame, including after installing the A2 row. A critical catalog migration
must preserve that original proof or use an explicitly authorized
predecessor witness. Keep active rejoin stages and installed A2 catalogs
outside the first migration unless that additional format has been
implemented and verified. The first increment can use fresh vaults and
exact eligible pre-rejoin mounts.

## Publication and recovery

Ordinary exact lookup, selection and audit must use the same verified
custody after the migration. Preserve the strict read and the narrower
sibling read used for already selected local ancestry while unrelated WAL
exists. Preserve the ordinary negative proof used by the review display.
Presence of a reviewed or local outbox original is not an ordinary
selected receipt.

Live projection and serving can keep bounded hot state. To replay very
long target branches, use a bounded continuation over the authenticated
target and ancestry graph, or persist independently checked local effects
that can be proved against that graph. The existing Core rules are the
oracle for the maximal undominated edit, Join traversal and Delete
precedence. Keep the exact target root through publication so a new
relevant Delete suppresses a prepared older display. Unrelated additions
may advance independently.

Page relay inventory by globally ordered exact ActionIds. Share the
selected read root across authorized peers and keep paid bounded cursor
and offer state per peer. A stale root restarts its cursor. A previous
offer cannot authorize newly selected entries. A removed-Admission query
uses the authorized per-Admission range and exact prior offer. Rejected
or reopened review bodies remain in the founder's local journal.

## Required evidence

- Write and cold-reopen more than 2,048 small signed operations across
  authors. Verify every exact original, including superseded edits and
  deleted Creates, remains available to the authorized audit. Compare
  ordinary results against Core across deep chains and concurrent edits.
- Exercise quota and payment refusal before the first write, every
  irreversible migration and index cut, exact retry, forged or missing
  locator and bundle, vault/channel separation, stale target publication
  and an unrelated WAL. Retain old evidence on every failure.
- Use a real predecessor executable against fixtures with and without the
  successor. Reopen applicable fixtures with staged and installed rejoin
  evidence to prove the first migration leaves the original frame intact.
- Exercise 20–50-member partitions and restarts over the live protocol.
  Compare the exact signed operation union and current projections and
  report per-turn work, paid memory and eligible unrelated progress.
  Confirm original history is recoverable when peers leave and return.

The current 2,048-record quota and whole-catalog relay remain release
constraints until the selected storage, projection and live relay are
verified together.
