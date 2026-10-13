# Gate 5 design and status: collaborative Flipnote frame claims

Status (2026-10-12): **implementation candidate on `gate5-frame-claims`; not accepted.**

- Base: `e8aad1acb7ccc43f5e8757bcffbcd83997f7c231` (`origin/gate4-agent1-runtime`, tree-identical to
  `Create-suite-2` at `ff8e276`).
- The early adversarial design review (section 11) found no blocker and three HIGH findings. H1 and
  H2 are fixed in the implementation; H3 is a pre-existing admission defect outside Gate 5,
  escalated in THREAT-MODEL hardening item 7.
- Gate 5 acceptance still needs the evidence in section 9, a review of the final diff, and the
  user's review.

Authoritative inputs: `BACKEND-IMPLEMENTATION.md` (gate 5 row), `design-creative-suite.md` sections
2.3, 2.7, 2.10 and 2.18, `INTERFACES.md` sections 12 and 13, `THREAT-MODEL.md` and
`design-voice.md`.

## 1. Scope

In scope, from the user's Gate 5 brief:

- Carry the authenticated full 64-hex device identity through `CallSignal`, the bridge and the
  frontend call state (design 2.18, slice C0d).
- Use the ordered `draw` data channel (id 8) with the existing bounded admission model. That model
  is extracted unchanged into `rt-admission.ts` (slice C0b).
- Implement the exact capability/session-bound frame-claim message `t:"c"` (design 2.7 and 2.10).
- Claim, Ask and Pass; receiver-observed 90-second expiry; 30-second refresh; a visible countdown.
- Sequence numbers bound to the authenticated channel capability and the sender session.
- Claims are advisory: never authorization, never an edit lock.
- Claims exist only during a call. They are hidden while a peer is unreachable, and dropped when
  its channel closes or its session changes.
- Tests on deterministic clock seams: two-member observation, collision, replay, malformed input,
  identity prefix collision, disconnect, expiry, stale-session delivery and Pass.

Out of scope:

- Standalone Draw/Pictochat: strokes, replay requests, `pv` presence, the Draw tab.
- Games, avatar consent and the `play` channel.
- Flipnote sound, linked scores, `.pixa` and GIF export.

No native Studio command, persistence format, P1 path or Gate 4 lifecycle/recovery/registration
boundary changes.

## 2. Prerequisite audit (at the base commit)

| Prerequisite | State at base | Gate 5 work |
|---|---|---|
| C0d full identity | Missing. Details below. | Commits `cdba0b3`, `1ebabdc` |
| C0b shared admission | Missing. The admission mechanism lived inside `jam-budget.ts` and `jam-channel.ts`. | Commit `136dcdc` |
| C4a draw channel | Missing. | Claim frames only, `a324474` and `1ebabdc` |
| Studio claim UI | Only a local fixture table (`studio-store.ts`) and a placeholder sentence. | `1ebabdc` |

What was missing for C0d:

- `serve_call_signal` reduced the verified `DeviceId` to its 4-byte fingerprint before queuing.
- Outbound `peer_for_fingerprint` picked whichever colliding member `HashMap` iteration yielded.
- Queue fairness was keyed by the fingerprint string.
- The frontend keyed every call map by fingerprint.

## 3. Slices as built

| Commit | Slice | Content |
|---|---|---|
| `966b4db` | design | This document's first revision |
| `136dcdc` | C0b | `rt-admission.ts`: capability registry, token bucket, pre-parse gate with exhaustion auto-mute. Jam re-exports them; jam limits are pinned by a test. |
| `cdba0b3` | C0d backend | crypto/sync/app/actor/bridge full identity |
| `a324474` | claim protocol | `draw-claims.ts` codec, book, sender and Studio snapshot helpers |
| `1ebabdc` | C0d frontend + wiring | `call-identity.ts`, App call-state re-key, jam seed label, `draw` channel lifecycle, Studio claim UI |
| `017f54d` | documents | Interfaces, threat model, gate board, UI hooks, design notes |
| `0f41172` | review fixes | Context menu keying, snapshot gating, selection/Pass, auto-mute diagnostic, rotation |
| `7ad6f34` | re-review fix | An expired claim never outlives the view |
| `9c5510c`, `086f854`, `32e37c3` | records | Commit map, HANDOVER verification record |
| `f178666` | PR #43 review #1, #3 | Per-edge delivery records; an unreachable peer is hidden, its deliveries held and applied on recovery |
| (this round) | PR #43 review #2 | `draw-wiring.ts` shared by the App and `scripts/claims-webrtc-check.mjs`; real-WebRTC evidence; acceptance records |

## 4. Identity contract (C0d)

