# V2 voice signalling: production scheduling and the call driver

22 October 2026, branch `communication-v2-signalling` (from `f227b4f7`). Package 2 of the
[chat, files and voice plan](CHAT-FILES-VOICE-PLAN.md), under the
[V2 signalling decision](VOICE-SIGNALLING-V2.md). **Live V2 calls are still not enabled, and V2
network service is OFF by default.** This increment adds the production dispatch and scheduling
path behind a local switch and pins the renderer half of the call contract. Turning service on is a
separate reviewed rollout decision; real SDP/ICE through the native-authorized path waits for the
handoff adapter (see "Activation").

## What this change provides

| Layer | State |
| --- | --- |
| Kind-28 request dispatch | **Implemented, off by default.** With `VoiceRouting::Disabled` (the default) kind 28 is answered exactly as before (empty reply). `Direct`/`Forwarding` route it to the existing admission seam. |
| Endpoint/source/helper scheduling | **Implemented, off by default.** `catcoms-sync/src/voice_schedule.rs` runs the reviewed seams each sync tick while service is on and V2 state is live. |
| Helper forwarding (B not in the room) | Works under `Forwarding` with the group's authenticated policy active PeerToPeer (Rust tests). No production caller enables it. |
| Native V2 participation (LocalJoin, Watcher) | Exists (`publish_voice_join_key` reaches `start_voice_endpoint`), but no production UI calls it. |
| Renderer legacy exclusion for a V2 join | **Live code, inert in practice.** No join selects V2 yet. |
| V2 call driver (`voice-v2-call.ts`) | Implemented against a narrow native interface with contract tests. **Not activated.** |
| Encrypted SDP/ICE, native phase factories, handoff action delivery | Not implemented here; owned elsewhere. |

When enabled, the kind-28 service carries only what its seams already admit: public room controls
(Announcement, WatchChallenge, JoinProof) to native endpoint owners, protected confirmation
custody, and helper forwarding of signed envelopes. Encrypted SDP/ICE delivery to a renderer does
not exist yet, so a call cannot negotiate media over V2.

The read-only review held that default-on service would contradict the decision's own gate
("forwarding stays disabled until ... compatibility tests pass review"), with no production kill
switch. Before any default-on: an ADR amendment with the central documents, mixed-version
fixtures through normal dispatch, flood and multi-helper fan-out tests through real dispatch, a
desktop path to set the switch, and real TCP A-B-C with A-C forbidden under chat/blob load.

## File ownership and interface dependencies

Owned by this work: `crates/catcoms-sync/src/voice_schedule.rs` (new), the kind-28 dispatch arm and
per-tick pump in `catcoms-sync/src/lib.rs`, the voice rotation slot in `recovery_wait.rs`, the shared
`indirect` helper used by `voice_v2.rs` and `voice_runtime.rs`, `voice_endpoint.rs::needs_schedule`,
`apps/desktop/src/voice-v2-call.ts`, the V2 helpers in `voice-signaling.ts` and `call-identity.ts`,
and the small App.svelte gate (`callSignalMode`, `handleCallSignal`, `sendSignal`, `joinVoice`,
`leaveVoice`).

Not touched: `voice_handoff/*`, `voice-ipc-*`, `voice-crypto.ts`, native `lib.rs`, the command
security policy and the transport fixtures. The driver depends on a native adapter implementing
`VoiceV2Native` (below). That adapter belongs with the native/browser handoff owner.

## Production scheduling (Rust)

`pump_voice_schedule` is one bounded pass on the sole Root, and does nothing while `Disabled`:

1. settle any completion that was refused only for temporary ACK-verification capacity (at most
   8 retained; more are abandoned with their attempt spent, never refunded). This runs even with
   no current actor (locked), so such completions fail terminally instead of stranding credit;
2. return unless V2 state is live: a live native join or watcher, or a retained source/helper
   packet. These are cheap checks with no reconcile, directory read or route index, so an idle
   attached actor pays almost nothing per turn;
