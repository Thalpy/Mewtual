# Chat, files and voice implementation evidence

The communication programme remains **open**. File bridging and explicit private Download resume
are implemented and exercised; live V2, production member media relay, combined rollout and
independent physical audio/network acceptance are not complete. The dated sections distinguish
current runtime paths from inactive prerequisites and experiments.

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

Root Rust source is unchanged from the3,033-pass full checkpoint below. The separately reviewed
unpublished token/origin prerequisite is not part of this integration result. Live V2, production
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
