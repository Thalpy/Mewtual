# Native derivation Input acceptance correction

The Windows native suite at 0e32b443 (run43569805855) passed1032 tests
and failed21. The first actual failure was the rejected-confirmation scenario;
the remaining20 were intentionally refused by poisoned fixture isolation.
The failure compared encrypted reply classes: Control0 versus Data1. It did
not demonstrate acceptance of a malformed cryptographic result. Confirmation
rejection/history assertions had not yet been reached. Linux native passed.

The preceding derivation fixture decoded its one-shot Input reply as Data.
Production legitimately returns authenticated Control Capacity when a short
native/Actor binding cannot be acquired. The exact Control plaintext in that
old CI run was not logged, so Capacity is not established as its precise cause.

The fixture now retries only independently authenticated exact Protocol(Capacity)
for the SAME original row/action/selector/purpose. Each physical request uses
the next real Session sequence and collects its reply before a Clock wait.
There are at most32 observations under one watchdog using the remaining original
Root deadline. That watchdog includes the forced refusal, all Actor/UI waits
and all retry sleeps. Data must also arrive before the original deadline.
Serialization errors and every other Control remain failures. There are no
attempt refunds, new actions, renewed deadlines or outcome alternatives.

The existing genuine six-client rejection scenario now holds the real servers
mutex while polling that SAME helper. A scheduling observation fires only after
it decrypts exact Capacity and drops the actual reply. The row must be unchanged;
then the real guard is released and the same future must obtain Data before the
unchanged downstream result-rejection/history/ACK/settlement checks. Returning
the first Control unconditionally fails this deterministic barrier.

Read-only adversarial review is clear after removing a proposed invalid synthetic
positive fixture. Rustfmt and diff checks passed. Compilation, the focused real
TCP scenario and complete native validation remain pending for this correction.
No new acceptance test name or focused count was introduced.

## Exact storage-reservation refusal evidence

Linux native run43576853387 at f443dd92 passed1060 tests and failed4.
The first actual failure expected a stage5 SDP reservation refusal but observed
stage3 Capacity; the other three failures followed poisoned fixture isolation.
Stage3 preflight precedes creating the SDP intent. The shared progress table was
also lossy, so its old completion tuple could inherit a stale stage. That tuple
alone cannot conclusively identify the old request's refusal boundary.

The fixture now retains the same ordinary-pool pressure while forcing the real
servers mutex refusal and proving no intent was created. It then releases only
that mutex and requires actual stage5 Capacity before continuing. Both observations
use the exact physical request sequence, allow at most32 authenticated Capacity
retries, and keep the same parent/action/body/proof and original deadline. Missing
diagnostics are never evidence of either boundary. Mutex acquisition, exchanges
and sleeps are bounded by the original remaining Clock deadline. All downstream
fixed-intent, drainage, proof recovery and lost Seal/Open-result assertions remain.

Test-only completion now publishes stage, parent, physical sequence and refusal
from one call's local stage rather than reading best-effort shared progress. The
pure regression deliberately omits a progress update and proves completion cannot
inherit an older request's boundary. No production protocol or attempt policy is
changed. Final-source real-TCP lost-result regression passed (one test,5.92s),
and the local completion-attribution regression passed. Read-only actual-diff
re-review is clear. These focused runs used the same native correction in the
UI-state integration worktree. Full native execution at the corrected checkpoint
is still pending; no completed acceptance or production activation is claimed.
