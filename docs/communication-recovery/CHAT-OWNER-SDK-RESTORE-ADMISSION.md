# Private SDK restore admission: exact closed profile

Design for review, not an implemented heap bound. Compilation/tests/builds are paused. The
independent malformed-read patch returns storage errors; it does not implement this proposal.
Nothing here activates typed Chat V2, changes legacy restore grammar, or grants owner authority.

## Actual resolved source

Both primary workspaces resolve **openmls0.9.2**, archive checksum
`ce4173f914cbc2dd28d9499f19e92f3e1bfebd9eea1607889924a77ca63d3ffc`:
primary root `Cargo.lock:4055`, native `apps/desktop/src-tauri/Cargo.lock:5323` at inspection.
The membership/factory locks resolve the same version. `catcoms-mls/Cargo.toml` requires0.9.2;
the checked-in0.8.1 snapshots are compatibility inputs, not the resolved SDK. Related pinned
implementations inspected are memory_storage0.6.0, basic_credential0.6.0, tls_codec0.5.0,
serde/serde_core1.0.228, serde_json1.0.150 and repository toolchain1.94.0.

Actual narrow path:

1. App original charged canonical source -> Sync closed owner checkpoint restore.
2. `persist::restore_server_provider_bounded`: borrowed map counts/framing/backing proof,
   then one owned provider map. Its bytes/backing fees already exist.
3. `restore_device_and_group` -> `MlsDevice::restore` -> `SignatureKeyPair::read` -> typed JSON.
4. `ServerGroup::load` -> SDK `MlsGroup::load` (`group/mls_group/mod.rs:500`).
5. SDK `PublicGroup::load` (`group/public_group/mod.rs:531`) loads Tree, queued proposals,
   GroupContext, InterimTranscriptHash and ConfirmationTag; MlsGroup then loads EpochSecrets,
   OwnLeafNodeIndex, MessageSecrets, ResumptionPsk, join config, own-leaf list and GroupState.
6. Strict local device/own-leaf and independently pinned fact checks run afterward. They also
   clone bounded public member keys/credentials while iterating the roster.

The SDK load itself does not restore by rebuilding its tree through public tree constructors.
`TreeSync.tree` is an `ABinaryTree` with inline leaf/parent vectors and blank defaults. Boxed
TreeNode values belong to construction/diffs; treating all persisted nodes as pointer-sized
boxes would underpay the real load.

## Allocations missing from the current provider invoice

| Actual retained/temporary object | Current enforcement gap | Proposed closed handling |
| --- | --- | --- |
| Signer private/public Vec; private Vec -> SecretVLBytes copy | Provider raw bytes do not pay the decoded key or overlapping copy | Exact Ed25519 lengths32/32, prepay32-byte extra private copy before getter |
| Device Basic credential and SignaturePublicKey | Constructed after signer decode | Fixed32-byte copies, separate retained debit |
| Typed provider-key generation | GroupId/signature StorageId JSON and prefixed key Vecs coexist | Audit actual fixed-key builders; prepaid checked key scratch before SDK entry |
| Tree leaf/parent vectors | Private inline element sizes, including empty slots, exceed byte-token size | Count bounded slots; debit actual generic element layout before first Vec element |
| Leaf/parent keys, signatures, credentials, capabilities, unmerged lists | Many nested vectors and spare growth capacity | Closed field-specific shape/count/length validation and per-container debit |
| SecretTree four vectors | Even `null` ratchet slots own private enum-sized inline storage | Debit `size_of::<Seed::Value>()` for actual nullable slot type, not pointer/JSON bytes |
| Secret bytes and serialized GroupContext | Additional typed copies of provider JSON | Exact ciphersuite secret/key lengths and bounded serialized-context length |
| Sender ratchets/old epoch trees/resumption entries | Nested VecDeque/ratchet/epoch graphs | Initial profile refuses nonempty ratchets/past epochs; resumption count separately bounded |
| Own leaf list/proposal store/PendingCommit | Nested inner JSON, references, maps and boxed staged graph | Initial profile admits only empty lists and Operational; preserve/refuse other sources |
| Optional draft/PPRF/unknown rows read during load | Hash maps, custom byte copies and feature-dependent constructors | No legacy fallback under a private policy; an unsupported getter/shape refuses |
| Member iterator/credential checks | SDK `members()` clones keys/credential per visited member | Separate bounded maximum live member scratch, paid before those checks |

