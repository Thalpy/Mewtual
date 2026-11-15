# Inactive issuer-bound call activation draft

This draft prepares and selects a call profile before key delegation. The native entry
is private and unregistered: no Tauri handler, App activation, key, Ready, SDP or media
producer is enabled. A successful selection is not permission to use an old crypto path.

The actual Root issuer creates an immutable admitted lease. Legacy endpoint factories,
queued source work and legacy ingress cannot write into a selected admitted call. A
healthy existing participant cannot be upgraded in place. Epoch changes close admitted
children before inherited callbacks run. New admitted-child notifications and retained
alias destruction happen outside the original Group/Policy guards, including callback-panic
paths. Inherited Existing/ActorGate/session notification paths are not rewritten here.

Native retains the exact prepared pair and original paid work. Its original UI-session
guard orders Root selection against lock, replacement and epoch invalidation. Root
selection stays inert until the native final check. Closed or unavailable weak pointers
never certify owner return. The original one-way residence marker distinguishes actual
Root destruction from a provably unclaimed acquisition right. Lost replies recover the
same outcome; cleanup and dropping an observer never implicitly acknowledge that outcome.

The private runner owns the actual boxed execution future from outside that future. On
completion or cancellation it destroys the same Box and all its captures before publishing
the task-return marker. Recovery cannot reuse the native payment while the old execution
body is still being destroyed. An empty outer executor shell is not a claim that Tokio has
synchronously freed its scheduler cell. Normal Forget and Coordinator teardown keep the
same final Cell/payment through actual index-backing deallocation, including revocation
unwind; End and destruction still do not acknowledge a lost result.
All recovery tickets use one original `Arc<AtomicU64>` return stamp. Only equality with
the exact current checked serial permits a negative replacement; stale/future stamps and
serial exhaustion refuse. A paused older marker cannot allocate a new retained signal
for each retry or authorize the successor.

The Actor's paid control owner stays local to its actual inbox. Native history keeps a
real `std::Weak`, original instance/resource/group pin and the same funding through the
final weak allocation. No strong Actor or raw Tokio WeakSender enters native history.
Only the final open check and Permit.send occur under the same send/retirement gate.
Callbacks, allocation, argument destruction and Root teardown stay outside it. Receiver
RAII closes that gate on abort, panic and unpolled-future destruction. Terminal paths
retire Root and remove the control before blocked close-event delivery can delay custody.

Focused evidence on the rebased draft:

- Root call-activation tests: 17 passed; source-retirement regression: one passed.
- Actor activation/control tests: 14 passed, including both send/receiver orderings,
  retained ordinary handles, blocked Closed delivery, weak-tail funding, distinct actual
  Roots sharing group/resources, callback unwind and lost replies.
- The two adapted legacy watch/snapshot regressions pass with their original assertions.
- All nine native activation tests pass: actual Root completion and lost-reply recovery,
  lock while Root work is parked, cancellation before the task is first polled, and
  rejection of inline or exhausted Clock implementations without an automatic spin;
  measured inventory, completion/cancellation destruction barriers, suspended cancellation
  joining the same Root Work, Coordinator backing return/revocation unwind, and 65 exact
  same-stamp recoveries with stale/future/overflow refusal (0.76s test execution).
- Root retained/preparation graph measures 1,432/4,096 bytes; actor RPC graph measures
  2,392/4,096 bytes. The independently paid control-owner graph guard passes within its
  separate 4 KiB fee. The final native graph measures 17,752 bytes on Windows, including
  the opaque execution future, weak Context allocations, index backing, native callback
  overlap and sanctioned Clock bodies. Its admission guard correctly refused the old
  provisional 16 KiB fee (the earlier 17,552-byte inventory had two tests pass and six
  refuse). The fee is now 18 KiB within unchanged 512 KiB/group and 2 MiB/process limits.
  Actual Runtime EffectiveClock plus source/header/backing chains measure 72/256 bytes
  with SystemClock and 192/256 bytes with ManualClock; their sleep bodies also fit.
  Arbitrary trusted Clock implementations are not promised an arbitrary retained heap;
  the production SystemClock/Runtime adapter and deterministic ManualClock are the measured
  sources. Allocator/RSS overhead is outside these logical application-owned inventories.
- Strict sync/app Clippy passes. Read-only design/review and the scoped fixture re-review
  have no remaining findings on the original draft. The native re-review identified index
  teardown ordering and the missing complete native inventory; both have corrections and
  new destruction regressions that now pass. Final actual-diff re-review has no remaining
  findings. The first strict native Clippy
  invocation stops because this worktree has no `apps/desktop/dist`; the frontend build then
  passes, and the next lint invocation identifies staged-unused entries. Those entries now
  have narrow annotations explaining their inactive status. Strict all-target/all-feature
  native Clippy and the separate native `cargo check` both pass (1m47s each). Complete
  integration suites remain pending with the primary.

Review also confirmed an inherited normal-terminal mailbox gap in old queue families:
a pre-close permit can send after receiver destruction while an ordinary Sender remains.
That issue predates this draft and is assigned a separate mailbox-retirement leaf. The
new-family gate blocks that late send for this draft; it does not claim to retrofit every
older family or solve their abort/panic behavior.

Still required: production command/browser integration, bilateral query/agreement and
role-specific consumption, original-key/KDF/Ready and SDP/ICE/RTC handoff, relay and
combined/physical acceptance. V2 remains disabled. This draft is not ready for activation.
