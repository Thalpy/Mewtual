# Pending-only destination journal checkpoint (5 November; authored, unexecuted)

This is an inactive private Rust API for the finite OwnerFact-v1 checkpoint protocol. It stores
and reopens original client keys/request DATA. It is not production Chat V2 joining, native UI
activation, a recovered network driver, or completion of durable join acceptance. No test or
compiler/build command has run under the user's pause. Thirteen new App tests are authored;
read-only design and final actual-diff reviews are clear. Ordinary production-config compilation
and all focused/full validation remain required once the pause is lifted.

## Concrete policy and compatibility

`ServerStore::open_with_pending_join_catalog` accepts a trusted `PendingJoinCatalogPolicy` with
one shared `CriticalCatalogReadPool` and an allowance for BOTH fixed catalog files. It opens only
an already activated actual v6 vault. Ordinary open and the older configured Empty reader remain
Empty512-only. They refuse a nonempty v2 catalog before older cleaners. Neither stored resource
limits nor an IPC payload can issue the configured pool or namespace support.

The configured mount disables the old unjournalled checkpoint join factory BEFORE reserve/mint,
even when its catalog is Empty. An old bound attempt from an earlier physical mount remains
cancelled and cannot send after reopen. The new preparation exposes only `save_pending`,
`pause_pending` and `resume_pending`; there is NO transport/network/old-initializer/ready adapter.
The authenticated recovered source remains opaque DATA with no into-Prepared/finish method.

Public entry is `prepare_journalled_owner_checkpoint_join`, from a genuine ready source whose
Saved/Current/head was checked and released before destination work. It captures the destination
row and expected typed root, reserves actual creation/source resources, prepares one KP, and seals
that original source once. Equal group+invite nonce coalesces the stored exact full invite/request/
package and discards a caller's unused replacement device. A changed signed full invite with that
nonce refuses. New root-kind2/public-owner-v2 invitations remain unsupported; same-width Profile
IDs or the new actual v2 founder cannot be reinterpreted as finite OwnerFact-v1 proof.

## Closed bytes and transaction

V1 generation1/Empty bytes are unchanged. Nonempty candidates use explicit catalog-v2 tag,
original vault domain, checked generation, COMPLETE prior plaintext hash and sorted ID/version/
payload frames. Only namespace1/version1 Pending is supported. ARCHIVE2, unknown progress,
versions, IDs and malformed/trailing data preserve/refuse. No caller callback can declare a
namespace supported. Progress Pending=1 is separate from Active/Paused scheduling.

Five retained rows TOTAL, at most one Active, are fixed metadata slots. All five may be Paused:
a full pool can pause an unavailable Active and then resume another in two durable steps without
admitting a sixth source. Every source binds protocol1/root-kind1, domain/source nonce, full
group/signer/Profile/founder, complete invite/request digests, selected KP reference and exact
original optional admission row. Selected private bundle/group/nonce checks are repeated when
coalescing from the actual authenticated current file. Dormant Store metadata keeps no plaintext
master or whole catalog plaintext.

A stage is either equal complete plaintext DATA or the exact checked next generation with whole
parent hash and one closed transition: add an original Active row, or toggle one row's scheduling
with its checked next revision. Existing immutable/ciphertext bytes remain equivalent, no row is
removed, and no JOIN-only comparison can discard a different namespace or parent. A nonempty
stage without its actual parent refuses. BOTH files authenticate/preflight before older gates,
then actual opened-file/root barriers precede duplicate removal or known promotion. Cold opening
starts no worker and supplies no Fresh/Current/Saved authoring witness.

Large read/encode/AEAD work occurs before final writers. Under original admission -> catalog,
the permit rechecks original negatives AFTER the catalog wait, exact original row/whole-parent/
raw-source state and pin, then retains the SAME complete encoded candidate and its original
capture/fee in Store. It cannot be overtaken or replaced by a re-seal. The I/O retry uses that
same original permit/candidate. All retired bytes/captures/fees drop after both guards. Revision
or parent changes from a different preparation cannot be adopted as this object's retry.

