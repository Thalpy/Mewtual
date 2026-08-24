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

/// The most of a runtime field name that is kept.
///
/// Only the `tracing` bridge produces these; a literal from our own source is already `'static` and
/// needs no bound. A field name is a short identifier in every sane case, which is exactly why it
/// should not be the thing a memory bound depends on.
pub const MAX_FIELD_NAME: usize = 64;

/// The most of an emitting module path that is kept.
///
/// A target is a module path, so a long one is a deeply nested module rather than an attack. It is
/// bounded for the same reason as everything else here: the type is a `String` from a caller, and
/// an event that can hold an unbounded one holds it in the ring and writes it to the file.
pub const MAX_TARGET: usize = 200;

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

/// The name of one field on an event.
///
/// An enum rather than a `&'static str` because field names arrive from two places with different
/// lifetimes. New code writes literals, which are already `'static`. The `tracing` bridge gets
/// them as runtime strings, and the first attempt at squaring that interned them in a global table
/// behind a mutex, which put a second lock acquisition on the emitting thread for *every field of
/// every event* and leaked a string per distinct name. Diagnostics that add lock contention to the
/// hot path become a cause of the stalls they exist to explain, so the allocation moves into the
/// event where it belongs and no shared state is touched at all.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FieldName {
    /// A literal from the source. Every name new code uses.
    Static(&'static str),
    /// A name that arrived at runtime, from the `tracing` bridge.
    Owned(Box<str>),
}

impl FieldName {
    pub fn as_str(&self) -> &str {
        match self {
            FieldName::Static(name) => name,
            FieldName::Owned(name) => name,
        }
    }
}

impl From<&'static str> for FieldName {
    fn from(name: &'static str) -> Self {
        FieldName::Static(name)
    }
}

/// Whether a character in a field name would fabricate structure in a rendering.
///
/// Every rendering of a field is some form of `name=value` separated by spaces, so a name that
/// contains a space or an equals sign is a name that can invent a second field. `x=1 peer` reads
/// as a peer reference the event never carried, and the debug console composes the same shape from
/// the structured view, so the two would agree with each other about a field nothing recorded.
/// Whitespace in general rather than the space character in particular, because `U+2028` is a line
/// separator that is not a control character, and a reader that breaks a line on it sees one row
/// where the event produced none.
fn forges_structure(c: char) -> bool {
    c == '=' || c.is_whitespace() || c.is_control()
}

impl From<String> for FieldName {
    /// Bounded and stripped of separators, because this is the runtime path.
    ///
    /// The `Static` arm is a literal from our own source and needs no bound; this one arrives from
    /// the `tracing` bridge and from the webview, where the name is whatever a call site anywhere
    /// in the dependency tree chose. A field *name* is a short identifier in every sane case, and
    /// both the bound and the substitution are here so that "every sane case" is not the thing
    /// holding up the memory bound and the report's grammar. See [`forges_structure`] for what a
    /// name with a separator in it can make a reader believe.
    ///
    /// An ordinary name is neither long nor odd, so the usual path checks and keeps the buffer the
    /// bridge already allocated rather than building a second one per field on the emitting thread.
    fn from(name: String) -> Self {
        if name.len() <= MAX_FIELD_NAME && !name.chars().any(forges_structure) {
            return FieldName::Owned(name.into_boxed_str());
        }
        FieldName::Owned(
            name.chars()
                .map(|c| if forges_structure(c) { '_' } else { c })
                .take(MAX_FIELD_NAME)
                .collect::<String>()
                .into_boxed_str(),
        )
    }
}

