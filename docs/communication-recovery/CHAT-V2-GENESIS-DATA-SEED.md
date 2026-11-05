# Closed V2 genesis DATA seed

Status: design review CLEAR; implementation and13 focused regressions authored, all UNRUN.
Final independent actual-diff STATIC review CLEAR, including the13th cancellation test.
No blocker/high/medium remains in this boundary. No compiler, tests or build have run. This is a prerequisite of `CHAT-OWNER-V2-GENESIS-CURRENT.md`, not a
Current, runnable scheduler, live operation admission or production activation.

## Actual input and public genesis

The builder borrows the actual opaque `OpenedBootstrapSourceData`. It does not take a
dbkey, caller-supplied S0, raw expected Profile, success boolean or source callback.
The existing opener already checks the complete source frame/id, AEAD, expected full
context, exact profile-bound S0 digest, complete closed S0, signed public Profile and
the real sealed genesis records/manifest. Extend its PRIVATE result with:

* the exact whole `BootstrapSourceId` already checked;
* owned `SignedOwnerProfile`, `AdmissionBody`, `SignedOwnerHeader`, the EXACT canonical
  state image from `HeaderRow`, and the fixed empty acceptance `Frontier`;
* no dbkey, live source/Store owner, Fresh lifetime, Saved receipt or Current permit.

`validate_genesis` returns this private verified DATA while the actual record plaintext
is still borrowed. It must not synthesize another signed representation or re-encode
the state image. The retained image is bounded by `MAX_STATE_BYTES`; each of the three
public typed objects has one bounded group allocation. `RecordBatch` still verifies
all representatives and exact founder/header/state/body binding before retention.

The public `BootstrapSourceDataView` remains a borrowed raw DATA view. An additional
source-id getter is DATA only. No new public constructor of the opened result exists.

## Exact Sync API

Proposed exports under `catcoms_sync::owner_v2`:

```text
V2GenesisSeedLimits::new(max_snapshot_bytes, max_mls_bytes, max_provider_backing_bytes)
    -> Result<V2GenesisSeedLimits, SyncError>

prepare_v2_genesis_seed(
    source: &OpenedBootstrapSourceData,
    limits: V2GenesisSeedLimits,
    sdk: OwnerSdkRestoreAdmission,
) -> Result<UnboundV2GenesisSeed, SyncError>

seed.data() -> V2GenesisSeedData<'_>
seed.prepare_scope(&mut self, original: &impl ManagedChatScopeReservation)
    -> Result<(), SyncError>
seed.prepare_runtime<T: MeshTransport, R: CryptoRngCore>(
    self, parts: V2GenesisRuntimeParts<T, R>
) -> Result<PreparedUnboundV2GenesisNode<T, R>, ReturnedV2GenesisAttach<T, R>>
returned.into_originals() -> (UnboundV2GenesisSeed, V2GenesisRuntimeParts<T, R>, SyncError)
prepared.into_node() -> UnboundV2GenesisNode<T, R>

FounderBootstrapPreparation::into_genesis_runtime_parts<T, R>(
    self, original: ChannelSync<T, R>, expected_id: BootstrapSourceId
) -> Result<V2GenesisRuntimeParts<T, R>, V2GenesisRuntimeRefusal<T, R>>
refusal.into_originals() -> (FounderBootstrapPreparation, ChannelSync<T, R>)

V2GenesisRuntimeParts::<T,R>::allocation_bytes() -> usize
V2GenesisRuntimeParts::<T,R>::warm_refusal_allocation_bytes() -> usize
V2GenesisRuntimeParts::cold_data(
    transport: Arc<T>, rng: R, clock: Arc<dyn Clock + Send>
) -> V2GenesisRuntimeParts<T, R>
node.data() -> V2GenesisSeedData<'_>
```

Limits are negative caps, NOT a paid resource/source permit. Their constructor reuses
the closed `FounderSnapshotLimits` grammar (MLS/backing <=8MiB, whole S0 <=8MiB+64KiB).
All three configured allowances must cover the actual source's stored allowances; the inner
loader still uses those ORIGINAL source negative caps so a larger host limit cannot widen them:
the builder never derives payment from decrypted limits. SDK admission is the real
non-Clone host-paid graph admission, and its SAME internal Arc stays LAST in BOTH
actual MLS device/group. A second independent seed restore requires another payment.

`V2GenesisSeedData` exposes ONLY source/context labels, borrowed verified public genesis,
empty acceptance tip, metadata counts and original scope-label DATA. No SDK, device,
secret routing, raw mutable graph, `ChannelSync`, Fresh, Server, Current, run/author or
positive projection/proof token is exported. A caller-attached runtime remains DATA.

## Complete closed S0 and scaffold

