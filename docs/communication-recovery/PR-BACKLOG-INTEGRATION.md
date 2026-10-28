# PR backlog integration — 27 October 2026

[PR #74](https://github.com/Thalpy/Mewtual/pull/74) consolidates the reviewed recovery
changes and inactive V2 checkpoints while preserving their original history. Tested
code checkpoint: `c20a64d0a023758cad5a255532d384cc66cd963f`. Runtime/configuration is
`c4aa9064c078440652e69ddb7aaac4346d2f041c`, followed by fixture, CI and documentation
corrections. The initial combined source was `2babf97e3162de2bb4212ad044c76c0c6e15f8d3`.
App V2 remains OFF; the unfinished programme work is listed below.

28 October status: #74 merged into `main` at `85b345cf`; its tree matches the tested
integration head. The incorporated stacked PRs were closed with their source history
preserved. Seven PRs remain open: #47, #51, #55, #56, #73 and the two draft integration
candidates #75/#76. #75 (`communication-gap-completion`, runtime `3e598b50`) preserves
#51/#55/#56 on that main and fixes native Lock/Leave write and reveal races. #76
(`score-backlog-integration`, tested runtime `fa3107d2`) adds #47 while preserving Save-before-switch,
and includes the latest #75 diagnostics. It is stacked on #75. Close the original
feature PRs only after their integration candidate passes and lands.

Current local Score evidence: the full Root suite at `c27e7694` passes 3,960 tests
with 25 existing ignores; compiled Root sources/locks are unchanged through `fa3107d2`.
Its current-native-source construction scan was separately rerun after the corrections and passes.
Complete native passes 1,011 library plus six ACL tests (2,336.29s library runtime),
including both previous causal failures and both original/alternate media cases.
Complete frontend passes 3,539 tests, all 379 native handoff checks pass, and all four
actual native batch/reopen/lock/lost-reply regressions pass (1.57s). Both refreshed
dependency audits and ambient/Store raw-fs gates pass. Native check, strict Clippy,
final build (5m59s) and real startup pass. Hosted CI remains pending.

#75 also passes its focused storage controls, the original media scenario (77.33s)
and a new forced-alternate-peer media scenario (77.54s), plus native check/strict
Clippy and 3,314 frontend tests. Linux's earlier `c3877704` native run still has an
unresolved second-pair storage Capacity failure after genuine confirmation/Ready.
The new exact-parent/boundary diagnostics are running in current CI. Local passes
do not explain that earlier refusal. The overlay job at `3e598b50` passed all seven
negative/restored pairs but was cancelled at its 30-minute aggregate limit; its
follow-up allows 45 minutes with no case-bound or assertion change. That cancelled
job is not green. Neither draft is ready to merge until hosted evidence is reconciled. Historical
passes below must not be reused as acceptance of these newly combined features.

The separate #74 store-repair follow-up is now complete on both hosted OSes at
`91d29aa1`: [Linux](https://github.com/Thalpy/Mewtual/actions/runs/40604428160/job/122540272407)
and [Windows](https://github.com/Thalpy/Mewtual/actions/runs/40604428160/job/122540272656)
both pass all 22 negative/restored pairs. This supersedes the partial Windows evidence
below for that unchanged store checkpoint. The earlier cancelled 90-minute job is
still not a pass, and these results do not validate the new #51/#55/#56 combination.

## Landed and prepared work

- PR #26 merged into main at `701da398`. It included all PR #54 commits, and GitHub
  marked #54 merged. The merge rehearsal differed from the tested Studio source only
  in the pending-action document and toolchain comments. Its historical green audit
  predates newer advisories; it does not establish present dependency safety.
- PR #57 merged into `chat-update-implementation` at `0cf203f2`, after the documented
  #26/#54 trigger was satisfied. Its original head `bace1bf3` had 28 passing hosted checks.
- The integration preserves the original upload repairs `c4005b89` and `985c1f1d` together.
  These correct the older foundation's upload/lock race with session-bound final admission.
  The independent actor-event fence prevents a bounded event-sink/publication deadlock.
- The integration includes the reviewed recovery/V2 stack through #72, both newer Gate7
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

## Merged #74 evidence

| Check | Latest verified result and source |
| --- | --- |
| `cargo test --all --all-features`, Windows | 3,727 passed, 25 existing ignores at `8e2e7aaf`; excludes the separate 48-test focused run |
| Complete Root suite, Linux | 3,733 passed, 26 existing ignores at `8e2e7aaf`; excludes the separate 48-test focused run |
| Root construction scan on current native source | 1/1 passed at `c20a64d0`; separate rerun, not added to the full-suite total |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`, Windows | 900 library plus 6 ACL/parser tests passed at `c20a64d0`; library runtime 2,214.12 seconds |
| Complete native suite, Linux | 900 library plus 6 ACL/parser tests passed at `c20a64d0`; library runtime 2,260.41 seconds |
| `npm --prefix apps/desktop test` | 3,201 passed locally and again in hosted Linux at `c20a64d0` |
| Frontend check / production build | Passed on unchanged frontend inputs; hosted Linux Svelte check has zero errors/warnings |
| `test:flows` | All 10 rendered Edge scenarios passed; initial sandbox CDP failure ran no scenarios |
| `test:startup` / native build | Passed on unchanged production inputs; actual app opened and was not unlocked |
| Actual-App send/retry/recovery / connectivity | 28 / 15 passed, including combined ambiguity/refusal cases |
| Focused MLS crate | 67 passed; 1 existing ignored subprocess helper |
| Root / native formatting and strict Clippy | Passed; includes fresh Windows/Linux native Clippy at `c20a64d0` |
| `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` | Passed on Windows and Linux at `c20a64d0` |
| Root / separate native `cargo deny check` | Both pass all four categories on the current graph/policy |
| `scripts/check-no-ambient.sh` / `scripts/check-store-raw-fs.sh` | Both passed at `c20a64d0` |
| WMI-only Windows compatibility check | Passed locally and in hosted Windows Root job |
| Store mutation controls, Linux | All 22 exact negative assertions and 22 restored controls pass in hosted CI at `c20a64d0` |
| Store mutation controls, Windows | All 22 pairs covered: 19 in the interrupted hosted run, then 3 locally on the same source; hosted job itself is cancelled, not green |

The [Windows Root job](https://github.com/Thalpy/Mewtual/actions/runs/40570215597/job/122422109731)
and [Linux Root job](https://github.com/Thalpy/Mewtual/actions/runs/40570215597/job/122422109595)
execute the complete command after their focused modules; each has 59 full-command
result groups. Both jobs, including their remaining platform gates, complete successfully. The
[current Linux desktop job](https://github.com/Thalpy/Mewtual/actions/runs/40584438347/job/122472061929)
passes the complete frontend/native checks and strict Clippy. Unlike the older native
checkpoints, both current native runs include the new contention regression and all
three corrected fixture boundaries below.

Root Rust/Cargo/toolchain/scripts/workflow source files are byte-identical from
`8e2e7aaf` through `c20a64d0`. One Root integration test also scans native Rust source;
its changed scan input is covered by a fresh `cargo test --locked -p catcoms-replication
--all-features --test construction_gates -j1` run: 1/1 passed in 0.40 seconds. Other
cross-workspace fixture inputs are unchanged. Frontend inputs are byte-identical to
the fully tested `c4aa9064` inputs. Runtime manifests verify 1,477 hashes; only four
native fixture files changed
from `04e20ed3` to `c20a64d0`, and all remained unchanged through the full native run.
Production build/startup inputs retain the same runtime behavior: subsequent Rust
changes are confined to `cfg(test)`. The final follow-up changes documentation and the
aggregate CI job allowance only. It changes no target source, dependencies, features,
compiler flags, assertions or test deadlines. Full suites are not repeated for those
metadata changes; the recorded code inputs and their actual results remain pinned.
After both full `8e2e7aaf` Root jobs and the current source scan passed, the duplicate
`c20a64d0` Root jobs were cancelled. Those cancelled runs are not counted as passes;
the completed `c20a64d0` Linux desktop and supply-chain jobs remain green.

Interrupted local Root/native runs are not counted as passes. Earlier native results
at `121ad952` and `04e20ed3` passed 899 library plus six ACL tests locally; the
intermediate `8e2e7aaf` Linux run also passed 899 plus six. Those results did not clear
the later Linux fixture failure. The current `c20a64d0` Linux pass does.

Read-only reviews of the actual merge and neighboring enforcement found no new blocker,
high or medium in the scoped send/upload, MLS/dependency and native custody boundaries.
The stale commit-refusal comment was corrected. These reviews are not a fresh audit of
every inherited file. The final observer review's low watchdog-cleanup finding was fixed
and re-reviewed with no remaining actionable findings. Full native runs include the
vectors/layout guards and original session-bound upload regressions.

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
Root formatting, strict Clippy and the ambient gate pass. The complete Root suites pass
3,727 tests with 25 existing ignores on Windows and 3,733 with 26 on Linux at `8e2e7aaf`;
the changed native-source scan is separately covered at `c20a64d0` as recorded above.
The local full run was interrupted to serialize native diagnosis
and is not counted as a pass. The earlier Root inputs passed 3,726 tests with 25 ignores
on hosted Windows; that older result did not validate this fixture correction.

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
to Capacity. Hosted Linux at `c20a64d0` now passes the complete native suite including
this correction. Dedicated known-Incoming/attachment
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
Fresh complete native suites at `c20a64d0` pass all 900 library and six ACL/parser
tests on Windows and Linux, including this regression and Ready scenario. Native
format, strict Clippy and check pass too. The current Linux result clears the old
observer failure without changing production admission or renewing any deadline.

## Windows store mutation build allowance

The Linux store job at `c20a64d0` passes all 22 negative/restored pairs. The
[Windows job](https://github.com/Thalpy/Mewtual/actions/runs/40584438220/job/122472061324)
hits its 90-minute aggregate allowance while compiling the twentieth mutant. Its
preserved artifact `12779197118` contains 38 logs: 19 exact intended failures and
19 passing restored controls, all independently checked against the unchanged harness.
The first build takes 10m47s; later pairs spend about four minutes compiling, with
the last completed tests taking 1.09/1.12 seconds. This is not a successful hosted job.

The remaining three pairs run locally on the same `c20a64d0` source through the
unchanged runner, with the same `RUSTFLAGS=-D warnings`, disabled incremental build,
`profile.test.package.catcoms-app.debug=0`, serial Cargo command and 900-second
command watchdog. Each fails at its intended assertion and passes after byte-exact
restoration: `REPAIR-current-admission`, `REPAIR-answered-report` and
`REPAIR-b0-before-source-refusal`. All three final source hashes match the pinned
inputs. Together with the 19 preserved hosted pairs, this completes Windows per-entry
coverage; it does not turn the interrupted hosted run into a pass.

The reviewed workflow correction gives the complete serial build workload 120 minutes.
All 22 entries, serial execution, failure/restoration checks, command watchdogs and
protocol/test deadlines remain unchanged. A subsequent complete automated run must
pass before that workflow can be described as green at the new CI allowance.

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
