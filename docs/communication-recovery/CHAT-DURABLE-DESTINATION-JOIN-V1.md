# Durable destination join1 (5 November; proposed, not implemented)

The implemented in-process source and Empty-v6/read-pool prerequisites do NOT yet provide this
durable outcome. This design extends the finite canonical checkpoint client; production v2
Profile/invite/header distribution, governing/native activation and complete SDK/global memory
admission remain independent open gates. No join namespace is currently supported and no new
tests have executed during the compilation pause.

## Closed namespace and storage transaction

Permanent catalog namespace IDs are JOIN=1/version1 and ARCHIVE=2/version1. JOIN payload is at
most16MiB; the future archive descriptor is at most4KiB and points only to fixed authenticated
layout, with mutable archive roots in their own records. No message append rewrites the catalog.
JOIN's private journal is embedded directly in its sealed catalog payload, avoiding a separate
mandatory-file/bootstrap-absence gap. Adding a namespace is a durable monotonic declaration.

Proposal for extended catalog framing: retain the existing v1-tag generation1/EMPTY bytes
unchanged, introduce an explicit v2 catalog tag for nonempty sets with original vault domain,
checked generation, previous COMPLETE canonical plaintext hash, and sorted unique ID/version/
payload frames. First nonempty generation2 must name the actual original Empty parent. Each later
stage must name the actual current digest and checked next generation; namespace set removal or
unsupported ID/version refuses. An old Empty6 reader refuses the new tag before old cleaners.
This is catalog-format versioning, not a change to signed chat action or owner protocol bytes.

Both fixed current/stage candidates authenticate and completely preflight before old gates or
cleanup. Equal known plaintext duplicates can settle their real barriers. A known next stage
must satisfy its exact parent and the closed transition grammar before promotion. Foreign,
malformed, missing-parent or unknown candidates preserve/refuse. A nonempty stage alone is not
an empty-vault or fresh-admission proof. A renamed exact current candidate retries its real file/
parent barriers; Current inventory alone never invents Saved.

One synchronous catalog serializer retains an exact uncertain CATALOG successor after write
failure. No later catalog rewrite may overtake it, and dependent join-source effects stay held.
Retry keeps the SAME complete framed candidate, parent and authority instead of resealing a new
choice. This is NOT a global pause of independently admitted archive/control work: immutable
already-declared descriptors remain valid across every known possible successor. Ordinary
archive/control records use their own original scoped writers and must continue when their
dependencies are unchanged. New namespace declarations and join effects depending on the
uncertain catalog state wait for its resolution. No catalog mutex stays held after refusal or
over network work. Unknown cold sources still refuse before exposing any unsupported handler.
All mutation remains in Store persistence.

## Slots and duplicate intent

Admit one active attempt plus four retained paused attempts, with a separate explicit persistent/
hot allowance. Pausing preserves the original private source and frees scheduling capacity only
after its durable transition; it does not undo a remote Add or authorize minting another package.
A full retained pool refuses a new attempt rather than evicting ambiguous keys. This is a local
concurrency/resource policy, not a group-member or lifetime-admission cutoff.

Rows use a cryptographic source nonce and checked local revision. Scheduling (Active/Paused) is
separate from progress (Pending/ValidatedReply/ScopeIntent/FrozenCandidate/Settled). Unknown states/flags refuse. Source
identity is immutable across all phases; only Settled rows may be recycled under exact authority.
The signed history remains in its canonical/archive ledger; journal recycling is not chat deletion.

An exact invite already retained must coalesce/resume its ORIGINAL source, even if the caller
generated an unused replacement device. A different full invite encoding with the same group/
nonce must not replace that source. At most one unresolved source may own the same group/full
signer. A warm already-active worker returns a bounded in-progress preparation rather than a
second worker. Native task joining/UI treatment is not supplied by a decoded status bit.

## Immutable original source

The journal and source binding encode independent protocol-flavour and ROOT-KIND discriminants.
Only finite OwnerFact-v1/checkpoint-direct is initially supported; same-width public-owner-v2
Profile identifiers and actual v2 founder output cannot be coerced into this proof type. Unknown
root kinds preserve/refuse until the genuine v2 invitation/root-distribution adapter exists.

