//! An independent check on a rendered report, run after rendering and before anything leaves the
//! machine.
//!
//! # Why this exists as a second pass
//!
//! Every other privacy control in this crate works by construction: [`SessionRef`] has no
//! constructor from a plain string, [`SafeValue::Outcome`] holds a `&'static str` that a runtime
//! value cannot reach, and [`AddressValue`] consults the capture mode when it renders. Those are
//! the strongest guarantees here and this module does not replace them.
//!
//! But two variants can carry arbitrary text and both do so deliberately.
//! [`SafeText::describe`] takes any `&str` and bounds it, and [`BridgedMessage`] exists precisely
//! to carry `tracing` prose written long before any of this. So the promise a report's footer
//! makes to its reader is not established by the type system alone, and a promise that is not
//! established is a promise that is false. The adversarial review found a legacy warning of the
//! form `failed to render "Private Support": C:\Users\<name>\...` reaching a report whose footer
//! said it contained no names.
//!
//! This pass reads the finished bytes, knowing nothing about how they were produced. That
//! independence is the point: a validator sharing the renderer's assumptions would share its
//! blind spots, and the failures worth catching are the ones nobody thought of at the call site.
//!
//! # What it does not claim
//!
//! It is a net, not a proof. It finds evidence of the categories below; it cannot certify that
//! text is free of everything. A clean report is one where nothing known-dangerous was spotted,
//! which is a weaker and more honest statement than "safe to share", and the wording the user
//! sees should stay weaker too.
//!
//! # Finding something is not the same as refusing
//!
//! Reading a report and deciding what to do about it are separate, and conflating them broke the
//! feature outright: refusing on every category meant the first Save in a real session failed with
//! several hundred findings, because the networking layer's un-migrated `tracing` prose narrates
//! every address and peer id it sees. Those are the ordinary contents of today's log, not
//! escapes.
//!
//! So [`ExportPurpose`] decides. Writing a file into the user's own log folder is not disclosure
//! and nothing refuses it; what it owes the user is [`Report::disclosure`], an honest account of
//! what they are holding. Sending the same bytes somewhere public is the boundary the review was
//! about, and there a finding refuses.
//!
//! [`SessionRef`]: crate::redact::SessionRef
//! [`SafeValue::Outcome`]: crate::redact::SafeValue::Outcome
//! [`AddressValue`]: crate::redact::AddressValue
//! [`SafeText::describe`]: crate::redact::SafeText::describe
//! [`BridgedMessage`]: crate::redact::BridgedMessage

use crate::config::CaptureMode;

/// The code every un-migrated `tracing` event carries.
///
/// Must match `BRIDGED_CODE` in `crates/catcoms-log/src/lib.rs`. Pinned by a test there, and by
/// one here, because this module treats its presence as a reason for suspicion.
pub const BRIDGED_CODE: &str = "LOG.TRACING.EVENT";

/// What a finding is evidence of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    /// A filesystem path. Carries the account name on every desktop platform.
    LocalPath,
    /// An absolute URL. Names a host, and its query string is a common place for tokens.
    Url,
    /// A key whose value is a credential: a password, a bearer token, a TURN secret.
    Credential,
    /// A literal network address in a mode that promised not to render one.
    RawAddress,
    /// A long opaque run: a raw identifier, an invite token, or key material.
    OpaqueBlob,
    /// An un-migrated `tracing` line. Its prose was never constrained, so it is suspect by
    /// construction rather than by anything spotted in it.
    BridgedProse,
}

