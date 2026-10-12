# Gate 5 design: collaborative Flipnote frame claims

Status (2026-10-12): **design proposed, early adversarial review requested; implementation in
progress on `gate5-frame-claims`.** Base: `e8aad1acb7ccc43f5e8757bcffbcd83997f7c231`
(`origin/gate4-agent1-runtime`, tree-identical to `Create-suite-2` at `ff8e276`). Nothing in this
document accepts Gate 5; acceptance needs the evidence listed in section 9 plus the user's review.

Authoritative inputs: `BACKEND-IMPLEMENTATION.md` (gate 5 row), `design-creative-suite.md`
sections 2.3, 2.7, 2.10 and 2.18, `INTERFACES.md` section 12 (the jam admission model this reuses),
`THREAT-MODEL.md` (jam and honest-client sections) and `design-voice.md` (call signalling).

## 1. Scope

In scope, from the user's Gate 5 brief:

- Carry the authenticated full 64-hex device identity through `CallSignal`, the bridge and the
  frontend call state (design 2.18, slice C0d).
- Use the ordered `draw` data channel (id 8) with the existing bounded admission model, extracted
  into `rt-admission.ts` without changing jam behaviour (slice C0b).
- Implement the exact capability/session-bound frame-claim message `t:"c"` (design 2.7/2.10).
- Claim, Ask and Pass; receiver-observed 90-second expiry; 30-second refresh; a visible countdown.
- Sequence numbers bound to the authenticated channel capability and the sender session.
- Claims are advisory: never authorization, never an edit lock.
- Claims exist only during a call and disappear on disconnect and on session change.
- Tests: two-member observation, collision, replay, malformed input, identity prefix collision,
  disconnect, expiry, stale-session delivery and Pass, all on deterministic clock seams.

Out of scope: standalone Draw/Pictochat (strokes, replay requests, `pv` presence, the Draw tab),
games, avatar consent and the `play` channel, Flipnote sound, linked scores, `.pixa` and GIF
export. Nothing here weakens a Gate 4 lifecycle, recovery or command-registration boundary: no
native Studio command, persistence format or P1 path changes.

## 2. Prerequisite audit (at the base commit)

| Prerequisite | State at base | Gate 5 work |
|---|---|---|
| C0d full identity | Missing. `serve_call_signal` reduces the verified `DeviceId` to `roles::fingerprint` (4 bytes) before queuing. Outbound `peer_for_fingerprint` picks whichever colliding member `HashMap` iteration yields. Queue fairness is keyed by the fingerprint string, so two colliding devices share one fairness bucket. The frontend keys every call map by the 8-hex fingerprint. | Slices 5A and 5B |
| C0b shared admission | Missing. The pre-parse cap, all-frame bucket, exhaustion auto-mute and channel capability live inside `jam-budget.ts` and `jam-channel.ts`. | Slice 5C |
| C4a draw channel | Missing. Only `inst` (id 7) is created. | Slice 5D, claim frames only |
| Studio claim UI | `studio-store.ts` holds a local fixture claim table (no wire); `Studio.svelte` shows only the sentence "claims ... arrive with the call draw channel". | Slice 5E |

## 3. Slices

Each slice gets focused tests. Slices land as separate commits so each one can be reviewed alone.

- **5A, C0d backend.** `catcoms-crypto`: a strict `DeviceId::from_canonical_hex`. `catcoms-sync`:
  queue and fairness keyed by `DeviceId`; outbound by full identity; the legacy fingerprint resolver
  refuses ambiguity. `catcoms-app` actor/`Server`: the event carries `from` and `from_fp`, and the
  command takes a `DeviceId`. Bridge: `call-signal` carries `from`, and `send_call_signal` takes
  `target`.
- **5B, C0d frontend.** `call-identity.ts` validates the inbound sender pair. App call state is
  re-keyed by the full identity. Display stays fingerprint/name based.
- **5C, C0b.** `rt-admission.ts` holds the token bucket, the pre-parse admission gate with
  exhaustion auto-mute, and the generic capability registry. Jam re-exports them, and the existing
  jam tests pass unchanged.
- **5D, draw transport and claim protocol.** `draw-claims.ts` holds the codec, the receiver claim
  book and the local claim sender, with an injected clock. It is wired as one negotiated `draw`
  channel per peer connection.
- **5E, Studio surface.** Claim chips on the timeline, a frame card with countdown and Ask, a sticky
  "asks for frame" notice with countdown and Pass, and an "outside a call" explanation.
