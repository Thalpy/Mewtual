# Chat reconnect and history recovery

Status: bounded implementation and independent static review, 2026-09-26.
Executed checks and unresolved acceptance cases: [scope and evidence](communication-recovery/STATUS.md).
Base: `48f9069`, branch `Chat-method-redesign`.

This continues the supplied reconnect review. The first change set repairs concrete lifecycle,
record-exchange, persistence, and neighbor-reconciliation gaps. It does not claim that every
reply-code connection is restart-safe or that a dedicated security mode already exists.

## Security contracts

The current product is a member mesh. P2P groups should automatically exchange signed member
records and authorized history through changing neighbors. Direct connections between every
pair, an always-online inviter, and an online document owner must not be prerequisites for
ordinary chat. Store-and-forward requires each intermediate replica to retain the document and
its verification material. A transport relay and an admission helper do not promise that custody.

A future dedicated group must bind its group identity, mode, policy version, designated service
authorities, forwarding rules, and recovery rules in authenticated creation state. Invitations
and authenticated establishment must bind that policy. Persist and verify the same policy on
restore. Mode is fixed at creation until a separately authenticated migration exists. A device's
relay/switchboard role is not the group mode. Unknown policy must fail closed; legacy groups
need an explicit migration rule. An unreachable dedicated service must preserve local work
without enabling a P2P fallback. A local boolean or UI selector cannot establish these guarantees.

Dedicated admission must gate every direct and forwarded service request before disclosing
history, inventory, member addresses, or key material. Requester-bound, expiring recovery grants
need receiver-side size, time, and aggregate budgets, including across helper/identifier rotation.
Bounded revocation and abuse state must survive service restart. These protocol changes are
separate unfinished work; this patch must not present them as enforced.

## Implemented boundaries

1. **Admission records:** publish the joining device's signed record and attempt a timeout-bounded,
   connected-only member PEX fetch after every successful supported admission method, before its
   initial snapshot. A failed or empty response does not undo membership or establish restart
   safety. PEX pulls records; it does not acknowledge that the remote has fetched our record.
   Keep this independent of permission to reuse a local endpoint. Neither a signed
   record returned by PEX nor an inbound source socket proves an authorized reverse route.
   Preserve helper/reply capability expiry and existing route policy.
2. **Lifecycle:** successful warm unlock wakes the existing discovery worker using its coalescing
   watch signal. Reuse actors and transports, authenticate first, retain session-generation
   fencing, and suppress duplicate already-open unlock wakes. A wake means work was requested,
   not that connection or history recovery completed.
3. **Neighbor reconciliation:** periodically reconsider open documents against currently proven
   connected members using the existing bounded catch-up queue. Preserve pagination, cooldowns,
   author verification, and existing source selection. A quiet network wakes eligible cooling
   work using the injected clock. The 30-second minimum backoff gains a 1-second offset in one
   peer-order direction so simultaneous reciprocal timeouts do not wake in lockstep. Selection
   checks current transport liveness before preferring an old connected-source observation.
   Rechecking unchanged neighbors lets history learned through another neighbor propagate
   without requiring a new chat message or a socket between every pair of members. Rotation
   admits later documents when queue capacity returns; permanently unfinished tasks can fill it.
4. **Truthful persistence:** return whether the exact accepted actor incarnation's snapshot was
   saved, remains pending, or was superseded. Only a successful covering write is durable.
   Retain dirty counters and retry on the discovery cadence without creating a new message.
   The frontend must not restore an accepted message into the composer because snapshot or
   subsequent refresh failed. Fence late completions against a replacement UI session.
5. **Receive persistence:** independently track raw document operation counts/identities and the
   membership epoch in the actor. A native-only snapshot invalidation must not depend on visible
   message rows or roster counts changing. Before the UI lock gate, mark the exact running server
   incarnation dirty and wake its single coalesced persistence worker. Never await that actor from
   its event consumer. Save through the existing snapshot locks/counters; retain failed writes for
   retry. The mounted store can save encrypted received state while the UI remains locked.
   The worker requests an initial snapshot and batches subsequent wakes in fixed 250-ms windows;
   that interval is not a write-completion deadline. The raw tracker replaces its current
   document map on each owner turn, costing O(open documents) with no message-body scan and no
   accumulation of removed document identities. It supplements existing persistence triggers;
   it does not version every field in the server snapshot or alter Studio persistence barriers.

The admission, mesh, and persistence implementation agents each inspected the plan adversarially;
the primary agent reviewed their proposals before production edits. The mesh agent independently
reviewed the warm-unlock design. Findings rejected automatic promotion of reply endpoint authority,
a UI-only dedicated flag, and ordinary send errors after local acceptance. Independent inspection
of the implementation also identified reciprocal retry lockstep and missing receive-only/raw-state
persistence; both received corrections and regression tests. Review dispositions and executed
results are tracked separately in [STATUS.md](communication-recovery/STATUS.md), under
[the review doctrine](ADVERSARIAL-REVIEW.md).

## Reusing the epoch work

Reuse stable signed operation identities, paginated exchange, retained pending work, injected
clocks, and session/server ownership fences. Keep membership/key epochs, document generations,
owner tenure, and local runtime generations separate. Chat does not enter Flipnote's
owner-controlled Open/Closing/Settled lifecycle in this change. Provider hints remain distinct
from verified document state. Snapshot persistence is local durability, not remote custody or
proof that all history everywhere has converged.

## Acceptance and remaining work

| Scenario | Required evidence |
| --- | --- |
| Reply/assisted admission then snapshot/restore | Successfully fetched signed records survive; no fabricated endpoint authority |
| Mounted lock/unlock | One recovery wake, actors reused, stale unlock cannot reopen the session |
| Save fails after message acceptance | Pending outcome, dirty state retained, retry saves the same message |
| UI refresh fails after accepted send | Composer does not invite a duplicate send |
| First source is stale | Other current member sources remain eligible |
| A-B-C only, B learns C's history after serving A | A can later obtain that history from B without A-C requests |
| Cooldown on a quiet network | Pending work resumes on the injected clock |
| Repeated discovery/unlock | No new worker per wake; bounded queue, cursors and backoff preserved |
| Receive-only member, including locked UI | Incoming history requests an encrypted snapshot without a local send |
| Signed change does not alter rendered messages | Raw document movement still requests persistence |
| Membership epoch changes with the same roster count | Raw epoch movement still requests persistence |

Persistence remains asynchronous for received history, and local sends still publish before the
native snapshot. A process or actor can stop before a requested write completes. Dirty tickets and
worker wakes are in memory; a failed or pending save does not establish crash-safe custody. The
`SnapshotNeeded` event requests a write and carries no renderer payload or durability acknowledgement.

Still required beyond this patch: an authenticated continuing reconnect agreement that preserves
the successful direction and explicitly authorizes private routes after reply admission; real
two-client asymmetric-NAT and independent/simultaneous restart tests; authenticated dedicated
mode and legacy migration; retained document/attachment coverage and storage policy; membership
recovery beyond retained key windows; persisted dissemination/custody acknowledgements and
full process-restart bridge acceptance. No network can recover history if no usable time-ordered
path and no surviving authorized copy exist.

The complete programme's requirements are preserved in
[IMPLEMENTATION-PLAN.md](communication-recovery/IMPLEMENTATION-PLAN.md); this patch does not close
an entire CR work package or replace its real-device and process-restart acceptance requirements.
