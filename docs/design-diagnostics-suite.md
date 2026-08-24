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
runtime without a restart.

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
| M3 | Correlation, typed errors, task supervision, invoke migration | subsystems done, correlation incomplete |
| M4 | Rebuild the console on the hub | not started |
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

### What M3 did not do

All eight subsystem groups have been through, but **M3's headline requirement is not met**, and
`docs/reviews/Mewtual_PFixes_Part3_Adversarial_Review.md` is right to say so rather than let the
group tally stand in for it.

* **The trace does not cross the actor mailbox** (P3-004). A command's own stages are correlated;
  the work the actor does *later* in response is not. So a `channel-updated` arriving two seconds
  after a send carries the emit's sequence number but not the send's trace, which is precisely the
  ten-stage question the milestone existed to answer. This is the next real step.
* **The console consumes a lossy projection** (P3-005). `catcoms-log`'s `project()` flattens
  section, phase, span and refs away and hard-codes Enhanced rendering, so the console cannot
  faithfully show what the canonical model now records. The review argues M4 should be pulled
  forward before more subsystems are instrumented, and that argument is sound: instrumenting
  further widens the gap between what is recorded and what can be read.
* `push_file_chunk` accepts a trace and ignores it, on purpose. See its signature.

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

## 4. Deferred, with reasons

- **Reloadable `tracing` filter.** Runtime capture toggling works through the hub's gate, so a
  disabled section still costs field formatting before the hub discards it. A performance item;
  belongs with M7's budgets.
- **`check_connection` and `retry` remediations.** Added with the subsystems that can return them.
  An enum listing outcomes no code produces invites a UI branch for a case that never happens.
- **Structured detail on `AppError`.** Removed rather than carried unused.

## 4a. Open findings from the Part 3 review

`docs/reviews/Mewtual_PFixes_Part3_Adversarial_Review.md`, pinned to `f6c1be6`. Three are fixed
(see "Corrections made after review"); the rest are open. Read the review for the detail, and treat
this as the triage rather than the record.

**Do these before instrumenting anything further.** The argument for the order is the review's own,
and it is sound: more instrumentation widens the gap between what is recorded and what can be read.

| ID | Sev | Open finding | Note |
|---|---|---|---|
| P3-005 | High | The console consumes a lossy, hard-coded Enhanced projection | **Start here.** This is M4, pulled forward. |
| P3-004 | High | Trace correlation stops before the actor and event pipeline | M3's actual headline requirement. |
| P3-010 | High | Event sequencing detects gaps but does not repair them | Pairs with P3-004. |
| P3-011 | Med | Frontend structured events flatten into `UI.EVENT` | Falls out of P3-005. |
| P3-006 | High | Async frontend diagnostic-send failures are silently uncounted | The batcher counts sync throws, not rejected promises. |
| P3-007 | High | Console polling can overlap and scales linearly with server count | One `get_member_routes` per server per second, unguarded. |
| P3-008 | High | A formatted event has no size bound before allocation or queuing | I removed `MAX_EVENT_CHARS` when the ring moved to the hub. |
| P3-009 | High | Task supervision misses the paths that make the UI silently stale | Only actor tasks are supervised. |
| P3-012 | Med | Console/export reads clone events under the global hub mutex | Contradicts the hot-path work; the read side was never audited. |
| P3-013 | Med | The event format permits duplicate JSON keys and forged text rows | My hand-rolled JSON does not reject repeated field names. |
| P3-014 | Med | Capture modes cannot override the tracing layer's static filter | Already known and deferred; see section 4. |
| P3-015 | Med | Redaction and frontend field ordering break deterministic export | Directly undercuts the determinism claim in `render.rs`. |
| P3-016 | Med | Startup capture begins too late for static-import failures | `main.ts` imports run before its first statement. |
| P3-017 | Med | The typed-error registry is conventional, not enforced | Nothing stops a code being used without registering it. |
| P3-018 | Med | Unread reconciliation uses an unsafe sender-clock cursor | Pre-existing, not introduced by this work. |
| P3-019 | Med | The jukebox digest is weak and the revision can saturate | 32-bit FNV was chosen for brevity, not collision resistance. |
| P3-020 | Low | Some status messages overstate what was retained | |

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