Refactor existing `bootstrap_source::s0::preflight` into a shared borrowed complete
parser returning private `Fields`. Parse ALL framing/counts/UTF-8 and EOF before the
new builder copies metadata or enters the provider/SDK loader. Unknown/trailing/v1
owner/operation tails remain unsupported, never ignored. The existing opener uses
the same grammar. Nested provider row/JSON/type validation remains the real bounded
MLS loader's job; no duplicate unbounded serde path is introduced.

Fields and their retained representation:

| Original field | Closed validation / owned scaffold |
| --- | --- |
| MLS | borrowed full frame; actual closed loader receives it once |
| routing | label + <=8 strictly increasing slots, each fixed32 secret; one exact-cap Vec of `(u64, Zeroizing<[u8;32]>)` |
| invite ledger | both sets zero; private closed-empty marker, no legacy ledger/scheduler |
| documents/commits/peer descriptors | each count zero; same closed-empty marker |
| file wrap key | exactly32, retained `Zeroizing<[u8;32]>` |
| admin generation/obsolete grants | generation0/count0 retained as closed-empty invariant |
| rendezvous | <=8 rows, address UTF-8 <=1024 bytes and opaque value <=128; one exact-cap Vec plus exact String/Vec copies |
| direct results | count0, no cache recreated |
| tenure | exact v2 epoch0/owner/start0/leaf-index0/nonimported; `OwnerTenure::decode` against the ACTUAL restored group |
| policy | complete <=4101 frame, optional P2P policy; supported mode + actual `verify_pin(group)` |
| durable chat | version1/2 and count0; retain version DATA and closed-empty invariant |
| pending finalization | exact supported version + count0; retain version DATA and closed-empty invariant |
| admin authority | exact v2 tag/count0 |
| local scope | Active, original group/full signer/domain/nonce, no aliases; actual `LocalChatScope::decode`, no runtime/publication binding |

This is a non-generic private `GenesisMetadata` scaffold, not merely an MLS pair and not
a partially initialized legacy `ChannelSync`. Empty inventories have no allocated maps
to populate later implicitly. A future actual V2 governor must explicitly introduce its
own scheduler/queues and operation admission; no old dispatch/start API accepts this node.

After SDK restore require epoch0/member-count1/active/full local signer/designated owner,
actual group bytes equal the source context and founder. Actual tenure decode checks
its leaf digest against that graph; policy verifies its actual pin. Recheck the retained
public image with `StateFrame::verify_genesis_binding` and epoch0/one founder admission.
The public state format contains MLS EPOCH, not an epoch-authenticator field: DO NOT
invent a signed authenticator comparison. The authenticated exact original sealed S0
and SourceId provide the private snapshot binding. No remote root provenance follows.

## Scope and runtime ownership

`prepare_scope` captures the trusted host reservation fields ONCE. Before attaching,
require the original group/full signer/domain/nonce AND planned durable revision equal
the source context, same original observer live, and actual closed MLS membership. It
attaches receiver-only reservation state. No publication authority getter is called and
no watch sender/runtime is manufactured. Repeat attachment accepts ONLY the SAME
identity and planned revision while its original observer remains live. A replacement
identity never renews it. Arbitrary Rust trait implementations remain trusted host code,
not authenticated disk evidence. Root's actual App consumer must repeat sourceSaved,
mount/row/pin/head checks and source cancellation at final admission.

Warm extraction consumes the actual `FounderBootstrapPreparation` and original sync.
Before taking ANY field, require its exact transferred SourceId/no prepared candidate,
actual private source checks and complete actual context equality. On refusal return
BOTH SAME original objects; no RNG/transport loss, regeneration, rewind or clone.
On success destructure the original sync and move its actual `Arc<T>`, SAME R,
`Arc<dyn Clock + Send>` and private `VerifiedFreshChatAdmission` (including its original
FreshOriginLife sender) into opaque parts. Drop the original SDK/source/caches OUTSIDE
host writers; the Root consumer must invoke extraction outside all those writers.
The Root's separate original RAM-plan sender stays owned by its actual Current wrapper.

`cold_data` accepts already allocated Arc inputs and R, with NO Fresh-life field. It
cannot recreate historical Fresh from decoded attempt labels. Root's new paid cold
capture/current host is a separate authorization boundary; join life remains separate.
Before final attachment, `prepare_runtime` compares a warm part's PRIVATE exact SourceId/full
context, original Fresh observation against the restored graph (including the private original
authenticator) through a new private no-allocation observation comparison, and original managed reservation identity/planned revision/negative against the
seed's receiver-only attachment. It never refreshes the original observer. The new scope's
process-local revision is deliberately0: compare original reservation identity and durable
planned revision, not the old sync's process-local revision. Both scopes must remain private
(no runtime/aliases/recovery). Cold parts have no historical Fresh/observation; they still
require a live attached reservation, and only Root's actual new capture can authorize Current.
The existing generic Fresh comparison allocates `group_id()`/`public_key_bytes()` Vecs,
so pairing does NOT call it. Its new private comparison uses the actual already-validated
fixed signer, `group_id_ref()`, own-leaf/full-signer/active checks, private authenticator
and SAME weak identity/negative. It cannot mint Fresh or replace a life.
Wrong pairing returns SAME seed and SAME parts plus fixed error, with no allocation/callback.
Seed and runtime-parts have preallocated private Box bodies, so this owning error does not
need a new large allocation; runtime-parts Box layout depends on host T/R and is explicitly
outside the non-generic metadata invoice. The public negative `allocation_bytes` and
`warm_refusal_allocation_bytes` expose actual pinned generic Box layouts (and the bounded warm
context group), allowing the real host to reserve them before entry. They issue no fee. Warm runtime refusal can box the original tuple
before returning (outside writers), without replacing or cloning either original.
Final `prepared.into_node` has no callback, validation, fallible allocation, await, Arc creation
or replacement of T/R/Clock; it performs field moves only. The unbound node retains all
private metadata/genesis/SDK and runtime parts, exposes only DATA and has no scheduler.

