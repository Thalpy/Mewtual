# PR 48 review revision

## 19 October follow-up: programme remains open

The external review of `ac54b2d8` found a hosted Linux onward-directory timeout, a six-client
notification adapter omission, a late ordinary allocation in prepaid SDP routing, and provider
contract drift. Earlier Windows results below do not resolve that hosted failure. Work now
continues above `a41933f3`, including the independently merged Gate7 Studio fixes; they are
preserved. Only one physical desktop is available, so two-device audible/network acceptance
remains unavailable.

The adapter forwards exact cancellable notification authority/custody, with an actual TCP
authenticated indirect-probe receipt test. Root now discharges a restored directory sweep on
the first bound upgrade even when PEX previously proved the same peer unbound. Root-level SDP
tests cover full ordinary-pool pressure, actual authenticated ACKs and two held real lower
owners through cancellation. Route metadata shares prepaid Flight custody. The provider
contract now distinguishes batch size, distinct candidates, concurrency and execution limits.

Production-clock TCP fixtures exposed additional membership/history failures. The old accelerated
clock shortened prepared native authority windows; fixtures now use SystemClock and separately
derive periodic bounds from production cadence/cooldown, while command deadlines remain 30s.
Diagnostics record per-recipient epoch/catalog/queues, current routes, prepared request counts,
actual receiver kinds and transport outcomes. A prepared request count is not wire receipt.

A focused regression reproduced PEX backoff suppressing quiet membership polling; independent
paced bound-member selection now repairs that case. Actual TCP traces also confirmed immediate
kind-2 EOF refusal at the receiver's two-request peer cap during recovery bursts. The revised
capacity-only maintenance lane preserves two ordinary owners and adds one membership owner per
peer, within unchanged aggregate caps. An intermediate one/one split passed the full membership
matrix (188.92s) but reduced released ordinary concurrency; a red regression pins that mistake.
Its eight-case run passed six and failed legacy voice receipt and hostile partition history.
An isolated diagnostic legacy rerun passed (11.29s), which does not establish the earlier failure's
cause. General request fairness and the complete native matrix are still being checked.

Preserving the second ordinary owner passes all **149** network unit tests, including actual TCP
maintenance plus independently released ordinary capacity. The next eight-case native run again
passed six and failed two, this time both partition-history scenarios. Its client-5 observation
had zero bound connected sources, no recovery queue and an outstanding sweep, while client 4
could reach client 6. This is unresolved connectivity evidence, not proof of an adequate timeout.
Failure diagnostics now have independent 500ms per-recipient bounds and report cadence passes,
saved target/listener equality and exact dial-submission outcomes without printing ServerNet keys.

New-format durable Keep work now uses full native scope, a purpose-separated metadata key,
exact sealed successor reconciliation and Resumable/Completed/Deleting ownership. Initial
storage evidence: **22 passed**, plus the four-independent-HKDF-keys regression. Initial actor
integration: **16 passed**, including resumed private prefix verification after cache removal,
shutdown preserving a non-serving reservation, explicit release, actual signed oversized-page
refusal and exact request/debit tail bounds. Subsequent review identified delayed-worker deadline
renewal and post-lock native reply publication; both corrections and focused regressions are
being checked. Automatic resume, durable user pause, bounded whole-variant fallback and indirect
file service are not implemented yet. This slice does not close Package 4.

Subsequent focused storage target: **25 passed**, adding equal-key refusal, combined legacy/V2
quota and failed completed-release visibility. Actor transfer target: **19 passed**, including
original-deadline refusal before first worker polling and held lower admission after expiry or
same-CID replacement. Manual unit workers now use private process pools with the unchanged
production capacity; an intermediate run otherwise hit unrelated parallel-test capacity pressure.
Native retention projection/input/actor-lock checks: **4 passed**. Frontend file/fixture/ACL checks:
**23 passed**, then **11** label/actual-handler lifecycle checks after faulted-copy visibility fixes.
Svelte check passed with zero errors/warnings before the latest small FileInfo action updates.
Two medium integration review findings (deadline renewal and post-lock reply publication) are
corrected and clear on short static re-review. Full suites remain deferred.

The diagnostic native baseline subsequently **failed after 263.77s**, at final six-client reunion;
its earlier bridge phase passed. Several recipients still lacked three original operations despite
live bound connections and empty recovery queues. The saved listener comparisons were correct.
This distinguishes a quiet history-search gap from the earlier disconnected-recipient failures.
Source review confirmed completed searches lack an owned reconciliation timer. An initially
suspected positive-progress task-loss finding was withdrawn: the shared apply path already queues
that work, and the three-source test had stopped before a legitimate source cooldown expired.
Focused timer/union regressions are being added; neither the failure nor the hosted Linux result
is resolved by this observation alone.
An independent deterministic dial-budget regression reproduces a denied pass advancing the saved
route cursor without trying those peers (`pr48-reconnect-denied-cursor-red.log`).

The owned timer target subsequently passes **10 tests**, including an actual timer-created
signed Since request/response with all live gossip discarded. Both focused reconnect tests pass
after denied reservations preserve their cursor turn; four admitted peers/pass retain all default
scheduler limits, with the group-window burst tradeoff documented in RECONNECT.md. Short static
adversarial re-review is clear for these boundaries.

The next eight-case native run passes **7 and fails 1** (421.47s). Onward channel discovery,
missed-membership recovery, baseline partition recovery, signed-history rejection, authenticated
probe receipt and the disclosed legacy voice reproduction pass. The hostile reversed-order,
gossip-loss/command-load scenario still fails at the bridge-4 phase: clients 4 and 6 each lack one
original signed operation, have live bound sources and nonempty queues with a pending response.
Client 5's diagnostic actor/store query exceeds its separate 500ms bound. This is unresolved;
it does not justify a timeout increase or a claim that hosted Linux acceptance is green.

Durable Keep automatic-resume work is in progress. Initial four additional storage regressions
pass (29 total), then explicit V2/V3 byte-stable migration and a separate user-intent CAS were
added after design review. Older manual-only records require explicit Resume before automatic
execution. Native-session retirement, short background admission, integrity-failure suppression,
actor/UI integration and their focused regressions are still being implemented and checked.
No automatic-resume completion claim is made here.

Automatic Keep integration now passes **32 storage** and **23 actor transfer** tests, including
owned actor wake after reopening, original session retirement before queued lower admission,
providerless scans without spend, and stopping a held completed repair without releasing its copy.
An initial auto fixture dropped its diagnostic receiver and panicked in its transport adapter;
keeping that receiver alive corrects the fixture, not production behavior. The eligibility check
uses actual bound current connections rather than stale presence. Frontend checks pass **16**
actual-handler/label tests, including migrated Retry intent, same-value Resume ordering and
exhausted completed repair. Svelte check passed before the latest review corrections.

Read-only review found V2 pending-successor migration and captured pre-Retry intent defects;
regressions and fixes now preserve old V2 pending debit/progress/deletion before V3 upgrade and
resolve the refreshed exact transfer after Retry. Completed checks retain Stop checking and
Retry check. Native compilation/session tests and short re-reviews remain in progress.
A further review confirms the receipt ACK prefix can be cancelled repeatedly before inbound
work is polled; its causal role in the hostile native failure is not established yet.

Native retained-copy projection/input/actor-lock tests pass **4**, and exact native session retirement
tests pass **2**. Fixture/permission-chain tests pass **18**. Both storage and worker/UI read-only
re-reviews are clear after the latest corrections. A new rendered copy-manager flow was added,
but its first harness attempt timed out enabling CDP before any page scenario ran; it establishes
no rendered acceptance. These tests do not establish indirect file service or variant fallback.

The receipt-prefix starvation regression executes red, then passes with retained detached sends.
All **612** sync unit tests pass before the subsequent admission-arbitration correction. Review
identified strict receipt priority exhausting each freed shared slot before repair forwards;
the new actual-owner regression reproduces **8 receipt / 0 repair** admissions before the fix.
All **8** submission tests pass after persistent successful-admission rotation, preserving Busy
intents and actual lower custody. An isolated hostile native rerun is in progress; no causal or
native-success claim is made from unit evidence alone.

The detached-receipt native hostile rerun still **fails** (295.20s): client 4 lacks one
original operation despite two bound sources, an empty queue and a responsive actor. Review
confirmed a newly bound second source cannot reopen searches after the restore's global latch
was consumed. Its new signed-other-document regression executes red, then passes with a
coalesced bounded owner wake, without gossip, discovery or advancing time. All **11** owned-wait
regressions pass. A broader run exposed stale wake intent from proofs before any documents open;
limiting the new obligation to existing documents preserves that boundary. The corrected affected
sync unit target passes **614** tests (127.09s; `pr48-sync-existing-document-scope.log`). Short
read-only review is clear. The native rerun built before that last empty-inventory restriction
still **fails** (266.65s): clients 4 and 5 now have all 15 original operations, but client 6 lacks
one with an empty queue and a reconciliation wake due in 486ms. This is unresolved onward
reconciliation evidence, not complete native acceptance or established causality.

The rendered retained-copy flow passes with exact server/file/transfer/grant and cancellation-lease
oracles (`pr48-kept-rendered-exact-tab.log`). It covers persistent Pause, short Resume admission,
lossless completed Retry and Stop checking without release. A 900x640 screenshot is retained as
`target/pr48-kept-copies.png` in the worktree. Earlier CDP timeouts were browser startup failures:
Edge's GPU subprocess exits with access denied under the execution sandbox; the same focused test
runs with ordinary OS subprocess permission and Edge's own sandbox enabled. The first successful
startup found a test selector that ignored the real tab's icon/count; that selector is corrected.
No native disk/network/audio property is inferred from mocked IPC in this browser flow.

Harness review additionally closed stale-port attachment and early-startup cleanup gaps. Vite
readiness now comes from the exact owned strict-port child. Edge uses a fresh private profile's
exact debugger identity and owned blank page, then navigates the fixture. Cleanup targets only
live owned process trees, and a closed debugger rejects outstanding commands. Three dedicated
helper tests pass, including occupied Vite and stale debugger-port refusal without stranger-page
mutation. The final rendered screenshot exposed replacement characters in newly edited copy
labels; the original middle-dot/ellipsis punctuation has been restored for the next capture.

Reciprocal frontier regressions then execute red for both onward propagation and an older
empty response erasing a later hint (`pr48-reciprocal-frontier-red-executed.log`). The signed Since
serving path now admits bounded reverse recovery when an already-held document lacks a current
connected requester's frontier. Cooldowns bind full member identity rather than transport aliases;
an older pending wait retains dirty completeness through Full fallback. All **18** recovery-wait
checks pass (0.69s; `pr48-reciprocal-frontier-saturation.log`), including known ancestors, invalid
signature/absent document, 512/513-head bounds, expired/full tables, full queues and actual disconnect.
The replication lookup regression passes independently. Read-only review is clear, with saturated
queue admission explicitly best effort. The new eight-case native matrix is building/running;
no green native result is inferred from these unit regressions.

The final punctuation-only screenshot repeat did not reach the visual fixture marker within the
unchanged readiness bound (`pr48-kept-rendered-final.log`); no scenario assertions ran. Its screenshot
is blank. The earlier exact-control scenario remains a passing observation, distinct from this
negative startup result. The harness now reports navigation errors and its last observed URL,
document state and app child count to distinguish failed navigation from fixture initialization.

The inert native CryptoAction seam additionally retains exact accepted and closed-rejected
terminal metadata, hashes bounded actual output with SHA-256, uses its original clock and
post-transition owners, and keeps actual engine credit through settlement. Closed confirmation
preparation precedes token consumption and infallible application. This does not enable V2.

Focused results so far: sync unit target **603 passed**; app maintenance **11 passed**; network
unit target including actual TCP pressure/retirement **148 passed** before preserving the second
ordinary owner. The malformed/tampered maintenance-tag authentication regression passes, with a
genuine signed positive control. Affected-package strict
Clippy passed before the latest ingress-lane change. Several intermediate native membership
runs failed and remain negative evidence; no complete suite or hosted-head success is claimed
for this follow-up. Final checks will follow coherent completion of the remaining work.

