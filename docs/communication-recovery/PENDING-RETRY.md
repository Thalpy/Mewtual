# Quiet retry of unfinished UI sends

The unlocked UI now schedules one retry pass for unresolved caller intents after five
seconds, doubling the interval to at most sixty seconds. An empty eligible queue resets
the backoff. Foreground sends and existing retry passes pause the timer. Lock, session
replacement and component teardown cancel its wake and invalidate queued callbacks.
Browser timer throttling can delay a pass; these intervals are pacing limits, not a
delivery deadline or a guarantee that a suspended WebView runs in the background.

Each pass uses the existing `retryPendingSends` and `submitPendingSend` paths. Those paths
seal continuity before invoking native authoring and reuse the intent's original token,
conversation, payload, reply target and authoring context. A failed continuity write
cannot reach native authoring. Busy storage, capacity pressure, failed writes and ambiguous
IPC completions retain the same intent for a later pass. At most the existing bounded
pending-intent collection is scanned, sequentially, with no per-message timer.

Explicit invalid-input, token-conflict and changed-context refusals pause automatic retry
for that intent. A small `retryBlock` enum is sealed beside the intent so reopening the
vault preserves the pause after that continuity write succeeds. If the write fails, the
previous sealed disposition survives; a later unlock may recheck the native refusal.
The native outer `CHAT.SEND.REJECTED` error also wraps
temporary storage errors, so classification uses its inner stable refusal code rather
than treating every rejection as permanent. Unknown future stored block values remain
paused. Manual retry preserves the original identity. Recovering text never publishes it;
the user must explicitly put it into a composer and send under the new context.

Settings > Pending messages lists the whole vault's pending requests, including requests
whose server or channel no longer exists. The founding screen also opens this manager when
the vault has no servers. The manager pauses automatic retry while open; an invocation
already dispatched may still finish. Every request offers **Recover to saved draft** and
**Stop retrying**, each with an explicit confirmation. Stop retrying removes the stored
request and its text; it cannot retract a message that may already have been accepted.

Only a fresh never-dispatched intent can carry `acceptance: not_accepted`. Before every
native invocation, the UI seals `acceptance: ambiguous` with the same token and payload.
Missing legacy markers, rejected/conflicting tokens, changed contexts and lost responses
remain ambiguous. Native error text is never evidence that a previous invocation failed
to commit. The manager warns that sending recovered text later may create a duplicate.

Explicit resolution prepares a replacement continuity snapshot, saves it through the
ordinary serialized queue, and only then removes the pending entry or installs a recovered
copy in memory. The entire pending collection stays occupied until that write succeeds.
Queued ordinary saves construct their snapshots at execution time, after preceding
decision commits, so an older captured snapshot cannot resurrect retired work. A resolution
temporarily holds submission and completion mutations. Every submission also checks its
original session and exact retained identity; late responses cannot recreate retired work
or clear a newer composer. Lock invalidates completion callbacks. A failed write retains
the original payload; an interrupted session asks the user to reopen the manager to check
what reached the sealed record. Leaving a conversation does not erase pending requests.

Orderly window close fences new continuity-save admissions, decisions and submission
continuations before waiting for the already-admitted save queue. That queue includes each
decision's synchronous `afterSave` memory commit. Only after it settles does close capture a
fresh continuity snapshot and invoke native close. Waiting only for native write ordering is
insufficient: a final snapshot captured while the old token remains in memory could overwrite
the successful decision. A failed decision write retains the original request in the final
snapshot; it is not reported as a successful cancellation or recovery.

If the queue has not settled within ten seconds of browser timer time, this close attempt
stops before native close and leaves the window open with a warning. It does not destroy or
restart the process. The active decision remains fenced until it settles; the user can close
again afterward. A UI-generation change also aborts that close attempt, preventing a snapshot
from a replacement session from being used under the original close request.

Ctrl+L still clears the view immediately. If it interrupts a decision before its memory
commit, generation validation rejects that decision instead of claiming cancellation succeeded.
The request remains unresolved and may retry on a later unlock if it was eligible beforehand;
the user must revisit the manager to complete the decision. A fresh close while already locked
reuses the lock coordinator's exact immutable snapshot. This preserves immediate privacy
redaction without adding a new stored retry state or promising that an interrupted decision
completed. Hard process termination and unacknowledged filesystem writes remain outside the
orderly-close guarantee.

Recovered drafts are a separate collection, never a retry queue, with the same independent
32-entry and 256 KiB payload limits. Full recovery storage does not prevent Stop retrying.
Saved copies retain their original text, reply target and source identity. Copy text, use
in an available conversation's draft, and remove are explicit actions. Using a copy retains
the recovered original and clears the old conversation's reply reference in the composer.
Text above the 32,768-character composer limit remains available to copy in full.
The native continuity record still has its combined 1 MiB limit, shared with ordinary
drafts and preferences. An envelope-size refusal is a failed save, not permission to evict
pending work or recovered text.

The scheduler grants no network or backend authority and does not claim peer delivery.
Accepted publication retries remain owned by the native actor. UI intents that have not
yet reached durable native acceptance require an unlocked, hydrated WebView. The pending
identity and payload must still cross the existing continuity barrier to survive a crash.

`pending-send-retry.test.ts` uses an injected fake clock and executes the production App
scheduling/submission functions with I/O replaced. It checks capped pacing, coalescing,
busy-pass exclusion, lock/session fencing, storage recovery, lost-acknowledgement token
reuse, permanent refusal persistence and the absence of authoring before a successful
continuity save. It does not model browser suspension or native filesystem durability.

`pending-manager.test.ts` executes the actual App save queue, hydration, decisions and
submission functions with native I/O replaced by deferred promises. It covers orphaned
blocked requests, capacity retirement after acknowledgement, a full recovery collection,
disk/aggregate-size failure, queued saves before and after resolution, lock/reopen,
pre-dispatch and in-flight IPC races, newer composer text and no implicit publication.
`pending-manager-surface.test.ts` mounts the compiled production component and drives its
confirmation buttons through those App functions. These tests verify the frontend contract;
the native sealed-store and generation checks remain responsible for filesystem durability.

R3 validation (2026-09-27): the seven focused send/continuity/manager files passed 47 tests;
the onboarding/file-trust file passed 15 tests, including the unchanged hydration-before-
onboarding ordering requirement. `svelte-check` reported zero errors and warnings. An
independent read-only implementation review examined the save queue, delayed IPC outcomes,
orphan handling and resource limits and found no blocker or high-severity issue. Full-App
Tauri account replacement and actual filesystem faults are not simulated by these UI tests.

R5 validation (2026-09-28): the cancellation and recovery tests in `pending-close.test.ts`
first failed against the preceding implementation because the actual registered
`onCloseRequested` callback dispatched a competing snapshot before the deferred decision save
finished. With the fence and queue drain, all ten close-specific tests and the surrounding
focused send/continuity suites passed (58 tests). The fixture executes that production callback,
save queue and decision functions with serialized native-write mocks; it covers successful and
failed saves, admitted trailing saves, denied new work, pre-dispatch continuations, timeout and
retry, immediate-lock interruption, and replacement UI generations. It is not a real native
filesystem or operating-system close test.
