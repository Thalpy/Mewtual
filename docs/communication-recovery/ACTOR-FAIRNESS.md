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

Regressions added (execution belongs to integration; no test success is claimed here):

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

Integration must also rerun the existing throttled delivery wake, reciprocal reconciliation,
multi-page/legacy catch-up, cancellation and Studio scheduling regressions. No timeout inflation or
idle command window is part of this correction.
