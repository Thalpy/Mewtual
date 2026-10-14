# Gate 6 archival playback controller

## Existing local/shared take consumer candidate (review pending)

The existing App `jamStartTakePlayback` path now delegates to `JamTakeTransport` around the same
archival controller. It no longer sends imported/take notes through raw live `noteOn`; a 95-second
authored hold or mid-hold seek therefore uses the exact remaining duration and the same allocator
deadline. Take files, shared-deck transport/consent and live-call packet formats are unchanged.
The existing call/server/channel/deck-CID/generation lease gates preparation, event dispatch and
gain changes. One App-owned latest-task queue bounds preparation across replacement instances;
the controller retains the combined 128-event work budget, delayed-drum age and release horizon.

Remote deck state cannot resume a suspended context. A local take/deck gesture can; in-progress
suspension cancels rather than queuing a burst for eventual reopen. Studio and local/shared takes
explicitly retire the other archival UI owner on a new Play. Deafen and membership edges retire
pending/live take ownership and suppress the previous shared CID until a new local deck control
or another track; a heartbeat alone cannot resurrect it. Leaving the call resets that suppression.

The controller exposes `completed` separately from `state === "stopped"`. Only the complete
authored horizon (or a valid seek already beyond it) marks natural EOF. Manual stop, stale scope,
revocation, suspended audio and render failure do not advance the DJ or restart a Studio loop.
The DJ alone advances after genuine EOF; listeners retain the existing done marker. Empty Scores
do not spin a zero-length automatic loop. The engine's opaque gain API applies the existing
jukebox volume to current/future archival lanes without exposing channels or changing live gains.

Focused transport/controller/Studio-loop tests cover remote/local resume, 95-second seek holds,
release completion, late EOF seeks, stale asynchronous leases, suspension, gain validation,
revoked/render-failed non-completion and empty-loop refusal. Complete suites and final audible
native verification remain integration-owned; this is not shared-deck PIXA support or Gate 6
acceptance by itself.

Candidate verification: 84 engine/planner/transport/controller/Studio-audio tests pass, followed
by all 97 legacy take/jukebox focused tests. Three source-layout guards intentionally moved from
the retired raw-channel scheduler to its new archival-controller enforcement path; their drain,
stale timer, Deafen and seek guarantees remain asserted and have behavioral transport regressions.
Static check reports zero errors/warnings. Production build passes with the existing large-chunk
warning. Independent caller review is pending for this checkpoint, not inferred from engine tests.

## Combined-review corrections (2026-10-14)

Archival drum events now retain their receiver-owned kit closing time. Delayed ticks consume
expired events within the same bounded work budget without replaying old hits. Eligible hits
carry their actual age; a private deadline is rechecked before and after queued seed hashing,
and limits the rendered tail to its remaining window (plus the existing 5-ms de-click margin).
Live drum behavior and seek's no-drum reconstruction rule are unchanged. Four regressions were
demonstrated failing before the correction; a seed-started signal explicitly pins the post-hash
expiry, partial-tail and regressing-clock cases. Engine/planner/controller tests pass73/73;
independent re-review closes the MEDIUM timing finding and LOW async-test gap.

`setArchivalPlaybackLevel(handle, level)` accepts only a current engine capability and finite
0..1 gain, applies it to existing and future lazy archival lanes, and never changes live sources.
Replacement, disposal and Deafen revoke that control. Its focused regression passes; integration
into the pre-existing shared/local take consumer remains a separately reviewed caller change.

This isolated implementation extends the existing JamEngine, not a second synthesizer. It does
not activate Studio controls, fetch content, publish files, register commands or close Gate 6.
The parent integration owns App lifecycle wiring and the combined mandatory full suites.

## Boundaries and API

`planJamArchivalPlayback(take, animation?)` validates through the existing take validator and
returns an owned, deeply frozen audio schedule. Animation timing is `{fps, frameCount, sfx}`;
it is not PIX/image validation or evidence of publication. `.pixa` playback enters through the
existing byte-capped `decodePixa` before this schedule. Data-only snapshotting rejects accessors,
hidden/symbol fields, non-plain instances, sparse/extended arrays, excess structure and text
before any digest yields. The immutable copy prevents later caller mutation changing timing,
patches or labels. Parsed content remains untrusted attribution, never membership authority.

`JamEngine.prepareArchivalPlayback(take, animation, isCurrent)` mints an opaque capability for
one complete validated log. One archival capability is current per engine; replacement, explicit
revocation, Deafen or disposal retires it. The engine binds every dispatched index to its own
lane, original sequence, patch, note and closing time; reused indices reject. Another engine,
forged handle or pre-Deafen handle has no authority. Deafen ABA also invalidates an unresolved
preparation. Existing patch preparation, deterministic drums, fixed synth topology, fairness,
receiver gates, release cap and limiter are reused.

