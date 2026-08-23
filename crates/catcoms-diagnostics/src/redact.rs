//! What a diagnostic event is allowed to carry, and how identifiers are reduced before it does.
//!
//! # The problem this replaces
//!
//! The previous approach masked identifying values by pattern-matching the finished text: take the
//! line about to be rendered, look for things shaped like an IP address or a peer id, swap in
//! `[ip 1]`. That holds exactly as long as everything sensitive looks the way the pattern expects,
//! and it did not: a peer id shorter than the rule's minimum tail rendered in the clear while the
//! screen claimed to be safe to share. The failure mode is silent and the blast radius is a
//! screenshot or a public bug report.
//!
//! The replacement inverts it. Rather than searching for secrets in the output, the type that
//! carries a value into an event has no variant capable of holding one.
//!
//! # What is and is not guaranteed
//!
//! Honest about the boundary, because a privacy claim that overreaches is worse than none:
//!
//! * **Identifiers cannot leak.** A peer id, group id, channel id, device fingerprint or content
//!   address enters only as a [`SessionRef`], which is a keyed hash. There is no code path that
//!   puts a raw one into an event, so no amount of carelessness at a call site produces one.
//! * **Keys and credentials cannot be encoded.** [`SafeValue`] has no bytes variant and no
//!   `From<String>`, so key material has no representation to travel in.
//! * **Content is discouraged structurally, not prevented absolutely.** Rust has no taint
//!   tracking, so somebody determined to write `SafeText::describe(&message.body)` can. What this
//!   does is remove every accidental route: there is no `impl From<String>`, the constructor is
//!   named for what it is for, and the value is bounded to a length no message body survives.
//!   The export validator is the second line, and the CI rules the review calls for are the third.
//!
//! # Why references are per-session
//!
//! A stable hash of a peer id would be a stable cross-session identifier: exactly the tracking
//! token this app exists to avoid, handed out in every bug report. The key is random per
//! diagnostic session, so `peer-5d09` correlates perfectly *inside* one report and means nothing
//! when compared against another. Correlation inside the report is the evidence ("it keeps dialling
//! the same two addresses"); correlation across reports is surveillance.

use catcoms_rt::RngCore;

use crate::config::CaptureMode;

/// How long a rendered reference is. Twelve hex characters is 48 bits: far beyond collision range
/// for the few thousand distinct ids one session sees, and short enough to read off a screenshot.
const REF_CHARS: usize = 12;

/// The longest a piece of [`SafeText`] may be.
///
/// Deliberately short. This is for describing an outcome ("no advertised route completed"), not
/// for carrying a payload, and a cap that no message body, wiki page or file name would survive
/// intact makes misuse obvious in the output rather than invisible.
pub const MAX_SAFE_TEXT: usize = 200;

/// What kind of thing a reference stands for.
///
/// Part of the hash input, so the same underlying bytes used as two different kinds produce two
/// different references. Without that, a group id that happened to equal a channel id would
/// silently correlate across sections that have nothing to do with each other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefDomain {
    Server,
    Channel,
    Peer,
    Device,
    Document,
    File,
    Transfer,
    Invite,
}

impl RefDomain {
    /// The prefix a rendered reference carries, so a reader can tell what kind of thing it is.
    pub fn prefix(self) -> &'static str {
        match self {
            RefDomain::Server => "srv",
            RefDomain::Channel => "chan",
            RefDomain::Peer => "peer",
            RefDomain::Device => "dev",
            RefDomain::Document => "doc",
            RefDomain::File => "file",
            RefDomain::Transfer => "xfer",
            RefDomain::Invite => "inv",
        }
    }
}

/// The per-session key that references are derived under.
///
/// Random per diagnostic session and never persisted. Two reports from the same machine on the
/// same day share no reference values, which is what stops a bug report from being a tracking
/// token.
#[derive(Clone)]
pub struct SessionSalt([u8; 32]);

impl std::fmt::Debug for SessionSalt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never rendered. A salt in a log would let anyone recompute every reference in it, which
        // undoes the entire mechanism.
        f.write_str("SessionSalt(..)")
    }
}