- **5F, documents and acceptance.** Update INTERFACES, THREAT-MODEL, HANDOVER, the backend
  checklist, the UI hooks, design-voice, design-creative-suite and the tracker.

## 4. Identity contract (C0d)

### 4.1 Backend

- `DeviceId::from_canonical_hex(s)` accepts exactly 64 lowercase hex characters, which is the
  `Display` encoding `MemberView.identity` already uses. Anything else returns `None`, including
  uppercase, whitespace, a `0x` prefix or the wrong length.
- `ChannelSync::pending_call_signals: Vec<(DeviceId, Vec<u8>)>`. `serve_call_signal` queues the
  verified signer's `DeviceId` directly; the fingerprint is never computed on the receive path.
- `bound_call_signals` counts entries per `DeviceId`. Ties evict from the lowest `DeviceId`, by
  byte order, which preserves the old "lowest fingerprint" rule wherever fingerprints differ. Two
  devices that share four bytes now have separate fairness shares.
- `take_call_signals() -> Vec<(DeviceId, Vec<u8>)>`.
- `send_call_signal(target: &DeviceId, payload)` routes through a new `peer_for_device(target)`.
  That requires `group.contains_device(target)` and a signed peer record stored under exactly that
  `DeviceId`. A device that is not a member, or a member with no record, returns `Ok(false)`, which
  is the existing "no route" answer. A forged identity sharing a victim's four bytes can never route
  to the victim.
- `peer_for_fingerprint`, now used only by the DM-invite path, refuses (`None`) when more than one
  current member has the fingerprint. This is the "legacy route refuses ambiguity" rule from design
  2.18, applied to its last caller. Re-keying DM invites themselves is not part of Gate 5.
- `Server::take_call_signals` / `send_call_signal` mirror the sync types.
- Actor: `AppEvent::CallSignal { from: String, from_fp: String, payload }`. `from` is
  `DeviceId::to_string()`, the 64 lowercase hex, and `from_fp` is `roles::fingerprint` of the same
  id, kept for display only. `AppCommand::SendCallSignal { target: DeviceId, .. }`.
  `ServerActor::send_call_signal(target: String, ..)` parses with `from_canonical_hex` and returns
  `Err` on malformed input, before any command is queued.
- Bridge: `CallSignalEvt { server, from, from_fp, payload }` and
  `send_call_signal(server, target, payload, trace)`. A malformed `target` is a typed command error
  (`VOICE_SIGNAL_FAILED`), not a silent `false`.

The network wire is unchanged. `KIND_CALL_SIGNAL` already carries the signer's public key, and
`DeviceId` is derived from it on receipt. Mixed versions interoperate at the signalling layer. The
bridge and the actor ship in the same binary as the frontend, so their shape changes need no
compatibility shim.

### 4.2 Frontend

- `parseCallSignalSender({from, from_fp})` (in `call-identity.ts`) requires `from` to match
  `/^[0-9a-f]{64}$/` and `from_fp === from.slice(0, 8)`. Otherwise the signal is dropped with a
  diagnostic. The bridge derives both from one `DeviceId`, so a mismatch means a broken bridge,
  not a peer attack.
- **Re-keyed by full identity:** `callPeers`, `callParticipants`, `callPeerStates`, `peerMeta`,
  `waitingIce`, `remoteStreams`, the remote-audio router and chains, `peerTransport`, room
  presence (`voiceRooms`), `speaking`, focus/solo, every jam map (channel capabilities, decoders,
  call-epoch budgets, causal queues, sender sessions, held notes, manual/abuse mutes), jukebox DJ
  attribution, recording participants/consent and all draw/claim state. The self key is
  `callSelfId`, the full identity of this device on the call server.
- **Perfect-negotiation politeness** becomes `callSelfId < peerId`. Two different 8-hex prefixes
  order the same way as their 64-hex extensions, so a new build and an old build still pick
  opposite roles. Only an exact prefix collision can disagree, and old builds are already ambiguous
  there.
- **Outbound targets** are identities. The roster read (`get_members`) supplies `{fingerprint,
  identity}`; a member counts as online when its fingerprint is in `get_online_members`.
  - A colliding offline member can therefore be addressed, but `send_call_signal` routes only
    through that exact identity's own signed record, so the signal still cannot reach the wrong
    device.
- **Display** (names, monograms, profile frames, the `xxxx·xxxx` label) resolves through
  `identity.slice(0, 8)`. Profiles, roles and names are keyed by fingerprint across the whole app.
  That is a pre-existing display residual and is not an authority key.