Before the first request, capture from the actual trusted root and actual destination Store:

* original group/full signer, finite-checkpoint Profile type/ID and founder;
* exact original optional admission-row CAS basis (nonce/revision/phase/origin/provenance);
* vault domain, source nonce/revision, exact invite/KP/request hashes and selected KP reference;
* immutable secret prejoin master produced by the actual `PreparedDirectJoin`;
* original negative mount and journal-active observations in memory (never serialized alive bits).

Persist an authenticated original-source envelope before any network request. Proposed Sync
boundary is seal-from-ACTUAL-Prepared, then authenticate/open encrypted original with the SAME vault
key and exact immutable journal binding. No plaintext snapshot -> Prepared constructor is added.
The authenticated-open output is an opaque source, not Fresh/Current/Saved authority. It checks
the complete bounded frames, canonical request, signed invite, historical data-only package
reference/subject/credential binding and master full signer. Historical label extraction does not
validate lifetime or admit a new package. Actual selected-KP Welcome processing still creates
fresh completion only on a private candidate; new owner admissions retain all fresh policy checks.

The App's ONLY recovery constructor consumes an actual mounted journal-file capture, compares
its original CAS basis against current lineage and checks the actual immutable pin. It does not
accept an arbitrary opened source plus a new capture. New-mount recovery obtains a new negative
observation for the SAME authenticated source; old queued tasks keep cancelled old observations.
Persisted root expectation is inert prior trusted configuration, never a downloaded replacement.
Conflicting installed pins, foreign/later lineage or an exact retired source refuse positive work.

Sealing is a fixed original-source operation, not a generic secret export/retarget adapter.
Private process state distinguishes freshly minted local source, sealed-local source and
authenticated recovered source. Seal is allowed only before any request has started and binds
the source once; same-candidate retries reuse the exact cached ciphertext. A recovered source
cannot be resealed under a NEW binding to turn old keys/reply into a different local attempt.
The original encrypted envelope binds its complete request/master to that exact source identity;
only actual Store recovery may supply the matching current journal capture. No serialized
"never sent", "paid", "alive" or accepted bit grants authority.

The initial source flavor is explicitly the finite legacy-direct checkpoint. Production v2 join
flavor is unsupported until its actual signed Profile/header-floor/request producer and consumer
exist. Unknown source flavors refuse; they cannot silently enter legacy completion.

The original active journal observation revokes before pause/replacement. Every final storage
admission rechecks the exact source/activation revision; an old worker cannot adopt a resumed row.
The existing typed Bound<T> owns the same actual transport and consumed event prefix across warm
retry. Cold explicit recovery may bind a new transport with an EMPTY prefix, never infer a prefix
from PeerId or an unordered snapshot. No private key is reminted to make a retry convenient.

## Accepted reply, local save and terminal recovery

The actual successful Sync candidate must retain an opaque receipt containing its exact
authenticated fully processed reply, selected package and Welcome binding. No plaintext reply
getter or decoded phase can fabricate it. `OwnerJoinedInitialization::retry(store)` first saves
that original ValidatedReply durably, BEFORE reserving local admission. A failed save retains
the actual private candidate/receipt; a merely signed invalid first reply cannot poison the row.

### Exact initialization nonce across the reservation gap

ValidatedReply is followed by a durable SOURCE-BOUND SCOPE INTENT before any admission-row
mutation. The intent contains the original optional CAS row, checked next revision, ONE sampled
nonce, Fresh provenance from the actual completed Welcome, and exact source/activation identity.
Its opaque preparation is built from the actual private completion; decoded fields alone cannot
issue an intent. Encoding/sealing happens outside writers, then original admission -> catalog
guards recheck the original row/source/whole-parent CAS before saving its exact candidate.

Only after that intent's actual catalog/file/root barrier may a narrow private admission consumer
install its precomputed record. It samples NO new nonce. Under the actual admission writer it
accepts only (a) the exact original CAS basis, installing the intended Initializing record, or
(b) the EXACT intended nonce/revision/Fresh provenance in Initializing or Current. Any later,
foreign, changed or Retired record refuses, including an equal-key/equal-provenance row with a
different nonce/revision. Overflow refuses. Existing generic admission APIs remain unchanged.