impl SessionSalt {
    /// A fresh random salt, from an injected RNG.
    pub fn random(rng: &mut impl RngCore) -> Self {
        let mut bytes = [0u8; 32];
        rng.fill_bytes(&mut bytes);
        SessionSalt(bytes)
    }

    /// A salt with a chosen value, for tests that need references to be reproducible.
    ///
    /// Test-only in spirit but not gated on `cfg(test)`: the export golden tests live in another
    /// crate, and a determinism property that cannot be exercised from outside is not much of a
    /// property.
    pub fn for_tests(seed: u8) -> Self {
        SessionSalt([seed; 32])
    }

    /// Derive the reference for one identifier.
    pub fn reference(&self, domain: RefDomain, id: &[u8]) -> SessionRef {
        let mut input = Vec::with_capacity(id.len() + 8);
        input.extend_from_slice(domain.prefix().as_bytes());
        // A separator, so ("srv", "abc") and ("srvabc", "") cannot collide.
        input.push(0);
        input.extend_from_slice(id);
        let digest = blake3::keyed_hash(&self.0, &input);
        let hex: String = digest
            .as_bytes()
            .iter()
            .take(REF_CHARS.div_ceil(2))
            .map(|b| format!("{b:02x}"))
            .collect();
        SessionRef(format!("{}-{}", domain.prefix(), &hex[..REF_CHARS]))
    }
}

/// One identifier, reduced to something safe to write down.
///
/// The only way an id reaches an event. Constructing one requires a [`SessionSalt`], and there is
/// no constructor from a plain string, so a call site cannot produce a reference that is secretly
/// the raw value.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionRef(String);

impl SessionRef {
    /// The rendered form, e.g. `peer-5d09a1b2c3d4`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Bounded free text describing an outcome.
///
/// For explaining *what happened*, never for carrying what it happened to. See the module docs on
/// what this does and does not guarantee.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SafeText(String);

impl SafeText {
    /// Describe an outcome in at most [`MAX_SAFE_TEXT`] characters.
    ///
    /// Named `describe` rather than `new` on purpose: a call site reading
    /// `SafeText::describe(&message.body)` is visibly wrong in a way `SafeText::new(..)` would not
    /// be. Control characters are stripped, because a diagnostic line that can move the cursor is
    /// a diagnostic line that can lie about what came before it.
    pub fn describe(text: &str) -> Self {
        let cleaned: String = text
            .chars()
            .filter(|c| !c.is_control())
            .take(MAX_SAFE_TEXT)
            .collect();
        SafeText(cleaned)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SafeText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The longest a bridged `tracing` message may be.
///
/// Much larger than [`MAX_SAFE_TEXT`] because these are existing log lines written before any of
/// this existed, and truncating them to a sentence would degrade a working debug log to make a
/// migration look tidier.
pub const MAX_BRIDGED_MESSAGE: usize = 4000;

/// A message from a `tracing` event that has not been converted to a structured code yet.
///
/// **This is the known hole in the guarantee at the top of this module, and it is deliberate.**
///
/// The app emits thousands of `tracing` events written long before this crate existed, whose
/// messages are ordinary format strings. Refusing to carry them would mean either a flag-day
/// rewrite of every call site or a debug log that sees less than the old one did, and both are
/// worse than an explicit, greppable exception.
///
/// It is a distinct type from [`SafeText`] precisely so it stays visible: every place a bridged
/// message can reach is a place the export validator treats as suspect, and the count of them is a
/// measure of how far the migration to structured codes has actually got. New code has no reason
/// to construct one.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct BridgedMessage(String);

impl BridgedMessage {
    /// Carry an existing log message through, bounded and stripped of control characters.
    pub fn new(text: &str) -> Self {
        BridgedMessage(Self::clean(text))
    }

    /// The same, taking ownership.
    ///
    /// The `tracing` bridge has already allocated a `String` per field by the time it gets here,
    /// and copying it again doubles the per-field allocation on the emitting thread. An ordinary
    /// message is short and control-free, so the scan usually finds nothing and the original
    /// buffer is kept as it is.
    pub fn from_owned(text: String) -> Self {
        let clean =
            text.len() <= MAX_BRIDGED_MESSAGE && !text.chars().any(|c| c != '\n' && c.is_control());
        if clean {
            return BridgedMessage(text);
        }
        BridgedMessage(Self::clean(&text))
    }

    fn clean(text: &str) -> String {
        text.chars()
            .filter(|c| *c == '\n' || !c.is_control())
            .take(MAX_BRIDGED_MESSAGE)
            .collect()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BridgedMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A network address, which is the one genuinely useful thing that is also genuinely identifying.
///
/// In Safe mode an event carries only what the address *is*: its family and transport, which is
/// enough to diagnose the failure that stranded a node for an hour (every candidate was IPv6 on a
/// host with no IPv6 route). The literal value appears only when the user has deliberately turned
/// on a mode that says so, with a warning and an expiry.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct AddressValue {
    family: AddressFamily,
    transport: SafeText,
    raw: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AddressFamily {
    V4,
    V6,
    Dns,
    Other,
}

impl AddressFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            AddressFamily::V4 => "ip4",
            AddressFamily::V6 => "ip6",
            AddressFamily::Dns => "dns",
            AddressFamily::Other => "other",
        }
    }
}

impl AddressValue {
    /// Classify a multiaddr-shaped string into its family, transport and literal value.
    pub fn new(addr: &str) -> Self {
        let family = if addr.starts_with("/ip4/") {
            AddressFamily::V4
        } else if addr.starts_with("/ip6/") {
            AddressFamily::V6
        } else if addr.starts_with("/dns") {
            AddressFamily::Dns
        } else {
            AddressFamily::Other
        };
        // The last protocol segment is the transport: quic-v1, tcp, ws, and so on. Which one was
        // tried matters and is not identifying on its own.
        let transport = addr
            .rsplit('/')
            .find(|part| !part.is_empty() && part.parse::<u16>().is_err())
            .unwrap_or("unknown");
        AddressValue {
            family,
            transport: SafeText::describe(transport),
            raw: addr.to_string(),
        }
    }

