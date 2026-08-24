# Design: the diagnostics suite

Status: in progress. Driven by `docs/reviews/Mewtual_Adversarial_Debugging_Review.md`; the debug
console's own visual design is `docs/design-debug-console.md`.

This is the plan of record. It exists because the work spans many sessions and the decisions below
were expensive to reach: without them written down, the next pass re-litigates them or, worse,
quietly contradicts one.

## 1. What the review changed about the goal

The app was not short of log lines. It was short of a record that could answer a question.
Observations were spread across nine unrelated mechanisms with different fields, lifetimes, clocks,
naming and privacy rules, and none of them correlated. Given "my message did not arrive", the
evidence could not establish which of ten stages failed. Given "the unread badge did not appear",
it could not distinguish a backend that never emitted from a webview that never received.

So the target is not verbosity. It is a record that can say what was attempted, which stage failed,
what changed before it failed, what was lost, whether it recovered, and whether the report is safe
to share.

## 2. Decisions that are settled

Each of these was a real fork in the road. Revisit them deliberately, not by accident.

### 2.1 The console's six sections are a view, not the taxonomy

Events are tagged with one of 22 sections (`catcoms-diagnostics::Section`). The console renders six.
Collapsing them into one list would mean either a console with 22 rail items or a taxonomy too
coarse to filter on. `Section::view()` maps one onto the other.

### 2.2 Identifiers are keyed per session, not masked by pattern

The original approach searched finished text for things shaped like an IP or a peer id. That holds
only while everything sensitive looks the way the pattern expects, and it did not: a short peer id
rendered in the clear while the screen claimed to be safe to share.

Identifiers now enter an event only as a `SessionRef`, a keyed hash under a salt that is random per
diagnostic session. A peer id cannot leak however careless a call site is, and a report correlates
internally without becoming a stable cross-session tracking token. The old regex survives as the
console's render-time screenshot redaction and, later, as the export validator's second pass. It is
a backstop, not the primary defence.

**Known and deliberate hole:** `BridgedMessage` carries un-migrated `tracing` messages, which are
arbitrary text. It is a distinct type precisely so it stays greppable, and counting them measures
how far the migration to structured codes has got.

### 2.3 Capture mode and section level are separate axes

One switch forced a choice between capturing almost nothing and capturing the transport layer
narrating every address the node sees. So it stayed off, and nobody had a log when they needed one.
Mode (`Off`/`Safe`/`Enhanced`/`Full`) and per-section level move independently, and both change at
runtime without a restart. Both are controls in the console's Capture card, and neither is
persisted: a capture mode that survived a restart would be a privacy setting nobody remembers
making, and `Full` is meant to expire on its own.

**Capture starts Safe**, corrected from Enhanced. The original reasoning was that the in-memory
store never touches the disk, so it could afford the more revealing rendering. That was wrong when
it was written: the console has Copy and Save, so its contents reach a clipboard and a file on the
first occasion anyone has a reason to look at them, which is the same occasion they are about to
send them to somebody.

**A mode decides content, not verbosity.** Safe used to hold every section at `Info`, which made the
mode do a second, unrelated job. The correlation stages are recorded at `Debug`, because there is one
per stage of every command and none of them is interesting until something goes wrong, so the
default mode threw all of them away before anything could ask: the record could say a send failed
and never which stage. The base level is now the same for Safe and Enhanced. What a mode changes is
whether values render literally and whether the transport firehose is on; how much of the app speaks
is the per-section levels, which move separately.

Safe therefore costs exactly one thing: transport *debug*, which is not captured at all and so
cannot be recovered by raising the mode afterwards. The console says how many events the settings
excluded rather than letting that pass as a quiet period.

### 2.4 Typed errors add to the message, never replace it

A failed send says "message too long" today. Swapping that for "Message could not be sent" would be
a regression dressed as an improvement: the specific text is what tells someone what to do
differently. `AppError` preserves the message exactly and adds a code, a trace and a remediation.

`describeError` on the frontend reads both the typed shape and a bare string, which is what makes
the migration incremental: a call site can adopt it before its command is migrated.

### 2.5 Diagnostics must not become the latency they explain