## Earlier revision and verification

Status: six review corrections implemented. Final local results are recorded below. The wider
communication programme is not accepted. Baseline: communication branch commit
`2dfc0357ff39ad37edaf44662c04b05e7b5f0f2b`. The review concerns the communication programme,
not accepted Gate 7. The main checkout now contains independent Studio work on Gate7; revision
work uses the isolated `target/communication-review-worktree` checkout of the communication branch.
Build artifacts and verification logs are shared in the original checkout's target directories.

The external review identifies six concrete findings plus known legacy/session and programme
limitations. Its current PR description already states that voice V2, files and media bridging
are unfinished. Do not mark the wider programme complete from these corrections.

| Finding | Current revision status |
| --- | --- |
| Application actor waits for driver while driver waits for event consumption | Real TCP regression reproduced and passes after bounded driver dispatch. Automatic cached/reciprocal/probe/outbox submissions are detached with exact authority and shared custody. Six Root regressions and the real TCP notification-custody check pass; short adversarial re-review is clear. |
| Replacement peer misses a pending router mapping | Shared acquisition promise and exact call/peer/slot/local ICE generation checks pass 24 focused checks; short adversarial re-review is clear. |
| Sole file holder excluded by first four candidates | Four-peer search batches rotate with a per-chunk/round tried set capped at 320 peers. Ten sync and 14 worker tests pass, including every sole-holder position, proof reorder, a still-busy first provider, and exact wrapped-key fallback across network retries. Short adversarial re-review is clear. |
| Legal 32 KiB Offer/Answer cannot fit combined 256 KiB native budget | Canonical ceiling is 8 KiB. The narrow native contract prepays 224 KiB, work lanes, continuation slots and endpoint/helper origin records before admission. Maximum native packets/replies and declared engine/result capacity fit under unrelated pressure. Affine ACK retries preserve custody. Final re-review is clear for this contract; verification is recorded below. General SDP action/IPC/browser integration remains disabled. |
| Announcement reads the issue/expiry clock twice | Reproduced with advancing injected Clock. One sample and checked expiry pass four focused checks; short adversarial re-review is clear. |
| Native warning policy rejects unused action registry | Unfinished native action registry now compiles only for contract tests. Production warning policy is unchanged; native check passes without warnings. |

## Focused evidence so far

The pending-mapping regression fails before the fix with one acquisition and no current-peer ICE
delivery (`pr48-pending-mapping-red.log`). The corrected callroutes/lifecycle run passes 23 tests
(`pr48-pending-mapping-focused.log`). It verifies each waiter's own candidate/username fragment;
media-section and session-level ICE generation checks distinguish restart from unrelated SDP
renegotiation. Svelte check has zero errors and warnings (`pr48-mapping-typecheck.log`).

The two advancing-clock regressions fail before the fix: ordinary one-millisecond advancement
rejects an announcement, and timestamp saturation silently publishes an invalid shortened
interval (`pr48-announcement-red-rerun.log`). All four matching announcement/freshness checks pass
after using one issued sample and checked addition (`pr48-announcement-focused.log`). An initial
test harness compile missed Clock's Debug bound; that was corrected before the red reproduction.

Native cargo check passes without unused registry warnings (`pr48-native-cfg-check.log`). This
compilation boundary means the new desktop action registry is presently a contract-test model,
not production native action IPC enforcement. The sync typed-action foundation remains separate.

Initial read-only review found no blocker/high/medium in these three fixes. Two low findings
were corrected: absent SDP now refuses, and the actual App fixture supplies media SDP plus a
same-peer ICE-restart regression; canonical docs qualify the desktop registry as test-only.
All 24 callroutes/lifecycle checks pass after those corrections
(`pr48-pending-mapping-generation-focused.log`). Short re-review is clear for this slice.

The actual TCP full-256-event-sink regression fails before the driver correction with a local
dial ACK timeout (`pr48-driver-event-sink-red-final.log`) and passes afterward
(`pr48-driver-event-sink-focused-rerun.log`). Independent outbound response and disconnect lease
revocation progress while the sink remains full. Seven focused dispatcher tests pass, including
process-shared bytes/slots, exact Bytes/response/lower custody, same-peer generation order and
fair coalesced recovery after stale gossip (`pr48-driver-dispatch-recovery-focused.log`).
Adversarial review's two medium recovery findings were corrected; short static re-review is clear.

Four focused automatic submission tests pass (`pr48-detached-submissions-focused-rerun.log`):
actual signed removal, descriptor/policy replacement, transferred driver permit after Root drop,
stale outbox head isolation and process-shared actual-last-custody release. Five updated checks
also pass, including same-peer replacement before Root reads a disconnect
(`pr48-detached-submissions-current.log`). All six now pass, including current successful
completion (`pr48-detached-submissions-six.log`). Review identified
unneeded large notification answers; the revised native notification seam retains lower custody
but discards answers at the driver. Its actual TCP empty/maximal-reply test passes without client
event polling (`pr48-owned-notification-tcp-focused.log`); short static re-review is clear.
Opaque budget-window ownership also closes same-instant zero-length-window late refunds.

The six-member search found a third first-four gate in whole/page preparation; that gate now
checks actual known/proven connected eligibility independently of ranking. All ten sync blob
tests pass (`pr48-provider-search-focused-rerun.log`). Review additionally found busy-prefix
starvation and an overly broad ciphertext-CID rejection key; both are corrected. All 14 worker
checks pass (`pr48-provider-worker-fixtures-fixed.log`, two test threads), including invalid-key
local fallback, fetched-invalid reference then valid-reference timeout/retry, and a fifth sole
ciphertext holder while the first actual request stays gated. Four tests originally dropped
their remote transport while expecting a queued request; their named fixture owner now preserves
that tested lifetime. The new Hub fixture also supplies explicit connection edges because Hub
registration alone emits none. Short read-only re-review closes both medium findings.
The original test harness failures and later corrected runs are
retained in target logs. Full branch suites had not run at this checkpoint; final results are below.

The current focused cross-crate pass also confirms the same-instant/foreign-policy late-refund
regression, all ten blob checks, seven dispatcher checks, the real TCP full-sink case, and all
25 runtime tests (`pr48-current-*.log`). Strict affected-crate Clippy passes, including all
targets/features for net, sync and app (`pr48-network-provider-strict-clippy-final.log`). Native
cargo check also passes without warnings (`pr48-native-network-provider-check.log`). These
checks preceded the ongoing SDP budget integration and must be repeated afterward.

## Reviewed network correction contracts

The driver must never await application event delivery inside its swarm selector. Command batches
must be bounded. Keep payload count and actual retained bytes bounded across drivers, including
one legal 16 MiB frame; excess requests must fail honestly rather than receive fabricated success.
Gossip overload requires a coalesced reconciliation wake, which grants no membership authority.
Connection generations require bounded last-announced/latest-actual reconciliation, with old
disconnect before replacement connect. Queue requests under their actual ingress lease; never
rebind old input to a new same-peer connection. Preserve original gossip issuer independently
of the carrier connection. Retained inbound responders also need count/byte bounds and actual
failure/deadline retirement.

Automatic recovery jobs must prepare immutable authority and permits on the sole owner, then
detach transport waits. Cover periodic discovery, maintenance completion, reciprocal repair and
repair outbox paths. Driver admission repeats Root/member/policy/descriptor/connection ownership;
completion alone is too late. Register probe identities before sending. Actual notification and
dial custody must survive waiter cancellation until the lower request/connection retires.

Acceptance needs actual TCP driver saturation, PEX-discovered dial progress, control responsiveness,
independent request/response and disconnect generation progress, shared limits across two groups,
and stale queued-job refusal after membership/policy/descriptor/Root replacement. A command-only
escape from a blocked event send is insufficient because it still freezes swarm responses.

## Reviewed voice resource correction contracts

For ciphertext size C and policy/OfferId-bound targeted helper envelopes, current charges are:
outgoing `2C+1340`, source cache `C+2154`, Source lower `4C+9128`, helper retention `C+3068`,
helper lower `3C+7668`, verification `5C+9196`, and incoming packet/event `2C+1340`.
At 8 KiB plaintext, outgoing + source + verification needs 78,354 bytes. But two retained outgoing
and source packets, four held Source lower flights, one verification and incoming packet need
292,068 bytes before engine/result/setup. Reducing the unreleased ceiling alone cannot establish
unrestricted negotiation concurrency.

A native Runtime group progress owner must reserve measured phase capacity and continuation/
receipt slots before returning a local reservation or acknowledging an incoming Offer. Extra
hedges cannot borrow protected reply headroom. Crossed Offers require a deterministic native
glare decision and capacity-bounded merge, or refusal before ACK. Actual old custody stays charged.
All covered allocation stages must use the same prepaid owner. No limits are widened and no
live replay/retry protection is discarded as part of the proposed correction.

The revised narrow contract admits one local and one remote SDP, one Source record and at most
two actual lower holders, with one fixed OfferId/initiator-or-responder role. Crossed Offers
refuse before positive admission; this does not promise glare recovery. One exclusive Crypto
lane and two exclusive Forward lanes are prepaid from the existing shared slot caps. Packet,
Source, both retained action records, engine, result and 4,096 bytes of workflow metadata have
a declared simultaneous peak of 225,318 bytes within the 229,376-byte reserve, including
512 bytes of route metadata per lower holder. Every phase
credit retains its actual count/lane until last custody drops. Native result capacity and
readiness prerequisites are implemented and tested; general SDP action factories, historical
receipts, browser-copy ownership and RTC delivery remain separate enablement gates.

Both new canonical ceiling tests fail against the former ceiling and pass after the change
(`pr48-sdp-ceiling-{native,browser}-{red,green}.log`). The real native outgoing cache plus
successful Source ACK and exact maximum signed Answer verification/admission test passes both
with and without delayed actual lower retirement (`pr48-maximum-offer-answer-native.log`).
The initial 183 foundation checks pass before budget threading; a later 187-check run passes
after packet/Source/verification threading (`pr48-v2-prepaid-integration-second.log`).

Review found one high stale-preparation defect and two medium lifecycle defects in the new
budget integration. Exact exchange identity plus final admission revocation now fence old
prepared packets; source cancellation observes exchange closure; idle readiness and the fixed
work window have separate deadlines. All six focused SDP checks pass, including replacement
inside the final callback, crossed-Offer refusal, responder identity pinning, delayed readiness
and actual lower-custody cancellation (`pr48-sdp-lifecycle-focused.log`). Short re-review closed
those three findings. Its later medium ACK-retry finding is corrected: occupied prepaid lanes
are tested before requester debits, and Capacity/RateLimited return exact affine completion
custody. The actual helper test retries 160 times, survives genuine rate pressure, then accepts
the same ACK without another send (`pr48-affine-ack-rate-pressure.log`).

The combined native pressure regression retains an actual maximum Offer, Source and two lower
requests, verifies and admits an actual maximum Answer through the final actor/origin transaction,
then holds the declared action records, native result and engine capacity while unrelated work
fills ordinary bytes and work/continuation slots (`pr48-native-sdp-composition-pressure-rerun.log`).
The generic engine/action-capacity owners are real; general SDP AEAD factories/IPC/RTC are absent,
so this is not an integrated WebCrypto or audible-call claim. The 232-test current voice run
passes (`pr48-voice-foundation-combined-first.log`). Ten final SDP checks also pass, including
actual retained three-flight confirmation prerequisites, fixed helpers, current permission on
cached readiness and the single readiness clock sample (`pr48-sdp-readiness-final-focused.log`).
All 47 browser crypto/routes/lifecycle checks pass (`pr48-browser-voice-current.log`).
Final read-only re-review finds no remaining blocker/high/medium in the revision and confirms
the original arithmetic defect is resolved for the declared narrow native contract. Compatible
helper selection between endpoints remains a coordinator enablement gate: the eventual native
coordinator must establish compatible protected routes or refuse readiness/delivery honestly.
This is additional to actual SDP factories, historical receipts, browser-copy/RTC custody and
pressured initiator/responder/browser integration. Branch-wide validation results are below.

