# Adversarial Debugging Review and Debug Suite Design for Mewtual

**Repository:** `Thalpy/Mewtual`  
**Reviewed branch:** `main`  
**Reviewed commit:** `406884b1789e70a740fb3b0abda25dc1549072d5`  
**Commit date:** 23 August 2026  
**Review date:** 23 August 2026  
**Review type:** Read-only static adversarial review, focused on observability, diagnostics, failure communication, and bug-report generation.

> This review is pinned to the commit above so that findings do not silently drift as `main` changes. It does not claim that every runtime path has been reproduced. No source files were modified and no full build or test suite was run. Concrete code defects below are based directly on the reviewed source; proposed detection rules and architecture still need implementation and runtime verification.

---

## 1. Executive verdict

Mewtual is not completely devoid of logging. It already has several useful fragments:

- Rust `tracing` events and a `catcoms-log` subscriber;
- a timestamped desktop debug file;
- forwarding of frontend `console.warn`, `console.error`, uncaught exceptions, and unhandled rejections;
- join-attempt outcomes and a connectivity report;
- storage-health diagnostics;
- a safe browser-based GitHub issue composer.

The problem is that these pieces do **not form a trustworthy diagnostic system**.

The app is presently blind for a more dangerous reason than “not enough log lines”: observations are fragmented, inconsistently structured, frequently context-free, sometimes silently discarded, and not correlated across the frontend, Tauri bridge, actors, storage, replication, and network layers. The current logger can also misreport its own health, duplicate itself after frontend remounts, panic while truncating Unicode, grow files without a session quota, and accept arbitrary frontend strings despite documentation asserting that content is not logged.

The highest-priority conclusion is:

> **Do not begin by adding a large debug textarea to `App.svelte`.**  
> First create a native diagnostic event contract and a bounded `DiagnosticHub`. The debug tab should be a viewer and controller for that system, not another independent logger.

The recommended product is a dedicated, lazy-loaded **Debug & Diagnostics** tab with:

1. runtime-selectable capture modes;
2. per-section capture controls;
3. a live, virtualised, notepad-style event viewer;
4. deterministic issue detection and highlighting;
5. a separate editable notes pad;
6. self-tests and targeted probes;
7. a segmented, privacy-reviewed export;
8. automatic notification and issue preparation;
9. explicit user approval before anything is posted publicly to GitHub.

Unattended automatic GitHub submission is specifically **not recommended**. Mewtual is a privacy-focused peer-to-peer app, and even content-free diagnostics can expose IP addresses, peer relationships, stable identifiers, timing, and usage patterns. “Automatic” should mean **detect, aggregate, notify, redact, prepare, fingerprint, and open the prefilled issue**. The final public submission should remain an explicit action.

---

## 2. What exists now

### 2.1 Native file logging

[`crates/catcoms-log/src/lib.rs`](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/crates/catcoms-log/src/lib.rs) installs a `tracing_subscriber` console layer and, when enabled, a timestamped `debug_log_YYYYmmdd_HHMMSS.txt` file.

The GUI file filter deliberately includes product/protocol layers at `debug` while leaving transport crates at `info` to reduce identifying network churn. That is a sensible privacy instinct. The module documentation also explicitly warns that a log can contain addresses, peer IDs, fingerprints, group IDs, invite nonce prefixes, CIDs, and activity metadata.

However, logger initialisation errors are discarded, rotation is not bounded within a session, and the returned state does not prove that a subscriber or file sink is actually working.

### 2.2 Tauri logging controls

The logging controls in [`apps/desktop/src-tauri/src/lib.rs`](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src-tauri/src/lib.rs) currently use an **opt-out** file. In this alpha build:

- no opt-out file means debug logging is enabled;
- changing the setting applies only after restart;
- `LogState.active` is assigned from the preference, not from verified sink health;
- the UI locates the “current” log by selecting the newest matching file in the directory.

The same bridge also contains useful connectivity, join, storage, transfer, pairing, and session state, but these are separate snapshots rather than one diagnostic timeline.

### 2.3 Frontend console forwarding

[`apps/desktop/src/uilog.ts`](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/uilog.ts) wraps frontend warnings and errors, serialises arguments, keeps `Error` stacks, performs a small consecutive-line dedupe, and invokes the native `log_ui` command.

This is useful, especially for WebRTC and webview failures that would otherwise disappear into developer tools. It is nevertheless string-based, does not assign an operation or section, and serialises arbitrary objects without a privacy schema.

### 2.4 Join and connectivity diagnostics

[`apps/desktop/src/joinlog.ts`](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/joinlog.ts) is the strongest existing diagnostic pattern:

- backend outcomes use stable IDs;
- frontend copy explains the outcome and next action;
- the join history has timestamps and bounded peer/invite prefixes;
- connectivity records attempted steps and evidence;
- the output can be rendered as paste-friendly text.

This design should be generalised across the whole application.

### 2.5 Feedback and issue creation

[`apps/desktop/src/FeedbackOverlay.svelte`](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/FeedbackOverlay.svelte) and [`apps/desktop/src/feedback.ts`](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/feedback.ts) safely generate a bounded, prefilled GitHub issue URL. The native command validates the exact tracker destination before opening the system browser. Nothing is posted without the user submitting it.

This is the correct security boundary to extend. It should not be replaced by a hidden uploader.

---

## 3. Severity model

| Severity | Meaning |
|---|---|
| **Critical** | Can broadly destroy diagnostic evidence, expose secrets/content, or make the application materially unsafe without an unusual precondition. |
| **High** | Can make common debugging sessions misleading or unusable, destabilise the app, leak sensitive metadata, or conceal important failures. |
| **Medium** | Materially slows diagnosis, creates blind spots, or makes evidence ambiguous, but usually has a workaround. |
| **Low** | Quality or maintainability weakness with limited immediate impact. |

Confidence is separate from severity:

- **Confirmed:** direct defect visible in the reviewed source.
- **High confidence:** architecture clearly creates the stated blind spot.
- **Needs runtime confirmation:** plausible source-level risk whose observed impact depends on execution.

---

## 4. Adversarial findings

### Summary

| ID | Severity | Confidence | Finding |
|---|---:|---:|---|
| DBG-001 | High | Confirmed | Debug logging can be reported as active when no functioning sink exists. |
| DBG-002 | High | Confirmed | Frontend logging hooks leak and multiply across HMR/remounts. |
| DBG-003 | High | Confirmed | Native truncation can panic on a non-ASCII UTF-8 boundary. |
| DBG-004 | High | Confirmed | Default-on, unbounded session logging has no aggregate rate or disk quota. |
| DBG-005 | High | High confidence | The documented “no content” privacy guarantee is not mechanically enforced. |
| DBG-006 | High | High confidence | Diagnostics are split across incompatible channels with no canonical record. |
| DBG-007 | High | High confidence | There is no cross-layer trace or request correlation. |
| DBG-008 | High | Confirmed in reviewed paths | Operational failures are deliberately discarded or reduced to unactionable strings. |
| DBG-009 | Medium | Confirmed | Frontend startup failures can occur before the frontend logger is installed. |
| DBG-010 | Medium | Confirmed | Important diagnostics are latest-only snapshots, not histories. |
| DBG-011 | Medium | Confirmed | Dedupe hides repetition frequency and therefore retry storms. |
| DBG-012 | High | High confidence | Stringly typed errors erase category, retryability, cause, and remediation. |
| DBG-013 | Medium | Confirmed in reviewed paths | Spawned actor task handles are discarded, preventing supervision from the desktop shell. |
| DBG-014 | Medium | Confirmed | The two largest desktop files are the wrong place to embed another subsystem. |
| DBG-015 | Medium | Confirmed | The issue composer cannot include diagnostic evidence, fingerprints, or a redaction audit. |
| DBG-016 | Medium | High confidence | The current debug stream can itself become a denial-of-service surface. |

---

### DBG-001 — Debug logging can claim to be active when it is not

**Evidence**

In `catcoms-log`:

- `create_dir_all` is assigned to `_`;
- `try_init()` is assigned to `_`;
- the code prints a log path and returns a `LogGuard` even when the subscriber could not be installed.

In the Tauri setup:

- `active` is set to `enabled`;
- it is not derived from successful writer creation or subscriber registration;
- `newest_log_file` may return a stale file from an earlier run.

**Impact**

A user can reproduce a difficult issue, see “logging active,” and later discover that no current session was captured. This is worse than visibly having no logger because it consumes the only reproduction while giving false confidence.

**Required correction**

Replace the current boolean with an explicit state:

```text
desired: true
state: active | degraded | failed | stopped
session_id: ...
file: ...
started_at: ...
last_successful_write_at: ...
events_written: ...
bytes_written: ...
events_dropped: ...
queue_depth: ...
last_error: ...
```

`active` must only become true after:

1. the target directory was resolved;
2. the file was opened;
3. the subscriber/layer was attached;
4. a session-start event was accepted;
5. preferably, the file can be statted or read back.

Logger initialisation should return `Result<InitializedLogger, DiagnosticInitError>` rather than an apparently healthy guard.

---

### DBG-002 — Frontend logging hooks multiply across remounts

**Evidence**

`installUiLogging` installs:

- wrapped `console.error`;
- wrapped `console.warn`;
- a window `error` listener;
- a window `unhandledrejection` listener.

Its returned cleanup restores only the console methods. It does not remove the two window listeners.

`App.svelte` calls `installUiLogging` during `onMount` but does not store or invoke the returned cleanup. The application explicitly supports frontend F5/HMR remounts while the native process remains alive.

**Impact**

After each remount:

- another console wrapper can be layered over the previous wrapper;
- another pair of global exception listeners remains;
- one event can be forwarded multiple times;
- the diagnostic log becomes misleading;
- IPC and disk load can multiply;
- a retry loop may be misdiagnosed because the logger itself fabricated duplicates.

**Required correction**

- Add `removeEventListener` to the logger scope.
- Return a cleanup that removes both global listeners and restores the console.
- Store the cleanup in `App.svelte` and call it during unmount.
- Add an idempotency guard, such as a symbol on `globalThis`, so accidental double installation cannot occur.
- Test at least ten install/uninstall cycles and assert exactly one emitted record per source event.

---

### DBG-003 — Unicode can panic native log truncation

**Evidence**

The frontend limits lines by JavaScript string characters. The native command then limits by UTF-8 bytes:

```rust
if text.len() > MAX_UI_LOG_BYTES {
    text.truncate(MAX_UI_LOG_BYTES);
}
```

Rust’s `String::truncate` requires the byte index to be a valid UTF-8 character boundary. A line containing emoji or other multibyte text can be longer than 2,000 bytes while byte 2,000 falls inside a character.

**Impact**

The act of recording an error can panic the logging command or task. Depending on panic configuration and runtime boundaries, this can lose the original evidence and may destabilise more of the process.

**Required correction**

Use a boundary-safe helper:

```rust
fn truncate_utf8_bytes(text: &mut String, max: usize) {
    if text.len() <= max {
        return;
    }
    let end = text
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|i| *i <= max)
        .last()
        .unwrap_or(0);
    text.truncate(end);
}
```

A better fix is to stop passing opaque strings and accept a bounded structured event with validated string fields.

Tests must include emoji, combining marks, CJK text, malformed boundary targets, and strings where the first character alone exceeds a tiny test cap.

---

### DBG-004 — Logging is default-on and not bounded as a session

**Evidence**

- Packaged alpha logging is opt-out rather than opt-in.
- The writer uses a timestamped `rolling::never` file.
- There is no reviewed per-file limit, total directory quota, retention count, session timeout, or aggregate `log_ui` rate limit.
- A frontend line is bounded individually, but a loop can emit unbounded numbers of lines.
- The non-blocking writer’s loss/backpressure state is not exposed.

**Impact**

A fault, hostile peer stimulus, compromised webview, or accidental retry loop can:

- consume disk space;
- consume IPC and formatting time;
- displace the useful lines with noise;
- retain sensitive metadata for longer than expected;
- silently drop records under writer pressure.

**Required correction**

Recommended packaged-build defaults:

| Build/profile | Default |
|---|---|
| Developer build | Safe capture enabled, capped. |
| Packaged alpha | First-run consent; otherwise off. |
| Packaged general release | Off until started by the user or a crash-recovery prompt. |
| Enhanced/full trace | Never persistent across restart; automatically expires. |

Recommended initial limits, configurable later:

- 10 MiB per segment;
- 5 retained segments;
- 50 MiB hard session cap;
- 100 MiB hard diagnostics-directory cap;
- warning at 70% of the cap;
- critical finding and automatic stop at 100%;
- bounded in-memory queue;
- per-source token bucket;
- visible suppressed/dropped counters;
- repeat aggregation rather than silent deletion.

The exact limits should be benchmarked, but **having explicit limits is non-negotiable**.

---

### DBG-005 — “No content in logs” is policy, not enforcement

**Evidence**

The native logger documentation says message text, file contents, wiki bodies, display names, passphrases, and key material are not logged.

The frontend logger can nevertheless:

- forward arbitrary console strings;
- `JSON.stringify` arbitrary objects;
- retain exception stacks;
- include URLs and local paths;
- forward developer-written warnings whose arguments may later contain content.

This does not prove that current code is already logging message bodies. It proves that the architecture cannot guarantee that it does not.

**Impact**

A future `console.warn("send failed", message)` can silently violate the documented guarantee. A public GitHub issue prepared from such a log could expose private content.

**Required correction**

Use a typed allowlist rather than a string scrubber:

```text
Public       -> app version, stable error code
Safe         -> count, duration, outcome, bounded enum
Sensitive    -> IP, peer ID, group ID, CID, path
Secret       -> key, credential, invite bearer, auth token
Content      -> message/wiki/status text, filenames where sensitive, media payload
```

Rules:

- `Secret` and `Content` fields are unrepresentable in the diagnostic event API.
- `Sensitive` fields are omitted or session-HMACed in Safe mode.
- Raw addresses and paths require Enhanced or Full mode.
- Export applies a second independent scanner and records what was removed.
- A failed redaction validation blocks export rather than “best-effort” sharing.

---

### DBG-006 — There is no canonical diagnostic record

Today the app communicates failures through several unrelated mechanisms:

- structured Rust `tracing`;
- raw `eprintln!`;
- `Result<T, String>` returned over Tauri;
- frontend `console` forwarding;
- user-visible `error = String(e)`;
- ignored `catch {}` blocks;
- in-memory connectivity and storage snapshots;
- emitted Tauri events;
- ad hoc plain-text join logs.

These records have different fields, lifetimes, clocks, naming, and privacy rules.

**Impact**

An investigator cannot reliably answer:

- Did the frontend command reach Rust?
- Did Rust enqueue the actor operation?
- Did the actor remain alive?
- Was the network request sent?
- Was a response received?
- Did state mutate?
- Did persistence complete?
- Was the Tauri event emitted?
- Did the frontend receive and apply it?
- Did the UI decide to suppress the visible indicator?

**Required correction**

Create one canonical `DiagnosticEvent` schema and ingest all existing sources into a native `DiagnosticHub`.

---

### DBG-007 — No end-to-end trace IDs

Mewtual is highly asynchronous. A typical operation crosses:

```text
user action
  -> Svelte handler
  -> Tauri invoke
  -> bridge command
  -> actor mailbox
  -> sync / MLS / CRDT / storage / network
  -> actor event
  -> Tauri event
  -> frontend listener
  -> state reconciliation
  -> rendered result
```

Current logs do not carry one identifier through those stages.

**Impact**

Concurrent sends, server switches, event bursts, reconnects, and retries become indistinguishable. This is particularly damaging for:

- unread indicators;
- cross-server notifications;
- jukebox state;
- join/reply timing;
- file transfer stalls;
- WebRTC negotiation;
- stale async responses applying after view changes.

**Required correction**

Every user-visible operation needs:

- `trace_id`: the whole operation;
- `span_id` and `parent_span_id`: one stage;
- `request_id`: one IPC command;
- `server_ref`, `channel_ref`, and `peer_ref`: privacy-safe session references;
- `attempt`: retry number;
- `phase`: start, progress, success, failure, cancel, timeout;
- `duration_ms`;
- `outcome_code`.

The same trace must be attached to the resulting Tauri event and frontend state application.

---

### DBG-008 — Important failures are discarded

Examples in reviewed paths include:

- `eprintln!` followed by degraded continuation during unlock, pairing-ledger restore, server restore, persistence, and invalid rendezvous handling;
- `let _ = app.emit(...)` for progress or event delivery;
- frontend `.catch(() => {})` and empty `catch` blocks;
- failed logging IPC intentionally swallowed with no local dropped counter.

Some best-effort failures should not interrupt the user. That does **not** mean they should be invisible.

**Impact**

The application can continue in a degraded state without preserving:

- what failed;
- which server or operation was affected;
- whether the failure was transient;
- whether a retry is pending;
- whether the user-visible state is now stale.

**Required correction**

Introduce explicit outcomes:

```text
ignored_by_policy
degraded_continue
retry_scheduled
cancelled_by_session_lock
event_delivery_failed
persistence_failed_after_memory_commit
optional_capability_unavailable
```

A best-effort failure should produce a bounded diagnostic event even when it produces no user-facing error.

---

### DBG-009 — Frontend startup has a blind window

[`apps/desktop/src/main.ts`](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/main.ts) dynamically imports `App.svelte` and mounts it. Frontend logging is installed only inside `App.svelte`’s `onMount`.

A module-evaluation error, failed dynamic import, or mount failure occurs before that hook.

**Required correction**

Install a minimal bootstrap logger in `main.ts` before importing the application. It should:

- capture startup `error` and `unhandledrejection`;
- buffer a small number of safe records in memory;
- forward them once the Tauri bridge is available;
- render a minimal startup-failure screen with a diagnostic ID;
- hand over cleanly to the full UI logger without double installation.

---

### DBG-010 — Latest-only diagnostics erase chronology

The desktop `AppState.diag` intentionally stores the most recent found/join attempt. Connectivity and storage surfaces are point-in-time views.

That is appropriate for ordinary product UI, but inadequate for debugging.

**Impact**

Late-arriving mapping, AutoNAT, relay, network-change, or retry events overwrite context. Investigators cannot distinguish:

- a route that never existed;
- a route that existed and was withdrawn;
- an old result arriving after a new attempt;
- repeated failure with no progress;
- a short-lived success followed by regression.

**Required correction**

Keep ordinary snapshots for UI, but also emit every transition into a bounded chronological event stream.

---

### DBG-011 — Dedupe suppresses evidence without counting it

The frontend deduper drops identical consecutive lines inside a two-second window. It does not report how many were suppressed.

**Impact**

“One timeout” and “4,000 timeouts in two seconds” become the same evidence, even though the latter identifies a retry storm and likely performance problem.

**Required correction**

Aggregate repeats:

```text
first_seen
last_seen
occurrences
suppressed_occurrences
max_rate_per_second
```

When the window closes, emit one summary event such as:

```text
UI.CONSOLE.REPEATED: "ICE candidate rejected" repeated 312 times in 2.0s
```

---

### DBG-012 — String errors remove the information debugging needs

The bridge widely returns `Result<_, String>`, and the frontend often displays `String(e)`.