Non-negotiable, and already the source of three real bugs caught during the work:

- an event the config excludes costs two relaxed atomic loads and **no lock**;
- a recorded event holds the store's lock for a bounded push, with no allocation, I/O or scanning;
- the clock is read outside the lock;
- field names are owned by the event, never interned in a shared table.

### 2.6 Alpha keeps opt-out logging

Default on, one click to turn off permanently. Revisit before general release, not before.

## 3. Milestones

| | Milestone | State |
|---|---|---|
| M0 | Stop the logger lying and crashing | done |
| M1 | Console out of `App.svelte` | done |
| M2 | `catcoms-diagnostics`: canonical event, privacy model, store, renderers | done |
| M3 | Correlation, typed errors, task supervision, invoke migration | done |
| M4 | Rebuild the console on the hub | reading and capture control done; findings, checks and virtualised list outstanding |
| M5 | Findings and checks | not started |
| M6 | Export bundle and GitHub issue flow | partly: text export done |
| M7 | Hardening, privacy property tests, performance budgets, CI gates | not started |

### M3, as delivered

Wrappers, emit sequence numbers, task supervision and `AppError` are in, and all eight subsystem
groups have been through:

1. messages: send, edit, delete, react, pin
2. channels, unread, inbox: the unread **decision** is recorded with its reason
3. jukebox: queue digests, so an event that changed nothing is visible
4. join, connectivity, reachability: aggregate route findings, and the connectivity panel joined to
   the record by trace
5. files and transfers: begin/finish bracketing, so an abandoned upload is visible
6. voice signalling: a delivery with no member route recorded as its own outcome
7. vault and persistence: partial unlock made visible
8. documents and settings: wiki, status, calendar, channel topic

**Read commands were deliberately left on string errors.** Their failures are almost entirely
"locked" or "unknown server", so a code adds nothing a caller can act on, and migrating them would
be churn with no reader. The same judgment applies throughout: instrument where the diagnosis was
missing, type the errors a user must act on, and leave the rest.

### The trace, end to end

M3's headline requirement, and the last thing it was missing (P3-004). A command's own stages were
correlated; everything the actor did *in response* was a separate record, so a `channel-updated`
arriving two seconds after a send carried the emit's sequence number and no evidence of being that
send's consequence. Six of the ten stages were past that line.

The trace now travels:

```
frontend invoke → Tauri command → AppCommand → actor → AppEvent → emit → listener → refresh
```

Three mechanisms carry it, one per boundary, each chosen so the crossing could not be forgotten:

* **Into the actor**, as an envelope on the command channel rather than a field on each of the
  fifty `AppCommand` variants. `ServerActor::with_trace` returns a handle whose commands carry one;
  `Operation::actor` and `channel_target` return an already-bound handle, so a command cannot get
  an actor without also adopting the operation.
* **Out of the actor**, as an envelope on the event channel. The actor handles one command at a
  time, so the sink adopts that command's trace for as long as the arm runs and stamps whatever it
  emits. The `sync_once` branch clears it: an op arriving from a peer belongs to no local operation,
  and a trace that gathers an unrelated stage asserts a causal link that never existed, which is
  worse than a trace with a gap in it.
* **Across the `tracing` facade**, by lifting a `trace` field into the canonical `TraceId`. A
  library crate emits through the facade precisely so it does not depend on whichever binary is
  observing it, so `catcoms-app` states its operation as a field and the bridge makes it structure.
  One `hub.trace(id)` then recovers both sides.

The emit carries `__trace` beside `__seq` in the payload, so the webview's stages join the same
operation: `UI.EVENT.RECEIVED`, the unread decision, and `UI.REFRESH.APPLIED` when the rows are on
screen. A send whose trace ends at `CHANNEL.SEND.PERSISTED` reached the disk; one that ends at
`UI.REFRESH.APPLIED` reached the eye, and until this existed there was no way to tell those apart.

**Not done: `UI.RENDER.SETTLED`.** The review lists it as a final stage. `UI.REFRESH.APPLIED` fires
when the rows are assigned and in scope, which is the last point the code can honestly speak for;
anything past it is a claim about paint that would need a frame callback to be true rather than
plausible. Left out rather than faked.

