# Native lock snapshots and close acknowledgements

Status: implemented on the disabled path; focused validation and adversarial
review pass. Full current-source backend acceptance remains pending. Chat V2
remains disabled. This is local UI continuity, not text authoring, protocol
history custody or operational restart acceptance.

## Receipt, order and ownership

The trusted native main-window ACL router bounds ingress before copying final
UI JSON. It publishes owned DATA synchronously, before scheduling a command.
A short finite prelude covers receipt, original authenticated-mount capture and
publication. An unpolled or abandoned task therefore cannot hide the only copy
from another close. A descriptor retains the immutable authorized generation;
a later authentication, including on the same mount, cannot adopt it.

Three independently permitted payloads bound copied JSON to three 16MiB bodies.
Eight control invocations are permitted independently of those buffers. Ordinary
UI requests keep their separate four-invocation limit and paid Root IO pool.
These are local bounds; JSON/IPC input already allocated by Tauri and browser
copies are not included in Root's physical-IO invoice. There is no claim that
these limits bound the entire application or process.

One immutable ingress order distinguishes the newest candidate bytes from the
newest capture failure. An older IO completion cannot erase a newer loss. An
ordinary save's actual successful IO records a durable watermark before releasing
the shared lane, including when its response later becomes stale or there were
no pending bytes yet. That watermark rejects an older delayed snapshot copy.

A snapshot job owns the original Root preparation/output, its recorder and the
shared IO lane. Actual Root output destruction precedes outcome recording; the
lane is released last on normal completion, unpolled drop and panic. Exact
pending bytes remain until success or nonretryable schema rejection. IO failure
or interruption retains them for a later bounded retry on the same original
mount. Each caller performs at most three coalesced snapshot writes. No control
cell contains a Root facade, lease, source, SDK object or native commit guard.

## Lock, authentication and close

Lock remains unconditional even if capture, parsing, IO or task admission fails.
It revokes UI authorization before cleanup and does not report key retirement
from a mere queued job. Control-only Busy is separate from retained data/IO loss;
a later actual key drain may settle the former without erasing the latter.

Close holds a long outer fence through destruction or defer. It joins finite
producers, not other long close fences. Authentication joins all outer fences,
then the same IO lane before UI commit; it retries the original pending snapshot
before publishing authorization. Fresh secret verification is still required.
Foreign-target pending data cannot be retried against the newly mounted vault.

A nonretryable loss with no pending payload is reported on authentication. Only
a matching actual native response records that the warning was shown. A second
explicit authentication can acknowledge that exact loss. A suppressed, stale or
panicking responder cannot consume an unseen warning. Paid target metadata and
retired outcomes remain owned outside the UI commit guard.

Close similarly returns an opaque decimal revision for an actually attempted,
ended failure. The UI echoes only the revision it displayed. A stale revision,
new unattempted candidate, active IO, control/key failure or superseded request
cannot authorize destruction. A confirmed screen-state loss may be acknowledged
without granting any authority to a foreign mount. History/key cleanup cannot be
discarded. An overloaded close resolves deferred/non-discardable; bridge rejection
would incorrectly reach the frontend's emergency-destroy fallback.

## Limits and pending validation

This state is process-local: pending unsaved bytes do not survive a process crash.
A successful encrypted UI sidecar does survive normal reopen; this implementation
does not establish durable protocol history, operational cold source restore or
filesystem path identity against another process replacing directories.

All43 actual native text routing/lifecycle cases passed in99.86 seconds, including
15 snapshot regressions. These cover both slow-copy orderings, an older ordinary
writer after a newer durable save, an unpolled newer capture, a superseded close
after reauthentication, two simultaneous close fences, exact/stale discard
revisions, control overflow with retained IO failure, suppressed warning replies,
actual Job drop and post-write panic, lost observers while authentication waits,
an older admitted IO success after a newer capture loss, and genuine foreign
original-mount acknowledgement without writing either vault.

Full frontend3446, Svelte check (zero errors/warnings), web build, both workspace
formatting checks, ambient/raw-fs and exact-count guard self-tests passed.
Strict native all-target/all-feature Clippy, the separate native compile check,
both shipping dependency audits, actual Windows build, built-binary startup and
browser flows passed. The checked binary SHA256 is
8DD8F68980F873E5DA0DF5C8258FDC8599A70A3C4FCEC6B02122A504730A5D92.
Startup reused the existing dev server on1420; it establishes native setup
completion, not physical WebView/network acceptance of this disabled backend.
The independent flow gate starts its own real Svelte fixture server.

Read-only design and actual-diff re-reviews are clear after the ordering fixes.
Root strict Clippy was actually run and fails with582 errors in the unfinished
App pipeline; no suppression or passing sign-off is claimed. Complete Root and
native execution for the final source is still pending CI. Only the previously
named slow catch-up cost test is deferred until final acceptance. Production authoring,
joined membership/archive, transport/projection/admin and 20?50-peer acceptance
remain separate unfinished work.
