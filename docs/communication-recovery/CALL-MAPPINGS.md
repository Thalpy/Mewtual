# Optional call router mapping custody

18 October 2026, communication Packages 0/1. Implementation and focused checks in progress;
this decision does not accept the complete communication programme or its media path.

## Problem and decision

The earlier frontend discarded late mapping responses, but native acquisition could still install
after leave/lock. Replacing an entry mapped the new port before deleting the old same-port entry.
Cleanup rediscovered the current gateway, so a network change could target another router.
An acknowledged Add followed by a failed external-address query lost its cleanup handle.

Use one process-owned native coordinator. A controller identifies the unlocked webview mount;
a call capability identifies one join and captures UI generation, server registry incarnation,
group and room; a grant identifies one port attempt. A UI counter or port number grants no release
authority. Prepare is non-destructive; activation compares the captured native revision after the
frontend checks its own mount/join. End names the exact capability. Lock invalidates before
awaiting cleanup. Server leave retires the exact registry incarnation.

Acquisition and cleanup run outside actor/UI commit locks. A native worker remains responsible
when its IPC waiter disappears. Before installation it obtains the current UI commit guard,
revalidates session and registry scope, and checks the coordinator's exact owner. A retired result
goes to cleanup. Port ownership lasts through acquisition and cleanup; a replacement gets busy
instead of queuing an unbounded waiter or racing a previous deletion.

IGD grants retain the original gateway and exact external UDP port. After acknowledged Add,
failed address validation cleans that grant before a clean fallback. Each mutating/query RPC and
the whole confirmation scan has a separate five-second allowance through the Clock seam.
Confirmation timeout yields an owned unconfirmed grant, not an unowned failure. Add timeout,
refusal or uncertain deletion yields no usable grant and no automatic protocol fallback.
Deleting after an unacknowledged Add could remove a pre-existing entry, so it is not attempted.

## Bounded uncertainty

The existing `portmapper` Client has no public joined shutdown. Drop requests cancellation;
`deactivate` and watcher `None` do not prove worker completion or router deletion. No new mapper,
dependency or stronger shutdown claim is introduced. Even a failed probe retains the attempt's
charge after Client creation. A successful PCP/NAT-PMP attempt owns that charge through call end;
retirement is then uncertain. Only the first advertised fallback protocol is attempted.

The coordinator holds at most 64 slots across current, acquiring, cleaning and uncertain states.
Uncertain slots and their port exclusions last for the process lifetime, across lock/unlock,
account changes and remount; repeated End cannot refund or multiply a charge. At exhaustion,
further optional mappings refuse and existing STUN/TURN candidates continue. This conservative
availability cost is deliberate until a separately reviewed owned shutdown extension exists.
Prepared candidates have a separate limit of 8 and expire after 30 monotonic seconds.

UPnP deletion failure, ambiguous RPCs, crashes and router-assigned lease lifetimes remain limits.
Restart loses process-local exclusions and cannot establish ordering against a previous router
effect. A mapping is a candidate, not external reachability or audible WebView evidence. The
existing claimed/default-interface gate remains; no misleading UDP liveness probe is added.

## Verification boundaries

Focused store-free coordinator tests cover revision CAS, stale End/unmap, pending/cleanup
custody, remount/lock, actor replacement, capacity, expiry and idempotent uncertainty. The shared
spawned acquisition seam additionally tests a dropped command waiter, deferred router/cleanup
results, a lock during final authorization, and retaining the authorization guard through install.
Injected IGD tests cover exact gateway/port cleanup, partial success, address/confirmation
timeouts and ambiguous Add/delete outcomes. Actual-App RTC tests cover delayed callbacks,
SDP/stats/default-route continuations and teardown of the old call on component destruction.

These are local deterministic regressions. They do not prove router behavior, PCP shutdown,
physical NAT traversal, room forwarding or native audio. Complete branch suites and final
adversarial review remain recorded in [the acceptance ledger](CHAT-FILES-VOICE-ACCEPTANCE.md).