* `push_file_chunk` still records no per-slice event, on purpose: a large file is thousands of
  slices. Its trace is used for the per-chunk progress emit, which is what a stalled transfer is
  diagnosed from.

### M4, as far as it goes

The console reads the canonical record now (P3-005). `catcoms-log`'s `LogRing` and its `project()`
are **deleted** rather than deprecated, so nothing can reach for the flattening again;
`catcoms_diagnostics::event_view` renders an event at a mode passed in by the caller, and the
desktop bridge carries every field of it across.

What that buys, concretely:

* The console groups by the section an event states, not by which crate emitted it or by searching
  the rendered text for the word "voice". A structured voice event from the webview is a voice
  event; a storage line that mentions a voice memo is not.
* A line shows its section, its trace and, for a migrated call site, its code, phase, duration,
  attempt and references. Previously four characters of the trace survived and the rest did not.
* The mode is a real control with a confirmation on the two modes that start writing this device's
  addresses down, and every page and every event says which mode it was rendered at.
* Per-section levels are adjustable, which is the half of decision 2.3 that had no user interface.
* Storage gained a feed, so an integrity failure or an abandoned upload has somewhere to appear.
* A trace filter, so pasting four characters off an error banner narrows every feed to that one
  operation.

Still outstanding for M4: the findings panel, the checks panel, the notes pad, and the virtualised
list the review asks for. The current feeds render a capped slice, which is bounded but is not the
same thing.

**Two renderers now describe one event.** `eventText` in `debug-console.ts` deliberately mirrors
`event_line` in `render.rs`, because the console composes the report text today and M6's bundle will
be written natively. They agree by inspection and by the comment on each; that is weaker than a
shared implementation, and M6 should collapse them rather than add a third.

### Corrections made after review

* **P3-001 (critical), fixed.** `AddressValue::new` took "the last non-numeric segment" as the
  transport. A real multiaddr ends `/p2p/<peer id>`, so Safe mode rendered raw peer ids: the exact
  leak the typed model was built to prevent, produced by the most ordinary input in the system, and
  missed by every test written alongside it. It matches a closed set of transport names now, so an
  unrecognised address shape renders nothing rather than whatever sat in the last segment.
* **P3-002 (critical), partly fixed.** The report footer promised it "never includes message text,
  file contents, names or key material" while the report writes every server name and the `tracing`
  bridge carries arbitrary prose. The wording now describes what a report may contain and asks the
  reader to check. The real fix is the export validator, which is M6's work; until it exists the
  honest sentence is the fix, because a false safety label is worse than none.
* **P3-005 (high), fixed.** See "M4, as far as it goes" above.
* **P3-011 (medium), fixed.** A trace minted in the webview arrived as a *field* called `trace`, so
  it rendered as text, never reached `DiagnosticHub::trace`, and could not gather the webview's half
  of an operation with the native half. Correlation stopped precisely at the bridge it exists to
  cross. Both doors the webview can knock on now share one `parse_trace`, and an all-zero trace is
  treated as absent rather than as something to correlate on.
* **P3-007 (high), fixed.** The console's poll ran the log read and one `get_member_routes` per
  server on the same one-second tick, with no guard, so five servers meant five round trips a second
  and a tick that overran simply had another started on top of it. The two polls are separate now,
  each with a re-entrancy guard; reachability runs every third tick, only while a section that
  renders it is open, and fans out concurrently rather than walking the list.
* **P3-004 (high), fixed.** See "The trace, end to end" above.
* **P3-012 (medium), fixed.** The hot-path guarantee in the hub's docs described writes and said
  nothing about readers, and the readers were the expensive half: `since`, `section_since` and
  `trace` deep-cloned every event they returned *while holding the one lock every producer needs*.
  An event can carry thirty-two fields of owned strings, the console polls every second, and an
  export pages the whole ring. The store holds `Arc<DiagnosticEvent>` now: a read locates its range,
  clones handles and lets go, and rendering happens outside the lock. Pointer identity is what the
  test asserts, because that is the property.
  Two smaller things in the same file. `Ring::index` was a linear scan over the twenty-two sections,
  twice per push under the lock, which is exactly what `Section::index()` exists to avoid and what
  its own doc comment warns about. And polling started from the oldest retained event every time, so
  a quiet second walked the entire ring to return nothing; sequences are dense across the deque, so
  the starting point is arithmetic.
  **No timing benchmark.** The review asks for a p95/p99 budget. A wall-clock measurement needs a
  monotonic instant read that `scripts/check-no-ambient.sh` forbids outside `catcoms-rt`, and a
  timing assertion in this suite would be a flake waiting for a busy machine. What is tested instead
  is the structural property the budget was a proxy for, plus readers and writers composing under
  contention without deadlock or a torn read.
