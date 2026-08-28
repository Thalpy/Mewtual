# Message delivery states; design

Status: **built**, including an explicit authenticated delivery-receipt frame; see
[As built](#as-built) for what ships and which claims each state is allowed to make. The early
sections preserve the original no-new-wire exploration where it remains useful, but the final
implementation deliberately spends a small amount of member-visible timing metadata so a quiet
recipient can confirm delivery.

## What "delivered" means here

A message is a signed op in the channel's Automerge document. There is no server to acknowledge
it; "delivered to X" can only mean "X's member device attested that it holds the exact op". A
newly applied remote op therefore queues a connected-only, member-authenticated receipt bound to
its document and change hash. A later signed change whose dependency graph descends from the op is
also valid compatibility evidence. Neither path says the content was displayed or read.

## States (per own message)

- **saving**; the send has not yet been accepted by the local actor.
- **sent; awaiting confirmation**; the op is committed locally, but no other member has yet
  authored causal evidence that proves it holds the message.
- **held by n peers**; n other members either sent an authenticated receipt or authored causal
  descendant evidence. The UI does not compare that holder count with the separate live-route
  count: without the identities' intersection, an offline holder could stand in for a different,
  unconfirmed connected member.
- **queued; no proven member path**; the eclipse-adjacent case; pairs with the existing
  banner and status bar rather than inventing a new alarm.

**No "read" state.** Read receipts are surveillance-adjacent metadata and would need new
gossip; explicitly a non-goal (as are typing indicators, same reasoning). If ever added,
they must be opt-in-per-server and symmetric (you only see read marks if you publish them).

## Trust and honesty caveats

- A modified current member can send an attributable receipt without displaying the content or can
  withhold every receipt. The UI wording stays factual: "held by n peers", never "read".
- Confirmed-heads knowledge may reset across restarts depending on what sync state is
  persisted (9-series persisted whole-server sync state; verify granularity). If per-peer
  confirmed heads don't survive restart, old messages simply show no delivery line;
  degrade to silence, never to a false "sending".
- Only own messages get a state line; peers' messages show nothing (their delivery is not
  our business to display).

## UI (kept minimal)

A mono micro-line under one's **most recent** message only (matching the overhaul mock):
`◌ saving…` / `◌ sent · awaiting confirmation` / `✓ delivered · 3 peers` /
`⚠ queued; no proven member path`, in
muted/ok/warn colours respectively. Older own messages show the state on hover (title
attr) rather than a permanent line; density stays intact. The status bar's transfers
segment pattern is the styling reference.

## Historical phases (superseded evidence plan)

- **D1 (sync)**: expose a query in `catcoms-sync`; for (doc, change-hash), the set of
  peers whose confirmed heads include it; plus the claimed-route member count already known to
  the app layer. Read-only over existing state; confirm `their_heads` availability and
  restart semantics here.
- **D2 (actor/bridge)**: per-channel "delivery snapshot for my recent messages" in the app
  actor, pushed with a throttle (≤1 event/s per channel) via a `delivery-changed` event;
  message ids ↔ change hashes mapped where ops are authored.
- **D3 (UI)**: the micro-line + hover states, driven by the D2 event; no polling.

The implementation discovered that Mewtual does not run Automerge's sync protocol and therefore
has no `their_heads` session state. The no-new-wire D1 plan was replaced first by signed causal
evidence and then augmented with the explicit receipt below.

## As built

Neither D1 route survived contact: Mewtual does not run automerge's sync protocol at all. Ops are
sealed, signed and broadcast on a blinded gossip topic, and a lagging member pulls the whole signed
op log over request/response, so there is no per-peer sync session to interrogate and publishing
proves nothing about receipt.

The document still supplies compatibility evidence: a member counts as a holder when it authored a
change that causally descends from the message (`EncryptedDoc::holders_of`). The explicit kind-18
receipt closes the quiet-recipient gap. `ChannelSync` registers only the bounded recent hashes the
local device authored, queues a receipt only when a remote signed op is newly applied, sends it
only over an existing connection, and accepts it only as an authenticated current-member request
for an exact registered target. Unknown hashes cannot allocate confirmation state; duplicates are
inert. `peers_with_changes` returns the union of causal authors and accepted receipts, filtered to
the current roster. `delivered` is a positive lower bound for that roster, so it may fall after
membership changes; `0` means "no proof yet", never "failed".

### What each state is allowed to claim (`apps/desktop/src/delivery.ts`, pure + unit-tested)

| Verdict | Shown when |
|---|---|
| `pending` ◌ | the op has not been acknowledged locally yet; labelled `saving…` |
| `waiting` ◌ | sent, nobody has proved they hold it, and a live peer previously proved it could serve authenticated group catch-up; labelled `sent · awaiting confirmation`, never `sending` |
| `partial` ~ | at least one member proved it, but not all |
| `everyone` ✓✓ | every other member of the roster proved it |
| `queued` ✕ | nobody holds it **and** no live peer has previously served authenticated group catch-up |
| *(nothing)* | alone in the group, or no report exists for a message that is not the newest |

Three rules keep it honest, each of which was a real false alarm before it existed:

1. **Evidence outranks the network.** Any `delivered > 0` result can never be shown as a failure. A
   peer that confirmed and then dropped still holds the message; letting a connection flap repaint
   a delivered message red was the loudest version of this bug.
2. **Infrastructure and descriptor claims are not delivery paths by themselves.** `reachable`
   resolves live connections through each member's signed but self-asserted peer record; it is a
   diagnostic count and can be wrong until reciprocal transport ownership proof exists. It is not
   intersected with holder identities and therefore cannot authorize a positive "all reachable"
   claim. `any_peer` is stricter: the live peer must already have answered a request-bound,
   roster-verified catch-up. A bare relay/rendezvous socket or a member record merely claiming one
   therefore cannot suppress the queued warning. This deliberately undercounts during bootstrap
   rather than declaring an unproven transport able to carry group data.
3. **A missing measurement is not a measurement.** `delivered`/`reachable` are `null` until the
   actor reports on a message, which is distinct from reporting zero. The actor keeps only
   `MAX_TRACKED_OWN_MESSAGES` (50) per channel, in memory, so after a restart older own messages
   have no record: those render **nothing**, rather than claiming to still be sending.

Each event/query is a complete bounded snapshot and replaces the frontend map. Within each row,
`delivered`, `reachable`, and `any_peer` are assigned exactly as the actor reports them. This removes
evicted rows and prevents a removed member's anonymous count from standing in for a new roster.

### UI, as built

The tick sits in the timestamp gutter of **every** own message the actor still has evidence for,
not only the newest, so the log can be read back up to see which messages landed. On header rows
(avatar + name) the gutter holds the avatar and the tick goes underneath it rather than following
the time inline after the name: a delivery state belongs in one column the eye can run down, not on
the left of some rows and the right of others. The spelled-out receipt line stays on the newest own
message only, which is the density argument from the original sketch.

Receipt state and recent authored targets are intentionally bounded and session-local. After a
restart an older own message may therefore have no report and renders no delivery claim. The frame
is delivery-only: no read cursor, foreground state, or human interaction is transmitted.
