# Reserved Global Governance capability4/version1

Current status, 8 November: the namespace4 codec and concrete private metadata
mount are declared in production modules. The mount grants historical DATA only,
not Source4, durable Add request, MLS operation, signer or publication authority.
Configured namespace4 handling preserves existing JOIN/ARCHIVE/CURRENT entries;
older configured readers refuse unsupported catalogs. No namespace3 grammar is
widened. Chat V2 activation and the subsequent source factory remain incomplete.

Repository-local assigned IDs are JOIN1, ARCHIVE2 and CURRENT3. ID4/version1 is now
reserved for a distinct immutable Global Governance capability. It binds the SAME
vault/root and exact canonical namespace3 descriptor. Its descriptor uses the tag
`catcoms/private-chat-v2-governance-layout/v1\0`, inner u16 version1, then three
length-framed b32 fields: vault, root, namespace3 descriptor commitment. The latter
is BLAKE3 of `catcoms/private-chat-v2-governance-current-descriptor/v1\0`, followed
by big-endian u64 descriptor length and complete descriptor bytes. This digest is
neither the whole-catalog plaintext ancestry digest nor exact ciphertext CAS ID.
Request rows, counters and progress do not belong in the immutable capability.

The declaration DATA builder first validates the COMPLETE existing supported
Current3 catalog (including actual known JOINv1/v2 DATA), requires ID2 and ID3,
then derives4. It checks whole input framing/domain/EOF, entry count, checked
successor generation and candidate size before growing an encoder. New catalog
parent is exactly `catalog_envelope::catalog_digest` of the WHOLE old plaintext.
Every prior entry ID, version and payload is copied and compared byte-for-byte.
Exactly one4/version1 is inserted at its sorted position (11 November: appended when
every existing ID is below 4, byte-identical to the earlier builder; inserted before an
already declared member-outbox ID5 otherwise, see CHAT-V2-MEMBER-OUTBOX.md); no JOIN
normalization, downgrade, scope reset,
root change or cap increase is permitted. Decode supplies no runtime support or
execution grant. Inputs/candidate/parser/sealing overlap need independent caller
funding; this DATA slice supplies no reservation or actual Root payer.

Four codec tests pin v1/v2 JOIN preservation, wrong vault/root/descriptor digest,
altered JOIN/2/3, unknown/duplicate4, parent/generation/overflow/EOF and byte caps.
An actual Current3 fixture separately authenticates its real encrypted catalog,
appends4 as a negative test object and verifies the real older private opener
refuses current4, stage4, equal-plaintext distinct-ciphertext4 in both slots, and
stage4 without current, preserving both fixed sources and using no new RNG.
The nested JOIN syntax fixtures are DATA only, not fabricated source authority.
The primary ran all five App tests matching `governance`: PASS in 36.44 seconds
after a 2m37s compile. Workspace formatting also passed. The implementing child
ran no Cargo commands. This paragraph records the initial negative codec slice;
the subsequently integrated metadata mount is described below.

## Integrated Current4 metadata mount

`governance_layout` now contains a separate opaque metadata mount preparation and
completed wrapper. Its parent module wiring is applied. It accepts only an
already settled Current3 vault, with actual existing ID2 root and ID3 marker and
no old declaration seed; new vaults and ID2-only catalogs refuse. The policy is
installed before common opening. The default and older configured openers gain
only a negative fence for either new fixed seed/marker, before their old gates.
To fence binaries that predate those names, declaration first creates and syncs
an owned empty legacy catalog stage and its parent, before either governance-only
output. The predecessor already rejects that stage's invalid framing. The same
owned stage is later filled from the complete authenticated seed. A cold empty
fence without a complete seed is preserved and refused, never treated as absence.

The new inspector reads both fixed catalogs, the compound declaration seed and
marker, and rejects conflicting old ID2/ID3 declaration debts before invoking
older inspectors or settlement. A complete seed authenticates BOTH exact catalog
frames, their whole-plaintext ancestry relation, immutable capability4 and marker.
Before promotion it can finish only a matching listed prefix; after promotion
the marker must already be complete. A warm promoted owner cannot adopt a new
stage name for cleanup, even with identical bytes. Only an independently captured
cold duplicate has that role, retaining its original File through all waits and
comparing its complete exact bytes again before unlinking. An existing4 mount without a seed requires
an already complete authenticated marker and holds its original native object
across older gate/account waits. Byte-equal replacement does not renew that owner.

