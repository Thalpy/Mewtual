# PR 48 review revision

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