impl Category {
    /// A stable identifier, for counting and for comparing across reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Category::LocalPath => "local_path",
            Category::Url => "url",
            Category::Credential => "credential",
            Category::RawAddress => "raw_address",
            Category::OpaqueBlob => "opaque_blob",
            Category::BridgedProse => "bridged_prose",
        }
    }

    /// What to tell somebody about to share the report.
    pub fn explain(self) -> &'static str {
        match self {
            Category::LocalPath => "a filesystem path, which usually contains your account name",
            Category::Url => "a web address, which names a host and may carry a token",
            Category::Credential => "something shaped like a password, token or shared secret",
            Category::RawAddress => "a literal network address",
            Category::OpaqueBlob => {
                "a long identifier or token that was not reduced to a reference"
            }
            Category::BridgedProse => {
                "a log line from before this checking existed, whose wording was never constrained"
            }
        }
    }

    /// Whether a finding of this kind must stop an export for a given purpose.
    ///
    /// The purpose is the whole question, and getting it wrong made the feature unusable. Refusing
    /// every category made the very first Save fail with a wall of several hundred findings: the
    /// `tracing` prose this crate has not migrated yet is written by the networking layer, which
    /// narrates every address and peer id it sees, so raw addresses and long identifiers are not
    /// escapes from the type system at all. They are the ordinary state of the log today, and a
    /// check that refuses the ordinary case is one people learn to bypass.
    ///
    /// [`Category::BridgedProse`] never refuses. It marks the un-migrated lines rather than
    /// anything spotted in them, and its value is as a measure of how far the migration has got.
    pub fn refuses(self, purpose: ExportPurpose) -> bool {
        match purpose {
            ExportPurpose::Local => false,
            ExportPurpose::Publish => !matches!(self, Category::BridgedProse),
        }
    }
}

/// Where a rendered report is about to go, which is what decides whether a finding refuses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportPurpose {
    /// Written to a file on this machine, or put on this machine's clipboard.
    ///
    /// Nothing refuses this. Saving a report into the log folder is not disclosure: it is drawn
    /// from a log file already sitting on the same disk, so refusing to write it protects nobody
    /// and costs the user the diagnostic they came for. What this purpose owes them is an honest
    /// account of what the file contains, which is [`Report::disclosure`].
    Local,
    /// Sent somewhere the user does not control: an issue tracker, a paste site, a chat.
    ///
    /// This is the boundary the review was actually about. Its words: "the planned next stage is
    /// GitHub issue submission. A false 'safe' label turns a local diagnostic failure into a
    /// public disclosure." Refusing here is worth an interruption because the alternative cannot
    /// be taken back.
    Publish,
}

/// One thing spotted, and where.
///
/// Deliberately carries no excerpt. A finding is a small structure that gets counted, logged and
/// passed around, and putting the offending text in it would copy the exact bytes this module
/// exists to contain into somewhere nobody is checking. The line number is enough to look at the
/// preview, which the user has locally anyway.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Finding {
    pub category: Category,
    /// One-based, matching what a person counting lines in the preview would say.
    pub line: usize,
}

/// The verdict on one rendered report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub lines: usize,
}

impl Report {
    /// Whether this report must not leave for `purpose` without a deliberate override.
    pub fn blocked(&self, purpose: ExportPurpose) -> bool {
        self.findings.iter().any(|f| f.category.refuses(purpose))
    }

    /// The categories that would refuse `purpose`, each with how many lines carry it.
    pub fn refusals(&self, purpose: ExportPurpose) -> Vec<(Category, usize)> {
        self.disclosure()
            .into_iter()
            .filter(|(category, _)| category.refuses(purpose))
            .collect()
    }

    /// What is in this report, aggregated: one row per category with a line count.
    ///
    /// Aggregated because the per-finding list is unreadable at real sizes. Reported verbatim, a
    /// live session produced several hundred entries of `opaque_blob at line N`, which filled the
    /// window and told the reader nothing they could act on. A count per category is the same
    /// information at a size somebody will actually read, and the line numbers remain in
    /// [`Report::findings`] for anyone who wants to jump to one.
    ///
    /// Ordered by [`Category`]'s own declaration order, so the sharper categories lead.
    pub fn disclosure(&self) -> Vec<(Category, usize)> {
        self.categories()
            .into_iter()
            .map(|category| (category, self.count(category)))
            .collect()
    }

