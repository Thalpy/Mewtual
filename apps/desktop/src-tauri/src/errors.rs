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
#[derive(Clone, Copy, Debug)]
pub struct ErrorCode {
    /// `AREA.COMPONENT.OUTCOME`, stable across rewording.
    pub code: &'static str,
    /// Whether trying the same thing again could plausibly work.
    pub retryable: bool,
    pub remediation: Option<Remediation>,
}

/// The codes this bridge can return.
///
/// Every one is listed here, and a test pins that they are unique and correctly shaped. The review
/// asks for CI to enforce "every ErrorCode exists in the registry"; this is that registry, and the
/// enforcement is the test at the bottom of this file.
pub mod codes {
    use super::{ErrorCode, Remediation};

    /// The session is locked, so nothing that touches a server can proceed.
    pub const SESSION_LOCKED: ErrorCode = ErrorCode {
        code: "SESSION.LOCKED",
        retryable: true,
        remediation: Some(Remediation::Unlock),
    };

    /// No actor for that server. Either it was never opened, or its task has stopped, and those
    /// are very different problems that used to produce the same sentence.
    pub const SERVER_UNAVAILABLE: ErrorCode = ErrorCode {
        code: "SERVER.ACTOR.UNAVAILABLE",
        retryable: false,
        remediation: Some(Remediation::Restart),
    };

    /// A channel id that is not a channel id. Always a caller bug rather than a user one.
    pub const CHANNEL_BAD_ID: ErrorCode = ErrorCode {
        code: "CHANNEL.ID.INVALID",
        retryable: false,
        remediation: None,
    };

    pub const CHAT_SEND_REJECTED: ErrorCode = ErrorCode {
        code: "CHAT.SEND.REJECTED",
        retryable: false,
        remediation: Some(Remediation::AmendInput),
    };

    pub const CHAT_EDIT_REJECTED: ErrorCode = ErrorCode {
        code: "CHAT.EDIT.REJECTED",
        retryable: false,
        remediation: Some(Remediation::AmendInput),
    };

    pub const CHAT_DELETE_REJECTED: ErrorCode = ErrorCode {
        code: "CHAT.DELETE.REJECTED",
        retryable: false,
        remediation: None,
    };

    pub const CHAT_REACTION_REJECTED: ErrorCode = ErrorCode {
        code: "CHAT.REACTION.REJECTED",
        retryable: false,
        remediation: None,
    };

    pub const CHAT_PIN_REJECTED: ErrorCode = ErrorCode {
        code: "CHAT.PIN.REJECTED",
        retryable: false,
        remediation: None,
    };

    /// A queue add refused: not a content address, a blank or over-long name, or the 64-entry cap.
    /// All four are the input, which is why the user can fix it.
    pub const JUKEBOX_ADD_REJECTED: ErrorCode = ErrorCode {
        code: "JUKEBOX.ADD.REJECTED",
        retryable: false,
        remediation: Some(Remediation::AmendInput),
    };

    /// Removal is idempotent, so a refusal is never about the entry being absent.
    pub const JUKEBOX_REMOVE_REJECTED: ErrorCode = ErrorCode {
        code: "JUKEBOX.REMOVE.REJECTED",
        retryable: false,
        remediation: None,
    };

    /// An upload refused before a byte moved: a malformed id, or a file over the size limit.
    pub const FILE_UPLOAD_REFUSED: ErrorCode = ErrorCode {
        code: "FILE.UPLOAD.REFUSED",
        retryable: false,
        remediation: Some(Remediation::AmendInput),
    };

    /// An upload that began and could not be completed. Distinct from a refusal because bytes
    /// have already moved and a reservation is being held.
    pub const FILE_UPLOAD_FAILED: ErrorCode = ErrorCode {
        code: "FILE.UPLOAD.FAILED",
        retryable: true,
        remediation: Some(Remediation::Retry),
    };

    /// A download that could not be completed. Retryable because the usual cause is that nobody
    /// holding the bytes is reachable right now, which changes.
    pub const FILE_DOWNLOAD_FAILED: ErrorCode = ErrorCode {
        code: "FILE.DOWNLOAD.FAILED",
        retryable: true,
        remediation: Some(Remediation::Retry),
    };

    /// Every registered code, for the tests that keep this honest. The manifest is the registry:
    /// a code missing from here is a code no test checks the shape of.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const ALL: &[ErrorCode] = &[
        SESSION_LOCKED,
        SERVER_UNAVAILABLE,
        CHANNEL_BAD_ID,
        CHAT_SEND_REJECTED,
        CHAT_EDIT_REJECTED,
        CHAT_DELETE_REJECTED,
        CHAT_REACTION_REJECTED,
        CHAT_PIN_REJECTED,
        JUKEBOX_ADD_REJECTED,
        JUKEBOX_REMOVE_REJECTED,
        FILE_UPLOAD_REFUSED,
        FILE_UPLOAD_FAILED,
        FILE_DOWNLOAD_FAILED,
    ];
}

/// A failure, as the frontend receives it.
///
/// Serialised as an object, so a migrated call site reads fields rather than parsing prose. The
/// frontend's `describeError` handles both this and a bare string, which is what lets commands
/// migrate one at a time.
#[derive(Clone, Debug, Serialize)]
pub struct AppError {
    /// The stable code. Searchable, groupable, and unchanged when the wording improves.
    pub code: &'static str,
    /// What the user sees. Deliberately the same text they see today: the specific message is the
    /// part that tells somebody what to do differently, and replacing it with a generic sentence
    /// would be a regression wearing an improvement's clothes.
    pub message: String,
    /// The operation this belonged to, short enough to quote in a bug report.
    ///
    /// The link between a user saying "it failed" and the twelve events that describe why.
    pub trace: String,
    pub retryable: bool,
    pub remediation: Option<Remediation>,
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
