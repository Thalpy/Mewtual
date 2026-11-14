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
