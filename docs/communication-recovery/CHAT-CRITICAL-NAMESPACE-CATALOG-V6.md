# Critical namespace catalog v6 (5 November; authored, unexecuted)

Current extension: an explicitly configured private Pending-only JOIN1 handler now has a larger
actual shared invoice and original fixed-file producer. Normal/Empty readers keep this document's
Empty-only behavior and refuse the new catalog-v2 tag. No ARCHIVE2, network/ready or native
activation follows. See [Pending checkpoint](CHAT-PENDING-JOIN-CATALOG-CHECKPOINT.md); tests remain
UNRUN during the compilation pause. The earlier Empty prerequisite below remains unchanged.


This is a narrow prerequisite for destination join custody and future retained proof archives.
The implementation currently supports an EMPTY namespace set only. It implements no join journal, archive,
fresh admission, historical authority, publication grant or active native host. Compilation,
tests and builds remain paused. Thirteen App tests (one Unix-only) and four Storage tests are
authored and UNRUN; the regression list below is not execution evidence.

## Compatibility and grammar

Vault wrapper v6 uses a distinct passphrase KDF domain and preserves the same root DEK and all
derived data keys. Normal v1-v5 readers reject it; changing its public header back to v5 does
not authenticate. This is a normal-reader fence, not an external component-rollback anchor:
replaying an authentic older wrapper ALONE can still expose the same preserved DEK to an old
binary, and a complete authentic older catalog is not protected by external monotonic storage.

V6 requires one sealed `chat-critical-namespaces.bin` record. The fixed possible replacement
is `chat-critical-namespaces.replace.tmp`. Plaintext grammar is a fixed tag, original vault
domain, checked positive generation, namespace count, then sorted unique entries containing
u32 namespace ID, positive u32 grammar version, and a u32-framed opaque payload. Namespace IDs
are permanent compatibility labels. A later supported namespace must be declared durably in
this catalog BEFORE creating its dependent files. Unknown ID, version, flags or state is a
critical unsupported format; it can never be ignored or downgraded to Empty.

This first implementation accepts only canonical generation 1 with count 0 and a fully consumed
frame. It parses bounded framing, including the declared entries, then refuses ANY nonempty
set in EITHER current or stage. No identifier/version is advertised as supported. The closed
small input bound admits each complete source before unsealing. The fixed logical inventory is
<=553 bytes raw input, <=528 bytes unframed ciphertext, <=512 bytes unsealed plaintext, 512 bytes
encoder capacity, <=528 bytes sealed output and <=552 bytes framing, plus two retained file
handles and fixed domain/key/codec state. Initial write verification may overlap those buffers;
an8KiB logical metadata allowance covers these named objects. This is not a global host-pool
debit, allocator/RSS measurement or SDK bound. Namespace set
growth, nonempty payload decoding and a larger explicit invoice require their actual consumers
to land together. Join1 is intended to embed its bounded journal payload directly, avoiding a
separate mandatory-file/bootstrap-absence ambiguity. Future archive declarations remain unknown.

## Constructor order and source preservation

1. Acquire the real physical vault session lock.
2. Before `open_or_create` could generate a new DEK, inspect the fixed catalog current/stage names.
   If the wrapper is absent or not a regular file while either catalog source may exist, refuse.
   Broken links and permission errors are presence/uncertainty, never genuine absence.
3. Authenticate the actual wrapper and obtain the SAME keys/format classification.
4. Read/authenticate BOTH fixed catalog sources with strict type/length bounds. If either source
   is malformed, foreign-domain, noncanonical, nonempty or unsupported, refuse before invoking
   any previous inventory gate or cleaner. No source is deleted in this case.
5. A v6 wrapper without a valid catalog refuses. Complete catalog absence under v1-v5 uses the
   unchanged prior gate. A valid current/staged empty catalog requires the EXISTING v5 activation
   intent, admission and owner-pin inventories, including when an older authentic wrapper remains.
   Catalog metadata cannot initialize missing prior lineage or manufacture an owner pin.
6. Cross actual file and root barriers for the known original catalog sources. A lone valid stage
   can be promoted; a duplicate stage can be removed only after BOTH complete candidates match
   and their real barriers succeed. Distinct or unsupported candidates refuse/preserve.
7. If needed, rewrap the same DEK as v6, cross the actual wrapper/root barrier, reauthenticate the
   same keys and confirm its actual format on the same original mount. Only then return the Store.

Every failure drops/revokes the constructor's still-private mount. No borrowed half-upgraded
Store is returned. Physical-session exclusion makes the fixed-source classification stable;
no catalog writer exists on an exposed Store in this empty-only implementation.

## Consuming activation and sticky behavior

`into_critical_namespace_store` consumes a private Store. It first completes the existing
supported owner-operation activation, retaining every original admission/pin/canonical source.
It then saves the canonical Empty catalog through a create-new fixed stage, file sync, rename
and actual parent/root sync, and only afterward activates/reauthenticates wrapper v6. Failed
stage, rename or parent barriers retain their original source for constructor repair; no empty
replacement is used to overwrite an existing unknown candidate.

V6 is sticky through scope/owner/operation activation and passphrase changes. Metadata helpers
may classify v6 as requiring existing owner evidence/operation inventories; that is not catalog
validation or a runtime capability. Only the actual Store constructor/consuming transition
establishes the supported catalog. Exact v4 source-absent negative initialization and v4 staged
first-initialization fallback remain exact v4; neither can infer absent journal/archive debt in v6.

## Authored tests (UNRUN until pause lifted)

* Actual activation/cold reopen preserves DEK, Current/Retired/Initializing rows, pins and canonical
  bytes; ordinary legacy opens remain unchanged and v6 activation/secret changes cannot downgrade.
* Unknown/nonempty current and stage, malformed/truncated counts and payloads, duplicate entries,
  foreign vault domain and unsupported generation preserve bytes and run before any old stage
  cleaner. Include a real root-pin stage that would be deleted if ordering were wrong.
* Catalog current/stage absence, nonregular and dangling-link sources refuse; no new DEK or RNG
  consumption occurs for catalog orphans without a regular wrapper.
* Actual retained-stage sync/rename/parent failures recover or conservatively refuse; mismatching
  stage/current are never discarded. Missing required old inventories are not recreated.
* V6 source-absent negative initialization and staged fallback remain closed while known positive
  canonical classification remains reachable; no test claims complete join or SDK admission.

Read-only design and actual-diff reviews are mandatory. Source formatting/whitespace review does
not establish executable validation. The full destination design remains separately incomplete in
[CHAT-DESTINATION-JOIN-CUSTODY](CHAT-DESTINATION-JOIN-CUSTODY.md); bounded active/paused attempt slots
must be finalized there before activation so an unreachable inviter cannot block every other join.