The full frontend suite passes all 1,776 tests (`pr48-full-frontend-test.log`).

The accepted Gate7 cleanup commit `149e7321` was subsequently merged without source conflicts.
Read-only merge review found no blocker/high/medium; two stale comments were corrected. The
post-merge frontend suite passes all 1,780 tests, with zero Svelte errors/warnings and a passing
production build. All nine standard browser flows pass with normal process access; the sandboxed
attempt failed at CDP Runtime.enable before any application assertion.

The first complete native unit run found three fixture failures (396 passes, three failures,
zero ignored): inspection fault cases reused an invalidated UI session, and worker/export tests
treated a generation bump or cleared lock request as authenticated unlock. The newer native
authorization floor correctly refused those states. Test-only corrections isolate inspection
cases and explicitly model authenticated fixture unlock under the commit mutex, while proving
old generations still refuse and actual saved content remains readable/exportable. Production
checks were not changed. All three focused reruns pass and read-only re-review is clear
(`pr48-native-{inspection,worker,export}-session-focused.log`). The original failed full log is
retained as `pr48-full-native-test.log`; the passing repeat is recorded below.

The initial two-worker root full run was deliberately stopped during storage fixtures to improve
test throughput on the 24-thread host. It is an aborted run, not passing evidence. The eight-worker
repeat (`pr48-full-root-test-final.log`) later aborts after 880 passes on a default-stack overflow
in the Studio unconfirmed-branch restart fixture. A single-case run reproduces it immediately
(`pr48-root-stack-focused-red.log`; an earlier `--exact` attempt selected zero tests and is not
evidence). Boxing that test's two large fixture/preview child futures preserves every assertion
and restart/drop boundary; the same single case then passes on the default stack
(`pr48-root-stack-focused-boxed.log`). A sixteen-worker full repeat then aborts after 911 passes
in the separate wrong-device/group basis fixture (`pr48-full-root-test-verified.log`). That
single-case failure also reproduces (`pr48-root-stack-scope-red.log`). Boxing the intermediate
proof fixture passes both isolated cases, but a focused Studio exchange run exposes a succession
constructor path that bypasses it after 117 passes (`pr48-studio-exchange-default-stack.log`).

The final correction boxes only the shared test-only `Pair::with_legacy_group` setup body. All
ordinary/legacy setup work remains lazy and in the same order; individual and intermediate
boxing experiments are removed, and every original assertion remains. All 160 Studio exchange
cases complete on the default stack: 158 passed, two existing ignored
(`pr48-studio-exchange-boxed-setup.log`). Read-only re-review is clear. The final unfiltered root
repeat uses sixteen test workers, two compiler jobs and no `RUST_MIN_STACK` override
(`pr48-full-root-test-complete.log`). Its application library passes all 1,143 tests, with 22
existing ignored; the following process-recovery integration fails, so this is not a passing
complete workspace run. No full-suite filters or extra ignores were introduced.
Native strict all-target/all-feature Clippy also passes
after the fixture corrections (`pr48-final-native-clippy.log`).

The complete native repeat passes all 404 tests (399 unit plus five command ACL checks), with
zero ignored (`pr48-full-native-test-final.log`). The normal native build and actual startup
also pass (`pr48-final-native-build.log`, `pr48-final-native-startup.log`).

Remote Gate7 subsequently advanced with Studio fixes `13e338eb` and `22e4ed77`; merge `74641c13`
retains both sets of handover notes. Read-only integration review found a stale cross-server wiki
rename target, close/no-work and final-microtask navigation gaps, and a stale channel-open sidebar
mutation. These are corrected with exact displayed-page identity, original unlocked scope and
mutation-time route/close checks. Six regressions fail before correction with the required Svelte
preload (`pr48-gate7-merge-races-red-corrected-loader.log`); two further continuation checks also
fail before their correction (`pr48-studio-route-continuation-red.log`). The final 55 focused
Studio/confirmation checks pass (`pr48-gate7-merge-races-final-focused.log`); re-review is clear.
The first red attempt omitted the Svelte loader, additionally failing one mounted-editor test;
that harness failure is excluded from the six reproduced defects.

The first full frontend run after that merge passed 1,811 of 1,812 tests. Its confirmation
retirement test incorrectly assumed two event-loop turns proved actual WebCrypto completion
under concurrent Rust test load. The corrected test waits for actual settlement, explicitly
holds the native settlement RPC and proves the original active/byte charge remains until it
settles. No broker invariant was weakened and production crypto code is unchanged. Read-only
review is clear. An intermediate full repeat passes 1,820 tests; final checks after the last
continuation guards are pending.

The final frontend suite passes all 1,822 tests with zero skipped; Svelte check reports zero
errors/warnings and the production build passes (`pr48-full-frontend-verified.log`,
`pr48-frontend-check-verified.log`, `pr48-frontend-build-verified.log`). Its existing bundle-size
warning remains. All nine final browser flows pass (`pr48-browser-flows-verified.log`). A repeat
native rebuild was initially blocked by a reopened application holding the executable. Once the
owner closed it, the final native build and actual startup check both pass
(`pr48-native-build-unlocked.log`, `pr48-native-startup-unlocked.log`).

The process-recovery diagnostic repeat preserves Alice's earlier log alongside Bob's timeout.
It confirms Alice rejected a recovery code before Bob's restart because detached PEX had not
yet learned Bob's signed device-to-transport record (`pr48-process-recovery-diagnostic-red.log`).
The fixture now waits for the full canonical roster identity's exact published transport peer
before releasing Bob. Authentication and the original timeout remain unchanged. The integration
file then passes two tests with one existing child-process ignore
(`pr48-process-recovery-observed-record-rerun.log`). An initial compile attempt incorrectly treated
the canonical parser's Option as Result; it is not passing test evidence.

Subsequent integration checks expose stale presence-fixture command barriers in both memory and
TCP tests (`pr48-remaining-app-integration-tests.log`, `pr48-other-app-integration-tests.log`).
Queries acknowledge queued discovery; they do not prove detached completion. Both fixtures now
wait for exact online projections while preserving actual ConnectivityChanged evidence. The
memory helper keeps twelve ticks and the sixty-second overall bound, lets each tick settle before
advancing its injected clock, and consumes events even when a short read probe times out. A new
regression fills the actual 256-event sink, proves queries block, and requires settling within
the first tick without advancing time. It fails against the old helper
(`pr48-presence-saturated-fixture-red.log`). Review corrected a fill/ACK setup race and bounded
teardown; the first attempt was deliberately terminated after its new fixture hung during cleanup
(`pr48-app-recovery-presence-bridge-final.log`), not counted as a passing run.

The TCP presence repeat passes before exposing a separate default-stack overflow in the retained
temporal-bridge fixture (`pr48-tcp-presence-temporal-final.log`). That test's lazy scenario setup
now returns a boxed future; every original message, sealed-checkpoint, crash and forbidden-edge
assertion remains unchanged. No production allocation or test-thread stack setting changes.
All four combined recovery/presence/temporal integration files pass: 18 tests, zero failures and
one existing child-process ignore (`pr48-app-recovery-presence-bridge-accepted.log`). The first
combined repeat exposed a missing saturation precondition: a full but idle sink still permits
read-only queries. The final regression queues one further real edit before proving its read is
blocked. Read-only re-review finds no blocker/high/medium in these four test-file changes; strict
root Clippy and both formatting checks pass (`pr48-root-clippy-accepted.log`). The subsequent full
workspace repeat is recorded below.

The final remote refresh found newly committed Gate7 review fix `5bc43d2c`. It is integrated
without changing Rust source. Merge resolution retains both handover sections, every regression,
the latest Index-half Save/affected-document behavior and the stronger mutation-time route guards.
One `wikiPaneSerial` now serves both displayed-page identity paths; the rename keeps its original
unlocked scope and checks before and after refresh/open. The first focused merge checks failed
four source audits because the resolution script introduced CRLF; restoring the original LF
format fixes those harness failures without changing the audits. One attempted normalization ran
from the wrong directory and made no edits; its repeated failures are not passing evidence.

Read-only integration review found one medium wiki draft-loss race in the imported Save path:
Save C can complete while another page's read has already stashed newer text N, deleting N before
that read replaces the pane. The new actual Save/open regression reproduces the loss
(`pr48-wiki-save-pending-open-red.log`). A same-pane Save now clears the stash only when its bytes
still match; otherwise it preserves the current text there. The test reopens the original page
and proves N returns dirty and editable. All 90 focused Studio checks pass and re-review is clear
(`pr48-gate7-latest-merge-focused-final.log`). The existing low-priority Index recovery-row grouping
limitation remains documented in HANDOVER; its native Index target must remain null.

The latest merged frontend passes all 1,829 tests, with zero Svelte diagnostics and a successful
production build (`pr48-full-frontend-latest-gate7.log`, `pr48-frontend-check-latest-gate7.log`,
`pr48-frontend-build-latest-gate7.log`). The actual native rebuild/startup also pass
(`pr48-native-build-latest-gate7.log`, `pr48-native-startup-latest-gate7.log`).

Later browser repeats exposed pending Vite entry-module requests, not a captured application
exception. The default dependency scan includes old generated native release HTML as live entries
(`pr48-vite-cold-scan-control.log`). Entry scoping and cache isolation alone did not establish a
passing run; diagnostic controls with short deadlines are not proof that either was the sole cause.
Development now prebundles the declared browser dependencies explicitly, retaining the Svelte
plugin's runtime additions and CommonJS support for qrcode/jsqr. It uses a worktree-local cache
outside junctioned node_modules. Future CommonJS subpath imports require explicit inclusion.
Production Rollup input, fixture restrictions and authorization are unchanged. All nine original
browser flows pass with this configuration and unchanged deadlines on fresh ports
(`pr48-browser-flows-explicit-deps-fresh.log`). A fixed-image real browser QR encode/decode/PNG
smoke check additionally covers those lazy CommonJS dependencies. All ten final browser checks pass
(`pr48-browser-flows-qr-final.log`). The final full frontend suite again passes all 1,829 tests,
with zero Svelte diagnostics and a passing production build
(`pr48-full-frontend-accepted.log`, `pr48-frontend-check-accepted.log`,
`pr48-frontend-build-accepted.log`).
Read-only configuration/harness review is clear. All temporary diagnostic configuration is removed.

The unfiltered root repeat passes the application library (1,143 plus 22 existing ignored) and
process recovery, then exposes a healthy-group eclipse fixture race. It advanced injected time
before detached discovery established the stated healthy precondition. Ordinary discovery now
warms up at the unchanged clock until Alice's exact three-member online set exists; consumed
events still assert no CAUTION, and the original ten-minute plus five-minute offline loops stay
unchanged. All 14 product integration tests pass after this correction
(`pr48-product-healthy-precondition-verified.log`); re-review is clear. A final short-read guard
keeps draining possible if that warm-up sink fills. All 14 final product tests and strict root
Clippy pass (`pr48-product-healthy-precondition-final.log`, `pr48-root-clippy-final.log`).
The full workspace run continued through all remaining targets with no fail-fast. The redundant
partial other-crate check was stopped during its long cost fixture
(`pr48-remaining-root-packages.log`); the unfiltered run completed that fixture.

The completed unfiltered root sweep reports 2,738 passed, two failed and 24 existing ignored
(`pr48-full-root-test-accepted.log`, exit 101). Both failures came from test defects: the eclipse setup
above and the request-budget inventory missing the new disabled voice transport kind. The latter
now declares the existing 512-byte receive ceiling. Each of the three signed ACK statuses encodes
to 186 bytes; verification and detached forwarding independently reject replies above 512 bytes.
No production limit, timeout, ignore or assertion is relaxed. All three focused budget audits pass
(`pr48-response-budget-focused.log`). The complete corrected synchronization rerun passes all 629
tests, including 589 unit tests, 38 integration tests and two doc tests
(`pr48-full-sync-corrected.log`). The complete 14-case product rerun passes. Other workspace
targets, including the 10,000-operation cost fixture and all doc tests, completed successfully in
the unfiltered sweep. The sweep's failed
exit remains recorded; affected-target reruns do not relabel it as a passing unfiltered command.

