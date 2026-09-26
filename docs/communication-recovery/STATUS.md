# Communication recovery: scope and gap ledger

Updated 2026-09-26. The supplied [programme plan](IMPLEMENTATION-PLAN.md) is preserved verbatim.
Its proposed work packages and T01-T58 are acceptance requirements, not executed results.
The [bounded patch design](../design-chat-recovery.md) describes the initial foundation.
Subsequent features have their own contracts and evidence: [safe shutdown](SHUTDOWN.md),
[bounded paging](PAGING.md), [queue fairness](QUEUE-FAIRNESS.md), [authenticated policy](POLICY.md),
[durable sends](DURABLE-SEND.md), [quiet UI retry](PENDING-RETRY.md),
and [temporal bridge recovery](TEMPORAL-BRIDGE.md). The final batch and its independent
review branches are recorded in [REVIEW-PUSHES.md](REVIEW-PUSHES.md). Evidence below the feature ledger is historical foundation
evidence, not a claim that later changes passed those same complete suites.

## Comparison with the supplied plan

| Package | Current patch | Gap before package acceptance |
| --- | --- | --- |
| CR00 | Pinned source baseline, design review, signed protocol regression fixtures | No real two-device reproduction; complete limits inventory and authority ADRs remain |
| CR01 | Owner-authenticated immutable P2P policy, signed invite and join binding, sealed pin, explicit legacy state and migration primitives | Dedicated service unsupported; settings and invite preview implemented; vacant-leaf owner succession requires an authenticated transition proof |
| CR02 | New chat sends save operation/MLS state/token/publication obligation before exposure; sealed UI retries; failed-write, abrupt-reopen and context-change coverage; truthful background persistence | Other authoring paths, remote durable receipts and coordinated multi-file admission acceptance remain; bounded pending capacity is explicit |
| CR03 | Coalesced warm-unlock wake, periodic neighbor sweep, quiet retry deadline and collision regression | General durable obligations, typed recovery status, detached outbound waits, account teardown/sleep matrix and load fairness remain |
| CR04 | Authenticated reciprocal member finalization, actual outbound route retention, sealed MemberMesh permission, failed identity-write retry and close barrier | Forced-direction real TCP restart regression passes in both orders; abrupt crash before first successful admission write and physical NAT acceptance remain |
| CR05 | Existing discovery, endpoint budgets and temporary capability boundaries retained | No new member-assistance protocol or relay-renewal acceptance |
| CR06 | Revisit unchanged neighbors, bounded cursor page scans, advancing-empty-page grace with abuse cap, fair full-queue rotation and signed bidirectional fixture | Durable target/coverage summaries, bounded legacy no-cursor compatibility path, alternate source generations and full inventory acceptance remain |
| CR07 | Signed history crosses a forced chain, including independently sealed bridge restart and a previously unknown channel | Opaque forwarding, alternate bridges, attachment custody and physical-device acceptance remain |
| CR08 | Existing epoch/lifecycle authority unchanged | Long-absence control adapter, historical-author admission audit and rejoin integration remain |
| CR09 | No dedicated guarantees exposed | Dedicated admission, persistent sustained-abuse controls and service failover remain |
| CR10 | Sealed pending-message UI with original retry identity, coalesced quiet retries, explicit changed-context draft recovery, storage failures and late-completion fencing | Full recovery-state UI, remote custody/delivery distinctions and diagnostics matrix remain |
| CR11 | Independent design and implementation reviews plus local checks | UI send-barrier mutation evidence recorded; pinned integrated checks, real devices, supported platform CI and release acceptance remain |

No complete CR package or T01-T58 integration case is marked passed by these narrower tests.
The patch has focused variants of T08, T13, T17-T18 and portions of T21, T28, T33 and T36.
The temporal bridge fixture now exercises production actors and independent sealed stores.
Its successful run does not replace physical-network, OS process-kill or full T37 acceptance.
The authenticated TCP reply/restart fixture passes in both orders in the 285-test native run.
Earlier helper-only and chain-only evidence below remains historical.