**Lost information includes:**

- stable error code;
- source subsystem;
- operation;
- retryability;
- cause chain;
- whether state changed before failure;
- whether user action can fix it;
- whether the error is safe to display;
- trace ID;
- severity;
- privacy classification.

**Required correction**

Introduce a serialisable error envelope:

```rust
struct AppError {
    code: ErrorCode,
    safe_message: String,
    trace_id: TraceId,
    retryable: bool,
    remediation: Option<RemediationId>,
    details: BTreeMap<String, SafeValue>,
}
```

Internal source chains stay local. The UI receives a safe message and diagnostic ID. Existing commands can migrate gradually.

---

### DBG-013 — Actor task supervision is absent from the reviewed desktop paths

Reviewed spawn sites destructure the actor task as `_task`, discarding the handle.

**Impact**

The desktop shell can retain an actor handle while the underlying task has exited. Subsequent symptoms may appear as stale state, missing events, or generic “actor stopped” errors with no preserved panic/exit cause.

**Required correction**

Register every long-lived task with a supervisor:

```text
task_id
task_kind
server_ref
started_at
last_heartbeat
exit_kind
panic_payload_safe
restart_policy
restart_count
```

A task exit must produce a high-severity finding and invalidate relevant health indicators.

---

### DBG-014 — The desktop monoliths are hostile to consistent instrumentation

`App.svelte` and `apps/desktop/src-tauri/src/lib.rs` are already exceptionally large. Adding the debug UI, rule engine, exporter, and command wrappers directly into them would increase coupling and make the diagnostic suite hard to test independently.

**Required correction**

The debug suite should be split into dedicated modules and loaded lazily. `App.svelte` should own only routing/open-state and pass no private application objects into the debug component.

---

### DBG-015 — Feedback cannot carry diagnostic evidence

The existing feedback flow includes:

- report kind;
- title;
- description;
- app version;
- browser user agent.

It does not include:

- detected findings;
- a trace ID;
- structured environment;
- reproduction markers;
- dropped-event counts;
- a redaction report;
- a stable issue fingerprint;
- a safe log excerpt;
- a diagnostic bundle.

**Required correction**

Extend the existing safe browser-opening mechanism rather than creating a separate reporter.

---

### DBG-016 — Diagnostics can become a denial-of-service surface

A logging system receives input from paths that can be influenced by:

- malformed remote traffic;
- a malicious peer;
- repeated network timeouts;
- buggy render/reconnect loops;
- a compromised webview;
- hostile object serialisation.

Without aggregate quotas, rate limits, bounded field sizes, and static finding text, a peer can indirectly induce:

- excessive disk writes;
- notification spam;
- huge issue bodies;
- expensive formatting;
- repeated rule evaluation;
- misleading issue titles containing attacker-controlled text.

**Required correction**

- Rate-limit by source, section, code, and peer reference.
- Bound every field and collection.
- Use static finding templates.
- Treat all peer-provided values as untrusted evidence.
- Never automatically file an issue solely because a remote peer triggered a condition.
- Quarantine malformed events and increment a counter rather than recursively logging the logger failure.

---

## 5. Design principles for the replacement

1. **The diagnostic system must be more reliable than the code it observes.**
2. **A disabled debug mode must actually be disabled.**
3. **“Active” must describe a verified sink, not a preference.**
4. **One canonical structured event precedes every presentation format.**
5. **The chronological timeline is primary; section views are indexes over it.**
6. **No message/file/wiki content enters Safe diagnostics.**
7. **Every asynchronous operation carries a trace ID.**
8. **Every ignored failure increments a visible counter or emits a bounded event.**
9. **Repeated events aggregate; they do not disappear.**
10. **Diagnostic overhead is bounded and measured.**
11. **Remote input cannot control finding prose or issue submission.**
12. **Automatic issue detection never means silent public upload.**
13. **The debug tab is separately testable and lazy-loaded.**
14. **Exports are deterministic, versioned, and self-describing.**
15. **The logger reports its own drops, queue pressure, and health.**

---

## 6. Proposed Debug & Diagnostics tab

### 6.1 Navigation

Add an application-level route:

```text
Settings
└── Debug & Diagnostics
```

Also expose a direct shortcut in error banners:

```text
Open diagnostics for trace 3f8a-61c2
```

The component should be dynamically imported, like the existing optional overlays, so ordinary chat startup does not pay for a large viewer.

