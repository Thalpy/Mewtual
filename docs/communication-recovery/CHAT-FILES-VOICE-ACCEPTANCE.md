# Chat, files and voice implementation evidence

## V2 dispatch, scheduling and call driver (22 October, branch `communication-v2-signalling`)

Isolated from `f227b4f7`; details, contract and limits are in
[VOICE-V2-CALL-DRIVER](VOICE-V2-CALL-DRIVER.md). Kind-28 dispatch and a production scheduler for
the existing endpoint/source/helper seams sit behind a local `VoiceRouting` switch that is
**Disabled by default** (the old empty reply; nothing scheduled). The renderer pins `legacy`/`v2`
per join and excludes every legacy writer from a V2 join's room. A dedicated V2 call driver
targets a narrow native interface with contract tests. It is **not activated**: no native
commands implement that interface and SDP/ICE action delivery is still pending.

Focused results: `cargo test -p catcoms-sync --lib -- voice` passes 458, zero failures, including
A-B-C discovery through unjoined B and recovery through replacement D under normal ticks, with
hop oracles showing only bound routes were used. Frontend voice/identity/lifecycle/wiring tests
pass 93, and Svelte check reports zero errors and warnings. Read-only adversarial review found one
blocker (default-on service contradicting the decision's gate) and six medium issues; all are
resolved with regressions confirmed to fail when reverted. A second, external PR #50 review
found five P2 issues (shared-slot fairness, pair loss on presence expiry during reconnect,
stranded submits after lost replies, malformed-room presence coercion and a deferred legacy bye
after V2 teardown); all are fixed with failing-when-reverted regressions recorded in the driver
document. Its re-review closed those and found that an exhausted receipt window was treated as
pair death (a P2 closing a connected call on presence expiry) plus a late-timer retry (P3); both
are fixed, and an unconditional native "retire lapsed joins" requirement that contradicted the
established-pair rule is withdrawn. Mem transport is not physical A-C exclusion; no WebView crypto, real RTC, SDP
forwarding or audio is claimed. Complete suites were not run in this session (the primary owns
them).

## Validated proof retention and automatic cleanup retry (22 October)

Frozen runtime `edf923c107f3d11bf635a3e58a04e0b673edd7c1` passed the current native/frontend/static/build gates.
Publication runtime/build inputs match it exactly; source-specific older evidence is preserved.
The native/browser handoff remains incomplete and final dispatch remains disabled.
Browser recovery requires its original owner to remain current. Native history survives End/lock,
but a retired browser owner receives retired; authenticated browser cleanup exchange after End
remains unfinished.

Native final-intent compaction now retains the same already-paid Phase/original Root observer.
Local compaction drops Live outside guards before requiring Root readiness. Channel closure and
independent history GC retain unready proof; ordinary continuation and ClosedPending do not
invent whole Workflow retirement. Existing Row/Phase/codec/listener caps are unchanged.

Browser v12 retries exact setup/channel cleanup after an actual refused RPC using one original
paid driver, one injected-Clock timer and the same availability Signal. Stale callbacks, early
firing and large clock jumps cannot produce duplicate work or missed-tick bursts. Pending RPCs
never overlap; actual SDK/read/linked-chain drainage precedes cleanup. General action retry
behavior is unchanged. Old retained brokers through v11 refuse adoption. This cadence does not
implement authenticated post-End final exchange, issuance or FinalACK.

Validation, zero failures:

- Native: **671 library plus five ACL tests passed**
  (513.28s gate), including all **199 focused handoff tests**
  (4.88s tests/56.53s focused gate). Formatting (8.8s), strict
  all-target/all-feature Clippy (78.61s) and separate check
  (51.84s) pass. No memory/copy/resource cap was raised.
- Frontend: **2456 passed** (79.25s); Svelte zero errors/warnings
  (63.98s), production build (45.81s)
  and all ten Edge flows (57.28s) pass. Focused139/139 passes1.95s.
- Root: **3,254 passed**, 24 existing ignored, 59 summaries on `e560d319fa72727b3298f96396307cfef4ea2ad1`
  (2781.50s). Its Root workspace/dependency inputs are byte-identical to this native/browser-only
  follow-up. The completed Root run and static/ambient/dependency policy retain that provenance;
  Root validation retains that exact source and command record.
- Development build (83.92s), actual startup
  (6.09s), packaged build (122.72s)
  and real packaged Windows document/reload/public-metadata/exact-CSP-CORS/normal-close probe
  (2.94s) pass. Binary SHA256: `9919a4c1c9ae3e568a025fb3ec24bff121371dc0371f7a3cb53bf37895eafa2d`.

Logs: frozen target/voice-final-proof-gates.jsonl, voice-final-proof-root-provenance.json,
voice-final-proof-focused.jsonl and voice-final-proof-front-focused.jsonl. All heavy Cargo
jobs ran serially. Actual-diff adversarial design/review/re-review has no remaining actionable
blocker/high/medium. Initial focused failures are retained: native198/1 dropped the real
LocalJoin revocation owner; browser137/1 expected one aggregate timer for two active channels.
Corrected tests preserve production checks and distinguish the independent owners/deadlines.
The first development build stopped with disk-full error112 on M:, not a code failure. Twelve
older incremental cache folders (23.42GiB logical/3,381 files) were moved intact to
G:/Mewtual-build-cache/CatComs-native-incremental-20261022 and verified; original paths remain
junctions. Relocation preserved cache data and targeted only M: and G: paths. Only the failed build and subsequent
gates were resumed. The relocation manifest is target/native-cache-relocation-summary.json.

Still required: factory/input/admission/result/receipt ACK/settlement/phase/final adapters and
dispatch, authenticated post-End final issuance/ACK/retry retaining original Session keys,
normal Actor/MLS pair success and combined native/WebView action acceptance. History remains
read-only Status9/Begin8(false). Local Phase drainage is not96KiB Workflow retirement. The8KiB
browser wrapper inventory is structural, not arbitrary JS/SDK/GC heap, RSS or caller retention.

The packaged probe requests no unlock/credentials and proves no selected crypto/action delivery
or media. One desktop only: independent physical audio/network and Linux/macOS WebView evidence
remain unavailable. Live V2/legacy isolation, member relay, combined rollout/rollback and separate
native/experiment dependency audits remain open. Current hosted-head acceptance is unverified;
the recorded hosted exact-union failure is not dismissed by local passes. Docker daemon is
unavailable; no container pass is claimed. Separate upload fixes still require Gate7 integration.

## Validated native/browser history checkpoint (22 October)

Frozen Alpha runtime `e560d319fa72727b3298f96396307cfef4ea2ad1` passed all required local gates.
Earlier publication `f42c8e5cbd8f40b6376c5d070d2ecf2901cebd18` has runtime/build inputs matching
that source. These Alpha native/frontend/build gates predate Beta and do not validate Beta's
changed ledger/cleanup code. The later source-specific section records Beta's own gates.
This remains a partial handoff: Status9 and existing-history Begin8(false), no effect dispatch.

Complete validation, zero failures:

- Root: **3254 passed**, 24 existing ignored across 59 summaries
  (2781.5s). New retirement sentinels are present in the rebuilt binary.
  Formatting, strict all-target/all-feature Clippy, ambient scanner and dependency policy pass.
- Native: **664 library plus five ACL tests passed**
  (567.23s gate); all192 focused handoff tests are included.
  Separate native check, strict all-target/all-feature Clippy and formatting pass.
- Frontend: **2447 passed** (72.0s), Svelte check zero errors/
  warnings (65.22s), production build
  (52.92s) and all ten Edge flows
  (64.75s) pass. Focused history/request/reply130/0 also passes.
- Development build (235.83s), actual startup
  (3.8s), packaged build
  (139.8s) and real packaged Windows document/reload/
  public-metadata/exact-CSP-CORS/normal-close probe (2.86s) pass.
  Packaged binary SHA256: `7060f6e924865f8ea660399fbf9e40f54830e7acd59b5700f3df26d4af7ca9f3`.

Exact commands/source/times: frozen target/voice-history-gates.jsonl and
voice-retirement-static.jsonl. Heavy Cargo jobs ran serially. Read-only adversarial design/
actual-diff review and re-review have no remaining blocker/high/medium. Failed prototype/fixture/
compile attempts remain recorded; tests and limits were preserved. Docker daemon unavailable;
no container pass claimed. Hosted acceptance for the current head remains unverified.

Historical lookup recovers the SAME purpose/Row/action/result after End/lock without new
factory/grant/Work/Phase/task or replacement keys/counters. Browser v11 uses the exact retained
owner/handle and established Session, coalesces identical query Promises and retains real SDK/
read/cancel custody through End. History refusal cannot acknowledge setup, close the channel or
refund payment. Shared242B status/3B refusal vectors pin exactu64s, flags/padding and original
receipt identity. Actual Derive reports Derived only; actor errors do not fabricate Rejected.

At Alpha publication `f42c8e5c`, the original Workflow observer/native snapshot distinguished
local16KiB return from96KiB variable retirement, but compaction could still drop its proof.
Missing proof never meant retirement. Retaining the same paid final-intent Phase and adding
bounded cleanup cadence were reviewed designs at that Alpha checkpoint. Beta implements them;
its separate source-specific section records current implementation and validation status.

Still unfinished: factory/input/admission/result/ACK/settlement/phase/final adapters and dispatch,
final-proof retention/issuance/retry coordination, normal production Actor/MLS-pair success and
combined native/WebView action acceptance. Retired browser cleanup needs a narrow original-
Session continuation, retaining framing keys through final delivery before collection.
The8KiB browser inventory is structural known payload/wrapper accounting, not arbitrary JS/SDK/
GC heaps, RSS or caller-retained results. Native idle is not browser SDK/Workflow proof.

The packaged probe requests no unlock or credentials and proves no selected-key/action delivery,
SDP/RTC or physical audio/network acceptance. One desktop only; independent multi-network and
Linux/macOS WebView acceptance remain unverified. Live V2/legacy isolation, relay, combined
rollout/rollback and separate native/experiment dependency-audit findings remain open. The
recorded hosted exact-union failure is not dismissed by local passes.

## Native action ownership checkpoint (22 October)

Fully validated runtime source `4a606d706c77e68cef7660571cdb66df4566862e` in
`target/communication-action-transport-worktree` follows the published8ef5bcff tree.
This remains a partial implementation: the application listener serves authenticated Status
only. Action dispatch, normal proven-pair Actor ingress, final delivery/history collection,
live V2/RTC, member relay and combined/physical acceptance are still unfinished.

The factory derives selection from its actual Route, repeats original generation/selection/
closed checks under the original UIcommit, and requires that same Session's unique physical
hold. Detached actor work and completed output retain that hold/payment after caller loss.
The opaque authenticated header can select only original Phase capacity; the future dispatcher
must still resolve the SAME header RowId and complete decrypted purpose before operations.

Phase result transfer reserves4KiB actor metadata from its original16KiB loan and pins the
actual Buffer/range without copying. Local refusal returns that exact Buffer. The trusted
actor bridge calls its owner slice callback once outside guards, then reads cached immutable
bytes. One body Flight covers queueing AND completed output; it cannot fund concurrent
requests. Root's original grant/action/tag checks remain unchanged. No result IPC is enabled.

Explicit HandoffLoan retirement closes its ORIGINAL confirmation plan; ordinary Drop merely
returns loan bytes to an open Workflow. Held inputs/lower work/copies remain paid after close;
local Phase drainage does not prove full96KiB release. Browser broker v10 shares an actual
Ready channel's existing execution position with its exact selected key and retains each
row's original native adapter. End retains framing/key/control until actual SDK/RPC cleanup.

Complete source-specific gates pass, zero failures:

- Root:3249 passed,24 existing ignored,59 result summaries (2466.83s). New retirement/body/
  actor sentinels are present in the actual rebuilt executable. Format4.23s, strict all-target/
  all-feature Clippy64.95s, ambient0.97s and all four deny categories2.08s pass. Root inputs
  were unchanged by the native test-only lint correction from d5660512 to4a606d70.
- Native:612 library plus5 ACL tests passed (610.91s gate;531.43s library). All140 focused
  handoff tests are covered. Check48.97s, strict all-target/all-feature Clippy37.72s and
  formatting0.88s pass. Focused actor/body22, Root handoff9 and browser crypto/channel162 pass.
- Frontend:2355 passed (89.91s gate), Svelte55.44s zero errors/warnings, production build41.62s
  and all10 rendered Edge flows54.73s pass. These frontend sources are unchanged by4a606d70.
- Development native build200.72s, actual startup3.64s, packaged build124.89s and actual packaged
  Windows metadata/reload/CSP/CORS/normal-close probe3.14s pass. Corrected source-label probe
  rerun also passes on the SAME binary, SHA256
  `3460b673856707326f073353a924c36c36922612c5618b9482fa2cb8b78e9c2e`.

Commands, source IDs, timings and failed attempts: `target/voice-action-custody-gates.jsonl`
and `target/voice-action-custody-*`. Missing frontend-dist and test err_expect Clippy failures
were corrected; no test/limit was loosened. Stale cross-checkout dep-info was preserved and
renamed, never deleted; earlier stale focused executables are NOT evidence for new tests.
The first packaged probe's source label still named test-only parentd5660512; its retained log
and corrected rerun distinguish metadata correction from a new runtime/build claim.

Read-only actual-diff adversarial review and re-review have no remaining findings for this
checkpoint. The separate action-dispatch checkout contains later shutdown/grammar/transfer
and typed result work; those changes are NOT covered or published by this ownership checkpoint.
Its full frontend checks pass, but current native compilation/review is separate and ongoing.
Only one physical desktop exists. The packaged probe requests no unlock or credentials and
proves neither selected-key/action delivery nor audible/multi-network/other-platform behavior.
Separate native/experiment dependency audits and the recorded hosted exact-union failure remain
open. No fully green latest-hosted-head or complete Revision2 claim is made.

## Native weak-context and lost-reply checkpoint (22 October)

Frozen runtime source4db3f8249ef7960e11cc40febd2b81f6c3a6018e in
target/communication-browser-action-worktree follows published PR48 checkpoint8773ac6b.
The native/browser handoff remains this agent's assignment. This checkpoint does not enable
an action opcode, live V2, SDP/ICE/RTC or member media relay.

The paid application IO state now holds nine Weak references to the actual native owners.
Per-call upgrade is all-or-nothing; ordinary commands and the native factory borrow the SAME
UI session gate, real store, atomics and original commit mutex. Broad native state drops outside
guards before detached work, which retains only actual actor/selected/grant/commit/work/payment.
Factory selection accepts the original native Arc; pure early/final Coordinator checks cannot
materialize a local snapshot when rejecting a foreign equal-label owner.

Weak references retain original inline Arc allocations after value destruction. The fixed IO
payment increased from24KiB to32KiB, within the same2MiB process limit. The current Windows
layout is32,256 bytes, including11,352 bytes for weak-retained allocations and context wrappers;
tail padding is measured using the pinned Arc layout. Context drops before its payment.
Other platform layouts continue to refuse if the measured peak cannot fit.

Ledger::lookup_original now recovers the same paid native row after a lost Begin reply supplied
no RowId. It checks original grant allocation/executor/lease/full purpose, runs no Clock/charge/
grant upgrade/factory action, and remains usable after End and actual grant destruction.
None means no retained matching row, never drainage. This historical lookup is separate from
currently authorized retry: idle handed-off Pending work still resumes on its original row/serial.
No caller or transport uses historical metadata as permission to re-arm a factory.

Focused current gates: all123 native handoff tests pass (2.06s); the preceding172 voice-filter
tests pass before the three additional lookup tests. Nine actual native command-security tests
and the full2,348 frontend tests pass (62.24s). Actual-diff adversarial design/review/re-review
has no remaining findings. The earlier24KiB layout refusal and test-only Context name collision
remain in failed logs; corrected focused runs and measured-layout check pass.
The restricted browser-flow attempt lost CDP before flow assertions; the unchanged gate was
rerun with normal Windows process access. Failed attempts are retained, never counted as passes.

Complete fresh native passes600 tests including five ACL tests, zero failures
(512.28s gate). Check27.17s, strict all-target/all-feature
Clippy40.94s, format0.84s and the fresh Root construction
scanner0.77s pass. Fresh complete Root passes3233 tests, zero failures,
24 existing ignored across59 result summaries (1920.34s). Root Rust/Cargo/
dependency-policy inputs remain byte-identical to validated f1c58519; prior Root static/ambient/
dependency checks retain their own provenance. Native and Root full suites ran serially.

Fresh frontend Svelte check57.41s has zero errors/warnings; production
build45.14s and all ten rendered flows55.89s pass.
Development executable build131.47s, actual startup3.62s,
packaged custom-protocol debug build119.19s and actual packaged Windows
document/reload/CSP/CORS/public-metadata/normal-close probe2.67s pass.
Probe binary SHA256:32e07da86119b0cc764d719fb9ec0c3e172a5443d062096b9eafc969470be8ab. It requests no unlock/credentials and verifies
only public metadata, exact-port policy and normal native collection. It does not establish
selected crypto/action delivery or audible/physical networking. Only one desktop is available.
Logs: target/voice-native-context-*; exact commands/times: voice-native-context-gates.jsonl.

Still required: authenticated factory/action input/result/receipt ACK/settlement/phase closure
and final delivery over the same Session; final actual-channel closure under the original UI
commit; production proven-pair registration; full96KiB Workflow retirement and copy funding;
sharing the original browser execution position; safe collection of the separate32-row action
history. The existing16KiB Phase loan is not the whole Workflow. Live V2/RTC/isolation, relay,
combined acceptance and independent physical audio/network acceptance remain open with their
assigned owners. Separate native/experiment dependency-audit findings remain open.

## Previous action-row ownership checkpoint (21 October)

Source b0b303ebcc140f6414e89ae4b06ee8a46ece5175 follows the published edb00dda app-owned
Status checkpoint in target/communication-browser-action-worktree. The native/browser handoff
remains the active assignment. All raw RowId-taking operational helpers are now private;
Ledger::resolve requires the original native-selected grant allocation, executor, lease and
complete peer/join/key/epoch purpose. A native-only ResolvedRow retains the actual paid Row.
Repeated resolution is not a once-only work permit; physical callers still must account and
bound their handles. The complete layout remains inside the original2KiB Row payment.

Historical lookup/receipt ACK/settlement remains usable after End and actual CallGrant drop.
Data work checks the original lease under State before copy/worker admission, including when
End won during a wait. Four new regressions cover foreign equal-field owners/all selector
fields/invalid IDs, each key component, exact post-End recovery and the actual held-State race.
All112 focused handoff tests pass (2.03s after49.66s compile). Actual-diff adversarial review/
re-review has no remaining findings. Full frontend2,348 passes (68.32s). Full native583 library
plus five ACL tests pass, zero failures (523.69s library;526.92s gate). Native check48.50s,
strict all-target/all-feature Clippy50.34s, format0.76s and the fresh Root construction scanner
(one test,0.97s gate) pass. These ran serially; no validation process remains active. Failed
test-Barrier compile and interrupted focus3 are retained; the final barrier samples
credits after End's legitimate independent generation retirement, without weakening work,
receipt, copy or worker assertions. Logs: target/voice-scoped-row-*.

This adds no action opcode, new command, live V2 driver or Root/wire/storage change. The passing
packaged probe/build/startup evidence below belongs to unchanged bac90 application lifetime;
it does not prove action delivery. Native/browser action/result/ACK/settlement/phase closure and
final delivery remain open. Native context, full Workflow/copy funding, sharing the original
browser execution position and safe32-row action-history collection still require integration.
Root Rust/Cargo/policy and frontend/build inputs are unchanged; preserve their recorded exact
provenance rather than treating previous gates as fresh runs. One physical desktop is available.

## App-owned status/bootstrap checkpoint (21 October, still partial)

Source bac90cfa in target/communication-browser-action-worktree adds the native application host,
exact pre-window numeric-loopback CSP, entered-runtime setup, Exit-before-SDK-cleanup collection,
three main-only public metadata commands and a retained production browser adapter. It also
implements reusable channel closure with actual independent header/reply/public-owner custody,
monotonic serials and grant-independent housekeeping. All setup variants retain their original
paid serialization position; JSON capacity is hard-capped, not inferred from final length.

All 108 focused native handoff tests pass; 35 browser adapter/Session and nine command-security
checks pass. Full frontend2,348, Svelte zero errors/warnings, build and all ten rendered flows
pass. Complete native passes579 library plus five ACL tests, zero failures. Native strict
all-target/all-feature Clippy/check/format, development build/startup and packaged build pass.
The actual packaged Windows committed document/reload, enforced exact-port CSP, CORS, public
metadata cleanup and normal-close/task-collection probe6 passes. Earlier failed harness attempts
remain recorded; no runtime change or relaxed readiness was required. Root's complete3,233 and
static gates passed on unchanged Root inputs; they were not repeated for this native-only delta.
The fresh Root construction gate also passes against current native source. Actual-diff review/re-review has no remaining blocker/high/medium finding. Exact final
gate records and failed-attempt provenance are in HANDOVER and target/voice-app-channel-*.

Only authenticated Status is enabled. Public metadata IPC is not action/result delivery; stock
framework/WebView public-metadata copies after handoff are outside the stated application-copy
bound. Factory input, result, exact receipt ACK, settlement, authenticated browser phase closure,
final delivery and separate action-history collection remain open. Live V2/SDP/ICE/RTC, legacy
isolation, member media relay and combined/physical acceptance remain unfinished. Only one
physical desktop is available. The passing packaged Windows startup/close probe exercises public
metadata/CSP/CORS/normal shutdown only, not selected crypto or audible calls; it requests no vault
unlock and supplies no credentials. Linux/macOS WebView compatibility remains unverified.

Historical hosted f227b4f7 run39649824415/Linux job119394907283 failed reversed-reopen exact signed
chat union (531 passes/1 failure, client5 had14/15; bridge4/client6 had15/15). Diagnostics did not
fix that cause; latest-head hosted acceptance is unverified. The local new-channel discovery
scenario passing does not establish a hosted recovery fix. Preserve the exact union and bounds.

## Historical bootstrap checkpoint (21 October, preceding complete gates)

Active checkout is target/communication-bootstrap-channel-worktree, based on published
`c5c52fd8`. An earlier fresh Root stopped with1209 passed/1 failed in the app library: a capacity-custody
test raced unrelated users of its real global source pool. `dda602f3` isolates the original
scenario in a dedicated integration process, also proving its source adapter and linked
public factory share that same pool. The fresh complete Root rerun passes3,233, zero failures,
24 existing ignored,59 summaries (2,055.81s). Root format, strict all-target/all-feature Clippy,
ambient and all four deny categories pass.

Seven new focused process-retained-capacity tests pass (0.01s after75s compile). The separate
affine owner shares existing process bytes but has no group/budget/work-slot conversion. It
covers fixed native infrastructure only, not group payloads or computation. Exact/overflow
refusal, both drop orders, full group table, unchanged Crypto/Forward limits, concurrent
admission and uncollected output/last Arc custody are checked. All27 resource tests pass;
actual-diff read-only review has no findings. An earlier cached
invocation matched zero tests and is not evidence;
only sync fingerprint result markers were invalidated before the actual seven-case run.
The internal selected-owner ECDH bootstrap has13 new passing native regressions, within61
passing handoff tests. Seven new real-WebCrypto/metadata tests and eight existing framing
tests pass. Full frontend passes2,312, Svelte zero errors/warnings, production build and all ten
rendered Edge flows pass. Complete native passes532 library plus five ACL tests, zero failures
(517.09s library;676.16s gate). Native check60.72s, strict all-target/all-feature Clippy72.50s,
format0.94s, build219.59s and actual startup4.38s pass. All four new diagnostic regressions pass.
Immutable shared output custody and UTF8 BOM identity regressions resolve the initial review
findings. p256 source review documents limited stored-value zeroization and no independent
arithmetic/assembly audit; separate native/experiment dependency findings remain open.

Hosted `c5c52fd8` Linux fails reversed-reopen exact-history recovery (client5 retains14/15 ops;
bridge4/client6 have15/15). The ordinary-request and passive-history observations added here
retain bounded fixture metadata and keep all original oracles/deadlines. They do not establish
the cause or fix it. Windows Root also fails the file-range actor bound during overlap with
the global custody fixture; the new process isolation removes that interference, without
establishing it as the sole hosted cause. Fresh hosted acceptance remains required.

No renderer bootstrap command, listener/CSP, browser Session or live dispatch is enabled.
Native/browser action delivery, actual setup ACK/provider custody, RTC/legacy isolation,
media and combined/physical acceptance remain open. Earlier running-Root notes are superseded.

## Framed handoff continuation (21 October, acceptance still open)

Published checkpoint is `71d11284`; reviewed file-fixture correction is `7609b66b`. Work is isolated in
target/communication-framed-handoff-worktree. The disabled native codec prepays one physical
position through header/body/reply/completed-output ownership, authenticates canonical metadata
before body allocation/counter debit, and never refunds admitted failed or lost flights.
Independent control custody survives data-phase closure. Its13 actual-native regressions and
eight real-WebCrypto/shared-vector cases pass, including maximum Data and sequences above JS's
safe integer range. There is no application listener, seed export, browser Session or V2 ingress.

The internal factory consumer resolves the actual selected native key/actor, fences original
UI generation again under final Coordinator, and retains owned UI commit across detached Pair/
Derive work. Pending RowId is only metadata. Six native regressions cover definitive refusal,
capacity before actor work, pending duplicates, canceled callers, lock ordering and completed
uncollected error ownership. Successful proven-pair Actor registration remains unproved through
normal disabled ingress. Static review/re-review has no remaining findings in this boundary.

Complete final Windows native passes515 library plus five ACL tests (518.49s tests;521.62s gate);
native check19.73s, strict Clippy10.42s and formatting0.78s pass. The earlier native check,
executable build274.05s and actual startup6.20s pass. Final100 focused voice tests, strict native
Clippy and formatting pass after lint-only attributes and vector relocation. Four strict-lint
findings were corrected with documented local attributes preserving prepaid inline custody and
existing command signatures. Frontend passes2,305, Svelte zero errors/warnings, production build
36.06s and all ten rendered Edge flows pass. The sandbox CDP attempt failed; the unchanged flow
gate passes with normal Windows access. Root's621 source/manifest/script inputs are identical
to the frozen3,226-pass checkpoint; fresh complete Root is now running serially after native.
Root formatting, ambient and all four dependency categories pass freshly. Normal-access Git
Bash/advisory-lock retries passed after sandboxed WSL/Cargo-lock failures. All24 final source
hashes are frozen; only three disposable construction-test fingerprint result markers were
invalidated to force the source-reading test to scan this checkout. No cache tree was cleaned.

Hosted `dbb8d36b` is not green: Linux native membership recovery fails with both returning peers
disconnected, and the native Studio mutation gate fails its expected assertion marker. The
continuation now waits for a real sealed3->1 route at original admission; dial queue admission
alone was not that prerequisite. It adds bounded sealed-target/current-listener and requested/
submitted dial observations without injecting restore hints or extending deadlines. Both
focused Windows chat-matrix cases pass (74.13s tests;146.60s gate). Static tracing confirms the
fixture gap, not that it is the sole Linux cause. The Studio mutation marker now includes the
exact diagnostic assertion reached by a recreated context, preserving exact expected errors,
one intended failing test and byte-for-byte restoration. Both injected failures and restored
cases pass (389.75s). The subsequent full native run failed514/1 at the file bridge's immediate
manifest assertion: C had received the directory before B applied its independently queued
copy. The original30-second barrier now observes both exact catalogs before exercising the
unchanged bridge, private-storage, restart and byte assertions. All seven file scenarios pass
44.56s, and the final complete native rerun passes as above. Static re-review has no findings.
Hosted Linux native confirmation remains required. The local Docker daemon is unavailable,
but the new exact public-vector allowlist and context assertion pass on hosted71d11284.

Native/browser bootstrap, actual framed input/result/ACK/settlement, whole-chain/SDP phase
transfer, RTC and legacy-mode isolation, live V2, media relay, external destination resume,
combined rollout and physical audio/network acceptance remain open. Only one desktop is available.

## Validated handoff prerequisites (21 October)

Validated runtime source is `fef81f48`. Current code passes434 focused Root voice tests, including eight new
handoff cases for original binding/Clock/expiry, settlement order, sibling rows, pressure and
actual last-loan custody after End. Original2KiB/1536-byte allowances are unchanged. Root format,
strict whole-root Clippy, ambient and all four deny categories pass. Complete current root
passes 3,226, zero failures, 24 existing ignored, 58 summaries (4,815.7s). Complete native passes
496 library plus five ACL tests, zero failures (597.81s library;600.73s gate). All80 native voice
cases pass (18.02s tests;199.59s compile/gate), native check47.79s, executable build191.16s,
formatting and actual startup2.00s pass. Full frontend passes 2,297, Svelte zero errors/
warnings, production build and all ten rendered flows pass. Stale-cache and failed attempts are not
accepted current-source evidence; affected crate markers were invalidated before these results.

Ten candidate-reader tests pass, with zero Svelte errors/warnings. Packaged Windows
Edg155.0.4283.62 passes the exact transpiled helper on a real17-byte loopback BYOB response,
releases its reader and records two successful GETs. Existing framing/replay/ACL/keyless
takeover/native shutdown/profile cleanup pass. Cleanup-fault injection refuses publication
and preserves the previous result; all 17 experiment Node tests pass. This uses generated fixture data and no account;
it establishes no live V2 call or physical audio result. Read-only re-review has no findings.

The disabled native pilot now includes selected-owner binding, original-action registration,
one shared phase loan and immutable compacted terminal history. All 60 actual-source harness
cases pass, including 29 phase/parser/ledger cases and two new selected-owner cases. This is
not actual Actor/MLS/browser acceptance. Those same cases now also pass in the complete native
workspace gate. Ledger re-review has no
remaining blocker/high/medium after cleanup and observer readiness fixes. The 32-row pending/live/drained bound is an explicit
pilot limit; uncertain history is never evicted.

The first actual native compile exposed two test-only futures helpers supplied by the smaller
harness but absent from native dependencies. They use std::task::Wake now, with no dependency
change. A sandboxed startup attempt produced no output and timed out; the unchanged gate and
same rebuilt binary pass with normal Windows access. Neither failed attempt is a pass. All19
Root/native frozen source hashes match after validation, and command-security extraction
passes freshly on current native source. Earlier pending sections below are historical.

The loan conserves capacity; actual actor integration, physical-copy/terminal-proof delivery,
SDP phase transfer/live V2/RTC/isolation, media relay, external destination resume and combined/
separate-device acceptance remain incomplete. No production listener/CSP/dispatch is enabled.

## Validated selected key bootstrap (21 October)

Published source remains `f5ae59a3`. Local frozen `451ecbaa` integrates one native/browser selected
key through actual ServerActor endpoint publication and typed executor-refusal recovery.
Current frontend/security focus passes 28 cases (19 selected-key,
nine command-security), including retained orphan settlement on remount and two distinct
brokers while the original engine is held. Definitive native refusal grants the second broker
no work or settlement authority, including if End occurs while its reply is pending.

Nine initial native regressions pass: four actual actor/bridge cases and five synthetic-grant
Coordinator cases. They cover frozen-point Root Capacity retry, actual publication after outer
waiter cancellation, lock ordering, generation fencing, settlement-first and stale completion.
The original native focus takes0.35s after6m48s compile; the corrected nine cases pass in0.33s
after2m16s compile.
Earlier 49 native voice/2,285 full frontend passes precede the refusal correction. The subsequent
native full run was deliberately stopped to include that correction; it is not accepted evidence.
The corrected source now passes complete native:465 library plus five ACL tests, zero failures
(586.76s library;589.92s gate), native check27.8s, all49 native voice tests, full frontend2,287
(70.61s tests), Svelte zero errors/warnings, production build51.08s and all10 rendered Edge
flows64.61s. Existing complete root3,218/static evidence covers byte-identical root source;
the native-source construction gate passes freshly here in1.09s. Native executable build163s,
formatting0.69s and actual startup2.55s pass. Static re-review has no remaining blocker/high/medium.

This bounds paid generation records, broker flights and admitted publication work. It does not
bound arbitrary stock-IPC invoke waiters/framework copies and does not complete action/result
IPC, SDP phase transfer, live V2/RTC, media relay or combined/physical acceptance.

## Validated accepted-activation retry correction (21 October)

In communication-activation-recovery-worktree on319db214, accepted native controller/call
activation now recovers exact current metadata before consumed-candidate lookup. The final
native fence checks original allocations/lease/controller, UI generation and server incarnation.
Concurrent duplicate misses recover before RNG. Refused fresh activation leaves temporary grant
cleanup outside Coordinator/UI guards; it cannot revoke an existing owner. No new allowance,
registry, serialized command, listener/CSP, wire/persistence format or live V2 is introduced.

All 40 focused native voice tests pass (19.47s;10m01s compile), including thirteen new cases.
Five Coordinator tests distinguish
synthetic grant provenance; seven native tests use actual actor queries and held UI commit
barriers for accepted reply loss, concurrent early misses, End, entry replacement, pending-End,
actual actor-stop and lock/unlock. One additional native test isolates actual lock registration
while original child recognition is still live, proving generation refusal before its delayed
voice revocation hook. Dedicated actual room/policy closure remains a test follow-up.
Read-only static review/re-review has no remaining blocker/high/medium after the borrowed-grant
correction. Standalone native rustfmt, full frontend2,268 (75.15s), native command-security9
(22.16s), Svelte0errors/0warnings, build48.69s and all ten rendered Edge flows pass. The complete
root suite passes3,218, zero failures,24existingignored,58summaries (5,304.73s), on319db214 with
byte-identical root manifests/lockfile/crates/bins/scripts. Fresh native-source construction gate
passes here. The cumulative native suite passes 456 library tests plus five ACL tests, zero
failures (579.06s library; 645.71s complete gate). Native check (66.43s), executable build
(338.48s), native formatting (0.82s) and actual application startup (3.67s) also pass.
Session14577's older native compile was deliberately interrupted before testing; it contributes
no native pass claim. No validation process remains active. Pending notes in earlier sections
below are historical and superseded by this cumulative evidence. Logs are this checkout's
`target/voice-activation-*.log` and the pair checkout's `target/voice-pair-*.log`.
No physical or live RTC acceptance is claimed.

## Native pair continuation (21 October, focused Root checks passed; full gates pending)

Next source is communication-pair-origin-worktree based on 7e19b948. New Root/Actor pair factory,
immutable native implicit-watch selector, original pair retry/epoch/window and atomic publication
are implemented. Authored regressions cover both native tables/serial, actual revocation at every
Clock sample, final expiry and post-pin Group/Policy contention, full-pool and traffic conservation,
explicit-watch substitution, actual newly-proved rejoin and epoch-before-first-derive. Existing
positive low Root fixtures now use the production factory; the bridged public-presence test also
requires both endpoint roles to produce the same handshake and native Derive action.

Static review exposed inherited source_context Clock-under-Group/Policy reentry through grant
validation. Sampling is moved before nonblocking native locks with original owner rechecks.
Its new retryable Capacity is handled in SourceLedger without dropping earlier spent jobs;
an actual two-packet Policy-contention test passes. Other inspected callers propagate or retain
the refusal before mutation. All 426 focused Root voice tests pass, zero failures (22.49s),
including both-role bridge proof-to-Derive,8-confirming/16-total/16-pin bounds and exact retry.
The initial compile failed two fixture type checks (u64 callback return/non-Copy Context move);
corrected rerun passes without weakening runtime gates. Logs: target/voice-pair-root-focused-{1,2}.log.
Read-only adversarial re-review has no blocker/high/medium. All10 actor and the external API
test pass, zero failures; complete frontend2,268 (73.48s), Svelte0errors/0warnings, production
build51.22s and all10 rendered Edge flows pass. Strict whole-root Clippy1m55s (warnings denied),
formatting, ambient and root dependency policy pass. Logs: target/voice-pair-{actor-focused-2,
public-api,frontend-full,svelte,frontend-build,flows,format,clippy,ambient,deny}.log. Complete
current Rust/native/check/build/startup gates remain pending. Old derived-origin full was interrupted for
the context correction and has no pass claim. Published head remains
c1532331 and no live V2/RTC/media, combined or physical acceptance is claimed.

## Native actor and terminal lifecycle (21 October, full gates pending)

The body-free trusted actor bridge and terminal-retirement correction pass **10 actor tests**,
**one external API test** and **411 Root voice tests**, zero failures (Root16.46s). Original
Root state and affine grants remain authoritative. The mailbox exposes no renderer command or
byte-bearing result path. Stopped means uncertain outcome; exact native retry identity survives.

Tests exercise reply loss during watch creation, independent lock/unlock/replacement, canceled
queued stop, lost ACK after admitted stop, saturated Closed delivery, explicit shutdown and
canceled/completed saved freeze. Actual native terminal cleanup conserves engine/input custody
until external owners drain and leaves another Runtime group live. Pending transport authority
refuses when its actual Clock callback retires Root, replaces the connection or closes its room;
held lower custody is returned only when actual pending/authority owners drop. No limits changed.

Logs in communication-derived-origin-worktree/target: voice-actor-native-focused-3.log,
voice-actor-public-api.log and voice-actor-root-focused.log. First actor run passed7; the second
attempt failed to compile an across-crate private wait-method call in a test. The corrected test
uses actual queued reply disconnection, without widening the production API; all fresh cases pass.
Read-only static re-review has no blocker/high/medium. LOW diagnostic wording is corrected.
Dedicated sync-error fault injection is a residual test gap. Full current Rust/native gates are
pending; the identical frontend already passes 2,268. Strict whole-root Clippy passes with warnings
denied (1m35s), formatting, Git Bash ambient and root dependency policy pass. Initial ambient
invocation chose Windows WSL Bash and was denied before running; explicit Git Bash passes.
Logs: voice-actor-{format,clippy,ambient-2,deny}.log. Historical full results below apply only
to the published c153 checkpoint. Production pair orchestration/IPC/RTC/V2, media relay, combined
rollout and physical acceptance remain open. Only one physical desktop is available.

## Root derivation and late-publication deadline (21 October, full gates pending)

Active continuation in communication-derived-origin-worktree, based on local fe9c1048, connects
actual Root row-zero production and Derived result to original call/action/pair/chain allocations,
native actor/session, current room/member/epoch and final immutable deadline. Exact cached action
lookup survives ACK/input compaction without key reconstruction. Affine preparation retains real
custody through settlement/unwind. The shared first-seal fence now checks minimum construction,
source and original SDP deadline, preventing a late output from promoting an expired Finish.

Fresh focused tests pass **407/407** (17.07s, compile2m16); after adding direct busy-Policy and
actual signed room-delete/recreate evidence, the rerun passes **409/409** (18.62s, compile1m01).
Tests cover both derive roles, exact compacted recovery, Group/History/Policy/final Capacity,
full ordinary-pool pressure, original native closure, last Clock owner/expiry, actual session
close blocked on Policy with live child, settlement/unwind, wall rollback/epoch, Root substitutions
and late Hello/Proof/Finish at15s -> final17s against pair16s/construction20s. Original row/metadata
allowances are unchanged. Full frontend passes **2,268**, zero failures (211.08s, two workers).
Strict root Clippy (37.86s), formatting, ambient and root dependency policy pass. A sandboxed deny
attempt could not acquire its external advisory lock; authorized normal-OS retry passes.

Read-only review found and fixed one MEDIUM late-seal deadline gap and LOW test assumptions about
idle Workflow reservations. Short re-review has no remaining blocker/high/medium. Residual LOW:
the deterministic Policy holder test relies on an external suite watchdog if a regression replaces
try-lock with a blocking acquisition. It releases safely on normal return and unwind.
Logs: active target/voice-derived-origin-{focused-1,focused-2,frontend-full,clippy,format,ambient,deny-2}.log.

The full fe9 root/native/check/build run was deliberately interrupted for this correction; it
has no full-pass claim. Published checkpoint remains c1532331 with the complete evidence below.
Complete current Rust/native validation, production native registration/IPC/result-settlement
retry custody, authorized SDP key/phase transfer, normal V2/RTC and isolation, member media relay,
combined rollout and physical acceptance remain open. Only one physical desktop is available.

## Validated protected confirmation checkpoint (21 October)

Published code through `9dc1b8d3` (`73c444a5` browser implementation) has identical Rust/native
source to frozen `ecc8232f`. The sequential full validation in communication-proof-finish-worktree
has completed: root **3,146 passed**, zero failures, 24 existing ignored across 57 summaries;
native **443 library plus five ACL tests passed**, zero failures (874.42s for the library);
native check passed (1m32s), debug build passed (6m26s), and actual startup passed with the
original 20-second deadline. The native suite includes the previously failing onward channel
discovery and authenticated automatic notification receipt, plus chat-bubble and durable
Keep/private Download bridge-restart scenarios. Logs: voice-followup-root-full.log,
voice-followup-native-full.log, voice-followup-native-check.log, voice-followup-native-build.log
and voice-followup-startup.log in that frozen checkout's target directory.

The selected browser code passes **2,268 full frontend tests**, zero failures (67.43s), 105 focused
real-crypto/fault-seam tests, Svelte with zero errors/warnings, production build (36.24s) and all
ten rendered Edge flows. Root strict Clippy, formatting, ambient checks and dependency policy pass;
root/native formatting was rechecked. Read-only adversarial re-review has no remaining findings
for the protected confirmation increments. This is a validated partial checkpoint, not live V2.
Separate native/packaged-experiment dependency audit failures remain recorded; root cargo-deny
passing does not clear them. Only one physical desktop is available.

The newer Root result-safety code is committed locally as fe9c1048 in
communication-root-results-worktree and is not included in this published checkpoint. Its full
attempt was interrupted for the reviewed deadline correction above; prior complete suites do
not validate it. Production native registration/IPC/result settlement,
SDP/key phase transfer, normal V2/RTC and legacy isolation, member media relay, combined rollout
and physical audio/network acceptance remain unfinished.

## Native result continuation (21 October, focused verification passed)

Current result implementation in communication-root-results-worktree adds private prepared
signatures, actual Root seal/open result admission, deferred consumed token/slot/promotion cleanup,
preconstructed replacement attempts and an affine engine preparation hold. Compatibility
seal/open/derivation also move housekeeping before Engine. Competing settlement fences first
effects immediately but cannot return actual preparation capacity or report drainage early.
No wire/persistence format, row limit or byte allowance changes.

The first fresh run passed 388/failed 1 (15.58s, voice-root-results-focused-1.log): its synthetic End
call lease was independent of actor children, so runtime revoke did not itself retire that lease.
The fixture now also ends its original call and retains the retired assertion; actual Root
regressions use native Directory/CallGrant custody. Corrected full focused execution passes
**389/389**, zero failures (15.69s, voice-root-results-focused-2.log). All fixed-layout guards pass.
The new cases exercise all three protected flights, exact accepted/rejected retries, final
Capacity without resigning/token effects, actual session revoke blocked on Policy with a live
native child, final Clock revocation, signing/RNG settlement, unwind, native owner substitution,
MLS epoch and equal-join replacement. No runtime gate or assertion was loosened.

Static adversarial re-review found no remaining blocker/high/medium after fixing allocation
inside admission, cleanup under Engine and premature credit return during preparation. Formatting
and ambient pass. Strict whole-root Clippy passes (2m25s), root dependency policy passes, and the complete frontend
passes 2,268 tests with zero failures (216.25s, two workers). Complete root/native validation
was subsequently interrupted for the later deadline correction. Earlier complete results above apply only to published c1532331.
Production native IPC/RTC/V2, complete Root derivation, authorized SDP phase transfer, member media
relay, combined rollout and physical acceptance remain open. Only one physical desktop exists.

## Protected WebCrypto confirmation (21 October, native integration pending)

The selected browser broker now consumes immutable protected row projections for seal/open,
using the original accepted derivation keys. Twelve row claims and all byte limits are unchanged.
The serial prepaid execution position remains reserved between flights; maximum retained input
plus maximum serial engine peak must fit before copying. Real owned TypedArrays prevent aliasing
subclass/Buffer slices. Historical tagged and tagless admission outcomes cannot settle a prior
browser's engine. Lost result replies retry original computed output without another AEAD.

New real P-256/HKDF/AES-GCM tests cover all three flights and all three candidates, maximum
4,096-byte AAD/512-byte confirmation plaintext, lost accepted/rejected replies, delayed settlement,
full ordinary byte/queue pressure, engine cancellation, nonce reuse, corrupted ciphertext,
identity/role/binding/allowance substitutions, aliased caller mutation and metadata-only history.
The native adapter is an explicit fault seam, not actual native IPC or RTC. All 14 new tests pass
(`voice-protected-boundaries-1.log`); existing 91 pass. The earlier new run passed ten/failed one
because changed-length fixture input triggered the correct engine-byte rejection before the
intended equal-size identity check. No runtime check was loosened. Corrected combined verification
passes104/104 (`voice-protected-all-2.log`) before the final maximum-candidate boundary test.

Final full frontend: **2,268 passed**, zero failures (67.43s,
`voice-protected-frontend-full-2.log`). Svelte: zero errors/warnings (`voice-protected-svelte-2.log`).
Production build: pass,36.24s (`voice-protected-build-1.log`). All ten rendered Edge flows pass
(`voice-protected-flows-1.log`). Read-only re-review resolves both medium findings and reports no
remaining blocker/high/medium. The initial existing-test command did not run because the new log
directory was absent; it is not counted as evidence. The corrected command passed91 existing cases.

Rust/native sources are unchanged from frozen `ecc8232f`; its one sequential complete validation
chain is still running in `communication-proof-finish-worktree`. Native startup follows its
full root/native/check/build gates. No parallel Cargo suite is running here. Production Root result
admission, native IPC, phase/key transfer, normal V2/RTC, media relay, combined rollout and physical
audio/network acceptance remain open. Only one physical desktop is available. The separate native
provider experiment is unselected/uncompiled and is not included in this implementation.

## Protected Proof/Finish work in progress (21 October)

The `communication-proof-finish-worktree` (base local `600186e5`) contains protected follow-up
intake and immutable output factories. New tests cover all three flights, full ordinary
pressure, three input identities/fourth refusal, malformed Hello intake-lane mapping, reversed
Proof arrival, supersession before/after engine admission, old Finish crossing a new Hello,
terminal input compaction and old produced/unproduced response retries. Original resource/layout
allowances are unchanged. This is **uncommitted work**, with read-only re-review clear. The
latest focused run passes all365 cases; complete combined gates remain pending. The complete native suite against frozen low source `600186e5` passes **443 library
plus five ACL tests**, zero failures (885.37s for the library, `voice-low-native-full.log` in low
checkout); root deny also passes there. Native check/build/startup and combined full root remain
pending. No prior passing suite establishes this new increment.

The first follow-up invocation (`voice-followup-all-1.log`) reused the old shared-target 352-test
binary, with none of the new cases. It is **not accepted evidence**. The catcoms-sync compiled
fingerprint files were invalidated without cleaning target or deleting any source/worktree.
The actual fresh compile/test ran **363 passed, two failed** (`voice-followup-all-2.log`, 16.70s).
The fixed metadata guard measured 2,088 bytes against the unchanged 2,048-byte allowance. The
correction moves bounded follow-up links and exact response pointers into their separately paid
input records, retaining only one follow-up head in shared metadata; it raises no allowance.
The other fixture held the sole protected engine while requesting another, correctly receiving
Capacity. It now defers opening old Finish until after the newer Hello finishes. No execution
limit or runtime refusal was weakened. A new compile/test retry is running
(`voice-followup-all-3.log`) passes **365/365**, zero failures in 15.76s. Both unchanged layout
guards pass. The remaining unused-state warning is corrected with an actual held-History
closed-state and input-pointer check. Final focused verification passes **365/365**, zero failures
in 15.67s (`voice-followup-all-4.log`). Strict whole-root Clippy passes in 1m15s
(`voice-followup-root-clippy-1.log`); full frontend passes **2,254/2,254**, zero failures in69.50s
(`voice-followup-frontend-full.log`). Root/native formatting and ambient checks pass. Read-only
adversarial re-review is clear, including the revised acyclic history graph. Complete combined
root/native/check/build/startup gates remain pending. No live V2/crypto
interoperability/IPC/RTC claim.

The communication programme remains **open**. File bridging and explicit private Download resume
are implemented and exercised; live V2, production member media relay, combined rollout and
independent physical audio/network acceptance are not complete. The dated sections distinguish
current runtime paths from inactive prerequisites and experiments.

## Low-side protected Hello work in progress (21 October)

The separate `communication-low-hello-worktree`, based on reviewed local source `53191c87`,
now contains the native Root/local low producer and affine outgoing preparation. New tests target
exact predecessor retries, all three lanes, packet/action/chain cancellation, ordinary pressure,
final History contention, real Root call/epoch replacement and original-clock source custody.
The fixed row/metadata/Pair allowances have not changed; their growth guards pass.

Adversarial review found two medium issues during implementation: a later busy effect discarded
earlier prepared source jobs after attempt debit, and Clock callbacks could revoke a previously
checked capability while the same observation returned true. Both are corrected and have focused
regressions. Re-review found no remaining actionable defect. Formatting and whitespace checks
pass. All **352 focused voice tests pass**, zero failures in 15.33s (`voice-low-all-4.log`).
The first two compile attempts exposed fixture-only MLS key-package/Context ownership mistakes
(`voice-low-all-1.log`, `voice-low-all-2.log`). The third ran 350 passing and two failing cases:
one incorrectly expected a still-open Workflow's protected engine lane to disappear; the other
mutated a roster without its epoch, correctly refusing freshness (`voice-low-all-3.log`). Corrected
fixtures assert original reservation lifetime and actual epoch/member rejection semantics.
No production rejection, deadline, capacity or assertion contract was loosened.

The frozen previous source `53191c87` passes its full root suite: **3,096 passed**, zero failures,
24 existing ignored across 57 summaries (`voice-hello-root-full-2.log` in atomic-intake checkout).
Its full unchanged frontend suite passes 2,254 tests (`voice-hello-frontend-full.log`). Frozen
Clippy found only the high-intake argument count; the low increment narrowly allows that lint
with its independent-capability rationale. The combined strict Clippy first found a default-value
replacement style lint (`voice-low-root-clippy-1.log`), corrected to `mem::take`. Strict whole-root
Clippy now passes (`voice-low-root-clippy-2.log`), and the final focused rerun passes all 352 cases
in 17.81s (`voice-low-all-5.log`). The full frontend suite passes **2,254 tests**, zero failures in
75.82s (`voice-low-frontend-full.log`). Root/native formatting and the ambient gate pass. Complete
combined root/native suites and native check/build/startup remain pending; earlier results are not
attributed to this increment.

This is no live dispatch, IPC/browser/RTC, Proof/Finish, relay or physical acceptance claim.

Private Download resume preserves verified encrypted acquisition progress and its spent execution
allowance across restart. External export uses a fresh destination; continuing a partly written
external plaintext file at its old offset is not implemented.

## Refreshed TLS audit correction (20 October)

The audit database now reports rustls-webpki and webpki-root advisories. Both workspace locks
move to verifier 0.103.16 and roots 1.0.10; the native DER bundle also moves to 1.0.10. No source,
feature, licence or API changes accompany these patch releases. The 0.26.11 roots compatibility
shim only re-exports the fixed parent; its advisory exception is paired with stable and explicit
published-prerelease version bans. Root deny passes (`voice-hello-root-deny-final-2.log`); actual
metadata-only vulnerable 1.0.9 and 0.26.0-alpha.2 fixtures are rejected by bans independently of
the exception (`voice-hello-tls-downgrade-final.log`, `voice-hello-tls-prerelease-final.log`).
Read-only review caught the ordinary-semver prerelease gap; the explicit family ban resolves it.
See [the correction and limits](TLS-AUDIT-2026-10-20.md).

The separate native audit still fails on existing h2/proc-macro-error/quick-xml/rust-unic/zip
findings (`voice-hello-native-deny-final-2.log`). No new exception suppresses them. The separate
IPC experiment audit also remains open. The first audit attempt was sandbox-blocked on the
advisory-db lock, and the first native final attempt was offline before its new DER artifact
was fetched. Those are not accepted audit results. The initial full Rust run was deliberately
stopped before tests to avoid validating obsolete locks; the frozen full root result is recorded above.

## Protected native Hello consumer (20 October, live dispatch disabled)

The first protected high-side Hello consumer now composes original origin allowance, replay/token,
decrypt action and credited admission receipt under one nonblocking final transaction. Root uses
one public/encrypted immutable identity classifier and combined 256-entry limit. There are three
independent candidate lanes, even when all plaintext results are malformed; metadata/history
retains no Workflow or payload/engine. The fixed receipt split explicitly includes its allocated
EffectAuthority and Arc counters, within the original 1,536-byte row and 2 KiB shared metadata.

Final focused `voice_` tests pass 315/315 in 26.56s (`voice-live-integration-all-2.log`), including:

- Actual Root signed outer request and endpoint-admission ACK, exact lost ACK after token use,
  public/encrypted ID substitution in both orders, and equal-join replacement call refusal.
- Real Root MLS addition with both endpoints still members: the queued driver reply immediately
  loses authority, first result has no effect, and response/engine/reserved custody retire separately.
- Actual Root document room deletion/recreation: old call, queued reply and decrypt cannot revive.
- Three malformed candidates and fourth refusal without another replay/token/action/quota effect;
  consumer plus actual engine admission under full ordinary pressure after a prepaid verification
  ticket; final History contention with one remaining original allowance and exact fresh retry.
- Clock outside final Group/Policy guards, nonblocking contention with fresh retry samples,
  pristine-pair-only selection, no ordinary fallback after valid selection/capacity refusal,
  and original-epoch first-result checks in both directions with exact terminal history recovery.

Earlier focused attempts are retained: the Clock regression failed before correction
(`voice-live-origin-clock-before.log`); a naive history layout exceeded 2 KiB and was replaced by
compact row/ticket state plus fixed weak-action tracking (`voice-live-candidate-layout-2.log`).
Root test compilation initially missed a test-only RngCore import (`voice-live-root-hello-1.log`).
The next Root run passed 7/7 (`voice-live-root-hello-2.log`). A candidate test first confused pending
pulls with retained terminal rows (`voice-live-protected-all-3.log`: 259 passed/1 failed), corrected to
assert current pending actions and retained receipts separately. An epoch test first expected the
live chain's reserved execution position to vanish at engine settlement (`voice-live-integration-
all-1.log`: 313 passed/1 failed). It now verifies loan return to the still-admitted chain, then actual
release at End. No runtime refusal, quota, row limit or assertion contract was relaxed.

Read-only design/implementation review found no remaining functional blocker/high/medium in this
Hello-only increment. Its requested effect-allocation accounting correction is included and its
layout guard passes. Frozen full root/frontend results are recorded above; combined native gates
remain pending. Production kind28 dispatch, Proof/Finish, SDP phase transfer, native/browser
IPC/RTC, helper compatibility, media relay and combined/physical acceptance remain unfinished.

## Native event-drain follow-up (20 October)

A final production-lifecycle review found a HIGH missed by the earlier upload tests: the real
Studio/Settlement event branch waited for UI-session commit, while admitted upload publication
could own that guard and wait behind a saturated actor event sink. Lock then joined the cycle.
The new exact-name regression fails before correction (0 passed/1 failed, 0.35s,
`upload-event-drain-before-exact.log`). The earlier short-name `--exact` invocation matched zero
tests and is not regression evidence.

Separate commit `985c1f1d` gives Studio event emission a short fence. Generation is captured before any await,
and current generation/closed state/registry incarnation are checked before synchronous emission.
Lock revokes voice/mapping/inline-read owners before its first await, crosses and drops the event
fence, then waits for command publication. A review correction preserved that pre-await revocation
ordering. Studio still drops its vault lease before any EventSink wait.

The initial 14 upload tests pass in 3.43s (`upload-event-drain-focused-2.log`), including a genuine actor
Studio event, a saturated sink, admitted publication and actual lock with automatic forwarding
rather than a manual drain. Tests also cover old authorization after waiting for the short fence
and actual lock waiting for synchronous emission. The extended real voice bridge test verifies
call lease, mapping owner and inline cancellation are revoked while both fences remain held;
it passes (0.36s, `upload-event-drain-revocation.log`). Old-incarnation/locked event checks also
pass (0.36s, `upload-event-drain-incarnation.log`). Re-review has no remaining findings. The first
expanded test compile failed on two fixture-only type/capture issues, corrected without dependencies.

The complete root retry passes 3,079 tests, zero failures, 24 existing ignored across 57 summaries
(`chat-bubbles-root-full-2.log`). Root sources are unchanged by this native-only follow-up. The first
native follow-up run (`upload-event-drain-native-full.log`) was deliberately stopped, and is not accepted
validation: re-review found that the event wait preceded native pending-snapshot registration.
Cancelling that waiting lock could lose its JSON and let a later snapshot-less close succeed.
The wait now follows snapshot registration and nonresumable state, before the long commit.
The strengthened valid snapshot test uses actual lock preparation/cancellation and verifies exact
disk bytes before close; a new malformed case requires an error, no destruction and retained debt.
Re-review closes that MEDIUM. The final 14 upload cases pass in 6.19s
(`upload-event-drain-final-focused-2.log`); valid cancelled-lock, malformed cancelled-lock and
pre-await owner-revocation cases each pass (`upload-event-drain-valid-close-2.log`,
`upload-event-drain-malformed-close-final.log`, `upload-event-drain-final-revocation.log`).
The malformed fixture first expected an absent first-run file to error, contrary to the store's
documented empty-state contract (`upload-event-drain-malformed-close.log`). It now starts with
valid saved continuity and requires byte-for-byte preservation on refusal. A second short-name
`--exact` invocation matched zero cases (`upload-event-drain-malformed-close-2.log`); only the
subsequent fully named run counts. No runtime check or rejection assertion was weakened.
The fresh complete native run passes 443 library tests in 895.76s plus five ACL cases, zero
failures (`upload-event-drain-native-full-2.log`, one compiler job/two test workers). Native
check passes in 43.62s (`upload-event-drain-native-check.log`) and debug rebuild in 1m55s
(`upload-event-drain-native-build.log`). Actual startup passes with normal OS access and the
original 20-second deadline (`upload-event-drain-native-startup-2.log`). The first sandboxed
startup timed out without the setup marker (`upload-event-drain-native-startup.log`); its
owned leftover Vite processes were identified and stopped before the unchanged retry. No
source, deadline, assertion or application security setting changed between these attempts.
The final complete frontend
passes all 2,254 tests in 84.44s (`upload-event-drain-frontend-full.log`). Its source/scripts/manifests
remain identical to the image-viewer integration, so the zero-warning Svelte check, production
build and ten rendered flows below still cover that frontend. The earlier native/build/startup
evidence below precedes this correction and must not stand in for its final native gates.

## Automatic chat bubbles and V2 packet capacity (20 October, continuation)

The new native TCP scenario passes twice (81.59s then 78.07s after an additional dependency-closure
assertion; `chat-bubbles-native-first.log`, `chat-bubbles-native-final.log`). Three isolated pairs
write from one saved base with all gossip dropped. It combines owner deletion against a concurrent
author edit, an independent retained edit/pin/three reactors, an MLS removal and automatic recovery
of a previously unseen original pre-removal signature. A replacement bridge serves returning
members with the original A/B authors offline. Assertions compare original signed-operation unions, an
independently assembled visible projection, dependency-derived heads, current membership/epoch,
and production freeze/save followed by solo reopen. Read-only review has no remaining finding.
This is six devices and one membership transition, small unpaged history, sequential causally
independent pairs and graceful close. It does not establish 50 simultaneous network peers or
abrupt-crash custody. See [the coverage trace](CHAT-HISTORY-CONVERGENCE.md).

The inactive V2 native chain now also protects logical packet-table positions, using the actual
pair role and the existing 32-row limits: low 3-in/6-out, high 6-in/3-out. Actual ordinary incoming/
outgoing rows own positions separately from bytes; close returns unused reservations, not actual
loans. Partial reservation failure returns unused claims. The focused V2 run passes 249 cases in
10.65s (`voice-v2-packet-capacity-final.log`), including actual ordinary packet APIs under pressure,
cross-ledger refusal, replay/serial preservation, constructor/close and a retained-owner layout
growth guard within the existing 2 KiB allowance. This structural estimate excludes allocator slack
and framework/engine heap. Read-only review has no remaining finding.
Protected packet/event allocation, candidate history lanes and atomic high-side Hello remain
unimplemented; no unsupported protected role or live V2 path is enabled.

Gate7's newer clip-editor commits were merged at `7e90f8b1` with the communication session guards.
That frontend passes 2,227 tests in 85.92s, Svelte check (zero errors/warnings) and
production build (`chat-bubbles-frontend-after-merge.log`, `chat-bubbles-frontend-check.log`,
`chat-bubbles-frontend-build.log`). A focused 53-case frontend run includes the post-now outcome
and session-retirement combinations. A subsequent merge of Gate7 `fd788d80` retains dispatch
evidence when Pending messages removes an in-flight entry; its initial 52 focused send/upload-flow
cases pass (`chat-bubbles-latest-base-focused.log`). Read-only review then found a remaining
ambiguity: a definite refusal of a retry could erase an older lost-response outcome. A new actual
composer regression fails before correction (15 passed/1 failed,
`chat-bubbles-prior-send-before.log`). Separate prior uncertainty now survives both thrown and
structured refusals, including manager recover/cancel and background warnings; first-attempt
definite refusal stays failed. The final 54-case focused run passes in 1.86s
(`chat-bubbles-prior-send-final.log`). The final complete frontend at `10dc3d23` passes 2,230
tests with two workers in 72.99s (`chat-bubbles-frontend-final.log`), Svelte check with zero
errors/warnings (`chat-bubbles-frontend-check-final.log`) and production build in 1m12s
(`chat-bubbles-frontend-build-final.log`, ordinary chunk-size warning). All ten rendered Edge
flows pass (`chat-bubbles-frontend-flows-final.log`), including send, search, friend acceptance,
pending recovery and boot-failure surfaces.

Earlier browser flows failed before assertions: the first
attempt could not obtain its owned Edge debugger, and later attempts reached the document but
not fixture readiness. Local network diagnostics show the App module request still pending,
without an application exception. The successful final run retains the same deadline, actual
fixture readiness marker and all assertions. Earlier failures remain failed attempts.

Gate7 subsequently advanced to `f87058f0` with the image-viewer changes. Integration commit
`88b87dfe` preserves those changes and both handover sections; the App merge leaves communication
handlers unchanged. At `88b87dfe`, `git diff --quiet 10dc3d23 88b87dfe -- Cargo.toml Cargo.lock
crates bins apps/desktop/src-tauri` passes, so the earlier native/full-root results apply to that
integration's identical backend sources. The later native event-drain correction has its own gates above.
The complete merged frontend passes 2,254 tests in 102.20s
(`chat-bubbles-frontend-lightbox-merge.log`), Svelte check with zero errors/warnings
(`chat-bubbles-frontend-lightbox-check.log`), production build in 47.64s
(`chat-bubbles-frontend-lightbox-build.log`) and all ten rendered flows
(`chat-bubbles-frontend-lightbox-flows-2.log`). The first flow attempt lost its Edge debugger
during `Runtime.enable`, before application assertions (`chat-bubbles-frontend-lightbox-flows.log`).
The unchanged retry adds debugger tracing; deadlines, fixture markers and assertions are intact.
Refreshed native build passes in 1m38s (`chat-bubbles-native-lightbox-build.log`), as does actual
startup with the original 20-second deadline (`chat-bubbles-native-lightbox-startup.log`).
Read-only integration review has no findings. The image-viewer's Linux/WebKitGTK, main-thread
resampling and silent-canvas-failure limits remain documented in HANDOVER.

The merged base disclosed a native streamed-upload/lock race. Separate pushed commit
[`c4005b89`](https://github.com/Thalpy/Mewtual/commit/c4005b89) retains
original generation/actor/instance ownership and transfers final publication into an owned worker
under session commit. It releases commit before ordinary persistence to preserve leave's lock
order. Ten focused cases pass in 2.18s (`chat-bubbles-upload-focused-2.log`): both lock/publication
orders, caller loss with actual saved-server restore, lock/unlock, uploads-map waits, server
replacement, quota/failure cleanup and persistence lock ordering. Native formatting and read-only
re-review pass. The first compile attempt failed only in the new test helper (unavailable direct
`futures::poll!` import); the corrected helper uses the standard library without a dependency change.
Native check, debug build and actual startup subsequently pass as recorded below. The complete
native suite at that checkpoint also passed; integration of both native corrections into Gate7
remains outstanding. Final event-drain validation is recorded above.

The first complete-root attempt (`chat-bubbles-root-full.log`) stopped in the replication library
with `memory allocation of 1787552 bytes failed`, without a reported test assertion failure.
The native check also failed an allocation of 2 MiB (`chat-bubbles-native-check.log`). Another
session was observed running full Rust builds during validation; after the failures, system
committed memory was approximately 128 GiB against a 150 GiB limit. These observations identify
allocation failures and concurrent load, not a proven source-level defect or a successful full
run. The single-job native check retry passes in 4m45s (`chat-bubbles-native-check-2.log`);
the single-job debug build also passes in 11m53s (`chat-bubbles-native-build.log`). Actual startup
passes against that rebuilt executable, with the original 20-second deadline
(`chat-bubbles-native-startup.log`). The complete native suite then passes 438 library tests in
880.98s plus five ACL tests (`chat-bubbles-native-full.log`, one build job/two test workers).
It includes the ten upload/lock cases, automatic-notification endpoint receipt, onward channel
discovery, chat bubbles, indirect files and durable resume. Strict whole-root Clippy passes with
warnings denied in 1m47s (`chat-bubbles-root-clippy.log`).
After the competing full suite stopped, the complete root retry passes 3,079 tests with zero
failures and 24 existing ignored across 57 target summaries (`chat-bubbles-root-full-2.log`, one
build job/four test workers). This successful result is separate from the failed first attempt.

A read-only design review for the next protected Hello step identified two enablement gates,
now recorded in [VOICE-NATIVE-RESULTS-NEXT](VOICE-NATIVE-RESULTS-NEXT.md): the existing origin
seam still samples Clock under Group/Policy, and the confirmation first-result path lacks the
protected chain-epoch check already used by derivation. No protected confirmation action or live
V2 intake was enabled to bypass those gaps. These notes are design evidence, not new runtime
acceptance.

## Chat history adversarial checkpoint (20 October)

This checkpoint adds 20 chat tests: 15 application cases, four signed-history
boundary cases and one rejected-page case alongside three existing paging checks. The trace and
compatibility/audit limits are in [CHAT-HISTORY-CONVERGENCE](CHAT-HISTORY-CONVERGENCE.md).

Completed checks:

| Check | Result | Local log under this checkout's `target/` |
| --- | --- | --- |
| Application chat history | 15 passed, 3.54s | `chat-history-adversarial-after-2.log` |
| Signed-history boundaries | 4 passed, 0.17s | `chat-history-writer-boundaries-after-1.log` |
| Replication document/index/paging | 14 passed, 61.35s | `chat-history-replication-doc-after-2.log` |
| Requester paging | 4 passed, 143.46s | `chat-history-pages-after-1.log` |
| Complete frontend | 2,067 passed, 77.36s | `chat-history-frontend-full.log` |
| Root dependency policy | Passed | `chat-history-root-deny.log` |
| Strict whole-root Clippy | Passed, 2m51s | `chat-history-root-clippy-2.log` |
| Native check / debug build | Passed, 1m28s / 4m33s | `chat-history-native-check.log`, `chat-history-native-build.log` |
| Actual native startup | Passed, original 20s deadline | `chat-history-native-startup.log` |
| Rendered browser flows | 10 passed | `chat-history-frontend-flows.log` |
| Corrected file-bridge fixture | 1 passed, 8.80s | `chat-history-file-bridge-after.log` |
| Strict Clippy after fixture correction | Passed, 31.41s | `chat-history-root-clippy-3.log` |
| Root dependency policy after fixture correction | Passed | `chat-history-root-deny-2.log` |
| Disconnect fixtures and stale-event regressions | 6 passed | `chat-history-disconnect-fixtures.log` |
| Strict Clippy after disconnect fixtures | Passed, 27.31s | `chat-history-root-clippy-4.log` |
| Complete native | 430 library passed, 569.80s; 5 ACL passed | `chat-history-native-full.log` |
| Complete frontend, final repeat | 2,067 passed, 68.63s | `chat-history-frontend-full-final.log` |
| Complete root, final repeat | 3,073 passed, 0 failed, 24 existing ignored, 57 target summaries | `chat-history-root-full-4.log` |

Two signed-history cases intentionally characterize still-open same-device equivocation and
missing original-group signature binding. They are not passing security acceptance for those
properties. The 20/50-member cases are deterministic semantic replicas; the ten-epoch MLS case
explicitly delivers commits/history. Neither is independent physical or automatic 50-peer
network acceptance.

Before the fixes, the application run failed six of seven cases, the writer test accepted forged
modern identity-first (3 pass/1 fail), and the rejected-page test incorrectly advanced cursor 7 to
10 (0 pass/1 fail). Their logs end in `before.log` / `before-2.log`. An already-failed broader
replication run was stopped during unrelated Studio cases. Existing synthetic-writer fixtures
were migrated to actual signers, preserving their graph/assertions; the final document cases pass.

The first strict root Clippy attempt selected an older library from the shared target and could
not see ActorMismatch. Only first-party fingerprint metadata was preserved/invalidated for a
correct-source rebuild; no target tree or third-party cache was deleted. Strict Clippy and native
check/build/startup now pass. All three mandatory application suites pass after the failed
attempts detailed below. Read-only adversarial review finds no new code issue;
legacy schema authorization, actor namespaces, poisoned snapshots and audit limits remain explicit.

The first complete-root command failed after the application binary: 1,199 passed, one failed,
22 ignored, 4,432.38s (`chat-history-root-full.log`). The Studio archive-budget test failed during
setup at `rotation/overlay.rs:166` with `Io("out of memory")`, before preservation/release or its
budget assertions. Read-only review finds a resource-failed attempt, not a demonstrated code
defect; the exact originating I/O operation and cause remain unknown. The same executable passes
four exact standalone runs (26.34s, 47.55s, 82.20s, 109.16s; `chat-history-studio-budget-reproduction.log`
and suffixes `-1` through `-3`). None of these clears the failed complete attempt.

The second full attempt passed that archive case but failed the new file-bridge fixture's terminal
drain (`chat-history-root-full-2.log`). Its exact standalone reproduction fails in 10.35s
(`chat-history-file-bridge-reproduction.log`). Successful completion readied the reply and emptied
the queue; the unbiased test select could unwrap the empty queue before observing success.
Read-only review confirmed this fixture race. The already-failed full attempt was stopped and
is not a complete result. The corrected drain prioritizes the terminal reply, retaining every
error/prefix/deadline/budget/byte assertion; no production behavior or limit changed. Its focused
case passes in 8.80s and the re-review has no findings.

The third root attempt completed with 3,069 passed, four failed and 24 ignored across 57 result
summaries (`chat-history-root-full-3.log`, two build jobs/eight test threads). All 1,200 application
cases passed. The four failures were sync test fixtures delivering a disconnect while the actual
Mem transport remained connected; the production guard correctly treats that event as stale.
They now drop the endpoint and assert actual loss/revoked lease first. Reconnects retain a fresh
registration and prove its live distinct lease; every prior assertion remains. Read-only re-review
finds no issue. Existing separate tests cover stale old edges preserving replacement readiness
and refusing artificial reconnect work. All six focused fixture/stale-event checks pass, as does
strict Clippy. The corrected complete-root run passes 3,073 cases, zero failures and 24 existing
ignored across 57 target summaries (`chat-history-root-full-4.log`). All four fixtures and both
previously failed application cases pass inside this full run. Its application binary passes
1,200 in 1,775.19s, replication 403 in 269.38s and sync 752 in 121.99s. The 10,000-operation
cold-provider regression transfers one operation in one page, 590 payload bytes.

The other session's full suite has stopped. Complete native testing passes 430 library cases in
569.80s plus five ACL cases (`chat-history-native-full.log`, two build jobs/four test threads),
including automatic notification receipt, onward-directory discovery and all active-read restart
scenarios. The latest four fixture changes only compile in root unit tests, not native dependency
builds. The final frontend repeat passes all 2,067 cases in 68.63s
(`chat-history-frontend-full-final.log`). Runtime source is unchanged across the full attempts;
only reviewed fixtures changed. Final tracked-diff and all four new source fingerprints match
the frozen root inputs. These local passes do not assert hosted CI or completion of the programme.

## Active-read recovery checkpoint (20 October)

This continuation was developed in the local
`M:\Git (local)\CatComs\target\communication-atomic-intake-worktree` checkout, following
checkpoint `202c17f141fc58bcd3e3ee4ee5d1db073c6bc151`, for `chat-update-implementation`. See
[the current handover](../HANDOVER.md) for the resume order and checkout boundaries.

The first full native run passed 426/429 library cases. Its two new active-read failures exposed
missing background reconnect work after the startup discovery pass; a third failure came from a
fixture that had closed its supposedly connected unresponsive peers. The production correction
adds a finite, bounded retry window for an authenticated member's actual connection loss, using
only existing consented saved routes and shared dial budgets. The corrected fixture retains its
connections and checks current outbound finalization readiness.

All three focused real-TCP active-read cases now pass: Keep, private Download, and private
Download with B returning after a confirmed local dial admission while absent. Each recovers the
original invocation and exact file without another Resume request or replenishing durable work.
Log: `communication-chaos-native-active-focused-7.log` (14.53s). Both focused reconnect-selection
cases pass in `communication-chaos-native-capture-focused-2.log` (5.92s). All logs in this section
are under the isolated atomic-intake checkout's `target/`.

Local dial admission does not prove terminal socket failure: the admitted attempt may connect
when B returns. A real failed first dial followed by a second successful dial is not established
by that case. Separate injected-clock scheduling/deadline cases (three), actual command-custody
cases (two), and current finalization/loss cases (27) pass. The actor prefix test checks retained
bytes/accounting and exact eventual bytes; direct capture of its next signed wire offset remains
a low-priority coverage follow-up. The complete root/native/frontend passes are recorded above;
they do not complete live V2, media relay or independent physical audio/network acceptance.

## Previous combined checkpoint validation — `202c17f1` (20 October)

The integrated Gate7 UI, inactive token/origin transaction and standalone framing probe are
validated together. Production root Rust is identical to frozen `5f5f20cc`; the full command
completed there with exit 0, **3,035 passed, zero failed, 24 existing ignored, 56 target summaries**
(`communication-native-origin-root-full.log`, driver checkout). Strict whole-root Clippy passes
(`communication-native-origin-root-clippy.log`, 82s); root formatting, ambient-dependency gate
and root cargo-deny pass. The initial deny attempt could not obtain its advisory-cache lock in
the sandbox; the authorized retry passed, without changing policy.

The combined application checks pass 427 native library + 5 ACL cases, native check/build/startup,
2,067 frontend cases, Svelte 0 errors/0 warnings, production frontend build, ten standard rendered
flows and the separate Download/Keep/Gate7 flows. Final native and frontend logs are
`communication-combined-origin-native-full.log` and `communication-combined-origin-frontend-full.log`
in the integration checkout's `target/`; native startup is `communication-combined-origin-startup.log`.
The standalone probe separately passes 9 Rust and 17 Node tests, strict fixture Clippy, real packaged
framing/keyless-takeover runs and cleanup-fault checks. Its **additional dependency audit fails**
as described below; it is not included in the passing root cargo-deny claim. Existing frontend
bundle-size warnings remain. No remaining code-review finding exists in these included increments.

The user's 20 October priority is graceful functional recovery under valid delay, duplication,
reordering and bridge churn. Subsequent review found production file-route/capacity stranding
and disabled-V2 retry/late-positive-ACK gaps. Those fixes are a separate isolated continuation,
not part of this validation. Live V2, production media relay and physical acceptance remain open.

## Packaged IPC platform experiment (19 October)

The subsequent encrypted-framing fixture also passes on actual packaged WebView2. Two8192-byte
plaintext patterns round-trip through directional HKDF/HMAC/AES-GCM; altered header/body and exact
replay refuse. The native process sees three authenticated admissions, two executed bodies and
two replies. Distinct finite bootstrap seeds prevent browser-counter reset under one key. One
64-admission ceiling covers both immutable slots; body EOF/timeout/bad AEAD never refunds it.
Four header and two body positions remain held through actual completed-worker collection.

An independent keyless responder then binds the exact native port after both listeners/workers
drain. It receives one8208-byte ciphertext body without the8192-byte plaintext pattern and returns
HTTP200 with a syntactically fresh forged reply. Browser diagnostics pin that its HMAC check
actually ran and refused; stale-sequence refusal alone cannot pass the oracle. A forged maximum
sequence also cannot advance the accepted reply floor. Actual native/keyless/profile/output
cleanup precedes evidence. The final cleanup-fault run refuses publication and preserves the
previous record. Logs: `communication-loopback-framing-takeover-native.log` and
`communication-loopback-framing-cleanup-fault-final.log` in the probe checkout's `target/`.

Nine Rust and17 Node tests pass, plus strict fixture Clippy/formatting and the packaged build.
Review corrected input mutation across HMAC, stalled-read cancellation, dribble-renewable socket
timeouts and a stale-reply false-positive oracle; regressions pin each. Final read-only re-review
has no remaining code finding. A later final-handoff review removes redundant asynchronous body
cancellation after the last plaintext check. The new regression fails with the isolated prior
condition and passes with the guard; the rebuilt packaged run passes again. Logs:
`communication-loopback-handoff-before.log`, `communication-loopback-framing-node-handoff-final.log`
and `communication-loopback-framing-native-handoff-final.log`. The additional standalone deny audit **fails** on existing pinned
Tauri maintenance and quick-xml/plist advisories, including with the Windows filter. No exemptions
or package-version changes were added; the README records the source reachability observation.
This audit follow-up, production action authority and full copy/phase custody remain open. These
fixture results do not enable V2, establish RTC/media behavior or certify general confidentiality.

The disposable Windows harness at `experiments/loopback-ipc` passed its actual packaged
WebView2 run on `Edg/155.0.4283.62`. Main/restricted/remote WebViews exercise real generated
Tauri ACLs. After injected successful bootstrap reply loss, stock fallback consumed the second
and last process-lifetime fixture grant; a third call and hard refresh refused another grant.
CSP denied the unlisted listener before it received any request. The wrong-CORS listener
received its expected request but the browser withheld its reply. The healthy fixed GET passed.
Two ordinary preflights were seen; zero Private Network Access preflights were observed, so no
PNA permission result is claimed. Peak active workers were three, under the shared four-position
cap that persists through completed-task collection.

The runner waited for native worker drain, process exit, captured-output EOF and owned-profile
cleanup before publishing ignored evidence. A second real run injected cleanup failure and
correctly failed without printing success or replacing the prior record. Three Rust tests and
two Node tests pass. Read-only adversarial re-review has no remaining finding. The probe uses
only generated fixture material and fixed replies, with no account or application payload.

Logs: `communication-loopback-probe-native.log`, `communication-loopback-probe-cleanup-fault.log`,
`communication-loopback-probe-unit.log` and `communication-loopback-child-output-tests.log` in
the isolated loopback checkout's `target/`. This standalone workspace preserves pinned native
registry versions and changes no production manifest, command, CSP or wire format. This paragraph
records the earlier platform-only checkpoint; the later framing evidence above does not complete
copy custody or production V2 integration.

## Gate7 integration validation (19 October)

Gate7 `e9998678` is integrated with the communication branch, preserving the incoming video,
upload/drop, readability and Studio save/closure work. Read-only adversarial review identified
four medium lifecycle/boundedness gaps and one low emoji-completion gap in the incoming paths;
all are corrected and re-reviewed with no remaining finding. New regressions cover wiki pane
replacement, held close, lock/unlock, exact leave-operation replacement, current server closure
while an older leave is pending, overlapping/oversized batches and bounded review teardown.

Vite's native-tree watcher exclusion is checked against its actual configured Chokidar admission
predicate. The test fixture disables its HMR WebSocket listener. This proves path admission,
not OS notification delivery or the cause of earlier drive activity.

Complete frontend:2,067 passed (`communication-gate7-integration-frontend-final-4.log`). Complete
native:427 library +5 ACL passed (`communication-gate7-integration-native-full.log`,613.71s).
Svelte has0 errors/0 warnings; production frontend build and native check/build/startup pass.
Ten standard browser flows pass. Separate actual-render scenarios pass for private Download
resume/Pause/Retry, retained-copy controls, and Gate7 inline playback/looping/Studio close.
Their logs and900x640 screenshots are under the integration checkout's `target/`.

The first browser harness attempt timed out while Vite prepared dependencies; a later kept-copy
attempt timed out before app mount. Unchanged readiness/timeout retries passed; neither failed
attempt reached its behavioral acceptance. Initial focused invocation omitted the Svelte preload;
the corrected command passes. Intermediate close-fixture dependency and declaration-order
failures were fixed without relaxing their assertions. Production bundle-size warnings remain.

At the original UI merge, root Rust source was unchanged from the 3,033-pass checkpoint below.
The later token/origin prerequisite is covered by the combined 3,035-pass result above. Live V2, production
media relay and independent-network/audible acceptance remain open.

## Validated checkpoint 3691bd25 (19 October)

All required complete checks have now passed for this checkpoint:

| Check | Result |
| --- | --- |
| `cargo test --all --all-features -j2 -- --test-threads=4` | 3,033 passed, zero failed, 24 existing ignored, 56 target summaries; exit 0 |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml -j2 -- --test-threads=4` | 427 library + 5 ACL passed; main/doc targets passed |
| `npm --prefix apps/desktop test` | 1,903 passed |
| Root strict Clippy, all targets/features | Passed in 38.89s |
| Root/native formatting, ambient-dependency gate, `cargo deny check` | Passed; existing dependency-duplication warnings remain |
| Native check and debug build (`--no-default-features`) | Passed |
| Svelte static check and production build | Passed; zero static errors/warnings, existing build chunk-size warning |
| Actual native startup and ten rendered Edge flows | Passed |

The root command ran on the frozen native continuation worktree's Rust source at `c569b3c9`;
`3691bd25` adds browser code/docs without changing that Rust source. Its log is
`target/communication-v2-continuation-root-full.log` in `communication-live-v2-worktree`, alongside
`communication-v2-continuation-root-clippy.log`. Native and browser logs are listed in their
dated sections below. The root result was summed from the completed command after exit 0;
no running test, stale shared-target binary or zero-match filter is counted as passing evidence.

The new atomic-confirmation intake work is a separate uncommitted continuation and is not covered
by this checkpoint. Live V2, production media relay, combined rollout and independent physical
audio/network acceptance remain open. Green checkpoint tests do not complete those packages.

## Inactive protected browser derivation (19 October, isolated continuation)

Broker v5 shares one closed action driver across derivation and confirmation. Its opaque key
handle, accepted native local join/key/public point, complete frozen native pair setup and exact
chain/epoch/device/join/key/handshake/generation/role/deadline binding precede protected row0
publication. The plan protects twelve combined row claims (24 KiB fixed history),96 KiB variable
custody,8 KiB metadata and a real execution position; only derive consumes a claim in this slice.
Other published records plus unused claims stay at32. The ordinary queue can be full without
occupying the protected position. Native engine bytes are checked independently against the
actual `3 * transcript.length + 4096 + 256` formula.

Lost admission/accepted/rejected replies retry the exact native request without repeating the
ECDH/HKDF pipeline. Historical native acceptance without matching local keys cannot reconstruct
them. End/expiry clears pending keys/capacity but keeps actual engine/RPC promises charged until
drainage. Fixed history and finite closed pair stamps remain separate. The derive-only keys
refuse ordinary encrypt/open/confirmation fallback until protected typed consumers are implemented.

The first30 focused new cases passed; review then found a final microtask handoff gap and an
ordinary-confirmation fallback into protected keys. The corrected contract refuses all those
protected fallbacks, and ordinary wrappers repeat their final pair fence (clearing late plaintext).
New actual-AES regressions pin the final handoff gap. Review also caught coercible closed string
fields, now refused before charge/publication. A real maximum-input test exposed Node/OpenSSL's
1024-byte HKDF info limit;987 transcript bytes plus the direction prefix pass, the next byte
refuses before admission. The current native canonical transcript fits; no wire format changed.

These checks use real WebCrypto and simulated native action faults. Native registration/IPC,
complete protected confirmation/SDP rows, atomic encrypted intake, agreed helpers, live dispatch/
RTC and production member media relay remain open. Windows source inspection identified missing
framework/COM reply-owner hooks; app serialization completion cannot close that accounting gate.
See VOICE-NATIVE-RESULTS-NEXT. No physical-network/audible acceptance is claimed on this desktop.

Final browser validation: all1903 frontend cases pass64.17s
(`target/communication-protected-browser-frontend-full-2.log`), Svelte reports zero errors/warnings
(`target/communication-protected-browser-check-final.log`) and the production build passes49.05s
with the existing large-chunk warning (`target/communication-protected-browser-build.log`). This
includes43 new derivation cases and the existing48 crypto/confirmation cases. A prior1903-case
run passed before the last history-only settlement correction; the final run includes it.
Read-only re-review has no remaining blocker/high/medium/low finding. The first static run caught
two union-narrowing errors, corrected before the final check. The initial synthetic4096-byte
transcript failed at real HKDF; it is retained as investigation evidence, not a passing boundary.
Actual native startup also passes with the same20-second marker deadline
(`target/communication-protected-browser-startup.log`); this exercises setup, not live V2.
All ten actual rendered Edge flows pass (`target/communication-protected-browser-rendered-flows.log`).
All91 focused crypto/confirmation/derive cases pass1.35s
(`target/communication-protected-browser-focused-final.log`). The same Rust/native source was
already checked by the427+5 complete native run below; the complete root run subsequently passed
on its frozen native continuation source. No test sources were changed in that running Rust
worktree. The checkpoint summary above records the final completed result.

## Inactive native confirmation capacity (19 October, isolated continuation)

`prepare_crypto_chain` protects twelve local fixed history rows/slots, a provisional96 KiB
confirmation-only variable envelope, one Crypto and two Forward lanes. Only derivation uses
its role row currently. The exact native pair permanently stamps successful preparation;
close cannot reset it. Plan and consumption bind actual resources/slots, scope/epoch, both
join/key identities, pair pin/generation, role and original deadline. A first derive result
checks the captured epoch even without another factory call. Exact past results remain
metadata-only and recoverable before current pending checks.

One weak process/group Workflow index remains busy through actual variable input/engine/lower
custody after End. History and idle row issues do not own Workflow. Close revokes the separate
pending chain lease, releases unused rows/bytes/lanes, and visits weakly registered actions
outside control/history locks. Proven unstarted input retires; admitted uncertain work stays
charged until actual settlement. The shared fixed2 KiB binding/control metadata owner stays
charged through its actual last control, row issue or Workflow owner, without pinning progress.
Runtime housekeeping closes idle expired/ended plans even without an active UI/actor grant.

All287 focused voice cases pass15.48s (`target/communication-v2-chain-voice-2.log`), including
ten chain cases and four LocalJoin integration cases; strict affected all-target/all-feature
Clippy passes31.02s (`target/communication-v2-chain-clippy.log`). The first actual broader run
had286 pass/one failed: the membership fixture removed the local member too, so the existing
owned gate correctly refused before the intended remote-pair revocation. The fixture now
keeps the local member and removes only the peer, exercising the engine-fence path directly.
No production check/assertion was weakened or test skipped.

An earlier filtered command used a shared-target fingerprint whose dependency records still
pointed to the sibling checkpoint worktree and matched zero new cases. It is not evidence for
this slice. Refreshing only affected local artifact fingerprint files forced the actual
continuation build; its first nine chain cases and the final287 voice cases passed. No target
cleanup or dependency rebuild was used to hide a failure. Review identified and corrected the
fixed-metadata lifetime and direct epoch-check gaps described above.

Complete continuation validation passed at the checkpoint above. This is not native/browser
registration readiness, protected confirmation-operation factories, atomic encrypted intake,
measured SDP phase transfer, live V2 dispatch/RTC, media relay or physical acceptance. The new
media candidate section also records the mux pre-allocation/retained-copy boundary that must be
resolved before enabling a production relay; no replacement mux profile is selected.

Continuation validation update: all20 native authority cases pass0.98s after rebuilding the
actual isolated worktree (`target/communication-v2-chain-native-authority.log`). The complete
native command passes427 library and5 ACL cases in645.42s, with main/doc targets also passing
(`target/communication-v2-continuation-native-full.log`). The complete frontend command passes
1860 cases in67.55s (`target/communication-v2-continuation-frontend-full-3.log`); Svelte reports
zero errors/warnings, the production build passes56.92s and all ten rendered flows pass. Their
logs are `target/communication-v2-continuation-{svelte-check,frontend-build,rendered-flows}.log`.
Formatting, the ambient-dependency gate and `cargo deny check` pass; existing duplication warnings
remain. Native `cargo check` passes51.40s and the debug build with `--no-default-features` passes
3m08s (`target/communication-v2-continuation-{native-check,native-build}.log` in the native
continuation worktree). The first sandboxed startup attempt timed out at its unchanged20-second
bound with no marker; an unchanged normal-desktop-access rerun passes2.34s
(`target/communication-v2-continuation-startup{-2,}.log`). Both results are retained. Complete
root validation subsequently passed at the checkpoint above. The earlier startup timeout and
passing unchanged rerun both remain part of the record.

The first frontend command had1859 passes and one occupied-port harness failure: app dependency
prebundling used the original20s deadline before Vite tried its strict-port listen. That test now
uses a fresh minimal root/config and native config loading while retaining the installed Vite
CLI, original deadline, strict-port refusal, zero requests to the stranger and stranger-usability
assertions. Re-review also tightened the oracle to require the exact occupied-port diagnostic;
this exposed and corrected a sandbox config-bundling error previously accepted by the broad exit
check. The final five focused harness cases pass1.74s; the final1860-case command above includes
that stronger oracle. Real application compilation/readiness still runs through `test:flows`.
Earlier permissive-oracle results are not used as proof of the final occupied-port path.

## Prepared confirmation binding (19 October, disabled continuation)

Candidate/attempt binding and the original pair deadline are now installed in `PreparedInbound`
before replay/token publication. Final admission checks require that binding for all three
confirmation kinds; the generic application wrapper cannot publish an unbound confirming token.
Higher accepted Hello, attempt replacement and original deadline expiry refuse before origin
debit, replay, serial or incoming-row mutation, including when retirement occurs inside the final
callback. Unpublished abandonment consumes no candidate and returns only its unused byte custody.

All273 focused voice cases pass16.26s (`target/communication-v2-prepared-binding-voice.log`),
including four new regressions. Strict affected all-target/all-feature Clippy passes16.92s
(`target/communication-v2-prepared-binding-clippy.log`). Review found and fixed a generic-ledger
bypass and added inside-callback retirement cases; short re-review has no remaining finding.
The first focused compile caught two fixture mistakes (missing direction and a clock return
type), both corrected. No tests were skipped. Full continuation validation is still separate
from checkpoint80927a85's full run. This is token preparation only: typed complete-chain/native/
browser reservations, unpublished decrypt action and endpoint receipt still need composition.
No action/engine grant escapes this preparation and normal encrypted intake remains disabled.

## V2 exchange/history separation (19 October, isolated continuation)

Fixed Exchange-backed action history no longer retains live phase custody. History and input
are independently prepaid; only variable input and actual engine loans retain the exchange.
Settlement permanently removes the pending lender. Installed terminal outcomes require exact
ACK before input compaction; no-terminal actual settlement and proven unstarted retirement can
drop input because first effects are permanently fenced. Held input readers retain real bytes.
An affine per-direction issue permit is returned only on unpublished abandonment and consumed
at publication. Input compaction cannot issue a second seal or steal the receive row.

All269 focused voice cases pass16.36s (`target/communication-v2-action-ownership-2.log`), all19
native authority cases pass0.98s (`target/communication-v2-action-native-authority-2.log`), and
the final11 action cases, including exact result recovery after Exchange drain, pass0.92s
(`target/communication-v2-action-last-owner.log`). Strict affected all-target/all-feature Clippy
passes18.12s (`target/communication-v2-action-ownership-clippy-2.log`). Read-only review and short
re-review have no remaining finding. Root/native formatting checks pass.

The first focused run caught three old expectations that no-terminal input stayed retained and
one pressure fixture using generic action-stage charging. Existing assertions remain; those
fixtures now hold actual input readers, then verify exact last-reader byte release, and use
typed directional preparation. The first native run caught the same old assumption in two
registry cases; both now preserve all old custody/rejection checks with real held readers.
No tests were skipped. The first strict lint rerun found one obsolete test import, now removed.

This continuation is separate from checkpoint80927a85's complete suites. It does not establish
whole-chain row/browser/IPC readiness, activate V2, implement member media relay, or provide
physical media/network acceptance. Those programme gates remain open.

## Checkpoint80927a85 validation (19 October)

This committed runtime checkpoint implements the native file requester/private durable Download
follow-up and the disabled V2 result/input/reply custody described below. Its full root suite
passes against fixed files; the separate live-V2 continuation worktree is not covered by these
results. No live V2, production member relay or physical media acceptance is claimed.

| Check | Current checkpoint result |
|---|---|
| Complete native suite |427 library and5 ACL cases pass; main/doc targets pass |
| Complete frontend suite |1860 pass |
| Svelte checks and production frontend build |Pass;0 errors/warnings from Svelte |
| Rendered browser flows |All10 pass |
| Separate native workspace `cargo check` |Pass |
| Native debug rebuild and `test:startup` |Pass; actual application setup completes |
| Strict root all-target/all-feature Clippy |Pass |
| Root/native formatting, ambient-dependency check and `cargo deny check` |Pass; existing dependency-duplication warnings only |
| Complete root suite |2,993 pass;0 fail;24 existing ignored;56 target summaries; command exits0 |

Logs are `target/communication-checkpoint-{native-full,frontend-full,svelte-check,frontend-build,
rendered-flows,native-check,native-build,startup,root-clippy,cargo-deny,root-full-2}.log`.
The first root attempt aborted on the registry seed test's nested async fixture stack. The
unchanged shared fixture body is now boxed, preserving all operations/assertions and default
test stack settings. The exact four seed cases and all16 registry-ingress cases pass; read-only
review found no remaining issue. The successful full rerun includes that regression.

The complete-suite commands were `cargo test --all --all-features -j2 -- --test-threads=4`,
`cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml -j2 -- --test-threads=4` and
`npm --prefix apps/desktop test`. Build concurrency and test-thread limits avoid competing heavy
storage workloads; no test filters, enlarged stacks or extended acceptance deadlines were used.
The successful root command includes the large offline-prefix and10,000-message catch-up cases.

## Endpoint media gateway feasibility (19 October, experiment only)

The repeated probe also passes in the freshly rebuilt80927a85 native host: relay/relay,
generated-audio peak0.10104 and28 packets before the cut; jam/drawing delivery;129 accepted
bridge writes /14,618 bytes in5.83s; stop/resume assertions and cleanup pass. Frozen evidence is
`target/member-media-probe-native-rebuilt-19oct-3.log` and
`target/member-media-probe-native-rebuilt-19oct.json`. Two earlier repeats did not mount the
document and sent no TURN packets. The first lacked its dev server; the second was not diagnosed.
Failure diagnostics now report mount state without page content; assertions/deadlines are unchanged.

The retained loopback experiment passes in headless Edge and the existing Mewtual WebView2 host.
Both endpoints select relay candidates; generated audio decodes and jam/drawing fixture messages
arrive. A runner-acknowledged TCP bridge cut stops accepted bridge writes and new endpoint audio/
message delivery; restoring it resumes messages and audio packet progress. The native checkpoint
records 134 accepted writes / 15,165 bytes in 6.23s, with separate frozen native/Edge log and JSON
artifacts identified in [the architecture candidate](MEDIA-RELAY-NEXT.md).

Review found and resolved incomplete setup cleanup, uncapped silent sockets, an inactivity timer
where an absolute role deadline was required, attempted-versus-accepted counter drift, and
ambiguous overwritten run artifacts. Four injected failure/cleanup cases plus high-water refusal
and silent/partial-frame dribble checks pass. Early native runs also exposed an initial-document
navigation race and relative HTTP control reaching the app dev server. The retained probe waits
for the actual mounted app document, keeps its CSP, and uses closed acknowledged runner controls.
Read-only final re-review has no remaining finding for the experiment.

The Node TURN/role handshake is fixture-only. This is no production membership, consent, V2
signalling, mesh-driver, media-key isolation, helper-failover or load acceptance.
Only one physical desktop is available, so independent-network and audible-device acceptance
remain open. A full member relay is still unimplemented.

## V2 input custody follow-up (19 October, not live enablement)

Each action now retains fixed terminal history separately from variable input bytes, with one
fixed two-part credit partition that neither debits nor refunds the pool. Exact receipt ACK plus
actual engine settlement removes the action's input reference. Actual readers use an owned input
lease, keeping buffers and credit through last drop. The32 protective rows, serials, immutable
operation/output bounds and engine fences remain. Exact history recovery works after compaction;
the compatibility sending seam refuses without reconstructing bytes or signing again. This does
not implement IPC-copy accounting, full confirmation-chain prepayment or Crypto-to-RTC handoff.

Focused results:263 voice tests pass in9.72s (`target/communication-voice-payload-focused-4.log`),
including actual accepted/rejected open, derive and seal history after compaction; all19 native
authority tests pass in0.92s (`target/communication-voice-native-compaction.log`); strict affected
Clippy passes in15.36s (`target/communication-voice-payload-clippy.log`). The first broader run had
two stale assertions that expected input bytes to remain charged after the intentional last
lease drop. Assertions now distinguish held readers from compacted history and retain the
mismatched-result protection checks. Read-only implementation review has no B/H/M/L finding.
The complete checkpoint validation above includes this revision. Historical full-suite results
below remain evidence only for their named revisions. Live V2, media relay and physical acceptance
remain open.

## V2 result and reply ownership follow-up (19 October, not live enablement)

Rust history now has exact-output `recover_result` and a separately named metadata-only
`terminal_status` for End after browser-buffer destruction. Both accepted and closed-rejected
outcomes remain recoverable after settlement and LocalJoin loss. The cfg(test) native registry
retains32 combined pending/terminal rows across replacement owners; immutable receipt ACK can
precede settlement, while an acknowledged terminal Arc cannot register again. Three native
history regressions pass (`target/communication-voice-native-history.log`). Production native
registration, independently authorized action IPC and finite serialized reply-copy custody remain
unfinished; no current-call or RTC authority follows from history.

The disabled transport seam now constructs custody-backed typed ACKs and consumes them only
through the original responder. Original verification and exact first helper/public packet
ownership follow byte clones and the driver's separate serialized copy. Original requester,
actor/session/policy, route/proof/descriptor/epoch/connection and absolute request deadline gate
the final send. One native wall/monotonic sample precedes all observing gates, avoiding retirement
inside a later Clock callback. Original responder authority also gates new/cached intake and the
final origin transaction, but is excluded from persistent packets and final sends because receiving
the reply itself closes that wait. Cached helper receipt validity retains its first actor/room/
permit, including zero-edge room fan-out. Another departed room target does not cancel all edges.

All259 relevant voice tests pass (`target/communication-voice-reply-all-focused-3.log`,9.70s),
including actual held Mem request/reconnect, an unrelated-member MLS epoch advance with old
members surviving, negative/positive driver time-sample races, original-request retirement while
waiting for the helper lock, route-capacity refusal, two original watchers, and actual-last-owner
credit retirement. Strict affected Clippy passes (`target/communication-voice-reply-clippy-2.log`).
The initial broad focused run exposed an intentionally changed old expectation: actor replacement
previously returned cached positive status; it now refuses that receipt while preserving identity
and spent allowance. The test retains those protection checks. Read-only re-review reports no
remaining blocker/high/medium/low finding for these changes. These are software boundary checks;
normal kind28 dispatch, encrypted intake, full browser/RTC integration and media relay remain open.

Final aggregate read-only review of the file/resume diff against `ebdd8447` reports no confirmed
B/H/M/L finding. Residual limits include absent physical-device/WebView acceptance and no
power-loss durability claim for the external final directory entry: file data is flushed before
publication, but its parent directory is not. Original encrypted progress is removed only after
successful publication under the current owner; the tests cover process restart and collisions.

Checkpoint80927a85 follow-up (19 October): the native requester is connected to explicit
chunk/range/Keep reads after direct-provider exhaustion. Actual C/B/A handler tests cover signed
page receipt, effective-wall rollback and temporary source pressure. The actual large offline-prefix
case and TCP/native sealed-Keep checks pass, including C reopening and replacement helper D.
This supersedes historical statements below that the requester search was test-only, but is not
complete package4 acceptance. Full-suite evidence at the top covers80927a85; later historical
entries apply to their named checkpoints.
External publication now refuses final-destination collisions and never deletes another local
file on cancellation. Private durable Download ownership/resume is connected through the actual
native storage adapter and actor; completed copies are excluded from shared serving. Restart
retains encrypted whole chunks, original manifests and spent allowance, requires explicit Resume,
and never automatically writes plaintext. Private acquisition through B with A–C blocked, solo
native reopen/revalidation, and streamed export pass over actual TCP; these are same-host tests.
Focused current evidence: 53 storage, 45 workers, 15 native download and 37 frontend cases pass.
Four actual TCP native file scenarios pass, including interrupted private and Keep recovery through
D after C reopens. The four scenarios also pass with an observer-only check of the sealed D→A/C
routes before restarting C; this check adds no dial or hint and keeps the existing setup bound.
Strict affected Clippy, Svelte and rendered900×640 Transfers Pause/Resume/Retry/completion checks pass. No current
full-suite, abrupt-crash external publication receipt or physical-network claim is made. Live V2,
member media relay, combined rollout and physical audio/network acceptance remain open.

Started 18 October 2026 on `chat-update-implementation`, from verified and pushed Gate7 commit
`4d321fef17514c5080f96d98f01be3e86c375333`. Scope and ordering follow
[CHAT-FILES-VOICE-PLAN](CHAT-FILES-VOICE-PLAN.md). This ledger is separate from Gate7 acceptance.
The pre-existing uncommitted invite regression in `apps/desktop/src/joinlog.test.ts` is preserved
outside this programme's changes.

The [PR48 revision ledger](PR48-REVIEW-REVISION.md) records the subsequent transport congestion,
detached automatic submissions, pending mapping, provider rotation, announcement clock, native
build and narrow SDP resource corrections. Those fixes have focused evidence and clear read-only
reviews. The revision ledger records suite results, including the earlier failed full root sweep,
its affected-target reruns and the subsequent fresh successful full root command. Earlier pending/running notes
below are chronological checkpoints. Live V2 remains disabled. The 19 October durable Keep work
below adds scoped resumable storage, paced automatic partial resume, persistent Pause/Resume and
explicit Retry, but does not close file recovery.

The previously pushed 19 October checkpoint b867c622 passes419 native tests and1849 frontend tests,
static/build, browser-flow, native-startup and strict checks. Its root unfiltered command finishes all other
targets with2906 passes and24 existing ignored cases, but exits101 after the single failing
integration executable stalls in a one-turn serving fixture. A bounded reproduction confirms that
fixture defect; the corrected complete target passes all30 cases on the original workspace
feature graph. See the revision ledger's final verification table for exact commands and limits.
Those are historical checkpoint results. The subsequent probe-target rotation closes the narrow
starvation finding, including cancellation after the first admitted helper. The native MLS catalog
and still-test-only file search remove the permanent source-roster cutoff; requester dispatch
remains unfinished. The fresh full root command passes2949 cases with24 existing ignored cases,
the final frontend suite passes1851, and all ten browser flows pass. Strict root Clippy, formatting,
ambient-dependency and dependency-audit checks pass. Final native verification is recorded in the
revision ledger; no new hosted-Linux, live V2 or physical media acceptance is claimed.

Current follow-up: hosted Linux acceptance remains unverified. The latest eight-case Windows
recovery run passes all eight scenarios (277.63 s), including onward channel-directory discovery
and hostile partition/restart exact signed-operation union. The reciprocal-frontier correction
adds bounded reverse reconciliation and fences older empty completions; all 621 affected sync
unit tests pass (113.85 s), with 18 focused recovery-wait cases. Earlier failed runs remain in
the revision ledger. This is local evidence and does not relabel the earlier hosted failure.

The final rendered retained-copy flow passes with exact command, lease and CAS identity oracles
on a fresh fixture port. Its screenshot is `target/pr48-kept-copies.png` in the worktree. It uses
mocked IPC and establishes no native persistence or audio property. A supported absent-key
legacy admission/restore state now refuses Keep before reservation or worker admission; the
root regression and all 24 affected actor tests pass. The first actor rebuild ran out of build
space; removing only verified obsolete app incremental caches recovered 13.6 GiB and the retry
passed. V4 whole-variant storage and worker integration pass40 retained-store and29 file-worker cases,
including saved-set validation, unlisted restart, actual signed-response fallback, interrupted
selection/commit and exhausted Retry history. Read-only re-review is clear. No current full-suite, live V2 or physical media
acceptance is claimed for this WIP.

| Package | Current state | Acceptance still required |
| --- | --- | --- |
| 0: reproduction and baseline | In progress: source trace and read-only design review completed; deterministic regressions being added | Constrained live topology, replacement races, measured recovery/control baseline and redacted native evidence |
| 1: connections and scheduling | Detached connected maintenance and automatic repair; bounded driver delivery; native router/call lifecycle corrections | Broader permitted alternate paths, integrated multi-group congestion latency and full acceptance |
| 2: room/signalling | Reviewed v2 design; focused crypto, signed codec and native ownership implementation; no live V2 or forwarding enabled | Fresh endpoint sessions, confidentiality/forwarding enforcement, legacy isolation, topology and native evidence |
| 3: chat recovery | Exact signed-operation union, automatic new-channel/membership onward recovery and sealed-close restart scenarios pass the six-client TCP fixture | Broader topology/restart matrix and abrupt-crash custody beyond the graceful freeze/save barrier |
| 4: file recovery | Provider rotation, scoped Keep/private Download journals, persistent progress/allowance, distinct Pause/Resume/Retry, actual indirect service and explicit private re-export; checkpoint complete suites and native/rendered evidence | Complete abrupt-crash/cancellation/compatibility/load matrix and combined acceptance |
| 5A: existing TURN feasibility | Native WebView2 loopback experiment observes relay/relay selection and generated audio; production TURN unaccepted | Production forced-relay, physical audibility/network reconnect and unavailable-path evidence |
| 5B: media bridging | No relay implementation or new dependency selected | Architecture/key/authority review, physical-network/audio evidence and resource bounds |
| 6: combined rollout | Not accepted | Combined named scenarios, versioned compatibility, no forbidden edges or downgrade |

## Initial source findings and design constraints

`App.broadcastOn` filters through `onlineCallTargets`; `Server.online_members` reports local
connected-member evidence. `ChannelSync.send_call_signal` addresses one exact peer and does not
forward through another member. Thus A-B-C room/signalling failure is structurally supported,
but source inspection alone is not a native reproduction or audible-media observation.

Read-only review also found RTC callbacks and SDP continuations that can outlive their exact peer
or call join. Regressions must demonstrate replacement, same-channel rejoin and suspended work
before changing those paths. Legacy call signals already lack recipient/nonce-cache binding and
use arrival time for presence; the threat model records that residual. New routing cannot inherit
those checks as upgraded-session authority.

Connected-only deferred work can reuse the reciprocal helper's capture/queue/live-revalidation
pattern, but not its discovery protocol or authority. Group gossip is suitable only for reviewed
group-visible ephemeral announcements. Forwarding plaintext SDP/ICE to B would broaden visibility.
Targeted forwarding therefore waits for an endpoint confidentiality design and review.

An isolated two-endpoint MLS signalling channel is a design option, not an approved implementation.
Existing `MlsDevice.duplicate` and server snapshot APIs copy/serialize the whole provider; generic
group receive can merge control traffic and omits the application sender in its return value.
Safe reuse would need a fresh non-persistable provider owner, application-only authenticated
receive, original-member identity delegation, exact handshake/Welcome retry ownership and bounded
ratchet/session state. MLS alone does not bind the original server membership, room or recipient.
No additional media-encryption claim follows from this investigation.

## Verification policy

Use focused regressions during implementation and sequential full suites before code handoff,
with compilation limited to two jobs. The passing Gate7 suites are baseline evidence only; they
do not qualify future changes on this branch. Record actual commands/results with each package.
Native headless TCP and mocked RTC tests do not establish physical NAT or audible WebView results.

## Package 0 / initial RTC correction checkpoint

The actual-App RTC tests reproduced four failures before the fix: retired audio attachment,
obsolete local offer submission, an old remote-offer continuation consuming replacement ICE,
and callbacks surviving call invalidation/rejoin. Five initial regressions then passed; review
found the adjacent deferred router mapping continuation. Three additional regressions reproduced
stale ICE sends, route-count repopulation after lock/rejoin and same-port slot replacement.
All eight lifecycle regressions now pass. Grants remain owned by a live call and exact slot,
while sends require the exact peer. Current closed-peer cleanup remains valid. The independent
lock guard is tested before peer-map cleanup. The subsequent native custody correction is below;
the frontend fix alone did not establish native cleanup.

Focused verification: 30 voice/media/identity tests, then 32 lifecycle/routes/call/draw wiring
tests passed; Svelte check reported zero errors/warnings. The final read-only re-review reports
no remaining blocker/high/medium in this frontend correction. These are focused checkpoints;
the branch's complete final suites have not yet been run.

The real TCP native baseline passed with A-C blocked in the actual swarms, fixed six-member
membership and only A/B/C online, then B removed and authorized D reopened from its sealed state.
B and D each received A's exact authenticated announcement. C received none in each one-second
control observation window. Direct submission returned `true` in both phases, confirming that it
does not prove receipt. Helper receipt measured 26 ms / 31 ms in this run; ten control commands
completed in 1080 ms / 1064 ms windows including their intentional 100 ms service intervals.
This is a quiet local baseline, not a latency SLO, physical-network result or successful indirect
voice-room acceptance. No artificial A-C edge, test-only history handoff or room join by B/D was
introduced. New protocol acceptance must add positive endpoint discovery/signalling oracles.

Commands/logs: focused Node commands retain `test-dom.mjs`; `chat-voice-rtc-red.log`,
`chat-voice-router-red.log`, `chat-voice-router-focused.log`, `chat-voice-router-check.log`;
`cargo test --offline --manifest-path apps/desktop/src-tauri/Cargo.toml --lib legacy_voice_signals_have_no_member_bridge_fallback_on_tcp -- --test-threads=1 --nocapture`
passed (`target/chat-voice-bridge-baseline.log`). Compilation used two jobs. Test observations
are bounded to 64 actor deliveries per native client and do not persist or expose user content.

The review-strengthened TCP rerun asserts `submitted == true` before observing non-delivery,
so local refusal cannot make the reproduction pass. It passed through both B and D (27/29 ms
helper receipts, 1096/1050 ms control windows; `chat-voice-bridge-baseline-rerun.log`). Native
diagnostics now say `VOICE.SIGNAL.SUBMITTED` rather than claiming delivery.

## Initial Package 1 native custody checkpoint

Read-only design review confirmed stale native grant installation, port-only cleanup races,
gateway retargeting and lost custody after acknowledged IGD Add. The correction follows
[CALL-MAPPINGS](CALL-MAPPINGS.md): native mount/call/grant identities, prepare/activate CAS,
exact UI/actor/group revalidation and port custody through acquisition/cleanup. Eight prepared
candidates expire after 30 monotonic seconds. Sixty-four process port slots include uncertain
retirement; those charges survive lock, remount and account changes until exit. The legacy PCP
Client's missing drain barrier is explicit, with conservative refusal at exhaustion and continued
STUN/TURN. No new wire, encryption, relay service or dependency is enabled.

Nine initial native coordinator tests and eight injected IGD tests passed. Seven frontend mapping
class tests plus actual-App/route/wiring/fixture neighbors passed (45 tests). Adversarial review
then identified an old call surviving retained-WebView component destruction; the correction runs
the actual leave path and tests stopped microphone/camera tracks, closed peers and inert queued
RTC work (25 focused tests passed). Review also requested a production async acquisition seam;
its dropped-waiter, delayed authorization/cleanup and commit-guard tests are in final focused
verification. The final worker/advice run passed 13 native tests; eight injected IGD tests and
48 frontend lifecycle/route/fixture tests pass. Focused net and native strict Clippy pass. Final
read-only re-reviews have no remaining blocker/high/medium in this initial correction. Displayed
call facts now distinguish mesh advertisements, helper admission offers and router candidates from
media connectivity; a mapping cannot assign blame for failed ICE. Full branch suites remain
outstanding. No physical router,
native audible audio or indirect room success is claimed by these tests.

## Package 2 design checkpoint

[VOICE-SIGNALLING-V2](VOICE-SIGNALLING-V2.md) selects native original-member signatures/delegation
and browser P-256 ECDH/HKDF/AES-GCM as the preferred implementation. It avoids changing the Rust
provider RNG API or introducing an MLS pair-provider/Welcome lifecycle. Native single-use nonce
reservations and exact ciphertext retries, post-decrypt native authorization, process-retained
crypto charges, separate watcher proofs, policy-bound one-helper permits and strict legacy
exclusion are required. Design review corrections pin allocator/renewal/cache lifetimes, offer
identity under glare, exact transport roles, existing active P2P authority, helper replay ledgers
and fair discovery rotation. Both design re-reviews report no remaining blocker/high/medium in
their respective crypto/lifecycle and routing/policy scopes. This is permission to implement the
reviewed design, not runtime/cryptographic acceptance or physical media evidence.

## Package 2 foundation checkpoint

`voice-crypto.ts` implements real WebCrypto P-256/HKDF/AES-GCM with non-extractable endpoint
keys, directional context, frozen single-attempt plaintext/AAD, cached ciphertext, retained-window
queue/byte bounds and native resource-admission callbacks. Nineteen focused Node WebCrypto tests
pass, including altered tuples, an unrelated endpoint key, engine failure, reordered admission,
nonce-window pruning and a fresh module using the retained HMR broker. Review corrected an
incoming token that was not bound to its exact encrypted tuple, mutable owner rebinding and
HMR branding; the standalone foundation re-review has no remaining blocker/high/medium.
The retained broker's version 3 refuses an older owner rather than constructing another pool.
Decrypt keeps its engine/native/byte charge through exact native plaintext commit, withholds the
result until authorization, and clears owned engine/plaintext-copy buffers after commit rejection
or final owner retirement. Real-engine tests and Svelte check pass
(`target/chat-voice-crypto-confirmation-focused-rerun.log`,
`target/chat-voice-confirmation-browser-check-rerun2.log`); cleanup re-review is clear.
These tests do not prove native WebView support or confidentiality from a real MLS-member helper.

`catcoms-sync::voice_v2` defines strict signed Announcement/delegation, WatchChallenge, JoinProof
(including challenge identity and nonce), encrypted headers, renewal-stable pair transcripts and
nonrecursive original-member route permits. `PacketLedger` retains native direction allocators,
single-use reservations, exact ciphertext retries, bounded replay windows and exact post-decrypt
delivery tokens. The native join lease permits only one ledger claim; loss revokes the join,
preventing reconstruction under unchanged keys. Both signing/retry and delivery repeat freshness
after wall-clock changes. Thirty-five focused sync tests plus one product directory test pass
(`target/chat-voice-v2-ledger-focused-rerun.log`). Earlier testing exposed a fixture that had not
opened the channel directory, and a stale root library artifact; neither failed run is acceptance.

Native `voice_authority` separately owns mount/call prepare/activate CAS and active-or-uncertain
crypto charges, independent of optional mappings. Its shared resource pool admits at most eight
crypto operations/process and four/group, sharing 2 MiB/process and 256 KiB/group declared bytes
with retained packet/cache credits. Charges outlive lock/remount/account/call changes until exact
real settlement. The pool additionally bounds its group table to 1024 records. Native authority
tests pass (seven pure plus one real bridge test, including deferred query versus End and actual
lock races; `target/chat-voice-authority-races-focused.log`). No actor snapshot is treated as
signing or delivery authority.

The enforced main-window command policy/capabilities and frontend security ledger now include
the mapping lifecycle and native voice resource commands. Five real Tauri RuntimeAuthority ACL
tests and sixteen frontend command-security/mapping tests pass. Exact revocation/unmap/settlement
remains callable while locked; it grants no fresh work and returns no token-existence bit.
Live forwarding, native Endpoint presence/confirmation/offer integration, actor process-pool attachment,
UI V2 admission/legacy exclusion and mixed-version/live-topology acceptance remain outstanding.
These are foundation results, not Package 2 completion; full branch suites remain outstanding.

## Package 2 fresh-presence foundation checkpoint

Native `Watcher` and `LocalJoin` keep nonparticipant room observation separate from active-call
proof responses. Announcements alone never admit presence. Challenges/proofs bind exact full
identities, watcher incarnation, native challenge id/nonce, room/policy and fixed join/key. Local
presence expires from challenge creation without arrival/duplicate/skew extension. Newer proof
admission retires every older pending challenge, independently of later visible-row expiry.
Actor-owned limits preserve join keys and renewal high-water marks across watcher/helper changes;
active pairs can pin exact bindings beyond delegation expiry without resetting rate windows.

Read-only review found removal/re-add reuse of retained evidence, renewal regression and inability
to refresh a full roster. Corrections synchronously revoke exact evidence and pending proofs,
reject stale new challenges and atomically transfer only uniquely held row credits. Actual clones
remain charged. Wrong-group checks precede irreversible membership decisions. All 57 focused
voice-v2 sync tests and affected sync/app strict Clippy pass (`target/chat-voice-presence-focused-rerun2.log`,
`target/chat-voice-presence-clippy.log`). The initial presence run had one test expecting a removed
challenge to survive re-add; the corrected contract requires a fresh challenge. Re-review is
clear with no remaining blocker/high/medium. No live forwarding, UI room integration or physical
media is claimed.

## Package 2 initial key-confirmation checkpoint

`LocalJoin` now owns one confirmation state per pair transcript. Exact native actions/reservations
gate outbound signing and Finish establishment. Opaque affine committed-flight receipts bind the
original signature/hash/tuple, local and pair leases, freshness and retained bytes. Native state
keeps validated nonces/actions after response-capacity or engine failure; signed duplicates resume
only the current cached action. Three starts share one original 15-second monotonic deadline.
Admitted pair pins remain independent of roster observation replacement, and successful newer-pair
promotion retires old pairs/candidates atomically. No bare-header or old async result establishes a
pair. Review corrected split pair-revocation handles (all paths now share one irreversible lease)
and lost prepare replies (expired unsigned drafts recover without unavailable old ids, while old
results and credits retain their original ownership). All 73 focused voice-v2 tests pass
(`target/chat-voice-confirmation-focused-rerun3.log`),
including deferred engine workers, capacity recovery, nonce echoes, old/new promotion, renewal,
lock/removal, exact cached flights and receipt ownership. Native state tests mock the trusted
browser AEAD return; the separate WebCrypto tests use the real engine. This is not an end-to-end
native/browser/transport test. Affected sync/app strict Clippy passes
(`target/chat-voice-confirmation-clippy-rerun2.log`); actual-code re-review has no remaining
blocker/high/medium. Native/UI
IPC, legacy exclusion, helper transport, offer/ICE ownership and physical media remain outstanding.

## Package 2 process and helper foundation checkpoint

The unused native Runtime now retains group limits/helper caches and one byte/slot pool across
actor/controller/session changes. Captured inactive session tickets, separate actor registration,
exact member generations, policy leases and real-owner drop prevent removed/replaced work from
reviving. An additive transport RequestAuthority performs a synchronous final driver admission
check and race-free cancellation wait without changing lower keepalive custody. Closed native
cancellation publishers fail closed. Three runtime-seam tests and two real libp2p request tests
pass (`target/chat-request-authority-focused-rerun.log`,
`target/chat-request-authority-net-queued.log`, `target/chat-request-authority-net-custody.log`).

The helper core preserves original signed bytes, one target edge or rotating four-member room
batches, cached duplicate outcomes and three attempts per helper/final edge. Targeted work can
wait for a current proven connected route without spending an attempt or dialing. Four forwarding
slots/group and eight/process share declared bytes with browser/cache work. Actual lower/worker/
completion custody holds each affine Flight; only its final drop permits another lower attempt.
Persistent packet/edge/cursor admission commits atomically after all charges succeed. Room
authority remains separate from call participation; production must supply/revoke its exact
directory incarnation. Each helper enforces its own budget; separate machines do not share a
counter. Sender total-route limits are a separate outstanding enforcement point.

Review corrected dropped-Runtime and contradictory-active-view authority, plus wall rollback
before signed expiry. V2 logical wall time now advances with local monotonic elapsed time and
forward local wall observations; remote timestamps cannot move it. Large clock corrections can
make V2 signalling unavailable to other peers until clocks align or a fresh process starts.
Remote initial validity includes both skew windows: up to 25 seconds for packets, 40 for
delegations. Local challenge lifetime remains 15 seconds. All 104 focused V2 tests and affected
rt/net/sync/app strict Clippy pass (`target/chat-voice-helper-focused-rerun3.log`,
`target/chat-voice-helper-clippy-rerun.log`). Tests include three groups sharing eight lower slots
and forced mid-edge/cursor allocation failure. Foundation re-review is clear with no remaining
findings. A subsequent native integration design review identified registration/activation
ordering, mounted-unlock reattachment and directory mutation hooks; those integration corrections
and checks are in progress. No live request kind, helper route, UI V2 or physical media acceptance
is established by this checkpoint. Full branch suites remain outstanding.

## Package 2 desktop actor and directory checkpoint

Desktop voice custody now shares one Runtime/pool with product actors. Found/join/pair prepare
outside their final UI commit; reload uses the ticket captured before authentication. Native
registration CAS runs under the exact row guard immediately before insertion. Typed conflicts
preserve the current restored row; optional preparation expiry/capacity leaves chat usable with
no voice authority. Mounted unlock reattaches fresh owners to retained actors one at a time,
without holding native commit during actor preparation. Duplicate unlock keeps the same owner;
registered inactive restores do not consume the eight uncommitted preparation slots.

Root observes the exact newly authenticated directory change, including compound deletion and
recreation, buffered dependencies and a successfully applied prefix before failure. The product
schema bounds keys/rooms and preserves permanent `general`. A narrow raw-column budget precedes
expanded operation inspection. Old duplicate, forged and local no-op operations never replay
deletion authority. These hooks do not change legacy signed-operation admission or close R6.

Review found stale native install ordering, actor waits under native commit, optional voice
failures blocking unlock, preparation expiry discarding restored chat, and an older unlock
erasing a newer lock flag before revocation hooks. Corrections include exact registration CAS,
affine abandoned-reply grants and a separately committed UI authorization generation. The lock's
first generation increment now closes IPC before any mutex/revocation hook. Static re-review of
the corrected native attachment has no remaining blocker/high/medium finding. Production grants
retain exact room custody and 256 declared bytes until actual last-owner drop. Additional
read-only review of this correction has no remaining findings; its two capacity/lifetime
regressions pass within all 123 Root voice tests (`target/chat-voice-grant-custody-focused.log`).

Focused results: 121 Root voice tests (`target/chat-voice-native-attachment-root-focused-rerun.log`),
17 native voice tests (`target/chat-voice-native-attachment-focused-rerun2.log`), four earlier product
directory tests (`target/chat-voice-directory-product-authenticated-focused.log`) and three bounded
scanner regressions (`target/chat-voice-directory-bounds-focused.log`) pass. Native regressions
include twelve retained actors, idempotent unlock, actual lock versus pending native commit,
healthy plus stopped actors, process-capacity refusal/recovery, expired preparation with continued
chat, exact scope/grant rejection, and retained Owner clones after End. Initial native compiles
needed a missing Arc import and updated fixture constructors; those runs are not acceptance.
Five product/actor tests now pass (`target/chat-voice-actor-commands-focused.log`), including
twelve lost preparation replies without exhausting pending capacity. Affected rt/net/replication/
sync/app strict Clippy passes (`target/chat-voice-native-attachment-root-clippy-rerun.log`);
its initial run requested two borrowed singleton slices in test code, corrected. Native session
checks passed all 47 focused lock/unlock tests (`target/chat-voice-native-session-lock-focused.log`).
The next voice rerun passed its 16 authority/lifecycle tests but its legacy TCP baseline exposed
an unpinned target-descriptor readiness condition. A first normal-discovery readiness correction
still timed out when both actors were driven together. The requester-only readiness correction
passed (`target/chat-voice-descriptor-readiness-diagnostic.log`), preserving the forbidden A-C
edge and all original receipt assertions, but measured 5,491 ms bridge receipt and a 7,147 ms
control window while the actor awaited discovery RPCs. All 17 native voice checks then passed
(`target/chat-voice-native-origin-rerun.log`). Native Clippy completed, but read an intermediate
maintenance module and reported dependency warnings; this is not final clean-lint acceptance.
Those earlier failing runs (`target/chat-voice-native-grant-custody-focused.log`,
`target/chat-voice-native-grant-custody-focused-rerun.log`) are not acceptance.
No live request kind,
Endpoint/helper transport, UI V2, native WebView crypto or physical audio acceptance is claimed.
Full branch suites remain outstanding.

A subsequent read-only transport design review identified additional pre-enablement corrections:
cheap bound-Noise-peer work admission before outer signature verification; exact frame hash/length
and route-generation custody; original-issuer accounting only after replay classification and
persistent capacity reservation; and helper-queued ACKs that cannot discharge a source's final
recipient obligation. The [V2 decision](VOICE-SIGNALLING-V2.md) records these contracts. The current
helper core's origin-charge ordering is now corrected: requester work remains independent;
exact cached IDs and all persistent capacity are classified/reserved before an atomic original
issuer commit. Rejected renewal changes restore the bounded origin/binding buckets, and final
actor/session currency is rechecked under the locks immediately before commit. All 126 focused
Root voice tests and affected strict Clippy pass (`target/chat-voice-origin-admission-focused-rerun2.log`,
`target/chat-voice-origin-admission-clippy.log`). Read-only re-review has no remaining findings.

## Package 1 detached discovery correction in progress

Measured reciprocal actor waits motivated splitting connected PEX, member finalization and
switchboard-offer network suspension from their sole mutable sync owner. Preparation and fresh
completion preserve the released wire/authentication grammars. The networking-only generation
is independent of desktop lock/V2 sessions and revokes on actual MLS transitions, effective
policy changes, descriptor replacement, disconnect and originating owner drop. Finalization
commands use the same scheduler. Two lower flights/server and four/process retain per-target
dedup and declared bounded response capacity through actual transport retirement; a shared
retirement wake lets a queued-only server resume without another minute tick. Injected-clock
deadlines and a sixth fair ready source leave controls and inbound requests eligible.
Review corrections preserve a discovery intent behind explicit finalization, even if its caller
closes, avoid cached success for later explicit callers, retain per-peer occupancy between stages,
rotate around lower-busy peers and limit benign supersession to one fresh retry without peer-health
backoff. Idle actors ignore unrelated global retirements. First authenticated descriptor discovery
pins once instead of spuriously canceling reciprocal initial finalization; later replacement remains
irreversible. Actual empty-offer regressions cover current removal and descriptor/policy/disconnect/
root-superseded replies preserving newer cache entries. Raw signed-record revision now drives the
existing `SnapshotNeeded` worker; exact duplicates and connectivity alone leave it unchanged.

Nine app and nine sync focused checks passed (`target/chat-maintenance-focused-rerun4.log`),
including held reciprocal finalization plus overlapping discovery with continued control service.
This includes actual signed local removal and injected deadline classification. Earlier runs exposed the first-
descriptor case and missing peer-bound `/p2p` routes in the new offer fixture, corrected; they are
not acceptance. The native quiet-legacy PEX regression passed in 0.67 seconds, checking actual sealed
snapshot contents before close and cache reconstruction with an offline transport
(`target/chat-maintenance-native-persistence-focused.log`). The forbidden-edge TCP baseline passed
in 6.90 seconds; B/D receipt now measures 26/21 ms and its nominal one-second control window
1,095/1,046 ms (`target/chat-maintenance-native-tcp-focused.log`). These measurements preserve the
original no-forwarded-legacy-receipt assertion. Affected root and native strict Clippy pass
(`target/chat-maintenance-root-clippy-rerun.log`, `target/chat-maintenance-native-clippy.log`);
the initial lint requested a boxed completion variant and collapsed else-if, corrected. The
final read-only actual-code re-review has no remaining blocker/high/medium findings. One narrow
actor regression rerun passed after the boxed completion change
(`target/chat-maintenance-final-actor-regression.log`); full branch suites remain
outstanding until the whole programme is coherent.
Existing local dial submission waits, mesh repair and the bounded rendezvous drain remain
separate residual scheduling work. This correction alone does not claim a universal control SLO.

## Package 3 stronger existing chat evidence

The six-client native TCP fixture now compares exact canonical original signed-operation unions
as well as stable message IDs, displayed authors and text. Every operation verifies its full device,
signature and channel scope, and expected native sends contribute exactly one new original signed
operation. Partition overlap must remain identical. The parser only observes the real actor's
snapshot; it never supplies state to recovery actors. Secret prefix bytes are borrowed/skipped under
a `Zeroizing` owner. The baseline partition/restart/solo-reopen scenario passed in 61.35 seconds
(`target/chat-exact-signed-union-native-rerun.log`). The hostile reversed-restart/gossip-loss/delayed-
reply/command-load variant passed in 138.29 seconds (`target/chat-exact-signed-union-native-hostile.log`).
Two negative-oracle tests pass (`target/chat-signed-union-oracle-negative.log`): corrupt signatures,
wrong-channel operations, duplicates, and rendered equality with missing/re-authored logs. Review
and re-review have no remaining blocker/high/medium or oracle coverage finding. The first
compile had a wrong DocType import, corrected to its existing `catcoms-wire` owner. No production
chat defect or R6 semantic-binding closure was claimed by that checkpoint. The new-channel and
membership-change matrix below is subsequent, separately recorded evidence.

The new native TCP matrix passes both cases in 107.58 seconds
(`target/chat-new-channel-membership-matrix-final-focused.log`). A channel created while clients
0/1/2 are closed reaches the unselected bridge 3 through catch-up despite gossip loss. After its
creator 4 and other recipient 5 close, bridge 3 reopens from sealed storage and serves the directory
and exact original channel history onward to 0/1/2 without a UI open command. The returning pair
replies bidirectionally; recipients 0/1/2/3 each reopen alone with the final sealed signed union.
Directory discovery is semantic; exact signed-byte equality covers channel history, not its index.

The membership case first failed: quiet connected members missed an owner removal when its control
broadcast was dropped. Discovery now polls the existing kind2 commit exchange before PEX, then
prepares subsequent stages from the recovered epoch. Empty/failed probes still continue ordinary
discovery and cannot assert currency or PEX failure. The native case verifies member/role notices,
original current-epoch history, non-founder control service to bridge 3, then onward recovery of
survivor 4 after the non-founder closes. The founder and removed member remain offline. Surviving
4 can author; 3/4 independently reopen with the exact full roster and signed history.

Fourteen focused maintenance tests pass (`target/chat-quiet-commit-maintenance-final-focused.log`),
including actual equal-count remove/add, exact request binding, old-root/removal/deadline refusal,
oversized answers and signed count-cap rejection before promotion/application. Four existing/new
commit-bundle tests pass (`target/chat-commit-bundle-boundary-focused.log`). Ten actor maintenance
checks pass (`target/chat-quiet-maintenance-actor-focused-rerun.log`), plus the actual equal-count
member/role projection and raw snapshot-dirty regression
(`target/chat-quiet-equal-count-ui-focused.log`). Quiet legacy descriptor persistence still passes
(`target/chat-quiet-commit-legacy-persistence-focused.log`); the legacy TCP voice reproduction passes
in 7 seconds with B/D control windows at 1089/1049 ms
(`target/chat-quiet-commit-legacy-voice-tcp-focused.log`). A compile-time Send-bound correction kept
the projection helper on the mutable sole owner; no failed compile is acceptance evidence.

Commit responses retain the released 16 MiB byte ceiling with a separate 1024-record work cap;
default legacy logs hold 256. Larger configured legacy responses above that count refuse. This
build serves larger retained local logs in consecutive epoch polls. Two/four actual lower permits
bound retained answers at 32/64 MiB plus bounded decoder scratch; signature scratch is released
before per-record decoding. Released kind2 remains unbound member evidence; kind26 separately
proves the endpoint binding. Read-only design/re-review resolved both medium findings (quiet UI
projection and record-count work); no blocker/high/medium remains in this slice. Affected strict
Clippy passes (`target/chat-quiet-control-progress-reserve-clippy-rerun.log`); its first run only
rejected an unnecessary constant assertion in the new test, subsequently removed. These results
do not close removed-author R6 semantics, universal topology recovery or the whole programme.

## Package 2 inert transport and source ownership in progress

Strict borrowed kind28 framing, bound Noise/current MLS identity, pre-signature requester/process
work, exact-frame affine admission, shared actual verification credit and typed role-bound ACKs
are implemented as trusted Rust seams. Normal dispatch remains disabled. Review corrected helper
as endpoint ACK confusion, repeated membership scans before work admission, expired admission at
final helper origin commit, and serialized-buffer undercount. All136 focused voice tests and
affected strict Clippy passed (`target/chat-voice-transport-review-fixes-focused-rerun.log`,
`target/chat-voice-transport-review-fixes-clippy.log`); read-only re-review found no blocker/high/medium
in that inert admission slice.

Detached helper requests now retain actual lower flight custody and capture native Root route,
descriptor/proof/member/policy/room/expiry owners. The first139 focused tests and affected Clippy
passed (`target/chat-voice-forwarding-core-focused-rerun.log`, `target/chat-voice-forwarding-clippy.log`).
Review then found an undrained-disconnect/same-peer-reconnect race. A new opaque driver connection
lease revokes before snapshot/event publication and on shutdown/Drop; a fresh reconnect gets a
different owner. Old events preserve replacement generations. Production maintenance captures
this additive owner too; unsupported legacy seams retain documented event-time fencing.
Three driver tests pass, including actual libp2p Noise reconnect with unread application events
and a stale queued driver command plus fresh positive control
(`target/chat-driver-connection-lease-focused-rerun.log`). Sixteen transport tests passed before
the final local-registration harness correction (`target/chat-voice-forwarding-driver-focused-rerun.log`),
including retained lower responder, stale signed ACK, descriptor/proof replacement, lock, Root
drop and old-event/new-generation ordering. Ten maintenance and eight app checks passed
(`target/chat-maintenance-connection-lease-focused-rerun2.log`,
`target/chat-maintenance-connection-lease-app-focused.log`). A new maintenance fixture initially
hit the existing one-second remote rate limit; injected time advance corrected it.
The subsequent review closed the production race and identified missing local registration
retirement in the Mem harness. Both local and remote dependent connection owners and retired-local
send/snapshot rejection are now implemented. Both rt pair-owner tests and all ten sync maintenance
tests pass (`target/chat-rt-connection-pair-focused-rerun.log`,
`target/chat-maintenance-connection-pair-focused.log`); read-only re-review has no remaining findings.
The native TCP/persistence checks have not yet been repeated after this additive driver change.

The source core now retains original issuer+packet/hash ownership, three total source submissions,
at most two helpers, independent overlapped lower owners and bounded rotating room cursors.
Actual LocalJoin/Watcher accessors issue private incarnation-bound activity handles; a newer live
lease cannot adopt an old first-admitted packet. HelperQueued cannot settle a final endpoint, and
a direct room ACK cannot settle the room. The 500ms hedge is capacity-dependent; maximum-payload
tests retain B and show no D attempt debit until capacity actually returns. Eight focused source
tests passed initially (`target/chat-voice-source-focused.log`). Subsequent tests use the actual
native PacketLedger reservation/cache/pair and per-action retirement, including every real
confirmation supersession. All156 voice checks and strict affected Clippy passed after that
correction (`target/chat-voice-confirmation-source-final-focused.log`,
`target/chat-voice-confirmation-source-final-clippy.log`); source re-review has no remaining
blocker/high/medium finding.

Detached source framing now shares connected-driver/route/Root ownership and the typed ACK verifier
with helper work while retaining distinct source settlement. Two native-root transport regressions
cover helper ACK roles, unchanged originals across new direct routes, three total submissions, End
and unread driver replacement. Typed WatchLease parents cancel actual queued source work without
an actor turn. Call-grant validation recognizes both actual issuer registries, rejecting foreign,
wrong-room, replaced-room and replaced-actor grants. All160 voice tests and strict affected Clippy
passed (`target/chat-voice-source-transport-owner-focused.log`,
`target/chat-voice-source-transport-owner-clippy.log`); both increment reviews have no remaining
blocker/high/medium finding. Signed fixture endpoint assertions test only transport settlement;
they are not native endpoint-delivery evidence. A retained source-responder timeout regression is
added and awaits its next focused run.

Prepared encrypted ingress now reserves bytes before native origin admission and publishes replay
and token only after its final transaction succeeds. Twenty packet tests pass, including rejected
origin, competing preparation, retired pair and End-during-commit regressions
(`target/chat-voice-prepared-ingress-focused.log`); review and live endpoint wiring remain pending.
Production source/helper scheduling, native Endpoint/Watcher/LocalJoin integration, UI V2/legacy
exclusion, actual WebView crypto and physical TURN/audio acceptance remain outstanding.
No whole-programme completion is claimed.

Root endpoint lifecycle now owns bounded Watcher/LocalJoin state with affine observer custody,
recognized call grants, idempotent same-key starts and coalesced nonblocking metadata wakes.
Authenticated member removals invoke synchronous native watcher/pair/proof retirement inside the
existing observed MLS transaction before additions/awaits. A real remove/re-admit regression verifies
retained FreshPeer and PairSetup never revive and cached proofs are retired. The initial lifecycle
run exposed a missing reconciliation in the metadata getter, corrected; all166 voice tests and
strict affected Clippy then passed (`target/chat-voice-endpoint-lifecycle-all-focused.log`,
`target/chat-voice-endpoint-lifecycle-clippy.log`), including the retained source-responder timeout.

Adversarial review found Root's strong Arc could keep native call custody alive after every external
owner disappeared. Root now holds Weak call identity and a revocation handle. All four endpoint
checks pass (`target/chat-voice-endpoint-weak-owner-focused.log`), including loss of the only native
owner with an actual bound-peer queued source flight, no Root turn, and independently retained
lower credit. Short re-review has no remaining blocker/high/medium finding. Native public-key
documentation explicitly limits its claim to structural SEC1/key-reference checks; on-curve import
remains the browser crypto boundary. The prepared-ingress review is clear after a callback-contract
wording correction; actual endpoint origin/queue integration remains pending.

Native controllers now carry revocable parent consent for future explicit watchers. Replacement,
End, lock and coordinator Drop close it, while a call start preserves the controller. Ten focused
native checks passed (`target/chat-voice-controller-parent-native-focused.log`); read-only review
has no blocker/high/medium finding. None of these lifecycle tests enable kind28,
recoverable payload pull, browser V2 mode or a media path. Full branch suites remain outstanding.

Trusted public-control intake now reserves bounded queue/ACK custody before committing original
allowance under the current Group/Policy transaction. It captures exact native watch/call owners;
replacement watchers cannot adopt cached ACKs. A native public-control pump retains exact cached
challenge/proof responses on source capacity refusal. Announcements remain candidates, and only
fresh native challenge responses admit presence. Actual MLS/bound-peer A/B/C tests prove both
native participant joins through an unjoined bridge, but use Mem transport; they do not establish
physical A-C exclusion, WebView encryption or audible media. Queue capacity, immutable retries,
changed signed ID, parent retirement, expiry/rollback and held-receipt regressions pass. Read-only
review has no blocker/high/medium finding; its requested capacity/expiry coverage is now included.
All172 voice checks and affected strict Clippy passed at that checkpoint
(`target/chat-voice-public-controls-all-focused.log`, `target/chat-voice-public-controls-clippy.log`),
followed by both ingress lifecycle checks (`target/chat-voice-public-ingress-lifecycle-focused.log`).

Actual `InboundPacket` payloads now retain their Arc byte credit after native token/receipt/pair
retirement. Private borrowed payload accessors prevent moving the prepaid vector out of its owner;
the compile-fail contract passes (`target/chat-voice-inbound-custody-doc-focused.log`). The prior
receipt test was intentionally strengthened to retain credit while its actual event remains alive.
Twenty-three transport checks passed (`target/chat-voice-public-controls-transport-focused.log`).

Cold cache collection now runs independently of current session permission, through a bounded
process snapshot and sole-actor absolute Clock wake. Origin windows/live pair pins, group member/
epoch/revision evidence and native pair/replay/sequence tables survive collection. Actual lower,
public receipt, decrypt payload and FreshPeer owners retain their credits. Unknown browser engine
operations are never TTL refunded. Confirmation grants capture the original 15-second deadline
even without collection; outgoing send deadlines are fixed at reservation, including exact retries
after a wall correction. All184 focused voice checks pass (`target/chat-voice-collection-focused.log`)
and affected strict Clippy passes (`target/chat-voice-collection-clippy-rerun.log`). Read-only review
has no blocker/high/medium finding. Its requested idle-actor timer regression passes unlocked and
locked without another command/network event (`target/chat-voice-collection-quiet-actor-focused-rerun.log`),
and app strict Clippy passes (`target/chat-voice-collection-quiet-actor-clippy.log`). All12 native
ownership checks pass, including injected-Clock collection, engine uncertainty and shutdown
(`target/chat-voice-collection-native-focused-final.log`). An app forwarding method missing from the first Clippy
run was added. New test imports/Clock Debug bounds needed harness corrections. Startup validation
for the native setup change passes (`target/chat-voice-collection-native-build.log`,
`target/chat-voice-collection-native-startup-unsandboxed.log`). The sandboxed launch timed out;
the repository's same startup script passed outside the sandbox. Follow-up clock-domain coverage
passes with independent server/voice monotonic origins and only the voice clock advanced
(`target/chat-voice-collection-clock-domain-actor-focused-rerun.log`); the actor now pairs each
absolute deadline with its own Clock. No production V2 dispatch/scheduler, encrypted action
queue/pull/negotiation, browser adapter or physical media acceptance is claimed by this checkpoint.

Pending crypto authority now binds actual native pair/action/reservation/token owners. Signing
retires a separate pending encryption lease while exact cached submit/source eligibility survives
the five-second construction cutoff. Token field custody revokes on consume/removal. Native nonce
restart closes old Proof/Finish tokens, including attempts to reissue a grant for the old token.
Hello candidates instead capture one stable per-pair accepted-sequence floor, advanced only by
closed successful native progress and repeated at final token commit. Both completion orders pass:
lower progress leaves a queued newer Hello/waiter eligible; higher progress closes older work
before consuming its token or mutating the attempt. Failed higher input and duplicate completions
do not advance state, and actual engine/payload charges remain held until release. Application
packets after confirmed setup use their own lifetimes. All193 focused voice checks and affected
strict Clippy pass (`target/chat-voice-pending-crypto-all-focused.log`,
`target/chat-voice-pending-crypto-final-clippy.log`). Read-only implementation review has no
blocker/high/medium finding. These factories remain pending-only trusted Rust seams: prepaid
post-transition historical receipts, current OfferId fencing, recoverable encrypted action pull,
browser commit uncertainty handling and production V2 scheduling/dispatch remain outstanding.

Native prepaid progress capacity is now implemented as an inert resource primitive. Reservation
holds one shared-pool charge; ByteBudget loans validate exact pool/group and acquire actual slots
only at work admission. Dropping a chain owner cannot release actual loan custody, including a
zero-byte engine loan. Eleven resource checks pass under full ordinary pressure and concurrent
charge/drop (`target/voice-progress-reserve-complete-focused.log`), with read-only implementation
review clear. All199 focused voice checks pass (`target/voice-progress-reserve-all-focused.log`),
and the affected strict Clippy result above covers this primitive too.
No confirmation/action/engine/source call sites use this budget yet. Raw-inbox
self-starvation remains an open integration finding until a conservative complete confirmation
chain and protected continuation slots pass saturated actual-path tests. Initial integration is
confirmation-only; unknown SDP/ICE buffering remains deferred. Full branch suites are outstanding.

The browser now has an inert confirmation-only action registry on the surviving broker v4.
It freezes exact owner/pair/action/version/native input and shares one 32-record pending/
uncertain/terminal cap plus the ordinary byte/engine budgets. A lost admission or commit reply
retains the operation and retries only the exact native RPC; actual decrypt/digest computation
runs once. Closed accepted/rejected receipts bind identity and input/result tags. Confirmation
plaintext is withheld and wiped, never exported to RTC. Owner/action retirement wakes observers
without refunding actual pending promises. Terminal rows retain their conservative byte charge;
action-only retired identities cannot reconstruct until their owner ends. Temporary queue
pressure is retryable and reports its actual queued state once capacity returns.

All47 focused Node crypto checks pass (target/voice-confirmation-actions-queued-status-focused.log),
and Svelte check reports zero errors/warnings (target/voice-confirmation-actions-typecheck-reviewed.log).
They exercise real P-256/HKDF/AES-GCM but simulate the native action adapter. Tests include lost
success/rejection/admission replies, full terminal and shared-byte capacity, both occupied broker
slots, hanging RPCs, late retirement, exact u64 strings beyond safe integers, malformed receipts,
closed input custody and fresh module/new owner/key/pair creation on the retained broker.
Review corrected two medium findings (fresh-HMR pair marker mismatch and permanent failure on
temporary queue saturation) and two low findings (extra input retention and stale queued status).
Read-only re-review has no remaining actionable findings in this browser slice.

The required native action queue, idempotent admission and permanently serialized settlement
fence are NOT implemented by these Node tests. No native/WebView integration or live V2 acceptance
is claimed. General Pair.open remains unused by calls and must not be the future adapter path;
SDP/ICE require separate charged result/dispatch leases and native current-offer receipts.

The subsequent native increment implements capacity close and typed engine-action custody.
Reservation.close permanently refuses new loans and releases only unused backing; actual closed
loans release their remaining backing and slots on actual drop. Fifteen resource checks pass
(target/voice-progress-reserve-close-focused.log), including full group/process capacity,
zero-byte engine/new-same-group credit and barrier-held concurrent close. Read-only close review
has no blocker/high/medium/low finding. Closing alone never proves a workflow has completed.

LocalJoin now issues opaque actions from actual native proof/pair, confirmation reservation or
inbound token state. Its combined 32 pending/terminal records retain charged immutable inputs,
typed fingerprints and shared irreversible engine fences. Pull rotates at most four without
dequeueing. Native registration matches exact call lease/scope/process pool. Lost admission
reuses one actual permit; settle-before-admit permanently closes delayed work. First actual sign
and closed-confirmation wrappers use that same fence. End/lock/replacement close unstarted rows
and retain admitted engines through actual settlement. Six new native-action path tests plus one
existing matching source test pass (target/voice-native-typed-action-focused.log); four pure
queue/fence checks pass (target/voice-native-action-fence-queue-focused.log). Four native registry
tests pass (target/voice-native-action-registry-focused-rerun.log); the first build lacked a direct
development-only crypto dependency and was corrected. Native cargo check passes with unused
registry warnings while its IPC consumers are still unimplemented. These tests use actual signed
public proof and confirmation state but simulate browser results and the native desktop grant.
They do not prove native WebView, network topology, or full-chain progress.

Native action factories can now use prepaid capacity; a proven-pair focused test exercises that
under full ordinary pressure. Public prerequisites and the whole confirmation chain still do not
use a retained Runtime progress gate/reserve. Exact post-transition success/rejection receipts,
pre-ingress queue/origin atomic admission, coalesced action revisions, browser derive/seal recovery,
native IPC and production V2 scheduling/dispatch remain outstanding. Native action implementation
review has no blocker/high/medium/low finding. The combined 32 identities per join is a hard
current limit; future cleanup cannot recreate expired identities or reset native counters.
All213 voice-focused tests pass (target/voice-native-action-all-focused.log), and affected strict
Clippy passes (target/voice-native-action-fence-clippy-rerun.log). The first lint attempt found
an oversized enum variant, corrected by a prepaid boxed inbound owner. Native typed tagging now
streams borrowed variable fields, avoiding an extra encoding copy. Full branch suites remain
deferred until the programme is coherent.
