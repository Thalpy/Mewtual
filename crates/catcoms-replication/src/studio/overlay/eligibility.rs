//! Why a retained local draft can or cannot be handed off automatically (design section 7, P2).
//!
//! A classification, never an authority. It reads durable state and answers "would the automatic
//! handoff accept this branch now, and if not, what should the user be told". Nothing here mints a
//! basis, signs, or changes a record, and a `Transferable` answer is a description of the present,
//! not a promise: the handoff re-derives everything under custody when it actually runs.

/// What a user can do about a retained draft right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOverlayEligibility {
    /// The automatic handoff's own preconditions hold: the installed successor is exactly the one
    /// this branch was based on and untouched, this device authored the branch, and it can prove
    /// the current owner's tenure under which the branch's receipt is still current.
    Transferable,
    /// Automatic handoff will refuse; the manual path (export, archive, copy, disposal) remains.
    Manual(StudioOverlayManualReason),
}

/// The one reason given for a `Manual` draft. When several apply, the most permanent wins, so a
/// user is not told to wait for something that would not help: see the order in
/// `StudioEpoch::overlay_successor_hold` and the app-level classifier.
///
/// `closeMissing` from design section 11 is deliberately absent. A Closing branch carries its
/// receipt, and nothing in the handoff precondition consults the close record separately, so no
/// durable state produces that reason; a variant nothing can produce would be a promise the
/// classifier cannot keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StudioOverlayManualReason {
    /// The branch could not be reconstructed. Permanent: only export, archive, preserving disposal
    /// and discard remain, and none of them needs reconstruction.
    NotReplayable,
    /// A preview-based branch. It has no installed source to hand off into.
    Unconfirmed,
    /// Another device authored this branch. Only its author may transfer it.
    NotCurrentAuthor,
    /// No installed source for the document.
    SourceMissing,
    /// An installed source exists but could not be read or does not belong with this branch.
    /// Not in design section 11's list: the alternative was letting the lifecycle read fail
    /// outright, which would hide a branch the user needs to export or archive behind an error
    /// about a different record.
    SourceUnreadable,
    /// The source is faulted.
    Fault,
    /// The source is at an EARLIER epoch than the one this branch's receipt closed.
    SourceRewound,
    /// The source is at the closed epoch but no longer Closing.
    SourceNotClosing,
    /// The source is still Closing at the closed epoch: its successor is not installed yet.
    SuccessorMissing,
    /// The successor was opened by a different receipt or owner than this branch's.
    ReceiptChanged,
    /// The source has moved past the successor, or carries a different seed.
    SourceReplaced,
    /// The successor already holds other work, is adopting, or is no longer Open.
    SuccessorNotPristine,
    /// This device cannot presently prove the current owner's tenure. Nothing is wrong with the
    /// work; a handoff signs under that tenure and must not guess it.
    TenureUnknown,
}