### 4.1 Backend

**`DeviceId::from_canonical_hex`**
- Accepts exactly 64 lowercase hex characters, the `Display` form and `MemberView.identity`.
- Refuses uppercase, a `0x` prefix, whitespace, the 8-hex fingerprint and any other length.

**`ChannelSync`**
- Queues `(DeviceId, payload)`, and `take_call_signals` returns full ids.
- `bound_call_signals` keeps one fair share per `DeviceId`. Ties evict the lowest id in byte order,
  which is the old lowest-fingerprint order wherever fingerprints differ.
- `send_call_signal(&DeviceId)` routes through `peer_for_device`. That requires the target to be a
  current member and uses the signed record stored under exactly that id.
- `peer_for_fingerprint` (DM invites only) refuses whenever more than one current member carries
  the fingerprint. The rule is the pure `unique_member_for_fingerprint`.
- Residual: `peer_for_device` inherits the documented peer-id squatting caveat of member records.
  "Never routes to a prefix twin" means never by identity confusion. It does not mean a member's
  own record cannot claim another member's transport.

**Actor**
- `AppEvent::CallSignal { from, from_fp, payload }`, both derived from the one verified id.
- `ServerActor::send_call_signal(String)` parses strictly and returns `Err` before queuing.

**Bridge**
- The `call-signal` event carries `from`, `from_fp` and `payload`.
- `send_call_signal(server, target, payload, trace)` refuses a malformed target as
  `VOICE_SIGNAL_FAILED`.

The `KIND_CALL_SIGNAL` network wire is unchanged.

### 4.2 Frontend

**Inbound and outbound signals**
- `parseCallSignalSender` requires `from` to be 64 lowercase hex and `from_fp` to be its first
  8 characters. Otherwise the signal is dropped and diagnostic `VOICE.SIGNAL.BAD_SENDER` is
  recorded.
- Self-addressed signals are ignored.
- Sends, room broadcasts (`onlineCallTargets`) and hangups (`rosterIdentities`, `hangupTargets`)
  address full identities.
- Presence is still reported per fingerprint, so an online twin makes the member it collides with
  look online too. That widens addressing only; it never misroutes a signal.

**Call state keyed by full identity**
- Peer connections and states, peer meta, ICE buffers, streams, audio routing, transport badges,
  presence and speaking.
- Focus/solo, jam capabilities/decoders/budgets/queues/mutes, juke DJ attribution, recording
  consent, and all draw/claim state.
- The self key is `callSelfId`.

**Roster revocation (review H2)**
- `revokedCallPeers` compares full identities. A removed device whose prefix twin is still a member
  loses its edge, jam lane and claims.

**Politeness**
- `isPoliteTowards(self, peer)` compares identities. The order matches an older build's fingerprint
  comparison whenever fingerprints differ.

**Display and local preferences**
- `nameOf`, `profileFor`, `callProfileFor`, `callNameOf`, `instColor`, the per-peer
  voice/share/instrument trims and console logs resolve an identity through `displayKey` (its
  fingerprint).
- Saved trims therefore survive.
- `identityLabel` appends a longer id slice and "shares a fingerprint" whenever two roster members
  share one (review M5).

**Jam (review M1)**
- The engine's `openSource(identity, seedLabel)` keys the capability, sequencing and voice ownership
  by identity.
- Drum seed material and `jam-take:v1` `parts` keep the published fingerprint label
  (`callJamLabel`). Mixed-build rooms therefore render identical noise, and the take format is
  unchanged.
- Two call participants sharing a fingerprint produce a duplicate take label. The recorder refuses
  that (it pauses) rather than merging lanes.

## 5. Draw channel transport

- `pc.createDataChannel("draw", { negotiated: true, id: 8, ordered: true })` is created in
  `createPeer` before the first offer. An older build has no channel 8; frames sent to it are
  discarded and it sends none. Section 9.3 records that in Chromium.
- **Wiring.** `draw-wiring.ts` (`openDrawEdge`, `drawConnectionState`) binds the channel and the
  peer connection's state to the session. `App.svelte` and the real-WebRTC check both run exactly
  this code.
- **Capability.** The claim session mints it synchronously in `createPeer` (review L2).
  - Callbacks close over the capability and the call's session.
  - `onopen` checks that the peer generation is still current.
  - `onclose` retires the capability and drops its entries.
- **Admission.** Every delivery is charged against the sender identity's call-epoch gate before any
  content-dependent work.
  - The gate's limits are the jam `inst` numbers: 1024-byte pre-parse cap, 80/s burst 160, and
    auto-mute after 10 s exhausted inside 60 s (review M2). A later C4a peer's legitimate stroke
    traffic therefore cannot auto-mute a Gate 5 receiver.
  - Before `JSON.parse`, every draw frame must fit 200 bytes.
  - An auto-mute retracts that peer's claims.