    /// The distinct categories present, in a stable order.
    ///
    /// This is the "specific list of categories included" the review asks the preview to show. A
    /// count alone tells somebody there is a problem without telling them what to look for.
    pub fn categories(&self) -> Vec<Category> {
        let mut seen: Vec<Category> = self.findings.iter().map(|f| f.category).collect();
        seen.sort_unstable();
        seen.dedup();
        seen
    }

    /// How many findings of one kind.
    pub fn count(&self, category: Category) -> usize {
        self.findings
            .iter()
            .filter(|f| f.category == category)
            .count()
    }
}

/// Read a rendered report and say what is in it.
///
/// `mode` matters for exactly one category: under [`CaptureMode::Enhanced`] and above the user has
/// deliberately asked for literal addresses, so finding one is the feature working. Under
/// [`CaptureMode::Safe`] the same bytes mean something rendered an address that promised not to.
///
/// The caller should validate the report body and append any fixed footer afterwards. A footer
/// that names a URL to send the report to would otherwise be reported as a finding in every
/// report, which trains people to ignore the result.
pub fn validate_export(text: &str, mode: CaptureMode) -> Report {
    let mut findings = Vec::new();
    let mut lines = 0;
    for (at, line) in text.lines().enumerate() {
        lines = at + 1;
        let mut push = |category: Category| {
            findings.push(Finding {
                category,
                line: at + 1,
            })
        };
        if has_local_path(line) {
            push(Category::LocalPath);
        }
        if has_url(line) {
            push(Category::Url);
        }
        if has_credential(line) {
            push(Category::Credential);
        }
        if !mode.allows_raw_addresses() && has_raw_address(line) {
            push(Category::RawAddress);
        }
        if has_opaque_blob(line) {
            push(Category::OpaqueBlob);
        }
        if line.contains(BRIDGED_CODE) {
            push(Category::BridgedProse);
        }
    }
    Report { findings, lines }
}

// --- the scanners ------------------------------------------------------------------------------
//
// Hand-written rather than a pattern list. The frontend redactor this replaces was a set of
// regexes, and the review's complaint about it was precisely that a pattern list only ever knows
// the shapes somebody already thought of. These are not immune to that, but each one is a few
// lines of explicit reasoning that can be read and argued with, which a wall of regex cannot.

/// A filesystem path: `C:\Users\...`, `/home/...`, a UNC share, or a `file://` URL.
fn has_local_path(line: &str) -> bool {
    if line.contains("file://") || line.contains("\\\\") {
        return true;
    }
    for prefix in [
        "/home/",
        "/Users/",
        "/root/",
        "/var/folders/",
        "/private/var/",
    ] {
        if line.contains(prefix) {
            return true;
        }
    }
    has_windows_drive(line)
}