Warm failures retain the original intent/preparation; cold explicit recovery obtains a new
negative observation for that SAME sealed intent and actual row, never a newly sampled scope.
Initializing continues private source recovery; Current selects actual canonical restore rather
than replaying an initializer over its saved head. Intent tuple is threaded through pin, stage,
Current and Settled checks. If the row advanced but its required prior intent is missing, keep
the source unavailable/preserved; do not infer authority from provenance alone. Tests must crash
immediately before/after intent save and before/after prepare_chat_admission, plus later/retired/
foreign same-key rows. This closes the random-nonce gap identified in design review.

Pending-without-reply recovery queries the original owner result with the same request. A saved
ValidatedReply may be replayed offline through the SAME actual signature/selected-KP/group/policy
checks on the private master. It does not mint a proof by decoding status. Original owner facts
must still be available/resupplied and verified before canonical member initialization; this
design does not manufacture missing historical membership or current public-header authority.

### Exact frozen candidate before first canonical I/O

After the actual private Welcome completion, original owner evidence, ScopeIntent admission and
pin checks, freeze that SAME private Server once. Persist the EXACT frozen initial plaintext
in a FrozenCandidate journal record before ANY canonical write. The record is constructed only
from the actual private freeze; there is no public caller-plaintext or decoded-Frozen constructor.
A full source-bound record retains the immutable source nonce, original request/KP/reference,
authenticated Welcome binding, pinned root and original facts, exact ScopeIntent, snapshot length
and digest, and intended generation1 canonical context (vault domain, nonce, revision, profile,
original completion provenance and EMPTY parent). The snapshot cap is at most1MiB and never more
than the original actual creation reservation. This is an additional explicit retained invoice.

The secret master and large reply may be replaced by this complete frozen representation ONLY
after its actual catalog/file/root barrier. Before that barrier, failure retains the original
source, candidate and exact proposed frozen bytes. After it, retries never reconstruct/re-encode
MLS maps or substitute a different display name/RNG; exact bytes and digest survive pause,
remount and changed map iteration. The original public source/Welcome/fact bindings required to
verify and close the member initialization must remain after master/reply disposal.

New-mount recovery consumes an authenticated ORIGINAL catalog-file/source capture and obtains a
new negative activation lease for that SAME durable source and immutable intent. The source and
intent identity never become the scheduling revision: pause/resume advances/cancels the worker
lease without retargeting or resampling either. Old queued workers cannot adopt the new lease.
Decoded progress, Fresh provenance, a hash or an equal group cannot manufacture this capture.

FrozenCandidate plus the exact intended Initializing row uses a NEW journal-bound v6 gen1 member
adapter. It validates the complete closed snapshot and initial chain, actual own credential,
immutable pin, exact source/intent/canonical context, and ALL supported critical namespaces under
original guards before writing/promoting ONLY those exact bytes. It does not call or widen v4 S1.
FrozenCandidate plus exact Current instead selects actual canonical restore, requires its full
exact source/digest/context comparison and real covering terminal; it never replays a regenerated
first initializer. Foreign/later/retired heads or unknown namespace/debt preserve and refuse.

Under original admission -> canonical head -> catalog serializer order, canonical bytes, actual
Current and exact Saved/head/writer barriers precede the journal's Settled terminal. Only after
that terminal's actual file/root barrier may the private runnable-owner handoff consume its graph.
No catalog lock is held while acquiring admission/head or over network work. Reply persistence
before scope reservation uses its own original capture and releases the catalog lock afterward.

Lost terminal reply/reopen must resolve the SAME actual canonical source and compare its verified
original bootstrap/KP/Welcome/root/scope against the journal; it must not replay a first initializer
over an already saved head or build a replacement from a different name/RNG. A Settled record can
describe that source but supplies no Fresh/Current/Saved witness. Terminal retry must consume the
actual restore saver/covering result before clearing original secret material.