The durable-send feature supersedes the initial foundation's send ordering: the desktop chat
command now uses a reviewed actor/store barrier and a persisted caller retry identity. Existing
low-level and non-chat authoring paths are not converted by that feature.

Received state now independently emits `SnapshotNeeded` for raw open-document identity/count or
MLS-epoch movement. One native worker per installed server requests an initial save, then batches
wakes in fixed 250-ms windows; a locked UI does not disable the mounted store's writes. The event
has no renderer payload and is not a saved/custody acknowledgement. The tracker compares O(current
open documents) counts per owner turn and replaces its map rather than retaining removed ids.
Pending tickets remain in memory, so crashes before a covering write can still lose accepted work.
Existing Studio persistence barriers and fields outside the raw tracker retain their own triggers.

Likewise, broad in-group forwarding permissions require CR01. The periodic sweep added here
only reschedules already authorized catch-up through the existing verifier and resource limits;
it adds no wire message, disclosure class, endpoint authority, relay role, or group-mode default.

The initial foundation agents were started in one shared checkout before the later programme plan arrived.
Edits to overlapping native sections were serialized after detecting an overwritten extraction;
the final diff is checked for all intended boundaries. No reset, stash, commit, push, or mutation
harness was run in another agent's checkout. Later feature lanes used isolated worktrees, with central dispatch and evidence owned by integration.

## Independent review

The admission implementation agent reviewed the mesh scheduler and warm-unlock changes; the
mesh implementation agent reviewed persistence and the frontend; the persistence agent reviewed
admission and warm unlock. Each inspected the current diff and adjacent enforcement paths.

| Finding | Correction / disposition |
| --- | --- |
| HIGH: simultaneous sole-owner catch-up can retry in lockstep | Keep the 30-second minimum and add a 1-second offset in one peer-order direction; concurrent real `run_once` regression passes |
| Stale connected-source observation can shadow a live retry | Intersect the preferred live tier with one current transport snapshot; regression fetches history from the live alternative |
| Early ordinary PEX could redial a disconnected temporary contact | Dedicated connected-only call shares unchanged response verification; negative test makes ordinary request viable and asserts it is not used |
| Accepted send plus failed refresh could restore a duplicate composer draft | Return acceptance and persistence separately, isolate refresh failure, test actual composer function |
| Pre-submission UI yield could drop unsubmitted text on lock | Start IPC before the first await; fence completion against session/operation replacement |
| HIGH baseline: receive-only chat never requests a snapshot | Actor raw-state invalidation and an incarnation-fenced coalesced native snapshot worker; receive-only/locked write followed by fresh-vault restore passes |
| HIGH: rendered events can miss accepted invisible history or same-count membership changes | Persistence invalidation observes document versions and MLS epoch independently of UI projections |
| LOW: an actor can close its event stream without emitting Closed | Event-forwarder exit removes only that incarnation's persistence signal; unsaved tickets remain dirty |
| MEDIUM: dispatch can still race native lock before acceptance | Existing ambiguous pending-send custody remains CR02/CR10 work; no lossless send-versus-lock guarantee |
| MEDIUM: all queue slots can remain occupied by unresolved tasks | Preserve obligations; rotation is conditional on capacity becoming available, not unconditional all-document fairness |
| MEDIUM: chain test is not process-restart or physical transport proof | T19/T20/T37 and real-device acceptance remain unrun |

Static re-review found no remaining blocker/high in the changed boundaries. The reciprocal
retry and actor raw-state regressions pass; the native receive-worker tests are recorded below
separately. No external review or full programme acceptance is claimed.

## Verification

Completed on Windows in this worktree:

| Command | Result |
| --- | --- |
| `npm.cmd test` in `apps/desktop` | 1,236 passed, 0 failed |
| `npm.cmd run check` | 0 errors, 0 warnings |
| `npm.cmd run build` | Passed; existing large-bundle size warning remains |
| `npm.cmd run test:flows` | All 8 headless Edge flow checks passed, including real frontend send |
| `cargo test --locked -p catcoms-sync --lib --offline` | 275 passed, 0 failed, including quiet retries, reciprocal timeout recovery, forced-chain reconciliation and connected-only PEX |
| `cargo test --locked -p catcoms-app --lib actor::tests --offline -- --test-threads=2` | 27 passed, 0 failed, 2 existing probes ignored; includes both raw-state persistence invalidation regressions |
| `cargo test --locked -p catcoms-app --lib studio_actor_owner_return_installs_both_classes --offline -- --test-threads=1 --nocapture` | 3 passed in isolation; all three had failed under the broad concurrent suite's load |
| Same Studio command in an isolated worktree at `48f906962b37ed71a967c04ef4978d019ff43586` | 3 passed with the same observed recovery sequence; this comparison does not reproduce or close the high-concurrency failure |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --lib --offline -- --test-threads=2` | 267 passed, 0 failed after correcting the receive fixture; existing `PendingApproval` unused-field warning remains |
| Same native command with test filters `received_chat_is_saved snapshot_worker_keeps persistence_event_wakes` | 3 passed after routing both test polling helpers through `SystemClock`; no production change |
| `cargo clippy --locked -p catcoms-sync -p catcoms-app --lib --tests --offline -- -D warnings` | Passed on the final source |

Raw outputs: `logs/chat-recovery-frontend-tests.log`, `logs/chat-recovery-frontend-check.log`,
`logs/chat-recovery-frontend-build.log`, and `logs/chat-recovery-flows.log`.
The flow harness mocks native command responses; it does not establish backend delivery.
Rust outputs: `logs/chat-recovery-sync-tests.log`, `logs/chat-recovery-actor-tests.log`,
and `logs/chat-recovery-scheduling-current.log`.
The baseline comparison is in `logs/chat-recovery-scheduling-baseline.log`.
The complete native result is in `logs/chat-recovery-native-tests.log`.
The final worker rerun is in `logs/chat-recovery-native-worker-final.log`.
Strict core lint output is in `logs/chat-recovery-core-clippy.log`.

The broad core run (`cargo test --locked -p catcoms-sync -p catcoms-app --lib --offline`)
was stopped with one long-running registry-storage test unfinished. Before stopping, its app run
recorded 711 passes, 3 Studio scheduling failures and 12 existing ignored tests. It compiled
before the later receive-invalidation changes. This is not a passing full-suite result. The
three scheduling tests pass in a serial focused run on the final source; the heavy-load failure
is retained as an unresolved suite-level discrepancy. Raw output: `logs/chat-recovery-core-tests.log`.

The initial native run passed 266 tests and timed out in the new receive-worker fixture, which
waited for a UI update on an unlisted test channel. The corrected fixture uses the actual shared
channel and observes the encrypted stored snapshot directly, then stops both actors and reopens
the vault. It does not extend the timeout or manually save the receiving actor. Independent
re-review found no blocker/high. Initial output: `logs/chat-recovery-native-initial.log`.

Both workspace formatting checks and `git diff --check` pass. `scripts/check-no-ambient.sh`
fails on unchanged inventory comments, media decode timing and Studio test timing calls already
present at the base commit. Those unrelated files were not modified or exempted by this patch.
The final gate output is in `logs/chat-recovery-ambient-check.log`; new native test polling uses
the repository's clock interface and adds no gate violation.

The original debug build and first reduced-debug attempt failed from disk exhaustion, not test
assertions. Only generated Rust build artifacts under this repository were removed. Subsequent
Rust commands use `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_PROFILE_TEST_DEBUG=0`,
`CARGO_INCREMENTAL=0`, and `CARGO_BUILD_JOBS=1`. Native verification shares the root target
directory to conserve space. Source, vault data, and test evidence were retained.
