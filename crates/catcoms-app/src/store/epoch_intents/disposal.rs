//! The disposal transaction: the one path that drops an accepted local branch without transferring
//! it, in **one** accounted atomic replacement of the intent record.
//!
//! The terminal manifest is written in the same sealed plaintext that removes the branch's annotated
//! entries. That is not an optimisation, it is the correctness property: a vault must never hold a
//! branch whose entries are gone, nor entries whose branch has no acknowledgement. One replacement
//! means neither half can land without the other.
//!
//! What this transaction deliberately does NOT do: retire an ordinary intent, prune a source, delete
//! a blob, write a recovery record, or - for `Preserved` - write an archive of its own. The archive
//! is already durable before this runs, which is what makes a crash between the two safe in the only
//! direction that matters: evidence first, removal second.
use catcoms_mls::{MlsDevice, ServerGroup};
use catcoms_replication::studio::{
    StudioDiscardConfirmation, StudioDisposalDecision, StudioDisposalMode, StudioOverlayDisposal,
    StudioTarget,
};
use catcoms_replication::LogicalDocument;

use super::super::epoch_budget::EpochStorageBudget;
use super::super::{ServerStore, WriteHooks, WriteStep, WriteTag};
use super::{invalid, EpochIntentBudget};
use crate::AppError;
use catcoms_rt::CryptoRngCore;

/// What the user asked for, carrying the evidence each mode requires.
///
/// `branch`, `content` and `accepted` are all from the inspection the user actually saw. They are
/// three separate checks rather than one because they fail for different reasons and a caller
/// deserves to know which: a wrong `branch` means the request names another generation, a wrong
/// `content` means the branch changed under the dialog, and a wrong `accepted` means the caller and
/// the vault disagree about size even though the hashes matched, which is a bug rather than a race.
pub(in crate::store) struct StudioOverlayDisposalRequest {
    /// Branch identity including its generation.
    pub(in crate::store) branch: [u8; 32],
    /// `branch_content` from the inspection the user saw.
    pub(in crate::store) content: [u8; 32],
    pub(in crate::store) accepted: usize,
    pub(in crate::store) mode: StudioDisposalRequestMode,
}

/// The mode, with D5's confirmation as a required typed field on the destructive arm.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "constructed by the native dispose command, which lands later in this scope; its \
    own tests construct both arms today"
    )
)]
pub(in crate::store) enum StudioDisposalRequestMode {
    Preserve,
    Discard(StudioDiscardConfirmation),
}

