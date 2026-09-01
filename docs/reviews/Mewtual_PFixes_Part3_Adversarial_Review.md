# Adversarial Review: Mewtual `P-fixes` Diagnostics Work

**Repository:** `Thalpy/Mewtual`  
**Branch:** `P-fixes`  
**Reviewed head:** `f6c1be6cb3225d0ac83d921b076a4fd5d409eb26`  
**Head commit:** `Partial work`  
**Review focus:** Work in progress around Phase 3 — correlation, typed errors, event sequencing, task supervision, and the first subsystem feeds  
**Review type:** Read-only static adversarial review

---

## 1. Executive verdict

The branch is a substantial improvement over the original logging arrangement. It is not merely a debug-tab mock-up: the branch now has a canonical diagnostics crate, a bounded in-memory event hub, measured file-sink health, a separated debug-console component, typed IPC errors, an invoke wrapper, event sequence numbers, selected operation traces, and useful channel/unread/jukebox instrumentation.

However, the implementation is **not yet safe to describe as a trustworthy diagnostic pipeline**, and Phase 3 is not close enough to complete that the remaining work can be treated as ordinary subsystem migration.

The two most serious problems are privacy-contract failures:

1. `AddressValue::new` can place a raw libp2p peer ID into the supposed Safe rendering of a normal multiaddr.
2. The user-facing report says it never contains names or message text, but the report currently copies server names and arbitrary legacy tracing/webview prose, with no independent export validator.

There is also a structural problem: the new canonical event model is stronger than the adapter the actual Debug Console consumes. The adapter hard-codes Enhanced rendering and flattens away section, phase, span, structured references, and most of the trace model. Continuing to instrument more subsystems before replacing that projection will create a larger body of apparently structured diagnostics that the user-facing console cannot faithfully represent.

### Recommendation

**Do not merge the user-facing Copy/Save report flow into `main` in its current form.** Either disable those actions temporarily, label them explicitly as raw/private diagnostics, or fix the privacy blockers first.

**Do not declare Phase 3 complete.** The core requirement to carry trace IDs through actors and emitted events is not implemented. Event sequence checking and task supervision are also only partial.

**Pull the planned M4 console-on-hub rebuild forward.** Fix the canonical transport and privacy boundary before migrating the rest of the application command-by-command.

---

## 2. Scope and limitations

I inspected the current `P-fixes` source, the branch plan, and the branch-to-main comparison. Particular attention was given to:

- `docs/design-diagnostics-suite.md`
- `crates/catcoms-diagnostics`
- `crates/catcoms-log`
- `apps/desktop/src/diagnostics.ts`
- `apps/desktop/src/debug-console.ts`
- `apps/desktop/src/DebugConsole.svelte`
- `apps/desktop/src/uilog.ts`
- `apps/desktop/src/startup-log.ts`
- `apps/desktop/src/main.ts`
- `apps/desktop/src/unread.ts`
- `apps/desktop/src/jukebox.ts`
- `apps/desktop/src-tauri/src/errors.rs`
- `apps/desktop/src-tauri/src/lib.rs`
- `crates/catcoms-app/src/actor.rs`

This was a static review. There was no pull-request-triggered GitHub Actions run associated with the reviewed head. The environment could not resolve `github.com` for a local clone, so I could not compile the branch or run focused tests locally. Findings labelled as definite are direct consequences of the inspected code; performance and lifecycle findings should still be confirmed with the focused tests listed later.

The branch is ahead of its merge base but also behind current `main`, so merge/rebase conflicts may change some details. This report is pinned to the SHA above.

---

## 3. Phase 3 completion assessment

The original Phase 3 plan contains six requirements.

| Requirement | Status | Assessment |
|---|---:|---|
| Frontend invoke wrapper | **Partial** | The wrapper and trace source exist and selected calls use them. It does not solve failed diagnostic delivery, and migration is incomplete. |
| Native command wrapper | **Partial** | `Operation` provides useful start/progress/success/failure records for selected commands. It is not a universal command boundary and does not continue through the actor. |
| Event sequence numbers | **Partial** | Native emission numbers many events, but only `channel-updated` is checked by the frontend. Bare payloads cannot carry the sequence. No automatic resynchronisation follows a gap. |
| Carry traces through actors and emitted events | **Not implemented** | `AppCommand::SendMessage` and `AppEvent::ChannelUpdated` contain no trace. The trace stops at the Tauri command layer. |
| `AppError` and error-code registry | **Partial, promising** | The shape is useful and selected operations are migrated. The registry is not technically closed: callers can construct an unregistered `ErrorCode`. |
| Task supervision and actor health | **Partial** | Server actor task handles are supervised. Event forwarders and other important tasks are not; no actor-health state is exposed to the console. |

The branch plan's description of M3 as “in progress” is accurate. Comments calling the send path “instrumented end to end” are not accurate yet.

---

## 4. Severity summary

