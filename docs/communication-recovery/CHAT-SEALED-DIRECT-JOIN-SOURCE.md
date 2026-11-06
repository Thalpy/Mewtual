# Sealed direct-join source prerequisite (5 November; authored, unexecuted)

This slice adds secret DATA custody to the inactive finite-checkpoint client. It does not enable
JOIN1, reopen a mounted journal, provide a recovered finish API, activate native Chat V2, or
close SDK/global/lower-transport memory admission. All nine new tests are authored and UNRUN
under the user's compilation pause. Direct rustfmt and static diff inspection are the only
local verification. Read-only design and actual-diff reviews are clear after replacing an SDK
test-utils-only reference constructor with its public pinned TLS codec; normal production-config
compilation remains required once validation resumes.

## Original local seal

`PreparedDirectJoin::seal_source` accepts the actual fresh in-process source, original vault key
and immutable source-context digest. It serializes the exact private master, exact signed invite
and canonical selected KeyPackage request once. The fixed shape is u32-framed tag, checkpoint
flavour1 byte, explicit OwnerFact-v1 root-kind1 byte, framed32-byte binding/signer/selected
reference, framed secret master and framed
original request. The request retains the existing kind1/invite/KP network bytes. This is a
private persistence envelope, not a new wire protocol or production v2 invite. The immutable
source binding must include that same root kind and actual typed Profile/founder expectation; a
raw32-byte identifier or new actual v2 founder cannot be reinterpreted as a v1 checkpoint root.

The cap is512KiB for the master and640KiB for the complete plaintext; invite64KiB, package32KiB,
group256 and both address vectors64 remain explicit. A valid larger configured provider may
continue its existing in-process join but cannot use this narrower durable-source profile when
its actual master exceeds the cap. No truncation or replacement key is allowed.

The first seal caches its encrypted output. Repeating the same original binding/key returns
identical ciphertext and consumes no additional randomness. Another binding/key refuses. A
private process latch becomes started BEFORE the first transport request is invoked; immediate
failure or cancellation cannot reopen the seal boundary. Dropping an unpolled async future has
executed no request and does not claim an ambiguous send. Completed/recovered sources refuse
sealing. The latch and cache are never decoded alive/never-sent/paid authority.

The caller must save this source through the actual catalog/file/root barrier before the first
request. This API alone performs no file I/O and existing unjournalled in-process APIs are not
silently relabelled durable.

## Strict reader without resume authority

`SealedDirectJoinSource::from_encrypted_parts` checks ciphertext bounds before its one bounded
copy. `RecoveredDirectJoinSource::open` authenticates those bytes under the original key and
compares the exact expected source-context digest. Complete outer/request/invite framing,
counts, lengths, flavour and trailing-byte checks precede the first owned invite decode. The
normal invite decoder then verifies the exact canonical invite and self-signature. Its current
expiry is deliberately not reapplied to historical retained DATA.

The bounded master-provider decoder checks the original full signer. The new negative MLS
`validate_retained_join_package` loads the actual stored selected private bundle, bounds canonical
package serialization, compares its exact wire bytes/reference/full signer and original group/
nonce membership credential. A same-signer master without those original selected private keys
refuses. No package lifetime validation, new Add, provider mutation or membership witness follows.
Actual Welcome processing remains necessary to use the private keys and establish membership.
Stored bundle JSON/typed SDK decoding is not claimed to be heap/work-admitted here.

The recovered object exposes only inert group/full-signer/binding labels. It has NO public
finish, into-Prepared, plaintext, clone or serde adapter. Therefore caller-supplied key/binding
cannot by itself become a generic old-master-to-Fresh bridge. The actual future App resume
consumer must capture the original authenticated mounted catalog/source, root, original row CAS,
source intent and current negative activation BEFORE any await. Arbitrary Rust host trait
implementations, if introduced for that cross-crate bridge, must be documented as trusted code
rather than cryptographic proof of a disk event.

## Allocation and ownership boundary

These methods issue no resource capability. The caller must reserve before allocation and retain
fees until their data drop. Local sealing overlaps existing master/request with one exact-sized
zeroizing plaintext and AEAD output (plaintext+16 bytes), then retains the cached ciphertext plus
nonce. The cache owns no vault key; a keyed context digest detects changed-key retry.

Encrypted import owns an exact ciphertext copy after length preflight. Open overlaps that source
with AEAD decrypt output whose capacity includes the16-byte tag, bounded owned invite/canonical
verification copies, one bounded raw provider and stored-bundle SDK copies. Only after dropping
the provider does it copy the exact master/request into retained DATA. That DATA keeps its own
fee after scratch is returned. The API requires actual negative limits from the current original
reservation, never saved limit fields. General typed SDK heaps, allocator rounding, process-global
native/UI ownership and lower transport resources remain separate gates.

## Authored regressions and remaining acceptance

Six Sync tests cover exact repeat/no extra nonce, changed key/binding, ciphertext tampering and
caps, authenticated malformed inner counts/tails/unknown flavour before owned decode, missing
selected private bundle, current negative limits and cancellation before/after first request
poll. Three MLS tests cover repeated nonmutating exact package checks, wrong reference/device/
group/nonce and expired retained package DATA while new admission still rejects it, plus a fixed-digest typed
reference roundtrip through the production public TLS codec (no SDK test-utils constructor).

Connected original-journal replay/retarget, remount/pause stale workers, genuine owner expiry/
restart, actual ScopeIntent/FrozenCandidate/canonical/Current/Settled barriers and per-slot fair
progress remain in the JOIN1 work. Existing exact-v4 negative and S1 adapters stay unchanged.
