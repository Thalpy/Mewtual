# Repair progress with a full queue

Periodic reconciliation now rotates one waiting open-document task out of a full ready queue so
another document can run. The open-document inventory retains the obligation; provider cursors,
continuation claims and cooldowns remain attached to the parked document. The rotating inventory
cursor revisits it on later passes. Membership/control tasks, in-flight work and tasks without an
open document are not parking candidates. No extra unbounded waiting queue is allocated.

This addresses permanent slot occupancy by unavailable histories. It does not remove existing
network deadlines or make an individual sole-owner outbound request nonblocking. A queue
consisting entirely of protected tasks must still wait for one of those tasks to yield capacity.

All six reconciliation regressions pass, including a full queue that is deliberately never drained:
every one of five documents is revisited with only two slots, and its provider cursor survives.
The existing forced-chain, quiet retry, reciprocal timeout and source-liveness regressions pass.
Evidence: `logs/cr-queue-fairness-tests.log`. Independent static review found no blocker/high.
