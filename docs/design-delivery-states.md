# Message delivery states; design

Status: **built**, with the evidence source changed; see [As built](#as-built) for what actually
ships and which claims each state is allowed to make. The sections below are the original scoping
of what "delivered" can honestly mean in a CRDT-gossip system and how to surface it without new
wire messages or new metadata leakage; they remain accurate about intent.

## What "delivered" means here

A message is a signed op in the channel's automerge doc. There is no server to ack it;
"delivered to X" can only mean "X's device provably holds the op". The sync protocol
already gives us that for free: automerge sync state tracks, per peer and per doc, the
heads the remote has confirmed (`their_heads`; the exact field/API to be confirmed against
`catcoms-sync`'s per-peer records at implementation time). A message op is **delivered to
peer X** when X's confirmed heads causally include the op's change hash.

So delivery is **derived from existing sync bookkeeping**; no receipts on the wire, no
new doc, no new metadata anyone else can observe. A member who is offline learns nothing
new about you; a member who is online was already exchanging sync heads with you.

## States (per own message)

- **sending**; op committed locally; no connected peer's heads include it yet.
- **delivered n/m**; n of the m *currently reachable* members' heads include it. This is
  deliberately phrased against reachable peers, not the full roster: "delivered 2/2" with
  four members offline is the honest statement (the offline two will catch up via normal
  sync/snapshot, and the count rises if they reconnect during the session).
- **queued; no peers reachable**; the eclipse-adjacent case; pairs with the existing
  banner and status bar rather than inventing a new alarm.

**No "read" state.** Read receipts are surveillance-adjacent metadata and would need new
gossip; explicitly a non-goal (as are typing indicators, same reasoning). If ever added,
they must be opt-in-per-server and symmetric (you only see read marks if you publish them).

## Trust and honesty caveats

- A peer can misreport sync heads; delivery claims are as policy-trusted as everything
  else at this layer (attributable, not enforced). The UI wording stays factual: "held by
  n peers", never "read".
- Confirmed-heads knowledge may reset across restarts depending on what sync state is
  persisted (9-series persisted whole-server sync state; verify granularity). If per-peer
  confirmed heads don't survive restart, old messages simply show no delivery line;
  degrade to silence, never to a false "sending".
- Only own messages get a state line; peers' messages show nothing (their delivery is not
  our business to display).

## UI (kept minimal)

A mono micro-line under one's **most recent** message only (matching the overhaul mock):
`◌ sending…` / `✓ delivered · 3 peers` / `⚠ queued; no peers reachable`, in
muted/ok/warn colours respectively. Older own messages show the state on hover (title
attr) rather than a permanent line; density stays intact. The status bar's transfers
segment pattern is the styling reference.

## Phases

- **D1 (sync)**: expose a query in `catcoms-sync`; for (doc, change-hash), the set of
  peers whose confirmed heads include it; plus "reachable member count" already known to
  the app layer. Read-only over existing state; confirm `their_heads` availability and
  restart semantics here.
- **D2 (actor/bridge)**: per-channel "delivery snapshot for my recent messages" in the app
  actor, pushed with a throttle (≤1 event/s per channel) via a `delivery-changed` event;
  message ids ↔ change hashes mapped where ops are authored.
- **D3 (UI)**: the micro-line + hover states, driven by the D2 event; no polling.

D1 is the only phase with real unknowns (automerge sync-state API surface); if
`their_heads` turns out not to be exposed per peer, the fallback is counting peers whose
*outgoing* sync for that doc reports nothing pending; same honesty, coarser grain.

## As built

Neither D1 route survived contact: Mewtual does not run automerge's sync protocol at all. Ops are
sealed, signed and broadcast on a blinded gossip topic, and a lagging member pulls the whole signed
op log over request/response, so there is no per-peer sync session to interrogate and publishing
proves nothing about receipt.

What is used instead is the document itself: a member counts as a holder when it **authored a
change that causally descends** from the message (`ChannelSync::peers_with_changes` →
`EncryptedDoc::holders_of`). That is the design's own predicate, backed by the peer's signature
rather than its self-report. It is strictly one-sided, and this is the single most important thing
about the feature: **a member that received a message and has not written since is
indistinguishable from one that never got it.** A quiet reader produces no confirmation, ever.
`delivered` is therefore a lower bound that only rises, and `0` means "no proof yet", never
"failed".

### What each state is allowed to claim (`apps/desktop/src/delivery.ts`, pure + unit-tested)

| Verdict | Shown when |
|---|---|
| `pending` ◌ | the op has not been acknowledged locally yet |
| `waiting` ◌ | sent, nobody has proved they hold it, and the node is connected to something |
| `partial` ~ | at least one member proved it, but not all |
| `reachable` ✓ | every currently-reachable member proved it |
| `everyone` ✓✓ | every other member of the roster proved it |
| `queued` ✕ | nobody holds it **and** the node has no transport peer connected at all |
| *(nothing)* | alone in the group, or no report exists for a message that is not the newest |

Three rules keep it honest, each of which was a real false alarm before it existed:

1. **Evidence outranks the network.** Any `delivered > 0` result can never be shown as a failure. A
   peer that confirmed and then dropped still holds the message; letting a connection flap repaint
   a delivered message red was the loudest version of this bug.
2. **Red rests on `any_peer`, not on `reachable`.** `reachable` resolves live connections to member
   fingerprints through signed peer records, so it reads zero whenever a record has not arrived yet
   while ops gossip out perfectly well. `DeliveryState::any_peer` (from
   `ChannelSync::has_connected_peer`) is the accurate liveness signal and is the only basis for ✕.
3. **A missing measurement is not a measurement.** `delivered`/`reachable` are `null` until the
   actor reports on a message, which is distinct from reporting zero. The actor keeps only
   `MAX_TRACKED_OWN_MESSAGES` (50) per channel, in memory, so after a restart older own messages
   have no record: those render **nothing**, rather than claiming to still be sending.

Reports are merged, never assigned (`mergeDelivery`): `delivered` takes the max so a later report
that happens to see fewer holders cannot unprove what was already proved; `reachable`/`any_peer`
are live and are taken as reported.

### UI, as built

The tick sits in the timestamp gutter of **every** own message the actor still has evidence for,
not only the newest, so the log can be read back up to see which messages landed. On header rows
(avatar + name) the gutter holds the avatar and the tick goes underneath it rather than following
the time inline after the name: a delivery state belongs in one column the eye can run down, not on
the left of some rows and the right of others. The spelled-out receipt line stays on the newest own
message only, which is the density argument from the original sketch.
