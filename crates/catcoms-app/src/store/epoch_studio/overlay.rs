//! No source writes accompany local acceptance. Eligibility is checked under the same
//! exclusive store/group borrow as the shared intent transaction, with no detached gap.
use super::*;
use catcoms_replication::studio::{StudioClosingOverlayBasis, StudioOverlaySave};
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
        tenure: Option<u64>,
        basis: [u8; 32],
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
            operation,
            ts,
            rng,
            budget,
            atomic_write,
            super::super::epoch_intents::sync_intent,
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
        tenure: Option<u64>,
        basis: [u8; 32],
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
            operation,
            ts,
            rng,
            budget,
            atomic_write,
            super::super::epoch_intents::sync_intent,
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
        tenure: Option<u64>,
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
            atomic_write,
            super::super::epoch_intents::sync_intent,
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

    /// Everything Flow S does under the first custody visit: S0 validation, S1 classification,
    /// the terminal S1a acknowledgement, and for new authoring S1b authorization, media admission
    /// and capture.
    ///
    /// Both callers use this. The synchronous adapter below composes it with `plan` and the commit
    /// inline; the scheduled runtime runs the same three stages with custody released around
    /// `plan`. There is deliberately no second algorithm for a scheduler to drift from.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_studio_closing_overlay_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: Option<u64>,
        basis: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
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
        if let Some(metadata) = state.handoff_metadata() {
            if let Some(outcome) = metadata
                .completed_retry(target, basis, &intent)
                .map_err(invalid)?
            {
                let scope = super::super::epoch_intents::scope_bytes(server, &logical)?;
                let (_, old) = self.read_epoch_intent_record(&scope, &logical)?;
                self.write_prepared_intents(
                    server,
                    &logical,
                    state,
                    old,
                    true,
                    rng,
                    &mut budget.storage,
                    &mut budget.intents,
                    writer,
                    sync,
                )?;
                return Ok(StudioOverlayStart::Settled(Box::new(
                    StudioOverlaySave::HandedOff(outcome),
                )));
            }
        }
        let exact = match state.overlay() {
            Some(overlay) if overlay.target() == target => {
                overlay.exact_retry(basis, &intent).map_err(invalid)?
            }
            Some(_) => return Err(invalid("overlay belongs to another channel")),
            None => false,
        };
        // Equal nonce and body from an ordinary failed Save is not accepted local draft evidence.
        // That is classification rather than authoring, so it also precedes media admission; the
        // writer keeps its own copy of this check as defence in depth.
        if !exact
            && state
                .pending()
                .any(|(id, _)| *id == intent.operation.id(&intent.author))
        {
            return Err(invalid("ordinary intent cannot become an accepted overlay"));
        }
        // Recognize accepted retries BEFORE first/append eligibility. An installed successor,
        // a later fault or Unknown tenure cannot turn a saved exact request into a new append.
        if exact {
            return self
                .write_studio_overlay_intent(
                    server,
                    &logical,
                    target,
                    device,
                    group,
                    basis,
                    None,
                    operation,
                    ts,
                    rng,
                    &mut budget.storage,
                    &mut budget.intents,
                    writer,
                    sync,
                )
                .map(|draft| {
                    StudioOverlayStart::Settled(Box::new(StudioOverlaySave::Local(draft)))
                });
        }
        // Everything from here is new authoring. Authorization comes BEFORE media admission:
        // "not previously accepted" is not the same as "authorized to author now". A request
        // carrying a basis the document has legitimately moved past is stale, and must be told so
        // without reading, promoting or holding any pixels, and without consulting the reference
        // rails. Otherwise a stale request reports a media error, or promotes a blob into the
        // durable namespace, on its way to being refused for an unrelated reason.
        let tenure_value =
            tenure.ok_or_else(|| invalid("Closing overlay needs observed owner tenure"))?;
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
        // S1b, now that this request is both unaccepted and authorized: validate and promote the
        // referenced pixels into the durable namespace, then take the job-owned hold. The intent,
        // the frame facts and the hold are minted as one value bound to this operation, target and
        // document, so no caller can hold verified frame facts without the hold that protects them
        // or pair either with a different operation. The hold is carried through the detached stage
        // and released only when the commit returns.
        let authoring = self.admit_studio_overlay_authoring(target, &logical, device, operation)?;
        self.capture_studio_overlay_save(server, group, target, device, fresh, authoring, ts)
            .map(|capture| StudioOverlayStart::Captured(Box::new(capture)))
    }

    /// The synchronous adapter: the same three stages with no detach between them. Every caller
    /// that cannot release custody, and every existing test, takes this path.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn save_studio_closing_overlay_with_io(
        &mut self,
        server: u64,
        group: &ServerGroup,
        target: StudioTarget,
        device: &MlsDevice,
        close: &CloseRecord,
        tenure: Option<u64>,
        basis: [u8; 32],
        operation: DomainOp,
        ts: u64,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStudioBudget,
        // `FnMut` so this can lend the same writer to the start visit and then to the commit.
        // A reborrow `&mut F` is itself `FnOnce`, so neither callee's bound changes and no caller
        // has to pass anything twice.
        mut writer: impl FnMut(&Path, &[u8]) -> Result<(), AppError>,
        mut sync: impl FnMut(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioOverlaySave, AppError> {
        let capture = match self.start_studio_closing_overlay_with_io(
            server,
            group,
            target,
            device,
            close,
            tenure,
            basis,
            operation,
            ts,
            rng,
            budget,
            &mut writer,
            &mut sync,
        )? {
            StudioOverlayStart::Settled(saved) => return Ok(*saved),
            StudioOverlayStart::Captured(capture) => *capture,
        };
        let plan = capture.plan()?;
        self.commit_studio_overlay_save(
            server, group, target, device, close, tenure, plan, rng, budget, writer, sync,
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