    pub fn family(&self) -> AddressFamily {
        self.family
    }

    /// How this address reads under a given capture mode.
    ///
    /// Safe mode never yields the literal. That is not a formatting choice: it is the difference
    /// between a report a user can paste into a public issue and one they cannot.
    pub fn render(&self, mode: CaptureMode) -> String {
        if mode.allows_raw_addresses() {
            self.raw.clone()
        } else {
            format!("{}/{}", self.family.as_str(), self.transport)
        }
    }
}

/// Everything a diagnostic event field is allowed to be.
///
/// The list is closed, and what is missing from it is the point. There is no `Bytes`, no
/// `String`, and no `From<String>`: key material and message bodies have no representation here,
/// so they cannot travel by accident.
#[derive(Clone, Debug, PartialEq, PartialOrd)]
pub enum SafeValue {
    Bool(bool),
    /// A quantity: how many peers, how many chunks, how deep a queue.
    Count(u64),
    /// A signed quantity, for deltas and differences.
    Delta(i64),
    Duration(u64),
    /// A value from a closed set the code owns. `&'static str` is doing real work: a compile-time
    /// literal cannot be runtime data, so this variant is incapable of carrying user content.
    Outcome(&'static str),
    /// Bounded prose describing an outcome. See [`SafeText`].
    Text(SafeText),
    /// An identifier, already reduced. See [`SessionRef`].
    Ref(SessionRef),
    /// A network address, rendered according to the capture mode. See [`AddressValue`].
    Address(AddressValue),
    /// An unconverted `tracing` message. The migration's compatibility path, and the one variant
    /// that can carry arbitrary text. See [`BridgedMessage`].
    Bridged(BridgedMessage),
}

impl SafeValue {
    /// How this value renders under a capture mode.
    pub fn render(&self, mode: CaptureMode) -> String {
        match self {
            SafeValue::Bool(v) => v.to_string(),
            SafeValue::Count(v) => v.to_string(),
            SafeValue::Delta(v) => v.to_string(),
            SafeValue::Duration(v) => format!("{v}ms"),
            SafeValue::Outcome(v) => (*v).to_string(),
            SafeValue::Text(v) => v.as_str().to_string(),
            SafeValue::Ref(v) => v.as_str().to_string(),
            SafeValue::Address(v) => v.render(mode),
            SafeValue::Bridged(v) => v.as_str().to_string(),
        }
    }