The independent declaration account is `12 * MAX_SEED_FRAME + 512 KiB`, including
the bounded two-catalog read/unframe/plain/copy/reseal overlaps. Retained wrapper
and marker metadata have separate named `size_of`/buffer/path terms. Persistent
capacity includes current + stage + the TWO-catalog seed + marker (approximately
four maximum catalogs), not the older Current3 three-frame class. These are
requested logical allocations, not allocator/RSS or a process-global pool claim.
The large declaration account and original copies drain outside writers before
separately paid old gates/canonical inventory work. The integrated pre-review
cohort passed all19 governance tests (14new plus five initial compatibility tests)
in207.17s; native compilation and the named layout assertions passed. The later
independent-review corrections have additional regressions; their results belong
in HANDOVER rather than being inferred from that earlier run. Full suites remain open.
The prepared owner's field order also drains declaration Files/data before its
Store releases the actual vault session on ordinary failure/Drop; fees remain
last. A genuine tail-destruction test observes that vault exclusion is still Busy.

New physical tests cover warm exact-output retries without new RNG, every later
complete-seed crash cut, old-opener refusal/byte preservation, missing marker
after promotion, competing old debts, native-object replacement, fee/capacity
refusal, real JOIN pause/resume/migration/arm preserving4, and real saved-founder
historical DATA after the old SDK/read/mount owners drain. The resulting wrapper
has no Store getter, Current3 conversion, Source/Scope factory, request receipt,
SDK/key permission or publication capability. A positive Source4/savedG owner
constructor remains unpaired and is not supplied by this physical metadata slice.

Cold interruption during the FIRST seed create/write, before an independently
complete authenticated seed exists, is still Held and preserved. The same warm
owner can finish its own recorded exact prefix, but a cold opener cannot infer
the missing bytes or choose a new nonce. This is unfinished automatic recovery
work before overall Chat V2 completion; user-visible self-resolution needs a
later evidence-backed birth/scratch role. Unknown/foreign preservation must not
be weakened to hide that gap. Ordinary member sends/outboxes must remain
independent of owner/governance debt.

The separate `scripts/check-chat-v2-predecessor.py --target-dir <shared-target>`
gate builds an example public Current3 opener against the complete archived
`f5ef22a3688a1272edb418da8b02efe945c3008d` production sources. Each run uses a
fresh source tree; no modern fence is backported. It checks a clean positive
control and cuts229/232/234 after all original owners drop, accepting only the
predecessor's exact empty-stage framing refusal. The new-writer test compares
the complete small fixture tree's names, file lengths and content hashes, then
proves supported new-reader recovery. The script requires exactly one executed
test and all four probe markers. Ordinary tests without the supplied predecessor
binary establish writer fences/recovery only. Record an actual successful
dedicated gate before claiming two-version public-open acceptance.
The dedicated gate passed on8November: clean0 opened, cuts229/232/234 refused with
the expected predecessor decoder error, complete fixture inventory was preserved,
and supported new-reader recovery passed. One test executed in58.94s. The helper
build uses an isolated predecessor target to prevent incompatible same-name path
crate artifacts being reused by the current writer. See HANDOVER for source/caller
provenance, the initial cache failure and the broader acceptance still outstanding.

## Subsequent Current4 source admission

Use a NEW private physical opener after the old mount and admitted writer/reader
work have drained. Never upgrade an exposed Current3 Store with a support flag.
Install the concrete4 grammar/handler before common opening; inspect BOTH entire
current/stage sources and a distinct authenticated declaration seed before every
older cleaner. The seed must retain BOTH complete original and candidate catalog
frames plus exact marker; it may not reconstruct old rows from a Summary after
an uncertain promotion. Only its exact original cached-created partial stage may
continue. Unknown/torn/foreign/unsupported inputs remain preserved and closed.
Declare/settle marker, file, name and parent barriers before returning a wrapper.
JOIN/2/3 and later immutable4 must survive every configured re-encoder unchanged.

The crash gap between completed capability declaration and first request must
have a concrete supported empty-governance classification after both fixed
journal sources/active-work exclusions are proved absent. It grants no signer.
There can be no fallback from4 to a Current3 or genesis-only positive validator.
First request admission needs a genuine saved founder parent plus same original
pin/Scope/native generation and an independently paid bounded KP/request capture.

## Later distinct Add custody

Reserve the exact request token, KP and global public parent durably BEFORE
private SDK Add dispatch. Use a sibling non-Clone governance conversion of the
same strong group/full-founder negative lane, so ALL owner-Header producers
conflict while ordinary member signing/outboxes remain independent. Cache finite
admitted work and whole candidate ownership before leaving Current or returning
late errors. The durable Entered phase must precede SDK dispatch; actual original
outputs/snapshot and Core unsigned Admission intent must be covered before any
owner Header key call. New real SavedRequest/SavedIntent/current/signing roles
must issue from physical barriers; existing text SavedI/S/B remain Append-only.

Cold Entered without a complete original result is Held, never permission to
re-Add or recreate LocalMembershipChange from a restored roster/snapshot. Complete
cold result/intent continuation needs its own reviewed producer-provenance issuer.
Post-Add source needs a distinct sealed epoch1 source and Current owner; the
strict Genesis/S0 and one-founder epoch0 text codec must not be relabeled. Native,
publication, public invitation root and ordinary member outbox consumers remain
separate unfinished boundaries. No such positive constructor exists in this slice.
