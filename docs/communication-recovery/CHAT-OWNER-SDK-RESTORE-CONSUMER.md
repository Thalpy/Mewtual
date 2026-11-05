# Closed initial SDK restore consumer

The opt-in consumer is authored; design and final actual-diff static review are clear.
The initial checkpoint introduced25 regressions (9vendor,7MLS,9App). The four additional
resumption-store regressions below are authored UNRUN; the primary now owns resumed serial
compiled validation. Direct formatting/source review is not compilation or runtime evidence.
This narrows the larger SDK admission draft to the pinned OpenMLS0.9.2 founder/first-Add graph.
Ordinary legacy decoding, serialized bytes and unsupported retained sources stay unchanged.
It does not complete live text Chat V2 or native activation.

## Exact source and load surface

Both root/native locks resolve OpenMLS0.9.2, MemoryStorage0.6.0 (local patch),
serde/serde_core1.0.228, serde_json1.0.150 and tls_codec0.5.0. Rust1.94.0 is pinned.
The SDK load calls PublicGroup::load, then seven group getters; the device SIGNER getter
runs first. Thirteen logical rows are therefore required/observed:

| Row label | Closed accepted data | Actual getter |
| --- | --- | --- |
| SignatureKeyPair | private32/public32/ED25519 | direct signature_key_pair |
| Tree | one/two inline leaf slots; zero/one parent slots; blank defaults; hash32 | direct tree |
| ProposalQueueRefs | absent or empty JSON list | generic read_list, no companion proposal read |
| GroupContext | MLS1.0/suite3/group16/epoch0 or1/tree-hash32/confirmed-hash empty-or32/empty extensions | direct group_context |
| InterimTranscriptHash | byte vector32 | direct hash |
| ConfirmationTag | mac_value.vec32 | direct tag |
| EpochSecrets | five exact secret wrappers32 | generic read |
| OwnLeafNodeIndex | index within actual leaf dimension | generic read |
| MessageSecrets | zero past epochs, bounded serialized context, exact secrets, matching tree dimensions, all sender slots null | generic read |
| ResumptionPsk | max0/empty/cursor0 OR max32 with exact epoch0..current prefix and cursor=count, at most2 entries | generic read |
| MlsGroupJoinConfig | ciphertext-only/padding0/past0/resumption0/tree-extension=true/ratchet5 and1000 | generic read |
| OwnLeafNodes | absent or empty JSON list | generic read_list |
| GroupState | Operational only | generic read |

Tree leaf payloads have X25519/signature keys32, signature64, Basic credentials at most512
bytes (includes actual device-bound invite credential), pinned one-version/one-suite/Basic
capabilities and empty extension/proposal lists. KeyPackage lifetime scalar bounds are structural,
not a new expiry recheck on retained historical input; Commit parent hashes are empty or32.
Parent nodes have key32, empty-or32 parent hash and bounded unmerged indices. Empty default
leaf/parent are required. Every dimension/index is cross-checked before SDK entry. Public ABinaryTree has N leaf
slots and N-1 parent slots; SecretTree has N leaf, N PARENT, N handshake and N application
slots (the SDK intentionally allocates leaf_count parent slots). Its size is 2*N-1 and its
own index must equal the independently loaded OwnLeafNodeIndex and fall within N. The actual
SDK structs, not the0.8.1 fixture, define fields. MessageSecrets may contain added_at null or
an exact secs_since_epoch/nanos_since_epoch pair; no ambient time is read.

The actual OpenMLS0.9.2 founder builder creates `ResumptionPskStore::new(32)` and adds the
original epoch0 secret even though `MlsGroupJoinConfig.number_of_resumption_psks` is0. The
owner's first Add retains epoch0 and adds epoch1; the selected Welcome member's configured0
store is actually empty. A zero join-config scalar must therefore never justify an empty-only
restore policy or deleting the original founder secrets. The closed nonempty case requires
maximum32, count=current epoch+1, cursor=count, and ordered tuples for exactly epochs0 then1
when present. Each tuple contains one exact `SecretWrap.secret.value.vec` of32 bytes. Count is
bounded before reading a third tuple; every repeated row check resets its fixed two epoch
slots, count, maximum and cursor before recomputing them. Duplicate/out-of-order/gap/future
epochs, missing entries, mismatched cursor, unknown maximum and malformed tuple/wrapper/byte
shapes refuse before SIGNER. This narrow correction preserves the genuine original bytes;
it does not admit later epochs or initialized sender ratchets. The maximum32 scalar does not
reserve32 tuple slots: the ordinary typed Vec has at most2 entries, its actual growth and
secret-vector copies remain charged by the counted decoder, and the graph cap stays128KiB.

