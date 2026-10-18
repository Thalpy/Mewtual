# PR 48 review revision

Status: implementation in progress, not accepted. Baseline: communication branch commit
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
| Legal 32 KiB Offer/Answer cannot fit combined 256 KiB native budget | Canonical ceiling is 8 KiB. The narrow native contract prepays 224 KiB, work lanes, continuation slots and endpoint/helper origin records before admission. Maximum native packets/replies and declared engine/result capacity fit under unrelated pressure. Affine ACK retries preserve custody. Final re-review is clear for this contract; full checks are pending. General SDP action/IPC/browser integration remains disabled. |
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
retained in target logs. No full branch suites have run for this revision yet.

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
outgoing `2C+1340`, source cache `C+2154`, Source lower `4C+8616`, helper retention `C+3068`,
helper lower `3C+7156`, verification `5C+9196`, and incoming packet/event `2C+1340`.
At 8 KiB plaintext, outgoing + source + verification needs 78,354 bytes. But two retained outgoing
and source packets, four held Source lower flights, one verification and incoming packet need
290,020 bytes before engine/result/setup. Reducing the unreleased ceiling alone cannot establish
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
a declared simultaneous peak of 224,294 bytes within the 229,376-byte reserve. Every phase
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
pressured initiator/responder/browser integration. Branch-wide validation is running.

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
retained as `pr48-full-native-test.log`; the final full repeat is pending.

The initial two-worker root full run was deliberately stopped during storage fixtures to improve
test throughput on the 24-thread host. It is an aborted run, not passing evidence. The complete
rerun uses eight test workers and two compiler jobs (`pr48-full-root-test-final.log`); no test
filters or extra ignores were introduced. Native strict all-target/all-feature Clippy also passes
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

Root full verification is still running. Native WebView/forced relay/audio acceptance,
historical result recovery, file durable migration/resume and final combined rollout remain
outstanding.
