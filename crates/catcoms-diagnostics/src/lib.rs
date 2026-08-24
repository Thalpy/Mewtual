//! Mewtual's diagnostics: one canonical record of what the application did.
//!
//! # What this replaces
//!
//! The app was not short of log lines. It was short of a record that could answer a question.
//! Observations were spread across structured `tracing`, raw `eprintln!`, `Result<T, String>` over
//! the Tauri bridge, forwarded console output, `error = String(e)` in the UI, ignored `catch {}`
//! blocks, point-in-time snapshots, emitted events and ad hoc plain-text join logs. Each had its
//! own fields, lifetime, clock, naming and privacy rules, and none of them correlated.
//!
//! Given "my message did not arrive", the existing evidence could not establish which of ten
//! stages failed. Given "the unread badge did not appear", it could not distinguish a backend that
//! never emitted from a webview that never received. Given an hour of isolation, it recorded no
//! dial failure anywhere, because the layer that knew was held quiet to keep addresses out of a
//! file users were expected to share.
//!
//! # The shape of the answer
//!
//! * [`event::DiagnosticEvent`] is the one shape every observation takes, carrying a stable code,
//!   a section, a phase, and the trace that ties it to the rest of its operation.
//! * [`redact`] decides what a field is allowed to be. Identifiers enter only as keyed
//!   per-session references, so a peer id cannot leak however carelessly a call site is written;
//!   literal addresses appear only under a mode the user deliberately chose.
//! * [`config`] separates *whether* a session is being captured from *which parts* of the app feed
//!   it, which is what makes the transport layer's detail available to somebody debugging a
//!   connection without putting it in every report.
//! * [`ring`] keeps one chronological timeline, bounded, with counters that outlive the events
//!   they counted. Sections are a filter over it, never a separate store: a send failure usually
//!   depends on the exact interleaving, and per-subsystem logs destroy exactly that.
//! * [`hub::DiagnosticHub`] owns the store, the session salt and the clock, so every subsystem
//!   names the same peer the same way.
//! * [`render`] turns events into bytes deterministically, which is what makes golden tests,
//!   report hashes and peer-to-peer report comparison possible at all.
//!
//! # Rules this crate holds itself to
//!
//! 1. It is more reliable than the code it observes.
//! 2. A disabled capture mode is actually disabled, immediately, without a restart.
//! 3. Nothing it does can block or panic a caller. Recording an error must never destroy the
//!    evidence of that error.
//! 4. Every bound is explicit, and every event lost to one is counted.
//! 5. It never reports its own failures through itself.
//!
//! # Status
//!
//! The event contract, the privacy model, the store and the renderers. The correlation wrappers
//! that populate traces across the IPC boundary, the rule engine, the checks and the export bundle
//! build on this and land after it; see `docs/reviews/` for the sequence.

pub mod config;
pub mod event;
pub mod export;
pub mod hub;
pub mod redact;
pub mod render;
pub mod ring;

pub use config::{CaptureConfig, CaptureGate, CaptureMode, ConsoleView, Level, Section, SECTIONS};
pub use event::{
    DiagnosticEvent, FieldName, Phase, Refs, SpanId, TraceId, MAX_FIELDS, MAX_FIELD_NAME,
    MAX_TARGET, SCHEMA_VERSION,
};
pub use hub::{DiagnosticHub, DEFAULT_CAPACITY};
pub use redact::{
    AddressFamily, AddressValue, BridgedMessage, RefDomain, SafeText, SafeValue, SessionRef,
    SessionSalt, MAX_ADDRESS_CHARS, MAX_BRIDGED_MESSAGE, MAX_SAFE_TEXT,
};
pub use render::{event_json, event_line, event_view, EventView, ViewField};
pub use ring::{Ring, RingStats, StoredEvent};
