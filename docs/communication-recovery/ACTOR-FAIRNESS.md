# Actor progress during command traffic

Queued document and membership recovery now retains one request future, its original signed
request context and its task on `ChannelSync`. A command can cancel a poll of `sync_once` without
discarding the response wait, regenerating a nonce or cooling an innocent source. The owner polls
that wait alongside inbound transport events, alternating when both are ready. Reciprocal members
can therefore answer each other while recovering. Paged and membership requests retain the existing
two-second deadline; the whole-history compatibility request retains its existing larger deadline.
There is one request slot per sync owner, with no new task, wire grammar, connection or dial authority.

Completion runs only on the mutable owner. It checks current MLS epoch and active local membership,
and refuses a stale peer cursor before the existing request-bound signature, responder membership,
signed-operation and generic-document ingestion checks. Epoch movement cancels the obsolete wait
without retiring its recovery obligation. A document edit during the wait does not discard valid
history, but reopens source accounting for the new frontier. Ordinary documents are append-only in
this owner; replacing a server replaces the complete owner and drops its wait. Studio and Registry
continue to use their separate lifecycle-gated exchange and refuse generic signed-op import.

Ready command, network, delivery timer, file-completion and preview-reset sources rotate in the
actor. A ready class receives a turn within five ordinary selections. Already-completed bounded
Studio jobs retain their previous priority before another native lease; only owner commands launch
those jobs. This is a turn-count bound, not a wall-time latency guarantee for every command handler.
Explicit request commands and some discovery/profile/admission handlers still await their existing
operations inline. Slow event consumers also retain the existing bounded-event-channel backpressure.

The delivery/Studio wake is anchored to its sampled monotonic deadline before creating the selector.
Advancing an injected clock before the future's first poll must not start another full throttle
interval. The local wait helper checks the original deadline before and after polling its sleep,
including advances during the sleep's own arming. Delivery projection still checks the throttle
before emitting; a conservative early Studio wake only revisits its existing eligibility checks.

The broader polling schedule required correcting queue ownership before transport awaits. Control
publications, Welcome/admission pushes, eviction commands, receipts and repair pushes retain their
row until the attempt resolves. Cancellation alone cannot discard accepted bytes. Their existing
success/error retry policies and caps remain; an uncertain submission may deliver an exact duplicate.
Durable chat already retains its original publication record and cadence, and subscription resync
already retains its uncertain-topic cleanup token. This change makes no additional crash-durability
promise for transient outboxes.

Regressions exercise the production owner and semantic completion, rather than assuming that one
`run_once` drains all recovery work:

- `delayed_signed_history_completes_while_actor_commands_remain_saturated`: actual signed response
  delayed after service, gossip dropped, eight continuous actor readers, at least 128 reads during
  the delay, exact original message rows received before command producers stop.
- `actor_ready_sources_rotate_under_continuous_commands_and_network`: all ordinary classes ready
  throughout two complete rotations.
- `cancelled_owner_polls_retain_the_original_signed_control_publication`: actual signed MLS removal
  publication survives 64 cancelled production sync polls and is submitted unchanged on release.
- `removed_member_discards_owned_response_without_retiring_its_recovery_obligation`: the exact
  response passes admission in an equivalent pre-removal context, then fails after real local
  removal without importing history, retiring the target or issuing a replacement request.
- `stale_commit_wait_keeps_its_stronger_gap_when_a_bare_probe_fills_the_queue`: a real epoch
  transition invalidates the owned wait while a weaker probe occupies the sole queue slot; the
  original stronger gap must merge into the surviving task before capacity checks.
- `actor_deadline_*`: deterministic first-poll and sleep-arming clock advances preserve the
  original deadline, while wall changes and pre-deadline polling cannot advance delivery.
- `simultaneous_reciprocal_catchup_serves_both_owners_without_a_timeout`: both production owners
  recover exact separately authored A/B content within 32 polling rounds, with gossip disabled
  and no injected-clock advance or serving helper during recovery. Each owner's request count is
  latched at its first complete history and must equal one; neither peer is cooling off. A faster
  owner may legitimately request a confirmation at the new frontier while the other finishes.
