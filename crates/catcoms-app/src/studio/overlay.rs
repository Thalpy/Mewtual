//! Explicit local core/store foundation. No actor command, native Save or replay is enabled.
use super::*;
use crate::store::EpochStudioBudget;
use catcoms_replication::studio::{
    StudioClosingOverlayBasis, StudioHandoffOutcome, StudioOverlaySave,
};
use catcoms_replication::{CloseRecord, DomainOp};

mod admission;
pub(crate) use admission::{OverlayAdmission, OverlayOwnership};

#[cfg(test)]
mod tests;

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
        let tenure = self.sync.authoring_owner_tenure_start();
        self.sync.with_registry_context(|group, device, _, rng| {
            store.handoff_studio_overlay(server, group, target, device, basis, tenure, rng, budget)
        })
    }
    /// Prepare against actual observed tenure. Request data cannot supply its own authority.
    ///
    /// **A V1 refusal site, and the one place it is right to require tenure before anything
    /// else.** Preparation mints a fresh basis and has no terminal path: there is no retry to
    /// recognise, no acknowledgement owed and no durable Prepared record to resolve, so nothing V8
    /// protects runs here, and refusing first cannot strand one. That is what distinguishes it from
    /// Save and handoff, which must classify and acknowledge before they require anything and so
    /// still read the value instead (see `save_studio_closing_overlay` below and H1).
    ///
    /// Requiring through the typed seam rather than `authoring_owner_tenure_start()` changes no
    /// acceptance - both admit `Known` alone - but it keeps `Imported` and `Unknown` apart in the
    /// refusal, because they are different situations for whoever reads it: one is fixed by
    /// observing the owner take office, the other is not fixed by waiting at all.
    ///
    /// Returns the branch the Save must name alongside the basis, both from the one fresh basis and
    /// in the one custody visit, so a caller cannot hold one without the other or compute the
    /// branch itself.
    pub fn prepare_studio_closing_overlay(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        close: &CloseRecord,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlaySaveTicket, AppError> {
        let tenure = self.require_observed_owner_tenure()?;
        self.sync.with_registry_context(|group, device, _, _| {
            let basis = store.prepare_studio_closing_overlay(
                server,
                group,
                target,
                device,
                close,
                Some(tenure),
                budget,
            )?;
            let branch =
                store.studio_overlay_request_branch(server, group, target, &basis, budget)?;
            Ok(StudioOverlaySaveTicket { basis, branch })
        })
    }
    /// Save local draft data only. Exact acceptance retry survives source/tenure changes;
    /// new writes still require the actual independently observed current tenure and source.
    ///
    /// `branch` is the one the request was prepared with - from a [`StudioOverlaySaveTicket`] for
    /// a new Save, or the original request's for a retry. The tenure is **read** here and passed
    /// down rather than required, because a retry or an acknowledgement must still succeed under
    /// `Imported` and `Unknown` (V8); S1b and S3 require it at their own points.
    #[allow(clippy::too_many_arguments)]
    pub fn save_studio_closing_overlay(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        close: &CloseRecord,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlaySave, AppError> {
        let tenure = self.observed_owner_tenure();
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
                    branch,
                    operation,
                    clock.now_ms(),
                    rng,
                    budget,
                )
            })
    }
}

/// What a Closing-overlay Save must carry back: the basis it was prepared against and the branch
/// it names.
///
/// Both are derived from one fresh basis in one custody visit. The branch comes from
/// `StudioOverlayState::request_branch_id`, never from the caller: it is the live branch when one
/// exists, and otherwise the branch the next admission would open.
pub struct StudioOverlaySaveTicket {
    pub basis: StudioClosingOverlayBasis,
    pub branch: [u8; 32],
}

impl std::fmt::Debug for StudioOverlaySaveTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlaySaveTicket { .. }")
    }
}
