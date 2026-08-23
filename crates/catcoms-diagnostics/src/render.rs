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
        for (name, value) in [
            ("server", &event.refs.server),
            ("channel", &event.refs.channel),
            ("peer", &event.refs.peer),
            ("document", &event.refs.document),
            ("transfer", &event.refs.transfer),
        ] {
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

    if !event.fields.is_empty() {
        out.push_str(",\"fields\":{");
        for (at, (name, value)) in event.fields.iter().enumerate() {
            if at > 0 {
                out.push(',');
            }
            json_string(name.as_str(), &mut out);
            out.push(':');
            json_value(value, mode, &mut out);
        }
        out.push('}');
    }

    // Says what the reader is holding. A Safe report and a Full one look similar and mean very
    // different things, so the mode travels with every line rather than only in a manifest that
    // can be separated from it.
    out.push_str(",\"capture\":");
    json_string(mode.as_str(), &mut out);
    out.push('}');
    out
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
    for (name, reference) in [
        ("server", &event.refs.server),
        ("channel", &event.refs.channel),
        ("peer", &event.refs.peer),
        ("document", &event.refs.document),
        ("transfer", &event.refs.transfer),
    ] {
        if let Some(value) = reference {
            out.push_str(&format!(" {name}={value}"));
        }
    }
    for (name, value) in &event.fields {
        out.push_str(&format!(" {name}={}", value.render(mode)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Section;
    use crate::event::{Phase, Refs, SpanId, TraceId};
    use crate::redact::{AddressValue, RefDomain, SafeText, SessionSalt};

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
        assert!(json.contains("\"direct_candidates\":4"), "{json}");
        assert!(json.contains("\"duration_ms\":60123"), "{json}");
        assert!(!json.contains("\"direct_candidates\":\"4\""));
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

    #[test]
    fn odd_characters_are_escaped_rather_than_breaking_the_document() {
        let event = DiagnosticEvent::info(Section::Ui, "UI.TEST")
            .field("what", SafeText::describe("a \"quoted\" \\ backslash"));
        let json = event_json(&event, CaptureMode::Safe);
        assert!(json.contains(r#"\"quoted\""#), "{json}");
        assert!(json.contains(r"\\ backslash"), "{json}");
    }
}
