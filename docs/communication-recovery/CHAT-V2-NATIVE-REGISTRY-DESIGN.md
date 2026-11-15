# Native vault/row integration: implementation design (13 November)

The physical declaration/repair portion of this design is now implemented in the
separate [native-registry installation API](CHAT-V2-NATIVE-REGISTRY-INSTALLATION.md).
The existing preparation API stays non-writing and ordinary default, Current and
governance readers still refuse namespace7. The remaining native backend/command,
index update and operational ownership boundaries below are not implemented by
that physical increment; Chat V2 remains disabled.

## Coordinated format allocation

Reserve critical namespace **7/version1** for the independent native backend
registry. Leave namespace6 for the separately planned joined-pin format. Do not
reuse ID5/version1: that descriptor declares only member-outbox storage, and an
ID3/ID5-aware predecessor would otherwise open without understanding native rows.

The new immutable descriptor binds the actual vault/root, the canonical Current3
descriptor and the native-registry format. Its configured handler must inspect
both catalog candidates and fixed native seed/marker artifacts before any older
cleaner. Every unconfigured opener refuses the new capability. All JOIN and
governance catalog rewrites preserve its entry byte for byte in either declaration
order; a sorted numeric position is not permission to omit another namespace.

Use a separate authoritative native index file, retaining the old registry bytes
as migration input. Do not add a trailing field to the old codec. Namespace7
declaration owns the exact initial index, including existing legacy rows, and
cannot finish without that file's covering file/parent barriers. A durable old-
reader Stage fence precedes content-bearing declaration outputs. A complete
authenticated seed retains exact descriptor/catalog/index candidates for warm
and cold repair; ambiguous or foreign files are preserved and refused. The new
handler never guesses an empty index from absent/corrupt/pending storage.

The migration reads/authenticates the entire old registry within explicit byte,
row and field limits BEFORE declaration effects. Duplicate IDs, unsupported data
or excess capacity refuse upgrade without discarding records. The initial seed
and declared index must cover the same exact migration input; a newly read or
partially projected legacy list cannot replace it during recovery.

Initial migration is private startup work only, before exposing the mounted
wrapper or registering any actor/row. No online declaration API is provided.
Capture either a proven NotFound association or the actual bounded regular file,
its held identity and exact authenticated bytes/digest; an empty load_registry()
result cannot establish absence. Recheck that original association through
commit. Future online migration would need its own freeze/drain protocol.

Do not expose normal index writes until initial index, catalog and marker
barriers AND declaration seed/stage cleanup are complete. A later authenticated
index generation alongside an old declaration seed is preserved and refused;
recovery must never reinstall generation1 over later rows.

## Bounded registry DATA

Use a strict versioned/tagged codec with vault/layout context, monotonically
checked selection generation, stable row identity, numeric UI ID and explicit
backend/state. Bound aggregate bytes, row count and each length before allocation;
reject duplicate IDs, unknown variants, noncanonical order and trailing bytes.

Initial hard codec limits: 1,024 rows, 4MiB total plaintext, 256-byte display
names, 64KiB legacy invites and 256-byte full group IDs; fixed identities are
exactly32 bytes. These caps are checked before variable allocation. Migrating
older data outside them refuses unchanged; it is not truncated. The declaration
invoice must cover BOTH complete catalog frames, held legacy input, decoded
metadata, initial index plaintext/ciphertext, declaration plaintext/ciphertext,
AEAD/unframing copies, parser metadata and replacement/control overlap. Compute
the named warm/cold maxima and validate the full paid layout before any physical
implementation; namespace5's smaller declaration invoice must not be reused.

Legacy rows retain their current metadata as a compatibility input. Founder V2
rows bind the genuine product's complete vault/layout/group/full-signer/SourceSaved/
Profile/pin tuple. Pending and retired rows remain explicit records. Parsing this
DATA issues no owner, key, native session, Admission or cold runtime.

PendingCreate has a separate shape: native creation intent/invocation, original
input signer, stable row identity, numeric ID and reserved network association.
It contains no guessed group/source/Profile/pin. Its exact durable pending row
precedes SDK ticket spend. The genuine complete product later supplies the full
Active binding; the source/root/kernel proofs govern that transition.

A crash after ticket spend but before SourceSaved does not permit rerunning the
same ticket or selecting another group under the old invocation. Preserve the
pending intent and inspect the original creation journals. A valid complete
source can proceed only through its genuine cold recovery owner. If no complete
source exists, explicit unexposed-creation recovery/abandonment must prove the
absence of a possibly accepted source before releasing that request; uncertainty
stays retained. This is distinct from cold operational reopen and remains an
implementation/acceptance requirement. Never silently relabel it Active/Absent.

