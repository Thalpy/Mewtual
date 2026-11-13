# Native registry: original migration preparation

13 November. This increment prepares and checks the exact initial native registry
on a genuine newly opened private mount. It **does not write namespace7**, install
a native row, provide a physical namespace7 handler, freeze another writer or
enable Chat V2. See [the complete design](CHAT-V2-NATIVE-REGISTRY-DESIGN.md).

`PreparedNativeRegistryMigration::open` validates independent funding before
opening the vault. It finishes the Current3/member5 opener, which may declare
or repair **those existing formats**, then retains that whole private owner.
No Store, key, Host, SDK, row or generic effect callback escapes. Failed native
preparation writes none of its outputs. Drop discards its uncommitted memory-only
candidates and spends no creation ticket.

All existing openers refuse the reserved native seed, marker, index and index-stage
names before older cleaners, including orphan checks before vault creation.
Presence of any kind or access failure is not absence. No opener recognizes an
ID7 catalog yet. This negative fence does not establish successful declaration,
migration, or predecessor compatibility.

## Original inputs and cached outputs

The preparation holds the original vault/root directory handles, selected catalog
File/frame, and either the original bounded legacy registry File/frame or a strict
NotFound association. Revalidation compares real named directory identities, file
identities AND full bytes, and both parents around leaf reads. Equal bytes at a
replacement inode and changed bytes at the original inode both refuse.

Current3 and the actual member marker must validate. Both catalog sources and all
older declaration seeds must be settled. The mounted JOIN writer must have no
pending candidate, and **every JOIN row**, including paused rows, must be absent.
This is a newly opened private owner, not a borrow of an already exposed Store;
it nevertheless issues no exclusion grant to a future physical writer.

Legacy input authenticates with the existing database key. Complete bounded
preflight checks row/field/native-framing totals and duplicate numeric IDs before
copying names/invites. It preserves legacy v1 and nonzero v2 DM-flag semantics.
Excess or ambiguous data refuses without truncation.

The strict native index binds version/tag, full capability context, generation,
framed-ciphertext parent, canonical numeric order, unique stable row identities
and explicit Legacy/PendingCreate/Founder/Retired/Abandoned states. These are DATA
only. Pending input has no guessed group/source/Profile/pin. This preparation
produces only deterministic **legacy generation1 rows** from the exact input.

Exact original/candidate catalog frames, initial sealed index, marker and complete
seed are cached once. Independent key domains separate index, marker and seed;
the descriptor binds the canonical Current descriptor. The candidate inserts only
ID7/version1, preserving all other entries exactly. Validation also recomputes the
complete legacy-to-index mapping, rejecting authentic but inconsistent seed/index
contents. Hashing two independent authentic inputs together is not sufficient.

The count getter reports historical DATA, not installation. `revalidate_inputs`
checks the same originals and cached outputs without RNG, replacement adoption
or writes. Final original cancellation is checked after reads, AEAD and temporary
destruction. A physical consumer must separately acquire actual startup/writer
exclusion and revalidate at commit.

## Payment and validation

`NativeRegistryPreparationPolicy` requires an independent account, refusing both
catalog and member-pool aliases. Complete `required_capacity()` is reserved before
opening or RNG. This is logical accounting, not RSS or physical RAM; buffers use
actual bounded lengths rather than speculative maximum allocations.

Named terms in `preparation.rs` cover retained original/candidate catalog frames,
legacy input, index, marker and seed; seed encoder/AEAD/frame/unframe/plain overlap;
complete catalog authentication; legacy/index decode and clone overlap; bounded
row/uniqueness scratch, paths, handles and metadata. The reservation drops after
files, outputs and mount. Current/JOIN/KDF/SDK and future physical-writer invoices
are separate. Namespace5's smaller invoice is not reused.

CI pins26 codec/preparation cases, one real directory-association case and one
genuine active/paused JOIN scenario. The member-storage cohort grows from17 to18
because it also includes that new JOIN scenario. No existing test is removed,
ignored or weakened. HANDOVER records execution; authored tests are not evidence.

Actual-diff review caught a directory check using a regular-file-only helper.
It now retains/reopens the named real directory and compares OS identities, as
the established Current handlers do. Directory replacement, same-inode byte
change and genuine paused-JOIN regressions address the remaining coverage requests.

Still required: physical ID7 declaration/recovery and JOIN/governance preservation,
ID3/ID5-aware predecessor tests, mount-wide index serializer/CAS and uncertainty
handling, native mounted-vault/backend enums, original command/key/save admission,
durable Leave and cold operational ownership. Joined-member sources, durable
governing archive/transport and combined acceptance also remain open. Chat V2 stays off.