Separate v6 member-stage recovery is REQUIRED before claiming complete crash recovery. Existing
exact-v4 S1 and source-absent negative gates stay unchanged. A future journal-bound gen1 adapter
must verify actual retained source/ValidatedReply/pin/scope, full initial chain and the complete
supported catalog/debt namespace under original guards BEFORE stage promotion/Current. Current
closed owner operations require prior Saved/Current gen1 and have no separate debt file beyond
canonical .bin/.replace.tmp; that fact is not a future generic absence witness. The new v6 adapter
needs its own actual connected proof and independent review. Unknown/gen2/foreign stages remain
preserved/refused; local negative closure never undoes a remote Add.

## Exact resource ownership before allocation

The configured opener must hold an actual shared catalog-read reservation BEFORE larger frames.
Saved limits are only data. Default open remains Empty512 and refuses nonempty catalogs. Dormant
slots retain no full plaintext master in Store runtime. Metadata and any encrypted/decrypted
source copies get separate original strong reservations before allocation, transferred with
their data; returning transient scratch cannot strand a surviving uncharged copy.

The16MiB limit is for the WHOLE JOIN payload across all five slots, not per slot. Proposed fixed
per-row maxima are sealed master/source640KiB, reply wire2MiB (sealed envelope overhead separately
counted), original verified fact cache64KiB and fixed public metadata2KiB. Full encoded-size/count
preflight additionally enforces the whole payload cap before output. No budget is reconstructed
from a saved status/limit. Capacity exhaustion preserves all prior slots.

Checked implementing constants must name `C = JOIN16MiB + ARCHIVE4KiB + HEADER512`, framed maximum
`F = C + 40`, and unframed/decrypt allocation maximum `U = C + 16`. Sequential source classification
may drop each whole plaintext after copying ONLY bounded paid metadata/digests before reading the
other source. Its reservation covers `(F + 1) + U + U` plus BOTH metadata records. Any retained
metadata/source copies get their own debit BEFORE copying and keep it after scratch drops.
No whole plaintext for dormant slots stays in Store runtime.

A new writer re-reads the confirmed current source under its actual read reservation and retains
that parent plaintext while a SEPARATE admitted writer allowance owns new encoder `C`, ciphertext
`U` and framing `F`. Parent/source/activation CAS is repeated after encoding under the original
writer order. No mutex encloses encoding/crypto/large allocation. Named parent/read and writer
reservations overlap until their data are gone; they are not accidentally refunded merely because
one phase returned. Persistent allowance covers BOTH current and retained candidate/stage, at
least `2 * F`, including all embedded source/reply/proof/frozen copies. These formulas are proposed
concrete maxima; the actual implementing helpers and shape/drop table still require review.

Read invoice names raw framed input, unframed ciphertext and AEAD decrypt output capacities for
both classifications; aead0.5.2 decrypt copies ciphertext and then truncates, so its allocation
may include the16-byte tag. Writer invoice separately names retained parent plaintext, new
candidate encoder, ciphertext and framing, plus fixed row metadata. The pinned aead0.5.2 encrypt
preallocates message+tag before encrypting; no uninspected growth multiplier is assumed. Root/
native locks never enclose encoding, crypto or large allocations. Counter locks enclose none.

Warm original master/request copies and final retained raw candidate provider keep their actual
fees through ownership transfer. SDK typed JSON/Welcome/tree/group transient heaps, active server
graphs, lower network ownership and a process-global native/UI owner are still independent gates.
These catalog/journal resource reservations do not claim to close them.

## Required adversarial acceptance

Exact original request/KP/private source through reject, forged/signed-invalid reply, timeout,
pending cancellation, pause/resume, lock/unlock, remount and owner restart/expiry; one actual Add;
five-slot coalescing and no ambiguous eviction; malformed/duplicate counts, mixed namespace/
unknown versions, parent/revision overflow and capacity before allocation; every actual stage/
rename/root boundary for Pending, ValidatedReply, ScopeIntent, FrozenCandidate, member canonical/Current and Settled; both
lock-vs-publication orderings; actual selected-KP v6 member stage recovery and exact canonical
terminal reopen; exact frozen bytes survive map-order/name/RNG changes and every Frozen journal
barrier vs first canonical boundary; pause/remount stale-worker refusal; another group makes actual archive/control progress while an unrelated join or
catalog status retry remains pending under unchanged already-declared descriptors. All fixtures must use the genuine owner producer where its covering claim is
made. Source-cache or in-memory tests alone establish neither durable client completion nor
physical/native acceptance.
