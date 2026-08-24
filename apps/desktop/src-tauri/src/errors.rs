//! Typed errors across the IPC boundary.
//!
//! # What a string error costs
//!
//! Almost every command in the bridge returns `Result<T, String>`, and the frontend shows
//! `String(e)`. That single choice discards, at the boundary, everything a person debugging needs:
//! a stable code to search for, which subsystem failed, whether retrying could possibly help,
//! whether state changed before the failure, whether the user can act on it, and which trace the
//! failure belongs to. What survives is a sentence, and a sentence cannot be counted, grouped,
//! de-duplicated in an issue tracker, or tested against.
//!
//! # What this does not do
//!
//! It does not replace the message the user sees. Today a failed send says "message too long", and
//! swapping that for "Message could not be sent" would be a regression dressed up as an
//! improvement: the specific text is the part that tells someone what to do differently. So the
//! message is preserved exactly, and the code, trace and retryability are *added* alongside it.
//!
//! # The migration
//!
//! One command at a time. `describeError` on the frontend reads both shapes, so it can be rolled
//! out to a call site before that call site's command has been migrated, and a partly-migrated
//! bridge behaves identically to an unmigrated one. Nothing here requires a flag day.

use serde::Serialize;

/// The most of a message that crosses the boundary.
///
/// Errors from deeper layers interpolate values, and an unbounded one could carry a great deal of
/// whatever was in scope. This is generous enough for every real message and small enough that a
/// hostile one is not a payload.
const MAX_MESSAGE: usize = 500;

/// What the user should do about a failure, when there is something to do.
///
/// A closed set rather than free text, so the UI can offer the action rather than describing it,
/// and so a new remediation is a deliberate addition rather than a sentence somebody typed.
///
/// Only what something can currently return. `check_connection` belongs here too and arrives with
/// the subsystem that needs it: an enum listing outcomes no code can produce invites a UI branch
/// for a case that never happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Remediation {
    /// The session is locked; unlocking is the whole fix.
    Unlock,
    /// The input was refused. Changing it is the fix.
    AmendInput,
    /// Transient. Trying the same thing again is reasonable, usually because the cause is that
    /// nobody holding what was asked for is reachable right now, and that changes.
    Retry,
    /// The app is in a state only a restart clears, usually a task that stopped.
    Restart,
}

/// One kind of failure the bridge can report.
///
/// Registered rather than invented at the call site: a code that exists in one branch of one
/// function cannot be searched for, documented, or counted, and the whole value of a stable code is
/// that it outlives the wording around it.
///
/// The fields are private, and that is the enforcement rather than a style preference. While they
/// were public any module could write `ErrorCode { code: "WHATEVER", .. }` at a call site, ship a
/// code the registry had never heard of, and break nothing: the registry tests only ever looked at
/// what was already in `ALL`, so the one thing they could not see was an omission.
#[derive(Clone, Copy, Debug)]
pub struct ErrorCode {
    /// `AREA.COMPONENT.OUTCOME`, stable across rewording.
    code: &'static str,
    /// Whether trying the same thing again could plausibly work.
    retryable: bool,
    remediation: Option<Remediation>,
}

impl ErrorCode {
    /// The stable code, for a caller that records a failure as well as returning it.
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

/// The codes this bridge can return.
///
/// Every one is declared through `error_codes!`, which builds the constant and the manifest from
/// the same line, so "registered" is not a second step anybody can forget. Together with the
/// private fields above that closes the gap the registry used to have: outside this module a code
/// cannot be assembled at all, and inside it a code cannot exist without appearing in `ALL`.
pub mod codes {
    use super::ErrorCode;
    use super::Remediation::{AmendInput, Restart, Retry, Unlock};

