//! No source writes accompany local acceptance. Eligibility is checked under the same
//! exclusive store/group borrow as the shared intent transaction, with no detached gap.
use super::overlay_capture::OverlayBranch;
use super::*;
use crate::studio::{require_owner_tenure, StudioOwnerTenure};
use catcoms_replication::studio::{
    StudioClosingOverlayBasis, StudioOverlayAdmission, StudioOverlayRequestClass,
    StudioOverlaySave, StudioOverlayState,
};
use catcoms_replication::{CloseRecord, LocalIntent};

impl ServerStore {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_studio_closing_overlay(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: StudioOwnerTenure,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlaySave, AppError> {
        self.save_studio_closing_overlay_with_io(
            server,
            group,
            target,
            device,
            close,
            tenure,
            basis,
            branch,
            operation,
            ts,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    /// Ordinary durable IO for the scheduled runtime's first custody visit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_studio_closing_overlay(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: StudioOwnerTenure,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlayStart, AppError> {
        self.start_studio_closing_overlay_with_io(
            server,
            group,
            target,
            device,
            close,
            tenure,
            basis,
            branch,
            operation,
            ts,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    /// Ordinary durable IO for the scheduled runtime's commit visit.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn commit_studio_overlay(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: StudioOwnerTenure,
        plan: StudioOverlayPlan,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<catcoms_replication::studio::StudioLocalDraft, AppError> {
        self.commit_studio_overlay_save(
            server,
            group,
            target,
            device,
            close,
            tenure,
            plan,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_studio_closing_overlay(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: Option<u64>,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioClosingOverlayBasis, AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        let tenure =
            tenure.ok_or_else(|| invalid("Closing overlay needs observed owner tenure"))?;
        let (mut source, observed, _) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("Closing overlay source is missing"));
        }
        source
            .prepare_closing_overlay(close, group, tenure)
            .map_err(invalid)
    }

    /// The branch a Save prepared against `fresh` must name.
    ///
    /// A thin read over [`StudioOverlayState::request_branch_id`], which owns the derivation; this
    /// only finds the record. It exists so a caller never computes a branch identity itself - a
    /// second copy of that derivation is the defect the namespace was built to prevent.
    ///
    /// **Not for retries.** A retry must resend the branch its original request named. After a
    /// transfer or a disposal this returns the *next* branch, because that is what a fresh Save
    /// would open, and a retry that re-asked here would name a branch it never wrote to.
    pub(crate) fn studio_overlay_request_branch(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        fresh: &StudioClosingOverlayBasis,
        budget: &mut EpochStudioBudget,
    ) -> Result<[u8; 32], AppError> {
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        self.enter_studio_budget(server, group, budget)?;
        let state = self.checked_epoch_replay_state(
            server,
            &logical,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        let metadata = state.handoff_metadata();
        if let Some(metadata) = metadata {
            if metadata.target() != target {
                return Err(invalid("overlay belongs to another channel"));
            }
        }
        StudioOverlayState::request_branch_id(metadata, fresh).map_err(invalid)
    }

    /// Everything Flow S does under the first custody visit: S0 validation, S1 classification,
    /// the terminal S1a acknowledgements, and for new authoring S1b authorization, branch
    /// admission, media admission and capture.
    ///
    /// Both callers use this. The synchronous adapter below composes it with `plan` and the commit
    /// inline; the scheduled runtime runs the same three stages with custody released around
    /// `plan`. There is deliberately no second algorithm for a scheduler to drift from.
    ///
    /// **The order is load bearing.** Every terminal acknowledgement - a transferred branch, a
    /// disposed branch, an exact retry of an accepted operation - is reached before anything
    /// requires tenure, mints a basis, reads a source or touches media (V8, AG1-001). Only work
    /// that is genuinely new authoring reaches S1b.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn start_studio_closing_overlay_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: StudioOwnerTenure,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioOverlayStart, AppError> {
        current_member(group, device)?;
        let logical = target.document(&group.group_id()).map_err(invalid)?;
        // Bound the caller's public Vec before making a LocalIntent copy.
        match target {
            StudioTarget::Index { .. } => {
                IndexOp::decode_domain(&logical, &operation, &device.device_id())
                    .map_err(invalid)?;
            }
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::decode_domain(&logical, &operation).map_err(invalid)?;
            }
        }
        self.enter_studio_budget(server, group, budget)?;
        let state = self.checked_epoch_replay_state(
            server,
            &logical,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        let intent = LocalIntent {
            author: device.device_id(),
            operation: operation.clone(),
        };
        // S1: which terminal event, if any, is this request about? Structural, against the branch
        // the request names - no basis, no tenure, no source, no media. `Unmatched` is not a
        // verdict: it means no acknowledgement is owed, and S1b decides between a new branch and
        // a stale request. A record for another channel refuses here with `EpochScope`, from the
        // classifier's own target check - the same answer `completed_retry` used to give.
        let class = match state.handoff_metadata() {
            Some(metadata) => metadata
                .classify_request(target, branch, &intent)
                .map_err(invalid)?,
            // No overlay record at all: nothing to acknowledge and nothing live to join.
            None => StudioOverlayRequestClass::Unmatched,
        };
        let joins_live = match class {
            StudioOverlayRequestClass::Transferred(outcome) => {
                self.flush_acknowledged_overlay(server, &logical, state, rng, budget, hooks)?;
                return Ok(StudioOverlayStart::Settled(Box::new(
                    StudioOverlaySave::HandedOff(outcome),
                )));
            }
            StudioOverlayRequestClass::Disposed(disposal) => {
                self.flush_acknowledged_overlay(server, &logical, state, rng, budget, hooks)?;
                return Ok(StudioOverlayStart::Settled(Box::new(
                    StudioOverlaySave::Disposed(disposal),
                )));
            }
            StudioOverlayRequestClass::Active => true,
            StudioOverlayRequestClass::Unmatched => {
                // A transferred branch's acknowledgement outlives the next admission; the
                // classifier's recognition of it does not. `classify_request` derives the
                // transferred identity from the *current* generation, so once a newer branch is
                // admitted it can no longer match, and the request lands here. `completed_retry`
                // keys the same acknowledgement on basis and operation instead, which a later
                // admission does not disturb. Without this, the first version of this wiring
                // refused such a retry - before tenure, at the pending check below, because a
                // transferred operation stays pending until a rotation retires it - which broke
                // V8 for good on an `Imported` device.
                //
                // This only ever acknowledges an exact author-and-envelope match in the retained
                // transfer manifest. It accepts nothing and opens nothing, so the namespace's
                // guarantee - no delayed request is accepted into a new branch - is untouched.
                if let Some(outcome) = match state.handoff_metadata() {
                    Some(metadata) => metadata
                        .completed_retry(target, basis, &intent)
                        .map_err(invalid)?,
                    None => None,
                } {
                    self.flush_acknowledged_overlay(server, &logical, state, rng, budget, hooks)?;
                    return Ok(StudioOverlayStart::Settled(Box::new(
                        StudioOverlaySave::HandedOff(outcome),
                    )));
                }
                false
            }
        };
        // An accepted exact retry, recognized BEFORE eligibility. An installed successor, a later
        // fault or a tenure that is not Known cannot turn a saved exact request into a new append.
        // Only a request naming the live branch can be one: an operation enters that branch only
        // through a request that named it.
        if joins_live {
            let overlay = state
                .overlay()
                .ok_or_else(|| invalid("classified Active with no live branch"))?;
            if overlay.exact_retry(basis, &intent).map_err(invalid)? {
                drop(state);
                return self
                    .write_studio_overlay_intent(
                        server,
                        &logical,
                        target,
                        device,
                        group,
                        basis,
                        operation,
                        rng,
                        &mut budget.storage,
                        &mut budget.intents,
                        hooks,
                    )
                    .map(|draft| {
                        StudioOverlayStart::Settled(Box::new(StudioOverlaySave::Local(draft)))
                    });
            }
        }
        // Equal nonce and body from an ordinary failed Save is not accepted local draft evidence.
        // That is classification rather than authoring, so it also precedes media admission; the
        // plan keeps its own copy of this check as defence in depth.
        if state
            .pending()
            .any(|(id, _)| *id == intent.operation.id(&intent.author))
        {
            return Err(invalid("ordinary intent cannot become an accepted overlay"));
        }
        // Everything from here is new authoring. Authorization comes BEFORE media admission:
        // "not previously accepted" is not the same as "authorized to author now". A request
        // carrying a basis the document has legitimately moved past is stale, and must be told so
        // without reading, promoting or holding any pixels, and without consulting the reference
        // rails. Otherwise a stale request reports a media error, or promotes a blob into the
        // durable namespace, on its way to being refused for an unrelated reason.
        //
        // S1b is a V1 site, so this is where the tenure is required, and not before: everything
        // above must stay reachable under Imported and Unknown.
        let tenure_value = require_owner_tenure(tenure)?;
        let (mut source, observed, _) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("Closing overlay source is missing"));
        }
        let fresh = source
            .prepare_closing_overlay(close, group, tenure_value)
            .map_err(invalid)?;
        if fresh.fingerprint() != basis {
            return Err(invalid("Closing overlay basis changed"));
        }
        drop(source);
        // S1b, the branch half: resolve `Unmatched` against the basis just minted. A new branch is
        // admitted only when the request names exactly the identity the next admission would mint;
        // anything else - an older generation, a skipped one, a disposed or transferred branch the
        // request no longer matches, an unrelated basis - is stale, and is refused before any media
        // work. This is the only place a branch is opened.
        let joins = if joins_live {
            // The live branch must be able to take an append on this basis, decided here and not
            // after media admission. A ticket names the live branch whenever one exists, even if
            // the Closing source has since moved, so a request can carry a fresh basis and a branch
            // opened on an older one; and a branch being handed off is Prepared. The plan's
            // `append` refuses both - `EpochScope` and `EpochClosed` - but only after S1b has
            // promoted and held this request's pixels. Same refusals, moved ahead of the media
            // work, which is the rule for anything that cannot succeed.
            let metadata = state
                .handoff_metadata()
                .ok_or_else(|| invalid("classified Active with no overlay record"))?;
            if metadata.is_prepared() {
                return Err(invalid(catcoms_replication::ReplError::EpochClosed));
            }
            if state.overlay().map(|live| live.basis()) != Some(fresh.fingerprint()) {
                return Err(invalid(catcoms_replication::ReplError::EpochScope));
            }
            OverlayBranch::Live
        } else {
            let admission = match state.handoff_metadata() {
                Some(metadata) => metadata
                    .admit_new_branch(target, branch, &fresh)
                    .map_err(invalid)?,
                None => StudioOverlayState::admit_first_branch(branch, &fresh),
            };
            match admission {
                StudioOverlayAdmission::New { .. } => OverlayBranch::Admitted(admission),
                StudioOverlayAdmission::Stale => {
                    return Err(invalid(
                        "Closing overlay request names a stale branch; prepare it again",
                    ))
                }
            }
        };
        drop(state);
        // S1b, the media half, now that this request is unaccepted, authorized and admitted to a
        // branch: validate and promote the referenced pixels into the durable namespace, then take
        // the job-owned hold. The intent, the frame facts and the hold are minted as one value
        // bound to this operation, target and document, so no caller can hold verified frame facts
        // without the hold that protects them or pair either with a different operation. The hold
        // is carried through the detached stage and released only when the commit returns.
        let authoring = self.admit_studio_overlay_authoring(target, &logical, device, operation)?;
        self.capture_studio_overlay_save(
            server, group, target, device, fresh, branch, joins, authoring, ts,
        )
        .map(|capture| StudioOverlayStart::Captured(Box::new(capture)))
    }

    /// The flush barrier a terminal acknowledgement owes before it answers: the record it read is
    /// made durable as-is, so a retry that follows an uncertain outcome is never acknowledged from
    /// state that could still be lost. Writes nothing new.
    fn flush_acknowledged_overlay(
        &mut self,
        server: u64,
        logical: &LogicalDocument,
        state: super::super::epoch_intents::EpochIntentState,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<(), AppError> {
        let scope = super::super::epoch_intents::scope_bytes(server, logical)?;
        // Physical size only. The record was authenticated and classified above; decoding it again
        // here - which the inherited `read_epoch_intent_record` did, replaying the whole branch -
        // would pay C-1's full reconstruction just to acknowledge a terminal event.
        let old = self
            .read_scoped_intent_plain(&scope)?
            .map(|record| record.physical_bytes);
        self.write_prepared_intents(
            server,
            logical,
            state,
            old,
            true,
            rng,
            &mut budget.storage,
            &mut budget.intents,
            WriteStep::new(WriteTag::Intents),
            hooks,
        )?;
        Ok(())
    }

    /// The synchronous adapter: the same three stages with no detach between them. Every caller
    /// that cannot release custody, and every existing test, takes this path.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn save_studio_closing_overlay_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: StudioOwnerTenure,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        // `FnMut` so this can lend the same writer to the start visit and then to the commit.
        // A reborrow `&mut F` is itself `FnOnce`, so neither callee's bound changes and no caller
        // has to pass anything twice.
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioOverlaySave, AppError> {
        let capture = match self.start_studio_closing_overlay_with_io(
            server, group, target, device, close, tenure, basis, branch, operation, ts, rng,
            budget, hooks,
        )? {
            StudioOverlayStart::Settled(saved) => return Ok(*saved),
            StudioOverlayStart::Captured(capture) => *capture,
        };
        let plan = capture.plan()?;
        self.commit_studio_overlay_save(
            server, group, target, device, close, tenure, plan, rng, budget, hooks,
        )
        .map(StudioOverlaySave::Local)
    }
}

/// What the first custody visit concluded. `Settled` is terminal and already durable; `Captured`
/// is new authoring whose expensive reconstruction has not happened yet.
pub(crate) enum StudioOverlayStart {
    Settled(Box<StudioOverlaySave>),
    Captured(Box<StudioOverlayCapture>),
}