### 6.2 Recommended layout

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ DEBUG & DIAGNOSTICS                 SAFE CAPTURE · ACTIVE · 03:42 · 1.8 MiB │
│ Sink healthy · 0 dropped · 0 redaction failures              [Stop capture] │
├───────────────┬──────────────────────────────────────────────────────────────┤
│ OVERVIEW      │ Health summary                                               │
│ LIVE LOG      │ 2 high · 4 warning · 7 informational findings                │
│ FINDINGS      │ [HIGH] SYNC.CHANNEL.EVENT_GAP                                │
│ CHECKS        │ channel-updated sequence 1198 → 1200; event 1199 absent       │
│ NOTES         │                                                              │
│ EXPORT        │ Recent traces                                                │
│ CONFIGURATION │ 7f2c send_message · failed · 842 ms                           │
│               │ 64aa jukebox_refresh · completed · 131 ms                     │
│ Sections      │                                                              │
│ [x] UI        │ [Run safe checks] [Mark reproduction start] [Prepare issue]   │
│ [x] IPC       │                                                              │
│ [x] Sync      │                                                              │
│ [ ] Net trace │                                                              │
└───────────────┴──────────────────────────────────────────────────────────────┘
```

### 6.3 Overview

Show:

- capture mode and actual sink state;
- capture start time and automatic expiry;
- file/memory usage;
- event count;
- dropped and suppressed count;
- queue depth/high-water mark;
- current build and platform;
- active server count, actor count, and task health;
- latest high/critical findings;
- “safe to share” status;
- latest export path;
- whether startup events were captured.

A single visible distinction is essential:

```text
Preference: Enabled
Actual state: Failed — permission denied opening diagnostics directory
```

### 6.4 Live log: notepad-style, but not a giant `<textarea>`

The live output should look and behave like a notepad:

- monospace;
- line numbers;
- selectable text;
- pause/follow;
- copy selected;
- search;
- severity and section filters;
- trace filter;
- time-range filter;
- jump to first/previous/next issue;
- expandable structured fields;
- plain-text rendering mode.

It should **not** be implemented as one growing textarea or one enormous string. That would eventually freeze the webview and duplicate the entire log in memory. Use a virtualised list over paged native queries.

Suggested line format:

```text
0004812  2026-08-23T18:47:12.331Z  +12.443s  WARN  JOIN
trace=7f2c req=91  JOIN.ROUTES.EXHAUSTED  direct=4 relay=0 rendezvous=1 duration=60123ms
```

Issue lines should be visually distinct without relying only on colour:

```text
!!! HIGH  SYNC.CHANNEL.EVENT_GAP
Expected event sequence 1199; received 1200. Frontend state may be stale.
Evidence: events 4810, 4811, 4812
```

### 6.5 Findings

Each finding card should show:

- severity;
- confidence;
- stable rule ID;
- human summary;
- first and last occurrence;
- count and rate;
- affected section;
- trace IDs;
- evidence event links;
- likely causes, labelled as hypotheses;
- recommended checks;
- whether the condition recovered;
- suppress/mute controls;
- “Prepare GitHub issue”.

### 6.6 Checks

Checks are explicit probes, separate from passive logging:

- logger self-test;
- diagnostics directory write/read/rename/delete;
- disk-space and quota;
- actor heartbeats;
- command/event loopback;
- vault accessibility without reading content;
- storage index consistency;
- network listener inventory;
- reachability snapshot;
- rendezvous/relay configuration validation;
- channel event sequence consistency;
- unread-state invariants;
- jukebox document and playback-state checks;
- file manifest/chunk health;
- WebRTC device and ICE configuration;
- update endpoint presence;
- platform dependencies such as WebKitGTK/WebView2 and audio backend.

Each probe declares:

- what it touches;
- whether it sends network traffic;
- estimated cost;
- fields it will collect;
- cancellation support.

No network probe should run merely because the tab was opened.

### 6.7 Notes

Provide a separate editable notes pad:

```text
Reproduction:
1. Alice sends a message in #general.
2. Rail badge does not appear.
3. Switching server makes it appear.
```

Buttons:

- Mark reproduction start;
- Mark step;
- Mark observed failure;
- Mark recovery;
- insert current timestamp/trace;
- copy;
- include/exclude from export.

User notes are content and must never be automatically included without an explicit checkbox.

### 6.8 Export & GitHub

The export view should show:

- selected time range;
- selected sections;
- selected traces;
- privacy mode;
- files to be created;
- redacted field counts by category;
- any values the scanner could not classify;
- estimated size;
- public issue summary preview;
- local bundle path.

Buttons:

- Export safe report;
- Export enhanced report;
- Copy issue summary;
- Open prefilled GitHub issue;
- Clear diagnostics.

---

## 7. Capture modes

Debugging should be optional at two levels:

1. whether a diagnostic session exists;
2. which sections and detail levels feed it.

| Mode | Persistence | Content | Network metadata | Intended use |
|---|---|---|---|---|
| **Off** | None | None | None | Ordinary use. Current health gauges may be read live, but no history is retained. |
| **Safe** | Memory by default; optional capped file | Stable codes, counts, durations, safe environment, HMACed references | No raw addresses; route types and outcomes only | Default support capture. |
| **Enhanced** | Capped file; explicit expiry | Safe data plus selected sensitive metadata | Raw or partially masked addresses, transport paths, peer timing where selected | Network, sync, and multi-peer failures. |
| **Full trace** | Capped, short-lived, never retained as a preference | Detailed spans and transport events, still excluding secrets/content | Detailed route and protocol metadata | Maintainer/developer reproduction only. |

Recommended rules:

- Safe is the only mode eligible for a simple “Start diagnostics” button.
- Enhanced and Full show a privacy warning and automatic stop time.
- Full stops at restart.
- Section levels can be Off, Error, Warn, Info, Debug, or Trace.
- File persistence is separate from in-memory viewing.
- A developer environment variable may override filters, but the tab must show that override.
- Turning capture off takes effect immediately; it must not require restart.

### Runtime toggling

The current “subscriber can only be installed once” limitation does not require restart-only debug controls. Install one diagnostic layer during process setup and use:

- `tracing_subscriber::reload::Handle`, or
- a custom layer whose filter and sinks are atomically reconfigurable.

The global subscriber remains installed; capture destinations and filters change at runtime.

---

## 8. Section feed matrix

Every section is independently selectable. Safe mode should use allowlisted summaries; Enhanced/Full can add explicitly disclosed fields.

| Section ID | Area | Safe-mode feed | Enhanced/full additions | Example automatic findings | Sensitivity/cost |
|---|---|---|---|---|---|
| `diag` | Diagnostic pipeline | sink state, queue depth, drops, suppressions, rule errors, export status | writer timings and internal spans | sink unavailable, queue saturated, recursion prevented, redaction failure | Low data sensitivity; must always be cheap |
| `startup` | Build, process, platform, startup | app/version/commit, target triple, package type, OS family/version, architecture, webview family, startup phases | WebView2/WebKitGTK versions, X11/Wayland, XDG classification, audio backend, GPU/render backend | startup phase timeout, unsupported dependency, app-data unwritable | Paths and device details need scrubbing |
| `ui` | Svelte/webview/render/navigation | handler names, state transition codes, view generation, visibility/focus, render errors, long-task counts | selected stack traces, component timing, DOM/window metrics | unhandled rejection, stale async response applied, render loop, duplicated hooks | Never log message text or names |
| `ipc` | Tauri commands and events | command/event name, request ID, start/end, duration, result code, payload shape/count | safe field summaries, queue timings | command timeout, event emit failed, frontend event gap, response after cancellation | Moderate hot-path cost; batch |
| `runtime` | Actors, tasks, mailboxes, timers | task start/stop, heartbeat, mailbox depth, retry schedule, cancellation | poll duration and selected spans | actor heartbeat lost, task panic, mailbox saturation, runaway retry | Low sensitivity; performance-sensitive |
| `vault` | Vault, lock, continuity, backup | locked/unlocked state transitions, record counts, operation outcomes, backup phase | scrubbed storage roots, KDF timing, file operation detail | persistence failed, lock race, continuity rejected, backup incomplete | Never log secrets or decrypted state |
| `storage` | Store, manifests, integrity, repair | counts, byte totals, missing/corrupt categories, CID HMACs, repair outcomes | local path class, provider refs, per-record timings | integrity failure, index/file disagreement, disk low, repair loop | CIDs and paths are sensitive |
| `identity` | Device identity, pairing, grant lifecycle | ceremony phase, nonce HMAC, device HMAC, validation outcome, expiry | bounded route-count detail and cert-chain metadata | pairing nonce reused, ledger persistence failed, grant stuck | Never log bundle, SAS, keys, or transport passphrase |
| `membership` | MLS, membership, roles, moderation authority | epoch numbers/deltas, operation type, member counts, stable outcome codes | pseudonymous leaf/device refs and commit hashes | epoch gap, unauthorized operation rejected, commit stalled | Group and device linkage sensitive |
| `transport` | libp2p listeners, dials, connections | transport kind, direction, outcome, duration, reason code, aggregate counts | raw/masked addresses, peer refs, connection lifecycle | dial storm, connection flap, all candidates exhausted | High metadata volume; off by default in Safe |
| `reachability` | UPnP, PCP, NAT-PMP, AutoNAT, relay, rendezvous | mechanism, candidate count/type, state transitions, callback result | exact candidates, observer refs, lease detail | mapping claimed but callback failed, lease expired without refresh, relay lost | Raw addresses require consent |
| `discovery` | PEX, cached records, route changes | record sequence, route counts, source types, retry/backoff, freshness | HMACed peer/address history and validation detail | stale record retried forever, sequence regression, no viable route | Moderate metadata |
| `join` | Invite, preview, admission, reply, switchboard | stable outcome IDs, phase timings, route counts, expiry reason, trace | pseudonymous inviter/joiner/helper refs and per-candidate attempts | reply expired, wrong inviter, admission passed checks then failed, owner queue stuck | Never log invite bearer or full nonce |
| `sync` | Replication, catch-up, delivery, CRDT documents | document type, HMACed document ID, head hash, op counts, epoch, catch-up phase | peer refs, dependency summaries, queue depth | connected but no progress, catch-up loop, document head regressed, epoch unavailable | No document body or op content |
| `channels` | Chat, channels, unread, inbox, delivery, jukebox | channel HMAC, event sequence, change flags, row counts, read ceilings, visibility decision, queue hash/seq, playback drift | selected peer receipt refs and timing | appended message produced no unread transition, unread cleared while unseen, channel event gap, jukebox update event but unchanged state, competing juke sequence | Critical for current bugs; no message/title/track names |
| `documents` | Wiki, status, calendar, livery, moderation | document type, key HMAC, revision/head, operation type, counts, validation outcome | authority refs and timing | update event without head change, invalid moderation record, stale revision applied | No body, title, event text, or display names |
| `files` | Uploads, downloads, media protocol, cache | transfer ID, size bucket, chunk counts, progress, source local/network, hash outcome, stall age | provider refs, CID HMAC, range details, cache metrics | orphan upload, chunk mismatch, transfer stalled, declared-size mismatch, cache thrash | Filenames/content excluded |
| `voice` | WebRTC, signalling, ICE, TURN, devices | room HMAC, peer count, signalling phase, ICE state, candidate type, track state, reconnect attempt | candidate pair details, codec stats, TURN/STUN target class | signal without route, ICE failed, negotiation glare loop, no media after connected, heartbeat stale | Device names and addresses sensitive |
| `devices` | MIDI, microphone/output, notifications | permission state, device counts, route state, event-rate metrics, sound outcome | scrubbed device labels when explicitly selected | MIDI event flood, remembered device missing, audio context stuck | Device labels can identify hardware |
| `updates` | Updater and external launches | update channel present, phase, version comparison, signature outcome, launch result | endpoint host and download timing | updater misconfigured, signature rejected, relaunch failed | Never log credentials or full local paths |
| `performance` | CPU, memory, queues, responsiveness | process memory, event-loop lag, command p50/p95/p99, queue high-water, file rate | per-section spans and allocation samples | UI long tasks, queue growth, log overhead exceeded budget | Sampling must be bounded |
| `privacy` | Redaction, export, issue preparation | field classifications, redaction counts, validator result, consent state | candidate-sensitive fields visible only in preview | secret-like token detected, unknown field blocked, issue body exceeds safe policy | Must not echo rejected values |

### 8.1 Specific instrumentation for unread indicators

Because unread behaviour spans several layers, one trace should contain:

```text
CHANNEL.DOCUMENT.UPDATED
  messages_appended=true
  messages_changed=false
  topic=false
  jukebox=false
  channel_ref=...
  document_head=...

TAURI.EVENT.EMIT
  name=channel-updated
  event_seq=1199

UI.EVENT.RECEIVE
  name=channel-updated
  event_seq=1199
  active_server=false
  active_channel=false
  window_visible=true

UI.UNREAD.DECISION
  decision=mark_unread
  reason=message_appended_outside_observed_conversation
  previous=false
  next=true

UI.RAIL.BADGE.RENDER
  unread_channels=1
```

High-value invariants:

- `messages_appended=true` must eventually lead to either:
  - a valid “seen” explanation, or
  - an unread state transition.
- An unread mark must not be cleared while:
  - the window is hidden;
  - another surface covers chat;
  - another server/channel is active;
  - the relevant message window has not loaded.
- Every Tauri event sequence received must be contiguous or explicitly marked as coalesced.
- A refresh completing for an old view generation must never change current unread state.
- Backend channel heads and frontend read ceilings should be comparable by safe counters/hashes.

### 8.2 Specific instrumentation for jukebox sync

One trace should record:

```text
JUKEBOX.OP.LOCAL
  operation=add
  entry_ref=...
  local_seq=42

SYNC.DOCUMENT.POST
  doc_type=channel
  head_before=...
  head_after=...

PEER.DELIVERY
  holders=2
  expected_members=3

TAURI.EVENT.EMIT
  name=channel-updated
  jukebox=true

UI.EVENT.RECEIVE
  event_seq=...
  room_matches_call=true

UI.JUKEBOX.REFRESH
  queue_hash_before=...
  queue_hash_after=...
  entries_before=2
  entries_after=3

UI.JUKEBOX.CLAIM
  winner_ref=...
  sequence=42
  position_ms=...
  drift_ms=...