| ID | Severity | Finding |
|---|---|---|
| P3-001 | **Critical** | Safe multiaddr rendering can expose a raw peer ID |
| P3-002 | **Critical** | The exported-report privacy promise is false and unvalidated |
| P3-003 | **High** | Saved reports bypass diagnostics retention/quota and can fill the log directory |
| P3-004 | **High** | Trace correlation stops before the actor and event pipeline |
| P3-005 | **High** | The live console uses a lossy, hard-coded Enhanced projection |
| P3-006 | **High** | Asynchronous frontend diagnostic-send failures are silently uncounted |
| P3-007 | **High** | Debug Console polling can overlap and scales linearly with server count |
| P3-008 | **High** | One formatted log event has no size bound before allocation or queuing |
| P3-009 | **High** | Task supervision does not cover the paths that can make the UI silently stale |
| P3-010 | **High** | Event sequencing is incomplete and detects without repairing |
| P3-011 | **Medium** | Frontend structured events are flattened into `UI.EVENT` and lose canonical trace semantics |
| P3-012 | **Medium** | Console/export reads clone events while holding the global hub mutex |
| P3-013 | **Medium** | The canonical event format permits duplicate JSON keys, silent field loss, and forged text rows |
| P3-014 | **Medium** | Runtime capture modes cannot override the tracing layer's static filter |
| P3-015 | **Medium** | Redaction and frontend field ordering break deterministic export |
| P3-016 | **Medium** | Startup capture begins too late for static-import failures, and its “safe” copy is unsafe |
| P3-017 | **Medium** | The typed-error registry is conventional rather than enforced |
| P3-018 | **Medium** | Unread reconciliation still uses an unsafe sender-clock cursor and can log an action that never occurred |
| P3-019 | **Medium** | Jukebox instrumentation helps, but the shared revision can still be saturated and the digest is weak |
| P3-020 | **Low** | Several diagnostic status messages overstate what was retained |

---

# 5. Detailed findings

## P3-001 — Safe multiaddr rendering can expose a raw peer ID

**Severity: Critical**  
**Files:** `crates/catcoms-diagnostics/src/redact.rs`

`AddressValue::new` determines the “transport” with:

```rust
addr.rsplit('/')
    .find(|part| !part.is_empty() && part.parse::<u16>().is_err())
```

That is not a multiaddr parser. It chooses the last non-empty segment that is not a numeric port.

For a routine libp2p address such as:

```text
/ip4/203.0.113.10/tcp/22487/p2p/12D3KooW...
```

the selected transport is the peer ID. Safe rendering then becomes approximately:

```text
ip4/12D3KooW...
```

The literal IP and port are removed, but the stable peer identity is exposed. A circuit address can be worse because the final segment may name the destination peer after a relay chain.

This directly contradicts the source-side guarantee that identifiers cannot leak and makes a supposedly public-safe report a cross-session tracking artefact.

### Required fix

Parse the value as a real `libp2p::Multiaddr`. Derive a closed transport classification from protocol variants, for example:

- `tcp`
- `udp/quic-v1`
- `ws`
- `wss`
- `webrtc-direct`
- `relay`
- `unknown`

Treat every `/p2p/<id>` component as an identifier. Omit it in Safe mode or represent it as a per-session `SessionRef`.

Also bound the stored raw address length before allocating it.

### Required tests

Include Safe-mode cases for:

- direct TCP ending in `/p2p/<peer>`
- QUIC ending in `/p2p/<peer>`
- relay circuit with relay and destination peer IDs
- DNS addresses
- malformed and attacker-controlled strings
- addresses with additional protocols after the transport

Assert that no raw peer ID or address literal survives.

---

## P3-002 — The exported-report privacy promise is false and unvalidated

**Severity: Critical**  
**Files:** `apps/desktop/src/debug-console.ts`, `apps/desktop/src/DebugConsole.svelte`, `apps/desktop/src/startup-log.ts`, `crates/catcoms-log/src/lib.rs`, `apps/desktop/src-tauri/src/lib.rs`

Every copied report appends:

> This report can include your IP addresses, peer and device identifiers, and timing. It never includes message text, file contents, names or key material.

The implementation does not establish that guarantee.

Concrete counterexamples:

1. `routeLines` writes `s.name` for every server into the report.
2. The tracing compatibility bridge carries arbitrary `message` and field prose as `BridgedMessage`.
3. Frontend console warnings and error stacks are forwarded as arbitrary prose.
4. Startup reports contain full JavaScript stacks and user-agent text.
5. The regex redactor knows only selected IP/peer-like patterns; it does not remove server names, paths, URLs, document titles, arbitrary content, or values embedded in unexpected formats.
6. Native `save_diagnostics_report` accepts text composed by the webview and writes it without independently validating its privacy properties.
7. Redaction defaults to off, while the toggle is labelled only “REDACT FOR SCREENSHOTS” even though it also changes Copy and Save.

A legacy warning such as:

```text
failed to render "Private Support": C:\Users\Marisa\...
```

can therefore enter a report whose footer tells the recipient it contains no names.

This is especially serious because the planned next stage is GitHub issue submission. A false “safe” label turns a local diagnostic failure into a public disclosure.

### Required fix

Until a validator exists, change the wording immediately to something honest:

> This report may contain IP addresses, peer identifiers, server names, local file paths, URLs, error text and activity metadata. Review it before sharing.

Then implement the intended boundary:

- Generate the authoritative export natively from canonical `DiagnosticEvent` values.
- Default to Safe mode.
- Treat every `BridgedMessage` as suspect.
- Run an independent export validator after rendering.
- Block or visibly quarantine reports containing canary identifiers, local paths, bearer tokens, credentials, known server names, or raw IDs.
- Show a preview and a specific list of categories included.
- Require a deliberate second action for raw/Enhanced export.
- Keep the current browser-based GitHub flow reviewable; never auto-post a potentially sensitive report.

