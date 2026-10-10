# Durable chat submission

The desktop composer now seals a caller token, payload, conversation and original authoring
context before invoking native authoring. A retry preserves that exact request. The actor creates
one signed operation in a private preparation, then saves the operation, MLS state, retry receipt
and publication obligation together in the existing encrypted server snapshot. It installs the
operation into shared history only after the save succeeds. History export and gossip cannot see
a failed preparation. `accepted: true` from the new send command means local durable acceptance;
it makes no recipient-delivery or peer-custody claim.

The Ready/lease boundary acquires native persistence, session, vault and exact actor-incarnation
custody without waiting behind a blocked actor. Refusal drops Ready and resumes the actor. A
write failure retains the preparation and returns `accepted: false`, with the original message
identity available for retry. A lost command response is resolved by replaying the same token.
Successful tokens with changed payload, scope or original context are refused. Saved receipts
survive abrupt actor loss and a newly opened vault.

Uncommitted work under a changed MLS context remains recoverable but cannot automatically become
a new operation under different authority. The pending-message UI offers an explicit move back
to the draft. The user's subsequent Send creates a new request. Accepted historical operations
remain canonical history even when their original-epoch publication attempt becomes obsolete.

Publication uses the existing driver's `publish_once` acknowledgement. Only `Submitted` retires
the aggressive publication attempt; duplicate suppression and errors retain it. Retry deadlines
use the injected monotonic clock. Submission is not durable custody: ordinary history repair
continues independently, and no acknowledgement deletes canonical local history.

The continuity record also contains pending caller requests. Failed reads keep replacements and
sending disabled; failed writes prevent native invocation, including the retry button. Every
completion checks the UI session. A new draft with identical text has its own revision and is
not cleared by an earlier request's acknowledgement.

## Bounds and compatibility

- Existing snapshots without the extensions remain readable. The order is owner tenure, group
  policy, then durable chat. Unknown versions and partial frames fail closed.
- Each group retains at most 65,536 token receipts, without eviction, and 32 pending ciphertext
  preparations/publications. Capacity refusal is explicit. This currently limits a continuously
  disconnected group to 32 unsent durable publications until useful service resumes.
- The UI retains at most 32 unresolved caller requests and 256 KiB of serialized request state.
  Message text is bounded to 64 KiB; native reply identifiers to 256 bytes; sealed frames to 256 KiB.
- Legacy local callers without token fields still cross the store barrier, but cannot recover a
  lost IPC response by token. The shipped composer always supplies both fields.
- This change covers new chat sends. Edits, reactions and other authoring paths retain their
  existing persistence contracts. It does not create durable remote receipts or a multi-file
  admission transaction, and does not claim the complete CR02 acceptance matrix.

## Review and evidence

Independent review found and corrected obsolete-context channel reservation, retry after an
unsealed UI write, destructive fallback after failed continuity hydration, and identical new
draft cleanup. Publication timing uses monotonic time after review. Focused sync tests exercise
failed-write invisibility, exact retries, interrupted restore/publication, context changes and
capacity refusal. Actor tests use real encrypted stores and abrupt task loss. Desktop regression
tests execute the actual composer and continuity functions.

Commands and current results are recorded in the scope ledger. Raw local evidence is retained
under `logs/cr-durable-*`; physical-device delivery and crash-at-every-filesystem-barrier testing
are not established by these local fixtures.
