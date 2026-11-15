# Chat V2 renderer retirement

Chat V2 remains disabled. This change fixes two renderer lifecycle races; it
does not add joined-member delivery, shared history or cold authoring restore.

Lock clears the Native Found busy owner synchronously. A delayed completion
may release that owner only in its original UI generation, so it cannot clear
a newer Found operation after reauthentication.

Component destruction permanently retires that component before other Svelte
cleanup hooks. It invalidates local generations, cancels the debounce and retry
scheduler, and refuses new continuity saves, hydration and native-driver work.
Registration precedes media cleanup because a throwing cleanup can prevent
later Svelte callbacks from running. Retirement is idempotent and does not lock
or close the native vault, change its authentication generation, or submit an
empty snapshot.

The native ingress watermark already orders writes admitted before destruction.
The renderer must additionally prevent an old timer or late send cleanup from
submitting its stale journal with a newer native ingress order after remount.
Already-submitted native work keeps its existing custody and may finish.

The focused tests execute the actual App functions against two instances sharing
one simulated native store. They cover both pending and newly attempted saves,
an already-admitted write, late send cleanup, idempotence and a real Svelte
teardown that throws after retirement. A controlled pre-fix reproduction
overwrote the replacement component's journal. The browser flow unmounts and
mounts the actual App without reinstalling its native mock, holds replies across
replacement and proves that the newer draft and exact pending intent survive.
These tests establish renderer behavior, not native authentication, physical
durability, SDK drainage or peer delivery.

Read-only adversarial re-review found no remaining blocker, high, medium or low
finding. All3,485 frontend tests passed with zero skips (70.11s), Svelte checking
reported zero errors/warnings, and the production build passed (1m07s). The14
focused actual-function/teardown cases and all10 existing browser flows passed;
the separate Native Text browser run passed restoration, actual App remount and
empty-vault creation. Ambient-dependency and raw-filesystem gates passed.
No Rust changed; exact-head complete Root/native CI is queued separately and
remains required before acceptance. Root strict Clippy's564 inherited failures
remain unchanged. The one user-deferred catch-up cost test remains reserved
for final Chat V2 acceptance.
