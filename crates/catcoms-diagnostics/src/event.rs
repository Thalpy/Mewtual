//! The canonical diagnostic event: one shape that every observation is expressed in.
//!
//! # Why one shape
//!
//! The app used to report failures through at least nine unrelated mechanisms: structured
//! `tracing`, raw `eprintln!`, `Result<T, String>` over the Tauri bridge, forwarded console lines,
//! `error = String(e)` in the UI, silently ignored `catch {}` blocks, in-memory snapshots, emitted
//! events, and ad hoc plain-text join logs. Each had different fields, lifetimes, clocks, naming
//! and privacy rules.
//!
//! The cost was not verbosity, it was that no sequence of them could answer an ordinary question.
//! Given "the message did not arrive", nobody could establish whether the command reached Rust,
//! whether the actor was still alive, whether the request went out, whether a reply came back,
//! whether state changed, whether persistence completed, whether the event was emitted, whether
//! the webview received it, or whether the UI decided to ignore it. Ten stages, no common record.
//!
//! # Identity, order and time
//!
//! Three separate things that a single timestamp conflates:
//!
//! * [`DiagnosticEvent::seq`] is local observation order, and is the only total order here.
//! * [`DiagnosticEvent::at_ms`] is wall time, for lining up against a user's account of events.
//!   It can jump: clocks get corrected.
//! * [`DiagnosticEvent::monotonic_ms`] never goes backwards, so a duration derived from it is
//!   trustworthy even across a correction.
//!
//! Causality comes from [`TraceId`] and the span parentage, never from timestamps. Two events a
//! microsecond apart on different threads say nothing about which caused which.

use crate::config::{Level, Section};
use crate::redact::{SafeValue, SessionRef};

/// The schema version, carried in every export.
///
/// An exported bundle outlives the build that produced it, and a reader with a newer tool has to
/// know what it is looking at. Bumped when a field's meaning changes, never for an addition.
pub const SCHEMA_VERSION: u32 = 1;

/// The most fields one event may carry.
///
/// An event is an observation, not a serialised object graph. The cap exists because the hostile
/// case (a compromised webview, a malformed remote payload) is a producer with no such restraint.
pub const MAX_FIELDS: usize = 32;

/// Ties every stage of one user-visible operation together.
///
/// The thing whose absence made concurrent sends, reconnects, server switches and retries
/// indistinguishable from each other. Random per operation; not derived from anything about the
/// user, and never sent onto the peer-to-peer wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct TraceId(pub u64);

impl TraceId {
    /// The rendered form: short enough to read off a screen and quote in a bug report.
    pub fn as_hex(self) -> String {
        format!("{:016x}", self.0)
    }

    /// The first four characters, which is how a trace is referred to in prose.
    pub fn short(self) -> String {
        self.as_hex()[..4].to_string()
    }

    pub fn is_set(self) -> bool {
        self.0 != 0
    }
}

/// Identifies one stage inside a trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct SpanId(pub u64);

impl SpanId {
    pub fn as_hex(self) -> String {
        format!("{:016x}", self.0)
    }
    pub fn is_set(self) -> bool {
        self.0 != 0
    }
}

/// Where in its life an operation is.
///
/// The distinction that makes a stall diagnosable. Without it, an operation that started and never
/// finished looks exactly like one that was never attempted: both are "no success line in the
/// log", and only one of them is a bug in the thing you are looking at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Phase {
    Start,
    Progress,
    Success,
    Failure,
    Cancel,
    Timeout,
    /// Not part of an operation: a state transition, a measurement, an observation.
    Observation,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Start => "start",
            Phase::Progress => "progress",
            Phase::Success => "success",
            Phase::Failure => "failure",
            Phase::Cancel => "cancel",
            Phase::Timeout => "timeout",
            Phase::Observation => "observation",
        }
    }
}

