# Private native registry installation (13 November)

This implements the physical namespace7 declaration and cold repair. It does not
install a Chat V2 native backend, admit a command, sign a message or enable Chat V2.
The existing non-writing `PreparedNativeRegistryMigration` API stays non-writing.

`NativeRegistryInstallation::prepare` opens a fresh private Current3/member5 mount,
captures the actual old registry association, authenticates its complete bounded
contents and caches all sealed migration outputs. `retry` writes those exact
outputs without RNG. `into_native_registry` transfers only an opaque
`NativeRegistryStore`; there is no Store/Current getter or generic callback.
`NativeRegistryStore::open` uses the private paid handler to reopen and repair an
already-declared registry. Its current public operation is a checked DATA row count.

## Physical transaction

The fixed objects are the native declaration seed in the vault directory and the
native index/marker under `chat-owner-v2`. Namespace7/version1 is independent of
member-outbox namespace5. The old `registry.bin` remains unchanged as migration
input. Its actual held file, or strict NotFound association, is retained through
the entire initial declaration; absence is never inferred from an empty list.

The declaration order is:

1. Complete paid, validated, cached descriptor/catalog/index/marker/seed candidates.
2. A synced zero-byte claim of the seed name, then a covering vault-directory sync.
3. A synced empty catalog Stage fence and its covering directory sync. Readers
   predating namespace7 already reject that slot before cleaning older storage.
4. Complete authenticated seed, then the index and marker with file and parent
   directory barriers. Owned partial outputs may complete only their exact prefix.
5. Exact original current/Stage/input proofs, catalog rename and covering barrier.
6. Re-prove exact selected catalog/index/marker after the large seed verification.
7. Remove owned duplicate Stage and seed last, dropping every owned target handle
   before strict NotFound checks and cleanup directory barriers.

Every fallible warm cut retains the same candidate and actual opened objects.
Promoted/unlinked phases are recorded immediately after their filesystem effect,
before a later barrier can fail. Equal ciphertext in a replacement file does not
authorize a warm retry. Selection errors retain the recovery seed.

The pre-vault exception is private, prepaid and existing-vault-only. Missing vaults
still refuse orphaned artifacts; ordinary openers still refuse all reserved native
artifacts. The authenticated vault format must be CriticalNamespaces before any
native recovery write. The handler runs before older declaration/cleanup code.

## Cold classification

A complete seed must authenticate and validate its exact original/candidate catalog
pair, unchanged Current3/member5 descriptors, empty JOIN prefix (paused rows count),
and the deterministic mapping of the actual old registry into native generation1.
The selected catalog must equal one of those exact frames. Every existing output
is classified and held before a completion role issues. Writable captures must
match those same original file identities; a pathname recapture cannot adopt a
different equal object.

A promoted catalog requires an existing exact index and marker. Recovery never
recreates a missing committed output. A later native index or JOIN catalog beside
an old initial seed refuses unchanged; it cannot reinstall generation1 over it.
Ambiguous, unsupported or conflicting declaration state is preserved.

AEAD authentication is separate from grammar validation. An authenticated empty,
future or malformed seed stays preserved. Only a first seed that never authenticated
can be abandoned, with a valid pre-ID7 current catalog, no native index/marker and
no conflicting declaration. Its continuously held seed and classified empty Stage
are the only ordinary first-torn cleanup targets. Each unlink is followed by a
covering barrier and a strict NotFound proof.

## Predecessor governance during the claim window

Before the old-reader Stage fence exists, a predecessor may ignore the zero-byte
native claim and advance governance4. Refusing every such tree would strand both
the new Current-only native opener and the ordinary governance opener.

A narrow negative role therefore recognizes either the settled known4 marker or
its complete authenticated declaration seed, bound to the actual Current frame.
Only an unauthenticated native claim, with no native index/marker or ID7, may be
removed. **Stage is always untouched in this role**, including empty/partial bytes;
it may belong to governance or JOIN. Every governance object is preserved. The
Current-only native opener still refuses4 afterward; the genuine governance opener
can then finish/reopen its own format. Authenticated/future native or governance
seeds are not treated as disposable scratch.

This is not a configured governance4+native7 owner. Actual namespace7 installation
currently supports Current3/member5 without4. The DATA codec tests for both numeric
declaration orders establish preservation only. Governance's own first-torn seed
recovery remains outside this implementation.

## Resource ownership

`NativeRegistryPolicy` requires its own independent pool and fixed persistent
capacity. The complete preparation invoice remains charged; a separate named
physical invoice covers cold seed decode/copies, prefix and exact-file reads,
governance compatibility checks, handles and metadata. Both payments are acquired
before prerequisite opening can have effects. The public policy reports both
required capacities. These are logical bounds, not allocator/RSS or disk-space
guarantees, and do not reuse member-outbox credit.

Files and cached copies drain before their original payments. A shared registry
reader and the existing shared preparation revalidator serialize their retained
read inventory; a second reader refuses before copying another complete index.
The thread regression releases its first reader even if the second misbehaves and
uses bounded channels rather than unbounded barriers.

## Verification and remaining work

Focused regressions cover every warm declaration cut, complete-seed cold repair,
first-torn disposal cuts, byte-equal and in-place replacement, authenticated empty
and future seeds, late catalog replay, lost legacy absence, stale G1 evidence
beside a later index, original revocation and actual JOIN save/pause/resume/
migration/arm/cold reopen with the exact7 descriptor preserved.

The CI predecessor gate uses an unmodified
`a607123afbd3ebff2ef8ba10257f274fedef7eb3` checkout, which actually understands
Current3/member5. The only added file is its test harness. Sixteen head-written
vaults are driven through four capable public openers (Current5 and governance4/
member5, each with/without JOIN execution). Controls, claim-only and deliberately
unfenced copies must really open; protected cuts must refuse at open and preserve
the whole tree. The head then reopens an actual predecessor governance claim-window
output. Exact count guards reject empty, duplicated or changed evidence. Pending
execution is not a passing compatibility claim.

Still required before Chat V2 activation: serialized native index updates/CAS and
uncertainty reconciliation; the opaque mounted-vault/backend sum; PendingCreate
before SDK entry; genuine native row/product installation; original command/key
and final save admission; cold operational ownership; durable Leave; joined App
sources, trusted pins and governing archive/transport; user/admin UI and combined
adversarial acceptance. Global strict-lint and dependency findings also remain.