    /// Whether this value would read differently with raw addresses turned on.
    ///
    /// Used by the export preview to say what a mode change would actually reveal, rather than
    /// making the user find out by doing it.
    pub fn is_mode_sensitive(&self) -> bool {
        matches!(self, SafeValue::Address(_))
    }

    /// Whether this value came through the un-migrated `tracing` path.
    ///
    /// The export validator scrutinises these and only these, and counting them measures how much
    /// of the app still has to be converted to structured codes.
    pub fn is_bridged(&self) -> bool {
        matches!(self, SafeValue::Bridged(_))
    }
}

impl From<bool> for SafeValue {
    fn from(v: bool) -> Self {
        SafeValue::Bool(v)
    }
}
impl From<u64> for SafeValue {
    fn from(v: u64) -> Self {
        SafeValue::Count(v)
    }
}
impl From<usize> for SafeValue {
    fn from(v: usize) -> Self {
        SafeValue::Count(v as u64)
    }
}
impl From<i64> for SafeValue {
    fn from(v: i64) -> Self {
        SafeValue::Delta(v)
    }
}
impl From<SessionRef> for SafeValue {
    fn from(v: SessionRef) -> Self {
        SafeValue::Ref(v)
    }
}
impl From<SafeText> for SafeValue {
    fn from(v: SafeText) -> Self {
        SafeValue::Text(v)
    }
}
impl From<AddressValue> for SafeValue {
    fn from(v: AddressValue) -> Self {
        SafeValue::Address(v)
    }
}
impl From<BridgedMessage> for SafeValue {
    fn from(v: BridgedMessage) -> Self {
        SafeValue::Bridged(v)
    }
}
// Deliberately absent: `impl From<String>` and `impl From<&str>`. Either one would turn every
// careless `field.into()` into a hole in the guarantee above, which is exactly how the previous
// design leaked. Runtime text goes through `SafeText::describe`, at a call site that reads like a
// decision.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reference_is_stable_within_a_session() {
        let salt = SessionSalt::for_tests(7);
        let once = salt.reference(RefDomain::Peer, b"12D3KooWabcdef");
        let again = salt.reference(RefDomain::Peer, b"12D3KooWabcdef");
        assert_eq!(once, again, "correlation inside a report is the evidence");
        assert!(once.as_str().starts_with("peer-"));
        assert_eq!(once.as_str().len(), "peer-".len() + REF_CHARS);
    }

    /// The property that keeps a bug report from being a tracking token: the same peer, seen by
    /// the same machine on two different days, is two unrelated strings.
    #[test]
    fn the_same_identifier_is_unrecognisable_across_sessions() {
        let monday = SessionSalt::for_tests(1).reference(RefDomain::Peer, b"12D3KooWabcdef");
        let tuesday = SessionSalt::for_tests(2).reference(RefDomain::Peer, b"12D3KooWabcdef");
        assert_ne!(monday, tuesday);
    }

    #[test]
    fn two_kinds_of_thing_never_collide_on_the_same_bytes() {
        let salt = SessionSalt::for_tests(7);
        let as_server = salt.reference(RefDomain::Server, b"same-bytes");
        let as_channel = salt.reference(RefDomain::Channel, b"same-bytes");
        assert_ne!(as_server, as_channel);
    }

    #[test]
    fn a_reference_never_contains_the_value_it_stands_for() {
        let salt = SessionSalt::for_tests(7);
        let reference = salt.reference(RefDomain::Peer, b"12D3KooWabcdef");
        assert!(!reference.as_str().contains("12D3Koo"));
    }