The closed scanner refuses duplicate/unknown/missing fields, escapes/non-ASCII identifiers,
negative/floating numbers, unsupported variants, trailing input, excessive depth/count/length
and unreviewed optional draft rows. It uses borrowed slices/fixed counters only. Numeric zero
for past_epoch_deletion_policy (or its legacy alias) is the ONLY accepted shape: the pinned
SDK's untagged Format Content capture therefore has no allocating branch. No serde_json::Value
or AST is constructed. Unknown provider rows may remain raw, because load never reads them;
any typed getter outside this exact allowed row/key set refuses under the installed policy.

The audited field paths are SDK `group/mls_group/mod.rs::MlsGroup::load`,
`group/public_group/mod.rs::PublicGroup::load`, `binary_tree/array_representation/tree.rs::ABinaryTree`,
`treesync/treesync_node.rs`, `tree/secret_tree.rs::SecretTree`,
`group/mls_group/past_secrets.rs::MessageSecretsStore`, `schedule/message_secrets.rs`,
`schedule/psk.rs::store::ResumptionPskStore`, and `group/mls_group/config.rs`.
Custom-copy paths are `openmls_basic_credential-0.6.0/src/lib.rs::secret_bytes_as_vec`
and `openmls_serialization_helpers-0.1.0/src/deserialize.rs::VariantIdVisitor`.
`tls_codec-0.5.0/src/quic_vec.rs` derives Serde for VLBytes and transparent SecretVLBytes;
it does not add a second byte copy in the admitted Secret.value wrapper. The Basic signer
helper DOES make an additional secret copy and has a separate debit.

Both locks were re-read with tomllib at this checkpoint; OpenMLS checksum is
`ce4173f914cbc2dd28d9499f19e92f3e1bfebd9eea1607889924a77ca63d3ffc`,
serde_core `41d385c7d4ca58e59fc732af25c3983b67ac852c1a25000afe1175de458b67ad`,
serde_json `e8014e44b4736ed0538adeecded0fce2a272f22dc9578a7eb6b2d9993c74cfb9`.
No package or lock update is proposed. Optional-feature typed getters must be guarded even
though the initial profile refuses their rows. Existing0.8.1 compatibility fixtures remain
inputs for the ordinary decoder, not evidence for the new0.9.2 shape.

## One installed policy, not a fallback

MemoryStorage gains a private optional shared restore attempt. Default None is the unchanged
legacy path. The new owner loader installs the policy BEFORE MlsDevice::restore and its signer
key construction/getter. Installation preflights every reachable row under a values read guard,
then each actual getter repeats the row preflight and counted decode under its SAME read guard.
The policy is owned state, not a borrowed guard into a provider that will move into MlsDevice.
An exact allowed-key table uses thirteen fixed512-byte stack/inline slots, source group16 and
full signer32. A fixed canonical JSON writer constructs those keys without a Vec or generic
serializer. The signer key includes the actual Basic StorageId suffix public32 +
RustCryptoSignatureKey22 + scheme bytes8,7. Installation performs thirteen exact map lookups
and complete row preflights under one values guard before any typed getter. Unloaded raw crypto
rows remain inert data; they are not silently typed. Before each actual lookup the policy checks
Active/exact key/read-once and pays bounded hash/probe work from actual table allocation metadata;
the checked worst-case debit is actual_table_allocation_size ? (512 comparison bytes + one
fixed probe unit), plus512 bytes of key-hash work, before map.get. Allocation size overpays
the number of possible candidate slots; this explicitly pays full equal-length candidate-key
comparisons, not only empty/control probes. Overflow or insufficient work refuses before lookup.
No absent row bypasses Complete/Cancelled refusal. Empty absent list rows are observed as empty
under the same guard. All fixed policy/key slots are invoiced by actual layout before allocation.
It records Active/Complete/Cancelled; an incomplete guard Drop cancels without reopening the
legacy branch. Complete/Cancelled reads and all SDK writes refuse; a later SDK operation needs
its own explicit admission. Optional-feature getters also refuse, rather than bypass the policy.
Raw snapshot observation remains raw data. Test-utils provider clones retain the SAME state.

A local monotonic account pays named heap requests/work; no pool/admission callback, I/O or
await occurs while SDK storage is locked. The scanner charges lexical work before each character
and every sequence/field probe; decoder traversal and count rescans are charged again. Overflow,
poison and exhaustion refuse, never refund or wrap. Errors use fixed existing storage errors.

