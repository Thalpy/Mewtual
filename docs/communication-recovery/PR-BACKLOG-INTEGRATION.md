# PR backlog integration — 27 October 2026

[PR #74](https://github.com/Thalpy/Mewtual/pull/74) is a draft integration candidate,
not a completed communication programme. Runtime source at the initial combined checkpoint:
`2babf97e3162de2bb4212ad044c76c0c6e15f8d3`. Current runtime/configuration checkpoint:
`c4aa9064c078440652e69ddb7aaac4346d2f041c`, followed by the fixture-only corrections
below. Earlier results do not validate the changed native graph.

## Landed and prepared work

- PR #26 merged into main at `701da398`. It included all PR #54 commits, and GitHub
  marked #54 merged. The merge rehearsal differed from the tested Studio source only
  in the pending-action document and toolchain comments. Its historical green audit
  predates newer advisories; it does not establish present dependency safety.
- PR #57 merged into `chat-update-implementation` at `0cf203f2`, after the documented
  #26/#54 trigger was satisfied. Its original head `bace1bf3` had 28 passing hosted checks.
- The candidate preserves the original upload repairs `c4005b89` and `985c1f1d` together.
  Main's accepted older foundation contains the upload/lock race until this repair lands.
  The independent actor-event fence prevents a bounded event-sink/publication deadlock.
- The candidate includes the reviewed recovery/V2 stack through #72, both newer Gate7
  changes, and the scoped MLS admission/commit-log fixes #60/#62. Original history is
  preserved. The known-failing three-peer test from #73 is excluded; its reviewed design
  requirements are retained in [the admission handover](VOICE-MULTIPEER-ADMISSION-NEXT.md).
- PR #15 is also included: current connectivity formatting moves into its dedicated
  module, preserving trace correlation, ordering, privacy copy and all compatibility exports.
  Its 15 focused tests and read-only integration review pass.

The send merge preserves both branches' contracts. A native capacity/document-pending
refusal proves this attempt unaccepted after its pending entry is resolved. It cannot
erase possible acceptance of an earlier attempt with the same identity. Tests exercise
both facts through the actual App functions and kept/recovered/cancelled entries.

## Current combined evidence

| Check | Result at the initial combined source |
| --- | --- |
| Actual-App send/retry/recovery focused tests | 28 passed, including the combined ambiguity/refusal cases |
| Complete frontend suite | 3,201 passed; 74.29 seconds |
| Svelte check / production build | Passed; Vite build 37.66 seconds |
| Rendered Edge flows | All 10 scenarios passed with host browser access; the initial sandbox CDP failure ran no scenarios |
| Focused MLS crate | 67 passed; 1 existing ignored subprocess helper |
| Root / native formatting | Both passed |
| Root dependency audit | Passed against the current advisory database |
| Separate native dependency audit | Initially failed; corrected at the native follow-up below |
| Fresh complete Root / native suites, strict Clippy, native check/build/startup, ambient gate | Pending; older checkpoint results do not validate this combination |

On the final frontend inputs, the repeated full suite passes 3,201 tests in 64.90 seconds;
check, build and all ten rendered flows pass. Both formatting checks pass. The new native
dependency graph and policy pass both audits, including all four categories. Fourteen
upload/session regressions passed before the native dependency follow-up. The WMI-only
Windows check passes after its ABI correction. Six actual native authority/parser
regressions, strict native Clippy and the separate native check also pass on the new
graph. The first native run was interrupted to correct the fixture ordering below;
its partial result is not a full-suite pass. The fresh local Windows `121ad952` native run passes all
899 library tests in 2,186.57 seconds and all six ACL/parser tests. Fresh native formatting
and strict Clippy pass too. Native build and real application startup also pass; the app
was not unlocked. Hosted Linux native validation later fails at the separate observer
boundaries below. Fresh Root validation remains required. The intermediate `8e2e7aaf`
Linux desktop job `122422109228` passes 3,201 frontend tests, Svelte checks, all 899
native library tests in 2,700.78 seconds, six ACL/parser tests, native check and strict
Clippy. That earlier pass does not validate the later native fixture edits or clear
`04e20ed3`'s failure.
Through `121ad952`, Root Rust/Cargo/toolchain inputs are byte-identical to the initial
combined source tested by hosted Windows/Linux jobs. The Root observer change below
requires new execution; the earlier Root result cannot validate it. Runtime manifests record
1,477 file hashes; documentation updates do not change those inputs.

Read-only reviews of the actual merge and neighboring enforcement found no new blocker,
high or medium in the scoped send/upload, MLS/dependency and native custody boundaries.
The stale commit-refusal comment was corrected. These reviews are not a fresh audit of
every inherited file. The new full native run includes its vectors/layout guards; Root
validation still needs completion.

The separate completed `05c97adb` checkpoint passed 897 native library plus 5 ACL tests,
3,691 Root tests with 24 existing ignores, and 3,200 frontend tests. Its native run used a
forced rebuild and checked test inventory; its Root/native input files were byte-verified.
Those results establish the older Rust/dependency checkpoint only.

## Native audit remediation

The initial native audit independently found `h2` 0.4.15 (RUSTSEC-2026-0258; patched in 0.4.16),
`zip` 4.6.1 through `tauri-plugin-updater` (RUSTSEC-2026-0367/-0368/-0369), and six
unmaintained proc-macro-error/UNIC advisories. The follow-up at `c4aa9064` updates H2,
removes unused ZIP extraction, updates the Tauri utility/parser graph to remove UNIC,
and replaces a newly yanked native arithmetic release. Both audits now pass. One new
build-only maintenance decision has exact stable-version/parent fences and tested
positive/negative controls. The full native suite and all six actual native
authority/parser regressions pass on this changed graph, as do build/startup. See
[NATIVE-DEPENDENCY-AUDIT](NATIVE-DEPENDENCY-AUDIT.md) for the source analysis and limits.

The upgrade's existing crypto-bigint exception remains the reviewed, constrained policy:
native secret P-256 uses 0.14; the old MLS arithmetic is fenced behind the single
X25519/Ed25519 suite and dependency bans. This is not removal of every old crate.
New OpenMLS 0.9-created/joined group records do not safely downgrade to older builds.
Use isolated validation profiles; the compatibility limits are in
[the upgrade design](../design-crypto-bigint-upgrade.md).

## Root crossing-range observer failure

The initial combined Linux Root job `122321586207` failed in
`accepted_crossing_range_chunk_renews_quiet_deadline_before_fetching_the_next_chunk`:
the App library reported 1,257 passed, one failed and 23 ignored. Its generic five-second
observer does not identify whether setup, transport admission/completion, the range
result or shutdown stalled. The workspace command stopped at that crate; this is not a
complete Root pass. The failure's production cause has not been established.

Inspection confirms the intended cached-chunk acceptance renews the quiet deadline
synchronously before the second request is prepared. The next two-millisecond clock
advance should leave both renewed quiet and attempt deadlines current. No production
deadline or cancellation change is justified by the available log.

The diagnostic follow-up labels the same five-second bounds, awaits an actual FIFO
holder command before advancing its shared clock, and checks the real network step's
chunk and renewed deadline. A fixed-count observation and immediate completion checks
distinguish a pending retry from an accepted range result. CI runs the focused actor
module before the complete Root suite for earlier feedback; it still runs the full gate.
Read-only review found no remaining actionable findings. The following watchdog
correction and execution narrow the failure; they do not establish the historical cause.

The next focused Windows job (`122398330559`, `ec4e63f8`) identifies **pair setup** as
the failing stage: 46 passed, one failed. It fails before the range, storage hook or
renewal starts. That aggregate observer includes sealing a real 8 MiB chunk and making
MLS identities, as well as Join/catch-up. This identifies the stage of this newer failure;
it does not establish whether local preparation or networking consumed its time, nor
prove the earlier Linux failure had the same cause.

The fixture contract is now explicit: fixed local cryptographic preparation is not a
five-second performance gate. After that preparation, membership, file subscription
and catch-up share **one unchanged five-second watchdog**, with phase-specific failures
and timeout priority. It is never restarted between those stages. Concrete MemNetwork
preparation performs local publication/subscription; any future peer wait must stay
under the watchdog. The small-data helper's aggregate five-second bound remains. A new
negative fixture withholds actual holder Join service and requires the watchdog's exact
rejection. All range/currentness, Lower custody and production timing remain unchanged.
The corrected range test passes in 4.80 seconds. Removing only its production renewal
fails at the intended currentness assertion in 4.72 seconds; production source was
restored byte-for-byte with a fresh timestamp. The final Clock-based fixture rebuild
passes all 48 file-transfer module tests in 20.98 seconds, including the unserved Join.
Root formatting, strict Clippy and the ambient gate pass. Complete Root execution remains
required in hosted CI; the local full run was interrupted to serialize the new native
diagnosis and is not counted as a pass. The earlier unchanged Root inputs passed 3,726
tests with 25 existing ignores on hosted Windows, while Linux had the failure above.

## Shared expiry-clock fixture ordering

The initial combined Linux native job failed first in
`native_v2_pc_intent_original_http_begin_history_return_and_premature_high_refusal`:
the original SDP intent was already unauthorized before Begin. Thirteen later failures
were the fixture's fail-closed admission gate, not independent diagnoses. The old log
does not establish which endpoint advanced the clock, so historical causation remains
unproven.

Inspection confirmed a possible fixture race: both endpoints share one expiry clock,
and HIGH's negative expiry probe could advance it before LOW finished positive work.
The correction deliberately parks LOW before its first Begin, proves HIGH is waiting,
then allows LOW's genuine Input/provider/worker path to finish before advancing time.
One retained barrier future avoids counting HIGH twice. Every wait uses the original
intent deadline; no timeout, attempt, history or production authority is renewed.
Additional diagnostics contain only fixed roles/stages, times and atomic observations.

The corrected actual test passes 1/1 in 10.75 seconds. A missing-await mutation fails
at the intended ordering invariant, 0/1 in 10.01 seconds; the correct source was restored
byte-for-byte with a fresh timestamp and rebuilt before the positive run. A read-only
review of the actual restored diff found no actionable findings. The complete local
native suite now passes 899 library and six ACL tests, including this case. Hosted Linux
at `8e2e7aaf` also passes the complete native suite including this correction. The later
observer failures below are separate; the original focused pass alone did not establish
this full result.

## Later Linux native Begin-reply failure

At `121ad952`, Linux job `122376872569` reports 880 native library tests passed and
19 failed. One real failure is
`native_v2_original_sdp_result_uses_actual_actor_and_survives_eager_cancellation_lock_and_history_retries`:
`confirmation::outgoing` authenticated a three-byte reply, then blindly required a
242-byte status. The remaining 18 failures are the closed fixture admission gate.
The reply code was not logged, so its historical refusal cause remains unknown.

Begin legitimately returns three-byte Closed, Stopped or Protocol refusals, including
Capacity under transient local contention. The fixture must distinguish them, retain
the same channel/parent/purpose/body and original deadline, and retry only exact Capacity.
Capacity can also follow reservation/publication: it never proves no attempt occurred
and must not trigger a refund, replacement or renewed authority. Local Windows success
does not clear this Linux failure.

The corrected ordinary outgoing/incoming observer retains the actual Derive deadline
before Return in a fixture-only wrapper beside the real row. No production row, layout
or public authority API is added. One watchdog covers bootstrap, guard waits, authenticated
attempts and sleeps. The same setup, channel, parent, purpose/predecessor and body survive
each retry, and each actual reply is collected before observing the next sequence.
Only exact authenticated `[1,3,7]` Capacity retries; other refusals fail immediately.

The regression takes the real UI commit then server-map guard. It snapshots whichever
exact child/history exists, allowing native maintenance to have won earlier. The forced
Capacity goes through retry classification, proves that snapshot unchanged, then releases
both guards before retrying. A real authenticated wrong-selector refusal submits once;
an exhausted original bound submits nothing and does not advance the channel counter.
The test passes in 11.89 seconds; removing Capacity classification fails at the forced
`[1,3,7]` refusal in 6.87 seconds. Exact source restoration with a fresh timestamp is
hash-verified, then the rebuilt test passes in 10.50 seconds. Re-review resolves the
maintenance ordering finding with no remaining actionable findings. Native format,
strict Clippy and check pass. The `04e20ed3` complete local native rerun passes all
899 library tests in 2,210.67 seconds and all six ACL/parser tests. Both full dependency
audits pass on that exact unchanged graph. Linux found the separate pure-observer
failure below; local Windows success does not clear it.

This corrects a confirmed fixture assumption without attributing the historical refusal
to Capacity. The hosted Linux rerun is still required. Dedicated known-Incoming/attachment
negative-history probes retain their previous observer windows; this change converts
ordinary Begin observation only.

## Pure SDP/media observer contention

At `04e20ed3`, Linux job `122436188831` reports 872 library tests passed and 27
failed. Its first real failure is
`native_ready_is_automatic_over_blocked_tcp_member_bridge_after_browser_confirmation_returns`:
`sdp_test_phase` unwraps `Capacity` from the ledger's nonblocking Index acquisition.
The other 26 failures are the closed native fixture admission gate.

Production final admission intentionally refuses lock contention after sampling its
Clock; waiting could issue a stale deadline. Seven **test-only**, pure SDP/media
snapshot methods incorrectly reused that acquisition and unwrapped its result. Their
correction waits only in the observer path, using the existing validated blocking
accessor. After waiting it rechecks slot, serial and actual Row allocation. A removed
row remains `Unauthorized`, never a successful absent/drained snapshot. Production
admission, deadlines, authorization and wire/persistence formats are unchanged.

The deterministic regression holds the actual Index, observes a real contention refusal
on a worker, then releases it with either the original row retained or removed. It also
checks production still refuses contention and that resource counts do not change.
Channel handshakes establish ordering; five-second bounds are failure watchdogs only.
The lock-owning thread releases its guard before joining even if the production
acquisition regresses to blocking. The new regression passes; removing only the
contention fallback fails at its intended pending-row assertion. Restoration is
SHA-256 verified with a fresh timestamp, and the rebuilt regression passes again.
The exact previously failing Ready real-TCP scenario passes in 18.46 seconds. Native
formatting passes. Read-only review found one watchdog-cleanup weakness in the test;
the corrected worker ordering has no remaining actionable findings after re-review.
Strict Clippy/check and the fresh complete native suite are in progress. A fresh
hosted Linux pass is required before acceptance.

## Held PRs and remaining implementation

- #51: current native six-client convergence failure; its old all-green description is
  superseded by the current-head merge hold.
- #55: HIGH stale cache-limit command across lock/account/server replacement or leave.
- #56: inherited #55 HIGH and MEDIUM saved-metadata mutations after their generation
  guard is dropped. Both PR descriptions record the required barrier tests and fixes.
- #47: independent Score work with unresolved integration conflicts.
- #73: deliberately failing automatic three-peer reproduction. Do not merge its test
  as a green checkpoint or weaken the original deadlines to hide the failure.

App V2 remains OFF. The Source already enforces the bounded audio SDP shape before
HIGH's PC constructor and checks local descriptions. Runtime receiver/direction checks,
paid audio cells, capture/playback wiring, App lifecycle/renewal, automatic multi-peer
admission, member media relay and combined/physical acceptance remain unfinished.
Only one physical desktop is available. Native Current is not a separate server-enforced
speaking/listening/moderator-mute grant.