### Required tests

Seed the process with distinctive canaries for:

- message text
- server and channel names
- Unix and Windows usernames/paths
- peer ID, device fingerprint, group ID and CID
- invite token
- TURN credential
- vault-like secret
- URL query token

Assert none appears in Safe output. These should be property tests over every event/value variant and both text and JSON exports.

---

## P3-003 — Saved reports bypass diagnostics retention and quota

**Severity: High**  
**Files:** `apps/desktop/src-tauri/src/lib.rs`, `crates/catcoms-log/src/writer.rs`

The file writer limits:

- segment size
- segment count
- per-session bytes
- total diagnostic directory bytes

However, retention only considers files beginning with `debug_log_`.

`save_diagnostics_report` writes files named:

```text
mewtual-diagnostics-<session>-<timestamp>.txt
```

into the same directory. Each report may be up to 8 MiB. There is no report count limit, no report-byte quota, no pruning, and no rate limit. An unlocked webview can invoke the command repeatedly and fill the disk without touching the carefully designed log quotas.

This also means the Debug Console's own export feature can become the outage the bounded writer was designed to prevent.

### Required fix

Choose one:

- save reports to a user-selected destination and do not retain them automatically;
- apply a separate bounded export quota and retention policy;
- or include report files in the same total directory quota.

Additionally:

- write through a temporary file and atomically rename;
- reject concurrent report saves;
- rate-limit the native command;
- expose report-storage usage in sink health.

### Required tests

Repeatedly save maximum-sized reports and assert the directory never exceeds its configured quota. Test cleanup failures and concurrent calls.

---

## P3-004 — Trace correlation stops before the actor and event pipeline

**Severity: High**  
**Files:** `apps/desktop/src/diagnostics.ts`, `apps/desktop/src/App.svelte`, `apps/desktop/src-tauri/src/lib.rs`, `crates/catcoms-app/src/actor.rs`

The send flow allocates a frontend trace and passes it to `send_message`. Native `Operation` records command stages under that trace.

But the trace is not present in:

- `AppCommand::SendMessage`
- the actor's command handling
- `AppEvent::ChannelUpdated`
- the Tauri event payload
- the frontend event listener
- refresh/reconciliation/render completion

The current traced story therefore ends at native persistence. It cannot establish that:

- the actor processed the same operation;
- the actor emitted the relevant update;
- the Tauri bridge emitted it;
- the intended listener received it;
- the UI applied it to the correct server/channel;
- the message entered the rendered window.

That is the exact gap Phase 3 was intended to close.

The frontend-to-native structured-event bridge compounds this: it stores the supplied trace as an ordinary text field under a generic event rather than setting `DiagnosticEvent.trace`. A canonical `hub.trace(trace_id)` query therefore cannot recover the frontend stages.

### Required fix

Introduce a local operation envelope used across the internal boundary:

```text
OperationContext {
  trace,
  span,
  parent_span,
  operation,
}
```

Carry it through:

```text
frontend invoke
→ Tauri command
→ AppCommand
→ actor stage
→ AppEvent
→ emit envelope
→ frontend listener
→ reconciliation/render
```

The trace must remain local; it should not be placed on the P2P wire unless a later cooperative-trace design explicitly approves that.

Parse the frontend trace into canonical `TraceId`, rather than storing it as `fields["trace"]`.

### Required tests

Run two simultaneous sends to the same channel. Delay and reorder their actor replies/events. Assert each trace contains only its own stages and reaches a final UI-applied stage.

A minimum successful trace should resemble:

```text
UI.INVOKE.START
TAURI.COMMAND.RECEIVED
ACTOR.COMMAND.RECEIVED
ACTOR.STATE.CHANGED
TAURI.COMMAND.PERSISTED
TAURI.EVENT.EMITTED
UI.EVENT.RECEIVED
UI.REFRESH.APPLIED
UI.RENDER.SETTLED
```

---

## P3-005 — The live console uses a lossy, hard-coded Enhanced projection

**Severity: High**  
**Files:** `crates/catcoms-log/src/lib.rs`, `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src/debug-console.ts`

`catcoms-diagnostics::DiagnosticEvent` has:

- section
- phase
- operation
- trace
- span and parent span
- structured references
- mode-sensitive values
- typed fields

The current console adapter flattens it into the old `LogEvent` and hard-codes:

```rust
let mode = CaptureMode::Enhanced;
```

The projection:

- loses canonical section;
- loses phase;
- loses span/parent span;
- loses structured references as structure;
- renders mode-sensitive values as Enhanced regardless of requested mode;
- leaves the frontend to infer voice/network sections from target names and text;
- represents only a short trace string as a regular field.

The comment says section and phase “survive as ordinary fields,” but the projection does not add them.

This means much of the Phase 3 instrumentation is already being discarded before it reaches the actual user-facing tool. Migrating additional operations now will multiply adapters and text heuristics that M4 must later remove.

### Required fix

Pull M4 forward. Make the console read either:

- the canonical serialisable event schema; or
- a native, mode-aware projection that preserves every canonical field.

The current capture mode must be passed explicitly. The UI should segment by `event.section`, not by target/text guesses.

The file sink, ring and export may have different capture policies, but each output must state and honour its own policy.

### Required tests