```

Useful findings:

- `jukebox=true` event received but `get_jukebox` returns the same head repeatedly;
- backend head changes but no frontend refresh is started;
- frontend refresh starts but applies to a different server/channel;
- two peers report the same sequence with different winning entry hashes;
- queue converged but playback claim did not;
- playback drift exceeds a threshold for a sustained period;
- media fetch is stalled while queue state is healthy;
- a change was classified as jukebox-only but accidentally raised an unread message.

---

## 9. Canonical event contract

### 9.1 Proposed schema

```json
{
  "schema_version": 1,
  "seq": 4812,
  "event_id": "01J60...",
  "timestamp_utc": "2026-08-23T18:47:12.331Z",
  "ingest_monotonic_ms": 12443,
  "session_id": "d-83a1...",
  "trace_id": "7f2c...",
  "span_id": "91ab...",
  "parent_span_id": "6dc4...",
  "request_id": "ipc-91",
  "section": "join",
  "subsystem": "catcoms_sync",
  "code": "JOIN.ROUTES.EXHAUSTED",
  "level": "warn",
  "phase": "failure",
  "operation": "join_server",
  "outcome": "no_viable_route",
  "duration_ms": 60123,
  "attempt": 4,
  "retry_in_ms": null,
  "refs": {
    "server": "srv-a17c",
    "channel": null,
    "peer": "peer-5d09",
    "document": null,
    "transfer": null
  },
  "fields": {
    "direct_candidates": 4,
    "relay_candidates": 0,
    "rendezvous_targets": 1
  },
  "error": {
    "kind": "unreachable",
    "code": "NET.ALL_CANDIDATES_FAILED",
    "safe_message": "No advertised route completed.",
    "retryable": true,
    "remediation": "network.use_relay"
  },
  "source": {
    "crate": "catcoms-sync",
    "module": "join",
    "line": 1234,
    "build_commit": "406884b..."
  },
  "privacy": {
    "mode": "safe",
    "redacted_fields": ["candidate_addresses", "peer_id"],
    "contains_content": false
  }
}
```

### 9.2 Stable code rules

Use:

```text
AREA.COMPONENT.OUTCOME
```

Examples:

```text
DIAG.SINK.OPEN_FAILED
DIAG.QUEUE.EVENTS_DROPPED
UI.RUNTIME.UNHANDLED_REJECTION
IPC.COMMAND.TIMEOUT
RUNTIME.ACTOR.HEARTBEAT_LOST
VAULT.REGISTRY.LOAD_FAILED
STORAGE.MANIFEST.INTEGRITY_FAILED
NET.DIAL.ALL_CANDIDATES_FAILED
REACH.AUTONAT.CALLBACK_FAILED
JOIN.ADMISSION.INTERNAL_FAILED
SYNC.CATCHUP.NO_PROGRESS
CHANNEL.EVENT.SEQUENCE_GAP
UNREAD.STATE.CLEARED_WHILE_UNSEEN
JUKEBOX.STATE.HEAD_DIVERGED
FILE.TRANSFER.STALLED
VOICE.ICE.FAILED
PRIVACY.EXPORT.UNKNOWN_FIELD
```

The prose can change without breaking issue dedupe, tests, dashboards, or support instructions.

### 9.3 Time and ordering

Mewtual’s core architecture deliberately injects a `Clock`. Core crates should continue using that seam. The native diagnostic hub may add:

- a process-monotonic ingestion time;
- a local UTC wall-clock at the sink boundary;
- a strictly increasing `seq`.

This distinguishes:

- event-reported time;
- local observation order;
- causal parentage.

A timestamp alone must not be treated as causality.

### 9.4 Safe references

Safe mode should derive references with a random per-diagnostic-session salt:

```text
server_ref  = HMAC(session_salt, "server"  || group_id)[0..12]
peer_ref    = HMAC(session_salt, "peer"    || peer_id)[0..12]
channel_ref = HMAC(session_salt, "channel" || server_id || channel_id)[0..12]
file_ref    = HMAC(session_salt, "file"    || cid)[0..12]
```

This preserves correlation inside the report without creating a stable cross-session tracking identifier.

---

## 10. Correlation architecture

### 10.1 Command path

Create one frontend wrapper:

```text
invokeDebugged(command, safeSummary, args)
```

It should:

1. allocate a request ID and trace ID if one is not already active;
2. record frontend start;
3. invoke Tauri with a diagnostic context;
4. record frontend completion/failure and duration;
5. preserve the trace ID in `AppError`;
6. support cancellation and view-generation metadata.

Do not blindly log command arguments. Every command registers an allowlisted summary function.

### 10.2 Native command path

A command wrapper or macro should record:

```text
received
session_gate_checked
actor_lookup
actor_request_enqueued
actor_response
persistence_started
persistence_completed
event_emitted
returned_to_ui
```

A gradual migration could use:

```rust
diagnostic_command!(
    hub,
    context,
    section = Section::Join,
    operation = "join_server",
    async { ... }
)
```

### 10.3 Actor and task path

Actor messages should carry the diagnostic context separately from protocol payloads. The context must not be serialised onto the peer-to-peer wire unless a specifically designed cooperative trace mode is active.

### 10.4 Tauri event path

Every emitted event should have:

- native `event_seq`;
- trace ID;
- server reference;
- event type;
- safe change summary.

The frontend acknowledges receipt in its own event stream. Sequence gaps become detectable.

### 10.5 Cooperative peer trace mode

Some sync failures cannot be localised from one node alone. Add a later, explicit **Cooperative Trace Session**:

- both participants opt in;
- one device creates a short diagnostic session code;
- only a random trace-session nonce and coarse state digests are exchanged;
- raw logs are never exchanged automatically;
- no content, names, keys, full addresses, or stable cross-session IDs;
- the session expires quickly;
- each device exports its own local report;
- trace clocks are aligned using observed offsets, not assumed equal.

This can compare document heads, event sequences, delivery counters, and playback claims without disclosing document content.

A malicious peer’s diagnostic assertions remain untrusted observations, not facts.

---

## 11. Automatic issue detection

### 11.1 Rule engine, not an opaque AI dependency

Start with deterministic rules. They are:

- testable;
- versioned;
- explainable;
- cheap;
- safe offline;
- less likely to invent causes.

An optional summariser can later convert findings into prose, but it must never determine severity, redact data, or decide to submit an issue.

### 11.2 Finding schema

```json
{
  "finding_id": "find-183",
  "rule_id": "CHANNEL.EVENT.SEQUENCE_GAP",
  "rule_version": 1,
  "fingerprint": "sha256:...",
  "severity": "high",
  "confidence": "observed",
  "first_seen_seq": 4810,
  "last_seen_seq": 4812,
  "occurrences": 1,
  "summary": "A frontend channel event sequence was skipped.",
  "impact": "Unread, topic, or jukebox state may be stale.",
  "evidence_event_ids": ["...", "..."],
  "likely_causes": [
    "native event emission failed",
    "webview listener was not installed",
    "event was coalesced without declaring it"
  ],
  "recommended_actions": [
    "run IPC loopback check",
    "inspect the trace around the missing sequence"
  ],
  "recovered": false,
  "suppressed": false
}
```

### 11.3 Initial rule set

| Rule | Trigger |
|---|---|
| `DIAG.SINK.UNAVAILABLE` | Capture desired but no verified sink. |
| `DIAG.SINK.STALE` | No successful write while events continue. |
| `DIAG.QUEUE.SATURATED` | Queue above threshold or records dropped. |
| `DIAG.UI_HOOKS.DUPLICATED` | More than one frontend logger installation generation is active. |
| `UI.RUNTIME.UNHANDLED_REJECTION` | Unhandled rejection observed. |
| `UI.RUNTIME.LONG_TASK_STORM` | Sustained long-task rate above threshold. |
| `IPC.COMMAND.TIMEOUT` | Command exceeds its registered budget. |
| `IPC.EVENT.EMIT_FAILED` | Tauri emit returned an error. |
| `IPC.EVENT.SEQUENCE_GAP` | Frontend observes an unexplained event sequence gap. |
| `RUNTIME.ACTOR.HEARTBEAT_LOST` | Actor misses heartbeat or task exits. |
| `RUNTIME.MAILBOX.SATURATED` | Mailbox remains above high-water threshold. |
| `VAULT.PERSISTENCE.AFTER_MUTATION_FAILED` | In-memory mutation succeeded but persistence did not. |
| `STORAGE.DISK.LOW` | Diagnostics or vault volume below safe free-space threshold. |
| `STORAGE.INTEGRITY.FAILED` | Manifest, seal, CID, or reassembly verification fails. |
| `NET.DIAL.RETRY_STORM` | Repeated failed dials without candidate or epoch change. |
| `REACH.MAPPING.UNVERIFIED` | Mapping is reported but callback repeatedly fails. |
| `DISCOVERY.RECORD.SEQUENCE_REGRESSED` | New record sequence is lower/equal unexpectedly. |
| `JOIN.REPLY.EXPIRED_IN_PROGRESS` | Join reply expires while required path is still active. |
| `JOIN.ADMISSION.INTERNAL_FAILED` | External checks pass but admission fails. |
| `SYNC.CONNECTED.NO_PROGRESS` | Connected peer exists but document heads do not advance during pending work. |
| `SYNC.CATCHUP.LOOP` | Same gap requested repeatedly without progress. |
| `CHANNEL.EVENT.SEQUENCE_GAP` | Channel event sequence skipped. |
| `UNREAD.STATE.MISSING_AFTER_APPEND` | Append occurs outside observed view with no unread transition. |
| `UNREAD.STATE.CLEARED_WHILE_UNSEEN` | Read mark advances despite visibility/scope conditions. |
| `JUKEBOX.EVENT.NO_STATE_CHANGE` | Jukebox change event produces no head/queue change repeatedly. |
| `JUKEBOX.STATE.DIVERGED` | Cooperative trace sees equal sequence with differing queue/claim digest. |
| `FILE.TRANSFER.STALLED` | No byte/chunk progress within context-sensitive threshold. |
| `FILE.UPLOAD.ORPHANED` | Upload reservation survives lost frontend/session beyond grace. |
| `VOICE.SIGNAL.NO_MEMBER_ROUTE` | Signalling has no route for an expected participant. |
| `VOICE.ICE.FAILED` | ICE reaches failed with no viable fallback. |
| `VOICE.NEGOTIATION.LOOP` | Repeated offer/rollback without stable connection. |
| `PRIVACY.REDACTION.FAILED` | Export contains unknown, secret-like, or prohibited field. |
| `CLOCK.SKEW.SUSPICIOUS` | Peer or local timestamps exceed protocol-specific safe bounds. |

### 11.4 Highlighting and notification policy

- Critical/high findings create one app notification.
- Repeats update the existing finding; they do not create notification spam.
- Notifications have a per-fingerprint cooldown.
- Remote-triggerable findings never submit anything automatically.
- Finding prose comes from local static templates.
- A resolved condition remains visible as “recovered”.
- The viewer supports “mute this rule for this session” and “suppress this fingerprint”.
- Suppression is itself included in the report.

Suggested notification:

```text
Mewtual detected a diagnostic issue
CHANNEL.EVENT.SEQUENCE_GAP may have left unread or jukebox state stale.

