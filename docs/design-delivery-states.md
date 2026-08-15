# Message delivery states — design

Status: **proposed.** UI groundwork exists (the overhauled log has room for a per-message
mono delivery line; the status bar already reports node/peer state). This doc scopes what
"delivered" can honestly mean in a CRDT-gossip system and how to surface it without new
wire messages or new metadata leakage.

## What "delivered" means here

A message is a signed op in the channel's automerge doc. There is no server to ack it —
"delivered to X" can only mean "X's device provably holds the op". The sync protocol
already gives us that for free: automerge sync state tracks, per peer and per doc, the
heads the remote has confirmed (`their_heads` — the exact field/API to be confirmed against
`catcoms-sync`'s per-peer records at implementation time). A message op is **delivered to
peer X** when X's confirmed heads causally include the op's change hash.

So delivery is **derived from existing sync bookkeeping** — no receipts on the wire, no
new doc, no new metadata anyone else can observe. A member who is offline learns nothing
new about you; a member who is online was already exchanging sync heads with you.

## States (per own message)

- **sending** — op committed locally; no connected peer's heads include it yet.
- **delivered n/m** — n of the m *currently reachable* members' heads include it. This is
  deliberately phrased against reachable peers, not the full roster: "delivered 2/2" with
  four members offline is the honest statement (the offline two will catch up via normal
  sync/snapshot, and the count rises if they reconnect during the session).
- **queued — no peers reachable** — the eclipse-adjacent case; pairs with the existing
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
  confirmed heads don't survive restart, old messages simply show no delivery line —
  degrade to silence, never to a false "sending".
- Only own messages get a state line; peers' messages show nothing (their delivery is not
  our business to display).

## UI (kept minimal)

A mono micro-line under one's **most recent** message only (matching the overhaul mock):
`◌ sending…` / `✓ delivered · 3 peers` / `⚠ queued — no peers reachable`, in
muted/ok/warn colours respectively. Older own messages show the state on hover (title
attr) rather than a permanent line — density stays intact. The status bar's transfers
segment pattern is the styling reference.

## Phases

- **D1 (sync)**: expose a query in `catcoms-sync` — for (doc, change-hash), the set of
  peers whose confirmed heads include it — plus "reachable member count" already known to
  the app layer. Read-only over existing state; confirm `their_heads` availability and
  restart semantics here.
- **D2 (actor/bridge)**: per-channel "delivery snapshot for my recent messages" in the app
  actor, pushed with a throttle (≤1 event/s per channel) via a `delivery-changed` event;
  message ids ↔ change hashes mapped where ops are authored.
- **D3 (UI)**: the micro-line + hover states, driven by the D2 event; no polling.

D1 is the only phase with real unknowns (automerge sync-state API surface); if
`their_heads` turns out not to be exposed per peer, the fallback is counting peers whose
*outgoing* sync for that doc reports nothing pending — same honesty, coarser grain.
