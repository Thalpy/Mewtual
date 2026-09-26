# Orderly close saves group history

A normal window close now locks the UI, saves each running server through its actor, freezes
further actor work, and only then destroys the window. The native bridge retains the UI-session
and server-registry fences through destruction. It rejects a changed session or actor incarnation.
No wire or sealed snapshot representation changes.

The actor sends Ready before native acquires store custody. All locks acquired after Ready are
try-locks, so an existing snapshot or Studio transaction cannot deadlock against a frozen actor.
The store write is synchronous under the lease; releasing native locks precedes the frozen result.
Dropping Ready, cancelling the close, or abandoning a frozen handle resumes that actor. If one
server fails, earlier frozen servers resume too. Only successful window destruction commits stop.
A ten-second outer deadline bounds waiting, without cancelling a synchronous store write midway.

A history-save failure leaves the window locked and open for retry. The existing option to
acknowledge lost screen continuity cannot bypass a failed history save. This barrier protects an
orderly close; abrupt process termination before a background save remains separate durable-send
work. It does not claim new receiver custody acknowledgements or a multi-file transaction.

Adversarial inspection covered cancellation at Ready and after save, per-server save contention,
session/actor replacement, detached work, and destruction failure. The resume handles and final
session/registry fence are load-bearing. Independent subagent review found a HIGH stale-backup overwrite: backup captured actor bytes
outside the ordinary persistence lock and could replace the final shutdown save. Backup now uses
`persist_server_instance` for every actor and rechecks the UI generation and complete registry
incarnation set before copying. Re-review found no remaining blocker/high in the close boundary.

Verification: the two core shutdown regressions pass, including reopening the sealed store after
stop. Frontend type checking reports zero errors and warnings. Native library compilation and the two-group contention/cancellation/reopen regression pass.
The native test retains the existing unused-field warning in `security_intent.rs`. Tests use actual actors and encrypted stores, not a mocked successful write.

The follow-up `backup_snapshot_cannot_overwrite_a_later_shutdown_save` regression passes
(1 selected test): an outstanding backup write makes close defer, then a retried close preserves
the later message. Raw evidence: `logs/cr-backup-close-test.log`.
