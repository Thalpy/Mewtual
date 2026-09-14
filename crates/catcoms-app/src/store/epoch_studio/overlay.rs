//! No source writes accompany local acceptance. Eligibility is checked under the same
//! exclusive store/group borrow as the shared intent transaction, with no detached gap.
use super::*;
use catcoms_replication::studio::{StudioClosingOverlayBasis, StudioLocalDraft};
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
    ) -> Result<StudioLocalDraft, AppError> {
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
        writer: impl FnOnce(&Path, &[u8]) -> Result<(), AppError>,
        sync: impl FnOnce(&Path, u64) -> Result<(), AppError>,
    ) -> Result<StudioLocalDraft, AppError> {
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
        let exact = match state.overlay() {
            Some(overlay) if overlay.target() == target => {
                overlay.exact_retry(basis, &intent).map_err(invalid)?
            }
            Some(_) => return Err(invalid("overlay belongs to another channel")),
            None => false,
        };
        // Recognize accepted retries BEFORE first/append eligibility. An installed successor,
        // a later fault or Unknown tenure cannot turn a saved exact request into a new append.
        let fresh = if exact {
            None
        } else {
            let tenure =
                tenure.ok_or_else(|| invalid("Closing overlay needs observed owner tenure"))?;
            let (mut source, observed, _) = self.checked_studio_source(
                server,
                group,
                target,
                device,
                false,
                &mut budget.storage,
            )?;
            if observed.is_none() {
                return Err(invalid("Closing overlay source is missing"));
            }
            let fresh = source
                .prepare_closing_overlay(close, group, tenure)
                .map_err(invalid)?;
            if fresh.fingerprint() != basis {
                return Err(invalid("Closing overlay basis changed"));
            }
            Some(fresh)
        };
        self.write_studio_overlay_intent(
            server,
            &logical,
            target,
            device,
            group,
            basis,
            fresh.as_ref(),
            operation,
            ts,
            rng,
            &mut budget.storage,
            &mut budget.intents,
            writer,
            sync,
        )
    }
}