Native WebView/forced relay/audio acceptance, historical result recovery, file durable
migration/resume and final combined rollout remain outstanding.

## Final local check record

Commands below ran from the isolated communication worktree on Windows. Root and native artifacts
use the original checkout's corresponding target directories; compilation is limited to two jobs.
The final root tests use sixteen workers and the normal test-thread stack. Earlier failures and
aborted attempts above remain distinct from the final evidence.

| Check | Result | Log under original `target/` |
| --- | --- | --- |
| `cargo test --all --all-features --no-fail-fast -j 2` | Completed, exit 101: 2,738 passed, two failed, 24 existing ignored. Both test defects corrected; complete affected-target reruns below | `pr48-full-root-test-accepted.log` |
| `cargo test --all-features -p catcoms-app --test product_e2e -j 2` | PASS: all 14 product cases after the eclipse setup correction | `pr48-product-healthy-precondition-final.log` |
| `cargo test --all-features -p catcoms-sync -j 2` | PASS: 629 passed, zero failed/ignored, after the budget inventory correction | `pr48-full-sync-corrected.log` |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml -j 2` | PASS: 404 passed, zero failed/ignored | `pr48-full-native-test-final.log` |
| `npm --prefix apps/desktop test` | PASS: 1,829 passed, zero failed/skipped | `pr48-full-frontend-accepted.log` |
| `cargo fmt --all -- --check` and `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml -- --check` | PASS | Tool output, no diagnostics |
| `cargo clippy --all-targets --all-features -j 2 -- -D warnings` | PASS after the final budget inventory correction | `pr48-root-clippy-budget-final.log` |
| `cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets --all-features -j 2 -- -D warnings` | PASS | `pr48-final-native-clippy.log` |
| `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml` | PASS, no warnings | `pr48-final-native-check.log` |
| `cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml -j 2` | PASS after application was closed | `pr48-native-build-accepted.log` |
| `npm --prefix apps/desktop run check` | PASS: zero errors/warnings | `pr48-frontend-check-accepted.log` |
| `npm --prefix apps/desktop run build` | PASS, existing bundle-size warning | `pr48-frontend-build-accepted.log` |
| `npm --prefix apps/desktop run test:flows` | PASS: all ten Edge checks, including real QR interop | `pr48-browser-flows-qr-final.log` |
| `npm --prefix apps/desktop run test:startup` | PASS: actual native setup marker | `pr48-native-startup-accepted.log` |
| `bash scripts/check-no-ambient.sh` | PASS | `pr48-final-no-ambient.log` |
| `bash scripts/check-store-raw-fs.sh` | PASS | `pr48-final-store-raw-fs.log` |
| `cargo deny check` | PASS: advisories, bans, licences and sources; existing duplicate warnings | `pr48-final-cargo-deny.log` |
| `git diff --check` | PASS | Tool output, no diagnostics |
| Read-only adversarial re-reviews | No remaining blocker/high/medium in the reviewed corrections | Findings and corrections recorded above |

These checks establish local revision evidence. The programme's native media/forced-relay,
blocked-topology V2, durable resume and combined acceptance gates remain open.


## 19 October: current local recovery and retained-copy evidence

The reciprocal-frontier implementation passes all 18 recovery-wait tests and all 621 sync unit
tests (`pr48-reciprocal-frontier-saturation.log`, `pr48-sync-reciprocal-frontier-unit.log`). The
native eight-case recovery matrix now passes all eight in 277.63 seconds
(`pr48-native-eight-reciprocal-frontier.log`), including the formerly failing onward channel
scenario and hostile partition/restart exact signed-operation union. These tests use the real
TCP adapter and graceful freeze/save close barrier. They establish no abrupt-crash custody,
hosted Linux result, upgraded voice path or audible media result. The legacy voice reproduction
still demonstrates the missing member-bridge fallback.

The fresh-port rendered retained-copy flow passes its exact Pause/Resume, completed Retry,
check/cancel lease and lossless decimal CAS assertions (`pr48-kept-navigation-diagnostic.log`).
The final screenshot has the corrected punctuation (`target/pr48-kept-copies.png` in the
worktree). The earlier readiness failure remains a failed attempt, not application acceptance.
Three browser-process ownership helper tests pass. Read-only harness review has no remaining
findings; execution required running the local browser test outside the tool sandbox because
Edge's GPU subprocess was refused before app assertions. Edge's own sandbox stays enabled.

The absent-file-key regression executed red: an authenticated legacy join/restore can retain
valid file metadata while lacking its transfer key. Normal successful typed joins adopt that
key synchronously; this is not an ordinary asynchronous join race. Root and actor now refuse
Keep before reservation/debit/worker admission, and automatic execution cannot select such work.
The sync regression passes (`pr48-keyless-kept-green.log`). The first actor rebuild stopped at
LLVM output with no free build-volume space; no tests ran in that attempt
(`pr48-kept-key-admission-actor.log`). Only 23 verified regenerable `catcoms_app-*` incremental
cache directories were removed, recovering 14,566,744,064 bytes and preserving the source
worktree. The retry passes all 24 file-worker tests
(`pr48-kept-key-admission-actor-retry.log`). Rejoining is the existing file-key recovery path;
this adds no new transfer-key recovery protocol or old-device retained-scope migration.

V4 whole-variant ownership is being integrated. Its first retained-store run passes all 37
cases (`pr48-kept-v4-storage-first.log`), including five new candidate/selection/quota/fencing
regressions. This does not yet establish the actor's full-file fallback path. Live V2, indirect
file service, media relay and combined acceptance remain open. Only one desktop is available;
physical two-device/network/audio checks remain unperformed.


## 19 October: whole-variant durable Keep integration

New V4 transfers save the complete original two-to-four compatible encrypted candidate set,
per-candidate private progress/bad-reference verdicts, working selection and a separate committed
winner. All candidates retain the same scope, owner, original listing, allowance, Retry grant,
Pause/intent and native cancellation. Switching retires the old run before durable publication
and returns a fresh opaque owner; the actor resets whole-file hashing, chunks, staging, provider
and page state. It never mixes candidate prefixes. Ordinary retained read/has/CID enumeration
and completed inventory expose only the committed winner. Existing V2/V3 and legacy ownership
remain exact-only, with unchanged canonical bytes and 80 KiB record-slot charges. V4 conservatively
reserves all candidate bytes and two400 KiB metadata slots; the maximum encoding fits below360 KiB.

The actual worker regression first executed red: a bad first wrapped key stranded its healthy
saved alternative (`pr48-kept-v4-actor-red.log`). After integration, all26 file-worker cases passed
(`pr48-kept-v4-actor-first.log`). Read-only review then found a completion-only quiet-deadline
branch that bypassed fallback. A controlled-clock regression independently executed red with a
still-valid new KeepWork at the older exact quiet deadline
(`pr48-kept-v4-deadline-red.log`). Completion now rotates without importing that expired reply or
marking its source bad. All28 cases pass after the correction and a test-only missing display-name
argument fix (`pr48-kept-v4-actor-deadline-green-retry.log`).

The final affected worker run passes all29 cases (13.85 s)
(`pr48-kept-v4-actor-signed-fallback.log`), adding real signed network responses: a bad key fails
without cache writes, the healthy reference is fetched separately, and both prepaid request
windows remain spent under the same owner/grant/intent/reservation. Mid-file acceptance now
actually cancels/reopens the Server snapshot and disk after selection/unlisting, verifies the
unchanged saved inventory and completes the whole healthy candidate. Malformed saved equivalence,
digest, bounds and original-set identities refuse before admission or debit.

All40 retained-storage cases pass (14.39 s)
(`pr48-kept-v4-exhaustion-final.log`). New coverage includes current/next selection and winner
promotion recovery at flushed-file/renamed boundaries, all512 leaves reopening and bounded release,
private candidate isolation, whole-local eligibility, failed publication/old-lower fencing, and
V4 exhausted allowance, Pause, bad verdicts and exact Retry receipts surviving restart. The root
missing-key regression covers both new native APIs and passes
(`pr48-kept-v4-key-guards.log`); the canonical full-group namespace factory regression also passes
(`pr48-kept-canonical-namespace.log`). The storage/worker re-review has no remaining blocker/high/
medium or concrete low finding. These are affected-target checks; no new full-suite result is claimed.

Indirect page service is now the next design checkpoint
([FILE-BRIDGE-NEXT](FILE-BRIDGE-NEXT.md)); it is not implemented. External destination resume,
live voice V2, member media relay and combined acceptance remain open.

Affected-target strict Clippy now passes for storage/sync/app with all targets/features
(`pr48-kept-v4-affected-clippy-final.log`). Earlier lint attempts found equivalent boolean forms
and duplicated wake branches; the corrected short-circuit poll preserves delivery-before-voice
ordering and the explicit phase match preserves Pause/Resume eligibility. No lint was suppressed.
The next file-bridge design review requires separately owned bounded full-blob source reads,
full-device replay/miss limits at helper/source, custody for all overlapped lower attempts,
a responder cancellation wait and voice-independent revocation. These remain design prerequisites,
not implemented bridge evidence.


## 19 October: desktop adapter and owned source-read foundation

Further review found an active native integration defect: `ServerStore::blob_store` wraps its
retained store in `ProtectedBlobs`, whose default methods refused V4 creation/fallback and saved
plan reads. Two tests through that actual factory executed red
(`pr48-kept-protected-native-adapter-red.log`). The wrapper now delegates the three V4 operations
and bounded read preparation. All 31 file-worker cases pass (13.72 s), including repeated completed
checks, saved-plan Resume and whole-candidate fallback through the factory
(`pr48-kept-protected-native-adapter-green.log`). Its owned-read delegation/epoch test passes too
(`pr48-owned-read-native-factory.log`). The earlier raw-store results did not cover this adapter.

`BlobReadPlan` now captures a bounded exact-CID read without opening disk files on the actor.
Memory snapshots, compact path capacities, source count, parent fences, sealed input/output and
closed diagnostic metadata have explicit limits. The conservative explicit-buffer bound for an
8 MiB + 44-byte chunk exceeds 32 MiB; the proposed service must reserve 40 MiB, not round down.
Prepared reads and results carry weak store/current-winner fences. Private candidates and staging
are unavailable; Forget, winner replacement, poisoning and scope/store loss suppress stale results.
This remains an inert native foundation: no blocking executor, bridge request or service activation
is implemented by the read primitive itself.

All six owned-read tests pass, including maximum chunks, oversized/sparse input, corruption,
FIFO/delete/drop fences, 4 MiB spare path capacity and 4 MiB arbitrary error strings
(`pr48-owned-blob-read-metadata-green.log`). All 42 retained-store cases pass (17.31 s)
(`pr48-owned-read-kept-current.log`). Strict Clippy for storage/sync/app with all targets/features
passes (`pr48-owned-read-current-clippy.log`). Read-only re-review confirms the missing production
delegations and both captured-metadata bound defects are fixed, with no remaining finding in that
slice. Future blocking-worker custody and actor request/result revalidation remain required.

The transport now exports a reply-specific cancellation authority. Receiver cancellation/drop
revokes it synchronously for actual queued-driver admission, independently of asynchronous wake
propagation. It also follows original ingress revocation, and a deferred owner can await either
condition. Receiving a response ends this authority before network send; the driver's original
ingress authority and actual lower/byte credits stay separate. Notifications are not filtered by
reply lifetime. All 30 runtime tests and nine driver delivery tests pass
(`pr48-deferred-responder-runtime.log`, `pr48-deferred-responder-driver.log`). The concrete queued
in-memory request test abandons its caller before event consumption and verifies both immediate
effect revocation and continued actual accounting custody. Review found one low issue: a closed
receiver still invoked response capacity admission before send failed. It now refuses before that
callback, preserving original-ingress Cancelled precedence. The panic-on-retain regression passes;
all 31 runtime tests and strict rt/net Clippy pass (`pr48-deferred-responder-runtime-final.log`,
`pr48-deferred-responder-clippy.log`). Short read-only re-review has no remaining findings in this
slice. This does not implement the file-service caller.


## 19 October: actual blocking-source custody

The source read now has an affine native task/completion and concrete app `spawn_blocking`
executor. A process-static pool, independent of runtime/executor replacement, admits at most two
40 MiB explicit source charges and one per full group (1 to 256 bytes). Reservation precedes
memory-store preparation. Captured buffers drop before their credit; a completion exposes only
borrowed result/storage fences. Dropping a JoinHandle/Root observer cannot terminate or refund
its queued/running closure. No handler, membership authority, response custody or bridge is
activated by this executor factory.

All four source-custody unit tests pass, including full-group/process isolation, actual error and
absent completions, stale result fences, preparation refusal and runtime replacement
(`pr48-file-source-custody-sync-final.log`). The concrete native executor test passes
(`pr48-file-source-custody-native-final.log`): hold its real blocking closure, abandon observer and
runtime, refuse same-group/third-process work, allow another group only its own released slot,
then verify successful completion remains charged and panic unwind releases actual ownership.
The latch controls readiness, with no fixed sleep/slow-disk assumption. Strict all-target/feature
Clippy for storage/sync/app passes (`pr48-file-source-custody-clippy.log`). Read-only actual-diff
review found no blocker/high/medium; its low stale proposal-status wording is corrected.

Further bridge design review found an exact replay-expiry edge. Freshness is now proposed as
strictly less than 20,000 ms, with 40,000 monotonic ms protective retention and forward-step-aware
effective wall time. No replay ledger is implemented yet. A separate allocation checkpoint traces
the pinned libp2p per-substream codec: protocol-sized expected replies must be rejected before body
allocation, rather than relying on a 72 KiB application promise under a 16 MiB reader. That transport
correction is being tested separately before any new dispatch is enabled.


## 19 October: worker-entry authority and pre-allocation wire limits

Source preparation now requires the exact observing `RequestAuthority`, checked before any
reservation/snapshot and again at actual blocking read admission. A revoked queued task returns
explicit Revoked without entering the private storage-operation seam. A completion retains both
actual result and credit after revocation, and its checked borrow refuses new access. Previously
borrowed bytes are not recalled; actual signing/send and storage-fence checks remain separate.
Observer Arcs must not retain their publisher, Root/store or keys. All seven source-authority
regressions pass (`pr48-file-source-authority-sync.log`), as does the real native worker test
(`pr48-file-source-authority-native.log`), including cancellation before its held worker enters
read. Strict affected storage/sync/app Clippy passes (`pr48-file-source-authority-clippy.log`).
Read-only re-review has no functional finding; both low error/API documentation issues are fixed.

Reserved kinds 29/30 now select a per-substream 2 KiB request/72 KiB complete response limit in
the real codec. Legacy reads remain at 16 MiB; local writes above that already peer-unreadable
ceiling now refuse before emitting a header. Empty/legacy requests reset the per-clone class.
The initial TCP test expected Closed, but the actual driver reports NoResponse for the rejected
reader; its oracle now checks that exact result and an observed malicious header write. With ONLY
the response reader temporarily restored to the old 16 MiB limit, the actual TCP test executes red
and accepts 73,729 bytes (`pr48-file-service-codec-unbounded-control-red.log`). The bounded source
was restored in a finally block. All five final codec tests pass (0.06 s)
(`pr48-file-service-codec-green.log`), including hostile-writer TCP, concurrent legacy/new streams,
exact limits, header-only refusal, reset and malformed/truncated input. Strict rt/net Clippy passes
(`pr48-file-service-codec-clippy.log`). Actual-diff review finds no blocker/high/medium; its low
proposal-status wording is corrected. No service dispatch or forwarding authority is activated.

Closed signed grant/delivery/source/helper codecs are now being tested separately. Replay/rate
admission, Root revocation/service scheduling and actual indirect file acceptance remain open.


Closed signed-wire checks pass all eight cases (`pr48-file-service-wire-first.log`): canonical
maximum complete sizes, original/delivery/two response layers, role/source-list/window bounds,
malformed/truncated/oversized input, domain/signature changes, exact-grant cross-scope rejection,
negative marker isolation and source preservation. The first strict lint pass found five public
native types missing Debug; opaque formatters now avoid dumping groups, identities, file addresses
or ciphertext. Final strict sync all-target/feature Clippy passes
(`pr48-file-service-wire-clippy-final.log`). Read-only actual-diff review has no blocker/high/medium;
its low normative hash documentation item is fixed in the proposal and interfaces. BLAKE3 over the
complete raw signed kind 29 frame is the v1 compatibility contract, not an interchangeable internal
hash. These tests prove codecs/signatures only: no current membership, replay, indirect service or
native audio result follows. Replay/rate admission and Root/helper/source integration are next.


## 19 October: process-stable file-service policy

All eleven admission-policy cases pass (0.23 s, `pr48-file-service-admission-first.log`); strict
sync all-target/feature Clippy passes (`pr48-file-service-admission-clippy.log`). Coverage includes
helper/source nested signatures, exact immutable tickets, full-device C/B transactional quotas,
aggregate counting once, invalid-signature ingress charges, concurrent commit, protective table
refusal, strict freshness/replay boundaries, forward/rollback/overflow and serialized clock samples.
The sole production constructor preserves process rows across Root replacement. Read-only
actual-diff review finds no blocker/high/medium. This remains inert policy: Root/native/MLS/Noise
revalidation, two-second source hop composition, actual capacity acquisition, final response
authority, requester integration and indirect acceptance remain unfinished.


## 19 October: final response admission

An additive optional response effect observer is now checked before budget/queue, on receive and
at the actual native driver copy/send boundary, alongside original ingress. Final scope excludes
reply-wait lifetime, which retires on receive. All 34 runtime cases pass
(`pr48-reply-effect-runtime.log`), including pre-budget refusal, queued receive revocation,
successful lifetime separation and actual MemNetwork behavior. The TCP regression passes
(`pr48-reply-effect-driver-green.log`, 0.06 s): a synthetic scope permits queue and receive, then
refuses at driver handoff; the remote gets NoResponse while held byte/payload owners keep custody.
It also checks healthy scoped and unchanged legacy responses. With ONLY the final driver check
temporarily removed, the regression executes red with actual stale bytes, `Ok(10)`
(`pr48-reply-effect-driver-control-red-final.log`); the guarded source was restored in finally.
An earlier control stopped at an overly strict healthy observation count; the final control
reaches the actual stale-response assertion. Strict rt/net all-target/feature Clippy passes
(`pr48-reply-effect-clippy.log`). Review found no blocker/high/medium; its low test-authority
async-contract item is fixed with a permanent watch latch and exercised completion. Final scope
authorization and file-service caller integration remain open.


## 19 October: actor and Root file lifetimes

Read-only integration review found two additional defects in existing file work. A stopped actor
could acknowledge StopAndWait and then remain alive awaiting a full UI event sink, while its
detached requests still had authority. Queued requests also lacked a Root/member lifetime:
observed MLS removal invalidated eventual completion but not the driver's pending admission.

FileTransfers now owns a unique actor generation and every ReadJob retains it through local
steps, retries and completion. Lower authorities observe the same generation. Stop retires it
before acknowledgement/terminal event waits; freeze retires it before Ready/lease waits and a
cancelled freeze/failed acknowledgement resumes with a NEW generation. Thirty-three focused
worker tests pass (14.11 s, `pr48-file-actor-retirement-second.log`). Removing ONLY the pre-ACK
retirement executes red at `native ACK cannot precede file-effect retirement` with the actual
actor and full actual event sink (`pr48-file-actor-retirement-control-red.log`); source restored.
The first compile attempt needed two fixture updates: an explicit test constructor field and a
shared submission counter. These were compile failures, not passing runtime evidence.

The Root now owns a separate first-field scope publisher. Observed epoch/member/policy changes
retire original observers before optional voice returns; inactive membership refuses fresh
authority. Ordinary unchanged observations preserve healthy jobs. Both ReadJob and the actual
lower gate retain the original observer. The initial 29 file-service tests pass
(`pr48-file-service-root-scope-first.log`), including Root Drop without voice attachment. A real
policy migration/activation case was then added. The first combined actor run passed33 and
failed2 (`pr48-file-actor-root-scope-green.log`, despite that premature log name). One new test
polled an already empty transfer set instead of awaiting its ready result; the other manually
driven requester had not subscribed to membership control. The fixture corrections preserve
actual MLS delivery and the strict no-handoff/no-storage/retained-credit assertions. Rerun is
pending. Actual-diff review finds both production lifetime corrections clear. Exact native entry
ownership, strict bridge route scope and live file-service integration remain separate work.


The corrected actor run passes all35 cases (13.79 s,
`pr48-file-actor-root-scope-second.log`), including genuine subscribed MLS self-removal, old local/
network generation rejection and cancelled-freeze fresh work. All30 file-service cases pass
(0.24 s, `pr48-file-service-root-scope-final.log`), including actual policy migration and activation.
Both real shutdown/save-barrier tests pass (`pr48-file-actor-shutdown-barrier.log`), and strict
sync/app all-target/feature Clippy passes (48.84 s, `pr48-file-root-actor-scope-clippy.log`).
Read-only re-review has no remaining blocker/high/medium or concrete low in either lifetime slice.

A unique native entry publisher now composes the complete account-generation observer with
background Keep attachment. Registration and unlock bind it under the current row guard; reload
retires the old row only after final successful checks and before replacement publication. Exact
removal retires before returning a row that can remain alive across later awaits. Read-only
actual-diff review is clear. Native check passes (56.44 s, `pr48-file-native-entry-check.log`);
its four session and three leave tests are being run. This attachment currently covers background
Keep, not every external/manual download command or future service handler.

Before that native rebuild, 15 exact obsolete `catcoms_app-*` incremental cache directories from
before19 October were checked under the native target's `debug/incremental`, including an entire
no-reparse-point walk. Only those regenerable directories were removed, recovering2,430,390,443
bytes. No source, retained user data, target root or worktree was deleted. Current compiler caches
and the isolated source worktree remain present.


Native exact-entry checks now pass all four session cases and all three leave cases
(`pr48-file-native-entry-session-tests.log`, `pr48-file-native-entry-leaving-tests.log`). The
former required a3m25s native rebuild, then tests completed in0.00s; leave cases in0.03s.
The passing native check and tests describe the revision before the next additive runtime
response-custody seam. No current full-suite or native media acceptance is claimed.


## 19 October: response/source/helper custody before activation

`Responder::try_respond_with_custody` now composes a native service owner with independent driver
response capacity in a fixed two-owner Arc. Original Bytes clones and actual pending lower
serialization retain that SAME composite. Effect authority stays separate. All35 runtime cases
pass (`pr48-response-service-custody-runtime.log`). A real TCP regression holds actual terminal
evidence before pending-table retirement: the remote receives its page and the original Bytes/
waiter disappear, but service capacity remains until that real driver row retires. Discarding
ONLY the explicit service owner executes red (`pr48-response-service-custody-control-red.log`);
current source was restored in finally and the positive rerun passes.

Review found one low test-hook bound issue: refused channels can produce failure IDs without
ever owning a pending row. Holding now requires an actual owned row. The expanded TCP case
holds both real ResponseSent and admitted/no-reply InboundFailure rows, then sends20 actual
refused requests; only the two owned rows remain held. It passes in0.09s
(`pr48-response-service-custody-refusals-green.log`). Re-review is clear. Strict affected
rt/net/sync Clippy passes (`pr48-response-source-current-clippy.log`,23.63s).

`FileReadCompletion::storage_is_current` separately observes a positive result's weak storage
fence, without bytes or work/reply authority. This avoids self-cancelling healthy final sends
after reply receive. Work cancellation still refuses result borrowing; deletion still refuses
the final storage observation. Opaque whole-completion Arc custody remains charged after both
conditions and can transfer into the response keepalive. No current-stored-copy claim follows
from an absent/error completion. The final sender must still compose current native/Root/member/
deadline authority with this fence. Read-only review is clear; no service is activated.

The inert helper pool now admits at most4 chains/full group,8/process and1/original full device/
group, under one process-stable owner. It charges1MiB/chain; the conditional explicit-buffer
derivation is944KiB (four overlapped lowers, two request copies each, twelve complete response
windows and64KiB fixed metadata). The future handler must audit its actual allocation trace.
A non-Clone outer chain mints at most four non-Clone preparation tokens with no reset/refund;
final response custody consumes the outer chain. Its private pending-request constructor still
needs to consume each token once at integration.

Review found a low regression gap: retaining response custody masked broken lower custody.
The inverse-order test now drops outer/response first, then proves capacity remains through the
LAST actual lower owner. Returning an unrelated Arc from ONLY the lower keepalive executes red
(`pr48-helper-lower-custody-control-red.log`); actual custody was restored. All37 file-service
cases pass (0.24s, `pr48-helper-independent-custody-green.log`) and strict sync Clippy passes
(`pr48-helper-independent-custody-clippy.log`). Re-review has no remaining findings in this
capacity slice. Native handler attachment, strict route scopes, live dispatch/requester integration
and indirect Keep acceptance remain unfinished. Live V2/media/combined acceptance remain open.

## 19 October: native source-page dispatch

Source kind30 now dispatches before general request awaits, gated by current native account/
exact-entry attachment, actor generation, Root scope, authenticated active P2P policy, current
C/B MLS keys and a unique bound B descriptor with its actual connection incarnation. It creates
no C standing or dial. The Root retains proof/responder on a fourth fair completion source;
only an immutable bounded plan enters native blocking execution. The same private source credit
now has mandatory task/completion and exported deferred-metadata owners. Native panic cannot
refund surviving Root metadata; Root drop cannot refund an actual worker. The concrete blocking
executor regression passes (`pr48-source-shared-credit-native.log`,1 case,0.00s after2m19s build).
Nine focused read cases also pass (`pr48-source-shared-credit.log`). That first command omitted
`--lib`, so it built filtered integration targets unnecessarily; subsequent iteration uses `--lib`.

The initial handler compile caught a test-only private sibling-module reference; the test now
polls its own future directly. Four initial handler tests pass in1.57s
(`pr48-source-handler-tests-second.log`). Expanded service coverage passes all45 cases in1.63s
(`pr48-source-handler-expanded.log`): actual MemTransport request/run_once dispatch without UI or
voice, cancellation-safe owner polling, current page/signature/window, lock/freeze with fresh
attachment, B descriptor/connection replacement, actual C MLS removal, Root-drop custody,
distinct capacity/timeout Unavailable and past Absent, replay refusal and final storage deletion.

Review found a MEDIUM final-send deadline gap: the two-second fence was on the read worker but
the final Present used only the eight-second parent. Present now retains the original exact
read deadline, excluding reply lifetime. Removing ONLY that final deadline check executes red
(`pr48-source-final-deadline-control-red.log`,assertion that queue/receive cannot extend it);
the original source was restored in finally. Short re-review finds no blocker/high/medium.
A LOW first-poll refusal edge in PresentAuthority::revoked is also fixed: already-deleted
storage returns Ready immediately. Its additional test and strict checks are being rerun.
No asynchronous storage wake is claimed; actual driver admission always rechecks synchronously.

Before this rebuild, obsolete root `target/debug/incremental` directories dated before19 October
were resolved under that exact cache path and fully checked for reparse points, then removed.
Only regenerable compiler caches were deleted; the nested isolated source worktree and user data
were untouched. Available space rose from about1.4GiB to38GB. No F: build target was used.

This is source30, not completed bridge acceptance. Helper29/requester and indirect Keep evidence
remain unfinished. Helper design review found that a16-row shared scope registry could deny a
fourth child after admitting four chains. The next integration will prepay all five route slots
(parent plus four attempts) before nonce/work commit, retaining them through actual lower owners.
Live voice V2, member media relay, combined acceptance and physical two-device checks remain open.

## 19 October: connected helper dispatch and refusal custody

Helper29 now dispatches through the retained fourth completion turn. It prepays one1MiB chain
and all five route slots before nonce/work commit. The per-Root128-slot owner retains complete
plans until last actual scope/plan retirement; admission cannot strand a fourth child behind
the three earlier timed-out drivers. Four source positions and affine attempt tokens are
monotonic. Each lower request is connected-only, with the original C/Root/native authority and
an exact current A descriptor/proof/connection/key. Completion requires the selected A key and
DeviceId, not just any source listed in the grant. Final Found retains parent C and selected A's
original two-second cutoff; reply consumption is excluded. All actual signed Absent observations
can produce helper Absent; skips, unsupported, invalid, timed-out or Unavailable replies otherwise
produce Unavailable unless a later source succeeds. No dial, recursive forwarding, cache write
or file-layer key is added to the worker.

The first helper compile caught a typed SourcePage argument and a missing static transport bound;
the next passed. A test-only match initially mixed u64 and unit; it was corrected before executing
tests. All51 initial cases then passed in2.77s (`pr48-helper-handler-tests-second.log`), followed
by53 expanded cases in2.72s (`pr48-helper-handler-expanded.log`). These include four concurrent
nine-member helper chains reaching a fourth source while twelve earlier actual MemTransport
responders remain held, both actual helper/source owner paths, exact selected-source signatures,
final source/native/descriptor/deadline retirement, wrong named-source refusal, honest negative
grammar, unused-route rotation, reply cancellation and fixed metadata allocation bounds.

Review found a MEDIUM final-refusal custody edge: destructuring a completion could move the
outer flow's last chain credit into a refused response while its original source bytes remained
on the outer stack. Returned lower Bytes now independently own that SAME chain. It also exposed
the equivalent generic response-input edge: Responder now attaches service custody before its
earliest ingress/effect/closed/budget refusal, retaining the later driver composite as before.
No payload copy or authority change is added. The helper refusal/clone regression passes in0.66s
(`pr48-helper-response-refusal-custody.log`); the runtime refusal test passes, including the
review-requested fourth original-ingress branch. Removing only each ownership link executes red
(`pr48-helper-response-custody-control-red.log`, `pr48-response-refusal-custody-control-red.log`),
with original source restored in finally. The first control script stopped before mutation on
CRLF matching; its corrected normalized temporary input executed both red controls.

Short actual-diff re-review has no remaining blocker/high/medium/low. Stale helper/pending-source
documentation is corrected. Strict rt/net/sync/app all-target/all-feature Clippy passes in51.91s
(`pr48-helper-current-clippy.log`). Fresh current focused reruns pass36 runtime cases,
54 file-service cases and18 recovery-owner cases (`pr48-helper-current-runtime.log`,
`pr48-helper-current-service.log`, `pr48-helper-current-recovery.log`). All36 file-worker cases
also pass in13.91s after a123.5s build/run (`pr48-helper-current-workers.log`), including the
incoming/outgoing shared actor-generation regression. No current native rebuild or complete-suite
claim follows from these.

Requester design review requires a12-second C allowance around B's eight-second local scope,
independent bounded source coverage per helper/variant, and source-batch progress after signed
misses rather than only transient retry rounds. Typed bridge candidates remain paged at64KiB
even after direct growth/legacy fallback. This next requester/Keep boundary is not implemented;
native blocked-topology Keep, live V2/media and combined acceptance remain unfinished.

The next requester search primitive is deliberately test-only until the complete native context
and actor integration are ready. Its stable bounded snapshot and independent exact-variant/
helper cursors advance after signed misses, without proof/reconnect/source-order resets. Four
tests pass in0.02s after a2m51s build (`pr48-requester-search-tests.log`): fifth-holder coverage,
crossed helper/source adjacency, same-helper replacement/snapshot stability, and complete limits
of320 source IDs,4 helper devices,4 variants and1280 search batches. This is not a live requester
or Keep path. Static design/implementation review and strict app checks are in progress.


## 19 October: native read origin and whole-job checkpoint

Requester preparation review found two further integration hazards before the C bridge is wired:
queued/native range reads could use no original cancellation, and peer-busy jobs were bounded
only after reaching a network attempt. All native inline downloads, streamed saves and media
requests now capture the UI generation before the first authorization await and retain one exact
account/registry-row observer through every queued chunk, local/network continuation and final
plaintext publication. Range commands carry that original cancellation. Composing a caller scope
retains its complete original authority and the identical lower keepalive. A later unlock or row
replacement cannot reauthorize an earlier invocation. Actual storage admission repeats the check
after AEAD opening; retirement after an already-admitted synchronous write cannot recall it.

Whole read jobs have separate nonblocking custody: four per actor, eight process-wide, admitted
before head/range/staging allocation or starting/debiting a durable Keep run. That owner follows
busy-peer waits, local continuations and network completions until all job buffers retire. Actual
lower-attempt custody remains independent and may outlive the retired job. This is an actor bound,
not a new claim of a stable full-group bound across replaced actor instances.

All40 file-worker tests pass in20.47s (`pr48-read-origin-workers-second.log`), including a real
ReadFileRange queued behind the actor shutdown lease, fresh work after cancelled freeze, whole-job
saturation and actual successful AEAD opening followed by retirement before storage in both local
and network paths. The first new test run exposed a fixture receiver dropped too early; preserving
that observing receiver fixes the fixture without changing transport refusal or tested behavior.
The native check passes in1m07s (`pr48-read-origin-native-check.log`). New native lifecycle tests
and final checks are in progress; no full-suite result is claimed here yet.

Read-only core review has no blocker/high finding. Its MEDIUM post-save inspection/reveal gap and
LOW progress gap are corrected: the original row/account observer follows the saved-file tail,
checks again after inspection and reveal, and suppresses metadata progress after retirement.
Already-published files remain in Downloads if a later stage is refused. Media retains the same
observer through head/cache/ranges/transcode and the actual scheme responder. Controlled native
regressions cover each boundary; current re-review is pending.

The test-only requester Search primitive's strict app all-target/all-feature Clippy passes
(`pr48-requester-search-clippy.log`,40.77s). Its four passing tests do not establish a live requester.
The first complete frontend sweep has1845 passes and four failures, all in the existing Studio
structural audit whose function-boundary parser assumes LF checkout. Normalizing CRLF at its
source-read boundary preserves every guard, expected forced-edge count and negative probe; all20
focused audit tests pass (`pr48-checkpoint-studio-audit-second.log`). The complete sweep is being
rerun. No product navigation behavior or assertion was loosened.

This requested push is a checkpoint. C requester/Keep bridge integration and native indirect-file
acceptance, external destination resume, live voice V2/protocol isolation, member media bridging,
and combined rollout acceptance remain unfinished. Only one desktop is available, so physical
separate-network/audible acceptance remains explicitly open.


The native-origin focused build passes all9 then-current tests in1.53s after a5m14s rebuild
(`pr48-read-origin-native-tests.log`). Subsequent review found one additional MEDIUM before the
future's first poll: the synchronous media callback must capture its generation before spawning.
It now does so, passing that exact value through authorization; an entirely unpolled media future
held across lock/unlock has its own regression. Final actual-diff re-review is clear across all
four severities. The current native full suite will compile/run this last correction.

The complete frontend rerun passes all1849 tests (70.42s), static checks have0 errors/warnings,
and production build passes (58s, existing bundle-size warning). The complete browser flow gate
passes all ten scenarios on owned port5193. Ambient dependency checking passes. `cargo deny check`
passes advisories/bans/licenses/sources after approved access to Cargo's external advisory lock;
the initial sandbox attempt could not acquire that lock. Relevant checkpoint logs use the
`pr48-checkpoint-` prefix under this worktree's ignored `target/`.

The first full root sweep compiled in7m04s but exposed a receipt-timer fixture defect before its
timer step. It supplied a self-signed descriptor and connection, but no authenticated endpoint
exchange required by the hardened automatic-notification path. The actual compiled isolated test
reproduces `Alice did not authenticate both receipts` in10.48s. Its correction uses a real signed
empty-channel catch-up against the running Alice actor for each recipient after MLS convergence;
no production gate, timeout, receipt assertion or timer assertion changes. The corrected focused
case passes in0.59s after46.42s rebuild (`pr48-checkpoint-receipt-fixed.log`). The known-failed
partial sweep was stopped (over556 tests already passed); it is not recorded as a complete or
green run. A new unfiltered no-fail-fast sweep uses sixteen test workers and two compiler jobs.

During that full rebuild, only pre19October `.pdb` compiler debug-symbol files directly under the
verified non-reparse root `target/debug/deps` cache were removed, with every resolved file checked
against that exact directory.222 old symbol files were removed, recovering about35GiB (52.1GiB
available afterward). Current symbols, executables, library artifacts, nested source worktree and
user data were not deletion targets. No recursive target cleanup or F: build target was used.

The first complete current native run finishes with413 passes and one failure in964.38s
(`pr48-checkpoint-native-full.log`). The automatic authenticated indirect-probe test exceeds its
unchanged150-second periodic bound; onward channel discovery, membership/replacement recovery,
the other native restart scenarios and all original-read lifecycle regressions pass. The same
compiled probe test passes alone in7.25s (`pr48-checkpoint-automatic-probe-focused.log`). This does
not establish that the full-run failure is harmless or prove its cause.

Adapter acceptance now genuinely admits only members0,1,2 through the same native admission and
finalization paths. Previously it admitted six and then closed3,4,5: those authenticated offline
descriptors remained eligible recovery targets, while admission-time witnesses could also refer
to helpers other than the specifically asserted helper0. With three actual members,0 is the sole
eligible helper and2 the sole disconnected target for1. The forbidden1--2 edge, real production
cadence, signed-result witness and exact kind16/17 endpoint receipt assertions remain. All other
six-member scenarios still call the unchanged six-member entry point. Timeout diagnostics now
include independent bounded recipient reads and transport receipt/connection counters; the trace
mutex is released before panic. The timeout is not increased, and no proof or manual repair is
injected. Focused and complete native reruns are pending.

Read-only investigation also confirms a separate MEDIUM production fairness defect in
`Root::drive_mesh_repair_inner`: each pass selects the first ranked disconnected target, while
an authenticated negative result removes its pending probe without rotating that selection.
A consistently first-ranked offline device can therefore starve a later reachable target. It
is not established as the sole cause of this native timeout. This remains an explicit follow-up
before programme completion: retain a bounded round/cursor covering all512 peer records (or a
continuing paged scan), preserve one target per pass and do not consume a turn on zero local
admission. Descriptor refresh and negative replies must not rewind it. Required regression:
repeated authenticated negative responses for the first target still allow a later reachable
target's probe and reciprocal follow-up. A four-target burst is not an accepted fix: the helper's
four-request/device/minute allowance is shared with reciprocal forwarding and can be exhausted
before that positive follow-up. No rate limit is raised here.

The corrected three-member adapter check passes in2.75s after1m17s rebuild
(`pr48-checkpoint-automatic-probe-corrected.log`). Its read-only re-review is clear after the
LOW timeout-mutex hygiene correction. The complete current native rerun passes all414 unit tests
in504.36s plus all5 command-policy integration tests (`pr48-checkpoint-native-full-second.log`).
There are no ignored native tests. This includes the entirely unpolled media invocation, original
account/row publication boundaries and actual chat recovery scenarios. Hosted Linux and physical
network/audio results remain unverified.

The complete root command, `cargo test --all --all-features --no-fail-fast -j2`, runs every other
target to completion with2906 passes and24 existing ignored cases. Its only failing target is
`catcoms-sync --test sync`, where27 cases finish and two micelle cases stall in their shared test
helper. The exact verified hung test executable is stopped; the no-fail-fast parent then finishes
the remaining targets and documentation tests with exit101 (`pr48-checkpoint-root-full-second.log`).
This is not a successful unfiltered root command.

The helper joined one explicit requester exchange with exactly one `server.run_once()`. That
serving turn can now consume automatic recovery work or unrelated gossip, leaving the explicit
request unserved while the protocol's ManualClock remains fixed. Adding only a10-second sanctioned
SystemClock watchdog reproduces the original three-micelle failure in11.03s
(`pr48-checkpoint-micelle-control-red.log`). The correction pins one original requester future and
one watchdog, then keeps driving the serving owner. Biased selection checks completed request,
absolute watchdog and serving work in that order, so ready work cannot hide expiry. Every serving
turn still asserts an open transport. All original history, operation-count and frontier oracles
remain, and a new regression queues real signed gossip before the explicit request. No runtime
protocol code changes. The complete affected target passes all30 cases in7.86s before the final
watchdog-order/extra-frontier strengthening; its final rerun and short review are pending.

The stronger receipt setup also passes its final focused run in0.54s after1m08s rebuild
(`pr48-checkpoint-receipt-final.log`), now with a bounded setup and exact device/peer proof assertion.
Whole-root strict all-target/all-feature Clippy passes in26.85s
(`pr48-checkpoint-root-clippy.log`) before the last test-helper-only correction. Formatting checks
pass for both workspaces. Final affected test/lint checks and native build/startup remain pending.

## 19 October checkpoint verification

The final affected integration run uses the original whole-workspace feature graph and the same
`sync-40c7ef85d435e3db` target as the full sweep. All30 cases pass in7.99s after8.55s rebuild
(`pr48-checkpoint-sync-integration-final.log`). Its strict Clippy check passes in10.06s. Final
actual-diff adversarial re-review has no blocker/high/medium/low finding in the helper correction.
The independent production probe-target starvation MEDIUM above remains an explicit follow-up.

| Command/check | Result on this checkpoint |
| --- | --- |
| `cargo test --all --all-features --no-fail-fast -j2` | Exit101: the shared one-turn test helper stalls two cases in the sole failing `sync` integration target; every other target finishes with2906 passes and24 existing ignored tests. The hung executable is stopped, not reported as a successful target. |
| `cargo test --all --all-features --test sync -j2` after the test-only correction | All30 pass, including the two formerly stalled cases and the additional queued-gossip regression. No ignored cases. Together with the completed other targets this covers2936 passing cases and24 existing ignored cases; it is not a fresh successful unfiltered command. |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml -j2` |414 unit tests and5 command-policy tests pass; none ignored. |
| `npm --prefix apps/desktop test` |1849 pass; no failures, skips or cancellations. |
| `npm --prefix apps/desktop run check` |0 errors and0 warnings. |
| `npm --prefix apps/desktop run build` |Pass; existing bundle-size warning remains. |
| `npm --prefix apps/desktop run test:flows` |All ten real-browser fixture scenarios pass on owned port5193; owned child processes are cleaned up. |
| `cargo fmt --all -- --check` and native-workspace formatting check |Pass after the final test changes. |
| `cargo clippy --all-targets --all-features -j2 -- -D warnings` |Pass; the final test-only integration correction also passes its affected-target strict Clippy check. |
| `bash scripts/check-no-ambient.sh` |Pass, repeated after introducing the sanctioned SystemClock fixture watchdog. |
| `cargo deny check` |Pass for advisories, bans, licences and sources. Initial advisory-lock sandbox refusal and the successful approved retry are recorded above. |
| `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml -j2` |Pass in29.53s. |
| `cargo build --manifest-path apps/desktop/src-tauri/Cargo.toml -j2` |Pass in3m07s. |
| `npm --prefix apps/desktop run test:startup` |Pass: rebuilt actual app reaches `STARTUP.SETUP.COMPLETE`; standard script reuses the existing development server on1420 and closes its test app window. |
| `git diff --check` |Pass. |

