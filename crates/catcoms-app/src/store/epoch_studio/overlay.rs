//! No source writes accompany local acceptance. Eligibility is checked under the same
//! exclusive store/group borrow as the shared intent transaction, with no detached gap.
use super::overlay_capture::{CapturedStudioOverlayBasis, OverlayBranch};
use super::*;
use crate::studio::{require_owner_tenure, StudioOwnerTenure};
use catcoms_replication::studio::{
    StudioClosingOverlayBasis, StudioOverlayAdmission, StudioOverlayBasis,
    StudioOverlayRequestClass, StudioOverlaySave, StudioOverlayState,
};
use catcoms_replication::{CloseRecord, LocalIntent};

/// Result of Flow S's basis-independent classification visit. Terminal acknowledgements are
/// already durable. New authoring carries the authenticated record and the exact public request
/// forward to S1b, but no basis has been minted and no media has been touched yet.
pub(crate) enum StudioOverlayClassification {
    Settled(Box<StudioOverlaySave>),
    Authoring(Box<StudioOverlayAuthoringRequest>),
}

/// Basis-independent state captured by S1. This value never leaves store custody: a caller must
/// mint the provenance-specific basis from current authority, then immediately return it to
/// `authorize_studio_overlay_save` for branch and media admission.
pub(crate) struct StudioOverlayAuthoringRequest {
    document: LogicalDocument,
    target: StudioTarget,
    basis: [u8; 32],
    branch: [u8; 32],
    operation: DomainOp,
    ts: u64,
    state: super::super::epoch_intents::EpochIntentState,
    joins_live: bool,
}

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

    /// Prove that this target has no installed source against the current five-family inventory.
    /// The zero-body probe refuses before allocating an existing record's body; absence is then
    /// matched to the inventory entry. Unconfirmed authoring calls this before every preview mint
    /// and again immediately before its first durable acceptance.
    pub(crate) fn require_studio_source_absent(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        budget: &mut EpochStudioBudget,
    ) -> Result<(), AppError> {
        current_member(group, device)?;
        self.enter_studio_budget(server, group, budget)?;
        self.check_studio_source_absent(server, group, target, &mut budget.storage)
    }

    pub(super) fn check_studio_source_absent(
        &self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        budget: &mut EpochStorageBudget,
    ) -> Result<(), AppError> {
        let document = target.document(&group.group_id()).map_err(invalid)?;
        let scope = scope_bytes(server, &document)?;
        let storage_scope = StorageScope::new(server, &document.server_id).map_err(invalid)?;
        let result = (|| {
            self.read_studio_record_bounded(&scope, 0)?;
            budget
                .verify_record(&storage_scope, *blake3::hash(&scope).as_bytes(), None)
                .map_err(invalid)
        })();
        result.inspect_err(|_| budget.invalidate())
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
    pub(crate) fn studio_overlay_request_branch<'a>(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        fresh: impl Into<StudioOverlayBasis<'a>>,
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

    /// S0 and S1 of Flow S, shared by every provenance. No source, tenure, preview or media is
    /// consulted here. That ordering keeps terminal acknowledgements and exact retries available
    /// even when the authority needed for new authoring has disappeared or changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn classify_studio_overlay_save(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlayClassification, AppError> {
        self.classify_studio_overlay_save_with_io(
            server,
            group,
            target,
            device,
            basis,
            branch,
            operation,
            ts,
            rng,
            budget,
            &mut WriteHooks::None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn classify_studio_overlay_save_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioOverlayClassification, AppError> {
        current_member(group, device)?;
        let document = target.document(&group.group_id()).map_err(invalid)?;
        // Bound the caller's public Vec before making a LocalIntent copy.
        match target {
            StudioTarget::Index { .. } => {
                IndexOp::decode_domain(&document, &operation, &device.device_id())
                    .map_err(invalid)?;
            }
            StudioTarget::Flipnote { .. } => {
                FlipnoteOp::decode_domain(&document, &operation).map_err(invalid)?;
            }
        }
        self.enter_studio_budget(server, group, budget)?;
        let state = self.checked_epoch_replay_state(
            server,
            &document,
            &mut budget.storage,
            &mut budget.intents,
        )?;
        let intent = LocalIntent {
            author: device.device_id(),
            operation: operation.clone(),
        };
        let class = match state.handoff_metadata() {
            Some(metadata) => metadata
                .classify_request(target, branch, &intent)
                .map_err(invalid)?,
            None => StudioOverlayRequestClass::Unmatched,
        };
        let joins_live = match class {
            StudioOverlayRequestClass::Transferred(outcome) => {
                self.flush_acknowledged_overlay(server, &document, state, rng, budget, hooks)?;
                return Ok(StudioOverlayClassification::Settled(Box::new(
                    StudioOverlaySave::HandedOff(outcome),
                )));
            }
            StudioOverlayRequestClass::Disposed(disposal) => {
                self.flush_acknowledged_overlay(server, &document, state, rng, budget, hooks)?;
                return Ok(StudioOverlayClassification::Settled(Box::new(
                    StudioOverlaySave::Disposed(disposal),
                )));
            }
            StudioOverlayRequestClass::Active => true,
            StudioOverlayRequestClass::Unmatched => {
                // A terminal transfer retry is keyed on the retained basis and full envelope, so
                // it remains recognizable after a newer branch advances the generation.
                if let Some(outcome) = match state.handoff_metadata() {
                    Some(metadata) => metadata
                        .completed_retry(target, basis, &intent)
                        .map_err(invalid)?,
                    None => None,
                } {
                    self.flush_acknowledged_overlay(server, &document, state, rng, budget, hooks)?;
                    return Ok(StudioOverlayClassification::Settled(Box::new(
                        StudioOverlaySave::HandedOff(outcome),
                    )));
                }
                false
            }
        };
        // Accepted exact retries remain available before provenance-specific authority checks.
        if joins_live {
            let overlay = state
                .overlay()
                .ok_or_else(|| invalid("classified Active with no live branch"))?;
            if overlay.exact_retry(basis, &intent).map_err(invalid)? {
                drop(state);
                let draft = self.write_studio_overlay_intent(
                    server,
                    &document,
                    target,
                    device,
                    group,
                    basis,
                    operation,
                    rng,
                    &mut budget.storage,
                    &mut budget.intents,
                    hooks,
                )?;
                return Ok(StudioOverlayClassification::Settled(Box::new(
                    StudioOverlaySave::Local(draft),
                )));
            }
        }
        if state
            .pending()
            .any(|(id, _)| *id == intent.operation.id(&intent.author))
        {
            return Err(invalid("ordinary intent cannot become an accepted overlay"));
        }
        Ok(StudioOverlayClassification::Authoring(Box::new(
            StudioOverlayAuthoringRequest {
                document,
                target,
                basis,
                branch,
                operation,
                ts,
                state,
                joins_live,
            },
        )))
    }

    /// S1b after a provenance-specific caller has minted a fresh basis under current authority.
    /// Branch admission precedes media work, and the typed basis is carried intact into S2.
    pub(crate) fn authorize_studio_overlay_save(
        &self,
        server: u64,
        group: &ServerGroup,
        device: &MlsDevice,
        request: StudioOverlayAuthoringRequest,
        fresh: CapturedStudioOverlayBasis,
        budget: &EpochStudioBudget,
    ) -> Result<StudioOverlayCapture, AppError> {
        let StudioOverlayAuthoringRequest {
            document,
            target,
            basis,
            branch,
            operation,
            ts,
            state,
            joins_live,
        } = request;
        if fresh.fingerprint() != basis {
            return Err(invalid(match fresh {
                CapturedStudioOverlayBasis::Closing(_) => "Closing overlay basis changed",
                CapturedStudioOverlayBasis::Unconfirmed(_) => {
                    "Unconfirmed overlay basis changed; refresh the preview"
                }
            }));
        }
        let joins =
            if joins_live {
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
                        .admit_new_branch(target, branch, fresh.borrowed())
                        .map_err(invalid)?,
                    None => StudioOverlayState::admit_first_branch(branch, fresh.borrowed()),
                };
                match admission {
                    StudioOverlayAdmission::New { .. } => OverlayBranch::Admitted(admission),
                    StudioOverlayAdmission::Stale => return Err(invalid(match fresh {
                        CapturedStudioOverlayBasis::Closing(_) => {
                            "Closing overlay request names a stale branch; prepare it again"
                        }
                        CapturedStudioOverlayBasis::Unconfirmed(_) => {
                            "Unconfirmed overlay request names a stale branch; refresh the preview"
                        }
                    })),
                }
            };
        if matches!(&fresh, CapturedStudioOverlayBasis::Unconfirmed(_)) {
            match &joins {
                OverlayBranch::Live
                    if state
                        .overlay()
                        .map(|overlay| overlay.accepted())
                        .unwrap_or(0)
                        >= super::overlay_capture::MAX_UNCONFIRMED_OVERLAY_OPS =>
                {
                    return Err(invalid(
                        "Unconfirmed draft operation limit reached (64 accepted operations)",
                    ));
                }
                OverlayBranch::Admitted(_) => budget.preflight_unconfirmed_branch_count()?,
                OverlayBranch::Live => {}
            }
        }
        drop(state);
        let authoring =
            self.admit_studio_overlay_authoring(target, &document, device, operation)?;
        self.capture_studio_overlay_save(
            server, group, target, device, fresh, branch, joins, authoring, ts,
        )
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
        let request = match self.classify_studio_overlay_save_with_io(
            server, group, target, device, basis, branch, operation, ts, rng, budget, hooks,
        )? {
            StudioOverlayClassification::Settled(saved) => {
                return Ok(StudioOverlayStart::Settled(saved))
            }
            StudioOverlayClassification::Authoring(request) => *request,
        };
        // Closing S1b is a V1 site. It is intentionally after the shared classification above,
        // so terminal acknowledgements remain reachable under Imported and Unknown tenure.
        let tenure_value = require_owner_tenure(tenure)?;
        let (mut source, observed, _) =
            self.checked_studio_source(server, group, target, device, false, &mut budget.storage)?;
        if observed.is_none() {
            return Err(invalid("Closing overlay source is missing"));
        }
        let fresh = source
            .prepare_closing_overlay(close, group, tenure_value)
            .map_err(invalid)?;
        drop(source);
        self.authorize_studio_overlay_save(server, group, device, request, fresh.into(), budget)
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