/// The subjects an event can be about.
///
/// Every one is a [`SessionRef`], so this struct cannot carry a raw identifier however it is
/// populated.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Refs {
    pub server: Option<SessionRef>,
    pub channel: Option<SessionRef>,
    pub peer: Option<SessionRef>,
    pub document: Option<SessionRef>,
    pub transfer: Option<SessionRef>,
}

impl Refs {
    pub fn is_empty(&self) -> bool {
        self.server.is_none()
            && self.channel.is_none()
            && self.peer.is_none()
            && self.document.is_none()
            && self.transfer.is_none()
    }
}

/// One observation.
///
/// Built through [`DiagnosticEvent::new`] and the setters, which is the only way: the fields are
/// public for reading but the constructor is what enforces the field cap.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosticEvent {
    /// Local observation order. Assigned by the hub, so it is zero until then.
    pub seq: u64,
    /// Wall-clock milliseconds. May jump; see the module docs.
    pub at_ms: u64,
    /// Milliseconds from a process-local origin that never moves backwards.
    pub monotonic_ms: u64,
    pub section: Section,
    pub level: Level,
    /// A stable `AREA.COMPONENT.OUTCOME` identifier, e.g. `JOIN.ROUTES.EXHAUSTED`.
    ///
    /// `&'static str` for two reasons. It cannot carry runtime data, so a code can never become a
    /// vector for content or for attacker-chosen text in an issue title. And it is stable across
    /// rewording, so tests, issue de-duplication and support instructions do not break when the
    /// prose around it improves.
    pub code: &'static str,
    pub phase: Phase,
    /// The operation this belongs to, e.g. `join_server`. Empty for a bare observation.
    pub operation: &'static str,
    pub trace: TraceId,
    pub span: SpanId,
    pub parent_span: SpanId,
    pub refs: Refs,
    /// How long the operation took, on a terminal phase.
    pub duration_ms: Option<u64>,
    /// Which retry this is, when an operation is being retried.
    pub attempt: Option<u32>,
    /// The emitting module, for locating the code that said this.
    pub target: String,
    /// Ordered so rendering is deterministic. See [`crate::render`].
    pub fields: Vec<(&'static str, SafeValue)>,
}

impl DiagnosticEvent {
    /// A new event. Everything else is optional and set by the builders below.
    pub fn new(section: Section, level: Level, code: &'static str) -> Self {
        DiagnosticEvent {
            seq: 0,
            at_ms: 0,
            monotonic_ms: 0,
            section,
            level,
            code,
            phase: Phase::Observation,
            operation: "",
            trace: TraceId::default(),
            span: SpanId::default(),
            parent_span: SpanId::default(),
            refs: Refs::default(),
            duration_ms: None,
            attempt: None,
            target: String::new(),
            fields: Vec::new(),
        }
    }

    pub fn error(section: Section, code: &'static str) -> Self {
        Self::new(section, Level::Error, code)
    }
    pub fn warn(section: Section, code: &'static str) -> Self {
        Self::new(section, Level::Warn, code)
    }
    pub fn info(section: Section, code: &'static str) -> Self {
        Self::new(section, Level::Info, code)
    }

    pub fn phase(mut self, phase: Phase) -> Self {
        self.phase = phase;
        self
    }

    pub fn operation(mut self, operation: &'static str) -> Self {
        self.operation = operation;
        self
    }

    pub fn trace(mut self, trace: TraceId) -> Self {
        self.trace = trace;
        self
    }