The counted deserializer implements ONLY audited scalar/newtype/struct/option/enum/tuple/sequence
visitors. It borrows field/enum strings; every string identifier passed to a variant seed is
precharged for its possible actual VariantId::Name.to_string copy. String/bytes owned requests
are refused unless explicitly audited. Sequences report size_hint=0. At the FIRST actual
next_element_seed, the complete bounded sequence length and size_of::<S::Value>() are known;
the full Vec/VecDeque growth-request sum is debited BEFORE decoding that element or allowing
its container's first push. Empty containers allocate nothing. Tuples/arrays are inline, so
no Vec debit is inferred for them. This is valid only for the listed actual pinned visitors,
not for arbitrary DeserializeSeed implementations. Nonempty zero-sized/in-place seeds refuse
before their child can run; empty containers remain free. The growth-invoice helper separately
checks zero-sized backing/work arithmetic.

serde_core1.0.228 Vec and VecDeque create with_capacity(0), then next_element before push.
Rust1.94.0 RawVec uses minimum capacity8 for one-byte elements,4 for elements<=1024 bytes,
otherwise1, then checked doubling. Debit the SUM of requested capacities times actual element
size, covering old/new overlap without assuming realloc is in place. Source references:
<https://raw.githubusercontent.com/rust-lang/rust/1.94.0/library/alloc/src/raw_vec/mod.rs>
and the exact serde_core/src/de/impls.rs visitors. Allocator bookkeeping/excess/RSS remain
outside this named requested-buffer contract; no generic JSON-byte multiplier is used.

Before the SDK call, prepay the exact bounded key-builder Vec growth/copies (including
Basic StorageId public32+RustCryptoSignatureKey22+scheme2), signer private32 extra
Vec-to-SecretVLBytes copy, device Basic/public-key copies, GroupId16, policy state/Arc metadata
the fixed CatalogCache Arc header, and strict borrowed local own-leaf checks. The closed loader
maps storage failures to fixed errors rather than allocating legacy formatted diagnostic Strings.
Legacy diagnostic/backtrace/hex logging is suppressed whenever a policy is installed; normal
legacy diagnostics remain unchanged. All write wrappers repeat exclusion under the actual values
writer after the early pre-serialization check, so a waiting legacy writer cannot overtake install. The new strict loader can use borrowed own_leaf checks rather
than materializing members() just to establish local identity. Later Sync validation, catalog
and repeated member observations have separate named bounded scratch obligations; this packet
must not describe the entire Server graph or arbitrary later API as paid.

## Actual host reservation and graph lifetime

The actual MLS API is an explicit trusted-host resource boundary, not authentication:

- public negative `OwnerSdkRestoreLimits { max_typed_bytes, max_work }`;
- documented `OwnerSdkGraphReservation: Send + Sync + 'static` supplies those immutable limits;
  arbitrary Rust implementations are trusted host accounting, not cryptographic payment proof;
- non-Clone `OwnerSdkRestoreAdmission::new(reservation)` caches caps ONCE and pays its exact Arc
  layout before allocation; `restore_server_owner_checkpoint_bounded(..., admission)` performs
  no host callback under SDK locks;
- one internal Arc holds that non-Clone reservation as its last resource and is held in BOTH
  actual MlsDevice and ServerGroup as their last field. Either can outlive the other after tuple
  return, so one-holder accounting is insufficient. Default constructors have no lease;
- TWO independently acquired reservations are required for parent and child overlap; cloning
  the same Arc is not a new admission. Graphs/provider/signers drop BEFORE their fee.

The actual repository issuer is private App canonical accounting, not a test bool. An explicit
`OwnerEvidenceBudget::with_closed_sdk_restore(bytes, work)` local opt-in uses the SAME original
canonical shared hot pool. Original source validates mount/pin first, then acquires one real
strong-pool RAII debit per simultaneous graph BEFORE decode. Caps are queried exactly once before any SDK lock. The fee owns only pool+immutable caps,
never Store/Server/provider/keys/source ownership; decode runs outside host writers and a local
strong reservation survives partial getter/graph cleanup before pool refund. Strong pool contains metadata,
not vault keys/session; no callback to it is made under SDK locks. Fees move with the real SDK
objects through PrivateManagedRestoredServer/take_ready/OwnedOwnerServer and cannot disappear
with the source read buffer. Failure drops partially decoded graph before fee. Underflow closes
future SDK admissions permanently. The default checkpoint without this opt-in keeps its old
explicitly-unbounded typed decoder compatibility; no native activation is enabled.

Sync receives the non-Clone resource separately from Copy negative snapshot limits, adds a
private strategy/new explicit owner restore entry, and uses an independently acquired lease
for validate_owner_operation_parent. The existing membership-candidate Add is NOT authorized
by a completed load. Restored-device duplication/KP generation and mutative group entrypoints refuse the closed
profile before entering an unadmitted SDK operation. Storage writes also refuse while policy
exists. This is a negative limit, not a general claim that arbitrary calls/signing/export APIs
are bounded by the decoder invoice. Operation admission, initialized ratchet/live text extension, public root2,
whole-host/provider retention/native activation and physical acceptance remain OPEN.