- **Per-peer volume trims** stay stored under `catcoms.call.level.<fp>`. They are a local cosmetic
  preference, and keeping the key preserves users' existing trims. A colliding device would inherit
  a trim, which is cosmetic.
- **Jam takes recorded by a Gate 5 build** name their performers by full identity. The take
  validator already allows identities of up to 256 bytes, and take playback derives its engine
  domains from `parts` either way. An older build shows such a performer as the raw id. That is a
  cosmetic mixed-version effect, not a format break.

## 5. Draw channel transport

- `pc.createDataChannel("draw", { negotiated: true, id: 8, ordered: true })` is created in
  `createPeer` next to `inst`, before the first offer, so it rides the existing SCTP association
  and never renegotiates.
  - An older build has no channel with id 8, so frames sent to it are discarded and it sends none.
    Claims flow only between Gate 5 builds.
- **Capability.** Each opened `draw` channel mints an opaque `RtSourceChannel<"draw">` for the
  authenticated peer identity. Its callbacks close over that capability, and no authority is ever
  taken from a frame field. Reopen replaces the capability; removal closes it first.
- **Admission (C0b model).** Every delivery is charged against the identity's call-epoch draw
  budget before any content-dependent work:
  - Pre-parse cap: 1024 bytes, enforced in UTF-8 bytes.
  - All-frame bucket: 20/s, burst 40. This is new for `draw`, sized for claims only; C4a may revisit
    it when strokes arrive.
  - Auto-mute: sustained exhaustion for 10 s inside a rolling 60 s mutes that peer's draw channel
    receive-side for the rest of the call.
  - After parse, every draw frame must fit in 200 bytes.
  - The budget belongs to the identity for the call epoch, so reconnecting refills nothing. It is
    cleared when the call ends.
- **Message types.** Only `t:"c"` is implemented. Every other `t` is charged, parsed under the cap
  and rejected as unsupported. No other draw state machine exists in Gate 5.

## 6. Claim message and semantics

### 6.1 Wire

```
{"t":"c","sn":<16 lowercase hex, not all zero>,"q":<uint32, < 0xffffffff>,
 "obj":<32 lowercase hex>,"fr":<32 lowercase hex>,"on":0|1,"ask":0|1}
```

The key set is exact: all seven keys are required and no others are allowed. Integers must be
JSON integers with no fraction or exponent beyond what `Number.isInteger` admits. `obj` and `fr`
use the Studio element-id shape (`ELEMENT_ID_HEX = 32`). The frames are about 120 bytes, well
under 200.

| `on` | `ask` | Meaning |
|---|---|---|
| 1 | 0 | **Claim**, also the 30 s refresh: "I am drawing frame `fr` of `obj`." |
| 0 | 1 | **Ask**: "I would like frame `fr`; whoever is drawing it may Pass." |
| 0 | 0 | **Release**: this sender withdraws its claim **and** ask on `(obj, fr)`. Pass is a release that answers an ask. |
| 1 | 1 | Malformed; rejected. |

**Design question for review (D1).** Section 2.10 spells the variants `on:1`, `on:0`, `ask:1`. The
2.7 frame table lists the full key set `{t, sn, q, obj, fr, on, ask}`. This design keeps the exact
seven-key set, so every frame has the same shape and no key is optional. Pass carries no
recipient. A claim is self-asserted by its authenticated sender, so a holder cannot claim on
someone else's behalf. The asker completes the hand-over by claiming (6.4).

### 6.2 Session and sequence binding

- The sender mints `sn` once per call join, from `crypto.getRandomValues`, 16 lowercase hex, never
  all zero. `q` is one per-sender counter shared by all draw frames under that `sn`. Each frame is
  numbered once even when it is broadcast to several edges, so every edge sees a strictly
  increasing subsequence. If the sender would reach `0xffff_ffff`, it mints a fresh `sn` and
  restarts `q`. The receiver rejects `q == 0xffff_ffff`.
- **Receiver state per draw capability:** the current `sn`, the highest accepted `q` for it, and a
  bounded list of retired `sn`s.
  - The first valid frame binds the capability to its `sn`.
  - A frame with the current `sn` must have `q` greater than the highest accepted; equal or lower is
    rejected as a replay or reorder.
  - A frame with a new `sn` is a **session change**. It drops every claim and ask that capability
    created, retires the old `sn`, rebinds, and then applies the frame.
  - A retired `sn` is rejected as a stale session.
  - More than 4 sessions on one capability is rejected (`session-limit`); the sender must reconnect.
    This bounds the retired list.
