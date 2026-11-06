# Task-free cold genesis group read guards

Status: actual implementation and connected regressions are statically cleared; all eight
guard tests and six Root capture tests pass.
This packet supplies no cold owner, task claim, writer, scope mutation or Current completion.
Primary owns the single build/test pipeline. The guards/capture are jointly declared under
`cfg(test)` until the actual cold caller is connected; they do not activate a production path.

The actual inputs are Authority's `ColdV2InitializingReadAdmission` (unowned historical
Initializing1) or `ColdV2CurrentReadAdmission` (unowned historical Current2), the actual
`GenesisSourceDataRef`, and Root's separately acquired `ColdV2GenesisReadResources`. The
two admissions borrow the SAME real admission Runtime for the entire callback. Their fixed
basis includes the full group/signer, origin, nonce, provenance, phase and revision. Each
checks any-signer group row/predecessor/JOIN exclusion; no decoded basis constructs a role.

## Closed interfaces and lock order

`current_layout::{ColdV2InitializingGroupReadGuard, ColdV2CurrentGroupReadGuard}` are distinct
concrete types. Private `ServerStore::lock_cold_initializing_genesis_group(admission, resources)`
and `lock_cold_current_genesis_group(admission, resources)` accept only their corresponding
actual admission and the borrowed concrete resource loan. No phase argument, public callback,
raw Runtime accessor, task, SavedDirectory or caller path substitutes for either input.

The loan is cloned from `PreparedColdV2GenesisCapture` BEFORE admission. The mutable capture
stays outside the guards. Its original account, mounted source-read and SDK identities are
validated before path allocation and repeated after waits. The guard never acquires a new fee
or clones the loan under a writer. All paths/Files drain while that outside loan is still live.

Each constructor takes the existing locks in order: already-held admission, pins, canonical,
archive Root. Its private canonical child checks clean loaded pins, no old group pin, no
unresolved group-directory entry (even if its task expired), no old canonical HeadCell at this
exact key, and no live V2 writer at the key. A cancelled strong writer is still live. It borrows
the current filename association from the SAME admission Runtime and rechecks the actual old
directory handle/name and both fixed `.bin`/`.replace.tmp` names after Root acquisition. It
never treats an absent canonical budget or unknown filename walk as empty.

The group directory is selected only by the existing fixed vault/root/group `g-<64hex>` path.
It must already exist as a real no-reparse directory. The persistence child opens it exactly
once, retains its File and in-session identity, and compares the held File with a freshly opened
named directory. `kept::session_at` is file-only and MUST NOT be used here. There is no mkdir,
create, truncate, rename, remove, sync or adoption of a missing path. Equal empty replacements
cannot satisfy the retained original handle. Failure destroys only this read attempt's owners.
The owned directory type itself retains a borrowed loan lifetime, so its File/path cannot be
returned outside payment even by another future Store-private caller of the helper.

Final account/source/full-ID3 checks and old-file probes precede the last group handle/name
proof. Full ID3 is repeated after the source-control and old-file reads, preventing a late
marker mutation/replacement from using an earlier proof. Only cached original row/source/
resource/Root negatives follow the final directory proof. No cached Boolean
replaces a physical original object. The cooperative private-directory/OS-session assumption
is the same as the warm directory boundary; this is not hostile-filesystem atomicity.

The outer getters are exactly `store`, `source`, `archive_root`, `directory_data`, `resources`,
`scope_basis_data` and `validate_source_only`. The last method is cached negative validation;
it does not lock, perform I/O, access the account, encode, allocate or call SDK. Actual trusted
negative authorities remain bounded weak/atomic checks. Root's capture compares the SAME loan
Arc, exact basis, independently authenticates complete seed/pin/head, retains original file
objects and repeats physical proofs before issuing its NEW cold DATA receipt. The guard alone
does not authenticate any metadata and cannot enter a warm Seed/Head writer.

## Requested-buffer inventory

Root's actual independent cold-only 1MiB capture reservation includes the new guard's named live
inventory before construction: one bounded group path and held directory File, one temporary
named directory File and its metadata, concrete outer/canonical guard layouts, and the four
bounded old-owner selector paths plus fixed 64/128-byte key/name buffers. The existing closure
retains five paths and three small metadata frames. Preserve its source/expectation/basis/Arc
and returned-copy layout charges. A new exported private constant will enumerate the Factory
inventory using actual `size_of` layouts, `MAX_OPEN_BASE_PATH+256` path ceilings and the same
32KiB fixed native-path conversion scratch; Root adds that constant to its actual invoice.
The prior cold invoice already names 264,192 path bytes, 161,184 frame bytes, 32,768 scratch
bytes and actual retained structures. The additional simultaneous Factory inventory exceeds
the remaining 512KiB margin on supported layouts. Warm activation keeps its original 512KiB;
only cold capture uses the explicit 1MiB class. A checked exact combined sum must fit that class
or preparation refuses BEFORE any guard allocation. No overlap credit is taken between the
retained Root closure and the still-live Factory directory/old-selector proof buffers.

No saved size/cap creates quota. This is named logical Rust/native conversion scratch, not
allocator/RSS/global-host or SDK accounting. Parent/child SDK graphs and source-reader bytes
keep their existing independent original payments. No fee owner can become last under these
borrowed guards; the prepared capture and outside loan own it before admission and through
all File/path/lock destruction.

## Required connected regressions

Use genuine SourceSaved -> ID3 -> paid source/SDK -> actual warm task/Seed/head/scope, then
close/remount and independently read/restore for each cold role. Row1 fixture must have genuine
complete head bytes retained before a scope flush stop; no raw row, receipt, watch or directory
flag may be fabricated. Pin exact row1 and row2 role separation and no owner/sender creation.

Cover missing/non-directory/replaced group path; late replacement after full ID3 wait; account
pressure before any path/open; same bytes from a different paid read or different resource loan;
pending directory/task debt and strong cancelled V2 writer; old pin/head/stage/index unknown;
old mount/row mutation and different-signer predecessor/JOIN; unrelated group progress. Root's
actual capture tests must verify seed/pin/head/no-stage authentication and held-file replacement
refusal. First cold profile intentionally refuses partial or missing seed/head/pin and settled
seed-absent Current. Cold task/owner/writer and automatic partial recovery remain later work.

Eight focused tests are authored UNRUN: genuine complete historical row1/row2 capture/cache
and role separation; one-byte-short account with no real path/open entry; equal new paid read
and wrong resource Arc; late equal-directory replacement after full proofs; missing/regular
group names without effects; late original cancellation; and unknown old current/stage files.
The eighth pins late marker-byte mutation and equal-marker replacement with seed preservation.
The cfg(test)-only fixture performs the actual warm producer/head/flush and remount chain and
may be borrowed by Root's connected capture tests. No test fixture constructs a proof or row.
Actual source/Scope predecessor negatives, Root receipt authentication and eventual cold
claim/writer ownership retain their separate connected-review and serial-execution obligations.