- Only `t:"c"` is implemented. Other `t` values are charged and refused as `unsupported`.

## 6. Claim message and semantics

### 6.1 Wire

```
{"t":"c","sn":<16 lowercase hex, not all zero>,"q":<uint32, < 0xffffffff>,
 "obj":<32 lowercase hex>,"fr":<32 lowercase hex>,"on":0|1,"ask":0|1}
```

The key set is exact: all seven keys, no others, and key order is irrelevant. Frames are about
120 bytes.

| `on` | `ask` | Meaning |
|---|---|---|
| 1 | 0 | **Claim**, also the 30 s refresh |
| 0 | 1 | **Ask**: one-shot, not refreshed, lives 90 s at each receiver |
| 0 | 0 | **Release**: withdraws this sender's claim and ask on `(obj, fr)`. Pass is a Release. |
| 1 | 1 | Malformed |

Pass carries no recipient. A claim is self-asserted by its authenticated sender, so the asker
completes the hand-over by claiming (6.4).

### 6.2 Session and sequence binding

- The sender mints `sn` once per call join with the injected RNG (`crypto.getRandomValues` in the
  app) (review L8).
- One `q` counter covers every draw frame under that `sn`; a frame broadcast to several edges uses
  one `q`. On exhausting `q` the sender rotates to a fresh `sn` and re-sends its claim.
- **Receiver state per capability:**
  - The first valid frame binds the current `sn`.
  - Within that `sn`, `q` must strictly increase; otherwise the frame is refused as `replay`.
  - A new `sn` is a session change: it drops that capability's entries and retires the old `sn`.
  - A retired `sn` is refused as `stale-session`.
  - A fifth session on one capability is refused as `session-limit`.
- **Validation before commitment.** Every check (session, cap, rate) runs before anything is
  committed, so a refused frame changes nothing.

### 6.3 Receiver book

- Entries are keyed by `(sender identity, obj, fr)`.
  - A Claim replaces that sender's Ask on the frame, and vice versa.
  - A Release deletes both.
- **Expiry.** An entry is live while `now - seenMs < 90 000`, measured on the receiver's monotonic
  clock, so at exactly 90 s it is gone. No timestamps travel.
- **Bounds.**
  - At most 4 live claims plus asks per sender. Refreshing or releasing always works.
  - The claim bucket allows 2/s, burst 10, per identity per call. Releases are never charged
    (review L1).
- **Collisions.** All live claimants are shown, sorted by identity, so both delivery orders render
  the same. A frame with two or more claimants, counting this device, is "contested". Nothing is
  resolved automatically.
- **Independence.** Identities are compared in full. A prefix twin can never refresh, release,
  expire, session-reset or consume the sequence domain of another identity's entries.

### 6.4 Local Claim, Ask and Pass

- **Claim.**
  - Painting a frame calls `autoClaim`, and the "claim this frame" button claims explicitly.
  - Selecting another frame or object, leaving the Studio, viewing another server or leaving the
    call releases the claim. The Studio reports every selection change to the session (`select`),
    which withdraws a claim or ask for any other frame and lapses a Pass suppression for another
    frame.
  - Each edge is refreshed 30 s after its transport last took my claim. An edge that opens is
    told my claim and my ask. An edge that recovers is told only what changed while it was
    unreachable, because its receiver kept what it had.
- **Ask.**
  - Available on a frame someone else claims. Each edge is sent each Ask once, when that edge's
    transport takes it, and it is never refreshed (review L7). A record the edge took before this
    Ask was made belongs to an earlier Ask on the same frame and does not count.
  - Moving off the frame cancels it. Cancelling, a lapse at 90 s or claiming the frame sends a
    Release to every edge that took the Ask, and nothing to one that never did.
  - An edge that first takes the Ask after it was made (one that opened late, or whose transport
    refused it at first) counts down from its own receipt. The asker's lapse Release ends it there
    on time.
- **Pass.**
  - The holder sees a sticky "X asks for frame N · 52 s · pass" notice in the always-visible
    transport bar.
  - Pass releases the frame and suppresses `autoClaim` on it until the user selects another frame or
    claims explicitly. Coming back to that frame and painting claims it again.
  - The asker takes the frame only when an **explicit Release of a live claim** arrives from the
    holder while its own Ask is live and nobody else still claims the frame (review M4). Pass is the
    deliberate way to send that Release, but a holder moving to another frame or leaving the Studio
    sends one too, and that also hands the frame over. The asker sees "X let go of the frame you
    asked for; it's yours now". Two askers can both take it, which then shows as contested.
  - Expiry, a dropped channel, a reconnect, a session change, an auto-mute, or a peer withdrawing its
    own Ask is never a Pass.
