# Critical v5 gate for owner operations

Concrete format/ordering supplement for
[the retained operation tail](CHAT-OWNER-OPERATION-TAIL.md). The source is implemented and its actual
diff reviewed; compilation and test execution remain paused by the user.
This does not activate a scalable public profile or the native product. The existing fixed-founder
kernel uses owner-evidence vault **v4**; it is unsafe to assume that reader never ships or that an
unknown inner tail prevents it reaching its earlier stage cleaners.

## Wrapper and preserved data

- Add wrapper version `5`, exact same fixed wrapper framing/length and root DEK, but its own
  BLAKE3/Argon2 input domain `mewtual-vault-chat-owner-evidence/v5`.
- Add authenticated classification `VaultFormat::OwnerOperations`. Ordinary managed-scope and
  owner-evidence activation and passphrase change retain v5 once present; they never write v3/v4.
- `requires_owner_evidence()` classifies v4/v5 metadata for existing positive inventory/pin/head
  checks. It is not an admission/writer witness. New operation staging requires **v5 specifically**.
  The narrow v4 source-absent negative initializer remains v4-only until actual operation/journal
  absence is included in its proof; a broad metadata helper must not silently widen that exception.
- Every scope3/owner4 Current, Retired and Initializing row, pin, canonical source, original history,
  receipt, Stop and publication alias/uncertainty remains retained. The upgrade rewrites the DEK's
  passphrase wrapper only; it does not choose a new DEK or replace any data table with empty state.

An unchanged v4 reader refuses header5 before its Store constructor/cleaners. Changing that public
byte to4 cannot decrypt the v5 wrapper, because its KDF domain is different. This is a normal-reader
version fence, not rollback detection: replaying an authentic older v4 **wrapper alone** can reveal
the SAME preserved DEK to an old v4 binary. That old binary ignores the new marker and can reach its
old stage cleaners before rejecting a newer inner format. The marker protects the NEW reader's
recovery order; it cannot teach an old binary about itself. Preventing this local/backup component
rollback requires an external monotonic anchor or a separately agreed full rekey strategy. No claim
that an attacker must roll back every authorizing record is made here.

## Durable activation intent and automatic recovery

The App owns one fixed root record `chat-owner-operations.bin`. Its sealed plaintext contains only
`catcoms/chat-owner-operations-format/v1` and the existing32-byte vault domain, fully consumed with
strict framing. Maximum plaintext128bytes, maximum framed sealed record168bytes. Nonfile, oversize,
corrupt, foreign-domain and unsupported-tag records refuse; none means an empty/new installation.
It is activation **intent**, never a Current source or a successful wrapper/operation save receipt.

Its only stage is the fixed sibling `chat-owner-operations.replace.tmp`, created with create-new
under the original exclusive session. Recognition uses symlink_metadata for BOTH names, before any
absence decision or cleaner. A nonfile/broken link, oversized/truncated/corrupt or unsupported stage
refuses and is preserved; it is not silently absent. A valid stage with absent current marker is
re-synced through its actual file/root, promoted unchanged and parent-synced before wrapper upgrade.
If both exist, both must validate to the same immutable tag/domain intent; only then may the known
redundant stage be removed with its own parent barrier. An invalid current marker is never replaced
from a supposedly preferable stage.

The consuming, pre-exposure mounted-Store upgrade runs:

1. Authenticate the existing passphrase/DEK and consume the existing into_owner_evidence_store
   transition FIRST. A legitimate v3 vault may lack the owner-pin file: only that supported original
   absence can durably initialize an empty owner registry, while every present/nonempty row and
   whole uncertainty is retained. The v4 wrapper/parent barrier must complete before intent stage
   creation. No new operation, public response or operation namespace stage is admitted.
2. Write/seal the fixed activation intent through a real file+parent barrier. A partial intent write
   returns no usable upgraded Store. Exact retry does not silently discard a possibly renamed record.
3. Rewrite the same DEK wrapper as v5 through the existing interprocess transaction lock and actual
   file+parent barrier. Reauthenticate the resulting classified wrapper, compare the same DEK-derived
   keys and update the SAME mounted runtime format. Only then can the consuming method return.

The constructor recognizes current/staged intent before any owner/canonical stage cleaner. A present
valid intent with an older authenticated wrapper is a previous explicit upgrade's recovery
obligation, not a reason to ask the user to repair history. Before ordinary inventory constructors,
enter a dedicated private recovery loader: the validated intent requires BOTH existing scope and
owner inventories, regardless of the older wrapper's ordinary metadata classification. It does
not invent that classification or create an empty missing inventory. In particular, v3+valid intent
requires the already prepared valid owner-pin file and must not pass through the legacy v3 loader
which rejects nonempty pins. Invalid/missing inventories refuse with every retained file preserved.

After validation, re-sync/promote the actual current/staged intent and finish the exact DEK-preserving
v5 wrapper transition. Reauthenticate the opaque classified result and compare the same keys before
changing the SAME mounted runtime's metadata to v5. Ordinary cleaners/namespace exposure come later;
unsupported/current-pending records are never erased to get there. A v5 wrapper requires its valid
retained intent; absent current AND stage refuses. Any current/staged intent with a missing vault
refuses BEFORE open_or_create can generate a new root DEK, under the exclusive session. Use
symlink_metadata's exact NotFound rather than .exists() to establish that absence.

The intent alone never permits a new operation. Repeated failure leaves the existing records and
possible wrapper replacement intact for automatic reopen retry. A failure before wrapper rename
still permits the original v4 ciphertext to authenticate; a visible v5 after failed parent sync
still needs the managed reopen barrier. Original Store drop/revocation on failed consuming upgrade
prevents old queued work adopting the later mount. No borrowed successful Store is returned on an
ambiguous failure.

## Source and test invoice

The fixed marker limits are PER BUFFER:128 plaintext and168 sealed/framed input/output. A conservative
4KiB bound covers this helper's current+stage raw/unframed/plain buffers and a replacement
sealing/frame at once, plus fixed decoder state; it is a scratch estimate, NOT a charged runtime
quota/admission reservation. The explicit per-buffer caps enforce the local bound. It allocates no history collection
or peer-controlled count. Vault wrapper inputs
retain existing4096-byte new-secret and bounded legacy-secret behavior. All randomness uses the
existing injected RNG/crypto paths; no new clock or protocol RNG source is added.

Authored regressions must cover original v3/v4 rows/pins/debt surviving successful upgrade and cold
reopen; both actual marker/wrapper rename orderings and parent failures; missing/corrupt/foreign
intent; missing vault with intent; stale original mount after failed upgrade; original numeric
canonical aliases refusing; v5 sticky scope/owner/passphrase transitions; and header5->old-version
tampering failing without cleanup or wrapper replacement. Keep a real old canonical v1 source and
retained operation-stage sentinel when checking the pre-cleaner refusal/recovery order.

No tests/builds/compilation run while the user's pause applies. When validation resumes, default
MLS/root/native all-features is not sufficient for excluded SDK optional-feature code: explicitly
check `openmls_memory_storage/test-utils` and `/persistence`, including its alias decoder and preserved
separate std-HashMap JSON container. These are additional checks, not substitutes for mandatory suites.
