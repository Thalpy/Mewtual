# Private index3 metadata WAL

Status: implementation and focused validation in progress. Chat V2 remains off.
This is local storage work. It supplies no native command credential, membership
proof, SDK entry, network publication or text-chat activation.

The first concrete normal3 producer changes one active row's local display name.
It compares the stable numeric ID, immutable identity and expected metadata
revision. One successful change increments that row's revision and the index
generation; other rows, migration origin, creation receipts and ID high-water
stay unchanged. The closed `Index::successor_data` check requires exactly Rename.
Legacy invite refresh now uses the same cell and recovery, with a separately tagged
request and shared revision; see [the refresh contract](CHAT-V2-NATIVE-INDEX3-INVITE.md).
Founder-only Birth and activation now use that same cell and normal WAL.
Birth freezes the whole original request, including the exact sealed network
reservation, and assigns `last_issued_id + 1` with checked arithmetic. Activation
matches the immutable receipt and rebases on the latest selected index, retaining
neighboring names/revisions and other new rows. The public status is historical
DATA; it cannot recreate a cold creator or install a runtime.

The frozen-input invoice additionally covers the maximum 64 KiB network frame
plus the 256-byte local name. Exact retained retries compare every tagged field
and keep the same candidate ciphertext. An exact completed receipt can be
observed without sealing again; a conflicting receipt refuses before new RNG.

Joined/LegacyJoin Birth and activation, Abandon and Retire remain unimplemented
writers/recovery operations. Valid authenticated stages of those kinds are
preserved and refused. The private supported-transition classifier checks both
authenticated indexes after general successor validation: accepting a generic
Birth/Activate label alone would accidentally enable those future producers.

The actual checked native command, retained whole driver/runtime and completed
debt retirement consumer remain unfinished. Chat V2 stays disabled.

## Retained warm operation

The opaque selected3 physical allocation keeps one `Mutex<Cell>` before its
original Current. The Store's affine closer is its first field. Concrete metadata
methods use that same cell through the facade or one bounded private physical
capsule; no raw Store/Current or owned index snapshot escapes.

Preparation, decoding, canonical encoding, sealing and allocation happen inside
this paid cell before the catalog claim. The independently reserved version3
invoice covers the selected/candidate indexes, ciphertext, original file proofs,
IO owners and fixed metadata. A private narrow `PaymentTail` extends that same
invoice's lifetime; it does not grant another allocation or retain cold evidence.

Under the actual catalog mutex, final admission checks Pending absence, the
original handler, Current, native marker/catalog and physical parent. The claim
then moves with the whole transaction into the cell before any file effect.
An uncertain result retains that request and its original sealed candidate.
Only the exact request may retry it; retry neither reseals nor draws RNG.

The real cell mutex guard stays held across IO. An unwind after admission poisons
the mutex and cannot be repaired by clearing the poison or recapturing a parent.
A cold reopen is required. Close before admission has no effect. Close after
admission cannot expose warm success or reopen the gate.

After the covering file/directory barriers and final original-tuple proof, the
completed transaction moves out as finite `CompletedDebt`. Its original claim
stays active while IO, files, decoded buffers and the physical-session lease
retire. Only its LAST nested acknowledgement field releases that claim. A foreign
completion closes the gate. The acknowledgement performs no IO or catalog work.
This prevents new reads, writes and JOIN work from overlapping outgoing maximum
buffers. The physical lease also prevents another opener if the Store itself
has already dropped.

The future native consumer must move completed debt into an outer retirement
slot before leaving any native commit guard, including error and unwind paths,
then destroy it outside native/cell/catalog guards. That consumer is unfinished;
these storage tests do not establish its retirement ordering.

## Cold classification and recovery

The paid strict3 inspector runs before common Current/JOIN cleanup. It validates
the selected native index and marker against the independently authenticated
archive/Current pair. A semantic `VerifiedWal` binds the decision to THIS held
inventory, selected framed hash and actual Stage hash. Equal bytes captured by
a second inventory cannot substitute for the original proof.

Supported normal Stage cases are deliberately closed:

| Actual Stage | Action |
|---|---|
| One authenticated founder Birth/Activate, Rename or Legacy invite-refresh successor of this exact framed parent | Promote and cover the same held Stage |
| Byte-identical framed selected index | Cover selected data and remove the duplicate |
| Attributable 0–39-byte Stage | Cover selected data and remove the incomplete Stage |
| Bounded frame with an actual AEAD authentication failure | Cover selected data and remove the failed Stage |
| Authenticated invalid/future data or another Change kind | Preserve and refuse |
| Oversized, ambiguous or simultaneous JOIN/index debt | Preserve and refuse |

The dedicated3 IO role permits its own Stage while retaining every foreign
declaration/upgrade exclusion. The older declaration helper remains unchanged.
Before index replacement it repeats the full static native marker/catalog tuple,
then the exact destination, held Stage, directories and original Current pair.
Visible rename/unlink flags cache progress before fallible barriers, but never
mean completion. Retry repeats covering barriers and final proofs; a missing
selected output is never recreated.

The same completed IO/cleanup and original handles remain in cold `Observation`
through the common opener and final exposure. No fresh capture can adopt a
replacement index after repair. The observation's finite session lease retires
after its original files, with no Store or Handler back-reference cycle.

## Observations and compatibility

Rename/Refresh metadata CAS has no durable invocation token log. Founder Birth
retains a distinct immutable creation receipt; its DATA lookup is not runtime
authority. After completion or
restart, a stale revision refuses even when the desired name happens to match.
Current revision plus equal name is an unchanged observation and draws no RNG.
Neither observation acknowledges a prior native invocation by value equality.

No selected3 grammar or key domain changes. Old1/2 remain closed. The pinned
pre-normal-WAL3 reader must accept clean original/renamed3 controls and preserve
unknown native Stage without opening it. The actual predecessor harness and
head round trip run separately from the ordinary unit suite.

Focused tests cover real warm and cold cuts, exact retained retry, close on both
admission sides, post-effect poisoning, debt exclusion, cleanup covering barriers
and equal-byte object replacement. All eight owning cases pass before the final
barrier-observation strengthening. The actual cfd837a7 predecessor passed all
9×3 probes (14.18s), and this head verified all9 round trips (10.18s). The complete
79-case lifecycle regression run passes (357.93s). After narrowing three internal
items to their actual Lifecycle callers, all8 affected cases pass again (76.47s).
All3,441 frontend tests, Svelte checking and the production build pass. Strict
Clippy remains red with600 errors, including unconnected integration code; the
dependency audit still reports13 vulnerabilities. No gates are waived. Full
Root/native validation remains open. This does not close Chat V2 acceptance.