    pub fn span(mut self, span: SpanId, parent: SpanId) -> Self {
        self.span = span;
        self.parent_span = parent;
        self
    }

    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = target.into();
        self
    }

    pub fn took(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    pub fn attempt(mut self, attempt: u32) -> Self {
        self.attempt = Some(attempt);
        self
    }

    pub fn refs(mut self, refs: Refs) -> Self {
        self.refs = refs;
        self
    }

    /// Add one field, up to [`MAX_FIELDS`].
    ///
    /// Silently ignores anything past the cap rather than growing or panicking. Both alternatives
    /// are worse: growing hands a hostile producer the memory, and panicking means recording an
    /// event can kill the thing recording it, which is the failure mode this whole subsystem was
    /// built to remove.
    pub fn field(mut self, name: &'static str, value: impl Into<SafeValue>) -> Self {
        if self.fields.len() < MAX_FIELDS {
            self.fields.push((name, value.into()));
        }
        self
    }

    /// Whether this event's rendering changes with the capture mode, i.e. whether it holds
    /// anything that only a deliberately chosen mode reveals.
    pub fn is_mode_sensitive(&self) -> bool {
        self.fields.iter().any(|(_, v)| v.is_mode_sensitive())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::redact::{AddressValue, RefDomain, SafeText, SessionSalt};

    #[test]
    fn a_trace_reads_the_same_way_everywhere_it_is_quoted() {
        let trace = TraceId(0x7f2c_1234_5678_9abc);
        assert_eq!(trace.as_hex(), "7f2c123456789abc");
        assert_eq!(trace.short(), "7f2c");
        assert!(trace.is_set());
        assert!(
            !TraceId::default().is_set(),
            "an unset trace is distinguishable"
        );
    }

    /// A producer with no restraint is the case this cap exists for: a compromised webview, a
    /// malformed remote payload, a loop building fields from a collection.
    #[test]
    fn an_event_cannot_be_grown_without_bound() {
        let mut event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST");
        for n in 0..(MAX_FIELDS * 4) {
            event = event.field("filler", n as u64);
        }
        assert_eq!(event.fields.len(), MAX_FIELDS);
    }

    /// Overflowing the cap must not panic. Recording an event killing the thing that recorded it
    /// is the exact failure this subsystem exists to remove.
    #[test]
    fn overflowing_the_field_cap_is_not_fatal() {
        let mut event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST");
        for _ in 0..1000 {
            event = event.field("x", 1u64);
        }
        assert_eq!(event.code, "SYNC.TEST", "the event survives intact");
    }

    #[test]
    fn an_event_carries_the_whole_shape_of_one_failure() {
        let salt = SessionSalt::for_tests(3);
        let event = DiagnosticEvent::warn(Section::Join, "JOIN.ROUTES.EXHAUSTED")
            .phase(Phase::Failure)
            .operation("join_server")
            .trace(TraceId(0x7f2c))
            .took(60_123)
            .attempt(4)
            .refs(Refs {
                server: Some(salt.reference(RefDomain::Server, b"group-1")),
                ..Refs::default()
            })
            .field("direct_candidates", 4u64)
            .field("relay_candidates", 0u64)
            .field(
                "reason",
                SafeText::describe("no advertised route completed"),
            );

        assert_eq!(event.phase, Phase::Failure);
        assert_eq!(event.duration_ms, Some(60_123));
        assert_eq!(event.attempt, Some(4));
        assert_eq!(event.fields.len(), 3);
        assert!(event.refs.server.is_some());
        assert!(!event.refs.is_empty());
    }

    /// The export preview has to tell a user what turning Enhanced on would actually reveal,
    /// rather than making them find out by doing it.
    #[test]
    fn an_event_knows_whether_it_holds_anything_a_mode_change_would_reveal() {
        let plain = DiagnosticEvent::info(Section::Sync, "SYNC.OK").field("ops", 3u64);
        assert!(!plain.is_mode_sensitive());

        let addressed = DiagnosticEvent::warn(Section::Transport, "NET.DIAL.FAILED").field(
            "address",
            AddressValue::new("/ip6/2001:db8::1/udp/1/quic-v1"),
        );
        assert!(addressed.is_mode_sensitive());
    }

    #[test]
    fn fields_keep_the_order_they_were_added_in() {
        // Rendering is deterministic, and determinism starts here: a report that reorders its own
        // fields between runs cannot be diffed, hashed, or compared between two peers.
        let event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST")
            .field("first", 1u64)
            .field("second", 2u64)
            .field("third", 3u64);
        let names: Vec<_> = event.fields.iter().map(|(n, _)| *n).collect();
        assert_eq!(names, ["first", "second", "third"]);
    }
}
