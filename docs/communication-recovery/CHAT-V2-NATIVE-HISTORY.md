# Native own-history reader

Chat V2 remains disabled and incomplete. This increment connects the existing
Root saved-original reader to the real native `get_message_page` router. It is
not a complete group projection or a frontend activation change.

## Contract

The selected native route requires the actual main-window ACL, current unlocked
generation, original mounted facade and installed native row. It captures that
row's actual selection, incarnation and full account/entry cancellation before
scheduling. Renderer IDs never manufacture a Source or current authorization.
The request contains `server`, canonical decimal `channel`, nonzero 32-byte
hex `expectedContext`, decimal-string `start`, optional decimal-string
`endExclusive` and `limit` in 1..=8. Cursor strings preserve all u64 values in
JavaScript. Unknown fields, overlong input and noncanonical cursors are refused.

The response identifies `backend: "text-v2"` and
`scope: "own-saved-originals"`. Each item identifies its original action, signer,
actual channel, local ordinal, presentation time and semantic body. Pending,
submitted and held are local outbox observations; submitted is not remote
receipt or durability. A read neither signs nor admits an unsaved original.
Root may reconcile already-admitted interrupted local IO before reading.

Ordinals cover the entire member context. The route filters items by requested
channel but preserves Root's `next` and `end_exclusive`, including an empty
filtered page. Continue to that fixed exclusive end even if a page is empty.
Do not derive group completeness, unread counts or a cross-page status snapshot
from this response. Later saves appear in a new enumeration, not behind an
already fixed end.

## Ownership and final reply

A dedicated 16 MiB account permits two overlapping affine 8 MiB native response
credits, independently of the Root page's existing payment and UI/authentication
credits. The native route also shares the existing single in-flight admission.
Full payment precedes copies or Root work. One whole worker owns preparation,
negative scope and payment; one whole output owns the actual paid page, serialized
body and payment last. Root restores the runtime before native serialization or
any subsequent UI guard acquisition.

The writer preallocates a fixed 4 MiB byte vector and refuses growth beyond it.
Text is borrowed from the page, not cloned into a JSON value. Eight maximum-sized
texts fit even with sixfold JSON escaping. Remaining native credit covers the
bounded record wrappers and action-ID scratch. This is not an allocator, total
RSS, IPC or WebView memory bound.

After serialization the route acquires UI commit, Store and table in that order,
then repeats generation, original mounted facade, selection, incarnation,
context/channel, closing and full-scope checks through the actual resolver call.
The owning result stays outside these guards, including callback unwind. The
same final installed-object checks now protect founder and sender replies: a
still-live original parked outside the active table/Store is insufficient.

## Verification and remaining work

All 71 native-text cases passed (163.22 seconds), including nonempty foreign
facade and same-ID replacement, actual history roundtrips, fixed-end paging,
unrun admission, post-serialization lock, maximum escaping and actual responder
panic/lost-observer cases. All three Root response-credit cases passed. The
compiled registry inventory is 332 tests, with the same four explicit fixture
producers; the normal guard now expects 328. Static gates and 3,446 inherited
frontend tests passed. Read-only diff review found no B/H/M.

The last review's two LOW coverage gaps are corrected: the held-output test now
finishes actual whole-close and acquires the real vault session before releasing
the stale reply; a test-only notice after actual page/body destruction checks
all native guards and the original live response credit during callback unwind.
These final four history regressions passed (16.54s), as did strict native lint
(1m37s) and explicit native check (2m13s). The first lint attempt caught one
needless borrow in that test, fixed without suppression. Full current suites
remain pending; inherited Root strict
Clippy debt is not resolved by this increment.

Frontend continuity/routing, warm listings, joined/Add sources, durable archive,
transport, complete projection, admin audit, cold operational restore and
20–50-member acceptance remain unfinished. The shipping switch stays off.