/// A Windows drive prefix: one letter, a colon, then a separator.
///
/// The letter has to stand alone. Without that check `https://` matches on its `s:` and every URL
/// in every report is also a path. A single forward slash is required for the same reason, since
/// a URL scheme is always followed by two.
fn has_windows_drive(line: &str) -> bool {
    let bytes = line.as_bytes();
    for at in 0..bytes.len() {
        if bytes[at] != b':' || at + 1 >= bytes.len() {
            continue;
        }
        if at == 0 || !bytes[at - 1].is_ascii_alphabetic() {
            continue;
        }
        // The letter must not be part of a longer word: `https:` ends in a letter too.
        if at >= 2 && (bytes[at - 2].is_ascii_alphanumeric() || bytes[at - 2] == b'_') {
            continue;
        }
        match bytes[at + 1] {
            b'\\' => return true,
            b'/' => {
                if bytes.get(at + 2) != Some(&b'/') {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// An absolute URL. Only schemes that reach a remote host: a host name is itself information.
fn has_url(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ["http://", "https://", "ws://", "wss://", "ftp://"]
        .iter()
        .any(|scheme| lower.contains(scheme))
}

/// Terms that name a secret. Only a finding when one is followed by a value.
///
/// The distinction matters: a diagnostic code like `VAULT.SECRET.CHANGED` contains the word and
/// carries nothing, while `secret=hunter2` and `"secret": "hunter2"` carry the thing itself. A
/// validator that cannot tell those apart blocks on its own event codes.
const SECRET_TERMS: &[&str] = &[
    "password",
    "passphrase",
    "secret",
    "bearer",
    "authorization",
    "credential",
    "api_key",
    "apikey",
    "access_token",
    "refresh_token",
    "private_key",
    "session_key",
    "seed_phrase",
    "mnemonic",
];

fn has_credential(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    for term in SECRET_TERMS {
        let mut from = 0;
        while let Some(found) = lower[from..].find(term) {
            let after = from + found + term.len();
            if assigns_a_value(&lower, after) {
                return true;
            }
            from = from + found + 1;
        }
    }
    false
}

/// Whether what follows an offset reads as "and here is its value".
///
/// Skips the quote and whitespace a JSON key is wrapped in, requires a `=` or `:`, then requires
/// something non-empty after it. `secret=` with nothing after is a field that was already emptied.
fn assigns_a_value(lower: &str, at: usize) -> bool {
    let bytes = lower.as_bytes();
    let mut cursor = at;
    while matches!(bytes.get(cursor), Some(b'"' | b'\'' | b' ' | b'\t')) {
        cursor += 1;
    }
    if !matches!(bytes.get(cursor), Some(b'=' | b':')) {
        return false;
    }
    cursor += 1;
    while matches!(bytes.get(cursor), Some(b'"' | b'\'' | b' ' | b'\t')) {
        cursor += 1;
    }
    matches!(bytes.get(cursor), Some(c) if !c.is_ascii_whitespace() && *c != b',' && *c != b'}')
}

/// A literal IPv4 or IPv6 address.
fn has_raw_address(line: &str) -> bool {
    has_ipv4(line) || has_ipv6(line)
}

/// Four dot-separated numbers, each 0-255.
///
/// The range check is what keeps a version string out. `0.3.0-alpha.8` has four dot-separated
/// pieces and is not an address; `999.1.1.1` is not one either.
fn has_ipv4(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        if !bytes[at].is_ascii_digit() || (at > 0 && is_address_char(bytes[at - 1])) {
            at += 1;
            continue;
        }
        let mut cursor = at;
        let mut groups = 0;
        loop {
            let start = cursor;
            let mut value: u32 = 0;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() && cursor - start < 3 {
                value = value * 10 + u32::from(bytes[cursor] - b'0');
                cursor += 1;
            }
            if cursor == start || value > 255 {
                break;
            }
            groups += 1;
            if groups == 4 {
                // A fifth group, or a trailing letter, means this was something else.
                return !matches!(bytes.get(cursor), Some(c) if is_address_char(*c));
            }
            if bytes.get(cursor) != Some(&b'.') {
                break;
            }
            cursor += 1;
        }
        at += 1;
    }
    false
}

fn is_address_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'.' || c == b'-' || c == b'_'
}

/// An IPv6 address, judged on whole tokens rather than on a count of colons.
///
/// Counting colons across a line was the first attempt and it was wrong twice over: a Rust path
/// like `the::path::to::a::module` clears any threshold, and so does a whole JSON object, whose
/// every key ends in one. So this looks at maximal runs of hex digits and colons only, which stops
/// at the first character that could not be part of an address, and asks whether the run itself
/// has the shape.
///
/// A timestamp (`12:34:56`) is such a run and is excluded by the group test: it has too few hex
/// digits and no compressed `::`.
fn has_ipv6(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if !is_ipv6_char(bytes[start]) {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < bytes.len() && is_ipv6_char(bytes[end]) {
            end += 1;
        }
        let run = &bytes[start..end];
        let colons = run.iter().filter(|c| **c == b':').count();
        let hex = run.iter().filter(|c| c.is_ascii_hexdigit()).count();
        let compressed = run.windows(2).any(|pair| pair == b"::");
        // Four hex digits is one full group. Either the address is compressed, or it is spelled
        // out in full, and eight groups means seven separators.
        if hex >= 4 && colons >= 2 && (compressed || colons >= 7) {
            return true;
        }
        start = end;
    }
    false
}

fn is_ipv6_char(c: u8) -> bool {
    c.is_ascii_hexdigit() || c == b':'
}

/// The shortest hex run treated as an identifier rather than a number.
///
/// A [`SessionRef`] renders as twelve hex characters and a trace id as sixteen, and both are meant
/// to be in a report. The threshold sits above them on purpose: this looks for the raw values they
/// exist to replace.
///
/// [`SessionRef`]: crate::redact::SessionRef
const MIN_HEX_RUN: usize = 24;

/// The shortest mixed-alphabet run treated as encoded random bytes.
const MIN_BLOB_RUN: usize = 32;

/// The length past which an unbroken run is opaque whatever it is made of.
///
/// Nothing a report legitimately contains is a single forty-four character token. Codes are
/// dotted, field names are short, and prose has spaces. This is the backstop for encodings that
/// use one case and no digits, such as lowercase base32.
const MAX_TOKEN_RUN: usize = 44;

/// A long opaque run: a raw id, an invite token, or key material.
fn has_opaque_blob(line: &str) -> bool {
    let bytes = line.as_bytes();
    let mut start = 0;
    while start < bytes.len() {
        if !is_blob_char(bytes[start]) {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < bytes.len() && is_blob_char(bytes[end]) {
            end += 1;
        }
        if looks_opaque(&bytes[start..end]) {
            return true;
        }
        start = end;
    }
    false
}

fn is_blob_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'_' || c == b'-'
}

/// Whether one run is long enough and mixed enough to be an identifier rather than words.
///
/// Three rules, in the order they were needed.
///
/// Hex-dominated rather than pure hex: a fingerprint written as `fp9f3a2b1c...` is a hex value
/// with a label stuck to the front, and requiring every character to be a hex digit let exactly
/// that through. Four fifths is enough slack for a prefix without admitting ordinary words, which
/// are full of non-hex letters.
///
/// Then a mixed-alphabet rule for encoded bytes. Two classes rather than three: base32 and some
/// base58 output is lowercase and digits with no capitals at all, and demanding all three missed
/// them.
///
/// Then a length backstop, because an encoding can use one class and no digits.
fn looks_opaque(run: &[u8]) -> bool {
    let hex = run.iter().filter(|c| c.is_ascii_hexdigit()).count();
    if run.len() >= MIN_HEX_RUN && hex * 5 >= run.len() * 4 {
        return true;
    }
    if run.len() >= MAX_TOKEN_RUN {
        return true;
    }
    if run.len() < MIN_BLOB_RUN {
        return false;
    }
    let classes = usize::from(run.iter().any(|c| c.is_ascii_uppercase()))
        + usize::from(run.iter().any(|c| c.is_ascii_lowercase()))
        + usize::from(run.iter().any(|c| c.is_ascii_digit()));
    classes >= 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Section;
    use crate::event::DiagnosticEvent;
    use crate::redact::{AddressValue, BridgedMessage, RefDomain, SafeText, SessionSalt};
    use crate::render::{event_json, event_line};

    /// Distinctive values seeded through the renderer, one per class the review names.
    ///
    /// Each is unmistakable in output: if any of these strings survives into a report, it got
    /// there from the value it was seeded as and from nowhere else.
    const CANARIES: &[(&str, &str)] = &[
        ("message_text", "CANARYTEXT meet me at the usual place"),
        ("server_name", "CANARYSERVER Private Support"),
        ("channel_name", "CANARYCHANNEL after-hours"),
        ("unix_path", "/home/canaryuser/.config/mewtual/vault.bin"),
        (
            "windows_path",
            "C:\\Users\\CanaryUser\\AppData\\Roaming\\mewtual",
        ),
        (
            "peer_id",
            "12D3KooWCanaryPeerIdentifier9xQvT4rZmN7bKdL2sWpA",
        ),
        (
            "device_fp",
            "canary9f3a2b1c8d7e6f5a4b3c2d1e0f9a8b7c6d5e4f3a2b1c0d9e",
        ),
        (
            "group_id",
            "canarygroup0011223344556677889900aabbccddeeff0011",
        ),
        (
            "cid",
            "bafybeicanarycidzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ),
        (
            "invite_token",
            "mwtinviteCanaryTokenAAAABBBBCCCCDDDD1111222233334444",
        ),
        ("turn_credential", "password=CanaryTurnCredential99"),
        ("vault_secret", "passphrase=correct horse canary battery"),
        (
            "url_query_token",
            "https://example.invalid/report?token=CanaryQueryToken1",
        ),
    ];

    fn salt() -> SessionSalt {
        SessionSalt::for_tests(7)
    }

    fn both_renderings(event: &DiagnosticEvent, mode: CaptureMode) -> [String; 2] {
        [event_line(event, mode), event_json(event, mode)]
    }

    /// The property the whole reference mechanism exists for.
    ///
    /// A reference is a keyed hash of the value it stands for, so every one of these can be fed in
    /// and none of them can come back out. This is the strongest guarantee in the crate and it is
    /// worth pinning against every canary rather than one representative, because "identifier"
    /// covers peer ids, fingerprints, group ids and CIDs, and they have nothing in common but
    /// their purpose.
    #[test]
    fn a_reference_never_carries_the_value_it_stands_for() {
        let salt = salt();
        for (name, canary) in CANARIES {
            for domain in [
                RefDomain::Server,
                RefDomain::Peer,
                RefDomain::Channel,
                RefDomain::Document,
            ] {
                let event = DiagnosticEvent::info(Section::Join, "JOIN.CANARY.PROBE")
                    .field("subject", salt.reference(domain, canary.as_bytes()));
                for mode in [CaptureMode::Safe, CaptureMode::Enhanced, CaptureMode::Full] {
                    for rendered in both_renderings(&event, mode) {
                        assert!(
                            !rendered.contains(canary),
                            "{name} survived as a reference at {mode:?}: {rendered}"
                        );
                    }
                }
            }
        }
    }

    /// Safe capture renders no literal address, whatever was put in.
    #[test]
    fn safe_capture_renders_no_literal_address() {
        for raw in [
            "/ip4/198.51.100.7/udp/31484/quic-v1",
            "/ip6/2001:db8::dead:beef/udp/31484/quic-v1",
            "203.0.113.42:8080",
        ] {
            let event = DiagnosticEvent::info(Section::Transport, "TRANSPORT.CANARY.PROBE")
                .field("address", AddressValue::new(raw));
            for rendered in both_renderings(&event, CaptureMode::Safe) {
                assert!(
                    !rendered.contains(raw),
                    "a literal address survived safe capture: {rendered}"
                );
                let report = validate_export(&rendered, CaptureMode::Safe);
                assert_eq!(
                    report.count(Category::RawAddress),
                    0,
                    "safe output tripped the address scanner: {rendered}"
                );
            }
        }
    }

    /// The two variants that can carry anything, and the reason this module exists.
    ///
    /// `SafeText::describe` and `BridgedMessage` both take arbitrary text on purpose, so the type
    /// system does not stop a canary reaching a report through either. What must hold instead is
    /// that the validator sees it. A canary that neither the types nor the validator catch is the
    /// exact failure the review reported.
    #[test]
    fn anything_prose_can_carry_is_something_the_validator_catches() {
        for (name, canary) in CANARIES {
            // Only the canaries that are meant to be identifiable in free text. A bare server
            // name is prose and cannot be distinguished from prose, which is why the footer must
            // not promise that names are absent: see the wording note in P3-002.
            if matches!(*name, "message_text" | "server_name" | "channel_name") {
                continue;
            }
            let described = DiagnosticEvent::warn(Section::Join, "JOIN.CANARY.PROBE")
                .field("reason", SafeText::describe(canary));
            let bridged = DiagnosticEvent::warn(Section::Join, BRIDGED_CODE)
                .field("message", BridgedMessage::new(canary));
            for event in [described, bridged] {
                for rendered in both_renderings(&event, CaptureMode::Safe) {
                    let report = validate_export(&rendered, CaptureMode::Safe);
                    assert!(
                        report.blocked(ExportPurpose::Publish),
                        "{name} reached a safe report unnoticed: {rendered}"
                    );
                }
            }
        }
    }

    /// A bridged line is reported even when nothing specific was spotted in it.
    #[test]
    fn an_unmigrated_line_is_suspect_on_its_own() {
        let event = DiagnosticEvent::info(Section::Join, BRIDGED_CODE)
            .field("message", BridgedMessage::new("nothing alarming here"));
        let rendered = event_line(&event, CaptureMode::Safe);
        let report = validate_export(&rendered, CaptureMode::Safe);
        assert_eq!(report.count(Category::BridgedProse), 1);
        // Suspicion alone does not refuse, even at the publishing boundary. Today most lines are
        // bridged, and a check that refuses every report is a check people learn to bypass.
        assert!(!report.blocked(ExportPurpose::Publish));
        assert_eq!(report.categories(), vec![Category::BridgedProse]);
    }

    #[test]
    fn a_local_path_is_found_on_either_platform() {
        for line in [
            "failed to render: C:\\Users\\Marisa\\Documents\\notes.txt",
            "opened /home/marisa/.config/mewtual/log",
            "opened /Users/marisa/Library/Application Support/mewtual",
            "wrote file:///tmp/report.txt",
            "share \\\\FILESERVER\\team",
        ] {
            let report = validate_export(line, CaptureMode::Safe);
            assert_eq!(
                report.count(Category::LocalPath),
                1,
                "missed a path in: {line}"
            );
            assert!(report.blocked(ExportPurpose::Publish));
            // But it never stops the user writing the file to their own disk, which is where the
            // log it was drawn from already lives.
            assert!(!report.blocked(ExportPurpose::Local));
        }
    }

    /// The false positives that would make the check useless by blocking everything.
    #[test]
    fn ordinary_report_text_is_not_mistaken_for_a_leak() {
        for line in [
            "JOIN.ROUTES.EXHAUSTED phase=failure took=60123ms attempt=4",
            "VAULT.SECRET.CHANGED outcome=accepted",
            "12:34:56.789 INFO transport ready",
            "version=0.3.0-alpha.8",
            "trace=7f2c000000000001 span=91ab parent=6dc4",
            "server=[srv 3f2a9c] peer=[peer 91ab44]",
            "reason=no advertised route completed",
            "the::path::to::a::rust::module",
            "ratio=1.2.3.400 is not an address",
        ] {
            let report = validate_export(line, CaptureMode::Safe);
            assert_eq!(
                report.findings,
                Vec::new(),
                "ordinary text was flagged: {line} -> {:?}",
                report.categories()
            );
        }
    }

    /// A word that names a secret is not the same as a secret.
    #[test]
    fn a_secret_term_only_counts_when_it_has_a_value() {
        assert_eq!(
            validate_export("password=hunter2", CaptureMode::Safe).count(Category::Credential),
            1
        );
        assert_eq!(
            validate_export("  \"secret\": \"hunter2\"", CaptureMode::Safe)
                .count(Category::Credential),
            1
        );
        // Named but empty, and named but not assigned: neither carries anything.
        assert_eq!(
            validate_export("password=", CaptureMode::Safe).count(Category::Credential),
            0
        );
        assert_eq!(
            validate_export("VAULT.PASSPHRASE.ROTATED count=2", CaptureMode::Safe)
                .count(Category::Credential),
            0
        );
    }

    /// Addresses are a finding only where the mode promised not to render them.
    #[test]
    fn a_raw_address_is_judged_against_the_mode_that_was_promised() {
        let line = "dialled 198.51.100.7";
        assert_eq!(
            validate_export(line, CaptureMode::Safe).count(Category::RawAddress),
            1,
            "safe capture promised no literal addresses"
        );
        for mode in [CaptureMode::Enhanced, CaptureMode::Full] {
            assert_eq!(
                validate_export(line, mode).count(Category::RawAddress),
                0,
                "{mode:?} was asked for literal addresses, so one is the feature working"
            );
        }
    }

    /// The threshold sits above the values a report is meant to contain.
    #[test]
    fn a_reference_is_not_mistaken_for_the_id_it_replaces() {
        // Twelve hex for a reference, sixteen for a trace: both belong in a report.
        assert!(!has_opaque_blob("ref=3f2a9c8d7e6b"));
        assert!(!has_opaque_blob("trace=7f2c000000000001"));
        // Long enough to be a raw key or group id, and nothing else in a report looks like this.
        assert!(has_opaque_blob("raw=0011223344556677889900aabbccddeeff"));
        assert!(has_opaque_blob(
            "token=mwtinviteCanaryTokenAAAABBBBCCCCDDDD1111222233334444"
        ));
    }

    /// Pinned here as well as in `catcoms-log`, because this module treats it as a signal.
    #[test]
    fn the_bridged_code_matches_the_logger() {
        assert_eq!(BRIDGED_CODE, "LOG.TRACING.EVENT");
    }

    #[test]
    fn a_clean_report_says_so_without_claiming_more() {
        let report = validate_export(
            "JOIN.ROUTES.EXHAUSTED attempt=4\nTRANSPORT.DIAL.OK",
            CaptureMode::Safe,
        );
        assert!(!report.blocked(ExportPurpose::Publish));
        assert_eq!(report.lines, 2);
        assert!(report.categories().is_empty());
    }

    /// The case from the screenshot: a real session's log, and what it must not do.
    ///
    /// The networking layer's un-migrated prose narrates addresses and peer ids, so a genuine
    /// report carries hundreds of them. Saving that to the user's own log folder has to work.
    #[test]
    fn a_real_session_log_still_saves_and_says_what_is_in_it() {
        let mut body = String::from("== BACKEND ==\n");
        for at in 0..200 {
            body.push_str(&format!(
                "LOG.TRACING.EVENT message=dialling 198.51.100.{} via 12D3KooWQ9xTvR4bKdL2sWpAmN7zYcEfGh{:02}\n",
                at % 250,
                at % 100,
            ));
        }
        let report = validate_export(&body, CaptureMode::Safe);
        assert!(
            !report.blocked(ExportPurpose::Local),
            "saving a real session's own log to its own disk must not be refused"
        );
        assert!(
            report.blocked(ExportPurpose::Publish),
            "but posting it is another matter"
        );

        // And what the user is told is a handful of rows, not a finding per line. The wall of
        // several hundred entries is what made the refusal unreadable as well as wrong.
        let disclosure = report.disclosure();
        assert!(disclosure.len() <= 4, "{disclosure:?}");
        assert!(
            report.findings.len() > 200,
            "the detail is still there to jump to"
        );
        let raw = disclosure
            .iter()
            .find(|(category, _)| *category == Category::RawAddress)
            .expect("the addresses are disclosed rather than hidden");
        assert!(raw.1 > 100, "and counted honestly: {raw:?}");
    }
}
