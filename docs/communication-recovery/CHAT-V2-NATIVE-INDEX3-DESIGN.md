# Native index3 lifecycle design (not implemented or enabled)

Text Chat V2 remains disabled. The reviewed index2 storage component is deliberately
Founder-only. It does not provide joined provenance, Legacy retirement, rename or
invite refresh. A mounted physical vault can contain imported Legacy rows as well
as native rows; neither the old flat registry nor the physical vault tag may select
a row's backend. The new unscoped mapped Legacy lease intentionally refuses a
TextV2 vault. A scoped original-row path is required for mixed operation.

## Explicit format boundary

Introduce namespace7/version3 and an index3 descriptor/key domain. Keep version1
and2 grammars closed. A metadata sidecar cannot safely hide active rows from old
readers and would add another atomicity boundary. Do not silently widen index2.

Index3 separates immutable row identity/origin from local metadata and backend
lifecycle. Imported Legacy origin has no invented invocation receipt. New Founder,
Joined and compatibility Legacy admissions need distinct immutable request kinds
and real owning producers. Every new numeric ID advances a retained highwater;
terminal rows and creation receipts remain in the ledger.

Joined bindings must name the genuine durable joined pin (namespace6), group,
full signer, source/Profile/root and selected Current basis. Its Pending receipt
binds invocation/request, network and original join context. DATA cannot replace
own-Add, KeyPackage, MLS transition or governing-prefix proof, or authorize a live
runtime. Cold Pending never grants another fresh SDK/device/join execution.

Closed operations include birth/activation/abandonment, native retirement,
RetireLegacy, RenameActiveRow and RefreshLegacyInvite. A Legacy leave must select
a durable tombstone before destructive cleanup; old files/registry/actors cannot
reactivate it on reopen. Rename binds expected local metadata revision and changes
only bounded local display name. Invite refresh has its own checked original-row
transition: existing native minting changes route/expiry-aware invite text.
Keep is_dm immutable: native moderation uses group/DM classification as an
authorization boundary. Do not add generic backend conversion or snapshot setters.

Creation lookup remains bound to the original request, independent of a later
rename. A delayed metadata retry must not silently overwrite a newer revision.
Unrelated index appends may preserve a still-identical row, but must not turn an
unverifiable old operation into a claimed successful retry.

## Owning upgrade

The upgrade consumes a genuine opaque index2 owner after its retained transaction,
outgoing-buffer retirement and JOIN/catalog debts settle. Preserve every v2 state,
immutable receipt, stable Legacy identity, last_issued_id and exact Pending network
ciphertext. Do not recompute highwater from active rows. Index3 generation1 needs
an explicit migration-origin header; do not weaken index2's Legacy-only G1 rule.

Cache/authenticate complete old/new catalog, index and marker frames. Cover the
old empty catalog-Stage fence before content-bearing intent. Sync new index and
marker before selecting catalog3, with original destination identity proved
immediately before each rename. After selection, missing committed outputs refuse;
never rebuild them from initial Pending material. Authenticated malformed/future
intent is preserved. Migration grants no key, source, SDK or publication authority.

The old index2 opener's empty-Stage refusal is a proposed compatibility fence,
not acceptance evidence. Run real pinned main/v1/v2 binaries at every meaningful
cut with successful controls and unchanged-tree checks. Recover exact head bytes
with NeverRng. Recalculate complete old/new schema/frame/AEAD/row/retained-debt
invoices; extending an Arc payment's lifetime is not extra capacity.

## Scoped Legacy execution

Issue an opaque original Legacy-row observation only from the actual paid,
authenticated selected index: Legacy backend, numeric ID, stable identity, original
physical allocation and row commitment. Bound metadata records and their actual
payments independently; records must not own an Arc back to their physical
registry. A separate native Legacy-row lease retains the SAME native mutex,
observation and ordering. It cannot use the inline Legacy mapped-store selector,
reopen a side Store, or expose the hidden native Store.

Root concrete durable send/repair, shutdown and Studio operations revalidate the
same row under the physical cell and keep that serialization across synchronous
files. Release physical writers before awaited publication, preserving existing
native ordering/cancellation. Native cold row/snapshot/network/cache loading needs
the same scoped treatment. Actual removal/Leave closes the original row negative
before waiting and never recycles IDs. No dummy ServerActor for a native row.

## Required acceptance

Pin exact successor rejection, immutable binding/DM preservation, one-row changes,
terminal resurrection, highwater, stale rename/invite retries and unrelated-row
preservation. Exercise genuine Legacy/Founder/Joined mixed lifecycle, own removal,
rejoin without repinning, same-ID foreign mount and stale queued workers. Exercise
all actual upgrade/WAL/barrier/rename/cancellation cuts, no-effect refusal versus
uncertain admission, retained buffers and exact restart recovery. The1024-row
bound includes retained terminal receipts; receipt compaction and general history
pruning remain separate work.

Native UI authorization, cold restore and locked protocol activity require distinct
entry authority. UI-only cuts cannot restore before UI authorization is published;
early authorization and fabricated UI sessions are forbidden. The checked cold
factory and protocol producers remain implementation work, not implied by index3.