- `a_genuinely_missed_commit_heals_without_an_older_member_speaking`: bounded driving of both
  owners must advance the recipient to the verified epoch and current membership. It still
  requires exactly one commit request and one served commit, without an older-member post or
  injected recovery history.

Recorded integration evidence through `1912b34` (2026-09-28; paths name local evidence logs):

- Focused sync: **6 passed in 1.18s**: the four retained-recovery regressions, reciprocal owner
  recovery, and missed-commit recovery. `logs/six-client-sync-focused-final.log`.
- Focused app: **17 passed, 1 ignored in 30.82s**, covering deadlines, source rotation,
  continuously active readers, quiet delivery wake, cancellation, durable sends, shutdown and
  Studio owner-return scheduling. The ignored process-pool contention test intentionally owns
  all global preparation permits; run alone it **passed in 12.97s**.
  `logs/six-client-app-regressions-final.log` and `logs/six-client-owner-contention.log`.
- Temporal bridge: **1 passed in 7.22s**. Signed history survives a crashed sealed bridge and
  reaches a later member without an origin-to-recipient edge.
  `logs/six-client-temporal-bridge.log`. This is the existing actor/store topology fixture,
  which retains 300ms between observer reads; the separate continuous-reader regression proves
  progress under command load. It is not a physical-NAT test.
- Mutation check: removing commit requeue merging compiled, then the selected cap-one regression
  **failed as intended**: the queued bare probe retained `gap_at: None` instead of the held
  `Some(6)`. The integrating agent restored the source byte-for-byte at that checkpoint
  (SHA-256 `4E71F6B22AFE11D4E5BE23E09533AA00D9C698FFCBFE82AAE35967458CC30DB2`), then
  reran the focused six tests successfully. That hash records mutation restoration, not the
  later lint-adjusted source at `1912b34`. `logs/six-client-commit-gap-mutation.log` and
  `logs/six-client-sync-focused-final.log`.
- Strict core and native Clippy passed at `1912b34`.
  `logs/six-client-core-clippy-final.log` and `logs/six-client-native-clippy.log`.
- Existing integration binary `sync-f2b1660b382f34bc.exe`, built from that same Rust source,
  passed **29 tests in 20.50s** with `--test-threads=1`. It covers bidirectional gossip,
  removed-member access, missed/out-of-order commits, forks, late key-window recovery and
  interrupted multi-member history exchange. `logs/six-client-sync-integration-final.log`.
  The similarly built TCP and rendezvous integration binaries each passed their one test
  (0.22s and 0.25s); `logs/six-client-tcp-integration-final.log` and
  `logs/six-client-rendezvous-integration-final.log`.

These integration binaries were built before the broad package command exhausted disk while
linking a different target (`tcp_rendezvous_e2e`). They were then executed directly. The failed
package invocation, `logs/six-client-sync-suite-final.log`, is not a full-package pass. The
library suite is run separately with `cargo test --locked --offline -p catcoms-sync --lib -j1 -- --test-threads=2`.

The subsequent full native suite at the same Rust source revision passed: **301 passed,
0 failed, 0 ignored in 263.32s**, including both six-client variants and their negative delivery
assertions. `logs/six-client-native-suite-final.log`; command and scope are in
[SIX-CLIENT-RECOVERY.md](SIX-CLIENT-RECOVERY.md).

The complete sync library run at that same Rust source revision passed: **303 passed, 0 failed,
0 ignored in 224.74s**. It includes the large-history transfer-cost regression, multi-page and
legacy recovery, reciprocal recovery, cancellation, membership and lifecycle checks.
Output: `logs/six-client-sync-lib-final.log`. The other five integration targets were not executed
in this final local checkpoint; the failed broad package link is not relabelled a full-package
pass. Workspace/native formatting and the unchanged ambient-dependency gate passed. The new
fairness regressions require no timeout inflation or idle command window.