The final root changes after the unfiltered build are test-only: stronger bounded recipient-proof
assertions and the corrected serving helper. No production behavior changed after that build.
The complete affected target and strengthened receipt test were rebuilt and rerun; the expensive
unfiltered root command was not repeated again. Its exit101 remains visible above. Logs live in
this isolated worktree's ignored `target/` and are not committed. Startup is a boot check, browser
flows use mocked IPC, and graceful native restart uses the production freeze/save barrier; none
establish physical audio, separate-network behaviour or crash custody before a covering write.

This checkpoint is ready to push, not programme completion. Requester/Keep bridge integration,
external destination resume, live V2/legacy isolation, TURN/native media acceptance, member media
bridging, probe-target fairness and combined compatibility/load/rollout evidence remain open.

## After pushed checkpoint b867c622: probe target rotation

Checkpoint b867c622 is committed and the exact SHA is verified at the remote
`chat-update-implementation` branch. Work below follows that push; the complete verification table
above is evidence for that checkpoint, not a full-suite result for subsequent runtime changes.

The starvation regression reproduces the old selection in0.50s: after the helper's authenticated
negative, the same promoted offline target is selected again instead of the later reachable
member (`pr48-probe-rotation-control-red.log`). A fixed optional DeviceId cursor now retains the
last confirmed local target admission. Initial selection honors the old priority; subsequent
passes sort the complete bounded descriptor set in circular full-ID order. No connected-snapshot
truncation, additional fan-out or extra helper allowance is introduced. Local refusal, negative
replies, descriptor refresh and disappearance of the cursor's descriptor do not reset traversal.