`StudioPlaybackController(engine, nowMs)` exposes:

- `setScope({generation, server, channel, membership} | null, deafened)`: feed current trusted
  custody at every leave, lock, navigation, membership or Deafen edge. Advance generation even
  when returning to the same server/channel. Any edge stops playback synchronously.
- `startTake(take, offsetMs)` and `startPixa(bytes, offsetMs)`: call only from explicit local Play
  gestures. Audio resume is invoked immediately in that gesture turn, not by a remote callback.
  Seek/restart uses a fresh capability and sequencing domain. Neither method auto-fetches content.
- `tick()`: call from one surface-owned timer/render loop. It drains at most 128 expanded events
  per pass, including seek reconstruction, releases and cleanup; concurrent ticks do not overlap.
- `stop()`, `state`, `elapsedMs`: stop invalidates in-flight continuations and capability-scoped
  cleanup cannot remove a replacement. Reopening audio does not resume a stopped playback.

Preparation reuses the existing one-running/one-latest task queue. Scope and playback generations
are checked after every await. A pending drum digest cannot dispatch later events after revocation.
The controller recalculates elapsed time after each await; delayed attacks whose authored off has
passed are skipped rather than becoming a catch-up burst. No per-event or hidden timers exist.

## Timing and resource limits

Take notes retain their exact matching key-up, or end-of-log for an unmatched held note. Duplicate
attacks at an already held pitch do not replace the first attack's closing time. Frame SFX starts
at `floor(frame * 1000 / fps)` and holds for its authored `durationMs`, never a global fixed hold.
Each SFX has its own synthetic lane, so same-pitch overlapping occurrences remain independent.
All SFX share the take's first engine-minted performer owner, distinct from live identities; neither
occurrences nor the SFX track mint extra polyphony allowances beside that performer's score notes.
Reconnected take lanes retain their original engine-owned
performer grouping. Unsigned labels cannot consume or choke same-named live member voices.

Only bound archival dispatch receives a long remaining hold (`authored hold - seek/delivery age`,
at most 600000 ms). Both the audio-clock stop and allocator sweep use that same remaining value.
Ordinary `noteOn`, including local `remote:false` and remote live events, still uses the unchanged
30000-ms lost-key-up watchdog. Short archival notes ending inside attack/decay release from the
correct envelope level rather than jumping to sustain. Releases keep the existing 8-ms de-click
floor and 8000-ms ceiling; a late key-up can shorten, never extend, the hard audio deadline.

The combined schedule is at most 20000 take events plus three events per 4096 SFX (on, off,
cleanup), or 32288 entries. Every SFX source is created lazily and cleaned after authored off plus
release and 5-ms renderer stop tolerance. At most 64 SFX source lifecycles are retained even if
their release tails are stolen; the ordinary 16-held-per-owner and 64-global-voice limits remain
in force. Receiver overload may refuse an attack; it never steals a held chord or expands limits.

The entire animation is not capped to the take's ten minutes: 999 frames at 1 FPS ends at
999000 ms, and its last SFX can end at 1598000 ms plus bounded release. Controller completion
uses the maximum animation/take/SFX horizon plus the actual conservative release tail.
Cleanup and completion reserve the larger of authored recipe release, the receiver's legacy-wave
fallback release and the de-click floor, because Legacy Sounds Only can change after preparation.

## Verification scope

Focused regressions cover authored 10-minute holds, remaining seek age, short-note envelopes,
same-pitch overlapping SFX, lifecycle bounds after stolen tails, exact capability/replay/engine
binding, Deafen ABA, mutation before hashing, live watchdog preservation, scoped cancellation,
one-running/one-latest preparation, 128-event passes, no overlapping ticks and overdue attacks.
The existing engine/allocator suites remain intact. The parent agent owns complete root/native/
frontend suites and actual production lifecycle/visual validation after App connection.

At this checkpoint 75 focused plan/controller/engine/allocator tests pass; `npm.cmd run check`
reports zero errors/warnings and `git diff --check` passes. Design review reports no blocker.
Actual-diff adversarial implementation review and short re-reviews close two MEDIUM and one LOW
finding, with no remaining finding on this bounded capability/controller checkpoint. This is not
acceptance of App/native integration or a claim that the complete mandatory suites have run here.

The first actual-diff review identified and regression-pinned the joint score/SFX owner and
Legacy Sounds Only release-tail corrections above. Snapshot text accounting also charges field
names before downstream validation, including repeated names across events. The reviewer independently
ran the two final targeted plan regressions (2/2 pass); broader production integration remains open.