3. `pump_voice_endpoint` (renewed announcements, watcher challenges, join proofs: at most 4);
4. `prepare_voice_sources` (at most 4 flights);
5. `prepare_voice_forwards` (at most 4 flights; `Forwarding` only).

It runs at the top of each `run_once`, after each served kind-28 request, after each flight
completion, and on a 250 ms wake on the voice clock while V2 state is live. Helper edges now wait
500 ms between attempts (as source packets already did), so pumping after a refusal cannot send
all three attempts back to back. Every flight is a detached
connected-only request owned on the Root's `FuturesUnordered`, polled as the fifth source of the
existing fair recovery rotation, so a cancelled actor poll never drops a lower request or its
credit. Each completion is settled by the Root that prepared it (`complete_voice_source/forward`
re-verify the signed ACK, route generation and native owner). Announcement renewal is cached
until due, and the source ledger deduplicates by identity, so pumping each tick does not re-sign.

### Rollout switch

`ChannelSync::set_voice_routing(VoiceRouting)` is a local switch that can only narrow what the
authenticated policy allows:

| Mode | Behaviour |
| --- | --- |
| `Disabled` (default) | Kind 28 is not intercepted: the requester gets the same empty reply an unknown kind always got (unavailable, never "unsupported"). Nothing is scheduled; upper flight work is cancelled while lower credits stay until real retirement. Native owners can still be created exactly as before; they make no network progress. |
| `Direct` | Own endpoint traffic over direct bound routes only. `indirect` is false in every V2 Context and in the runtime policy View, so helper permits, helper delivery and helper hops all refuse. |
| `Forwarding` | Direct V2 plus one-helper forwarding. |

What a change ends, precisely:

| Change | Effect |
| --- | --- |
| `Disabled` <-> `Forwarding` | Pauses or resumes network service only. Native joins and watchers stay; while paused they make no progress and their signed lifetimes run out. |
| anything <-> `Direct` | Flips the `indirect` bit that is part of the runtime policy lease. Every live V2 join, watcher, helper edge and pending reply on that server ends (never converted to legacy); a fresh join is required. |

No desktop caller changes the default yet.

## Renderer: pinned mode and legacy exclusion

`callSignalMode` (`legacy` | `v2`) is pinned in `joinVoice` before the room becomes current and
before the first hello, and reset only at the end of `leaveVoice`, after the hangup went through
the gated send. While a join is V2:

- every incoming legacy `call-signal` for that room (hello, voice-ping, offer, answer, ICE, bye,
  juke and unknown types, with or without capability fields) is dropped in `handleCallSignal`
  before any presence write, peer creation, renegotiation or teardown, with a diagnostic carrying
  only the protocol word;
- nothing legacy is sent to that room (`sendSignal`);
- legacy signals for other rooms keep their presence-only meaning.

A wholly legacy call is unchanged. V2 is selected only when `voiceV2Activation()` is available,
which it is not.

## The V2 call driver contract

`VoiceV2Call` owns one V2 call. All authority stays native; the driver fences RTC effects.

`VoiceV2Native` (to be implemented by the native adapter). A rejected promise means uncertain
(lost reply); the driver retries the exact same request. A resolved value is authoritative.

- `start({server, room, attempt})` returns `started` with a lease `{lease, group, room, self, join}`
  or `refused`. The same `attempt` must return the same lease. If the call ends while a start reply
  is uncertain, the driver keeps retrying that same attempt (bounded) only to learn the lease and
  end it. A start whose every reply is lost must still be collectable by native (controller
  retirement), because the driver cannot end a lease it never learnt.
- `events(lease, after)` is a non-dequeueing pull: items after the decimal u64 cursor and the new
  cursor. Repeating `after` repeats the items. Each item carries a unique decimal `receipt`. A
  revoked or unknown lease must be answered authoritatively (an `ended` item), never by endless
  rejection; after 8 consecutive uncertain pulls the driver enters the non-terminal `uncertain`
  phase (reason `events-uncertain`), keeps polling and returns to `joined` on success.
  `unavailable` is always terminal for that join.
