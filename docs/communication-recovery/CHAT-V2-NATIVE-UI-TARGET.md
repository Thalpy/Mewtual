# Original authenticated mount for native UI state

Status: target-only implementation under validation. Chat V2 remains off. Native
lock snapshot custody and authentication retry are still unfinished; this change
must not be described as complete continuity or working text messaging.

## Identity is not authorization

RT's NativeChatUiMountObservation::same_original compares the original Host Arc.
It creates no identity and does not compare directory names, device labels or UI
counter values. Distinct Hosts can share every UI atomic. UI lock/reauthentication
preserves the same Host, whereas another mounted owner has a different one.

Root's opaque NativeTextVaultAuthenticationCheck adds original comparison and
is_for_original_vault. Both retain the existing negative-only contract: current
ACL, secret verification and generation admission remain independent. The private
selected3 facade never transplants its Host into a different physical index.
Its owning upgrade transfers the whole pairing once; version2 cannot authenticate.
Full original Gate/Life/Host negatives are still required.

prepare_ui_state_for_original is a distinct infallible owning capture. It retains
the actual checked target Arc and unique IO credit, including foreign/closed/
Busy refusals, until outside Store/commit guards. It checks the original target
before IO and carries it through Read/Saved output publication. A wrong live
origin therefore cannot write merely because the actual mounted vault is open.
The old unbound library DATA API remains; native ordinary commands never use it.
This logical-mount check does not add filesystem-path replacement protection.

## Native capture and publication

Successful native authentication creates the check Arc outside guards. Before
waiting for the IO lane or UI commit, the finalizer chooses its canonical target.
A live same-original target keeps its existing allocation across reauthentication.
This preserves references held by later continuity work without adopting another
mount by labels or by whichever wrapper Arc happened to be newest.

At the real UI -> resumable -> short Store edge, finalization compares the fresh
check with the actual mounted facade. This precedes resumable=true and voice
activation, so a changed/absent mount cannot bypass rollback through an early
error. Only then can the existing authorization branch install the prebuilt
canonical target before publishing the authorized generation. A replaced whole
check moves into an outer variable; payment/destruction stays outside guards.

The synchronous main-window ACL router captures both generation and canonical
target before parsing or scheduling ordinary UI reads/saves. No renderer field
selects a target. Missing/foreign state never falls back to the unbound Root API.
Generation checks remain mandatory even when the original mount is unchanged.

## Validation and boundaries

All27 RT key-lifecycle tests passed. The shared target initially reused an older
worktree RT artifact; that failed compilation was preserved, the changed RT
package explicitly rebuilt, and both original-mount cases plus the full key
cohort then passed. The actual Root live-foreign-target regression passed
(9.31 seconds): valid current IO funding, two live vaults, refusal before RNG/IO,
unchanged ciphertext and a genuine own-target positive control.

All27 native routing/lifecycle cases passed (60.73 seconds), including canonical
same-root reuse, live foreign target refusal and absent/foreign Store refusal
before authorization mutations. An additional actual unpolled-command test passed
(8.33 seconds): both vaults remain live with equal numeric counters, the queued
old request refuses without changing either ciphertext, a fresh replacement-target
request succeeds, and the original owner/target can then be restored. This pins
capture before scheduling rather than whichever target exists at first poll.

Full frontend3441 tests, Svelte check (zero errors/warnings), web build, both
workspace formatting checks, ambient/store raw-fs and exact-count self-tests
passed. Actual-diff read-only review, including the last test, is clear across
B/H/M/L. The native-registry normal cohort grows230->231 and RT key cohort26->27;
the two dedicated predecessor artifact generators are unchanged.

Strict native all-target/all-feature Clippy, separate native check, both shipping
dependency audits, Windows build (3m29s), actual startup and browser flows passed.
Root strict Clippy was run and still fails with582 App errors in the unfinished
pipeline; no suppression or passing sign-off is claimed.

Full Root/native execution is queued at runtime commitcc5e6d21:
[run43591075301](https://github.com/Thalpy/Mewtual/actions/runs/43591075301).
It supersedes the older ordinary-IO-only run. The single named slow catch-up
cost case remains deferred; no other acceptance regression was skipped.
This remains a draft, not merge-ready, and does not establish complete lock
snapshot custody or retry.

Still required: pre-schedule lock/close snapshot capture from an authorized UI,
freshness bound to the first actual SC predecessor, native custody through lost
waiters/panics, bounded control and payload queues, exact original-target retry,
latest-attempt outcome ordering and continuity settlement before authorization.
SourceRegistry/device/transport/creator/member ownership, operational cold restore,
joined source/archive, transport/projection/admin and multi-peer acceptance also
remain open.