* **P3-009 (high), fixed.** Only the server actor was supervised. Six other long-lived tasks had
  their `JoinHandle` dropped on the floor, so their deaths were unobserved: the network monitor, the
  discovery timer, the four reachability folds, and the event forwarder. The forwarder is the one
  that matters most, because it can die while the actor stays perfectly healthy, and what a user
  then sees is a stale unread badge, stale presence and a frozen jukebox on a server that is
  otherwise working.
  A `TaskRegistry` holds what became of each. The point is that it *outlives the log line*: a panic
  used to produce one `tracing` line, and once that aged out of the ring the task's state was not
  "dead" but unknown. State that has to stay in a bounded buffer to be true is not state.
  The console shows it first, above everything else, phrased as what the user will see rather than
  as the task's own name: "event_forwarder panicked" is precise and tells nobody anything.
  A stall is only ever claimed about a task that declared how often it expects to do something. The
  forwarder can have nothing to forward for an hour and be working perfectly, and a panel that
  cried wolf about that would stop being read.
  **No restart policy.** The review permits a bounded one for stateless monitors. It is not here,
  because nothing would act on it: the forwarder owns a receiver that dies with it and cannot be
  restarted without restructuring, and a policy field nobody consults is the same mistake as the
  unused `Remediation` variants already recorded in section 4.
* **P3-008 (high), fixed.** `LineWriter::write` did `extend_from_slice` with no per-event bound, so
  one formatted event could allocate an arbitrarily large `Vec` on whichever thread emitted it,
  arrive as a single queue item, and carry the file past a quota in one write rather than being
  stopped at it. The segment and session quotas are enforced by the worker, which only sees a line
  once it has been built, so a bound there is three problems too late. `MAX_EVENT_BYTES` caps it at
  the point of writing, the line says `[truncated]` for whoever reads it, and `events_truncated` is
  counted separately from `events_dropped`: a dropped event is absent and its absence is invisible,
  while a truncated one is present and usually still holds the part that mattered.
  Also bounded, for the same reason (the type is a `String` from a caller and nothing about that
  says "short"): the event's `target`, runtime field names from the bridge, and the literal address
  in `AddressValue`. `SafeText` and `BridgedMessage` were already bounded.
  Two notes on the tests. The write must report the full length even for bytes it discarded, or
  `io::Write` reads the short count as backpressure and the formatter offers the remainder forever.
  And the boundary walk needed a second attempt: the first passed only because 64 KiB happens to be
  a whole number of four-byte characters, so the test now covers every offset into one.
* **P3-006 (high), fixed.** Both frontend batchers counted a loss only when `send` threw
  *immediately*, and the real send returns a promise: it was fired and forgotten, the rejection
  arrived outside the batcher, and the counter still read zero. The pipeline reported perfect health
  exactly when the bridge was unhealthy. The transport is explicitly asynchronous now, returning
  `{offered, accepted}`; one batch is in flight at a time and is not retired until it is answered; a
  rejection is retried exactly once and then counted. Partial acceptance is counted too, because the
  native limiter suppressing a storm is it working as designed and is still a loss the webview is
  entitled to know about. What was lost while the bridge could not be reached rides on the first
  batch that gets through, so the count reaches the record rather than a counter nobody reads.
  The queue, the retry and the accounting are shared by both channels, which had the same bug.
  Prose and structured records now have **separate native budgets**: while they shared one, a
  `console.warn` storm spent it and suppressed the structured events describing what was going
  wrong.