- `submit(lease, action)` for `offer`, `answer` and `ice`. The `action` id is the exact-retry
  identity: the same id with the same content returns the cached result, and with different content
  is refused. `submitted` means native admitted, signed and queued it, never remote receipt. An
  offer's result carries the native OfferId (issuer = this device).
- `end(lease)` is idempotent; native sends the V2 bye naming this exact join.

Items: `peer` (native watcher proof made this exact join fresh), `peer-gone` (removed, expired,
retired), `paired` (key confirmation done), `offer`/`answer`/`ice` (decrypted and committed by
native for the exact pair and OfferId), `bye` (exact join) and `ended` (our join ended: epoch,
policy, removed, room, locked, replaced, capacity). Malformed items are refused before any effect.

Driver rules:

- Presence creates no RTC object; `paired` does. The lower identity initiates (native confirmation
  order). The higher never offers; a counter-offer is crossed and refused without rollback. One
  exchange per pair (the narrow native SDP contract); a second, different offer is refused.
- A newer join of the same identity waits until its own `paired`, then the old pair is retired.
  If native retires the old join first, the newer join becomes the waiting candidate. Bye, SDP and
  ICE for any other join are ignored, so a delayed bye cannot dismiss a newer join.
- The initiator's offer runs on the pair's serialized chain, so an answer or ICE that outruns our
  own submit reply waits for the OfferId it echoes instead of being judged stale.
- ICE for a retired or foreign OfferId is refused, never reassigned. Early ICE waits for its
  remote description: at most 8 candidates / 16 KiB per pair, refused rather than evicting.
- Every RTC continuation and callback is fenced by call generation, exact peer record and OfferId.
  End (leave, lock, unmount, server leave) retires every peer synchronously before awaiting.
  A start answered after End is ended natively, never adopted.
- Native `ended` for epoch or policy starts a FRESH join (new attempt, new nonce, no carried
  peers), at most 3 per minute; other reasons end the call as unavailable. Nothing falls back to
  legacy; uncertainty and refusal are reported as V2 states.
- Status keeps presence, signalling (including "offer sent" as native admission only) and media
  (RTC connection state) separate.

## Evidence

Frontend focused tests (Node, real modules and the production App.svelte handler text):

- `voice-v2-call.test.ts`: 27 contract tests, including a delayed same-room bye after join
  replacement, old-join retirement arriving before the newer join's confirmation, simultaneous
  joins and crossed offers, stale and early ICE, ICE and answers outrunning our own OfferId, lost
  start/submit/pull replies, ending during an uncertain start, duplicate results, remount with a
  late start, lock with suspended RTC work, two calls on different servers with the same room id,
  epoch rejoin and its budget, native end reasons, bridge delay without downgrade, sustained
  uncertain pulls, presence expiry with live media, start refusal, malformed projections, peer
  capacity, key-confirmation timeout, and activation tied to App driver wiring.
- `voice-peer-lifecycle.test.ts`: every legacy signal kind into a V2 room through the real
  `handleCallSignal` (no presence, peer, SDP, ICE, juke or bye effect; stripped fields too), the
  unchanged wholly legacy path, other rooms in V2 mode and the mode reset on leave.
- `voice-signaling.test.ts`, `call-identity.test.ts`, `call-wiring.test.ts` (gate order pins).
- Mutation checks: disabling the exact-join bye check, the start-after-end fence, the App gate,
  the chained offer, pending-join promotion or uncertain-start recovery each turned exactly the
  intended test red. All 81 focused frontend tests pass; Svelte check reports zero errors/warnings.

Rust scheduling regressions in `voice_runtime/transport_tests.rs` drive the normal sync tick and
normal dispatch; no test calls a pump, prepares a flight or serves kind 28 by hand:

- Off by default: an attached actor with live native state answers kind 28 with the old empty
  reply, schedules no wake and submits no flight.