- **Delivery (PR #43 review #1).** Each edge keeps a bounded record of what its transport *took*
  for each frame: `send` returned true and did not throw. A full WebRTC send buffer throws
  `OperationError`, and a closing channel refuses.
  - Every action, every 1 s tick and every recovery reconciles that record with what this device
    asserts.
  - A refused or throwing send changes nothing, so it is retried on that edge only.
  - An edge that took a frame is never sent it again, so receivers' countdowns do not restart.
  - A Release goes only to edges that took the claim or ask.
  - A record of something I no longer assert is kept until that edge takes its Release, however
    old. A frame the transport took may still be in flight after a long outage, and the Release
    must follow it; at a receiver that already expired the entry, the Release is a no-op. Records
    are bounded (`DRAW_TOLD_MAX`).
  - So an Ask or a Pass whose send failed still arrives once the transport recovers, before
    expiry.
- **Pacing.** I mirror each receiver's claim bucket per peer identity for the whole call, as the
  receiver keeps it, so a replaced channel does not start a fresh burst. A burst coalesces to the
  latest claim, and a send the transport refused is refunded. Ordered SCTP can still deliver a stalled backlog
  in one burst, so this lowers rather than eliminates refusals, and the 30 s refresh repairs a
  dropped claim (review L1).

### 6.5 Advisory only

- The claim module exposes no permission-shaped API.
- `studio-session.ts`, `studio-native.ts`, `studio-state.svelte.ts`, `studio-store.ts` and
  `studio-contract.ts` do not import it (source-hygiene test).
- `commit()` and `canEdit` in the Studio never consult claims (wiring test).
- Painting a claimed frame is allowed and shows a contested warning.
- Claims are honest-client coordination; THREAT-MODEL has the row.

### 6.6 Only during a call, only for the call's server (review H1)

- One `DrawClaimsSession` exists per call join. `leaveVoice` disposes it, and callbacks retained
  from an ended call hold a disposed object.
- The Studio receives claims, and its actions may assert anything, only while
  `inCall && activeServerId === callServer && view === "studio"`. Each asserting action re-checks
  that scope.
- Losing the scope releases my claim and cancels my ask.
- A Studio browsed on another server therefore sends nothing into the call, so object ids and edit
  timing cannot link the user's per-server identities.
- **An unreachable peer is absent** (review M3; PR #43 review #3). When the peer's connection
  leaves "connected" (disconnected or failed):
  - Its entries leave every view at once. They keep expiring underneath.
  - A delivery that still arrives (a delayed `onmessage`, or traffic after the peer recovered but
    before this side sees it) is held, not applied, and charged against the claim bucket at its
    arrival time when replayed. Holding is bounded (`DRAW_HOLD_MAX`): on overflow, a frame that a
    later held frame for the same (obj, fr) supersedes goes first, so a Release that is still the
    latest word on its frame is never shed while anything held is superseded; with nothing
    superseded, the oldest goes. A Pass followed by a re-claim of the same frame inside one
    overflowing hold reads as the holder still claiming it. Malformed
    frames are refused at once, and an auto-mute discards everything held.
  - Nothing is sent to that peer.
  - When this side sees "connected" again, the entries still within 90 s reappear and the held
    frames are applied in arrival order. A Pass sent during the gap therefore lands.
  - An unreachable claimant does not block a Pass to someone else.
- **What drops a peer's entries:** its `draw` channel closing, `removePeer` (bye, revocation,
  `closed`) or the end of the call.
- A link that never recovers stays suspended: the app answers "failed" with an ICE restart on the
  same connection. Its hidden entries keep expiring, and at most `DRAW_HOLD_MAX` frames wait,
  until the peer is removed or the call ends.

## 7. Studio surface

- **Timeline.** A dot on each frame with a live claim, in the claimant's colour. Contested frames
  get a warning ring, and the title text gives names and seconds left.
- **Transport bar.** One sticky notice per Ask on my frame, with countdown and Pass.
- **Frame card (Art tab).**
  - Contested warning.
  - "you are drawing this frame · stop".
  - Each claimant with seconds left, and each asker with seconds left.
  - "ask to draw it" or "you asked for it · n s · cancel", or "claim this frame".
  - A standing note that claims never lock a frame.
- **Outside the call's server.** "claims … appear while you are in a call on this server · they
  never lock a frame".
- **Countdowns** re-render on a 1 s tick driven by the same monotonic clock as the book.

## 8. Threat notes

THREAT-MODEL carries the authoritative text.

- **Identity.** Call authority keys are full verified identities, as listed in section 4. A
  member who grinds a key sharing a victim's four bytes cannot:
  - receive the victim's call signals, take its call edge or share its fairness bucket;
  - alias its jam/draw budgets, capabilities or claims;
  - shield a revoked device.
  Display names are still fingerprint-keyed app-wide; labels flag a shared fingerprint.
- **Replay.**
  - Draw frames ride DTLS between the two endpoints, and sequence/session checks reject duplicates,
    reorders and stale sessions.
  - Pre-existing residual: the call-signal transcript binds neither recipient nor a nonce cache, so
    a member can re-deliver another member's signed signal (including `bye`) to a third member
    inside the 60 s freshness window. That can briefly clear a rejoined peer's claims.
- **Floods.** Pre-parse cap and bucket, auto-mute with claim retraction, claim bucket, per-sender
  caps and the session limit.
- **Privacy.** A claim reveals which frame of which object a participant is drawing, and only to
  that call, and only for the call's own server.
- **Out-of-scope HIGH (review H3).** See THREAT-MODEL hardening item 7. The owner's inviter
  authorization (`inviter_is_authorized`) compares 4-byte fingerprints against its admin roster. A
  member who grinds a companion-device key sharing an admin's fingerprint (about 2^32 work) could
  pass the admission gate. Gate 5 does not change this; it needs a roster format decision from the
  user.

## 9. Tests and acceptance evidence

### 9.1 Frontend

All of these run on deterministic seams.

- **`rt-admission.test.ts`:** bucket monotonicity, charge-before-parse, UTF-8 cap, auto-mute and
  unmute, per-kind capabilities, and the pinned jam limits. The jam suite is unchanged.
- **`draw-claims.test.ts` (46 tests):**
  - Codec exactness, two-member observation with the 30 s refresh, and the 89 999 / 90 000 ms
    boundary.
  - Ask countdown, Pass hand-over and suppression.
  - Ask lapse. Expiry, disconnect and reconnect are not a Pass, and a withdrawn Ask is not a Pass.
  - Collisions in both orders.
  - Replay and reorder, session change, retired and limited sessions.
  - Stale capability, removal and disposal.
  - Prefix-twin independence.
  - Malformed input charged before parse, auto-mute retraction, caps with uncharged releases, sender
    pacing, sequence rotation.
  - The Studio snapshot, and the advisory-only source/API checks.
  - Delivery (PR #43 review #1), each on a two-way link whose transport can refuse, throw
    `OperationError` or queue:
    - a refused Ask is retried on that edge only, and an edge that took it is not re-sent it;
    - a throwing send is retried, and a refused Pass Release is retried until the asker takes the
      frame;
    - cancelling or lapsing an Ask no transport took sends nothing;
    - moving on while undelivered names only the new frame;
    - a replaced channel starts empty;
    - a refused send is refunded, and records are bounded.
  - Reachability (PR #43 review #3):
    - a delayed delivery while unreachable is held, then applied on recovery;
    - a Pass during the gap lands;
    - an unreachable claimant does not block a Pass;
    - close drops everything;
    - nothing is sent while unreachable;
    - holding is bounded, malformed frames are refused, and an auto-mute discards what is held.
  - Second review round: held frames charged at arrival (a recovery replay cannot refuse the
    newest Ask); a Release follows a frame taken before a long outage; pacing per peer survives a
    channel replacement; a held Release is not shed; overflow sheds superseded frames first, else
    the oldest; a closing channel's refusal is retried (`draw-wiring.test.ts`).
  - Mutation checks: the expiry boundary, the replay comparison and the Pass-trigger condition;
    then, for the PR #43 fixes, ignoring the send result, treating a throw as success, no refund,
    no hiding, no holding, sending while suspended, refreshing an Ask, no record bound, an
    unbounded hold, holding garbage, and a hidden claim blocking a Pass; then, for the second
    round, charging held frames at replay time, forgetting records after 90 s, a fresh bucket per
    edge, always shedding the oldest held frame, and no `readyState` check. Each fails a test.
    Removing *both* mute defences fails one; each alone is redundant by design.
- **`draw-wiring.test.ts`:** the channel parameters; retry after a refused or throwing send; `onopen`
  readies only a live edge; messages and close reach the session, and views rebuild only on change;
  connection states hide, hold and recover; and creation failure and an ended call.
- **`call-identity.test.ts`:** sender parsing, prefix-twin revocation, targets, politeness, labels
  and `displayKey`.
- **`jam-engine.test.ts`:** an identity-keyed live peer seeds exactly as its fingerprint did, and
  twins get separate capabilities.
- **`call-wiring.test.ts`:** source pins for the App wiring (listener, sends, broadcasts, hangups,
  revocation, politeness, jam labels, scope, trim keys). It also pins that `createPeer` uses
  `openDrawEdge` and `drawConnectionState` for this exact peer and session, with no second draw
  wiring.
- **`studio-surface.test.ts`:** compiled Studio claim-UI regressions (CLAIM-UI-001..003).

### 9.2 Rust

- **`catcoms-crypto`:** canonical-hex round trip and refusals.
- **`catcoms-sync`:**
  - The full-identity round trip.
  - A prefix-twin target gets no route.
  - Colliding senders keep separate fair shares.
  - The legacy resolver refuses ambiguity.
  - The existing call-signal tests, re-keyed.
- **`catcoms-app`:** the actor surfaces `from`/`from_fp` of one id and refuses fingerprint,
  uppercase and garbage targets.

The commands run and their results are recorded in HANDOVER's Gate 5 entry.

### 9.3 Real WebRTC (PR #43 review #2)

`scripts/claims-webrtc-check.mjs` runs three members, each its own headless Chromium 141 process,
the engine WebView2 ships. Each holds real `RTCPeerConnection`s with a fake microphone, `inst`
(id 7) and `draw` (id 8), and the shipped `draw-claims.ts` and `draw-wiring.ts`. Node relays SDP
and drives the pages over plain CDP. Time-dependent steps skew each member's injected clock while
the transport stays real.

Result: 34/34 checks, plus two notes (timing observations that cannot fail), in each of three
consecutive runs on 2026-10-13 after the second review round.

- **Claim, Ask and Pass** across data channels, with a notice. A contested frame is shown on both
  sides.
- **Refresh and expiry.**
  - Each receiver counts down on its own clock.
  - The 30 s refresh restarts the countdown.
  - A sender that stops refreshing expires at 90 s.
- **A genuine ICE disconnect and recovery.** One member's whole browser is frozen with SIGSTOP, and
  the other sees `connected → disconnected → connected` after about 6 s:
  - the claim is hidden at once;
  - the Pass the frozen member sends on waking lands;
  - which ordering occurs is timing, so it is a note, not a check. Both occurred across the runs:
    the Pass was held then applied (runs 2 and 3), and it arrived after recovery (run 1). The
    held path is asserted deterministically in `draw-claims.test.ts`.
- **Leave and rejoin.** Leaving drops (not merely hides) that member's claims at the other: its
  channel closes and the book is empty. A rejoined member's claims flow both ways.
- **Mixed versions.** An old build with no channel 8 connects to a current one, which sends claim
  frames on stream 8 toward it (engine stats: 23 messages, the channel stays `open`).
  - `inst` traffic still flows both ways (+20 each).
  - Voice packets keep arriving both ways.
  - The connection stays `connected`.
  - The old build sees no `ondatachannel` and no state change.
  - The current build's other edge is unaffected.

Not covered here: the Studio UI and the native backend (compiled Studio tests and the Rust
suites), and the App-level scope rules such as switching away from the call's server (`call-wiring`
pins and the scope `$effect`).

The repository's own `test:flows` gate, which drives the real Svelte app over CDP, also passes 9/9
on that Chromium (`EDGE_PATH` pointed at it, `--no-sandbox` for root).

## 10. Decisions taken and still open

- **D1 (taken, awaiting the owner's confirmation).** The seven-key frame with Pass carrying no
  recipient. What the owner is confirming:
  - Any explicit Release of a live claim hands the frame to an asker, so a holder moving to another
    frame or leaving the Studio satisfies an Ask as Pass does.
  - Two askers can both take a released frame, which then shows as contested.
  - A targeted Pass would need a wire change.
- **D2 (taken).** The draw gate uses the jam `inst` limits so C4a stays compatible (review M2).
- **D3 (taken).** Jam seeds and take `parts` keep fingerprint labels (review M1). Takes and noise
  are unchanged.
- **D4 (open).** DM invites still address and deduplicate by fingerprint. Their resolver now refuses
  ambiguity, but a twin can still replace a victim's pending friend request. This is proposed as a
  later slice.
- **D5 (open, HIGH, outside Gate 5).** The fingerprint-keyed admin roster in the owner's admission
  check (review H3).
- **Stream-8 compatibility per WebView** (review L9; PR #43 review #2):
  - **Chromium (WebView2's engine): verified** in section 9.3.
  - **WebKitGTK: the question does not arise on the stack tested.** Ubuntu 24.04's WebKitGTK
    2.52.6 does not expose `RTCPeerConnection` at all, whether by default, with `enable-webrtc`, or
    with the GStreamer WebRTC plugins installed. Neither the app nor wry 0.55 enables WebRTC in the
    Linux WebView, so the Linux desktop build has no calls of any kind there. That predates Gate 5.
    A Linux build that does get WebRTC needs this check run on it.
  - **WKWebView: unverified.** It needs a macOS run.
- **Two-desktop acceptance (PR #43 review #2): outstanding.** Two real desktop instances must run
  Claim, Ask, Pass, refresh and expiry, contested claims, continued editing of a claimed frame,
  disconnect and recovery, leave and rejoin, and switching away from the call's server through the
  Studio, with the owner's sign-off. Section 9.3 is engine-level evidence for the same paths, not a
  substitute.

## 11. Early design review response (2026-10-12)

A read-only adversarial reviewer inspected the design against the base code. It reported no
blocker.

| Finding | Resolution |
|---|---|
| H1: claims leak across servers | Fixed: scope rule 6.6 and wiring test |
| H2: revocation by fingerprint | Fixed: `revokedCallPeers` and test |
| H3: inviter check by fingerprint | Out of scope; THREAT-MODEL item 7; escalated (D5) |
| M1: drum-seed contract | Fixed: seed label (4.2), engine test |
| M2: draw rate vs C4a | Fixed: jam limits (5) |
| M3: disconnect wording | Fixed: connection-state, `onclose` and removal hooks (6.6) |
| M4: auto-claim trigger too broad | Fixed: explicit-release-only Pass, tests |
| M5: twin rendered as victim | Fixed: `identityLabel` in call and Studio labels |
| M6: integration evidence | Mitigated: identity helpers extracted and tested, wiring source pins, compiled Studio tests. A live two-client WebRTC run remains manual. |
| L1 | Fixed: releases uncharged, claim bucket per identity, auto-mute retracts; pacing wording corrected |
| L2 | Fixed: synchronous capability, `onopen` generation check |
| L3 | Self signals ignored in the frontend. `peer_for_device` does not refuse self, because no caller targets self. |
| L4 | Residual: presence stays per fingerprint (4.2) |
| L5 | Wording qualified (4.1) |
| L6 | The `studio-store.ts` fixture claim table is annotated as fixture-only, superseded by `draw-claims.ts` |
| L7 | Fixed: Asks are not re-sent to new edges |
| L8 | Fixed: injected RNG |
| L9 | Open check (10) |

Residuals the review listed as pre-existing are recorded in THREAT-MODEL:

- Signal replay of a stale `bye`.
- DM-invite deduplication by fingerprint.
- `voiceRooms`/`alertedRooms` keyed by unvalidated call ids.

## 12. Final diff review response (2026-10-12)

A read-only adversarial reviewer inspected `e8aad1a..017f54d` and reran the focused frontend
suites. It reported no blocker.

| Finding | Resolution |
|---|---|
| HIGH-1: the stage context menu was keyed by fingerprint, so its mute, volume and instrument actions silently did nothing | Fixed: `peerMenu` takes the call key, and only the profile card, the copied fingerprint and the head avatar use `displayKey`. Pinned in `call-wiring.test.ts`. |
| MEDIUM-1: every draw delivery and paint sample rebuilt the snapshot | Fixed. `rev` moves only on visible changes; refused frames, no-op releases and re-claims leave it alone. The app rebuilds only when `rev` moves or a countdown is live (`withDrawClaims`, `isQuiet`), and the Studio skips `autoClaim` for the frame already claimed. Covered by session tests, a mutation check and source pins. |
| L1: Pass suppression did not lapse on a selection change | Fixed: `selectionChanged`/`select`. Session test covers the review's probe sequence. |
| L2: "passed you" overstated intent | The toast now says the holder let go. Section 6.4 documents that moving on also hands over the frame, and that two askers become a collision. |
| L3: missing HANDOVER entry | Added after the re-review (N2). |
| L4: stale comments, labels and parameter names | Fixed, and the debug peer view reports fingerprint labels. |
| L5: silent draw auto-mute | Diagnostic `VOICE.DRAW.ABUSE_MUTED` added (no identity or content); session test. |
| L6: a rotation inside `flushAsk` desynchronised the Ask | Fixed: rotation marks the Ask unsent instead of dropping it. A test fails under the old behaviour. |

A short re-review of `017f54d..0f41172` confirmed those fixes and reported nothing blocking. It
found two more issues:

| Finding | Resolution |
|---|---|
| N1 (MEDIUM, new): a claim expiring inside a refused or no-op delivery left a "1 s" ghost in the view | Fixed: an expiry pruned in `receive` moves `rev`, and the 1 s tick also rebuilds while the last snapshot still shows anything. Regression test (no-op Release and claim-rate cases), mutation-checked. |
| N2 (LOW): the HANDOVER entry was claimed but missing | Added with the verification results. |

Leaving the claim scope also clears a Pass suppression, because `release()` runs on every scope
loss. That is intended: the user left.

Residuals carried forward (as amended by section 13):

- ~~A transient "disconnected" can hide a peer's claims for up to 30 s if only the receiver saw
  the blip.~~ Resolved in `f178666`: hidden entries reappear on recovery, and held frames are
  applied.
- The L9 WebView check: Chromium verified, WKWebView open, WebKitGTK has no WebRTC (section 10).
- A routing test against a *member* prefix twin is impractical: it needs a ground key.
- ~~There is no live two-client WebRTC run.~~ The engine-level run is in 9.3. The two-desktop
  acceptance run remains outstanding.

## 13. PR #43 review response (2026-10-13)

The review covered `ff8e276..32e37c3` and returned **request changes: Gate 5 is not complete**.
All three findings are addressed. Acceptance stays pending on the owner's two-desktop run and on
D1.

| Finding | Resolution |
|---|---|
| 1 (MEDIUM): a failed send silently discarded an Ask or Pass (`send` false or `OperationError`; `flushAsk` marked it sent; a failed Pass Release had nothing to retry) | Fixed in `f178666`: per-edge delivery records (6.4). The regressions requested: false, `OperationError`, recovery before expiry, cancel while pending, partial success across two edges, plus Pass retry, lapse while pending, replacement and bounds. All mutation-checked (9.1). |
| 2 (acceptance blocker): no real desktop claim path, no mixed-version check | Partly closed. The wiring moved to `draw-wiring.ts` (shared by the App and the check, behaviour-tested). Real-WebRTC evidence: 34/34 three times, including a genuine disconnect and recovery and the mixed-version check (9.3). On WebKitGTK the check is moot, because there is no WebRTC; WKWebView is unverified (10). **The two-desktop run with sign-off remains the owner's.** |
| 3 (lifecycle gap): disconnect cleared entries but a later delivery could repopulate them | Fixed in `f178666`: suspension hides and holds, and recovery restores and applies (6.6). It is tested deterministically with a delayed delivery during the interval, and exercised over real WebRTC, where the held ordering was observed in two of three runs (9.3). |
| Records: the Gate 4 ledger was stale | Fixed: GATE4-ACCEPTANCE records the merge of PR #27 (`e8aad1a` → `ff8e276`, 28/28 hosted checks) and supersedes "not merge-ready" and "Gate 5 remains closed". |
| Records: D1 still "please confirm" | Left with the owner, with the specific behaviours spelled out (10). |
| The inherited inviter HIGH | Unchanged and out of scope (D5, THREAT-MODEL item 7). Gate 5 acceptance is not whole-application security acceptance. |

## 14. Second review round (2026-10-13)

An independent adversarial review of `32e37c3..32e089b` found nothing blocking, one MEDIUM and four
LOWs. All are fixed, with regressions that fail without the fix.

| Finding | Resolution |
|---|---|
| MEDIUM-1: replaying held frames at recovery charged them all at one instant, so the newest Ask was refused (`claim-rate`) and, recorded as taken, never re-sent. This regression came from holding. | Held frames are charged at their arrival time. Nothing else charges a peer while its only channel is suspended, and the bucket ignores a backward step. |
| LOW-1: a record aged out 90 s after it was *taken* dropped the owed Release, but SCTP could deliver the claim after a long outage, leaving a 90 s ghost | Records of what I no longer assert are kept until the edge takes the Release (bounded). A late Release is a no-op at a receiver that already expired the entry. |
| LOW-2: a fresh sender bucket per edge, while the receiver's bucket lasts the call per identity | The sender bucket is per peer identity for the call. |
| LOW-3: shedding the oldest held frame could drop a Release | Overflow sheds superseded frames first. |
| Re-review LOW (new): with records kept, an earlier Ask's record on the same frame made a new Ask look delivered after an outage | An Ask is due unless the edge took it after that Ask was made (`Wanted.sinceMs`). Regression test, mutation-checked. The two wording nits (unconditional "never shed"; a Pass shed with its re-claim) are corrected in 6.6. The re-review found nothing blocking. |
| LOW-4: harness and documentation accuracy | Two logged rows are now notes, not checks (34 checks). The leave check asserts drop, not hide. "Failure closes the channel" is corrected (6.6). The record-ageing and pacing wording is corrected. The `readyState` refusal branch now has a test. |

