# Original bootstrap source container (authored; 5 November)

This is the Sync producer/offline DATA prerequisite for the actual
[bootstrap Store](CHAT-OWNER-ARCHIVE-BOOTSTRAP-STORE.md). It adds no App/outer catalog/predecessor,
Store/key permit, SDK restoration, Current, Fresh import, native command or network path. A
downloaded Profile, decoded Server/snapshot or supplied labels cannot construct its live producer.

## Owner, transfer and actual context

`FounderBootstrapPreparation::from_archive(original, node)` consumes the SAME actual
`FounderArchivePreparation` only after its cached S0 and encrypted genesis package exist and
the original managed source remains live. A refusal returns that SAME owner in a bounded Box;
the caller can finish preparation. It accepts no group/signer/attempt/scope/genesis replacement.
It is a private child of archive_candidate, accessing the actual owner's private
source, never a public `(Server, proof)` adopter. The host prepares archive pages FIRST.

`prepare(node, original_dbkey, rng)` repeats exact original/node/key checks BEFORE source RNG,
builds one complete original container and caches it once. It retains the unexposed encrypted
candidate even if the final negative check fails, so a retry cannot silently resample ciphertext.
Original archive key binding must agree. `borrow_prepared_for_handoff(node,key)` and
`take_prepared_for_handoff(node,key)` repeat the same checks. Take moves ONE owned
`PreparedBootstrapSource` (complete ciphertext/opaque SourceId/bounded context DATA), marks the
producer Transferred and never reseals or yields another candidate. The creator retains its
original archive/source; the detached output retains NO observer, creator, Server/transport,
Store/session/key or revoking sender. Its `.as_bytes()`/`.id()`/`.context_data()` need no live
source: Store debt must own that exact output BEFORE fallible I/O and can repair after creator
Drop. Those getters expose encrypted DATA, not continuing positive authority.

The actual context derives internally after original source checks: vault domain, actual group,
full signer, actual PRESENT Fresh attempt, original planned scope domain/nonce/DURABLE revision,
Profile/Founder Admission/header and original record/manifest IDs, exact S0 digest/limits and
record plaintext length. Do not call the process-local chat_scope_revision a durable row revision.

The default-None `ManagedChatScopeReservation::planned_revision_data()` is captured ONCE
at attach in the private Reservation, include equality in repeated attach and original observation
matches. Bootstrap requires positive Some revision from that original reservation; old None paths
stay unchanged and cannot opt into bootstrap. The actual RAM plan overrides with its real planned
record revision; a decoded label is not an implementation. OriginalInitializationObservation
exposes only internally checked planned context DATA, preserving physical identity/watch checks.
This field is not serialized into a Fresh proof or treated as Current. The trusted-Rust reservation
seam still does not authenticate arbitrary implementations; the actual App factory must own it.

`BootstrapSourceContextSpec` is a borrowed flat DATA grammar. The immutable bounded
`BootstrapSourceContextData::from_data(spec)` copies only its capped group; `spec_data()` supplies
the exact fields to the future typed intent codec. A caller-constructed expected context never
constructs a live producer. The actual `from_archive` derives every field internally.

After the one-time take, `validate_transferred_source(node,key,&expected_context,expected_id)`
returns only unit/refusal. The owner must actually be Transferred to that privately retained ID,
have no remaining prepared output, and retain the exact original context/key binding. It repeats
complete original archive/S0/node/negative checks; no bytes/token/second preparation or take is
enabled. This lets the actual host move complete ciphertext into prepaid inert custody BEFORE
SourcePrepared mutation, then repeat original source observations after writer waits. It is not
an atomic admission or a replacement for real Store debt/fee/root/writer evidence. Offline repair
after creator Drop still needs no live source getter. This narrow addition has CLEAR read-only
design and actual-diff reviews with no remaining findings; its twelfth regression is authored UNRUN.

## Exact body and frame

Body domain `catcoms/private-chat-bootstrap-source/v1\0`, u16 version1, followed by framed vault32,
group1..256, signer32, Fresh attempt32, scope domain32, scope nonce32, planned revision u64>0,
ProfileCID32, Founder AdmissionCID32, headerCID32, original recordsPageID32/manifestPageID32,
S0 limits(snapshot/MLS/backing as u64), framed Profile-bound S0 digest32, records plaintext u32,
then FOUR framed exact original payloads: S0, signed Profile envelope, encrypted record frame,
encrypted manifest frame; no trailing bytes. Scope domain==vault. SnapshotLimits::new validates
all encoded limits before policy use/copy; these numbers are DATA, not fees.

The small duplicate Profile envelope lets the offline reader supply the actual typed Profile to
RecordBatch's existing bound decoder. It must match the exact Profile representation in the
record batch, not just its normalized CID. No old public codec or record role is weakened.

Maximum S0 is8454144 (8MiB+64KiB), Profile494, record frame65622, manifest frame1110
(1024+86). Fixed maximum body metadata755 gives plaintext maximum8522125. Freeze the arithmetic
against the actual constants; a future limit/field change needs compatibility/invoice review.

Outer frame domain `catcoms/private-chat-bootstrap-source-sealed/v1\0`, u16 version1, raw nonce24,
framed ciphertext(p+16), no trailing bytes. Domain framing/version/nonce/length/tag overhead98,
complete frame maximum8522223. Derive a separate key with BLAKE3 derive_key context
`catcoms/private-chat-bootstrap-source-key/v1` from the original vault database key. Never reuse
ordinary db-key sealing or the generic private-page key. SourceId hashes the COMPLETE frame
under `catcoms/private-chat-bootstrap-source-id/v1\0`; a raw label does not select its own source.
Use a separate zeroizing secret-derived comparison binding to reject a replacement key on retry.
All source nonce draws use the caller's original RNG, never a cloned/reinstalled RNG.

