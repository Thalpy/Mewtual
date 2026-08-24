//! Turning events into text, deterministically.
//!
//! # Why determinism is a requirement rather than a nicety
//!
//! The same events and the same configuration must produce byte-identical output. That buys four
//! things the review asks for and one it does not say out loud:
//!
//! * golden tests that actually fail when the exporter changes;
//! * a report hash, so an issue can be de-duplicated without reading it;
//! * two peers' reports being diffable against each other, which is the only way some sync bugs
//!   are localisable at all;
//! * proof that the exporter did not quietly omit a section;
//! * and the unstated one: a user who exports twice and gets two different files stops trusting
//!   the tool, correctly.
//!
//! So: fields render in insertion order, never from a hash map; nothing is formatted through a
//! locale; and no timestamp appears except the ones the events themselves carry.
//!
//! # The JSON is hand-written
//!
//! No `serde`. Partly because this crate's job is capture and it should not take a serialisation
//! dependency to do it, but mostly because determinism here means controlling field order exactly,
//! and the value type is a small closed enum that takes about forty lines to write out.
//!
//! # One event, one row, one reading
//!
//! Both export renderings are grammars, and a value that carries the grammar's own punctuation can
//! forge a record in one. A `tracing` message keeps its newlines on purpose (a stack trace is why
//! bridged messages exist at all), so an unescaped one puts a line into `report.txt` that neither a
//! reader nor a screenshot can tell from a real event. Field names are whatever the bridge or the
//! webview chose and are not unique, so rendering them as a JSON object emitted the same key twice,
//! which readers resolve differently: first wins, last wins, or refuse.
//!
//! Neither is a formatting nit. A report that its reader and its author read differently is worse
//! than no report, because it is believed. So: every rendered value is escaped where it could break
//! the grammar around it, fields render as an ordered array rather than as a map, and one event
//! produces exactly one row. Found by adversarial review (P3-013).
//!
//! # Three renderings, one event
//!
//! [`event_line`] is text for a human, [`event_json`] is a line of an export bundle, and
//! [`event_view`] is the structured form the debug console reads. They live in one file so they
//! cannot quietly diverge about what an event contains: the console used to read a *different*
//! projection living in another crate, which flattened the section, phase, span and references
//! away and rendered every value at a hard-coded Enhanced. Most of what the app recorded was
//! therefore discarded before it reached the only tool anyone actually looks at.

use crate::config::CaptureMode;
use crate::event::{DiagnosticEvent, SCHEMA_VERSION};
use crate::redact::SafeValue;