impl std::fmt::Display for FieldName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
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
    pub fields: Vec<(FieldName, SafeValue)>,
    /// How many fields [`MAX_FIELDS`] refused.
    ///
    /// Carried on the event because a truncated event otherwise reads as a complete one: the reader
    /// sees thirty-two fields and no reason to suspect a thirty-third, which in the case that
    /// actually reaches the cap is the field they were looking for. Rule 4 of this crate is that
    /// every bound is explicit and everything lost to one is counted; this is that rule applied to
    /// the one bound whose loss used to be silent.
    pub fields_dropped: u32,
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
            fields_dropped: 0,
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
        let target = target.into();
        self.target = if target.len() <= MAX_TARGET {
            target
        } else {
            target.chars().take(MAX_TARGET).collect()
        };
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

    /// Add one field, up to [`MAX_FIELDS`], counting anything past it.
    ///
    /// Refuses rather than growing or panicking. Both alternatives are worse: growing hands a
    /// hostile producer the memory, and panicking means recording an event can kill the thing
    /// recording it, which is the failure mode this whole subsystem was built to remove.
    ///
    /// It used to refuse in silence, which made an event that lost a field indistinguishable from
    /// one that never had it. See [`DiagnosticEvent::fields_dropped`] for why that is worse than
    /// the loss itself.
    pub fn field(mut self, name: impl Into<FieldName>, value: impl Into<SafeValue>) -> Self {
        if self.fields.len() < MAX_FIELDS {
            self.fields.push((name.into(), value.into()));
        } else {
            self.fields_dropped = self.fields_dropped.saturating_add(1);
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
    use crate::config::CaptureMode;
    use crate::redact::{
        AddressValue, RefDomain, SafeText, SessionSalt, MAX_ADDRESS_CHARS, MAX_SAFE_TEXT,
    };

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
        // And the loss is admitted, or the reader of the resulting report has thirty-two fields
        // and no reason to suspect there were ninety-six. Found by adversarial review (P3-013).
        assert_eq!(event.fields_dropped, (MAX_FIELDS * 3) as u32);
    }

    /// A field name arrives from the `tracing` bridge and from the webview as a runtime string, and
    /// every rendering of a field is `name=value` separated by spaces. A name that carries those
    /// separators invents a field: `peer=peer-000000000000 x` reads as a reference to a peer this
    /// event was never about, in a report someone is using to decide what went wrong.
    #[test]
    fn a_runtime_field_name_cannot_fabricate_a_second_field() {
        let event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST")
            .field("peer=peer-000000000000 x".to_string(), 1u64)
            .field("first\nsecond".to_string(), 2u64)
            .field("split\u{2028}here".to_string(), 3u64);
        for (name, _) in &event.fields {
            let name = name.as_str();
            assert!(
                !name.contains('=') && !name.chars().any(|c| c.is_whitespace() || c.is_control()),
                "a name that can invent a field: {name:?}"
            );
        }
        assert_eq!(event.fields[0].0.as_str(), "peer_peer-000000000000_x");
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

    /// Every part of an event that arrives as an owned string has a bound.
    ///
    /// The field cap above bounds how *many* there are and says nothing about how large each one
    /// is, so an event could hold a hundred megabytes in thirty-two fields and satisfy it. These
    /// are the parts a caller supplies as a `String` or a `&str`, where nothing about the type
    /// says "short". Found by adversarial review (P3-008).
    #[test]
    fn every_owned_part_of_an_event_is_bounded() {
        let huge = "x".repeat(1_000_000);

        let event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST")
            .target(huge.clone())
            .field(huge.clone(), SafeText::describe(&huge))
            .field("address", AddressValue::new(&huge));

        assert!(event.target.len() <= MAX_TARGET, "the emitting module path");
        assert!(
            event.fields[0].0.as_str().len() <= MAX_FIELD_NAME,
            "a runtime field name"
        );
        assert!(
            event.fields[0].1.render(CaptureMode::Full).len() <= MAX_SAFE_TEXT * 4,
            "the value, which SafeText already bounded"
        );
        // The literal address, which only a deliberately chosen mode renders at all.
        let rendered = event.fields[1].1.render(CaptureMode::Full);
        assert!(
            rendered.len() <= MAX_ADDRESS_CHARS * 4,
            "{}",
            rendered.len()
        );

        // The whole event, in the only terms that matter: what it costs to hold.
        let total: usize = event.target.len()
            + event
                .fields
                .iter()
                .map(|(name, value)| name.as_str().len() + value.render(CaptureMode::Full).len())
                .sum::<usize>();
        assert!(total < 64 * 1024, "one event held {total} bytes");
    }

    /// A short name must not be trimmed, or every ordinary field pays for the hostile case.
    #[test]
    fn an_ordinary_field_name_is_untouched() {
        let event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST")
            .target("catcoms_app::actor")
            .field("direct_candidates".to_string(), 4u64);
        assert_eq!(event.target, "catcoms_app::actor");
        assert_eq!(event.fields[0].0.as_str(), "direct_candidates");
    }

    #[test]
    fn fields_keep_the_order_they_were_added_in() {
        // Rendering is deterministic, and determinism starts here: a report that reorders its own
        // fields between runs cannot be diffed, hashed, or compared between two peers.
        let event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST")
            .field("first", 1u64)
            .field("second", 2u64)
            .field("third", 3u64);
        let names: Vec<_> = event.fields.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["first", "second", "third"]);
    }
}