For one canonical event, verify the UI receives and displays:

- section
- phase
- full trace
- span ancestry
- references
- duration
- attempt
- typed fields
- capture mode

Test Safe and Enhanced outputs side by side.

---

## P3-006 — Asynchronous frontend diagnostic-send failures are silently uncounted

**Severity: High**  
**Files:** `apps/desktop/src/diagnostics.ts`, `apps/desktop/src/uilog.ts`, `apps/desktop/src/App.svelte`

Both frontend batchers treat `send` as synchronous. They increment their dropped counters only when `send(...)` throws immediately.

The production integrations do this:

```ts
void invoke("log_ui_batch", { records }).catch(() => {});
```

and:

```ts
void invoke("record_ui_events", { events }).catch(() => {});
```

`invoke` returns a promise. A bridge failure is therefore caught outside the batcher and discarded. The batcher has already removed the records from its queue and still reports zero drops.

This failure is most likely when the native boundary or session is unhealthy—the exact time the diagnostic pipeline needs to account for its own loss.

The same problem affects final flush: the frontend can enqueue an asynchronous IPC call and then be torn down before it completes.

### Required fix

Make the transport explicitly asynchronous:

```ts
send(records): Promise<{ accepted: number }>
```

Maintain a bounded in-flight state and either:

- await acknowledgement before retiring the batch; or
- retire it but count a rejected acknowledgement as lost.

Do not retry indefinitely. One bounded retry is reasonable for a transient remount; otherwise record a local pipeline-health counter and surface it after the bridge returns.

Structured and prose channels should have separate or reserved budgets so a console-warning storm cannot starve operation events.

### Required tests

- rejected promise;
- delayed promise;
- native acceptance of fewer records than submitted;
- HMR remount with a batch in flight;
- process/window close with pending data;
- 10,000-record storm with bounded memory and truthful loss totals.

---

## P3-007 — Debug Console polling can overlap and scales linearly with server count

**Severity: High**  
**Files:** `apps/desktop/src/DebugConsole.svelte`

Every second, `poll()`:

1. calls `get_console_log`;
2. calls `get_member_routes` sequentially for every server;
3. samples voice state.

The interval invokes `void poll()` with no in-flight guard. If one poll takes longer than a second, the next starts anyway. With many servers, slow IPC, a busy actor, or an export happening at the same time, polls can overlap indefinitely.

The complexity is approximately `O(number of servers)` IPC calls per second. A user with 100 servers creates at least 101 native commands per second simply by opening the diagnostic tool. Full report generation simultaneously pages the event ring.

A diagnostic console that changes timing or actor load can manufacture or amplify the problem it is inspecting.

### Required fix

Add a single coalescing/in-flight guard immediately.

Then replace per-server commands with one native snapshot:

```text
get_debug_snapshot(after_seq, event_limit)
```

returning:

- event page and counters;
- all local member-route summaries;
- sink health;
- task/actor health;
- current capture mode/generation.

Use an adaptive interval, for example 2–5 seconds when idle and faster only while explicitly tracing. Pause periodic polling during full export.

### Required tests

Simulate 100 servers and make each native request take two seconds. Verify:

- at most one poll is in flight;
- pending promise count stays bounded;
- snapshots do not apply out of order;
- closing the console cancels/ignores the result;
- producer latency remains within a fixed budget.

---

## P3-008 — One formatted log event has no size bound before allocation or queuing

**Severity: High**  
**Files:** `crates/catcoms-log/src/writer.rs`

`LineWriter::write` performs:

```rust
self.buf.extend_from_slice(bytes);
```

without a per-event bound. The segment/session quota is enforced later by the worker. A single formatted event can therefore:

- allocate an arbitrarily large `Vec` on the emitting thread;
- enqueue that allocation as one queue item;
- exceed the nominal 10 MiB segment limit in one write;
- jump beyond the session quota before the quota check can stop it.

Most structured values are bounded, but the tracing ecosystem is still broad, and error/debug values can include remote- or file-derived material. The writer should not depend on every producer remaining careful forever.

### Required fix

Set a hard `MAX_EVENT_BYTES`, such as 64 KiB. `LineWriter` should retain only that prefix and mark the line truncated. It should never allocate beyond the cap.

Also bound:

- `DiagnosticEvent.target`;
- owned field names;
- raw address strings;
- aggregate rendered field bytes.

Expose an `events_truncated` counter distinct from dropped events.

### Required tests

Feed a 100 MiB tracing field and assert:

- bounded allocation;
- one truncated event;
- segment and session quotas remain true;
- the application thread does not block or panic.

---

## P3-009 — Task supervision does not cover the paths that can make the UI silently stale

**Severity: High**  
**Files:** `apps/desktop/src-tauri/src/lib.rs`

`supervise` correctly awaits a server actor `JoinHandle` and records normal exit, cancellation, and panic. That is a good start.

But the implementation currently supervises only the main server actor tasks. `forward_events` is started with `tokio::spawn` and its handle is discarded. Other discovery, route-monitor, relay, AutoNAT and background tasks also need auditing.

The event forwarder is particularly important: it can die while the actor remains healthy. The protocol state continues changing, but the webview receives nothing. That produces stale unread badges, stale jukebox queues and stale presence—the exact class of symptom the suite is meant to expose.