/// Escape a string for a JSON document.
///
/// Control characters are escaped rather than stripped: this is the export path, and a reader
/// diffing two reports needs to see that something odd was present, not have it silently removed.
fn json_escape(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

fn json_string(text: &str, out: &mut String) {
    out.push('"');
    json_escape(text, out);
    out.push('"');
}

/// One value as JSON.
///
/// Numbers stay numbers so a reader can sort and compare on them; everything else is a string.
/// A duration is a number of milliseconds rather than the `60123ms` the text rendering uses,
/// because the two outputs have different readers.
fn json_value(value: &SafeValue, mode: CaptureMode, out: &mut String) {
    match value {
        SafeValue::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
        SafeValue::Count(v) => out.push_str(&v.to_string()),
        SafeValue::Delta(v) => out.push_str(&v.to_string()),
        SafeValue::Duration(v) => out.push_str(&v.to_string()),
        other => json_string(&other.render(mode), out),
    }
}

/// One event as a single JSON object, for `events.jsonl`.
///
/// Optional fields are omitted rather than emitted as null. A reader can tell "no duration was
/// recorded" from an absent key just as well, and every omitted key is a byte a bug report does
/// not carry.
pub fn event_json(event: &DiagnosticEvent, mode: CaptureMode) -> String {
    let mut out = String::with_capacity(256);
    out.push('{');
    out.push_str(&format!("\"schema\":{SCHEMA_VERSION}"));
    out.push_str(&format!(",\"seq\":{}", event.seq));
    out.push_str(&format!(",\"at_ms\":{}", event.at_ms));
    out.push_str(&format!(",\"monotonic_ms\":{}", event.monotonic_ms));
    out.push_str(",\"section\":");
    json_string(event.section.as_str(), &mut out);
    out.push_str(",\"level\":");
    json_string(event.level.as_str(), &mut out);
    out.push_str(",\"code\":");
    json_string(event.code, &mut out);
    out.push_str(",\"phase\":");
    json_string(event.phase.as_str(), &mut out);

    if !event.operation.is_empty() {
        out.push_str(",\"operation\":");
        json_string(event.operation, &mut out);
    }
    if !event.target.is_empty() {
        out.push_str(",\"target\":");
        json_string(&event.target, &mut out);
    }
    if event.trace.is_set() {
        out.push_str(",\"trace\":");
        json_string(&event.trace.as_hex(), &mut out);
    }
    if event.span.is_set() {
        out.push_str(",\"span\":");
        json_string(&event.span.as_hex(), &mut out);
    }
    if event.parent_span.is_set() {
        out.push_str(",\"parent_span\":");
        json_string(&event.parent_span.as_hex(), &mut out);
    }
    if let Some(duration) = event.duration_ms {
        out.push_str(&format!(",\"duration_ms\":{duration}"));
    }
    if let Some(attempt) = event.attempt {
        out.push_str(&format!(",\"attempt\":{attempt}"));
    }

    if !event.refs.is_empty() {
        out.push_str(",\"refs\":{");
        let mut first = true;
        for (name, value) in ref_slots(event) {
            if let Some(reference) = value {
                if !first {
                    out.push(',');
                }
                first = false;
                json_string(name, &mut out);
                out.push(':');
                json_string(reference.as_str(), &mut out);
            }
        }
        out.push('}');
    }

    // An array of `{name,value}` rather than an object, because field names are not unique.
    //
    // Nothing rejects a repeated name: the `tracing` bridge takes whatever a call site declared and
    // the webview sends what it likes, so `fields` as an object could emit the same key twice. JSON
    // says nothing about what a reader should then do, and readers duly disagree; two people on one
    // report would read two different values and neither would find out. An array carries exactly
    // what was recorded, in the order it was recorded, and asks the reader to resolve nothing. The
    // `refs` object above stays an object: its five keys are literals from one array in this file
    // and cannot repeat.
    if !event.fields.is_empty() {
        out.push_str(",\"fields\":[");
        for (at, (name, value)) in event.fields.iter().enumerate() {
            if at > 0 {
                out.push(',');
            }
            out.push_str("{\"name\":");
            json_string(name.as_str(), &mut out);
            out.push_str(",\"value\":");
            json_value(value, mode, &mut out);
            out.push('}');
        }
        out.push(']');
    }
    // Omitted when nothing was lost, like every other optional key, but never inferable from the
    // count: a reader who sees thirty-two fields has no way to know a thirty-third was refused, and
    // the event that reaches the cap is exactly the one whose extra field someone is looking for.
    if event.fields_dropped > 0 {
        out.push_str(&format!(",\"fields_dropped\":{}", event.fields_dropped));
    }

    // Says what the reader is holding. A Safe report and a Full one look similar and mean very
    // different things, so the mode travels with every line rather than only in a manifest that
    // can be separated from it.
    out.push_str(",\"capture\":");
    json_string(mode.as_str(), &mut out);
    out.push('}');
    out
}

/// Escape a rendered name or value for a report row.
///
/// The row is the unit a reader works in: they split a report on line breaks and expect each piece
/// to be one event. A bridged `tracing` message keeps its newlines deliberately, so without this a
/// message reading `dial failed\n0001234  ERROR  security      MLS.KEY.LEAKED` puts a line into
/// `report.txt` that is indistinguishable from an event the app never recorded. A literal address
/// under a mode that renders them is the same hole by a different route.
///
/// Carriage returns and the rest of the control characters go with the newlines: a line that can
/// move the terminal cursor can overwrite the line above it, which is the same forgery against a
/// reader who is watching rather than parsing. `U+2028` is neither a newline nor a control
/// character and is treated as one, because a reader in a webview or an editor breaks a line on it.
///
/// Names are escaped as well as values even though [`crate::event::FieldName`] already cleans the
/// runtime ones: a guarantee this file makes about its own output should not depend on a bound
/// enforced in another module, which is how it would be lost the day a third source of names
/// appears.
fn push_row_escaped(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() || c == '\u{2028}' || c == '\u{2029}' => {
                // The same `\u00XX` spelling the JSON escaping uses, so a reader who meets one in
                // either rendering reads it the same way.
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
}

/// One event as a line of text, in the shape the console and `report.txt` both use.
///
/// The console renders from this, and copy uses it too. That is the point: if copy had its own
/// formatting the two could disagree, and a pasted report that does not match the screenshot it
/// came with makes the reader work out which one is lying before they can start on the bug.
pub fn event_line(event: &DiagnosticEvent, mode: CaptureMode) -> String {
    let mut out = String::with_capacity(160);
    out.push_str(&format!("{:07}  ", event.seq));
    out.push_str(&format!("{:<5} ", event.level.as_str()));
    out.push_str(&format!("{:<13} ", event.section.as_str()));
    if event.trace.is_set() {
        out.push_str(&format!("trace={} ", event.trace.short()));
    }
    out.push_str(event.code);
    if event.phase != crate::event::Phase::Observation {
        out.push_str(&format!(" phase={}", event.phase.as_str()));
    }
    if let Some(duration) = event.duration_ms {
        out.push_str(&format!(" duration={duration}ms"));
    }
    if let Some(attempt) = event.attempt {
        out.push_str(&format!(" attempt={attempt}"));
    }
    for (name, reference) in ref_slots(event) {
        if let Some(value) = reference {
            out.push_str(&format!(" {name}={value}"));
        }
    }
    for (name, value) in &event.fields {
        out.push(' ');
        push_row_escaped(name.as_str(), &mut out);
        out.push('=');
        push_row_escaped(&value.render(mode), &mut out);
    }
    if event.fields_dropped > 0 {
        out.push_str(&format!(" fields_dropped={}", event.fields_dropped));
    }
    out
}

/// The named references an event may carry, in the order every rendering lists them.
///
/// One array rather than the same five-entry literal repeated in each renderer, because a
/// reference added to [`crate::event::Refs`] and forgotten in one of them is a subject that is
/// recorded and never shown, which is indistinguishable from not having been recorded.
fn ref_slots(event: &DiagnosticEvent) -> [(&'static str, &Option<crate::redact::SessionRef>); 5] {
    [
        ("server", &event.refs.server),
        ("channel", &event.refs.channel),
        ("peer", &event.refs.peer),
        ("document", &event.refs.document),
        ("transfer", &event.refs.transfer),
    ]
}

/// One field of an event, rendered at a capture mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewField {
    pub name: String,
    pub value: String,
    /// Whether this value would say more under a higher capture mode.
    ///
    /// Carried so a reader can be told what they are *not* seeing. A console that renders a
    /// reduced address identically to a literal one leaves the user to discover the difference by
    /// switching modes and comparing, which nobody does.
    pub sensitive: bool,
}

/// One canonical event, rendered at a mode, with nothing flattened away.
///
/// The shape the debug console consumes. Every field of [`DiagnosticEvent`] survives, including
/// the ones a line rendering has to drop for space: the canonical section as well as the console
/// view it falls under, the full trace as well as the short form, and the span parentage that says
/// which stage of an operation this was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventView {
    pub seq: u64,
    pub at_ms: u64,
    pub monotonic_ms: u64,
    /// The canonical section, one of twenty-two.
    pub section: &'static str,
    /// The console section this falls under, one of six. Stated natively so the console never has
    /// to guess a section from a target name or by searching the text for the word "voice".
    pub view: &'static str,
    pub level: &'static str,
    pub code: &'static str,
    pub phase: &'static str,
    pub operation: &'static str,
    /// Sixteen hex characters, or empty when this event belongs to no operation.
    pub trace: String,
    pub span: String,
    pub parent_span: String,
    pub refs: Vec<(&'static str, String)>,
    pub duration_ms: Option<u64>,
    pub attempt: Option<u32>,
    pub target: String,
    pub fields: Vec<ViewField>,
    /// Fields the event's cap refused, so the console can say the event is short rather than
    /// showing a truncated one as if it were whole. Carried here as well as in the export because
    /// the three renderings are in one file precisely so they cannot disagree about what an event
    /// contains, and "it contained more than this" is part of what it contains.
    pub fields_dropped: u32,
    /// The mode this was rendered at.
    ///
    /// Travels with every event rather than only in the payload's header, for the same reason
    /// [`event_json`] carries it on every line: a Safe rendering and a Full one look alike and mean
    /// very different things, and an excerpt that has been separated from its header must still say
    /// which it is.
    pub capture: &'static str,
}

/// One event as the debug console reads it.
///
/// The mode is a parameter, never a constant. The projection this replaced hard-coded Enhanced,
/// which meant the console showed literal addresses whatever the user had chosen and the mode
/// control could not have worked even once it existed.
pub fn event_view(event: &DiagnosticEvent, mode: CaptureMode) -> EventView {
    EventView {
        seq: event.seq,
        at_ms: event.at_ms,
        monotonic_ms: event.monotonic_ms,
        section: event.section.as_str(),
        view: event.section.view().as_str(),
        level: event.level.as_str(),
        code: event.code,
        phase: event.phase.as_str(),
        operation: event.operation,
        trace: if event.trace.is_set() {
            event.trace.as_hex()
        } else {
            String::new()
        },
        span: if event.span.is_set() {
            event.span.as_hex()
        } else {
            String::new()
        },
        parent_span: if event.parent_span.is_set() {
            event.parent_span.as_hex()
        } else {
            String::new()
        },
        refs: ref_slots(event)
            .into_iter()
            .filter_map(|(name, slot)| slot.as_ref().map(|r| (name, r.as_str().to_string())))
            .collect(),
        duration_ms: event.duration_ms,
        attempt: event.attempt,
        target: event.target.clone(),
        fields: event
            .fields
            .iter()
            .map(|(name, value)| ViewField {
                name: name.as_str().to_string(),
                value: value.render(mode),
                sensitive: value.is_mode_sensitive(),
            })
            .collect(),
        fields_dropped: event.fields_dropped,
        capture: mode.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Section;
    use crate::event::{Phase, Refs, SpanId, TraceId, MAX_FIELDS};
    use crate::redact::{AddressValue, BridgedMessage, RefDomain, SafeText, SessionSalt};

    fn sample() -> DiagnosticEvent {
        let salt = SessionSalt::for_tests(3);
        let mut event = DiagnosticEvent::warn(Section::Join, "JOIN.ROUTES.EXHAUSTED")
            .phase(Phase::Failure)
            .operation("join_server")
            .trace(TraceId(0x7f2c_0000_0000_0001))
            .span(SpanId(0x91ab), SpanId(0x6dc4))
            .target("catcoms_sync")
            .took(60_123)
            .attempt(4)
            .refs(Refs {
                server: Some(salt.reference(RefDomain::Server, b"group-1")),
                peer: Some(salt.reference(RefDomain::Peer, b"12D3KooWabc")),
                ..Refs::default()
            })
            .field("direct_candidates", 4u64)
            .field("relay_candidates", 0u64)
            .field(
                "address",
                AddressValue::new("/ip6/2001:db8::1/udp/31484/quic-v1"),
            )
            .field(
                "reason",
                SafeText::describe("no advertised route completed"),
            );
        event.seq = 4812;
        event.at_ms = 1_787_000_000_000;
        event.monotonic_ms = 12_443;
        event
    }

    /// The property every other guarantee rests on: same input, same bytes.
    #[test]
    fn rendering_the_same_event_twice_gives_the_same_bytes() {
        let event = sample();
        assert_eq!(
            event_json(&event, CaptureMode::Safe),
            event_json(&event, CaptureMode::Safe)
        );
        assert_eq!(
            event_line(&event, CaptureMode::Safe),
            event_line(&event, CaptureMode::Safe)
        );
    }

    /// The one that would make a Safe report unpublishable. A literal address must not survive
    /// into a mode a user did not deliberately choose, in either output.
    #[test]
    fn a_safe_report_carries_no_literal_address() {
        let event = sample();
        let json = event_json(&event, CaptureMode::Safe);
        let line = event_line(&event, CaptureMode::Safe);
        for rendered in [&json, &line] {
            assert!(!rendered.contains("2001:db8"), "{rendered}");
            assert!(!rendered.contains("31484"), "{rendered}");
            assert!(
                rendered.contains("ip6/quic-v1"),
                "the shape still has to survive: {rendered}"
            );
        }
        // And it does appear once the user has asked for it.
        assert!(event_json(&event, CaptureMode::Full).contains("2001:db8"));
    }

    #[test]
    fn a_report_never_carries_a_raw_identifier() {
        let json = event_json(&sample(), CaptureMode::Full);
        assert!(
            !json.contains("12D3KooWabc"),
            "even Full mode reduces identifiers: {json}"
        );
        assert!(json.contains("\"peer\":\"peer-"));
    }

    #[test]
    fn json_carries_the_schema_and_the_capture_mode_on_every_line() {
        // A bundle outlives the build that produced it, and the two modes look similar while
        // meaning very different things.
        let json = event_json(&sample(), CaptureMode::Safe);
        assert!(json.contains("\"schema\":1"));
        assert!(json.contains("\"capture\":\"safe\""));
    }

    #[test]
    fn numbers_stay_numbers_so_a_reader_can_sort_on_them() {
        let json = event_json(&sample(), CaptureMode::Safe);
        assert!(
            json.contains("{\"name\":\"direct_candidates\",\"value\":4}"),
            "{json}"
        );
        assert!(json.contains("\"duration_ms\":60123"), "{json}");
        assert!(!json.contains("\"value\":\"4\""));
    }

    #[test]
    fn absent_values_are_omitted_rather_than_rendered_as_null() {
        let bare = DiagnosticEvent::info(Section::Sync, "SYNC.OK");
        let json = event_json(&bare, CaptureMode::Safe);
        assert!(!json.contains("null"), "{json}");
        assert!(!json.contains("trace"), "{json}");
        assert!(!json.contains("refs"), "{json}");
        assert!(!json.contains("fields"), "{json}");
    }

    #[test]
    fn a_text_line_leads_with_what_a_reader_scans_for() {
        let line = event_line(&sample(), CaptureMode::Safe);
        assert!(line.starts_with("0004812  WARN  join"), "{line}");
        assert!(line.contains("trace=7f2c"), "{line}");
        assert!(line.contains("JOIN.ROUTES.EXHAUSTED"), "{line}");
        assert!(line.contains("duration=60123ms"), "{line}");
        assert!(line.contains("attempt=4"), "{line}");
    }

    /// The finding this rendering exists to answer (P3-005). The console was fed a projection that
    /// dropped the section, the phase, the span parentage and the references, kept only four
    /// characters of the trace, and rendered every value at a hard-coded Enhanced. Most of what the
    /// instrumentation recorded never reached the tool anyone looks at.
    ///
    /// Asserted field by field rather than against a golden blob, so a future change that drops one
    /// of them names the one it dropped.
    #[test]
    fn every_canonical_field_survives_into_the_view() {
        let view = event_view(&sample(), CaptureMode::Safe);

        assert_eq!(view.seq, 4812);
        assert_eq!(view.at_ms, 1_787_000_000_000);
        assert_eq!(view.monotonic_ms, 12_443);
        assert_eq!(
            view.section, "join",
            "the canonical section, one of twenty-two"
        );
        assert_eq!(
            view.view, "network",
            "and the console section it falls under"
        );
        assert_eq!(view.level, "WARN");
        assert_eq!(view.code, "JOIN.ROUTES.EXHAUSTED");
        assert_eq!(view.phase, "failure");
        assert_eq!(view.operation, "join_server");
        assert_eq!(
            view.trace, "7f2c000000000001",
            "the whole trace, not the four characters a line has room for"
        );
        assert_eq!(view.span, "00000000000091ab");
        assert_eq!(
            view.parent_span, "0000000000006dc4",
            "the parentage that says which stage of the operation this was"
        );
        assert_eq!(view.duration_ms, Some(60_123));
        assert_eq!(view.attempt, Some(4));
        assert_eq!(view.target, "catcoms_sync");
        assert_eq!(view.capture, "safe");

        // References survive as structure, named, rather than being collapsed into text.
        let named: Vec<&str> = view.refs.iter().map(|(name, _)| *name).collect();
        assert_eq!(named, ["server", "peer"]);
        assert!(view.refs.iter().all(|(_, value)| !value.is_empty()));

        let fields: Vec<&str> = view.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            fields,
            ["direct_candidates", "relay_candidates", "address", "reason"],
            "in the order they were added, which is what makes a report diffable"
        );
        assert_eq!(view.fields[0].value, "4");
    }

    /// An unset trace or span must read as absent rather than as a run of zeroes, which a reader
    /// would otherwise try to correlate against.
    #[test]
    fn an_event_that_belongs_to_no_operation_says_so() {
        let view = event_view(
            &DiagnosticEvent::info(Section::Sync, "SYNC.OK"),
            CaptureMode::Safe,
        );
        assert_eq!(view.trace, "");
        assert_eq!(view.span, "");
        assert_eq!(view.parent_span, "");
        assert!(view.refs.is_empty());
        assert_eq!(view.phase, "observation");
        assert_eq!(view.operation, "");
    }

    /// The half of P3-005 that was a privacy bug rather than a fidelity one: the projection
    /// hard-coded Enhanced, so the console showed literal addresses whatever mode was chosen.
    #[test]
    fn the_same_event_renders_differently_at_each_mode() {
        let event = sample();
        let safe = event_view(&event, CaptureMode::Safe);
        let enhanced = event_view(&event, CaptureMode::Enhanced);

        let address_of = |v: &EventView| {
            v.fields
                .iter()
                .find(|f| f.name == "address")
                .expect("the sample carries an address")
                .clone()
        };
        assert_eq!(address_of(&safe).value, "ip6/quic-v1");
        assert_eq!(
            address_of(&enhanced).value,
            "/ip6/2001:db8::1/udp/31484/quic-v1"
        );
        assert_eq!(safe.capture, "safe");
        assert_eq!(enhanced.capture, "enhanced");

        // And the console can say what it is not showing, rather than leaving a reader to find out
        // by switching modes and comparing.
        assert!(address_of(&safe).sensitive);
        assert!(
            !safe
                .fields
                .iter()
                .find(|f| f.name == "direct_candidates")
                .unwrap()
                .sensitive,
            "a count says the same thing in every mode"
        );
    }

    /// One event, three renderings, one set of facts. They live in one module so they cannot
    /// disagree about what an event contains, and this is what would catch it if they did.
    #[test]
    fn the_three_renderings_agree_about_what_the_event_says() {
        let event = sample();
        let view = event_view(&event, CaptureMode::Safe);
        let line = event_line(&event, CaptureMode::Safe);
        let json = event_json(&event, CaptureMode::Safe);

        assert!(line.contains(view.code) && json.contains(view.code));
        assert!(line.contains(&view.trace[..4]) && json.contains(&view.trace));
        for (name, value) in &view.refs {
            assert!(line.contains(value), "{name} missing from the line: {line}");
            assert!(json.contains(value), "{name} missing from the json: {json}");
        }
        for field in &view.fields {
            assert!(
                line.contains(&field.value),
                "{} missing from the line: {line}",
                field.name
            );
        }
    }

    #[test]
    fn odd_characters_are_escaped_rather_than_breaking_the_document() {
        let event = DiagnosticEvent::info(Section::Ui, "UI.TEST")
            .field("what", SafeText::describe("a \"quoted\" \\ backslash"));
        let json = event_json(&event, CaptureMode::Safe);
        assert!(json.contains(r#"\"quoted\""#), "{json}");
        assert!(json.contains(r"\\ backslash"), "{json}");
    }

    // --- P3-013: the canonical format cannot be made to say two things ----------------------

    /// A repeated field name is not hypothetical. The `tracing` bridge carries whatever names a
    /// call site declared and the webview sends what it likes, and nothing between there and here
    /// rejects a repeat. Rendered as a JSON object that was two identical keys, which the format
    /// does not define: readers variously keep the first, keep the last, or refuse, so two people
    /// reading one report would disagree about what it said and neither would be told.
    #[test]
    fn a_repeated_field_name_stays_two_fields_rather_than_one_ambiguous_key() {
        let event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST")
            .field("reason", SafeText::describe("first"))
            .field("reason".to_string(), SafeText::describe("second"));

        let json = read_json(&event_json(&event, CaptureMode::Safe))
            .expect("a report a reader can read only one way");
        let fields = json.get("fields").expect("the fields").items();

        assert_eq!(fields.len(), 2, "{fields:?}");
        assert_eq!(fields[0].get("name").unwrap().text(), "reason");
        assert_eq!(fields[0].get("value").unwrap().text(), "first");
        assert_eq!(
            fields[1].get("value").unwrap().text(),
            "second",
            "in the order recorded, which is the only thing telling the two apart"
        );
    }

    /// The event that reaches the field cap is the one whose extra fields somebody wanted. Losing
    /// them is defensible; losing them invisibly is not, because the report then reads as a
    /// complete account of an event that was not completely recorded.
    #[test]
    fn an_event_that_lost_fields_to_the_cap_says_so_in_every_rendering() {
        let mut event = DiagnosticEvent::info(Section::Sync, "SYNC.TEST");
        for n in 0..(MAX_FIELDS + 5) {
            event = event.field("filler", n as u64);
        }

        let json = read_json(&event_json(&event, CaptureMode::Safe)).expect("parses");
        assert_eq!(json.get("fields").unwrap().items().len(), MAX_FIELDS);
        assert_eq!(
            json.get("fields_dropped")
                .expect("the export admits the loss")
                .number(),
            5
        );
        assert!(
            event_line(&event, CaptureMode::Safe).contains(" fields_dropped=5"),
            "the row admits it too, since that is what a screenshot shows"
        );
        assert_eq!(event_view(&event, CaptureMode::Safe).fields_dropped, 5);

        // And an event that lost nothing carries no marker, or the marker stops meaning anything.
        let whole = DiagnosticEvent::info(Section::Sync, "SYNC.OK").field("ops", 1u64);
        assert!(!event_json(&whole, CaptureMode::Safe).contains("fields_dropped"));
        assert!(!event_line(&whole, CaptureMode::Safe).contains("fields_dropped"));
    }

    /// The forgery in the shape the review demonstrated. A bridged message keeps its newlines on
    /// purpose, so a message whose second line is shaped like an event header puts an event into
    /// `report.txt` that the app never recorded, and a screenshot of it cannot be argued with.
    #[test]
    fn a_value_cannot_add_a_row_to_the_report() {
        let forged = "real error\n0001234  ERROR  security      MLS.KEY.LEAKED";
        let event = DiagnosticEvent::error(Section::Sync, "SYNC.FAILED")
            .field("message", BridgedMessage::new(forged));
        let line = event_line(&event, CaptureMode::Safe);
        assert_eq!(line.lines().count(), 1, "one event, one row: {line}");
        assert!(line.contains(r"real error\n0001234"), "{line}");

        // The same hole by the other route: a literal address is rendered verbatim under a mode
        // that allows it, and nothing on the way in strips its control characters.
        let addressed = DiagnosticEvent::info(Section::Transport, "NET.DIAL.FAILED").field(
            "address",
            AddressValue::new(
                "/ip4/203.0.113.9/tcp/1\n0001235  ERROR  security      MLS.KEY.LEAKED",
            ),
        );
        let line = event_line(&addressed, CaptureMode::Full);
        assert_eq!(line.lines().count(), 1, "{line}");

        // A carriage return is the same forgery against a reader watching a terminal rather than
        // parsing a file: it returns the cursor and the rest of the value overwrites the row.
        // A bridged message loses one on the way in, but an address keeps everything it was given.
        let returned = DiagnosticEvent::info(Section::Transport, "NET.DIAL.FAILED")
            .field("address", AddressValue::new("/ip4/203.0.113.9\rtcp"));
        assert!(
            event_line(&returned, CaptureMode::Full).contains(r"203.0.113.9\rtcp"),
            "a control character reads as itself rather than acting"
        );
    }

    /// Events chosen for what they can do to a rendering rather than for what they report.
    ///
    /// Each is paired with the number of fields it was offered, so the accounting below can be
    /// checked against what the call site asked for rather than against what the event kept.
    fn hostile_events() -> Vec<(DiagnosticEvent, usize)> {
        let salt = SessionSalt::for_tests(5);
        let mut events = Vec::new();

        let mut plain = sample();
        plain.seq = 1;
        events.push((plain, 4));

        let mut forged = DiagnosticEvent::error(Section::Files, "FILES.SEND.FAILED")
            .field(
                "message",
                BridgedMessage::new("upload failed\n0000009  ERROR  security      MLS.KEY.LEAKED"),
            )
            .field(
                "address",
                AddressValue::new("/ip4/203.0.113.9/tcp/1\r0000010  WARN   join         JOIN.OK"),
            )
            .field(
                "transfer".to_string(),
                salt.reference(RefDomain::Transfer, b"t-1"),
            );
        forged.seq = 2;
        events.push((forged, 3));

        let mut repeated = DiagnosticEvent::warn(Section::Sync, "SYNC.CATCHUP.STALLED")
            .field("reason", SafeText::describe("first"))
            .field("reason".to_string(), SafeText::describe("second"))
            .field("reason".to_string(), SafeText::describe("third"))
            .field("stalled", true);
        repeated.seq = 3;
        events.push((repeated, 4));

        let mut named = DiagnosticEvent::info(Section::Ui, "UI.EVENT")
            .field("peer=peer-000000000000 x".to_string(), 1u64)
            .field("brace\u{7}d".to_string(), -2i64);
        named.seq = 4;
        events.push((named, 2));

        let offered = MAX_FIELDS + 9;
        let mut overflowed = DiagnosticEvent::info(Section::Startup, "STARTUP.READY");
        for n in 0..offered {
            overflowed = overflowed.field("filler", n as u64);
        }
        overflowed.seq = 5;
        events.push((overflowed, offered));

        events
    }

    /// The test the review asked for: read a rendered report back, and prove every row of it is
    /// exactly one event.
    ///
    /// Written as a parser rather than as `contains` assertions because that is what the finding is
    /// about. A rendering can satisfy any number of substring checks and still be a document that
    /// two readers resolve differently, and the only way to show it is not is to be a reader.
    #[test]
    fn every_row_of_a_report_maps_to_exactly_one_event() {
        // Both modes, because the literal address only exists in one of them and it is one of the
        // two ways a value reaches a row unfiltered.
        for mode in [CaptureMode::Safe, CaptureMode::Full] {
            let events = hostile_events();

            let text: String = events
                .iter()
                .map(|(event, _)| event_line(event, mode))
                .collect::<Vec<_>>()
                .join("\n");
            let rows: Vec<&str> = text.lines().collect();
            assert_eq!(
                rows.len(),
                events.len(),
                "{} events produced {} rows in {mode:?} mode:\n{text}",
                events.len(),
                rows.len()
            );
            for (row, (event, _)) in rows.iter().zip(&events) {
                let seq = row_seq(row).unwrap_or_else(|why| panic!("{why}"));
                assert_eq!(seq, event.seq, "row does not belong to its event: {row}");
            }

            let jsonl: String = events
                .iter()
                .map(|(event, _)| event_json(event, mode))
                .collect::<Vec<_>>()
                .join("\n");
            let lines: Vec<&str> = jsonl.lines().collect();
            assert_eq!(lines.len(), events.len(), "{jsonl}");
            for (line, (event, offered)) in lines.iter().zip(&events) {
                let parsed = read_json(line)
                    .unwrap_or_else(|why| panic!("a reader cannot read this: {why}\n{line}"));
                assert_eq!(
                    parsed.get("seq").expect("a sequence number").number(),
                    event.seq
                );

                // Everything the call site offered is either in the report or counted as missing
                // from it. Nothing goes without a trace.
                let kept = parsed
                    .get("fields")
                    .map_or(0, |fields| fields.items().len());
                let dropped = parsed.get("fields_dropped").map_or(0, |lost| lost.number());
                assert_eq!(
                    kept as u64 + dropped,
                    *offered as u64,
                    "event {} accounted for {kept} kept and {dropped} dropped of {offered}",
                    event.seq
                );
            }
        }
    }

    /// The only thing a reader must be able to do to a row: say which event it is.
    fn row_seq(row: &str) -> Result<u64, String> {
        let (seq, rest) = row
            .split_once("  ")
            .ok_or_else(|| format!("a row no event could have produced: {row:?}"))?;
        if seq.is_empty() || !seq.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("a row that does not open with a sequence: {row:?}"));
        }
        if rest.is_empty() {
            return Err(format!("a row with nothing after its sequence: {row:?}"));
        }
        seq.parse()
            .map_err(|_| format!("unreadable sequence {seq:?}"))
    }

    /// What a consumer of `events.jsonl` can get back out of it.
    ///
    /// Deliberately stricter than a real reader would be, on exactly the two counts this finding is
    /// about: a duplicate key and a raw control character inside a string are errors here rather
    /// than something to resolve, because both are places where two readers would part company
    /// about what the report said and neither of them would find out.
    #[derive(Debug, Clone, PartialEq)]
    enum Json {
        Str(String),
        Num(String),
        Bool(bool),
        Arr(Vec<Json>),
        Obj(Vec<(String, Json)>),
    }

    impl Json {
        fn get(&self, key: &str) -> Option<&Json> {
            match self {
                Json::Obj(pairs) => pairs.iter().find(|(name, _)| name == key).map(|(_, v)| v),
                _ => None,
            }
        }

        fn text(&self) -> &str {
            match self {
                Json::Str(value) => value,
                other => panic!("not a string: {other:?}"),
            }
        }

        fn number(&self) -> u64 {
            match self {
                Json::Num(value) => value.parse().expect("a whole number"),
                other => panic!("not a number: {other:?}"),
            }
        }

        fn items(&self) -> &[Json] {
            match self {
                Json::Arr(values) => values,
                other => panic!("not an array: {other:?}"),
            }
        }
    }

    fn read_json(text: &str) -> Result<Json, String> {
        let mut reader = Reader {
            chars: text.chars().collect(),
            at: 0,
        };
        let value = reader.value()?;
        reader.space();
        if reader.at != reader.chars.len() {
            return Err(format!(
                "text after the end of the document at {}",
                reader.at
            ));
        }
        Ok(value)
    }

    struct Reader {
        chars: Vec<char>,
        at: usize,
    }

    impl Reader {
        fn peek(&self) -> Option<char> {
            self.chars.get(self.at).copied()
        }

        fn space(&mut self) {
            while self.peek().is_some_and(char::is_whitespace) {
                self.at += 1;
            }
        }

        fn expect(&mut self, wanted: char) -> Result<(), String> {
            if self.peek() == Some(wanted) {
                self.at += 1;
                return Ok(());
            }
            Err(format!("expected {wanted:?} at {}", self.at))
        }

        fn value(&mut self) -> Result<Json, String> {
            self.space();
            match self.peek() {
                Some('{') => self.object(),
                Some('[') => self.array(),
                Some('"') => Ok(Json::Str(self.string()?)),
                Some('t') => self.word("true").map(|()| Json::Bool(true)),
                Some('f') => self.word("false").map(|()| Json::Bool(false)),
                Some(c) if c == '-' || c.is_ascii_digit() => Ok(Json::Num(self.number())),
                other => Err(format!("unexpected {other:?} at {}", self.at)),
            }
        }

        fn object(&mut self) -> Result<Json, String> {
            self.expect('{')?;
            let mut pairs: Vec<(String, Json)> = Vec::new();
            self.space();
            if self.peek() == Some('}') {
                self.at += 1;
                return Ok(Json::Obj(pairs));
            }
            loop {
                self.space();
                let key = self.string()?;
                if pairs.iter().any(|(seen, _)| *seen == key) {
                    return Err(format!(
                        "the key {key:?} appears twice, so no reader can say which value the \
                         event carried"
                    ));
                }
                self.space();
                self.expect(':')?;
                let value = self.value()?;
                pairs.push((key, value));
                self.space();
                match self.peek() {
                    Some(',') => self.at += 1,
                    Some('}') => {
                        self.at += 1;
                        return Ok(Json::Obj(pairs));
                    }
                    other => return Err(format!("expected ',' or '}}', found {other:?}")),
                }
            }
        }

        fn array(&mut self) -> Result<Json, String> {
            self.expect('[')?;
            let mut values = Vec::new();
            self.space();
            if self.peek() == Some(']') {
                self.at += 1;
                return Ok(Json::Arr(values));
            }
            loop {
                values.push(self.value()?);
                self.space();
                match self.peek() {
                    Some(',') => self.at += 1,
                    Some(']') => {
                        self.at += 1;
                        return Ok(Json::Arr(values));
                    }
                    other => return Err(format!("expected ',' or ']', found {other:?}")),
                }
            }
        }

        fn string(&mut self) -> Result<String, String> {
            self.expect('"')?;
            let mut out = String::new();
            loop {
                let c = self.peek().ok_or("a string that never ends")?;
                self.at += 1;
                match c {
                    '"' => return Ok(out),
                    '\\' => {
                        let escape = self.peek().ok_or("an escape that never ends")?;
                        self.at += 1;
                        out.push(match escape {
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            'b' => '\u{8}',
                            'f' => '\u{c}',
                            '"' => '"',
                            '\\' => '\\',
                            '/' => '/',
                            'u' => {
                                let digits: String = self
                                    .chars
                                    .get(self.at..self.at + 4)
                                    .ok_or("a \\u escape that runs off the end")?
                                    .iter()
                                    .collect();
                                self.at += 4;
                                let point = u32::from_str_radix(&digits, 16)
                                    .map_err(|_| format!("{digits:?} is not hexadecimal"))?;
                                char::from_u32(point)
                                    .ok_or_else(|| format!("U+{point:04X} is not a character"))?
                            }
                            other => return Err(format!("an escape nobody defines: {other:?}")),
                        });
                    }
                    // A raw newline here would end the JSON line the reader is on, which is the
                    // same forgery the text rendering is guarded against.
                    c if (c as u32) < 0x20 => {
                        return Err(format!("a raw U+{:04X} inside a string", c as u32))
                    }
                    c => out.push(c),
                }
            }
        }

        fn number(&mut self) -> String {
            let start = self.at;
            while self
                .peek()
                .is_some_and(|c| c.is_ascii_digit() || "-+.eE".contains(c))
            {
                self.at += 1;
            }
            self.chars[start..self.at].iter().collect()
        }

        fn word(&mut self, wanted: &str) -> Result<(), String> {
            for c in wanted.chars() {
                self.expect(c)?;
            }
            Ok(())
        }
    }
}