    /// Declare a code and register it in one act.
    ///
    /// The registry used to be two lists that had to agree by hand: a constant, and a line in
    /// `ALL`. A constant that never reached `ALL` compiled, worked, and was invisible to every
    /// test here, so the manifest could sit quietly behind what the bridge actually emits. One
    /// list removes the disagreement rather than testing for it.
    macro_rules! error_codes {
        ($(
            $(#[$note:meta])*
            $name:ident = $code:literal, retryable: $retryable:literal, $remediation:expr;
        )+) => {
            $(
                $(#[$note])*
                pub const $name: ErrorCode = ErrorCode {
                    code: $code,
                    retryable: $retryable,
                    remediation: $remediation,
                };
            )+

            /// Every registered code, for the tests that keep this honest.
            #[cfg_attr(not(test), allow(dead_code))]
            pub const ALL: &[ErrorCode] = &[$($name),+];

            /// The same codes under the names call sites use, which is what the guard against a
            /// hand-written constant needs: a call site says `codes::SESSION_LOCKED` rather than
            /// `"SESSION.LOCKED"`, so identifiers are the only thing the two can be compared by.
            #[cfg_attr(not(test), allow(dead_code))]
            pub const NAMES: &[&str] = &[$(stringify!($name)),+];
        };
    }

    error_codes! {
        /// The session is locked, so nothing that touches a server can proceed.
        SESSION_LOCKED = "SESSION.LOCKED", retryable: true, Some(Unlock);

        /// No actor for that server. Either it was never opened, or its task has stopped, and
        /// those are very different problems that used to produce the same sentence.
        SERVER_UNAVAILABLE = "SERVER.ACTOR.UNAVAILABLE", retryable: false, Some(Restart);

        /// A channel id that is not a channel id. Always a caller bug rather than a user one.
        CHANNEL_BAD_ID = "CHANNEL.ID.INVALID", retryable: false, None;

        CHAT_SEND_REJECTED = "CHAT.SEND.REJECTED", retryable: false, Some(AmendInput);

        CHAT_EDIT_REJECTED = "CHAT.EDIT.REJECTED", retryable: false, Some(AmendInput);

        CHAT_DELETE_REJECTED = "CHAT.DELETE.REJECTED", retryable: false, None;

        CHAT_REACTION_REJECTED = "CHAT.REACTION.REJECTED", retryable: false, None;

        CHAT_PIN_REJECTED = "CHAT.PIN.REJECTED", retryable: false, None;

        /// A queue add refused: not a content address, a blank or over-long name, or the 64-entry
        /// cap. All four are the input, which is why the user can fix it.
        JUKEBOX_ADD_REJECTED = "JUKEBOX.ADD.REJECTED", retryable: false, Some(AmendInput);

        /// Removal is idempotent, so a refusal is never about the entry being absent.
        JUKEBOX_REMOVE_REJECTED = "JUKEBOX.REMOVE.REJECTED", retryable: false, None;

        /// An upload refused before a byte moved: a malformed id, or a file over the size limit.
        FILE_UPLOAD_REFUSED = "FILE.UPLOAD.REFUSED", retryable: false, Some(AmendInput);

        /// An upload that began and could not be completed. Distinct from a refusal because bytes
        /// have already moved and a reservation is being held.
        FILE_UPLOAD_FAILED = "FILE.UPLOAD.FAILED", retryable: true, Some(Retry);

        /// A download that could not be completed. Retryable because the usual cause is that
        /// nobody holding the bytes is reachable right now, which changes.
        FILE_DOWNLOAD_FAILED = "FILE.DOWNLOAD.FAILED", retryable: true, Some(Retry);

        /// A call-signalling message that could not be handed to the transport at all. Distinct
        /// from one that was sent to a member with no route: that is an outcome, not an error.
        VOICE_SIGNAL_FAILED = "VOICE.SIGNAL.FAILED", retryable: true, Some(Retry);

        /// The vault would not open. Overwhelmingly a wrong passphrase, which is why it is
        /// retryable and why the message is left exactly as it was: telling somebody their
        /// passphrase is wrong is the entire useful content of this failure.
        VAULT_LOCKED_OUT = "VAULT.OPEN.REFUSED", retryable: true, Some(AmendInput);

        /// The vault opened and something inside it could not be read. A different problem from a
        /// wrong passphrase, and one the user cannot fix by typing more carefully.
        VAULT_READ_FAILED = "VAULT.READ.FAILED", retryable: false, Some(Restart);

        /// A backup that could not be written.
        VAULT_BACKUP_FAILED = "VAULT.BACKUP.FAILED", retryable: true, Some(Retry);

        /// A write to a server document (wiki, status, calendar) that was refused. Almost always
        /// the content: too long, malformed, or a name that is not allowed.
        DOCUMENT_WRITE_REJECTED = "DOCUMENT.WRITE.REJECTED", retryable: false, Some(AmendInput);

        /// A channel topic refused; over the byte limit, in practice.
        CHANNEL_TOPIC_REJECTED = "CHANNEL.TOPIC.REJECTED", retryable: false, Some(AmendInput);
    }
}

/// A failure, as the frontend receives it.
///
/// Serialised as an object, so a migrated call site reads fields rather than parsing prose. The
/// frontend's `describeError` handles both this and a bare string, which is what lets commands
/// migrate one at a time.
///
/// The fields are private so that `new` is the only way to make one. A hand-assembled `AppError`
/// would put whatever `&'static str` somebody typed on the wire without ever passing a registered
/// code, which is the same escape from the registry that `ErrorCode`'s public fields used to be.
#[derive(Clone, Debug, Serialize)]
pub struct AppError {
    /// The stable code. Searchable, groupable, and unchanged when the wording improves.
    code: &'static str,
    /// What the user sees. Deliberately the same text they see today: the specific message is the
    /// part that tells somebody what to do differently, and replacing it with a generic sentence
    /// would be a regression wearing an improvement's clothes.
    message: String,
    /// The operation this belonged to, short enough to quote in a bug report.
    ///
    /// The link between a user saying "it failed" and the twelve events that describe why.
    trace: String,
    retryable: bool,
    remediation: Option<Remediation>,
}

impl AppError {
    /// Build a failure from a registered code and the message the user should see.
    pub fn new(code: ErrorCode, message: impl Into<String>, trace: &str) -> Self {
        let mut message = message.into();
        if message.chars().count() > MAX_MESSAGE {
            message = message.chars().take(MAX_MESSAGE).collect();
        }
        AppError {
            code: code.code,
            message,
            trace: trace.to_string(),
            retryable: code.retryable,
            remediation: code.remediation,
        }
    }

    /// The user-facing text, for a command that has not been migrated and still returns a `String`.
    pub fn into_message(self) -> String {
        self.message
    }
}

impl std::fmt::Display for AppError {
    /// Reads the way the old string error did, plus the code.
    ///
    /// This is what a caller that has *not* been migrated will show if it stringifies the error, so
    /// it has to stay useful on its own.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} [{}]", self.message, self.code)
    }
}

impl std::error::Error for AppError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every bridge source except this one, which is where codes are declared rather than used.
    ///
    /// Read from disk rather than `include_str!` of a fixed list, because a module added later
    /// would otherwise be invisible to the guards below and they would quietly narrow to whatever
    /// the file list said years ago.
    fn call_site_sources() -> String {
        let mut pending = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        let mut joined = String::new();
        while let Some(dir) = pending.pop() {
            let entries = std::fs::read_dir(&dir).expect("the bridge sources must be readable");
            for entry in entries {
                let path = entry.expect("the bridge sources must be readable").path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && path.file_name().is_some_and(|n| n != "errors.rs")
                {
                    joined.push_str(&std::fs::read_to_string(&path).expect("unreadable source"));
                }
            }
        }
        joined
    }