There is no task registry or actor-health snapshot in the Debug Console. A panic produces a log line, but the current state remains “unknown” rather than visibly degraded.

### Required fix

Create a `TaskRegistry` containing, per critical task:

- task ID;
- kind;
- server reference;
- start time;
- last heartbeat/progress;
- state (`running`, `exited`, `cancelled`, `panicked`, `stalled`);
- exit cause;
- restart policy.

Register every critical spawn. Do not blindly restart stateful MLS/CRDT actors, but surface their failure immediately. Stateless monitor tasks can have an explicit bounded restart policy.

### Required tests

Fault-inject a panic into:

- server actor;
- event forwarder;
- discovery timer;
- network monitor;
- relay watcher;
- transfer worker.

Verify one stable diagnostic finding is raised and actor/task health changes without requiring the relevant log line to remain in the ring.

---

## P3-010 — Event sequencing is incomplete and detects without repairing

**Severity: High**  
**Files:** `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src/diagnostics.ts`, `apps/desktop/src/App.svelte`

The native bridge numbers events per event name and injects `__seq` into object payloads. This is directionally correct.

Current limitations:

1. The frontend calls `noteEventSeq` only for `channel-updated`.
2. Bare payloads such as a numeric server ID cannot carry `__seq`.
3. The tracker accepts the first seen sequence as its baseline. After a remount, events missed before the first listener observation are invisible.
4. A detected gap records an anomaly but does not force an authoritative resynchronisation.
5. Sequence order is per event name, not a total event-stream order. Cross-event causality remains unknowable.
6. Successful `app.emit` does not prove a particular listener handled the event.
7. Existing `__seq` fields are overwritten rather than rejected.

### Required fix

Use a typed event envelope for every event:

```text
{
  stream_generation,
  seq,
  trace,
  payload
}
```

Install one frontend event-listener wrapper that checks every event. On a gap:

- record one stable finding;
- request the relevant authoritative snapshot;
- mark the corresponding UI projection uncertain until the snapshot is applied.

Expose the current native sequence/generation in `resume_session` so a remounted frontend can detect what it missed.

### Required tests

Inject missing, duplicate and reordered events for every event family. Verify exactly one finding and exactly one resynchronisation, without duplicate notifications.

---

## P3-011 — Frontend structured events are flattened into `UI.EVENT`

**Severity: Medium**  
**Files:** `apps/desktop/src-tauri/src/lib.rs`, `apps/desktop/src/diagnostics.ts`

The frontend sends a runtime `code`, `section`, `phase`, `trace` and fields. Native conversion creates a canonical event with the static code:

```text
UI.EVENT
```

and stores the runtime code as a text field. The trace is likewise stored as text rather than in `DiagnosticEvent.trace`.

Consequences:

- code-based findings cannot group directly on the canonical event code;
- `hub.trace()` cannot join frontend and backend stages;
- the schema is structurally different depending on which side emitted it;
- unknown runtime code strings are accepted as ordinary prose;
- fields arrive in a `HashMap`, so their output order is nondeterministic.

The security concern behind a static code is valid: the webview must not be able to inject arbitrary issue codes. Flattening all frontend events is not the only solution.

### Required fix

Create a native allowlist that maps a closed frontend code enum to native static codes. Reject or map unknown values to a specific `UI.EVENT.UNKNOWN_CODE` event.

Parse a valid trace into `TraceId`, and use an ordered list or `BTreeMap` for fields.

---

## P3-012 — Console/export reads clone events while holding the global hub mutex

**Severity: Medium**  
**Files:** `crates/catcoms-diagnostics/src/hub.rs`, `crates/catcoms-diagnostics/src/ring.rs`, `crates/catcoms-log/src/lib.rs`

`since`, `section_since` and `trace` acquire the one hub mutex, scan the ring and clone full `DiagnosticEvent` objects while still holding the lock.

An event may contain many allocated fields and bridged strings. Paging or exporting thousands of events therefore blocks every producer that needs to append an event or derive a reference. The console already polls repeatedly, and full export performs multiple pages.

The code's stated hot-path guarantee—bounded push with no scanning or allocation under the lock—describes writes but omits the impact of concurrent readers.

### Required fix

Store `Arc<DiagnosticEvent>` or another cheap immutable handle in the ring. Under the lock:

- locate the bounded range;
- clone handles only;
- release the lock.

Perform deep clone/render outside the lock.

Use `Section::index()` rather than a linear search when updating per-section counters. Consider a sequence-to-offset calculation so repeated polling does not always scan from the oldest retained event.

### Required tests

Benchmark producer latency while another thread repeatedly exports a full ring of maximum-sized events. Define a maximum p95/p99 overhead budget.

---

## P3-013 — The canonical format permits ambiguous or forged output

**Severity: Medium**  
**Files:** `crates/catcoms-diagnostics/src/event.rs`, `crates/catcoms-diagnostics/src/render.rs`, `crates/catcoms-diagnostics/src/redact.rs`

Three related issues weaken report integrity:

### Duplicate field names

`DiagnosticEvent::field` permits duplicates. JSON rendering writes `fields` as an object, producing duplicate keys. JSON consumers disagree about whether the first or last value wins.

### Silent field loss

Fields beyond 32 are silently ignored. Avoiding unbounded growth is correct, but an exported event gives no indication that fields were discarded.

### Forged text rows