No row is truncated, stripped, compacted or rewritten to fit the closed profile. Unloaded raw
provider records remain covered by existing raw map/backing fees; this proposal does not imply
that later operation-specific SDK reads of those records are admitted.

## Proposed narrow implementation; no SDK fork needed

Use the already locally patched memory_storage, with a new explicitly installed private restore
policy. Default providers and ordinary legacy restore retain their existing JSON decoder. Only
the actual original private owner restore selects the new policy, using its captured configured
limits and a real retained resource reservation. No wire/native scalar or saved success bit
constructs that reservation.

The policy has two gates. First, a complete borrowed closed-shape scan of all rows that the
actual load can reach must finish before the first SDK typed decoder is entered. Second, each
actual typed read repeats its row/shape check under the SAME provider read guard, then uses a
counted borrowed JSON Deserializer. The source input and quota cannot change between validation
and decode. Generic JSON validity or a token count alone is not this gate.

The cursor holds borrowed slices, fixed counters and a fixed seen-field bitset. It has explicit
depth, bytes visited, array elements, object fields, string length and numeric bounds; it never
builds a `serde_json::Value`, owned AST, decoded String, vector of children or pending graph.
Objects accept fields in any order but reject duplicates/unknown fields. The initial profile
accepts only the ASCII field/enum spellings emitted by the audited SDK, integer scalars and byte
arrays; escaped/unknown strings, tagged legacy policy alternatives and unsupported drafts refuse.
Opaque user credentials remain byte arrays, never borrowed JSON strings. Empty/missing optional
lists keep their existing meaning, while malformed present lists still error.

The decoder implements the relevant Serde primitive, newtype, Option, sequence, struct-map and
externally tagged enum entry points over those same slices. Scalar/string keys are borrowed;
unsupported entry points return the existing serialization error. It must not silently delegate
an unknown field, enum, visitor or row to the unrestricted legacy decoder.

### Why actual private slot layouts are available

On the first call to `SeqAccess::next_element_seed<S>`, the adapter can compute the actual
`size_of::<S::Value>()` and alignment, even for SDK-private `Option<SenderRatchet>` and tree-node
types. This is useful only because the accepted visitors are individually audited. A generic
DeserializeSeed does not promise any allocation behavior and is not a general heap meter.

For the accepted serde_core1.0.228 `Vec<T>`/`VecDeque<T>` visitors, the adapter returns size hint0.
Those visitors begin with capacity0 and request elements before their first push allocation.
Before decoding the first element, the adapter knows the exact borrowed array count and must
debit its complete checked typed growth allowance. Rejecting at that point precedes child decode
and outer container allocation. Tuple arrays are distinguished by the closed schema and need no
outer Vec charge. Empty arrays allocate no container backing. No deserialize-in-place path is used.