Initial migration may create legacy records only. New pending/active V2 rows
must come from the checked producer/installation transaction; decoding or copying
the public binding DATA is insufficient. Publication of an active native row
follows its exact durable registry candidate. Ambiguous results retain both that
candidate and the original product for same-request reconciliation.

ONE mount-wide writer serializes every backend transition and legacy projection.
An admitted update names the exact original selected generation and frame digest,
merges against the complete index, and compares that same parent at commit.
Cache the complete sealed candidate before rename or late checks. An uncertain
commit fences ALL aliases and new candidates until exact lookup/reconciliation
of that candidate settles the original outcome; no writer may overwrite it with
another same-parent snapshot. Native row, persistence and UI fences remain
additional requirements, not substitutes for this index serializer/CAS.

An absent ID never authorizes a legacy write. Creating/joining a new legacy row
on a typed mount needs its own original prepared row selection before snapshot
work. Legacy projection updates merge against the whole selected backend index;
they cannot drop V2, pending or retired rows. Numeric ID reuse alone cannot grant
an older worker the replacement's storage or network authority.

## One mounted owner

Put the opaque mounted-vault sum in Root, with private legacy/typed variants and
concrete operations. Do not expose a Store getter, Deref, lease trait or generic
callback. Native keeps one physical mount. A true ServerEntry backend enum owns
either its actual legacy actor or its actual V2 product, never an optional sidecar.

Preserve the existing DurableSendLease constructor for older callers. A separate
mounted constructor and closed internal guard enum permit Root to perform the
legacy backend/row check without leaking the wrapped Store to native code.
Add a concrete mounted retired-fence operation that performs its save internally;
native retains its exact registry-absence guard throughout that synchronous call.

Common authentication, UI continuity, pairing-ledger, network/index and secret-
change operations must have concrete typed routes. Companion grants, legacy
snapshot/send/edit/delete/Stop, voice, cache eviction and legacy Leave explicitly
refuse V2/pending/retired selections. No unknown-state fallback is allowed.

If a typed loan retains physical storage after its outer native owner disappears,
retire the original mount before any wait, drain admitted work, then release the
physical mount. UI lock alone keeps the mounted source but retires its old command
authority. Backups freeze admission and drain ALL native V2 writers before copying
the opaque tree; the outer Store mutex alone is insufficient with escaping loans.
Passphrase changes preserve the DEK and existing post-rename uncertainty result.

## Key and save admission

Require an original native-issued row/session selector or capture synchronously
before dispatch. Sampling generation in an async handler's first poll can adopt
a post-lock generation. A selector is DATA, not authentication.

Register a Product-bound command owner against the actual session and row before
binding its immutable Draft/source/token once. After all waits and source proofs,
the actual key cut issues one checked non-Clone End, held through the key call and
immediate raw cache. Lock closes every such owner synchronously before awaits,
then drains Ends outside UI/Store/registry/Root guards. Unlock never revives an old
owner. Already cached raw or accepted receipts need no new key call; reply and
save admission still require fresh checks. Phase2's custody box is not phase1
authorization.

Admitted persistence moves immediately to a native-owned worker. IPC loss cannot
cancel it. Lock cannot report completion ahead of admitted work/outcome caching.
Leave/replacement retires the actual row before waits; durable V2 Leave must also
close the appropriate stored admission before registry removal. Cancellation or
cache deletion cannot stand in for that transaction.

## Acceptance before activation

- Use the genuine public founder producer on its original typed mount, never a
  fabricated Host, authorization boolean or test custody box as native proof.
- Test actual installation/save, immutable retries, uncertain index writes,
  same-ID replacement, unpolled stale invocations, lock/unlock at key/IO cuts,
  receiver loss and quiescent backup, with deterministic owned barriers.
- Exercise every declaration boundary, interrupted JOIN rewrite and both
  catalog declaration orders. Compare retained unknown bytes after refusal.
- The real predecessor gate must include the ID3/ID5-aware `a607123a` openers;
  `ce402364` alone already refuses ID5 and cannot prove namespace7 fencing.
- Keep cold-genesis rejection intact. A distinct cold operational owner,
  durable Leave, joined-member sources and governing transport/archive are
  hard activation prerequisites, not optional follow-ups.
- Run focused regressions while implementing; complete root/native/frontend,
  strict lint/audit and relevant native/flow gates remain required at handoff.

Read-only design review traced these boundaries against the actual native and
Root call paths. Source references and outstanding implementation are retained
in the native member-save prerequisite document. No code or completed acceptance
is claimed by this design.