Initial review identifies a MEDIUM in the public awaited path: committing the cursor after the
whole helper loop loses an already admitted first turn when a second helper's ACK suspends and
the caller cancels. The actual first-admission/held-second regression reproduces this in0.74s
(`pr48-probe-rotation-cancel-control-red.log`). Cursor commitment now follows EACH successful
helper return immediately, before the next await. The regression authenticates the first helper's
negative, expires the second pending probe under the original injected clock and verifies that
the next admitted work addresses the other target. It does not refund or erase a lower request.

All three focused rotation regressions pass in0.81s (`pr48-probe-rotation-cancel-fixed.log`). They
also cover two negative/positive cycles and the actual reciprocal delivery under unchanged
four-request/device/minute protection, refreshed descriptors, local full-outbox refusal and a
missing cursor record. The first new cycle fixture initially failed to give the helper the
refreshed signed target record, so its correct current-descriptor refusal stalled the next
receive; both endpoints now ingest that same signed record and the fixture asserts the exact
reply queue before proceeding. A fixed ten-second sanctioned SystemClock watchdog bounds failure
without advancing protocol time. A Duration qualification compile error is corrected before
the successful run.

Final neighbours pass nine recovery-submission cases in0.78s, the existing signed-negative case
in0.55s and two reciprocal cases in0.35s; earlier repair/indirect selections also pass. Strict sync
all-target/all-feature Clippy passes in15.56s after simplifying two test-fixture boolean expressions
reported by its first run. The retained receipt fixture uses `pass_first: false`, preserving its
original behavior. Read-only actual-diff re-review has no blocker/high/medium. A greater-than320
selector-only regression remains optional coverage; static inspection confirms the complete
at-most512 descriptor collection is scanned. Current authentication, expiry, replay, membership,
path and rate enforcement remains unchanged. This closes the narrow identified starvation defect,
not the broader multi-group/load acceptance or a wall-time guarantee under continuous churn.

