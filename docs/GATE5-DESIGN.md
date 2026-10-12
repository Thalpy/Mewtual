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
- Claims exist only during a call and disappear on disconnect and session change.
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
| C0d full identity | Missing. Details below. | Commits `56f276a`, `4f68892` |
| C0b shared admission | Missing. The admission mechanism lived inside `jam-budget.ts` and `jam-channel.ts`. | Commit `5c0d5e4` |
| C4a draw channel | Missing. | Claim frames only, `7815d2a` and `4f68892` |
| Studio claim UI | Only a local fixture table (`studio-store.ts`) and a placeholder sentence. | `4f68892` |

What was missing for C0d:

- `serve_call_signal` reduced the verified `DeviceId` to its 4-byte fingerprint before queuing.
- Outbound `peer_for_fingerprint` picked whichever colliding member `HashMap` iteration yielded.
- Queue fairness was keyed by the fingerprint string.
- The frontend keyed every call map by fingerprint.

## 3. Slices as built

| Commit | Slice | Content |
|---|---|---|
| `8ce97b3` | design | This document's first revision |
| `5c0d5e4` | C0b | `rt-admission.ts`: capability registry, token bucket, pre-parse gate with exhaustion auto-mute. Jam re-exports them; jam limits are pinned by a test. |
| `56f276a` | C0d backend | crypto/sync/app/actor/bridge full identity |
| `7815d2a` | claim protocol | `draw-claims.ts` codec, book, sender and Studio snapshot helpers |
| `4f68892` | C0d frontend + wiring | `call-identity.ts`, App call-state re-key, jam seed label, `draw` channel lifecycle, Studio claim UI |
| (this commit) | documents | Interfaces, threat model, handover, gate board, UI hooks, design notes |

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
  discarded and it sends none.
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
    call releases the claim.
  - The claim is refreshed every 30 s and re-sent to an edge that opens or whose connection
    recovers.
- **Ask.**
  - Available on a frame someone else claims. It is sent once and not refreshed (review L7).
  - Moving off the frame cancels it.
- **Pass.**
  - The holder sees a sticky "X asks for frame N · 52 s · pass" notice in the always-visible
    transport bar.
  - Pass releases the frame and suppresses `autoClaim` on it until the user selects another frame or
    claims explicitly.
  - The asker takes the frame only when an **explicit Release of a live claim** arrives from the
    holder while its own Ask is live and nobody else still claims the frame (review M4). The asker
    then sees "passed you the frame".
  - Expiry, a dropped channel, a reconnect, a session change, an auto-mute, or a peer withdrawing its
    own Ask is never a Pass.
- **Pacing.** The sender mirrors the receiver's claim bucket and coalesces a burst to its latest
  claim, sent on the next tick. Ordered SCTP can still deliver a stalled backlog in one burst, so
  this lowers rather than eliminates refusals, and the 30 s refresh repairs a dropped claim (review
  L1).

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
- **What clears a peer's entries at once** (review M3): the peer's connection leaving "connected"
  (failed, disconnected or closed), its `draw` channel closing, `removePeer` (bye, revocation,
  `closed`), or the end of the call. A link that recovers through ICE restart is restored by the
  sender's re-send.

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
- **`draw-claims.test.ts` (21 tests):**
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
  - Mutation checks during development: the expiry boundary, the replay comparison and the
    Pass-trigger condition each fail a test when inverted.
- **`call-identity.test.ts`:** sender parsing, prefix-twin revocation, targets, politeness, labels
  and `displayKey`.
- **`jam-engine.test.ts`:** an identity-keyed live peer seeds exactly as its fingerprint did, and
  twins get separate capabilities.
- **`call-wiring.test.ts`:** source pins for the App wiring (listener, sends, broadcasts, hangups,
  revocation, politeness, jam labels, draw channel lifecycle, scope, trim keys).
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

## 10. Decisions taken and still open

- **D1 (taken, please confirm).** The seven-key frame with Pass carrying no recipient.
- **D2 (taken).** The draw gate uses the jam `inst` limits so C4a stays compatible (review M2).
- **D3 (taken).** Jam seeds and take `parts` keep fingerprint labels (review M1). Takes and noise
  are unchanged.
- **D4 (open).** DM invites still address and deduplicate by fingerprint. Their resolver now refuses
  ambiguity, but a twin can still replace a victim's pending friend request. This is proposed as a
  later slice.
- **D5 (open, HIGH, outside Gate 5).** The fingerprint-keyed admin roster in the owner's admission
  check (review H3).
- **Open check.** Confirm on each shipping WebView (WebView2, WKWebView, WebKitGTK) that unsolicited
  data on SCTP stream 8 toward an old build is dropped without resetting the association (review
  L9). This cannot be verified in this container.

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