## Regressions and current validation

The initial25-test checkpoint is supplemented by four focused resumption tests. These four
remain UNRUN in this packet; the primary alone runs the resumed serial compiled validation.
Their final actual-diff static review is clear against the paired corrected production shape
scanner. The regression commit depends on that runtime correction, including its fixed two
epoch-fact slots; the tests alone do not supply it or establish compile coherence.

- All thirteen row routes reach actual guarded decoder; malformed late row refuses BEFORE
  signer entry, generic/direct getter escapes and optional draft getters refuse.
- Sequence bomb/duplicate/unknown/escaped identifiers, null private ratchet slot layouts,
  exact-cap/one-byte-short/overflow Vec/Deque growth, empty/ZST/inline tuple controls.
- Actual fresh founder, actual selected-KP/first Add valid roundtrip; unsupported pending,
  nonempty proposals/past/ratchets and resumption histories outside the exact initial shapes
  remain byte-identical on refusal and legacy restore works.
- Real founder/max32/epoch0, first-Add owner/max32/epochs0,1 and selected Welcome/max0/empty
  rows survive the bounded loader with exact secret/epoch/cursor bytes. All three real join
  configs still say0; this is asserted separately from the actual row contents. The cap remains
  128KiB, and either real graph half retains the SAME fee until its final Drop.
- The malformed resumption matrix covers maximum/count/cursor, duplicate/out-of-order/gap/
  missing/future epochs, tuple arity, scalar/null/unknown/duplicate/missing fields, exact secret
  lengths31/32/33, byte types/ranges and framing. Every case preserves source, observes zero
  actual SDK entries and refunds its graph fee. Separate borrowed-row DATA tests cross-check
  epoch geometry and repeat the getter with shorter/empty rows to exclude stale Facts carry.
- Same values guard prevents preflight/decode replacement; Complete/Cancelled never fallback;
  partial decoder failure releases graph then fee; malicious raw map changes are re-preflighted.
- Actual App original pool refusal before SDK, one-vs-two graph debit, child/parent overlap,
  device-first/group-first Drop and take_ready transfer do not free payment prematurely.
- Production cfg/type/feature checks and required full suites remain primary-owned validation;
  the exceptionally slow10,000-operation test stays until the end.


The real App parent-overlap regression constructs a genuine owner/direct selected-KP request,
retains its valid TempSynced generation2 stage, drops the old owner, then captures the exact
canonical parent/stage. One byte below TWO independently reserved graph caps refuses before
child SDK entry; a new physical capture with enough local capacity validates both graphs,
settles actual stage/Current/head barriers and retains the child fee in OwnedOwnerServer.
No source status/paid boolean or synthetic certificate stands in for that path.

Reviewed corrections included pre-SIGNER native unmerged-index u32/range/sorted checks,
precharged safe UTF8 revalidation and fixed key setup work, explicit diagnostic suppression,
values-writer recheck after policy-install contention, and the existing legacy lifetime fixture's
new None lease field. The native/public V2 root2 source decoder, initialized ratchets, complete
provider/Server/global fees and operation admission are separate unfinished gates.

Deferred verification selectors: root `catcoms-mls persist::owner_restore`, App
`store::owner_evidence::canonical::budget::sdk` and `canonical::restore::tests::sdk`, plus vendor
`cargo test --manifest-path vendor/openmls_memory_storage/Cargo.toml owner_restore` (dependency
unit tests are not automatically root-workspace tests). Production-config root/native checks,
optional-feature refusal paths, clippy, dependency audit and required whole suites remain for
the primary's resumed serial pipeline. No new ignored/skipped/loosened tests.


The first-Add candidate bridge itself now checks the ORIGINAL closed group/device before any
snapshot or legacy-candidate restore. Otherwise making an unrestricted DefaultNone copy would
lose the source's completed-load restriction. The genuine restored-Current owner/request
regression proves that a valid invite/request with adequate first-Add buffer capacity is refused
without changing its canonical file or retiring Current. A graph fee never stands in for a new
SDK operation admission. Optional V_TEST and CLI-import wrappers also refuse installed policies
before serializer/logger/import work; their legacy None behavior is unchanged.

Source-only checks at the original checkpoint: direct rustfmt over29 changed Rust files, whitespace,
ambient-dependency and raw-filesystem gates. The default Windows WSL bash launcher denied startup;
the same two scripts passed using installed Git Bash. No compiled suite/test/build/clippy ran
at that checkpoint. The resumption test addition has not independently run any compiled check.