## Named resource inventory and destruction

Keep SDK graph fee independent of raw provider table/payload, whole source AEAD buffers,
old live graph/S0, source debt/file copies and new Sync metadata. The actual opener's
`BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE` must explicitly add the retained verified objects
and image, plus the extra clone live during seed construction; use named sizes and a
const assertion rather than assume the old256KiB covers them.

New `V2_GENESIS_SEED_METADATA_BYTES` enumerates (logical requested allocations, not RSS):
exact8 routing slots; exact8 rendezvous Vec slots +8*(1024+128) payload; bounded optional
policy/group copies; exact retained public image + three public group copies; exact context
group; local scope/tenure/device-id/leaf/signature/public verification scratch; inline
`GenesisMetadata`, verified genesis, seed/data/runtime-parts fixed layouts. Vec capacities
are requested once from bounded counts, with no push growth past checked capacity. Retain
zeroizing owners for secret metadata. No resource fee callback or Store writer is held
inside provider/SDK getters. Constructor errors drop partial metadata/graphs outside
host writers; the SDK loader keeps its local strong fee until partial getters drain.
Only the SDK graph fee is retained by this constructor API. Caller must reserve the actual
named source/provider/metadata fees BEFORE entry AND its actual Root wrapper must keep those
fees through seed/node ownership until the corresponding bytes/graphs drain; acquire-and-drop
before return is insufficient. Neither
constant nor negative caps are a process-global/Native accounting claim.

Runtime T/R/Clock arbitrary host layouts and their Arc allocations, source old graph and
App-owned original source/plan fee are separately supplied/paid; this packet cannot bound
arbitrary Rust implementations. Last SDK lease remains in actual device/group regardless
of seed/node Drop ordering. Warm extraction must not drop the new paid seed while holding
the canonical pool. Parent and child restores continue to require TWO independent fees.

## Focused authored regressions (UNRUN)

Real fresh source -> existing sealed opener -> seed succeeds; exact Profile/Admission/
Header/image/source/context/Frontier and all nonempty allowed metadata match. Bad late
S0 field/trailing v1 tail/context/profile/source digest refuses before SDK load; malformed
provider refuses without changing opened source; epoch1/second member refuses initial
genesis even if supported by the two-member SDK shape. Limits one byte short refuse
before SDK; distinct SDK payments survive seed->node and independent half cleanup.

Scope wrong group/signer/nonce/domain/revision/replacement identity/dead observer refuses;
same original repeats without reading a publication authority. Cold parts never create
Fresh. Warm success preserves actual Arc<T>/ArcClock identity, exact RNG state and original
Fresh observer; refusal returns original source+sync unchanged; cancellation after source
transfer prevents extraction. Wrong-source warm pairing returns the same seed/parts; final prepared attach uses field moves only and sends no transport
event, samples no clock/RNG and invokes no host callback. No real App Current/pin/head,
live governor/SDK operations or physical/native activation is claimed by these tests.


## Current source and verification

The connected implementation is `owner_v2/founder/bootstrap_source/genesis_seed.rs` and
its public exports in `owner_v2`. The existing source opener now retains the actual verified
public objects/image and whole SourceId. Shared `s0::fields` fully scans the closed source
before this seed loader enters SDK. Scope and Fresh changes are new narrowly named private
negative helpers; existing legacy comparison/restore behavior is unchanged.

Thirteen new focused regressions are authored in `genesis_seed/tests.rs`, including a genuine
SDK first-Add graph that the paid loader supports but the genesis builder refuses. Source
rewriting in that regression explicitly chooses a new expected DATA context/id; it does not
claim to possess an actual Root SourceSaved capture. Existing malformed sealed-source tests
remain intact. Direct rustfmt of10 Rust files, whitespace, ambient-dependency and Store raw-fs checks passed.
Design and final actual-diff STATIC reviews are CLEAR; these are not compiled/runtime evidence. All13 new tests are UNRUN, and normal
production-config compilation, full suites and native validation remain deferred under the
user pause. No Current/governor/native/live-operation or complete-host claim is made.
