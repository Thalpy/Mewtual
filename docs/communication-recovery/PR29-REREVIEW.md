# PR #29: close transaction and CI follow-up

Review baseline: `d8f72ce84ae50b8cc1644949088731814be1c638`. Date: 2026-09-28.
The supplied re-review closes R1, R2 and the specific linear-history R4 counterexample.
It identifies R5, a successful pending-message decision overwritten by an older
window-close snapshot, and requests resolution or attribution of the red CI run.

## Independent changes

| Commit | Boundary and outcome |
| --- | --- |
| `4db2a5a` | Close fences new work, drains admitted saves through their memory commits, then captures fresh continuity. Failure preserves unresolved work; timeout aborts close. |
| `f859083` | Four historical Studio succession fixtures start explicitly as legacy groups. A separate restored-P2P regression keeps the immutable policy pin and verifies admission refusal without state changes. |
| `a35f2e8` | Owner-return scenarios use separate simulated-process preparation domains; actors inside a scenario still share the same bounded capacity. Production pools and timing limits are unchanged. |
| `d5e2e21` | Strict native builds omit an unwired test-only security-intent model and a test-only MIME helper. Tests assert the approval model's disclosure and expiry fields. No registered runtime authorization is removed. |
| `c2d4cb4` | Both lockfiles upgrade rustls 0.23.40 to 0.23.45 and rustls-webpki 0.103.13 to 0.103.15. No manifests or advisory exceptions change. |
| `583e017` | Native Clippy accepts the existing flat eight-field send command through a function-scoped annotation; its payload and retry checks are unchanged. |
| `aa5f633` | Native decode deadlines and existing worker polling use the approved Clock seam. The ambient gate's allowlist is unchanged. |
| `084fda0` | A blocking image worker owns its capacity permit until it actually exits; timeout, cancellation and panic cannot prematurely free a running worker's slot. |
| `990c450` | The native discovery cadence is shared with forthcoming transport acceptance fixtures, with unchanged jitter, network-change wake and one-pass-at-a-time behavior. |

The dependency update addresses [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html).
Upstream TLS validation tightens; the application's protocol, configured provider,
and security fallback behavior are unchanged. This is not a claim of unchanged TLS behavior.

## What the failing CI actually showed

