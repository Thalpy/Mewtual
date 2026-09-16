//! Explicit local core/store foundation. No actor command, native Save or replay is enabled.
use super::*;
use crate::store::EpochStudioBudget;
use catcoms_replication::studio::{
    StudioClosingOverlayBasis, StudioHandoffOutcome, StudioOverlaySave,
};
use catcoms_replication::{CloseRecord, DomainOp};

mod admission;
pub(crate) use admission::{OverlayAdmission, OverlayOwnership};

impl<T: MeshTransport, R: CryptoRngCore> Server<T, R> {
    /// Explicit internal handoff. Caller owns exclusive store/runtime custody; no actor or
    /// native command schedules this batch. Live tenure comes only from this sync instance.
    pub fn handoff_studio_overlay(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        basis: [u8; 32],
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioHandoffOutcome, AppError> {
        let tenure = self.sync.observed_owner_tenure_start();
        self.sync.with_registry_context(|group, device, _, rng| {
            store.handoff_studio_overlay(server, group, target, device, basis, tenure, rng, budget)
        })
    }
    /// Prepare against actual observed tenure. Request data cannot supply its own authority.
    pub fn prepare_studio_closing_overlay(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        close: &CloseRecord,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioClosingOverlayBasis, AppError> {
        let tenure = self.sync.observed_owner_tenure_start();
        self.sync.with_registry_context(|group, device, _, _| {
            store.prepare_studio_closing_overlay(
                server, group, target, device, close, tenure, budget,
            )
        })
    }
    /// Save local draft data only. Exact acceptance retry survives source/tenure changes;
    /// new writes still require the actual independently observed current tenure and source.
    #[allow(clippy::too_many_arguments)]
    pub fn save_studio_closing_overlay(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        close: &CloseRecord,
        basis: [u8; 32],
        operation: DomainOp,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlaySave, AppError> {
        let tenure = self.sync.observed_owner_tenure_start();
        self.sync
            .with_registry_context(|group, device, clock, rng| {
                store.save_studio_closing_overlay(
                    server,
                    group,
                    target,
                    device,
                    close,
                    tenure,
                    basis,
                    operation,
                    clock.now_ms(),
                    rng,
                    budget,
                )
            })
    }
}