- **Capability replacement or closure** (reconnect, `removePeer`, `bye`, roster revocation,
  leaving the call) drops every entry that capability created, at once. A frame delivered through a
  replaced or closed capability fails the `isCurrent` check before it can touch anything.
- **The call boundary.** One claim session object exists per call join and is disposed on leave.
  Callbacks retained from an earlier call hold the earlier, disposed object, which accepts nothing.

### 6.3 Receiver claim book

- Entries are keyed by `(sender identity, obj, fr)` and record their kind (`claim` or `ask`), the
  capability and `sn` that created them, and `seenMs`, the receiver's own monotonic receipt time
  from the injected clock.
- **Claim** creates or refreshes this sender's claim on `(obj, fr)` and replaces its ask there.
  **Ask** creates or refreshes this sender's ask and replaces its claim there. **Release** deletes
  both.
- **Expiry is receiver-observed.** An entry is live while `now - seenMs < 90_000`, and at exactly
  90 s it is gone. Sender clocks are never read: no timestamps travel. Expired entries are pruned
  on every read and every write.
- **Bounds.** Each sender holds at most 4 live entries (claims plus asks); a new key beyond that is
  rejected, while refreshing or releasing an existing key always works. The claim-semantic bucket
  allows 2 claim frames per second with a burst of 10, on top of the all-frame bucket. The book's
  total size is therefore at most 4 times the number of call peers (mesh ceiling about 8 to 16).
- **Collision.** A frame can have several live claimants. Views list them sorted by identity, so
  both delivery orders produce the same state, and they mark the frame as contested. Nothing is
  resolved automatically.
- **Prefix collision.** Identities are compared as full 64-hex strings. Two identities that share
  their first 8 hex characters are independent: one cannot refresh, release, expire or session-reset
  the other's entries.

### 6.4 Local sender, Claim, Ask and Pass

- **Claim.** The local user claims the selected frame when they first paint on it or press
  "claim". The claim is sent at once to every open draw edge and re-sent every 30 s while held.
  Selecting another frame, closing the object, leaving the Studio or leaving the call sends a
  Release. The UI keeps at most one local claim and one local ask.
- **Ask.** Available on a frame someone else claims. It is sent once and is **not refreshed**, so
  the holder's countdown runs from 90 s to 0 and the notice then disappears. The asker may ask
  again explicitly, within the semantic bucket.
- **Pass.** The holder sees a sticky "X asks for frame N · 52 s · Pass" notice for each live ask on
  a frame they claim. Pass sends a Release and suppresses auto-claim on that frame until the user
  selects another frame or claims explicitly.
  - When the asker sees the frame free while its own ask is live, it claims the frame, clears its
    ask and says "passed to you". The frame is free when no live claim from anyone else remains,
    whether that came from a Pass, a release or expiry.
  - Two askers can both claim; the result is shown as a collision.
- **New edge.** When a draw channel opens, including after a reconnect, the sender sends its
  current claim and ask to that edge only, with the next `q`.
- **Pacing.** The sender mirrors the receiver's claim bucket locally. If the user acts faster than
  that, the sender coalesces to the latest desired state and sends it when a token is available,
  so honest frames are never dropped by an honest receiver.

### 6.5 Advisory only: never a lock

- The claim module exports no "may edit" predicate. Studio editing, saving, Save refusal and every
  native command path never import or consult it. A source-hygiene test enforces this for
  `studio-session.ts`, `studio-native.ts`, `studio-state.svelte.ts` and `studio-store.ts`.
- Painting a frame someone else claims is allowed and shows a collision warning.
- Claims are honest-client coordination. A modified client can claim everything, never refresh, or
  ignore Pass. The bounds above limit what that costs receivers.

### 6.6 Only during a call

- The local user sends claims only while in a call. The Studio shows peers' claims only while in a
  call whose server is the Studio's server, and only for the object it has open.
- Leaving the call clears everything. A peer's disconnect, `bye`, removal or reconnect drops that
  peer's (or that capability's) entries immediately rather than waiting 90 s.

## 7. Studio surface (5E)

The existing layout and style are kept. Additions:

- **Timeline.** A small marker in the claimant's colour on each frame with live peer claims, and a
  contested marker when there are two or more. The title text gives the names and seconds left.