* **P3-010 (high), fixed.** Detection existed and repair did not, so the record could say the UI had
  gone stale while the UI stayed stale. All seven parts:
  * Every event goes through one listener wrapper, not just `channel-updated`. It is a shim with
    `listen`'s own shape, so no listener had to remember to opt in: the one that forgets is exactly
    the one whose gaps go unnoticed.
  * `reachability-changed` and `switchboard-changed` were bare server ids with nowhere to put the
    bookkeeping, which made them the two families whose gaps could never be detected. Both are
    objects now, and an unstamped event is reported rather than passed over in silence.
  * The tracker is seeded from `get_event_cursor` before the listeners go live, so what a remount
    slept through is a detected gap instead of the new baseline.
  * A gap requests exactly one authoritative resynchronisation, coalesced: channels re-read,
    unread rebuilt from durable heads, the open conversation refreshed.
  * `__ord` gives the stream a total order alongside the per-name `__seq`. The per-name sequence
    says *what* to re-fetch; the stream's says whether anything was missed at all, including the
    last event of a family, which leaves no successor to be numbered against.
  * A successful `emit` still does not prove a listener handled it, but `UI.EVENT.RECEIVED` is now
    recorded under the same trace, so the two ends are comparable.
  * A payload that shadows an envelope key is recorded as `IPC.EVENT.ENVELOPE_COLLISION`. The
    envelope still wins, because the alternative is the frontend reading application data as a
    sequence number, but it no longer wins in silence.
* **P3-014 (medium), fixed.** Capture settings stopped events at the *store*, and the `tracing`
  bridge formats every field of every event into a `String` before the store sees it. So turning
  capture off meant the app kept paying to be watched and stopped keeping the results, which is the
  worst of both. The hub's gate is consulted in the layer's filter now, before anything is
  rendered. The test counts renders through the field's own `Debug` impl rather than asserting on a
  proxy: with capture off, across a hundred events, the count does not move.

### Off means off, audited

"Off" was true of the store and not of anything in front of it. Four places kept working:

| Path | What it cost while off | Now |
|---|---|---|
| The `tracing` bridge | Every field of every event rendered to a `String` | Gated in the layer's filter |
| `new_trace`, `new_span`, `reference` | The store's mutex, two or three times per traced command | Lock-free: an atomic counter and an `Arc` salt |
| Native event building | References cloned and text bounded for events the gate rejects | `record_with` builds only if it will be kept |
| The webview's recorder | Objects built and an IPC call every 250ms, discarded natively | Told by `capture-changed`; produces nothing |

The last one is the one that could not be found by reading the native side: the webview cannot see
the gate, so it had to be told. It is told rather than polled, because the mode only moves when
somebody moves it.

`record_with` takes a closure and still counts the rejection. An `admits` check at each call site
would have been simpler and would have quietly lost `filtered`, which is the number that separates
a section silent by policy from one silent because nothing happened.

Each is pinned by a test that counts the work rather than asserting on a proxy for it: a `Debug`
impl that counts renders, a closure that counts builds, a fake bridge that counts batches.

### What "turn off the debug log" actually turns off

Worth stating because it is easy to assume otherwise, and a wrong assumption here is a privacy one.
The Settings checkbox controls the **file**. The console's in-memory record is a separate sink with
its own control, under Capture in the console, and it is never written anywhere unless the user
presses copy or save.

They are deliberately separate, for the reason in decision 2.3: the console has to be usable while
a problem is happening, and making it depend on having enabled a file beforehand and restarted puts
it out of reach in exactly the situation it exists for. What was wrong was that the Settings page
did not say so, which left a reader to conclude that one checkbox stopped everything. It says so
now. Whether the checkbox should *also* stop capture is a product decision, not a technical one; the
argument against is that it re-conflates the two axes 2.3 exists to separate.

### Nothing was starting the application

The P3-009 work shipped a panic in `setup`: `tokio::spawn` called from the main thread before the
async runtime is entered, which is fatal at startup. No window, exit code 101, and it passed every
gate in this repository.

It passed because none of them start the binary. `cargo test` tests functions and never constructs
a Tauri app; clippy cannot know which thread a call happens on; the flow check drives the visual
fixture in a browser and never launches the app. The first thing to notice was a person running it.

