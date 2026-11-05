# V2 genesis Current: exact metadata format and invoice

DESIGN ONLY against the integrated SourceSaved/SDK checkpoints. No format below is implemented
or a Current issuer. Exact-format design review is CLEAR after the path-capacity, PRE-DEK and
exact original-representation corrections; all actual consumers/invoices still need review.
This refines `CHAT-OWNER-V2-GENESIS-CURRENT.md`; Root owns the connected
physical opener/writer and Factory owns the unbound SDK seed. Compilation/tests/builds are paused.

## Mandatory handler before first exposure

Reserve critical namespace **ID3/version1**, Current-genesis layout. ID1 JOIN and ID2 bootstrap
remain unchanged. The closed ID3 payload is F(tag), u16 version1, F(vault32), F(ID2-root32),
u8 root-kind2, u8 supported-role1 (founder genesis only). Tag is
`catcoms/private-chat-v2-current-layout/v1\0` (42B); exact payload122B, cap512B.
Decoded caps/role are never a resource allowance or permission to skip a handler.

Only a NEW physical opener with an explicit Current policy may declare/read ID3. Warm creation
declares it BEFORE founding or captures. Existing ID2-only SourceSaved closes its old Store/owner
and resumes through a new physical declaration and NEW paid cold source/SDK capture; no exposed
Store flag upgrade or live creator/arm crosses mounts. The same physical OS session excludes
old running writes. ID2 descriptor/root marker/root ID and all source invocation bindings stay
unchanged. Default/Pending/bootstrap-only readers reject unsupported3 before older cleaners.

The new opener classifies BOTH whole catalog current/stage plus a fixed vault-level
`chat-critical-namespaces.current.seed`, before old cleanup. Preserve complete JOIN payloads
byte-for-byte, including their supported version. Actual ID2+ID3 descriptor bodies fit the
existing4KiB descriptor reserve; keep the existing whole-envelope hard cap
`C = 16MiB + 4KiB + 512 + 40` framed bytes. A future larger descriptor requires a new review,
not decoded budget expansion.
Any current-declaration seed presence must also participate in the missing-vault PRE-DEK
negative fence, including dangling/nonregular/partial objects. It cannot create a new wrapper
or DEK over retained older ciphertext before its marker authentication fails.

Under the original ID2 Root, add fixed `current-layout.bin`: separately tagged/key-separated
sealed copy of that exact ID3 descriptor. Marker plaintext tag is
`catcoms/private-chat-v2-current-marker/v1\0`, F(tag), u16 version1, F(descriptor122);
actual174B, cap1024/frame1064. Keep its held File/identity/full digest in the new mount's
Current namespace owner. Never change the ID2 marker to simulate a supported Current mount.

Declaration repair seed tag is `catcoms/private-chat-v2-current-declaration/v1\0`: F(tag),
u16 version1, F(vault32), F(root32), F(expected original whole catalog-frame ID32),
F(complete new marker frame<=1064), F(complete new catalog frame<=C). Its body is at most
`C+1233`, cap `C+4096`, frame `C+4136`. Cache all exact candidates before effects. Order is
independent seed file+parent -> exact held marker file+Root -> shared catalog stage/file ->
catalog current/parent -> exact seed settlement. No pin/head/group mutation precedes this.
Cold repair requires complete authentic seed and the actual known parent or exact candidate;
unknown/partial/foreign entries remain held. A marker with missing/older3 declaration and no
matching seed is not permission to declare afresh. New code checks the fixed marker/seed as
negative inventory even if a rolled-back catalog omits3.

An authentic OLDER catalog replay can hide3 from an older binary that does not know these
names, because the DEK is unchanged. That component rollback, like old-wrapper rollback, is an
external monotonic-anchor/full-rekey residual; this format does not claim to prevent it.

## Group reservation, then local heads

Group name is lower-case hex BLAKE3 of
`catcoms/private-chat-v2-group-root/v1\0 || vault32 || root32 || BE32(group.len) || group`.
Group is1..256 bytes; the physical name is `g-<64hex>` under the original ID2 Root. It excludes
local signer. Local head target remains the existing bootstrap target hash(group, full signer).
No caller string/filename, directory enumeration or lifetime history map selects either path.