    /// Every `codes::NAME` a source names, whichever constant that turns out to be.
    ///
    /// Finding none is a failure rather than a quiet pass. Both guards below are satisfied by an
    /// empty set, so a scan that stopped matching, because the sources moved or the call shape
    /// changed, would leave two tests reporting success while checking nothing at all.
    fn names_used_by_call_sites(source: &str) -> BTreeSet<&str> {
        let names: BTreeSet<&str> = source
            .match_indices("codes::")
            .map(|(at, marker)| {
                let rest = &source[at + marker.len()..];
                let end = rest
                    .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                    .unwrap_or(rest.len());
                &rest[..end]
            })
            .collect();
        assert!(
            !names.is_empty(),
            "the scan found no call sites, so it is checking nothing"
        );
        names
    }

    /// A constant written out by hand beside the macro would compile, be usable from a command,
    /// and never reach `ALL`, which is the exact drift the macro exists to remove. The compiler
    /// cannot see the difference, so this reads the file.
    #[test]
    fn the_macro_is_the_only_thing_that_mints_a_code() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/errors.rs"),
        )
        .expect("this file must be readable");
        // Only what the app compiles. Below this marker sits the test that spells out the pattern
        // it is looking for, and a scan of the whole file counts itself as a second definition.
        let shipped = &source[..source
            .find("#[cfg(test)]")
            .expect("this guard cannot find where the shipped half of the file ends")];
        let macro_at = shipped
            .find("macro_rules! error_codes")
            .expect("this guard cannot find the macro it is guarding");
        let mints: Vec<usize> = shipped
            .match_indices(": ErrorCode =")
            .map(|(at, _)| at)
            .collect();
        assert_eq!(
            mints.len(),
            1,
            "a code is defined outside error_codes!, so it will not appear in ALL"
        );
        assert!(
            mints[0] > macro_at,
            "a code is defined before the macro, so it will not appear in ALL"
        );
    }

    /// A registry that runs ahead of the bridge is the same lie as one that lags behind it: a code
    /// nothing can emit still gets documented, searched for, and looked up by somebody holding a
    /// bug report that will never contain it.
    #[test]
    fn every_registered_code_is_reachable_from_a_call_site() {
        let sources = call_site_sources();
        let used = names_used_by_call_sites(&sources);
        for name in codes::NAMES {
            assert!(
                used.contains(name),
                "{name} is registered but nothing emits it"
            );
        }
    }

    /// The other direction, which catches the hand-written constant the guard above misses if it
    /// is ever written in a shape the file scan does not match: a call site can only name a
    /// constant, so a name the manifest does not know is a code outside the registry.
    #[test]
    fn every_code_a_call_site_names_is_registered() {
        let registered: BTreeSet<&str> = codes::NAMES.iter().copied().collect();
        for name in names_used_by_call_sites(&call_site_sources()) {
            assert!(
                registered.contains(name),
                "codes::{name} is used but is not declared in error_codes!"
            );
        }
    }

    /// A code that appears twice means two different failures group together in an issue tracker
    /// and in every count derived from them.
    #[test]
    fn every_registered_code_is_unique() {
        let mut seen: Vec<&str> = codes::ALL.iter().map(|c| c.code).collect();
        let total = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), total, "a code is registered twice");
    }

    /// The shape is the contract. `AREA.COMPONENT.OUTCOME` is what makes a code greppable across
    /// the codebase, the log, and a pile of issues.
    #[test]
    fn every_code_follows_the_naming_convention() {
        for entry in codes::ALL {
            let parts: Vec<&str> = entry.code.split('.').collect();
            assert!(
                parts.len() >= 2,
                "{} needs at least AREA.OUTCOME",
                entry.code
            );
            for part in parts {
                assert!(!part.is_empty(), "{} has an empty segment", entry.code);
                assert!(
                    part.chars()
                        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()),
                    "{} is not upper snake case",
                    entry.code
                );
            }
        }
    }

    /// A failure the user can do something about has to say what. Offering "retry" on something
    /// that will never succeed wastes their time, which is worse than saying nothing.
    #[test]
    fn a_retryable_failure_says_what_would_make_it_work() {
        for entry in codes::ALL {
            if entry.retryable {
                assert!(
                    entry.remediation.is_some(),
                    "{} is retryable but suggests nothing",
                    entry.code
                );
            }
        }
    }

    /// The whole point of the migration: the user keeps the message they had, and gains a code.
    #[test]
    fn the_message_the_user_already_saw_is_preserved_exactly() {
        let error = AppError::new(codes::CHAT_SEND_REJECTED, "message too long", "7f2c");
        assert_eq!(error.message, "message too long");
        assert_eq!(error.code, "CHAT.SEND.REJECTED");
        assert_eq!(error.trace, "7f2c");
        assert!(!error.retryable);
        assert_eq!(error.remediation, Some(Remediation::AmendInput));
    }

    /// A caller that has not been migrated stringifies the error, so that has to stay readable.
    #[test]
    fn an_unmigrated_caller_still_gets_a_sentence() {
        let shown = AppError::new(codes::SESSION_LOCKED, "session is locked", "0001").to_string();
        assert_eq!(shown, "session is locked [SESSION.LOCKED]");
    }

    /// Errors from deeper layers interpolate values, and an unbounded message is a payload.
    #[test]
    fn a_message_from_a_deeper_layer_is_bounded() {
        let error = AppError::new(codes::CHAT_SEND_REJECTED, "x".repeat(5000), "7f2c");
        assert_eq!(error.message.chars().count(), MAX_MESSAGE);
    }

    /// The frontend reads fields, so the field names are the contract.
    #[test]
    fn the_serialised_shape_is_what_the_frontend_reads() {
        let json =
            serde_json::to_string(&AppError::new(codes::SESSION_LOCKED, "locked", "1")).unwrap();
        assert!(json.contains("\"code\":\"SESSION.LOCKED\""), "{json}");
        assert!(json.contains("\"message\":\"locked\""), "{json}");
        assert!(json.contains("\"trace\":\"1\""), "{json}");
        assert!(json.contains("\"retryable\":true"), "{json}");
        // Snake case, matching the Remediation union the frontend declares.
        assert!(json.contains("\"remediation\":\"unlock\""), "{json}");
    }

    #[test]
    fn a_failure_with_nothing_to_suggest_says_so_rather_than_guessing() {
        let json =
            serde_json::to_string(&AppError::new(codes::CHANNEL_BAD_ID, "bad", "1")).unwrap();
        assert!(json.contains("\"remediation\":null"), "{json}");
    }
}
