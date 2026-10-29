# Inactive issuer-bound call activation draft

This draft prepares and selects a call profile before key delegation. The native entry
is private and unregistered: no Tauri handler, App activation, key, Ready, SDP or media
producer is enabled. A successful selection is not permission to use an old crypto path.

The actual Root issuer creates an immutable admitted lease. Legacy endpoint factories,
queued source work and legacy ingress cannot write into a selected admitted call. A
healthy existing participant cannot be upgraded in place. Epoch changes close admitted
children before inherited callbacks run; notifications and actual destruction happen
outside the original Group/Policy guards, including callback-panic paths.

Native retains the exact prepared pair and original paid work. Its original UI-session
guard orders Root selection against lock, replacement and epoch invalidation. Root
selection stays inert until the native final check. Closed or unavailable weak pointers
never certify owner return. The original one-way residence marker distinguishes actual
Root destruction from a provably unclaimed acquisition right. Lost replies recover the
same outcome; cleanup and dropping an observer never implicitly acknowledge that outcome.

The Actor's paid control owner stays local to its actual inbox. Native history keeps a
real `std::Weak`, original instance/resource/group pin and the same funding through the
final weak allocation. No strong Actor or raw Tokio WeakSender enters native history.
Only the final open check and Permit.send occur under the same send/retirement gate.
Callbacks, allocation, argument destruction and Root teardown stay outside it. Receiver
RAII closes that gate on abort, panic and unpolled-future destruction. Terminal paths
retire Root and remove the control before blocked close-event delivery can delay custody.

Focused evidence before native validation:

- Root call-activation tests: 17 passed; source-retirement regression: one passed.
- Actor activation/control tests: 14 passed, including both send/receiver orderings,
  retained ordinary handles, blocked Closed delivery, weak-tail funding, distinct actual
  Roots sharing group/resources, callback unwind and lost replies.
- The two adapted legacy watch/snapshot regressions pass with their original assertions.
- Root retained/preparation graph measures 1,432/4,096 bytes; actor RPC graph measures
  2,392/4,096 bytes. The independently paid control-owner graph guard passes within its
  separate 4 KiB fee. Native 16 KiB cell/task and complete boundary inventories are still
  provisional until their actual native compilation/layout checks run.
- Strict sync/app Clippy passes. Read-only design/review and the scoped fixture re-review
  have no remaining findings. Native tests and complete integration suites remain pending.

Review also confirmed an inherited normal-terminal mailbox gap in old queue families:
a pre-close permit can send after receiver destruction while an ordinary Sender remains.
That issue predates this draft and is assigned a separate mailbox-retirement leaf. The
new-family gate blocks that late send for this draft; it does not claim to retrofit every
older family or solve their abort/panic behavior.

Still required: production command/browser integration, bilateral query/agreement and
role-specific consumption, original-key/KDF/Ready and SDP/ICE/RTC handoff, relay and
combined/physical acceptance. V2 remains disabled. This draft is not ready for activation.