Fixed names are:

| Scope | Slots |
| --- | --- |
| Group directory | `pin.bin`, `activation.seed` |
| Original local signer target | `head.bin`, `head.stage` |

The actual RAM group-pending owner is registered under admission -> pins BEFORE mkdir. It
retains the original source identity/negative/candidate/fee; Runtime holds no creator sender.
Obtain/retain the actual group directory File/identity, and cross its parent barrier BEFORE
seed/pin/scope/head writes. The permanent directory is never removed when a seed settles.
ALL new-code v1/v2 root-selection/creation/restore paths check group-directory presence AND the
mandatory Current marker before the older per-signer fence. Empty/unreadable/nonregular group
objects, missing/corrupt pins or a different signer never become root absence. Existing v1
group pins prohibit V2 founding, and a V2 reservation/pin prohibits a v1 B root for the same
group. Add admission before pins where an older caller currently takes pins directly; never
reacquire admission from inside a held pins guard.

The mkdir-before-first-held-open/seed cut can leave an unowned unknown reservation. Preserve
it; cold presence is neither adoption nor Current. Same original held warm objects may retry;
distinct negative cold repair needs the complete independent authenticated seed/source closure.
An incomplete first seed stays held. Whole group-directory deletion/physical rollback cannot
be distinguished from absence by a hash-path lookup alone; preventing that also needs an
independent monotonic inventory/anchor. Individual pin loss leaves the directory fence intact.

## Exact record fields

F(x) means BE-u32 byte length plus x; integer scalars are BE. Every codec requires exact tag,
version, literal role/kind, all bounded outer ranges and EOF BEFORE copied/typed public decoding.
Every actual signed Profile/Header representation must verify before equality/duplicate handling.
All hashes below are32B DATA labels over COMPLETE ciphertext frames, nonce included, with
separate pin/head/activation ID domains; none is a public semantic Profile/body/header CID.

**Group pin** tag `catcoms/private-chat-v2-group-pin/v1\0`:

```text
F(tag37), u16(1), F(vault32), F(root32), F(group<=256), u8(root-kind2), u8(origin1),
F(exact SignedOwnerProfile<=494), F(original invocation32), F(debit ID32),
F(SourceSaved terminal IntentId32), F(complete SourceId32)
```

Maximum1019B; cap1024/frame1064. Origin1 is original saved founder only, not a trusted-host
fallback. The profile's group/founder/Profile CID must match the actual saved source. Pin trust
is GROUP/Profile scoped; future local heads reference this same selected pin. Keep v1 serialized
`pins::Pin` unchanged. A protocol-discriminated mounted-root capture must check the actual kind;
v1/v2 Profile labels sharing Rust `OwnerFactId` does not make their formats interchangeable.

**App genesis metadata** tag `catcoms/private-chat-v2-genesis-app/v1\0`:
F(tag39), u16(1), F(original debit-bound UTF8 display name<=256). Maximum305B. Everything else
is the exact supported fresh `Server::from_restored_sync` defaults/derived identity; unknown
durable App fields are unsupported, never silently dropped.

**Local head** tag `catcoms/private-chat-v2-genesis-head/v1\0`:

```text
F(tag40), u16(1), u8(root-kind2), u8(founder-genesis1), u64(generation1), u8(no-parent0),
F(vault32), F(root32), F(group-pin frame ID32), F(original invocation32), F(debit ID32),
F(SourceSaved terminal IntentId32), F(complete SourceId32), F(target marker ID32),
F(complete BootstrapSourceContext<=692), F(App metadata<=305), u64(Current revision2),
F(Founder AdmissionBody<=490), F(exact signed Header1<=634), F(canonical genesis image<=676)
```

