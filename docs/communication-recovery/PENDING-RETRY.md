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
paused. Manual retry preserves the original identity; only the existing explicit
move-to-draft action can request a new send under a changed context.

The scheduler grants no network or backend authority and does not claim peer delivery.
Accepted publication retries remain owned by the native actor. UI intents that have not
yet reached durable native acceptance require an unlocked, hydrated WebView. The pending
identity and payload must still cross the existing continuity barrier to survive a crash.

`pending-send-retry.test.ts` uses an injected fake clock and executes the production App
scheduling/submission functions with I/O replaced. It checks capped pacing, coalescing,
busy-pass exclusion, lock/session fencing, storage recovery, lost-acknowledgement token
reuse, permanent refusal persistence and the absence of authoring before a successful
continuity save. It does not model browser suspension or native filesystem durability.