Two guards, because the bug had two sizes:

* `supervise_detached` gives the hazard one home: Tauri's spawn on the outside, tokio's inside,
  where a runtime is current. A plain `#[test]` (deliberately not `#[tokio::test]`) calls it from
  the same context `setup` is in, and reproduces the panic verbatim if the nesting is removed.
* `npm run test:startup` launches the built binary and waits for a marker written on the last line
  of `setup`. It does *not* assert liveness: it opens a real window, and somebody closing that
  window is not a failure. What it asserts is that startup finished, and that nothing panicked.
  Confirmed by reintroducing the bug and watching it reproduce the exact message and line.

The marker earns its place beyond the test. Everything before it can fail in ways that leave a
plausible-looking log, so "the log stops before `STARTUP.SETUP.COMPLETE`" turns a guess into an
answer.

### Two things that only showed up by running it

Neither was in a review, and both are the kind of thing a passing suite says nothing about.

* **The default mode was throwing the whole trace away.** Every correlation stage is recorded at
  `Debug` and Safe held every section at `Info`, so the chain built for P3-004 would have been
  invisible under the default the previous commit had just introduced. Caught because a test
  asserting the two sides of the bridge join up returned an empty list. See decision 2.3.
* **Optional parameters do not survive this file's TypeScript strip.** `trace?: string` in
  `App.svelte` leaves the `?` behind, and the only symptom is the app's own startup screen with a
  `SyntaxError` and a stack of no frames. `npm run test:flows` caught it and reported only "visual
  fixture never became ready"; it now prints what the page said, because the browser had already
  named the line and the harness was holding it in a buffer nobody printed.

## 4. Deferred, with reasons
- **`check_connection` and `retry` remediations.** Added with the subsystems that can return them.
  An enum listing outcomes no code produces invites a UI branch for a case that never happens.
- **Structured detail on `AppError`.** Removed rather than carried unused.

## 4a. Every finding from the Part 3 review
<!-- Seventeen of twenty resolved. Open: P3-015, P3-018, P3-020, plus the named remainders of
     P3-002, P3-016, P3-017 and P3-019. -->


`docs/reviews/Mewtual_PFixes_Part3_Adversarial_Review.md`, pinned to `f6c1be6`. Read the review for
the detail; this is the scoreboard.

**All twenty are listed, fixed ones included.** They used to be two lists, open here and fixed under
"Corrections made after review", and P3-003 fell between them: the work was done and committed and
the record simply lost it. A diagnostics project whose own tracking silently drops a row is not in a
position to lecture anyone, and two lists that have to agree are one list too many. The count is the
check: if this table does not have twenty rows, something has gone missing.

**Do the open ones before instrumenting anything further.** The argument for the order is the
review's own, and it is sound: more instrumentation widens the gap between what is recorded and what
can be read.

| ID | Sev | Finding | Status |
|---|---|---|---|
| P3-001 | Critical | Safe multiaddr rendering can expose a raw peer id | Fixed, `dbf965f` |
| P3-002 | Critical | The exported-report privacy promise is false and unvalidated | Wording fixed, `dbf965f`. The validator that would make a stronger claim true is M6. |
| P3-003 | High | Saved reports bypass retention and can fill the log directory | Fixed, `cfbead6` |
| P3-004 | High | Trace correlation stops before the actor and event pipeline | Fixed, `74166bf` |
| P3-005 | High | The console consumes a lossy, hard-coded Enhanced projection | Fixed, `dde528d` |
| P3-006 | High | Async frontend diagnostic-send failures are silently uncounted | Fixed, `93053a5` |
| P3-007 | High | Console polling can overlap and scales with server count | Fixed, `dde528d` |
| P3-008 | High | A formatted event has no size bound before allocation | Fixed, `8a403fb` |
| P3-009 | High | Task supervision misses the paths that make the UI stale | Fixed, `cd52a2c` |
| P3-010 | High | Event sequencing detects gaps but does not repair them | Fixed, `3770efa` |
| P3-011 | Med | Frontend structured events flatten into `UI.EVENT` | Fixed, `dde528d` |
| P3-014 | Med | Capture modes cannot override the tracing layer's static filter | Fixed, `3bcb6c7` |
| P3-012 | Med | Console/export reads clone events under the global hub mutex | Fixed, see below |
| P3-013 | Med | The event format permits duplicate JSON keys and forged text rows | Fixed, `6433907` + `013fc7a` |
| P3-015 | Med | Redaction and frontend field ordering break deterministic export | **Start here.** Directly undercuts the determinism claim in `render.rs`. |
| P3-016 | Med | Startup capture begins too late for static-import failures | Fixed, `3480681`. One part open: a failure to fetch `main.ts` itself needs `index.html`, which the CSP makes awkward. |
| P3-017 | Med | The typed-error registry is conventional, not enforced | Fixed, `d061574`. The `SERVER.ACTOR.UNAVAILABLE` split is not done; see below. |
| P3-018 | Med | Unread reconciliation uses an unsafe sender-clock cursor | Pre-existing, not introduced by this work. |
| P3-019 | Med | The jukebox digest is weak and the revision can saturate | Digest fixed, `f58d7f2`. The saturating revision is in `apps/desktop/src/jukebox.ts` and is still open. |
| P3-020 | Low | Some status messages overstate what was retained | |