Field maxima3170B; cap4096/frame4136. The676B image cap is the actual one-slot genesis image
at maximum group length; do not use the64-slot9244B cap as permission for extra genesis slots.
Verify Profile/Founder Admission/Header1/full state with existing typed genesis checks, exact
context Profile/Admission/header/page IDs, epoch0, sole founder, empty acceptance frontier/root,
pin frame ID, source snapshot digest/limits and all original source references. Local signer
is context.signer and must equal this original founder. Raw source Context already binds
planned revision1, nonce/domain/fresh attempt; Current revision is checked planned+1==2.
For this founder-source role, also bind Profile.creation to the producer's exact existing DATA
hash `BLAKE3("mewtual/chat/founder-creation/v2\0" || BE32(group.len) || group || signer32 ||
fresh_attempt32)`. This is the pinned `ChannelSync::prepare_owner_v2_founder` label computation,
not a new creation observation or public wire rule. A real-producer regression must pin it;
changing that source-version computation requires coordinated format review.
The row effect is exactly original Vacant -> Initializing1 -> Current2, historical OriginFresh,
provenance=context.fresh_attempt. This is derived DATA, never proof of actual vacancy/Fresh.
Never accept a later/unknown head or stage by falling back to retained bootstrap genesis.

**Activation seed** tag `catcoms/private-chat-v2-genesis-activation/v1\0`:

```text
F(tag46), u16(1), F(vault32), F(root32), F(complete SourceId32),
F(SourceSaved terminal IntentId32), F(complete pin frame<=1064), F(complete head frame<=4136)
```

Maximum5404B; cap8192/frame8232. The head binds its complete pin-frame ID; the seed binds the
exact independent SourceSaved source/terminal and both complete candidates, with no self-hash
cycle. Head references original source/manifest, not its own ID. SourceSaved debit/terminal/
origin records remain independently required; this seed does not regenerate missing source
controls, a warm SavedFiles/arm, SDK grant or actual Current.

Use separate KDF domains for descriptor marker, declaration seed, group pin, local head and
activation seed (the corresponding `...-key/v1` labels without a trailing NUL). All ciphertexts
use the existing injected-RNG seal: cache once before effects, never reseal on an exact retry.
The whole-global-scope inventory is an intentional separate case below.

## Data interfaces and actual writer contract

Proposed shared expectations are `CurrentLayoutData {vault, root}` and bounded
`SavedSourceExpectationData {invocation, debit_id, terminal_id, source_id, marker_id,
context, debit_display_name}`. These are DATA, not source/fee/admission witnesses. Root creates
them only from its real terminal capture and independent immutable debit. Codec methods verify
and borrow full framing, return sanitized DATA/views, and never accept a Saved/Current boolean,
Store callback, caller path or trait as authority. Factory's unbound seed receives the same
exact genesis/reference metadata. No private codec constructs a runnable Server or Current.
Also pass a borrowed `GenesisPublicData` (original Profile, Founder Admission, signed Header
and canonical image) from the actual opened source/Factory seed. Exact-source matching compares
COMPLETE original encoded public representations as well as their semantic IDs. A different
valid same-CID signature must verify before comparison but cannot silently replace the original
pin/head copy. A source-free head syntax/consistency parse remains DATA only; the actual Current
consumer MUST perform that exact-source match before save/handoff. This is not another trusted
trait or an authority claim from caller-supplied public bytes.

Root's concrete prepared activation owns actual saved source/read/SDK seed, actual group
reservation, pin/head/seed candidates and all fees before effects. Under admission -> pins ->
unified protocol-aware local canonical cell -> ID2/Current Root -> per-activation debt, repeat
original source/mount/group/row/head after waits. The group pending is already present. Save
the complete independent seed BEFORE first scope mutation; enter InstallingScope before that
call, retaining actual row preparation on dirty/error. Then Initializing, exact pin, head-stage/
head-current/parent, and scope Current LAST. New concrete phase guards replace neither old
VACANT nor old arm-current-only validators with a mode flag. Exact repair accepts its own
known objects; Current read requires complete existing closure. Unknown objects remain held.
Retire group RAM pending/source registry and unlink seed only through actual SavedCurrent/
owned-node handoff, with real parent barriers. Permanent group directory/pin prevents reopening
root vacancy. Drop payloads before fees and outside writers. Native registration stays separate.

## Explicit invoice and finite scheduling

