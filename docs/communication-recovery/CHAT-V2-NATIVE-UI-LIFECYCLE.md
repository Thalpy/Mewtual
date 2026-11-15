# Native UI key retirement

Status: implementation under focused validation. Chat V2 remains off. This
extends the disabled native warm factory; it is not working message send,
SDK shutdown, cold restore, UI continuity persistence or physical acceptance.

## Original retirement ownership

Native lock publishes its actual sequentially consistent UI generation before
waiting for Root. Both lock and authenticated unlock probe retirement before
acquiring the UI commit mutex. Root's bound is inclusive, so native passes
`generation - 1`: the newly authenticated same generation must remain open.
The selected native startup starts at generation1; zero remains unauthenticated.

One native coalescer owns the actual Root batch in an independently spawned task.
An IPC waiter does not own or cancel this task. Eight paid waiters are admitted;
excess contention returns a retryable refusal. Retirement uses its own metadata
account so exhausting the canonical-generation registry cannot prevent cleanup.
Every candidate and caller has its own invoice. The actual task/control ABI is
checked against the fixed allowance rather than assuming pointer-size wrappers.

A completion guard is armed before extracting or publishing the original batch.
The affine batch is destroyed before failure notification, including an unpolled
task drop or a panic between publication and spawn. Failure alone never proves
that admitted work ended. A non-cloneable Root observation retains the original
fixed array of key-counter records, without a UI owner, key capability, physical
Store, registry or SDK. It is installed in the paid job before publication.

A failed job recovers only after those original counters are closed AND zero.
Open-but-zero is insufficient. A replacement mount's empty registry is also
insufficient. Missing original proof refuses. After any completed or recovered
job, native probes the actual current Root again before collecting retired slots
or reporting success. Highest admitted generation never moves backwards on GC.

This observes key work only. A zero counter does not prove physical workers,
native SDK tasks, message custody or the entire application's shutdown finished.

## Lock and authenticated unlock ordering

A native mutation prelude registers before the lock generation is published. It
retains that registration through exact snapshot custody and the early
`session_resumable = false` assignment. Authenticated unlock waits for preludes
outside UI commit and rechecks idle/current-generation/original-mount negatives
around final authorization. A prelude finishing is not a successful snapshot save.

Once these mutations and old key work are complete, a correct same-generation
unlock may acquire UI commit before the old lock's remaining cleanup. That old
lock then observes the actual current authorization and resumable state and does
not clear the newly opened session. Cached renderer state never grants this
exception; native's actual authorization generation must match.

## Native window close

Every actual close registers an unconditional fresh negative generation before
awaiting close policy, even when acknowledging previously latched continuity
loss. An outer prelude lasts through actual destruction or the deferred-close
response. Unlock cannot reopen between final drainage and window destruction.

Snapshot failures and key-work failures remain separate internally. Explicit
continuity-loss acknowledgement cannot discard a key-work failure. Every path
allowed to destroy rechecks actual Root key drainage before the existing history
freeze and window operation. Existing history freeze remains a separate barrier.

## Evidence and remaining work

The Root focused cohort passed 12 cases, including two new tests with original
RT key ends and the open-zero/replacement-registry negatives. The native focused
cohort passed all 19 cases (35.74 seconds): the eight original real-router cases
and eleven new retirement/ordering cases. The latter cover forty lock/unlock
cycles, coalesced waiters with a lost response, unpolled task drop, publication-gap
panic, paused mutation preludes, same-generation authentication winning commit,
close-debt acknowledgement, a real coalescer failure that cannot be discarded,
cached prelude checks after the initial wait, same-generation Voice replacement
after the original pre-auth ticket, and shared Legacy authentication ordering.
Hooks control scheduling or force a real negative boundary; they grant no key.

The stronger prelude test exposed an actual stale-ticket failure. Both native and
Legacy entry now wait for prelude quiescence under the mount mutex, recheck the
original UI generation and only then capture their voice ticket BEFORE real
verification. Moving that capture after KDF would adopt an intervening Voice
session and is separately rejected by the new actual replacement test.

Workers that hold admitted key Ends must not wait for these same preludes while
retaining those Ends: key retirement waits on them. The native retirement worker
itself receives only the original batch and never waits on a prelude.

Read-only adversarial design/diff review is clear after fixing the regression
vacuity gaps and stale-ticket ordering. The complete frontend3441 tests,
Svelte check (zero errors/warnings), production build, ambient dependency gate,
store raw-fs gate and CI guard self-tests passed for this revision. The reviewed
native derivation Input fixture correction is also included; its actual TCP
regression remains pending. Full root/native execution, strict linting,
format checks, dependency audits and actual build/startup remain pending.
Prior warm-factory evidence does not sign off these new changes.

SDK-native production acceptance is still absent. UI state get/save and lock
continuity still use Legacy storage and require a distinct typed native
implementation before activation. Operational creator/member, cold restore,
joined source/archive, transport/projection/admin and multi-peer acceptance
milestones also remain open. Chat V2 is off and is not complete.