impl ServerStore {
    /// Dispose of an accepted local branch. D1 to D6, then one replacement.
    ///
    /// Every precondition is checked before any write, so a refusal costs nothing and leaves the
    /// branch, its ledger entries and any existing archive exactly as they were.
    #[allow(clippy::too_many_arguments)]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the native dispose command is this transaction's production caller and lands \
        later in this scope; its own tests exercise it today"
        )
    )]
    pub(in crate::store) fn dispose_studio_overlay_with_io(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        target: StudioTarget,
        group: &ServerGroup,
        device: &MlsDevice,
        request: StudioOverlayDisposalRequest,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioOverlayDisposal, AppError> {
        // D1, part one: current local membership, and the same check every intent write makes. A
        // device that is not a current member of this document's group cannot dispose of work in it.
        if document.server_id != group.group_id()
            || group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("disposal author is not a current local member"));
        }
        // D1, part two: the complete target and channel scope. The document is derived from the
        // target rather than trusted alongside it, so a request cannot name one channel's target and
        // another channel's document.
        if target.document(&group.group_id()).map_err(invalid)? != *document {
            return Err(invalid("disposal target does not name this document"));
        }

        let scope = super::scope_bytes(server, document)?;
        let state = self.checked_epoch_replay_state(server, document, budget, intents)?;
        let metadata = state
            .handoff_metadata()
            .ok_or_else(|| invalid("no local draft branch exists for this document"))?;
        metadata.check_target(target).map_err(invalid)?;

        // Design section 12, row 6: **the exact retry of a disposal that already landed.**
        //
        // A disposal can return an error after its rename has landed - a failed directory sync, for
        // instance - so a caller that reconciles and resends is doing the right thing. Without this
        // arm the resend meets "no local draft branch exists", which is the answer a vault that never
        // had a branch gives, and the two states collapse: the caller can never learn whether its
        // disposal succeeded, and the flush that would confirm durability never happens.
        //
        // Unlike release, disposal HAS a tombstone - the retained manifest - so it can recognise
        // itself, which is why section 12.1's exemption covers release and not this. The identity is
        // the manifest's own `branch` and `content`, and the mode must match too: a request to
        // discard is not satisfied by a preserving disposal that already happened, and reporting
        // otherwise would tell the user their bodies were destroyed when they were archived.
        if state.overlay().is_none() {
            let existing = metadata
                .disposed()
                .filter(|d| d.branch == request.branch && d.content == request.content)
                .filter(|d| {
                    matches!(
                        (&d.mode, &request.mode),
                        (
                            StudioDisposalMode::Discarded,
                            StudioDisposalRequestMode::Discard(_)
                        ) | (
                            StudioDisposalMode::Preserved { .. },
                            StudioDisposalRequestMode::Preserve
                        )
                    )
                })
                .ok_or_else(|| invalid("no local draft branch exists for this document"))?
                .clone();
            // Sync-only: the record is already exactly right, so this confirms its durability
            // without a second replacement and without needing replacement headroom.
            let old = self
                .read_scoped_intent_plain(&scope)?
                .map(|record| record.physical_bytes);
            self.write_prepared_intents(
                server,
                document,
                state,
                old,
                true,
                rng,
                budget,
                intents,
                WriteStep::new(WriteTag::Intents),
                hooks,
            )?;
            return Ok(existing);
        }
        let active = state
            .overlay()
            .ok_or_else(|| invalid("no local draft branch exists for this document"))?;

        // D1, part three: the requester must be the branch's own author. Membership alone is not
        // enough - another member of the same group has no standing over this device's local draft.
        if active.author() != device.device_id() {
            return Err(invalid("only the branch's author may dispose of it"));
        }

        // D2: no transfer hold. A Prepared branch is live evidence that someone else may be about to
        // accept this work, and dropping it here would race that acceptance. `dispose` refuses this
        // too; refusing here first means the error says what is wrong rather than reporting a
        // generic conflict from deeper in.
        //
        // D2's other half, "no live hold", is **not** checked here and cannot be: a live editing hold
        // is custody-layer state that this function does not see. A reader working down the D-table
        // will look for it here, so: the custody coordinator owns it, and this transaction runs
        // inside a visit that already holds custody exclusively.
        if state.handoff_prepared() {
            return Err(invalid(
                "a branch under a transfer hold cannot be disposed of; resolve the handoff first",
            ));
        }

        // D3: the request must name this exact branch, as the user saw it. All three halves.
        if metadata.branch_id() != Some(request.branch) {
            return Err(invalid(
                "disposal names another branch generation; re-inspect before disposing",
            ));
        }
        if metadata.branch_content(&state.ledger).map_err(invalid)? != request.content {
            return Err(invalid(
                "the branch changed since it was inspected; re-inspect before disposing",
            ));
        }
        if active.accepted() != request.accepted {
            return Err(invalid(
                "disposal disagrees with the branch's accepted count; re-inspect before disposing",
            ));
        }

        // D4 and D5: the evidence each mode requires.
        let decision = match request.mode {
            // D5. The confirmation is **moved** out of the request and into the decision, not
            // re-minted from the constant.
            //
            // An earlier version called `StudioDiscardConfirmation::parse(TOKEN)` here and discarded
            // the one the caller supplied. A review pointed out that this compares the constant to
            // itself: the refusal was unreachable and the comment claiming it checked for
            // substitution was false. Worse, because the confirmation was never consumed, one could
            // back any number of disposals - and the type's own contract is that a confirmation
            // reaching a second transaction "would confirm something the user never saw".
            //
            // Moving it makes the type do the work: the request is taken by value, so a caller cannot
            // reuse a confirmation, and this arm cannot be entered without one.
            StudioDisposalRequestMode::Discard(confirmation) => {
                StudioDisposalDecision::Discard(confirmation)
            }
            // D4. A durable archive for THIS EXACT branch must already exist.
            //
            // `content` is compared against `request.content`, which is the live branch's value
            // *transitively*, because D3 above has already proved the two equal. An earlier comment
            // here claimed every field was compared against the live branch directly, which a review
            // correctly called out as misdescribing the code. `branch` and `generation` are compared
            // against the live metadata, and they are mutually redundant - `branch_id` is
            // `H(basis, generation)` - but kept: the generation compare is the one that refuses an
            // archive of a *previous generation* whose entries and content happen to be identical,
            // which `content` alone cannot catch because `branch_hash` does not cover the generation.
            StudioDisposalRequestMode::Preserve => {
                let record = self
                    .read_studio_draft_archive(server, document)?
                    .ok_or_else(|| {
                        invalid(
                            "a preserving disposal needs a durable draft archive for this branch; \
                             none exists",
                        )
                    })?;
                let archive = &record.archive;
                if archive.content() != request.content
                    || Some(archive.branch()) != metadata.branch_id()
                    || archive.generation() != metadata.branch_generation()
                {
                    return Err(invalid(
                        "the preserved archive is for a different branch than the one being \
                         disposed of",
                    ));
                }
                // The full-envelope match. A content hash the archive merely claims is not evidence
                // that the archive contains this branch's work.
                if !archive
                    .matches_branch(active, &state.ledger)
                    .map_err(invalid)?
                {
                    return Err(invalid(
                        "the preserved archive's entries are not this branch's entries",
                    ));
                }
                StudioDisposalDecision::Preserve { archive: record.id }
            }
        };

        // The rebuild. `dispose` derives the manifest's identity from the state, validates the
        // result and hands back the exact ids to retire, read once rather than recomputed here.
        let (next_overlay, removed) = metadata
            .dispose(&state.ledger, decision, request.content, ts, ts)
            .map_err(invalid)?;
        let manifest = next_overlay
            .disposed()
            .ok_or_else(|| invalid("rebuilt state carries no disposal"))?
            .clone();

        let mut next = state;
        let retired = next.ledger.remove_disposed(&removed);
        if retired != removed.len() {
            // The ledger did not hold every id the branch claimed. Refusing is the only safe answer:
            // the manifest would acknowledge entries that were never there, and the disagreement
            // means this vault is not in the state the branch describes.
            return Err(invalid(
                "the ledger did not hold every entry this branch claimed",
            ));
        }
        next.overlay = Some(next_overlay);

        // D6 and the write. The physical size only; the record was authenticated by the replay state
        // above. `write_prepared_intents` performs the preflight, the reservation, the I-4 rotation
        // and the atomic replacement, so the manifest and the removal land in one sealed plaintext.
        let old = self
            .read_scoped_intent_plain(&scope)?
            .map(|record| record.physical_bytes);
        self.write_prepared_intents(
            server,
            document,
            next,
            old,
            false,
            rng,
            budget,
            intents,
            WriteStep::new(WriteTag::Intents),
            hooks,
        )?;
        Ok(manifest)
    }
}