Requester design review separately confirms that320 connected snapshots and512 cached descriptors
are both wrong permanent cutoffs for a full MLS source catalog. MLS membership has no product512
cap. The test-only Search also needs explicit local-requester exclusion before advancing a batch:
a grant containing C is invalid and cannot silently consume other legitimate sources alongside C.
The production requester remains inactive while bounded continuing epoch-scoped catalog pages,
independent helper/variant cursors and original durable execution allowance are integrated.

### Native catalog and unstarted source proposals

The new MLS catalog removes the proposed permanent320/512 source cutoff. Its immutable native
observation retains only count, highest occupied leaf and both32-byte key-shape attestations,
scoped to an opaque group-instance identity and committed epoch. Existing `member_count` performs
and reuses the observation. The cache is not serialized, and missing/stale request views never
rescan. Native load/first observation still traverse/copy the roster and are outside the page
claim. One page examines at most64 raw positions, including holes/exclusions, and returns at most
four sorted distinct public signing keys. Borrowed credentials are checked against4KiB BEFORE
OpenMLS copies the member; both key lengths were attested before that copy too.

The full affected MLS crate passes37 cases (`pr48-member-catalog-crate.log`):15 unit,10 group,
3 policy and9 invite. Catalog regressions include a real514-member MLS roster, sparse removed
leaves, exact same-epoch restore/new epoch rejection, count-cache staging/abort versus commit,
actual restored oversized HPKE/signature keys and exact4096/4097-byte credential boundaries.
Strict MLS all-target/all-feature Clippy passes before the final small public validate wrapper
and removed-receiver test (`pr48-member-catalog-clippy.log`). The added actual removed-receiver
case passes in0.06s (`pr48-member-catalog-removal.log`): old views retire on self-removal; a new
inactive public-tree catalog grants no active-membership authority. Read-only actual-diff review
has no blocker/high/medium; that optional coverage finding is now addressed.

