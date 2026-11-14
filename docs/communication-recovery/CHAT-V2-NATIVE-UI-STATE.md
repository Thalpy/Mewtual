# Native UI-state read and save

Status: disabled implementation under focused validation. This checkpoint adds
ordinary UI-state read/save to the native text owner. Lock snapshots, mount-bound
pending retry and authentication continuity still need the next increment. Chat V2
is off; no working text send or completed acceptance is claimed.

## Original IO ownership

The selected3 owner has a separate fixed UI-sidecar slot on its original physical
allocation. A retained index metadata worker or uncertain Rename WAL cannot occupy
that slot. UI-state IO changes neither the index, catalog nor its retained request.
Its last slot acknowledgement follows actual physical/Current destruction. The
physical metadata invoice includes both Slot Arc allocations and their headers.

Root exposes opaque NativeUiStateBudget/Credit/Preparation/Read/Saved values.
Capture is infallible, including Busy/version2, and retains the whole original
physical work and unique credit until outside native guards. A successful read
retains its plaintext before the original slot and payment. There is no consuming
unpaid Vec extraction, raw Store/Current getter or UI/signing/SDK authority.

Four independently charged operations are allowed. Each reserves four maximum
16MiB frames plus4KiB control; the fixed Root IO pool is256MiB plus16KiB.
This is accounting capacity, not eager allocation of256MiB. JSON parsing,
serialization, IPC and browser memory are outside that invoice. Native separately
bounds four ordinary invocations. Original Host/index/mount negatives fence a
closed logical mount, but path-based sidecar IO does not establish resistance to
an out-of-process filesystem directory replacement. UI-state is not protocol
or membership authority.

## Native publication and lock order

The trusted native-mode synchronous router handles get_ui_state/save_ui_state
only for the actual resolved main-window ACL. It captures generation before
parsing or scheduling and pays before copying bounded input. No renderer flag
selects this implementation. Old Legacy routing remains in place.

The owned worker acquires one IO lane before the UI commit and short Store
capture. Disk IO, UTF-8/JSON validation and response serialization run on the
blocking pool outside those guards. The worker retains that lane through its
actual Root output destruction, including after an IPC observer disappears.
Immediately before responding, the actual UI commit repeats original generation,
authorized generation, lock-request and original output-negative checks. Whole
Root output/payment stays outside that commit even if the responder panics.

Shared Legacy save, lock cleanup and unlock finalization acquire the same lane
before UI commit. This ordering is preparatory: native lock snapshots do not yet
use the Root sidecar operation, and the disabled path must not be activated at
this intermediate checkpoint. Authentication holds vault_mount while waiting for
the lane; ordinary workers must never acquire vault_mount after acquiring it.

## Missing and malformed records

ServerStore::load_ui_state_record returns None only for a strict NotFound from
symlink_metadata. A present authenticated empty record, empty physical file,
nonregular path, dangling link, invalid UTF-8/JSON or unsupported schema is refused
and preserved. Only a missing record receives the existing first-run defaults.
The old load_ui_state wrapper retains its compatibility behavior for old callers;
the actual native and Legacy get_ui_state consumers use the tagged reader.

## Verification and remaining work

The final eight Windows Root UI-state cases passed (10.51 seconds), including
three physical-slot cases, actual uncertain index WAL progress and the strict
reader's interrupted/oversize input cases. The corresponding Linux dangling-link
case remains for CI. All24 actual native routing/lifecycle tests passed after the
review fixes (50.67 seconds); five exercise UI-state, including a serialized read
after real lock registration, observer loss while the real lock waits for IO, and
actual responder panic with output retirement outside commit.

Full frontend3441 tests, Svelte check (zero errors/warnings), web build and both
workspace formatting checks passed. Ambient/store raw-fs and exact-count guard
self-tests passed. Compiled inventory is232 native-registry tests, exactly two
ignored artifact generators and230 normal cases; the dedicated predecessor gate
still runs those generators explicitly. Read-only design/actual-diff re-review is
clear after fixing both deterministic race observations and panic-safe cleanup.

Strict native all-target/all-feature Clippy, the separate native compile check,
both shipping dependency audits, actual Windows build (3m29s), built-binary
startup and browser flows passed. Startup verified the same SHA256 binary:
601A380648855F6DCC9F2F1F59DA742BA51F0DF658170DA4506F4409962084E9.
Root strict Clippy was run and still fails with582 errors in the unfinished App
pipeline; no suppression or passing sign-off is claimed.

Complete Root/native CI was dispatched at runtime commit0ba8cb08:
[run43588313636](https://github.com/Thalpy/Mewtual/actions/runs/43588313636).
It remains pending. Only the previously named slow catch-up cost case remains
deferred until final acceptance. This checkpoint is not ready to merge.

The next implementation must capture each final snapshot's original authenticated
mount before scheduling, retain its exact bytes and unique credit through lost
waiters, retry only on that same mount, and record outcomes before releasing the
IO lane. Newer capture failures must not be erased by older save successes.
Native authentication must settle this continuity outside UI commit before
publishing authorization. Creator/member operations, cold operational restore,
joined source/archive, transport/projection/admin and multi-peer acceptance also
remain unfinished.