[Run 36328899735](https://github.com/Thalpy/Mewtual/actions/runs/36328899735)
was inspected at the reviewed SHA, including all four failed job logs:

| Job | Observed failure | Attribution / correction |
| --- | --- | --- |
| Linux frontend & Tauri, `108646903803` | Strict native unused-code warnings after frontend checks | `d5e2e21` |
| cargo-deny, `108646903839` | rustls advisory RUSTSEC-2026-0285 | `c2d4cb4`; no suppression |
| Windows, `108646903914` | Four succession admission refusals and three owner-return failures | Policy fixture correction and bounded fixture isolation |
| Ubuntu, `108646903973` | Four succession admission refusals and two owner-return failures | Same boundaries; not labelled six new regressions |

Raw logs are retained locally as `logs/pr29-ci-<job-id>.log`. The succession fixture
is byte-identical at `48f9069`, `f77c82c`, and `d8f72ce`; the policy contract changed
earlier in this PR at `37c6e2f`. See [POLICY.md](POLICY.md) for the exact boundary.
The owner-return fixture and production scheduling code were likewise unchanged
over those review revisions; earlier failures are recorded in the Gate 4 status.
[OWNER-RETURN-CI.md](OWNER-RETURN-CI.md) records the mechanism and executed mutation.
Source attribution does not assert that those complete older baselines ran locally.

## Executed checks

At `c2d4cb4` on Windows, using locked/offline Cargo, one Cargo job, disabled debug
information/incremental compilation, and a shared target directory:

| Check | Result |
| --- | --- |
| R5 actual registered close callback on old source | Cancel and recover regressions fail because native close is called before the decision settles |
| Focused frontend close/send/continuity suites | 58 passed |
| `npm.cmd test` | 1,277 passed, 0 failed |
| `npm.cmd run check` | 0 errors, 0 warnings |
| `npm.cmd run build` | Passed; existing chunk-size warning |
| Four legacy succession cases, restored-P2P refusal, shared preparation custody | 6 passed, 0 failed |
| Three owner-return cases in parallel | 3 passed, 0 failed |
| Owner-return with all unrelated production slots occupied | 1 passed; removing isolation fails the intended assertion; exact restoration passes |
| Native `cargo test --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --lib -j 1 -- --test-threads=2` | 288 passed, 0 failed, 0 ignored; 121.41 seconds, at `c2d4cb4` plus the annotation committed as `583e017` |
| Native `cargo clippy --locked --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --tests -j 1 -- -D warnings` | Passed with that annotation |

Frontend outputs: `logs/pr29-r5-ui-tests.log`, `logs/pr29-r5-ui-check.log`, and
`logs/pr29-r5-ui-build.log`. Rust evidence is listed in the owner-return report.
An earlier local attempt ran out of disk before testing; generated artifacts alone
were removed. It is not passing evidence. The later missing-qualification compile
failure was corrected before the recorded green checks.
Native outputs: `logs/pr29-native-tests.log` and `logs/pr29-native-clippy-final.log`.
The initial stricter native Clippy run found the existing flat send command's argument-count
lint; its narrow correction was independently reviewed before the passing rerun.

[CI run 36404695407](https://github.com/Thalpy/Mewtual/actions/runs/36404695407)
at `c2d4cb4` subsequently passed cargo-deny and Linux frontend/Tauri. Windows formatting
and Clippy passed, followed by 727 passing app tests, one failing app test, and 13 ignored.
All seven previously reported Windows app failures passed. That run's remaining failure was
`actor::tests::a_throttled_delivery_receipt_gets_an_injected_clock_wake`, at the final
timer-wake assertion. The six-client follow-up anchors its wake to the original sampled
monotonic deadline and rotates ready actor sources. The original regression and three
deterministic clock-arming cases now pass locally; no timeout was increased. See
[ACTOR-FAIRNESS.md](ACTOR-FAIRNESS.md) for the exact scope and evidence.
The raw log is `logs/pr29-ci-108870498092.log`. Ubuntu's test step was still running
when these results were recorded.

[CI run 36425464902](https://github.com/Thalpy/Mewtual/actions/runs/36425464902)
at `255817f` passed formatting and cargo-deny but failed strict core Clippy on three
redundant slice borrows in the extracted response helpers. `1912b34` removes those
borrows; strict sync/app and native Clippy pass locally at that revision. Linux desktop
also exposed a command-inventory test parser that skipped `send_message` because a
comment sits between its command and Clippy attributes. That test failure is separate from native
command registration. `a82aa72` corrects the test scanner to handle comments and stacked
attributes, rejects malformed or unsupported prefixes, and retains the existing registration,
duplicate, classification and session checks. Its focused run passed all 9 tests, including
three new parser regressions. No native command or policy ledger was changed. Independent design
and final review found no blocker/high. The complete frontend `npm.cmd test` run at `a82aa72`
passed **1,282 tests, 0 failed, 0 skipped** in 122.64 seconds; output:
`logs/six-client-ui-suite-final.log`.
`npm.cmd run check` also passed with 0 errors and 0 warnings;
`logs/six-client-ui-check-final.log`.
Raw logs: `logs/pr29-ci-108938221541.log` and `logs/pr29-ci-108938222061.log`.

Independent read-only design and implementation reviews inspected the close queue,
generation fences, policy fixture, shared pool consumers and lockfile changes.
No blocker/high remains in those reviewed changes. The additional review found a
native image worker custody issue, corrected separately in `084fda0`. The initial
two custody tests passed. Moving the permit back to the waiter reproduced premature
capacity release (expected one available permit, got two; 0 passed, 1 failed). Source
was restored byte-for-byte and verified by SHA-256, then both custody tests passed
again alongside the cadence checks. The default two-worker cap and ten-second response
timeout are unchanged. A timeout is not decoder termination.

Additional executed native checks:

| Check | Result |
| --- | --- |
| `media_decode::tests` at `aa5f633` | 29 passed, including five new deterministic deadline checks |
| Unchanged `scripts/check-no-ambient.sh` | Passed at `aa5f633` |
| `discovery_cadence_ media_worker_` at the source committed as `990c450` | 6 passed, 0 failed; includes the existing jitter regression |

Outputs: `logs/pr29-clock-media-tests.log`, `logs/pr29-media-worker-tests.log`,
`logs/pr29-media-worker-mutation.log`, and `logs/pr29-cadence-worker-tests.log`.
The discovery runner tests cover startup and both interval boundaries, coalescing
changes during a blocked pass without overlapping work, and termination on a closed signal.

## Subsequent six-client review

The next attachment identified offline shutdown dependence on unfinished member proof and
starvation/cancelled response waits under command traffic. The fixes and their separate evidence
are in [OFFLINE-CLOSE.md](OFFLINE-CLOSE.md) and [ACTOR-FAIRNESS.md](ACTOR-FAIRNESS.md).
The combined native-adapter regression follows the supplied six-client sequence with independent
encrypted stores, fixed blocked edges, saved-route reopen and exact history unions; see
[SIX-CLIENT-RECOVERY.md](SIX-CLIENT-RECOVERY.md). A second variant drops gossip, delays real
transport replies and keeps command producers running while convergence is asserted.

Independent review also found that a stale held commit request could lose its stronger gap when
a bare probe occupied the only queue slot. Requeue now merges that gap before applying capacity.
The focused regression failed when that merge was removed and passed after exact restoration.
No current membership, signature, policy or document-lifecycle verifier was bypassed to obtain
progress. The retained response slot and transient outboxes remain bounded.

Final local checkpoint: Rust runtime/test source `1912b34`; frontend audit test correction
`a82aa72`; subsequent edits affect evidence documents only. On Windows with one Cargo build job,
locked offline dependencies and dev/test debug information and incremental compilation disabled:

| Check | Result |
| --- | --- |
| Complete native library suite | 301 passed, 0 failed, 0 ignored; 263.32s, including both six-client variants |
| Complete sync library suite | 303 passed, 0 failed, 0 ignored; 224.74s, including large-history transfer cost |
| Existing sync integration binary | 29 passed; 20.50s |
| Existing TCP and rendezvous integration binaries | 1 passed each |
| Complete frontend suite | 1,282 passed, 0 failed, 0 skipped |
| Svelte check | 0 errors, 0 warnings |
| Strict sync/app and native Clippy | Passed |
| Workspace/native format and unchanged ambient gate | Passed |

Commands, raw local log names, mutation results and the narrower focused app checks are in the
three linked feature reports. The broad sync package command exhausted disk while linking an
additional integration target; five integration targets remain unrun at this local checkpoint.
Generated artifacts were removed to make room, and the library and already-built integration
binaries were executed separately. No complete package/workspace or platform CI pass is inferred.

At the final status check, [CI run 36430998149](https://github.com/Thalpy/Mewtual/actions/runs/36430998149)
at `a82aa72` had passed cargo-deny and Ubuntu formatting/Clippy. Ubuntu tests, Windows checks
and Linux desktop checks were still running. This is pending platform evidence, not a green CI claim.

## Limits and remaining evidence

The close regression executes the production callback and queue with native writes
mocked in their real ordering; it is not an operating-system close or filesystem
fault test. Immediate lock still invalidates an interrupted decision rather than
claiming successful cancellation. P2P successor admission still requires an
authenticated transition proof. Fixture isolation does not promise arbitrary CPU
starvation tolerance. Complete passing platform CI remains outstanding;
the old red run is not relabelled green, and no Gate 4 or programme acceptance is claimed.