## Strict offline DATA open

`open_bootstrap_source_data(dbkey, expected_full_context, expected_source_id, frame)` is independent
of a live creator. Hard frame syntax/size/whole expected ID precede ciphertext copying and AEAD.
Then complete borrowed body framing and ALL payload caps precede bounded typed public decoding.
It checks all expected context/IDs/limits, full raw Profile-bound S0 digest, and actual labels in
the unchanged closed private S0 framing. Group/full signer match nested MLS envelope labels;
storage rows are borrowed length/count framing only, never JSON/OpenMLS/serde expansion. Closed
metadata (empty work/rosters/finalization, bounded routing/rendezvous/policy/tenure/scope) and
exact scope group/signer/domain/nonce/Active-without-aliases must match; unknown/v1 owner tails
refuse. Planned durable revision is bound by the container/independent intent; it is NOT inferred
from S0's scope tail, which does not serialize that durable table field.

Recompute the existing founder creation label from group/full signer/Fresh attempt, then verify
the supplied Profile signature/group/founder/creation/CID. Open the original nested record and
manifest frames with their expected private-page IDs and original page key. RecordBatch must be
GenesisRecords3 with the exact Profile representation, Founder Admission, sequence1 signed header
and full canonical image/genesis binding. Manifest must be sequence1/no previous/source/before,
Genesis body and exact original record ID/plain length/3 rows, same vault/group/Profile/header.
Every actual signature/state/frame comparison remains, including valid signature representatives.

Return immutable zeroizing owned plaintext plus borrowed context/S0/public encrypted frames DATA.
There is no `into_server`, generic verifier callback, RawSnapshot-to-Fresh/Prepared constructor,
SDK expansion or Current/read permission. MLS key/value duplicate semantics and expanded SDK
credential/group validity require the later separately paid typed restore; this reader proves
framing/digest/context/public genesis consistency, not that opaque raw SDK data is runnable.
The actual producer establishes original device provenance; public finalizer trust is separate.

The full context+SourceId must come from the actual typed PREPARED intent successor durably saved
BEFORE first seed write. The earlier pre-staging intent can omit not-yet-created genesis/frame IDs.
The reader must not choose a complete expected context by decoding its own untrusted frame.
Both intent versions/transition and actual Store debt/final admission are outside this Sync slice.

## Independent allocations and phases

This container is NOT the archive512KiB or founder64KiB allowance. During capture the original
cached S0 (<=8454144), original public frames (<=66732), live SDK/provider/backing/payload and
frozen public source remain retained. Compound encoding/sealing separately overlaps plaintext
P<=8522125, AEAD ciphertext P+16 and final frame P+98:25566489 requested bytes, plus fixed
context/comparison/crypto state. The output moves to debt without a second full ciphertext copy;
the actual seed/current writer may still need separately funded copies/read buffers/file state.

Cold decrypt overlaps caller raw8522223, copied ciphertext8522141 and SDK plaintext capacity
8522141:25566505 requested buffer bytes, not allocator/RSS or paid quotas. Drop that temporary
ciphertext BEFORE nested page validation. Nested record opening separately overlaps source
plaintext, any retained raw source, record ciphertext/plain capacities<=65552 each; manifest
<=1040 each, bounded typed Profile/Header/Admission/group/signature/state and fixed batch arrays.
No extra raw S0 copy is needed: hashing and inspection borrow it. The separate named
`BOOTSTRAP_SOURCE_NESTED_DATA_INVOICE` is256KiB: both nested cipher/plain capacity pairs, both
record metadata arrays, HeaderRow/header/state/Profile/Admission codec buffers, bounded groups,
policy/field/signature scratch and fixed owner/context/refusal layouts are conservatively summed
and statically checked against it. The manifest plaintext drops before record opening; summing
both is explicit overpayment. It is a logical requested-layout inventory, never a paid lease or
the existing archive512KiB allowance. Retaining raw seed/current/debt copies adds each
whole original frame. Constructor metadata/read fees must exist BEFORE unseal, not be asserted
by decrypted limits. Typed SDK/active-host heaps remain unbounded/unclaimed by this primitive.

## Authored evidence and current status

Twelve regressions use real founder/cached S0/archive frames and an original reservation carrying
planned revision DATA. They pin pre-constructor owner recovery, exact context provenance, no RNG on invalid source/key/retry,
same frame/ID on retry, single take and ciphertext accessible offline after creator Drop; original
cancellation before/during sealing prevents positive view/take without resealing. None/changed
planned revision with equal local revision/labels, wrong full expected context/key/source ID,
all framing/length/version/domain/limit/digest/raw-S0/scope/page/manifest/Profile/header mutations,
and cross-key-domain rejection are covered as authored regressions. A newly encrypted same-CID
invalid Header signature with a corrected row digest and newly bound manifest/source IDs reaches
the actual signature check; a validly encrypted manifest with the wrong record length also refuses.
No live-source getter is required for debt repair. All twelve remain UNRUN while tests/compiler/builds are paused. The concrete
read-only design and final actual-diff adversarial reviews are CLEAR, with no remaining findings.
Direct rustfmt and
whitespace checks do not establish compilation or physical source I/O acceptance. The actual
SourcePrepared intent, creation grant/fee, Store seed/source repair, atomic original final admission
and production/native activation remain separate unimplemented consuming gates in this slice.

Pending focused selector after compilation resumes (not executed):
`cargo test -p catcoms-sync bootstrap_source::tests`. Required root/native/frontend suites and
lint/dependency gates remain owned by the primary agent; slow acceptance runs stay at the end.