    /// A salt in a log would let a reader recompute every reference in it.
    #[test]
    fn a_salt_never_renders_itself() {
        let rendered = format!("{:?}", SessionSalt::for_tests(0xAB));
        assert_eq!(rendered, "SessionSalt(..)");
        assert!(!rendered.contains("ab"));
    }

    #[test]
    fn safe_text_is_bounded_and_strips_control_characters() {
        let long = SafeText::describe(&"x".repeat(MAX_SAFE_TEXT * 3));
        assert_eq!(long.as_str().chars().count(), MAX_SAFE_TEXT);
        // A line that can move the terminal cursor can lie about what came before it.
        assert_eq!(SafeText::describe("a\nb\tc\u{7}d").as_str(), "abcd");
    }

    /// The failure that stranded a node for an hour is diagnosable without a single literal
    /// address: what mattered was that every candidate was IPv6.
    #[test]
    fn safe_mode_keeps_the_shape_of_an_address_and_drops_the_value() {
        let addr = AddressValue::new("/ip6/2001:db8::1/udp/31484/quic-v1");
        assert_eq!(addr.family(), AddressFamily::V6);
        let safe = addr.render(CaptureMode::Safe);
        assert_eq!(safe, "ip6/quic-v1");
        assert!(
            !safe.contains("2001"),
            "the literal must not survive Safe mode"
        );
        assert_eq!(
            addr.render(CaptureMode::Full),
            "/ip6/2001:db8::1/udp/31484/quic-v1",
            "and must survive a mode the user deliberately turned on"
        );
    }

    #[test]
    fn an_address_of_any_shape_is_classified_rather_than_rejected() {
        assert_eq!(
            AddressValue::new("/ip4/203.0.113.9/tcp/443").family(),
            AddressFamily::V4
        );
        assert_eq!(
            AddressValue::new("/dns4/relay.example/tcp/443").family(),
            AddressFamily::Dns
        );
        assert_eq!(
            AddressValue::new("something else entirely").family(),
            AddressFamily::Other
        );
    }

    #[test]
    fn only_addresses_change_with_the_capture_mode() {
        assert!(
            SafeValue::Address(AddressValue::new("/ip4/203.0.113.9/tcp/1")).is_mode_sensitive()
        );
        assert!(!SafeValue::Count(3).is_mode_sensitive());
        // A reference is a keyed hash in every mode. There is no mode that reveals the original,
        // because nothing anywhere retains it.
        assert!(
            !SafeValue::Ref(SessionSalt::for_tests(1).reference(RefDomain::Peer, b"p"))
                .is_mode_sensitive()
        );
    }

    /// The compatibility path stays visible on purpose: it is the one variant that can carry
    /// arbitrary text, so both the export validator and the migration need to find it.
    #[test]
    fn a_bridged_message_is_distinguishable_from_everything_else() {
        let bridged = SafeValue::Bridged(BridgedMessage::new("dial failed"));
        assert!(bridged.is_bridged());
        assert!(!SafeValue::Text(SafeText::describe("dial failed")).is_bridged());
        assert!(!SafeValue::Count(1).is_bridged());
    }

    #[test]
    fn a_bridged_message_is_bounded_and_keeps_the_newlines_a_stack_needs() {
        let long = BridgedMessage::new(&"x".repeat(MAX_BRIDGED_MESSAGE * 2));
        assert_eq!(long.as_str().chars().count(), MAX_BRIDGED_MESSAGE);
        // A stack trace is the reason these exist at all, so its line breaks survive while other
        // control characters do not.
        assert_eq!(
            BridgedMessage::new("Error: boom\n  at foo\u{7}").as_str(),
            "Error: boom\n  at foo"
        );
    }

    #[test]
    fn values_render_in_a_form_a_person_can_read() {
        assert_eq!(
            SafeValue::Duration(60_123).render(CaptureMode::Safe),
            "60123ms"
        );
        assert_eq!(SafeValue::Count(4).render(CaptureMode::Safe), "4");
        assert_eq!(SafeValue::Delta(-2).render(CaptureMode::Safe), "-2");
        assert_eq!(
            SafeValue::Outcome("no_viable_route").render(CaptureMode::Safe),
            "no_viable_route"
        );
    }
}
