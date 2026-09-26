# Admission storage recovery

Admission now reports the outcomes of its core snapshot, transport identity and registry
writes separately. The UI warns when any remains unfinished. These files do not form an
atomic transaction, and the warning does not promise recovery from a crash before their
first successful writes.

A failed network-record write retains its original seed and sequence reservation in one
pending slot per live actor. The existing persistence worker, discovery pass and close
coordinator retry it. Exact actor-incarnation and transport-identity checks prevent a
replacement actor or a removed conversation from consuming an old obligation. Failed
reads preserve the highest reservation for the same seed; a conflicting saved seed is
never silently adopted or overwritten.

Retries read the current sealed record under store and registry custody. Newer saved
routes, consent and metadata take precedence over the earlier admission values; the
sequence reservation can only increase. Registry retries serialize the current registry
and retire their dirty ticket only after a successful write. No additional timer or
parallel vault is introduced.

Cold reload reserves fresh sequence space durably before constructing a transport. A
failed reservation or unreadable identity refuses that reload instead of continuing with
unreserved sequence space. Close retries pending identity and registry writes before
member-route finalization or actor freezing. The existing ten-second close deadline
remains cooperative: it cannot preempt a synchronous filesystem call.

The seven real-store regressions in `admission_storage::tests` cover initial write failure
and quiet recovery, newer-record preservation, replacement fencing, conflicting identity,
registry freshness, reload refusal and monotonic reservation retention after read errors.
Executed results belong in the integrated review report. Independent implementation and
integration reviews found no remaining blocker/high in these boundaries.
