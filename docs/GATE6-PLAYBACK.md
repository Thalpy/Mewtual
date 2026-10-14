# Gate 6 archival playback controller

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
All SFX share one engine-minted fairness owner, distinct from live identities; occurrences cannot
mint extra polyphony allowances. Reconnected take lanes retain their original engine-owned
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

## Verification scope

Focused regressions cover authored 10-minute holds, remaining seek age, short-note envelopes,
same-pitch overlapping SFX, lifecycle bounds after stolen tails, exact capability/replay/engine
binding, Deafen ABA, mutation before hashing, live watchdog preservation, scoped cancellation,
one-running/one-latest preparation, 128-event passes, no overlapping ticks and overdue attacks.
The existing engine/allocator suites remain intact. The parent agent owns complete root/native/
frontend suites and actual production lifecycle/visual validation after App connection.

At this checkpoint 70 focused plan/controller/engine/allocator tests pass; `npm.cmd run check`
reports zero errors/warnings and `git diff --check` passes. Design review reports no blocker.
Actual-diff adversarial implementation review is requested and pending; this published checkpoint
is not review acceptance or a claim that the complete mandatory suites have run on these changes.