| Resource | Required actual shared-pool reservation BEFORE work |
| --- | --- |
| New physical ID3 declaration | `12*(C+4136) + 256KiB`, separate from existing read/JOIN/source fees; pays both catalog candidates/open/decode/cipher/seed overlaps and marker copies. Source-specific layout assertions must verify actual phases before implementation. |
| Current namespace mount metadata | Separate64KiB logical File/identity/descriptor owner through Store Drop. |
| One prepared activation's named metadata buffers/layout | Separate512KiB: retained pin/head/seed, current/stage/pin/seed reads, raw/copied-cipher/open plaintexts, bounded public encodes/signature work metadata, two ownership/return copies and at most8 path buffers, INCLUDING transient copies. Reuse the source worker's32KiB base-path limit; construct each path with checked complete final length and `PathBuf::with_capacity` BEFORE pushing borrowed root+closed components, so32KiB+256 pays its requested capacity without growth. Ordinary base `.clone().join(...)` is not covered: it can double capacity. Assert actual capacities and all fixed layouts before allocation. SDK/source/provider/global table memory is excluded. |
| Global scope serialization | `4*(B+40) + 256KiB = 134,480,032B`, B=32MiB, acquired outside writers. Reusable serially for Initializing/Current. |
| Source read/provider/SDK graphs | Independent existing captured read/raw/backing/payload and real Factory graph leases; warm old+new graph overlap remains paid separately. |

Scope Encoder growth may retain capacity<2B (not necessarily a power of two). Its realloc peak
is<3B; sealing/frame overlap is<2B+(B+16)+(B+40), covered by4*(B+40). Aead0.5.2 reserves exact
message+tag; store `frame` reserves nonce+ciphertext. Cold raw/copy/decrypt buffers likewise
need their separately held read admission. Existing BTree row metadata/allocator/RSS is separate;
this is a named requested-buffer invoice, not a whole-process bound.

Head/seed bind the LOCAL row effect, never a whole-scope-file digest. The existing original
row serializer preserves the LATEST unrelated rows; retry may reseal that latest whole table,
not overwrite it with an old snapshot. Pin/head/activation candidates stay exact cached bytes.
The first Current uses the existing SYNCHRONOUS global scope writer, one bounded explicit
trusted-local attempt, no automatic retry loop. A fresh temporary quota on an explicit local
retry is not a refund of persistent source-write units. Do not invent another persistent scope
grant schema here. Worst-case other-group durable-save progress during this32MiB I/O remains
OPEN; async scope ownership/fairness is required separately before full Chat V2 signoff. Do not
weaken global dirty checks or fabricate durable-row floors in this format packet.

Persistent capacity is explicit: per new group pin+head current/stage+seed at most17,568 file
content bytes (plus the permanent directory and existing source17,119,058B allowance); global
scope current/staging needs its existing per-vault2*(B+40) allowance, not a per-message debit.
ID3 declaration temporarily needs3*C+4136+1064 content bytes for catalog current/stage+seed+
marker. Logical file/directory grants are separate from content/RSS/free disk. No eviction or
lifetime action/Admission cap may be inferred from these temporary allowances.

## Review/tests before exposure

Required DATA tests: exact maxima, strict field/tag/kind/version/EOF, true signed Profile and
Header verification before duplicate handling; all group/vault/root/signer/source/terminal/
pin/state/App-name substitutions; source-context count/limits/revision mismatches; no namespace
descriptor-as-resource claim; group hash identical across signers and distinct across contexts;
seed complete-cipher binding and sanitized Debug. No fake paid/Saved/Current fixture.

Root's actual tests must cover all declaration/group-dir/seed/pin/head/scope barriers and owner
Drop; ID3 unknown current OR stage before old cleaners; JOIN payload retention; ID2-only cold
remount without old creator/SDK renewal; V2 A pending BEFORE pin vs v1 B and inverse v1 root;
missing pin with permanent dir, unknown first dir/seed preservation, stale/equal-label mount;
role-specific repair/current, no genesis fallback from successor; real quota before writes and
fees through uncertain phases. Keep the synchronous scope-fairness limitation visible and test
the later async packet independently. No runtime, compiler, test or build evidence is claimed.