- **Frame card** (Art tab, "frame N" section). Replaces the placeholder sentence:
  - "you are drawing this frame" with a Release button.
  - "X is drawing this frame · 74 s" with an Ask button.
  - "you asked X · 61 s".
  - A contested warning.
  - Outside a call: "claims appear while you are in a call; they never lock a frame".
- **Sticky ask notice** for asks on frames I claim, with a live countdown and Pass.
- **A 1 s UI tick** re-renders countdowns and drives the 30 s refresh. The tick reads the same
  monotonic clock as the book.

## 8. Threat notes

- **Identity.** Call authority keys are the full verified identity. A member who generates a device
  key sharing a victim's four bytes can no longer receive the victim's signals, take over the
  victim's call edge, share its signal-fairness bucket, or alias its jam/draw budgets, capabilities
  or claims.
  - Display names remain fingerprint-keyed app-wide; that residual is not changed here.
- **Replay.**
  - Draw frames ride DTLS between the two endpoints. Sequence and session checks reject duplicates,
    reorders and stale sessions from the sender's own edge.
  - Third parties cannot inject frames into it.
  - Residual, pre-existing and not changed: a call *signal* transcript binds neither recipient nor
    nonce, so a member can re-deliver another member's signed signal to a third member inside the
    60 s freshness window. Signals carry SDP, ICE, presence and `bye`; DTLS still binds the media
    and data-channel endpoint. This is recorded in THREAT-MODEL as a follow-up.
- **Floods.** Pre-parse cap and bucket, auto-mute, semantic claim bucket, per-sender entry cap and
  the session limit.
- **Honest-client boundary.** Claims, asks and Pass are advisory and labelled so, and are never
  authorization.
- **Privacy.** A claim reveals which frame of which object a call participant is drawing, and only
  to the participants of that call.

## 9. Tests and acceptance evidence

Frontend (`node:test`, injected clocks):

- `rt-admission.test.ts`:
  - Bucket monotonicity.
  - Charge-before-parse.
  - UTF-8 cap.
  - Exhaustion auto-mute and its clearing.
  - Capability replacement.
- The unchanged jam tests.
- `draw-claims.test.ts`:
  - Exact-key codec, plus rejection of bad hex, uppercase, an all-zero `sn`, `q` bounds,
    `on:1,ask:1`, oversize input, non-string input, invalid JSON and unsupported `t`. Each
    rejection leaves the book unchanged and is charged before parse.
  - Two-member observation.
  - Expiry at 89,999 versus 90,000 ms.
  - The 30 s refresh.
  - Ask countdown.
  - Pass hand-over.
  - Collision in both orders.
  - Replay and reorder.
  - Stale session and retired `sn`.
  - Session limit.
  - Stale capability.
  - Disconnect and reconnect clearing.
  - Call disposal.
  - Prefix-collision independence.
  - Per-sender caps.
  - Sender pacing.
- `call-identity.test.ts`: the sender pair parser.
- `voice-signaling.test.ts`: identity-keyed hangup targets with colliding fingerprints.
- The source-hygiene test that claims never reach Studio edit or save paths.

Rust:

- `catcoms-crypto`: `from_canonical_hex` accepts the `Display` round trip and rejects malformed
  input.
- `catcoms-sync`:
  - The full-identity inbound round trip.
  - A forged prefix-twin `DeviceId` gets no route to the victim.
  - Fairness for two prefix-colliding senders.
  - The legacy resolver refuses ambiguity (a pure helper with crafted ids).
  - The existing rate tests, re-keyed.
- `catcoms-app` actor: the event carries `from` and `from_fp` of one id; a malformed command
  target is refused.

Mandatory gates before handoff (AGENTS.md):

- `cargo test --all --all-features`
- `cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml`
- `npm --prefix apps/desktop test`
- `cargo fmt --all -- --check` and both strict Clippy runs
- `bash scripts/check-no-ambient.sh` and `cargo deny check`
- `cargo check` of the desktop workspace
- `npm run check` and `npm run build`

The `test:startup` and `test:flows` gates need a display/Edge, so they run where available or are
reported as not run. A read-only adversarial review of the actual diff follows.

## 10. Open decisions for the user

- **D1:** The wire spelling of the seven-key claim frame, and Pass carrying no recipient
  (section 6.1).
- **D2:** The new draw all-frame bucket rate (20/s, burst 40) until C4a strokes exist (section 5).
- **D3:** New jam takes naming performers by full identity rather than by fingerprint (4.2).
- **D4:** Whether DM invites should also move to full identity. The proposal is a later slice; Gate 5
  only makes their resolver refuse ambiguity.