The app's still-test-only Search is changed to use continuing native pages and actual local-C
exclusion. It keeps at most16 helper/variant positions and one proposed nonempty page, committing
only an exact same-search ticket after local admission. Busy does not consume source coverage;
helper replacement/unavailability does not reset it. Empty pages commit one local bounded turn
and Yield, so holes cannot cause a hidden full-roster scan. Unsupported helpers never trigger
legacy kind5/27 forwarding. Initial five-case focus passes in0.11s after the default app graph
rebuild (`pr48-native-catalog-search.log`). Additional same-search identity/field checks and the
replacement-cycle regression follow that build; final execution is pending.

Read-only review finds a MEDIUM proposal-generation defect: changing B's PeerId away and back
could revive the first cloned ticket. The exact peer1/peer2/peer1 regression fails in0.05s with
only that correction removed (`pr48-native-catalog-search-cycle-control-red.log`); a finally
guard restores the corrected source before the command exits. Every observed PeerId change now
mints a checked fresh ticket while preserving the unconsumed source cursor. Unchanged local Busy
retains its current ticket. Same-PeerId connection-incarnation checks remain the Root's separate
mandatory authority. Actual-diff short re-review has no remaining blocker/high/medium. Original
search identity and exact-field comparisons also prevent another job or mutated proposal from
committing its cursor. The complete frontend rerun passes all1849 cases in93.09s, with no skipped
or cancelled cases (`pr48-postpush-frontend-full.log`). Backend full verification is in progress;
the earlier b867c622 table is not evidence for these subsequent runtime changes.

Requester design review has no blocker in this approach, with mandatory integration constraints:
bind helper-key attestations to the original MLS view; do not reintroduce hidden full-roster
scans through descriptor/member helpers; retain the original Root/native/read and durable work
scope; freeze C's twelve-second deadline before handoff; preserve spent allowance across timeout,
cancellation and restart. A finite Keep grant may expire before the complete catalog is searched.
Local preparation refusal, ambiguous handoff, signed negative and global absence are different
outcomes. Actual requester/Keep dispatch, bounded execution work and indirect acceptance remain
unfinished. These tests do not establish any live file bridge, V2 call or native media result.

### Post-push verification in progress

The final seven-case app search focus passes in0.10s after45.61s rebuild
(`pr48-native-catalog-search-final.log`). Root and native formatting and diff whitespace checks
pass. The ambient-dependency gate passes through Git Bash
(`pr48-postpush-no-ambient-gitbash.log`); the default Windows `bash` selected inaccessible WSL
and did not execute the script. Dependency audit passes advisories/bans/licences/sources after
normal advisory-cache-lock access (`pr48-postpush-deny-approved.log`). The first restricted
attempt cannot acquire that outside-workspace lock. No automatic approval review rejects a step.

PR48 is still open and references exact pushed b867c622. At11:22 UTC GitHub reports an empty
status-check rollup for that head; earlier failed workflow runs reference a41933f3. These local
results must not be described as hosted/Linux validation of the pushed checkpoint.

The browser gate first fails before any scenario because restricted Edge GPU subprocesses exit
with access denied (`pr48-postpush-flows-trace.log`). Running normally reveals a genuine transient
`EBUSY` while Edge publishes its private `DevToolsActivePort` file
(`pr48-postpush-flows-approved.log`). The helper now retries only that sharing error and existing
ENOENT under its unchanged absolute15-second startup deadline, on the same fresh owned path.
The strict localhost port/path parser, exact debugger WebSocket identity and EACCES refusal stay
intact. Two new focused regressions execute RED against the old catch (three passes/two failures,
`pr48-flow-profile-lock-control-red.log`); all five pass after correction in2.79s
(`pr48-flow-profile-lock-fixed.log`). Actual-diff read-only review has no blocker/high/medium or
actionable low finding. This changes a test helper, not browser security flags or product UI.

The next normal browser attempt gets through debugger startup but fails its original90-second
fixture readiness bound (`pr48-postpush-flows-fixed-approved.log`), before any scenario assertion.
It observes a complete document with an empty app element while the full root suite is doing
heavy persistence tests. Resource contention is a hypothesis, not an established cause; no
timeout has been increased and this is not a passing browser result. Final full frontend,
backend and browser results remain pending here.

The subsequent normal-permission browser run passes all ten original scenarios after the
frontend unit/static/build work finishes (`pr48-postpush-flows-final-approved.log`). It keeps the
same startup/readiness bounds and private debugger identity checks, then cleans up its own child
processes. This passing rerun does not establish the earlier readiness failure's cause.
The final full frontend suite passes1851 cases in82.34s, including the two new sharing-lock tests;
none are skipped/cancelled (`pr48-postpush-frontend-full-final.log`). Svelte check reports zero
errors/warnings and production build passes in47.40s with the existing bundle-size warning
(`pr48-postpush-frontend-check.log`, `pr48-postpush-frontend-build.log`). Root/native backend
completion and final strict Rust lint remain pending.

The fresh unfiltered root command now exits0: `cargo test --all --all-features --no-fail-fast -j2`
passes2949 cases with24 existing ignored cases across56 targets, including documentation targets
(`pr48-postpush-root-full.log`). No test is removed, newly ignored or loosened to obtain this run;
the24 ignored cases are unchanged. The main app target passes1182 with22 existing ignored in
1095.47s, replication passes403 in148.01s, and the expensive ten-thousand-operation catch-up
regression also completes successfully. The new seven MLS catalog tests, seven app search tests,
target rotation/held-ACK cases and all30 sync integration cases pass on the full workspace graph.
This supersedes the previous checkpoint's exit101 as current root verification; it does not
rewrite that historical failed command. Strict all-target/all-feature lint and native full
verification are running sequentially after this completed command.

Strict all-target/all-feature root Clippy passes in1m12s
(`pr48-postpush-root-clippy.log`). Native full verification is rebuilding next under the same
two-compiler limit; no other Rust suite/build is run concurrently.

The native command finishes with 413 passes and one failure in the legacy voice baseline
(`pr48-postpush-native-full.log`, 483.66s). Source 1 reports local submission, but bridge 0
observes no kind8 request or authenticated call event during the receipt window. The nearby
EOF has no retained notification kind, so it cannot establish that particular packet's failure
cause. An unchanged focused rerun passes both bridge phases in 7.86s
(`pr48-legacy-voice-focus.log`). A separate concurrent six-client diagnostic also passes the
voice and onward-directory tests; that diagnostic is deliberately stopped after the relevant
observations to release its executable for the fault-injection regression. It is not a passing
full or six-client-suite result (`pr48-six-client-debug.log`).

Read-only review identifies a test-contract mismatch: `send_call_signal` promises local queueing,
while the baseline requires delivery of one best-effort notification. The actual App repeats this
heartbeat every five seconds. The regression now deliberately loses exactly the first client-1
legacy heartbeat on each permitted helper edge, then must observe the real authenticated event
after a fresh native send. Its fixed thirty-second total receipt deadline, blocked endpoint's
zero receipts and physical graph assertions remain acceptance requirements. This correction
does not establish reliable legacy offer/ICE delivery or explain the original packet loss.

The original one-shot behavior executes RED under the injected loss in34.91s
(`pr48-legacy-heartbeat-loss-control-red.log`). The retry version passes in17.95s; review's LOW
finding replaces wall-clock cadence and elapsed measurements with `Clock::monotonic_ms`.
The final focused test passes in18.44s after29.61s rebuild
(`pr48-legacy-heartbeat-loss-final.log`): each helper receives the second heartbeat after about
five seconds, with exactly one injected loss per helper and zero blocked-endpoint receipts.
Short actual-diff re-review reports no remaining blocker/high/medium or actionable low.
Root and native formatting, whitespace and the ambient-dependency gate pass again after this
test-only correction. The final full native command runs against that exact monotonic version.

That command finishes with 413 passes and one failure in479.02s
(`pr48-postpush-native-full-final.log`). The first helper's injected-loss phase passes with two
submissions and a5039ms receipt. After closing the founder and reopening helper3, the existing
150-second connectivity prerequisite fails before any replacement-helper voice send. This is a
different failure from one-shot packet delivery. The failed run lacks sealed-route observations;
its specific missing routes therefore remain an inference, not an established log observation.

Static inspection confirms a missing preparation barrier: original admission requests3-to-1/2
connections but waits only for3-to-founder route custody. The voice scenario closes the founder
and needs both other directions. A queued dial is not authenticated, persisted route evidence.
The voice-only preparation now polls the actual sealed native record for `MemberMesh` and both
exact endpoint peer/listener pairs, using the existing90-second capture bound. It neither dials,
calls capture/finalization nor supplies route records. After the actual native close/reopen it
rechecks those same original routes. Unit boundaries reject either missing route, a wrong
listener, duplicate peers and singleton consent. Recovery keeps its original150-second bound
and receipt its original30-second bound. Failure diagnostics independently probe sealed target
indices/current-listener equality, online sets and cadence counts under500ms bounds, without
printing seeds or raw addresses. Actual-diff review finds no blocker/high/medium or actionable
code low; its stale-verification-note low is corrected here.

The initial prepared-route scenario passes in18.26s
(`pr48-legacy-voice-sealed-routes.log`). The final predicate unit and real sealed-close/reopen
scenario both pass in18.67s (`pr48-legacy-voice-sealed-routes-final.log`). Read-only re-review
clears the implemented boundary; the final full native command is running against this version.
The prior failed commands remain above. Formatting and the ambient-dependency gate pass again.

### Final verification of the post-b867c622 checkpoint

The final unfiltered native command exits0: all415 unit cases and all five command-ACL cases
pass, with no ignored or filtered cases (`pr48-postpush-native-full-sealed-routes.log`, 491.23s
for the unit executable). Every six-client scenario passes, including actual notification
receipt, onward directory discovery, membership recovery, both native restart orders and the
lost-heartbeat/replacement-helper baseline. This supersedes the two earlier native failures as
current verification; it does not erase them or establish physical media acceptance.

| Command | Final result |
| --- | --- |
| `cargo test --all --all-features --no-fail-fast -j2` | PASS:2949 cases,24 unchanged existing ignored, across56 targets. |
| `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml -j2` | PASS:415 unit +5 command ACL, no ignored/filtered cases. |
| `cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml -j2` | PASS:50.10s (`pr48-postpush-native-check.log`). |
| `npm --prefix apps/desktop test` | PASS:1851 cases, no skipped/cancelled cases. |
| `npm --prefix apps/desktop run check` | PASS:zero errors/warnings. |
| `npm --prefix apps/desktop run build` | PASS:existing bundle-size warning. |
| `npm --prefix apps/desktop run test:flows` | PASS:all ten real-browser scenarios. |
| `cargo fmt --all -- --check` and native-workspace formatting | PASS. |
| `cargo clippy --all-targets --all-features -j2 -- -D warnings` | PASS. |
| Git Bash `scripts/check-no-ambient.sh` | PASS, including the final test-only corrections. |
| `cargo deny check` | PASS:advisories/bans/licences/sources. |

The fresh root command includes every production change in this checkpoint. The subsequent
corrections affect only native test preparation/observation and documentation; their final full
native command is recorded above. No concurrent Rust suite/build or F-drive target is used.
The native startup gate is not repeated: this follow-up changes no production Tauri setup,
thread/runtime placement or process spawning. The previous b867c622 build/startup evidence is
historical and is not presented as a fresh application launch. Hosted Linux checks remain
unverified; live V2, C-side file requester/Keep bridging, external destination resume, media
relay and combined rollout/physical audio acceptance remain open.