## Partial writes and independent work

Before the first fallible stage identity query/write, Store retains the ACTUAL read/write File.
Warm retry requires its in-session `SessionId` at the fixed name, a bounded EXACT-prefix scan,
then writes the SAME cached full ciphertext through that SAME held handle. It never reopens or
truncates a path to repair it, never replaces a foreign/newer object, and uses no new RNG/KP.
File/root sync and postrename retries still cross their real barriers. The identity proves a
regular same object, not exclusive/no-hardlink custody; the existing non-hostile live private
vault-directory assumption remains. Stronger external-path race guarantees are not claimed.

Cold partial/unknown files have no retained created-file witness and still preserve/refuse.
Complete cold stage/current files can settle known DATA, but full cold partial recovery remains
an explicit open gate before durable JOIN1 completion. A caller dropping after an uncertain
write cannot erase Store's exact frame/original/fee. The catalog mutex is not retained over any
returned refusal or network work. Another already admitted group's canonical/control work can
progress while this catalog candidate is uncertain; no archive execution claim is made while
ID2's real handler is absent.

No network worker exists in this checkpoint. Actual worker activation/revocation, pause/remount
native generation fencing and publication require their own original activation lease; DATA
scheduling success cannot issue it. A source whose admission basis changes remains unavailable
for positive work. The eventual negative-only pause/custody repair must not counterfeit a fresh
capture or erase ambiguous keys to free a slot.

## Resource inventory

`C = JOIN16MiB + reserved archive descriptor4KiB + header512`, `F=C+40`, `U=C+16`. Reserving the
future descriptor's bytes does not support its handler. A read fee owns ONE raw `F+1`, unframe U,
decrypt U, two fixed summaries and one fixed parser. Sources are classified sequentially; no full
current plaintext survives the stage read. A separate fixed metadata fee survives in Store.
A writer separately owns new encoder C, ciphertext U, framed F and one8192-byte scan buffer,
while its parent-read fee remains charged until parent buffers/file are gone.

Each selected source pays named master512KiB, request(9+64KiB+32KiB), encrypted-cache/sealing-or-
decrypt outputs, nonce24, bounded invite/verification/address-vector copies, immutable frame2KiB
and its concrete preparation layout. Shared original capture/pin backing has its own strong fee
because it may outlive the caller inside a pending output. Candidate/frame, actual created File,
metadata and original byte owners precede their non-Clone fees. Pool clones share one counter;
limits decoded from disk pay nothing. Two fixed files are bounded by the configured persistent
allowance; this is not filesystem free-space reservation or a global disk quota.

These are named logical buffer/metadata inventories, not allocator/RSS or process-global native
admission. Existing creation pays original raw providers separately. Full typed SDK JSON/group
heap/work, lower transport, active server and native/UI lifetime owners remain separate gates.

## Authored verification and remaining work

Eleven actual App tests use the genuine Saved/Current founder invite producer and cover exact
save/reopen/coalescing, configured old-factory refusal, changed signed invite, full-five pause/
resume/no-sixth, zero/short/sync/rename warm retries after caller Drop, foreign/wrong-prefix
refusal, complete versus partial cold input, quota before read buffers/refund, independent other-
group control, old pre-remount bound request refusal, original negative winning while the actual
catalog writer is held, and unknown archive/progress/parent preservation. Two admission-basis
unit tests preserve all row fields/phases/origins and reject malformed flags/truncation/zero
revision. All13 are UNRUN; no assertion is loosened or test skipped to claim completion.

Still open: actual mounted source -> original active network driver; accepted reply journal;
source-bound ScopeIntent before row mutation; exact FrozenCandidate before canonical I/O;
journal-bound v6 member saver/restore; exact canonical/Current/terminal Settled barrier and ready
handoff; native/global/SDK admission, production root-kind2 invitation and physical crash/network
acceptance. Existing exact-v4 negative and S1 adapters remain exact-v4.