`BridgedMessage` allows newlines. The text renderer appends the rendered value without escaping or prefixing continuation lines. A message can visually create a second line that resembles another event:

```text
real error
0001234 ERROR SECURITY ...
```

JSON remains escaped, but screenshots and copied plain text become ambiguous.

Owned field names and `target` are also unbounded.

### Required fix

- Reject duplicate field names, or render fields as an ordered array of `{name,value}` objects.
- Add a `fields_dropped`/`truncated` marker.
- Bound field names and target.
- Escape newlines in one-line output or prefix every continuation line with a non-event marker.
- Add a canonical report parser test to prove every rendered row maps to exactly one event.

---

## P3-014 — Runtime capture modes cannot override the tracing layer's static filter

**Severity: Medium**  
**Files:** `crates/catcoms-log/src/lib.rs`, `crates/catcoms-diagnostics/src/config.rs`

The hub has a dynamic `CaptureGate`, but tracing events first pass through a static `EnvFilter` attached to `RingLayer`.

If that layer has already rejected a trace/debug event, changing the hub to a more verbose mode cannot recover it. A future Full/Trace control may therefore say the mode is active while upstream filtering still discards the requested detail.

Changing the global mode also replaces the capture configuration and can erase per-section overrides.

### Required fix

Use either:

- a reloadable tracing filter kept in lock-step with the capture configuration; or
- a deliberately widest safe static filter followed by the cheap dynamic gate.

The UI must show both requested and effective capture. Preserve per-section overrides unless the user explicitly resets them.

---

## P3-015 — Redaction and frontend field ordering break deterministic export

**Severity: Medium**  
**Files:** `apps/desktop/src/debug-console.ts`, `apps/desktop/src/DebugConsole.svelte`, `apps/desktop/src-tauri/src/lib.rs`

Redaction aliases are assigned in encounter order and kept in one mutable map for the console's lifetime. Merely visiting a section, typing a filter, or rendering a route can mint `[ip 1]` before a later export.

The same underlying events can therefore export differently depending on UI navigation before the user presses Save. This contradicts the deterministic-export requirement.

Separately, frontend diagnostic fields cross native code in a `HashMap`, so field order can vary between runs.

The regex redactor also over-masks arbitrary long hexadecimal values as “peer” identifiers and can still miss identifiers in unsupported encodings.

### Required fix

Derive aliases deterministically from the diagnostic session salt and value domain, or pre-scan a fixed, sorted input set before rendering.

Use source-typed references instead of output regexes. Use ordered field containers.

Add a golden test that navigates/filters the console in different orders and proves the exported bytes remain identical.

---

## P3-016 — Startup capture begins too late, and its “safe” report is unsafe

**Severity: Medium**  
**Files:** `apps/desktop/src/main.ts`, `apps/desktop/src/startup-log.ts`

`beginStartupCapture()` is the first statement in the module body, but JavaScript static imports execute before the module body. Failures evaluating:

- `svelte`;
- `app.css`;
- `startup-log.ts`;
- `uilog.ts`;

can still happen before capture exists.

The dynamic import of `App.svelte` is covered, which is valuable, but the comment claiming this is ahead of every application module is too broad.

The startup failure screen also says the report is safe and contains no names, while it includes:

- full error stack;
- earlier startup error text;
- user agent.

Stacks commonly contain local paths and URLs. No native validator is available because startup failed.

### Required fix

Install a tiny dependency-free bootstrap handler from `index.html`, before the main module, or use a first script that imports nothing.

Provide two startup outputs:

- a default sanitised summary;
- an explicit “show/copy raw technical detail” section that warns about paths, URLs and identifiers.

Never state that raw stacks are content-free.

---

## P3-017 — The typed-error registry is conventional rather than enforced

**Severity: Medium**  
**Files:** `apps/desktop/src-tauri/src/errors.rs`

The registry tests validate that entries in `codes::ALL` are unique and well-formed. They do not prove every `ErrorCode` used by the bridge is in `ALL`.

`ErrorCode` has public fields, so any module can construct:

```rust
ErrorCode {
    code: "...",
    retryable: ...,
    remediation: ...
}
```

and omit it from the registry. The comment claiming CI enforces registration overstates the actual property.

`SERVER.ACTOR.UNAVAILABLE` also combines at least two states—unknown/unopened server and stopped actor—then prescribes restart for both. The diagnostic suite should distinguish those states rather than making task death the default interpretation.

### Required fix

Make `ErrorCode` construction private and define codes through a macro or generated registry that creates both constants and `ALL`. Alternatively, use an enum whose variants are inherently exhaustive.

Split unavailable states into stable causes, such as:

- `SERVER.NOT_OPEN`
- `SERVER.ACTOR.STOPPED`
- `SERVER.ACTOR.MAILBOX_CLOSED`

Keep the current preservation of specific user-facing messages; that part is good.

---

## P3-018 — Unread reconciliation still uses an unsafe sender-clock cursor

**Severity: Medium**  
**Files:** `apps/desktop/src/unread.ts`, `apps/desktop/src/App.svelte`

Durable unread reconstruction compares `latest_incoming_ts` with a stored read mark. The timestamp comes from the sender's clock.

The code correctly notices that a wildly future timestamp can poison the cursor and applies a five-minute ceiling. But the chosen response can still lose unread state:

- a genuine message from a clock more than five minutes ahead is skipped;
- two messages with the same millisecond can collapse at the cursor boundary;
- a sender clock moving backwards can produce a later message with an older timestamp;
- `latest_incoming_id` exists but is not used to break ties;
- different senders have unrelated clocks.

The diagnostics path also calls `noteUnreadDecision` before verifying that the corresponding server/channel state exists and before `markChannelUnread` actually runs. The log can say `mark_unread` even when no badge was changed due to a race or missing catalog entry.

### Required fix

Use a local replicated/causal cursor, such as:

- authoritative per-channel message count plus stable message ID;
- Automerge heads/change ID;
- a local receive sequence persisted with the read mark.

At minimum use `(timestamp, message_id)` and represent skewed messages explicitly rather than silently skipping them.

Record the unread diagnostic after the transition, with:

- state before;
- intended decision;
- state after;
- whether the transition was applied;
- authoritative head/cursor used.

### Required tests

- two incoming messages in the same millisecond;
- sender clock moves backwards;
- sender clock is one hour ahead;
- multiple senders with skew;
- lock/offline catch-up;
- channel-list event races message event;
- user changes server while refresh is in flight.

---

## P3-019 — Jukebox instrumentation helps, but revision saturation remains

**Severity: Medium**  
**Files:** `apps/desktop/src/jukebox.ts`, `apps/desktop/src/App.svelte`

The new queue digest and before/after refresh observations are useful. They can distinguish “event arrived but materialised queue did not change” from an ordinary quiet queue.

Two residual concerns remain:

1. The queue digest is 32-bit FNV-1a. It is acceptable as a display hint, but not strong enough to support an invariant or automated finding that claims two queues are equal.
2. `nextJukeSeq` saturates at `MAX_JUKE_SEQ`. A peer can submit that maximum. Every later local claim remains at the maximum, so the fingerprint tie-break can permanently favour one peer and prevent lower-order peers from reclaiming the deck.

### Required fix

For diagnostics, compare the bounded ordered entry-ID list directly or use a 128-bit keyed digest.

For transport ownership, reject implausibly far-ahead revisions rather than accepting the global maximum, and include a session/epoch nonce or another non-saturating ownership generation.

### Required tests

- peer sends `MAX_JUKE_SEQ`;
- two peers press concurrently at/near the cap;
- stale maximum frame is replayed;
- queue changes with a deliberately colliding test digest implementation;
- event says jukebox changed but authoritative queue is unchanged.

---

## P3-020 — Some status copy overstates retention

**Severity: Low**  
**Files:** `apps/desktop/src/debug-console.ts`

When the in-memory ring drops old entries, the console says:

> The debug log file keeps everything.

That is not guaranteed:

- file logging may be disabled;
- file initialisation may have failed;
- its filter is narrower than the ring;
- its queue may have dropped events;
- it rotates and has session/directory quotas;
- a particular event may have been excluded before the file layer.

This is exactly the kind of reassuring but false statement the logger-health redesign was intended to remove.

### Required fix

Use measured wording:

> Older in-memory entries were dropped. The file log may contain additional history; check its actual sink state, dropped count and capture filter.

Show sink health directly in the Debug Console.

---

# 6. What is good and should be retained

An adversarial review should not erase the parts that are structurally sound.

## 6.1 File-sink health is now evidence-based

The distinction between requested logging and actual sink state is correct. Reporting:

- opened path;
- last successful write;
- queue depth/high-water;
- dropped events;
- written bytes;
- quota state;
- last error;

is a major improvement over a boolean preference.

The bounded asynchronous queue, non-blocking producer path, segment rotation and explicit quotas are the right architecture once a per-event size cap is added.

## 6.2 The canonical event model is the right direction

The separation of:

- sequence;
- wall time;
- monotonic time;
- phase;
- operation;
- trace/span;
- session-scoped references;
- typed values;

is exactly what Mewtual needs. Per-session keyed references are considerably better than regex masking after rendering.

The main problem is not this model; it is that the live console does not consume it faithfully yet.

## 6.3 The frontend logger lifecycle work is materially better

Using a realm-level `Symbol.for` guard, restoring console functions, removing listeners, counting repeated lines, batching IPC and bounding the queue addresses real HMR/remount failure modes.

The asynchronous acknowledgement gap is fixable without discarding this design.

## 6.4 Typed IPC errors preserve the useful user message

The implementation correctly keeps the specific message while adding code, trace, retryability and remediation. That avoids replacing actionable errors with generic support prose.

## 6.5 Channel-change classification is valuable

Separating:

- appended messages;
- changed messages;
- topic;
- jukebox;

prevents reactions and queue edits from masquerading as new chat. The unread-decision explanation and jukebox before/after observations are high-value diagnostic feeds; they need a stronger authoritative cursor and trace envelope.

## 6.6 Emit failures are no longer discarded

Recording a failed Tauri event emit is necessary. Adding a sequence is also the correct foundation; it needs to become a universal envelope plus recovery policy.

## 6.7 Extracting the Debug Console from `App.svelte` is correct

The debug suite should not live inside an approximately 20,000-line component. The separate Svelte component and pure helper module are much easier to test and review.

---

# 7. Recommended patch order

The current plan migrates subsystems while M4 is deferred. I recommend changing the order.

## Patch 1 — Stop unsafe sharing

Before more subsystem work:

1. Fix multiaddr classification.
2. Replace the false privacy wording.
3. Default exports to Safe.
4. Disable or prominently label raw Copy/Save until validation exists.
5. Apply quota/retention to saved reports.
6. Add privacy canary tests.

## Patch 2 — Make the canonical event the actual console contract

1. Replace `LogEvent` projection with a serialisable canonical event.
2. Remove hard-coded Enhanced mode.
3. Segment by `Section`.
4. Preserve phase, span, refs and full trace.
5. Give bridged messages an explicit warning/count.
6. Expose sink/capture/task health in the same snapshot.

This effectively pulls M4 ahead of the rest of M3.

## Patch 3 — Complete one genuinely end-to-end operation

Use `send_message` as the reference implementation:

1. frontend start;
2. Tauri receive;
3. actor receive;
4. state mutation;
5. persistence;
6. event emission;
7. frontend receipt;
8. authoritative refresh;
9. UI apply/render.

Do not call it end-to-end until a test proves every stage shares one canonical trace.

## Patch 4 — Make event delivery self-healing

1. Universal event envelope.
2. Sequence/generation on every event.
3. One frontend listener wrapper.
4. Gap finding.
5. Authoritative snapshot/resync.
6. Remount baseline.

## Patch 5 — Supervise every critical task

Add task registry/health before instrumenting additional background subsystems. Otherwise the suite will record symptoms but still miss the task that stopped producing events.

## Patch 6 — Fix diagnostic-pipeline backpressure

1. Promise-aware frontend batching.
2. Per-event writer cap.
3. Non-overlapping bulk console poll.
4. Hub read path that clones outside the lock.
5. Performance and loss counters.

## Patch 7 — Resume subsystem migration

Only then continue:

1. join/reachability;
2. sync/catch-up;
3. file/media;
4. voice;
5. vault/pairing;
6. remaining documents/settings.

Every migrated subsystem should use the same operation/event/task primitives rather than inventing local instrumentation.

---

# 8. Focused test suite required before merge

## 8.1 Privacy tests

- Safe multiaddr with trailing peer IDs and relay circuits.
- Canary corpus across every SafeValue and bridged path.
- Safe text and JSON exports contain no canary.
- Startup sanitisation.
- Server names do not appear unless explicitly permitted.
- Raw export requires explicit consent.
- Repeated export stays inside disk quota.

## 8.2 Correlation tests

- Two simultaneous sends to one channel.
- Same operation retried twice.
- Actor reply delayed past server/channel switch.
- Persistence fails after actor acceptance.
- Event emit fails after persistence.
- UI refresh resolves after user navigates away.
- Every stage retains one trace; no stage appears under both traces.

## 8.3 Event-sequence tests

- one dropped event;
- duplicate event;
- reordered events;
- remount between sequences;
- bare numeric payload converted to envelope;
- gap causes one snapshot refresh and one finding.

## 8.4 Task tests

Fault-inject each critical task to:

- return normally;
- be cancelled;
- panic;
- stop making progress.

Verify task health, stable code, and user notification.

## 8.5 Backpressure/performance tests

- 100 servers while Debug Console is open.
- Native poll takes longer than interval.
- Full export while events arrive at high rate.
- 8,192 maximum-sized held events.
- 100 MiB tracing field.
- rejected/slow frontend IPC sink.
- file queue full.
- report directory at quota.

## 8.6 HMR/startup tests

- repeated App mount/unmount;
- logger module HMR replacement;
- structured-recorder batch pending during unmount;
- failure in `App.svelte` dynamic import;
- failure in a static bootstrap import;
- startup report with Windows and Unix paths.

## 8.7 Unread/jukebox tests

- same-timestamp messages;
- sender clock backwards/future;
- locked/offline catch-up;
- channel catalog race;
- jukebox maximum revision;
- event/queue disagreement;
- stale room refresh resolves after switching calls.

---

# 9. Merge gate

I would use the following gate.

## May continue on the branch

- additional tests;
- fixes to the diagnostics foundation;
- M4 console rebuild;
- one end-to-end reference operation;
- task-health registry.

## Should not merge as a user-facing sharing feature

Until all of these are true:

- no raw identifier can escape Safe mode;
- report wording matches reality;
- Safe export is independently validated;
- saved reports are quota-bound;
- the console honours the selected capture mode;
- asynchronous loss is measured;
- opening the console cannot create overlapping polling load.

## Phase 3 cannot be marked complete

Until:

- traces cross actor commands and actor/Tauri events;
- all emitted events use and check one envelope;
- detected gaps trigger recovery;
- critical tasks are supervised;
- actor/task health is visible;
- the reference end-to-end tests pass.

---

# 10. Final assessment

The branch has moved Mewtual from “almost no usable evidence” to a credible diagnostics architecture. The direction is good, and several individual fixes are strong.

The immediate danger is that the UI and comments now sound more trustworthy than the current implementation actually is. In diagnostics, that is not a cosmetic documentation issue. A false privacy promise can disclose data; a false drop count can hide the only reproduction; a false end-to-end trace can send debugging effort to the wrong layer; and a console that overlaps polls can create the latency being investigated.

The best next move is not to instrument more commands. It is to make the canonical event, privacy boundary, event envelope and task-health model real all the way through the user-facing console. Once that foundation is trustworthy, the subsystem migration already started on channels and the jukebox becomes much more valuable rather than more technical debt.