For pinned Rust1.94, start nonzero capacity at8 elements for one-byte elements,4 through1024-byte
elements, otherwise1; double until covering the admitted count. Debit the sum of every requested
allocation layout, not just logical length or final capacity. This conservatively pays simultaneous
old/new buffers. ZSTs need no backing but still consume element/work credits. Reject checked
overflow/isize limits before the visitor. This is per actual typed container, not a multiplier of
encoded input. Source proof: [RawVec1.94](https://github.com/rust-lang/rust/blob/1.94.0/library/alloc/src/raw_vec/mod.rs)
and [VecDeque1.94](https://github.com/rust-lang/rust/blob/1.94.0/library/alloc/src/collections/vec_deque/mod.rs).
Allocator metadata/RSS rounding is outside a requested-layout policy and must not be called a
whole-process physical-memory limit.

Every accepted custom Deserialize path needs an explicit entry in the audit before it is enabled.
The basic credential secret helper creates a second byte allocation and needs its separate debit.
PastEpochDeletionPolicy's untagged parser is admitted only for its bounded integer spelling, whose
Serde Content is scalar. SenderRatchet's conversion and draft PPRF/byte-copy paths are avoided by
the initial all-empty-ratchet policy. A new custom visitor/Box/map or dependency change reopens
review; `size_of` alone cannot justify it.

### Initial resource and shape contract

Use an explicit `OwnerSdkRestoreLimits`/reservation with maximum requested typed bytes, lexical
byte visits, supported tree slots, credential bytes and fixed key/member scratch. These are
negative policy caps from the original immutable source budget, not semantic witnesses. Fully
prepay the configured typed cap before SDK entry. A monotonic per-attempt counter debits actual
typed container growth plus audited custom copies before allocation; it never refunds capacity
on a partial/error decode. A refusal drops the private partial graph and preserves the source.

The first closed profile is the existing no-history initializer / separately implemented first
Add snapshot only: Operational, blank tree defaults, internally consistent leaf/parent counts and
indices, exact32-byte keys/secrets and64-byte Ed25519 signatures, bounded Basic credentials and
capability arrays, no nonempty queued/own-leaf lists, no past epoch trees and no initialized sender
ratchets. GroupContext extensions/unsupported drafts refuse. Resumption entries/count and all
scalar configuration fields receive explicit bounds. A proposed64-slot default would cover the
requested50-member groups; the actual initial profile must be pinned against genuine founder and
selected-KP/first-Add outputs before choosing that default. No unsupported snapshot is shortened.

All lexical work is debited before character/element traversal, including repeated slice scans.
Do not derive work credit from a deadline or ambient clock. The closed depth and explicit work
cap bound rescans; signature/own-leaf checks have separately bounded counts/scratch. Input byte
limits alone do not establish this work bound.

## Custody, overlap and integration

The new typed cap must be acquired from the SAME original resource pool before SDK entry and
remain retained while the decoded private SDK graph exists. `PrivateManagedRestoredServer` drops
its Server before releasing that fee. On success the retained SDK fee moves with the actual
OwnedOwnerServer; it cannot disappear with the source's transient read fee at take_ready. A later
active host must convert/retain it under its own global admission before any Actor/run handoff.

The existing provider allowance still covers raw backing/payload during private overlap only.
Keeping one typed reservation does not solve active provider/whole-Server memory, multiple
retained old mounts, or whole-process RSS. These remain explicit active-host gates, not reasons
to release a decoded graph's accounting early or silently reset execution allowance on retry.

The policy/guard must survive moving OpenMlsRustCrypto into MlsDevice without self-borrows. An
owned shared attempt state can carry Active/Complete/Cancelled; dropping an uncompleted guard
must cancel its policy rather than resume legacy reads. Finishing the restore does not authorize
future mutative SDK work; that needs its own operation/resource admission. No async callbacks or
new Store lock order belong in this synchronous private decode.

Expected affected implementation: memory_storage policy/cursor/Serde adapter modules; a new MLS
owner-only restore entry; Sync's narrow strategy/negative limits; App's actual original-source
reservation/private-owner fee transfer. Authority's operation tail/head/successor/Add files remain
separate and must agree exact API before integration. No change to normal `restore_server` or
generic graph restore, no new private-key export and no SDK source vendoring is proposed now.

## Required evidence before any completion claim

- Root/native version/feature verification; exact accepted visitor/custom-copy audit.
- Borrowed scanner count/depth/duplicate/unknown/malformed bombs before SDK decoder entry.
- Real private-layout slot tests, including all-null ratchets whose inline elements exceed a
  pointer; zero/one/exact cap/overflow typed Vec and VecDeque capacity/request sums.
- Actual allocated-capacity observations for accepted private SDK output where practical; this
  supplements, rather than replaces, the before-allocation source proof.
- Each custom copy/temporary key/member overlap, failure partway through decode, quota retained
  until graph drop and transfer without a free accounting gap.
- Genuine founder, actual selected-KP member and real covering first-Add source success; preserved
  unsupported/pending/history/draft inputs and unchanged ordinary legacy/frozen compatibility.
- Actual complete private consumer with tight original caps; no helper-only or literal-paid test.
- Read-only design + actual-diff review and the primary's deferred focused/full/native gates.

Until those paths and checks exist, the SDK heap/work gate remains OPEN.
