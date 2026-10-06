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
/// `StudioEpoch::overlay_successor_hold_in_vault` and the app-level classifier.
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
    /// An Index entry names a Flipnote whose source is absent or empty here. The handoff refuses
    /// to publish an Index entry pointing at nothing (`check_index_object_sources`). Not in design
    /// section 11's list; added so the classifier names that refusal rather than calling the
    /// branch transferable.
    ObjectMissing,
    /// This device holds no tenure for the current owner: it has not watched that owner take
    /// office. Nothing is wrong with the work; a handoff signs under that tenure and must not guess
    /// it.
    ///
    /// Neither tenure reason is cleared by elapsed time or by an ordinary commit. Both are cleared
    /// by the same event: the next contiguous step that derives a fresh tenure here, which is an
    /// owner change or the committer's membership restarting on a new leaf
    /// (`owner_tenure_imported_and_unknown_both_end_at_the_next_observed_owner_change` in
    /// `catcoms-sync` pins the owner-change case). The UI must say that, and must not promise
    /// either one a cure by waiting.
    TenureUnknown,
    /// This device holds the current owner's tenure only from an imported (v1-migrated) snapshot.
    /// It still uses that value to verify receipts but cannot vouch for it, so it refuses to author
    /// under it, and the value is never promoted in place. Named apart from `TenureUnknown` because
    /// the device holds a different thing (an unverifiable value rather than nothing), not because
    /// it ends differently: see `TenureUnknown` for what clears both.
    TenureImported,
    /// A handoff of this branch was staged (Prepared) and its source no longer answers it cleanly:
    /// it holds a partial or conflicting set of the branch's signed operations, or it is not the
    /// Prepared epoch's document at all. The resolution can neither complete it nor return it to
    /// active. Permanent: no automatic path resolves it. Disposal is refused too, since a
    /// transfer hold may be an acceptance in flight; export and archive remain. Copy remains only
    /// into another document: the branch's own document is under the hold, and the app refuses a
    /// copy into a held destination.
    PreparedStuck,
}