## 4b. Found while fixing, not yet fixed

Three things that turned up as neighbours of the findings above and are worth writing down rather
than losing:

* **A locked session is reported as a broken server.** `actor_of` calls `require_unlocked_session`
  first, so a locked vault surfaces as `SERVER.ACTOR.UNAVAILABLE` with a `Restart` remediation
  rather than `SESSION.LOCKED` with `Unlock`. The user is told to restart the app when they need to
  type a passphrase. It is the natural companion to P3-017's unfinished half, splitting that code
  into the three states it currently conflates.
* **Webview field order is nondeterministic.** `record_ui_events` deserialises the webview's fields
  into a `HashMap` and iterates it, so events from that producer have no stable field order. That
  undercuts the byte-identical-output property for exactly the events the console shows most. It
  belongs with P3-015.
* **A fetch failure of `main.ts` itself still reports nothing.** P3-016 moved capture ahead of every
  static import, which covers a module that throws while evaluating. A module that never arrives is
  a different case and needs a first script in `index.html`; the app ships `script-src 'self'` with
  no `'unsafe-inline'`, so the review's first-choice inline bootstrap needs a separate file and a
  CSP decision.

## 5. Cooperative link test

A two-person test, run from the debug console, that exercises the paths between two peers and
leaves each side with a report. Scheduled after the M3 migration, deliberately: the probe is far
more useful once those paths are traced, or it reports a wall of bridged messages instead of a
readable story.

### 5.1 Shape

A picks a member and presses Start. B is **prompted and must accept** — it can never be initiated
silently, which is the review's requirement and not merely politeness. Both then run a scripted
sequence stamped with a shared random session nonce, so the two reports can be laid side by side.

Coordination rides one new kind byte on the existing `/catcoms/rr/1` request/response protocol
(kinds 0-12 are taken; the probe is the next). Not a new protocol.

### 5.2 What it exercises

1. **Reachability and round trip.** Can A dial B, over which family and transport, direct or
   relayed, and how long a request/response takes.
2. **Sync round trip.** A writes a marker into a scratch document; B confirms the head advanced and
   reports what it saw, then reversed. Targets "connected but no progress" and catch-up stalls.
3. **Message delivery and event sequence.** A sends a test message to a shared channel; B reports
   whether the event arrived, its sequence number, and what its unread logic decided.
4. **Voice signalling handshake.** Signalling without establishing media, reporting candidate types
   and whether a route exists.

### 5.3 What does not cross the wire

Logs. Each side exports its own report and the two are compared by hand. What travels is the
nonce, the probe steps, and coarse digests. No content, no names, no key material, no full
addresses, and no stable cross-session identifier.

A peer's diagnostic assertions are recorded as **its claims**, never as facts. A malicious peer can
say whatever it likes about what it saw, and a report that presented that as truth would be worse
than no report.

The session expires quickly, and clock offsets are observed rather than assumed equal: two devices
do not share a clock, and a timeline built as though they did invents causality.