[Review diagnostics] [Prepare GitHub issue] [Mute for session]
```

---

## 12. Segmented output design

The request for segmented logs is correct from a reading perspective, but physically writing one independent file per subsystem would destroy cross-layer chronology. A send failure often depends on the exact interleaving of UI, IPC, actor, network, persistence, and event delivery.

Use:

1. **one canonical chronological `events.jsonl`;**
2. **per-section indexes and filters;**
3. **a generated sectioned report;**
4. **a plain notepad export.**

### 12.1 Bundle structure

```text
mewtual-diagnostics-<session>-<timestamp>.zip
├── manifest.json
├── report.md
├── report.txt
├── events.jsonl
├── findings.json
├── metrics.json
├── environment.json
├── checks.json
├── redaction.json
├── user-notes.txt
└── hashes.sha256
```

### 12.2 `report.md` structure

```text
1. Manifest and capture configuration
2. Executive health summary
3. Detected findings, sorted by severity
4. Reproduction markers and user notes
5. Cross-section chronological timeline
6. Diagnostics self-health
7. Startup/environment
8. UI/webview
9. IPC
10. Runtime/actors
11. Vault/storage
12. Identity/membership
13. Network/reachability/discovery/join
14. Sync/channels/unread/jukebox/documents
15. Files/media
16. Voice/devices/updates
17. Performance metrics
18. Redaction and privacy audit
19. Suggested GitHub issue body
```

### 12.3 `report.txt` format

`report.txt` is for Notepad and simple paste workflows. It should contain explicit ASCII section headers:

```text
===============================================================================
MEWTUAL DIAGNOSTICS — DETECTED FINDINGS
===============================================================================

[HIGH] CHANNEL.EVENT.SEQUENCE_GAP
First seen: ...
Evidence: ...

===============================================================================
MEWTUAL DIAGNOSTICS — CHRONOLOGICAL TIMELINE
===============================================================================

0004810 ...
0004811 ...
0004812 ...

===============================================================================
MEWTUAL DIAGNOSTICS — SYNC / CHANNELS / UNREAD / JUKEBOX
===============================================================================
...
```

### 12.4 Determinism

Given identical canonical events and configuration, export order and contents should be byte-stable except for explicitly listed generation metadata. This enables:

- reliable tests;
- report hashing;
- issue deduplication;
- comparing two peers’ reports;
- verifying that the exporter did not omit sections.

---

## 13. GitHub issue workflow

### 13.1 Recommended first implementation

Extend the existing feedback mechanism:

1. a finding is detected;
2. the user is notified;
3. the app opens the diagnostics finding;
4. the user selects the relevant time range and sections;
5. the app creates a Safe report;
6. the redaction preview must pass;
7. the app builds a bounded issue body;
8. the native allowlisted command opens GitHub’s issue form;
9. the user reviews and presses Submit in the browser;
10. the local bundle remains available for manual attachment.

This preserves the current no-credential security model.

### 13.2 Issue body

```markdown
## Diagnostic finding

**Code:** `CHANNEL.EVENT.SEQUENCE_GAP`
**Fingerprint:** `sha256:...`
**Severity:** High
**App:** Mewtual 0.x.x (`406884b`)
**Platform:** Linux x86_64, AppImage, Wayland, WebKitGTK
**Trace:** `7f2c...`

## What happened

A channel update event sequence advanced from 1198 to 1200 without an explicit
coalescing record. Unread or jukebox state may therefore be stale.

## Reproduction

1. ...
2. ...

## Expected

...

## Observed

...

## Safe evidence

- Event 4810: native channel document changed
- Event 4811: native emit reported sequence 1199
- Event 4812: frontend received sequence 1200
- Diagnostic drops: 0
- Redaction validation: passed