- A and C discover each other through unjoined B; every V2 flight from A or C goes to B, B
  forwards both ways and gains no participation, and an unbound member D is never used.
- B is removed and D bound: presence is renewed well past the 15 s liveness window, through D
  only, under the original joins; pairing still derives the same handshake on both ends.
- Kind 28 reaches native endpoint admission through normal dispatch (an announcement still admits
  no presence); `Disabled` restores the old empty reply; `Direct` refuses helper custody and
  narrows every Context.
- Routing flips: `Disabled` <-> `Forwarding` keeps a live join; any change involving `Direct` ends
  it (lease revoked) in both directions.
- A refusing recipient receives one helper attempt per 500 ms gap and never more than three.
- The quiet wake exists only while V2 state or real lower custody is live, and stops after the
  retained packet expires and its lower request has actually retired.

`cargo test --locked -p catcoms-sync --lib -- voice`: 448 passed, 0 failed (isolated target, two
jobs). The first run of the revised code passed 446 and failed 2. The existing helper-retry test
assumed back-to-back attempts, which is the intentional retry-gap contract change, now explicit in
the test. The new quiet-wake test also assumed a packet could expire while an unconsumed lower
request still held it; it now lets that request actually retire. Rust mutation checks: removing
the helper gap, or ignoring `Direct` in `voice_indirect_allowed`, fails exactly the three guarding
tests. `cargo fmt --check` and `scripts/check-no-ambient.sh` pass.

The Mem transport connects every peer; these tests show V2 used only bound routes, not physical
A-C exclusion. No WebView crypto, SDP forwarding, real RTC or audible media is claimed.

## Activation (explicitly incomplete)

Before `voiceV2Activation()` may report available:

1. native commands implementing `VoiceV2Native` over the handoff channel, with the exact-retry,
   non-dequeueing pull and post-decrypt authorization semantics above;
2. native SDP phase factories on a confirmed LocalJoin and encrypted SDP/ICE ingress to the
   driver, including OfferId minting;
3. App wiring for V2 peers (tracks, data channels, `ontrack` fenced by the driver's `isCurrent`),
   plus the visual fixture commands;
4. an agreed default for `VoiceRouting` and a desktop path to change it;
5. real TCP A-B-C with A-C forbidden, then native WebView and physical audio acceptance.

## Remaining limitations

- No renegotiation in V2 (one exchange per pair); adding video later needs pre-created
  transceivers or a reviewed second exchange. Media failure is reported, not repaired.
- The legacy exclusion also covers `juke`, so the shared jukebox has no transport in a V2 room
  until it gets a V2 or data-channel path.
- Diagnostics record each excluded legacy kind once per join, not every signal.
- Presence lapse with live media is shown as stale rather than torn down; removal always tears down.
- The 250 ms quiet wake costs a timer per group while V2 state is live.
- A start whose every reply was lost leaves native to collect the unadopted owner.
- This branch adds a dated THREAT-MODEL section and an acceptance-ledger entry. ARCHITECTURE,
  INTERFACES and HANDOVER still say kind 28 is undispatched, which remains true by default; the
  switch and scheduler should be recorded there at integration by the primary owner.
- No physical forbidden-edge TCP topology, flood test through real dispatch or mixed-version
  fixture exists yet; those are prerequisites for enabling service by default.

Residual LOW findings from the read-only re-review (no blocker/high/medium remains):

1. The helper retry gap is set when a completion settles. A completion the Root discards first
   (route/actor revocation, disabled-then-resumed service) leaves the old gap, so that edge may
   resend as soon as its flight drops, still within three attempts. Setting the gap at attempt
   time, as source packets do, changes four existing lost-completion tests and is a follow-up.
2. Retained terminal ledger records keep a `Forwarding` helper pumping at 4 Hz until pruned; a
   member could sustain that at its origin rate. This belongs with the flood gate for default-on.
3. The cheap work check reads the current actor before reconcile, so a newly installed actor can
   miss one wake until the next tick reconciles. It never grants authority.
4. The activation-to-wiring test matches App text only.
