# Native paged history

Status: design + implementation on branch `perf-native-paging` (2026-09-03). Follows the
history-scaling work on `perf-history-scaling` (`cf608ba`), which made one materialization of a
channel cheap and cached it per document version. What remained was that the webview still fetched
and held the **entire** channel on every open and every arrival, and eight deriveds walked it.

## Goal

The webview holds only the rows it shows (plus a bounded margin). Every question the old code
answered by scanning the whole array is either answered natively against the cached list, or is
answered over the loaded slice where that is provably enough. Exact semantics are preserved for:
unread divider and count, read-mark advance, reply quotes and reply counts, pins, search jumps,
inbox/ticker jumps, delivery ticks, day dividers, message grouping, optimistic send.

## Bridge contract (INTERFACES § 10 addition)

`get_message_page(server, channel, anchor, before, after, unread?) -> MessagePage`

- `anchor`: `{kind:"tail"}` | `{kind:"id", id}` | `{kind:"index", index}` | `{kind:"first_reply_to", id}`.
  Ids are the durable anchor: they survive concurrent inserts, edits and deletes around them, which
  index arithmetic does not (a remote row can merge *above* a local tail). `index` is the fallback
  for legacy id-less rows and for an anchor that was deleted; `tail` is where an open starts.
- `before` / `after`: rows either side of the anchor, each clamped to 2048 at the bridge.
- `unread`: optional `{divider_ts, now_ms}`. The client's frozen divider (its read mark at open,
  or "everything read") and its own clock.
- Result: `{version, total, start, anchor_index, rows, unread}`.
  - `version` is `Server::doc_version` (ops applied); two pages at one version are slices of one
    list and can be stitched by index. `total` is the whole channel. `start` is the position of
    `rows[0]`. `anchor_index` is `null` when an id anchor names no current row; `rows` is then empty
    and the caller re-anchors (by index, or tail).
  - Each row is the `get_messages` row plus `targets_me` (mention or reply-to-mine, parent resolved
    against the whole channel), `reply_count` (whole channel) and `reply_to_preview`
    (`{id, author, text[..200 chars]}` of the parent, or `null`).
  - `unread` (when probed): `{ceiling_ts, first_index, count}` by exactly the desktop's rule:
    ceiling = newest timestamp within five minutes of `now_ms`; every timestamp is clamped to it; a
    row counts when its clamped timestamp is past `divider_ts` and somebody else wrote it.

`get_pinned_messages(server, channel)` returns every pinned row (pins are few and curated).

`get_messages` stays for the explicit, user-driven whole-history readers (server-wide search
corpus, moderation timeline). `get_message_tail` stays for the arrival ticker.

## Webview model

- `messages` is the **loaded slice**, `pageStart` its position, `pageTotal` the channel size,
  `pageVersion` the version it was read at. The DOM shows the whole loaded slice; the old
  loaded-vs-mounted window collapses into one.
- Open: tail page, `before = 320 - 1`.
- Scroll to the top edge / "Load older": page anchored at the first loaded id, `before = 200`,
  prepended; scroll position preserved as before. "Load newer" symmetric at the last loaded id.
- Channel changed (`channel-updated`): if the reader is at the tail, re-read the tail with
  `before = max(320, loaded) - 1`; otherwise re-read anchored at the first loaded id with
  `after = loaded - 1 + 200` so rows that merged inside the slice are picked up. A vanished anchor
  falls back to `index = pageStart`.
- Jump to an id (reply quote, pinned panel, inbox, ticker, search hit): if loaded, scroll; else
  page around `{id}` (or `{index}` for a legacy hit) and replace the slice, exactly the old
  `windowAround` behaviour but fetched instead of sliced.
- Unread: `firstUnreadIdx`, `unreadCount` and the clock ceiling come from the page's `unread`
  summary (whole-channel exact) rather than from a scan of the array. The divider renders when
  `first_index` falls inside the loaded slice.
- Read-mark advance runs only when the log is observed at the bottom, so the tail is loaded and
  "newest row from somebody else" over the loaded slice equals the whole-channel answer.
- `activeChannelHasUnseen` positions by the mark's id when loaded; the timestamp fallback over the
  loaded slice is unchanged.
- Reply quote: the loaded parent if present, else the row's `reply_to_preview`. Reply counts from
  the row. "N replies" jump uses the `first_reply_to` anchor when the first reply is not loaded.
- Pins: `get_pinned_messages`.
- Search: the active channel is loaded into the corpus the same way other channels are (a full
  read on an explicit search), so a hit index is an absolute index and the jump pages around it.
- Optimistic send: the pending row is appended only when the tail is loaded; the acknowledgement
  refresh replaces it. Away from the tail the send simply refreshes.
- Delivery ticks: `lastOwnIdx` over the loaded slice (own recent messages are tracked natively
  only for the last 50 anyway); indexes stay absolute (`pageStart + i`).

## What changes for the reader and what does not

Nothing visible changes at the tail. Scrolling up now fetches instead of revealing already-held
rows, which costs one bounded IPC per page instead of one whole-history IPC per open. Search
still loads whole histories, on purpose, when a person opens it.

## Residuals

- A jump to a legacy id-less row from a search hit uses its index; if the channel changed between
  the search read and the jump the index may land one row off. Same as before.
- The unread summary is measured against the client's clock; the client passes `now_ms` so the
  rule is identical to `unread.ts`, and both keep the same five-minute grace constant. The two
  constants must move together (`CLOCK_SKEW_GRACE_MS` in both).
- Whole-history search on the UI thread is unchanged (queued: worker over paged data).