<!-- mewtual-diagnostic-fingerprint: sha256:... -->
```

The issue title should come from a local template:

```text
[diag:CHANNEL.EVENT.SEQUENCE_GAP] Channel state event was skipped
```

Never put peer-supplied text into the title.

### 13.3 Fingerprinting and dedupe

Fingerprint inputs should include only stable local facts:

```text
rule_id
app major/minor
platform family
top stack-frame module where safe
normalised outcome code
```

Exclude:

- timestamps;
- peer IDs;
- server IDs;
- IP addresses;
- message/file/wiki content;
- full stack addresses;
- random trace IDs.

The app can search existing issues for the fingerprint before opening a new one, but failure to search must not block reporting.

### 13.4 Why silent automatic submission is rejected

A GitHub issue is normally public. A diagnostic report can reveal:

- IP addresses and ports;
- peer relationships;
- server and device identifiers;
- activity timing;
- local paths and hardware;
- crash stacks;
- user notes.

Silent submission would also require credentials and would let remote-triggerable conditions create public issues. That is an unacceptable security and abuse boundary.

A later **authenticated maintainer mode** may use OAuth/device flow and OS credential storage to create a sanitised issue with one click. Even then:

- preview remains mandatory;
- only the sanitised summary is submitted;
- raw bundles are not uploaded automatically;
- scope is limited to issue creation;
- tokens can be revoked;
- rate limits and duplicate detection apply.

### 13.5 Repository support

The reviewed `.github` directory contains workflows but no issue-template directory. Add:

```text
.github/ISSUE_TEMPLATE/diagnostic.yml
.github/ISSUE_TEMPLATE/config.yml
```

Suggested labels:

```text
diagnostic
needs-triage
area:ui
area:sync
area:network
area:storage
area:voice
privacy-reviewed
```

---

## 14. Privacy and security model

### 14.1 Field policy

| Class | Examples | Safe mode | Enhanced | Full |
|---|---|---|---|---|
| Public | app version, error code, enum outcome | Include | Include | Include |
| Safe operational | count, duration, retry number, queue depth | Include | Include | Include |
| Sensitive identifier | peer ID, group ID, device fingerprint, CID | Session-HMAC | HMAC or explicit raw selection | Raw only when explicitly selected |
| Sensitive network | IP, port, relay path, STUN/TURN target | Type/count only | Masked or raw by selection | Raw with warning |
| Local identity | username, home path, device label | Remove/scrub | Scrub unless individually selected | Scrub by default |
| Secret | key, credential, token, invite bearer, pairing bundle, passphrase | Impossible to encode | Impossible to encode | Impossible to encode |
| Content | message, wiki, status, event text, file bytes | Impossible to encode | Impossible to encode | Impossible to encode |

“Full” means fuller operational metadata, not content or secrets.

### 14.2 Source-side enforcement

Do not rely on regex after the event was built. Use typed values:

```rust
enum SafeValue {
    Bool(bool),
    Count(u64),
    DurationMs(u64),
    Outcome(&'static str),
    SafeText(BoundedSafeText),
    SessionRef(SessionRef),
}
```

Sensitive values require an explicit constructor that consults capture mode.

### 14.3 Export-side defence

The exporter performs an independent pass:

- known secret formats;
- invite/bundle prefixes;
- credential-like keys;
- URL query tokens;
- home-directory paths;
- email addresses;
- raw IP addresses in Safe mode;
- unexpectedly long high-entropy strings;
- fields not registered in the schema.

The redaction report records counts and field names, never the removed value.

An unknown field should fail closed:

```text
Export blocked: event 4812 contains unclassified field "candidate_blob".
```

### 14.4 Retention

The tab must show:

- exact directory;
- exact bytes retained;
- oldest/newest event;
- expiry;
- clear button;
- whether a previous crash session exists.

Enhanced and Full sessions should auto-delete according to a short retention policy unless the user explicitly saves an export.

### 14.5 Locked-session behaviour

The threat model treats the webview as trusted only while the session is unlocked.

Recommended behaviour:

- the native hub may continue recording Safe operational events while locked if the user started a capture;
- the debug tab and log query commands require an unlocked UI session;
- content-derived labels are never recorded;
- raw Enhanced/Full display is cleared when locking;
- export requires unlock;
- frontend pending batches are discarded on lock;
- the title bar shows no server-specific diagnostic content while locked.

### 14.6 Diagnostic integrity

A log is an observation, not cryptographic proof. Include:

- build commit;
- schema version;
- capture configuration;
- exporter version;
- bundle hashes;
- whether developer overrides were active;
- whether events were dropped;
- whether the client was modified or unsigned where detectable.

Do not present peer-provided diagnostics as authoritative.

---

## 15. Native implementation architecture

### 15.1 New crate

Create:

```text
crates/catcoms-diagnostics/
├── src/lib.rs
├── src/config.rs
├── src/event.rs
├── src/error.rs
├── src/hub.rs
├── src/layer.rs
├── src/ring.rs
├── src/writer.rs
├── src/redact.rs
├── src/rules.rs
├── src/findings.rs
├── src/metrics.rs
├── src/checks.rs
├── src/export.rs
└── src/issue.rs
```

`catcoms-log` can become the thin adapter that installs console formatting and the diagnostic `tracing` layer.

### 15.2 `DiagnosticHub`

```text
DiagnosticHub
├── current configuration
├── reloadable filter handle
├── bounded chronological ring
├── per-section indexes
├── session sequence allocator
├── HMAC session salt
├── bounded writer queue
├── sink health
├── metrics/counters
├── rule engine
├── current findings
└── batched frontend broadcaster
```

### 15.3 Data flow

```text
Rust tracing spans/events ───────┐
Typed AppError events ───────────┤
Command/event wrappers ──────────┤
Actor/task supervisor ───────────┤
Frontend structured events ──────┼──> DiagnosticHub
Probe/check results ──────────────┤        │
Logger self-health ───────────────┘        ├── bounded memory ring
                                          ├── capped file writer
                                          ├── metrics
                                          ├── deterministic rules
                                          ├── batched UI events
                                          └── export/issue summary
```

### 15.4 Writer health

The writer should expose:

```rust
struct SinkHealth {
    desired: bool,
    state: SinkState,
    path: Option<PathBuf>,
    started_at: Option<Timestamp>,
    last_write_at: Option<Timestamp>,
    events_written: u64,
    bytes_written: u64,
    dropped_events: u64,
    queue_depth: usize,
    queue_high_water: usize,
    last_error: Option<SafeDiagnosticError>,
}
```

A worker panic or I/O failure transitions the sink to `Failed` and generates an in-memory finding without recursively writing through the failed sink.

### 15.5 Batching to the webview

Do not invoke Tauri once per log line in either direction.

Recommended behaviour:

- frontend sends structured records in bounded batches;
- native UI stream emits every 100–250 ms or at a bounded count;
- high/critical findings can bypass ordinary delay;
- the UI queries historical pages by sequence range;
- pause stops rendering, not capture;
- a slow viewer cannot block the writer.

### 15.6 Error model

Introduce:

```text
ErrorCode registry
AppError
InternalErrorContext
Remediation registry
```

The user-facing error can say:

```text
Message could not be sent.
Diagnostic code: MSG.SEND.ACTOR_STOPPED
Trace: 7f2c
```

The local diagnostic event retains the safe cause chain.

### 15.7 Task supervision

Wrap long-lived tasks:

```rust
let task = supervisor.spawn(
    TaskMeta::server(server_ref, "server_actor"),
    async move { ... }
);
```

The supervisor records:

- normal completion;
- cancellation;
- panic;
- timeout;
- restart;
- missed heartbeat.

Do not blindly restart stateful MLS/actor tasks. The restart policy is task-specific and usually “surface failure, preserve evidence, require controlled reload”.

---

## 16. Tauri API

Suggested commands:

```text
debug_get_state
debug_get_config
debug_set_config
debug_start_session
debug_stop_session
debug_query_events
debug_get_findings
debug_run_check
debug_cancel_check
debug_mark_reproduction
debug_set_notes
debug_export_bundle
debug_prepare_issue
debug_open_issue
debug_clear
debug_reveal_export
```

Suggested events:

```text
debug://batch
debug://finding
debug://state
debug://check-progress
debug://export-progress
```

Security rules:

- state/config access may be available before unlock only in a minimal safe form;
- event queries, notes, export, and issue preparation require unlock;
- paths returned to the frontend are display-safe;
- no arbitrary file reads;
- issue URLs retain the existing exact allowlist;
- check IDs come from a registry, not arbitrary command strings.

---

## 17. Frontend file structure

```text
apps/desktop/src/debug/
├── types.ts
├── debug-bridge.ts
├── debug-store.ts
├── format-event.ts
├── filters.ts
├── DebugTab.svelte
├── OverviewPanel.svelte
├── LiveLog.svelte
├── FindingsPanel.svelte
├── ChecksPanel.svelte
├── NotesPanel.svelte
├── ExportPanel.svelte
├── ConfigPanel.svelte
├── RedactionPreview.svelte
└── IssueComposer.svelte
```

Small changes elsewhere:

- `main.ts`: bootstrap capture before dynamic import;
- `uilog.ts`: structured events, lifecycle cleanup, batching, redaction;
- `App.svelte`: lazy route/open state only;
- `feedback.ts`: diagnostic issue-body builder and fingerprint;
- `FeedbackOverlay.svelte`: optional selected finding/report input.

### 17.1 Fix `uilog.ts`

Required changes:

- add `removeEventListener`;
- make cleanup complete;
- add installation generation/idempotency;
- stop arbitrary `JSON.stringify` in Safe mode;
- categorise errors;
- capture source/component when registered;
- preserve repeat counts;
- batch messages;
- use a bounded local fallback queue;
- expose dropped count;
- avoid logging the logging sink’s own failures recursively.

### 17.2 Instrumenting direct invokes

There are many direct `invoke` calls. Do not attempt a risky one-shot rewrite. Introduce a wrapper, then migrate high-value paths first:

1. send/edit/delete/reaction/pin;
2. channel refresh, heads, unread, inbox;
3. jukebox;
4. join/connectivity;
5. files/transfers;
6. voice signalling;
7. vault and persistence;
8. remaining settings and document operations.

A CI check can prevent new direct invocations outside the wrapper while migration proceeds.

---

## 18. Platform-specific environment capture

A debugging suite for the desktop build should collect platform facts without collecting personal identity.

### Linux

Safe:

- distribution family/version;
- kernel version;
- architecture;
- AppImage, DEB, or development run;
- Wayland/X11;
- WebKitGTK version;
- XDG data/cache/config directory **classification**, not full username-bearing path;
- PipeWire/PulseAudio/ALSA backend availability;
- keyring availability;
- app-data directory writable;
- executable mount flags relevant to AppImage;
- firewall/mapping capability outcomes.

Enhanced:

- scrubbed library versions;
- selected environment-variable presence, not values;
- exact failing shared-library name;
- portal/sandbox error codes.

### Windows

Safe:

- Windows version/build;
- architecture;
- WebView2 runtime version;
- package/development run;
- app-data writable;
- firewall/mapping outcome;
- audio and MIDI permission/device counts.

Never include:

- Windows username;
- full home path;
- machine name;
- Wi-Fi SSID;
- device serial numbers.

### Other targets

Keep the schema target-neutral. Platform-specific fields live under a versioned `environment.platform` object and require registration.

---

## 19. Performance safeguards

Diagnostics must not become the cause of the bug.

### Budgets

Measure and enforce:

- event construction time;
- queue push time;
- writer throughput;
- UI batch size;
- rule-engine time;
- memory-ring bytes;
- export peak memory.

Recommended properties:

- no blocking file I/O on actor or UI paths;
- no unbounded channel;
- no serialising full application objects;
- no stack capture below configured severity;
- sampling for high-frequency success events;
- always retain errors, state transitions, and aggregate summaries;
- event size hard cap;
- field count hard cap;
- collection length hard cap;
- export streams to disk rather than building one giant string.

The Performance section should show diagnostic overhead itself:

```text
diagnostic_cpu_ms
diagnostic_bytes_per_second
queue_high_water
events_sampled
events_suppressed
events_dropped
viewer_render_ms
```

---

## 20. Implementation sequence

### Phase 0 — Stop the current logger from lying or harming the app

1. Make logger initialisation return a real success/failure state.
2. Set `active` from verified sink state.
3. Stop selecting a stale “newest” file as the current session.
4. Fix UTF-8 truncation.
5. Correct frontend listener cleanup.
6. Store and invoke the cleanup from `App.svelte`.
7. Add an idempotency guard for HMR.
8. Add file quota, retention, and writer-drop counters.
9. Rate-limit/batch `log_ui`.
10. Install startup capture in `main.ts`.
11. Replace silent logger initialisation failures with a visible diagnostics status.

### Phase 1 — Canonical event model and hub

1. Add `catcoms-diagnostics`.
2. Define sections, levels, codes, privacy classes, and schema version.
3. Add a bounded ring and sink health.
4. Add reloadable runtime filters.
5. Adapt existing `tracing`.
6. Convert raw `eprintln!` in desktop operational paths.
7. Add deterministic text and JSONL rendering.

### Phase 2 — Debug tab

1. Add lazy-loaded route.
2. Build overview and actual sink status.
3. Add paged virtualised live log.
4. Add section/level controls.
5. Add notes and reproduction markers.
6. Add clear and export controls.

### Phase 3 — Correlation and typed errors

1. Add frontend invoke wrapper.
2. Add native command wrapper.
3. Add event sequence numbers.
4. Carry trace IDs through actors and emitted events.
5. Introduce `AppError` and error-code registry.
6. Add task supervision and actor health.

### Phase 4 — High-value subsystem feeds

Order by current diagnostic pain:

1. channels, unread, inbox, delivery;
2. jukebox;
3. join, reply, switchboard, reachability;
4. sync and catch-up;
5. files/media;
6. voice/WebRTC;
7. vault/storage/pairing;
8. remaining document and settings paths.

### Phase 5 — Findings and checks

1. Logger self-health rules.
2. IPC/event-gap rules.
3. actor/task rules.
4. unread and jukebox invariants.
5. retry/no-progress rules.
6. transfer and voice rules.
7. explicit safe probes.

### Phase 6 — Export and GitHub integration

1. Deterministic segmented bundle.
2. source-side privacy types;
3. independent export validator;
4. redaction preview;
5. stable issue fingerprint;
6. current browser-based issue flow integration;
7. diagnostic issue template.

### Phase 7 — Hardening

1. failure injection;
2. privacy property tests;
3. performance benchmarks;
4. cross-platform packaging tests;
5. cooperative trace design and review;
6. documentation and support playbook.

---

## 21. Immediate patch set

The first change should be small enough to review adversarially and valuable before the full suite exists.

### Patch A — Logger correctness

- Change `init_debug_with` to return `Result`.
- Propagate `create_dir_all`, file-open, and subscriber errors.
- Add an explicit session filename to the returned handle.
- Make `LogState.active` reflect actual state.
- Remove `newest_log_file` from the current-session identity path.
- Add a `DIAG.SESSION.STARTED` marker.
- Add bounded rotation/quota.
- Add writer-drop counters.

### Patch B — Frontend lifecycle and Unicode

- Make `installUiLogging` accept/remove listeners.
- Capture cleanup in `App.svelte`.
- Add HMR idempotency.
- Fix byte-safe native truncation.
- Add tests for repeated mounts and multibyte text.
- Batch frontend records and count failed sends.

### Patch C — Bootstrap and self-test

- Install minimal logging before dynamic `App.svelte` import.
- Add `debug_self_test`.
- Show desired versus actual state.
- Add “write test record” and “reveal exact current file”.

### Patch D — First structured trace

Instrument one path end to end:

```text
send_message
```

Carry a trace through:

- click;
- Tauri invoke;
- command;
- actor;
- replication;
- persistence;
- `channel-updated`;
- frontend receive;
- unread/read decision;
- render.

This becomes the pattern for every later operation.

---

## 22. Test and failure-injection plan

### 22.1 Logger tests

- directory permission denied;
- nonexistent/uncreatable path;
- disk full;
- file removed mid-session;
- writer worker panic;
- subscriber already installed;
- queue saturation;
- process exits with queued records;
- Unicode truncation at every byte boundary;
- huge error stack;
- hostile/cyclic frontend object;
- repeated identical events;
- timestamp collision;
- retention deletion failure.

### 22.2 Frontend lifecycle tests

- install/uninstall once;
- ten HMR remounts;
- logger installed twice accidentally;
- error listener after cleanup;
- unhandled rejection after cleanup;
- original console preserved;
- sink unavailable;
- batched sink partially fails;
- lock/unlock clears sensitive viewer state;
- startup dynamic import fails.

### 22.3 Correlation tests

- one trace reaches all expected stages;
- concurrent identical commands remain distinguishable;
- retry increments attempt under one trace;
- cancellation closes the trace;
- stale view-generation result is recorded but not applied;
- event sequence gap produces one finding;
- emitted event failure is visible;
- no trace context crosses onto peer wire by accident.

### 22.4 Privacy tests

- property-based event generation cannot encode Secret/Content;
- all Safe exports contain no raw IPs;
- no home-directory usernames;
- no invite or pairing bundle patterns;
- no passphrase/credential fields;
- session refs change between sessions;
- same identifier correlates inside one session;
- unknown field blocks export;
- redaction report never echoes removed data;
- issue body is a strict subset of Safe report.

### 22.5 Unread tests

- append while another server active;
- append while another tab active;
- append while window hidden;
- append while chat scrolled up;
- append during server switch;
- edit/reaction/topic/jukebox change does not create unread;
- stale refresh cannot clear newer unread;
- missing event sequence is detected;
- frontend event listener remount does not duplicate transitions.

### 22.6 Jukebox tests

- add/remove converges between two deterministic peers;
- queue event reaches inactive call room;
- event before channel list;
- equal sequence/different digest creates finding;
- queue converges but playback claim diverges;
- track fetch stall separated from sync failure;
- jukebox-only event does not trigger unread;
- reconnect applies latest queue exactly once.

### 22.7 Network/sync tests

Use the existing deterministic transport and injected clock seams:

- all routes fail;
- late relay succeeds;
- mapping appears then expires;
- AutoNAT callback contradicts mapping;
- PEX record sequence regression;
- actor connected but catch-up stalls;
- repeated same gap;
- out-of-order responses;
- network-change burst coalescing;
- clock skew.

### 22.8 Performance tests

- 100,000-event session remains searchable;
- viewer renders only visible rows;
- export streams without duplicating whole log;
- diagnostic overhead under ordinary chat load remains within an agreed budget;
- high-frequency transport events are sampled/aggregated;
- logger queue cannot block actor progress.

---

## 23. Acceptance criteria

The debug suite is not complete until all of these are true:

1. A user can turn capture on and off without restarting.
2. Off mode creates no diagnostic history or file.
3. The UI distinguishes desired state from actual sink health.
4. A sink failure cannot remain silent.
5. A dropped event increments a visible counter and finding.
6. Ten frontend remounts still produce exactly one forwarded exception.
7. Any valid Unicode log input is non-panicking.
8. Every high-value command has a trace from UI start to final UI outcome.
9. Every emitted Tauri event has a sequence number.
10. Event gaps are detectable.
11. Repeated events preserve occurrence counts.
12. Safe mode cannot encode message/wiki/file content or secrets.
13. Raw IPs and stable identifiers are absent from Safe exports.
14. The live viewer remains responsive under a long session.
15. The file writer is bounded by explicit quota and retention.
16. Findings link to exact evidence events.
17. Findings state whether a cause is observed or inferred.
18. A malicious peer cannot control finding prose, issue title, or submission.
19. GitHub issue preparation includes a stable fingerprint.
20. Nothing is publicly submitted without explicit review and consent.
21. The exported report is segmented and also contains a canonical chronological timeline.
22. Export includes capture configuration, dropped counts, schema version, build commit, and redaction status.
23. Linux and Windows environment reports identify webview/package/runtime class without usernames or machine names.
24. Debug code is outside the main Svelte and Tauri monoliths except for narrow integration points.
25. CI prevents regression to raw `eprintln!`, unchecked event emission, and new direct frontend invokes in instrumented areas.

---

## 24. Suggested CI policy

Add repository checks that progressively enforce:

```text
- no eprintln! outside approved bootstrap/test modules
- no `let _ = app.emit(...)`
- no new Tauri command returning bare String error in migrated modules
- no new direct `invoke(...)` outside debug-bridge/invoke wrapper allowlist
- every ErrorCode exists in the registry
- every DiagnosticEvent field has a privacy classification
- every finding rule has unit tests
- Safe export golden tests pass
- logger lifecycle and Unicode regression tests pass
```

Do not turn every rule on at once. Establish the framework, migrate a subsystem, then make the check mandatory for that subsystem.

---

## 25. Example generated report excerpt

```text
===============================================================================
MEWTUAL DIAGNOSTICS
===============================================================================
Session: d-83a1
Mode: Safe
Build: 406884b
Platform: Linux x86_64 · AppImage · Wayland · WebKitGTK
Capture: 2026-08-23 18:46:59Z — 18:48:21Z
Events: 1,284
Suppressed repeats: 312
Dropped events: 0
Redaction validation: PASSED

===============================================================================
DETECTED FINDINGS
===============================================================================

[HIGH] CHANNEL.EVENT.SEQUENCE_GAP
First seen: 18:47:12.331Z
Last seen: 18:47:12.331Z
Occurrences: 1
Trace: 7f2c
Impact: unread, topic, or jukebox state may be stale.

Observed:
- Native channel document changed.
- Native event sequence 1199 was allocated.
- Frontend next received sequence 1200.
- No declared coalescing record exists.

Possible causes:
- event emission failed;
- frontend listener was not active;
- event was dropped before delivery.

Recommended checks:
- run IPC loopback;
- inspect native emit event 4811;
- inspect frontend remount generation.

===============================================================================
TRACE 7f2c — CHANNEL UPDATE
===============================================================================

0004808 18:47:12.201 INFO  SYNC     CHANNEL.DOCUMENT.HEAD_ADVANCED
0004809 18:47:12.208 INFO  CHANNEL  CHANNEL.CHANGE.CLASSIFIED messages_appended=true
0004810 18:47:12.212 INFO  IPC      IPC.EVENT.EMIT_STARTED sequence=1199
0004811 18:47:12.213 ERROR IPC      IPC.EVENT.EMIT_FAILED sequence=1199
0004812 18:47:12.331 WARN  IPC      IPC.EVENT.SEQUENCE_GAP expected=1199 received=1200
```

---

## 26. Source-level reference index

Reviewed source:

- [Architecture](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/docs/ARCHITECTURE.md)
- [Threat model](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/docs/THREAT-MODEL.md)
- [Native logging crate](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/crates/catcoms-log/src/lib.rs)
- [Tauri bridge and existing logging controls](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src-tauri/src/lib.rs)
- [Frontend log forwarding](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/uilog.ts)
- [Frontend log tests](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/uilog.test.ts)
- [Frontend bootstrap](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/main.ts)
- [Main Svelte application](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/App.svelte)
- [Join/connectivity diagnostics](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/joinlog.ts)
- [Feedback overlay](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/FeedbackOverlay.svelte)
- [Feedback report and GitHub URL builder](https://github.com/Thalpy/Mewtual/blob/406884b1789e70a740fb3b0abda25dc1549072d5/apps/desktop/src/feedback.ts)

---

## 27. Final recommendation

Treat diagnostics as a first-class subsystem, not an accumulation of print statements.

The immediate goal is not “more verbose output.” It is a record that can answer, with bounded privacy cost:

```text
What operation was attempted?
Which exact stage failed?
What state changed before failure?
What evidence was lost or dropped?
Did the system recover?
Can the user act on it?
Which events prove that conclusion?
Is the report safe to share?
```

The current join-outcome design already demonstrates that Mewtual can use stable codes and actionable explanations. Generalising that pattern through a native `DiagnosticHub`, structured traces, per-section controls, deterministic findings, and the existing consent-based GitHub flow would turn debugging from guesswork into a reproducible product feature.
