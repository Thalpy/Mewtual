//! Explicit local core/store foundation. No actor command, native Save or replay is enabled.
use super::*;
use crate::store::{EpochStudioBudget, StudioOverlayClassification, StudioOverlayStart};
use crate::studio_exchange::provisional::ServerPreparedProvisionalStudioSeed;
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
    /// refusal, because they describe different things the device holds: nothing at all, or a
    /// value from a snapshot it cannot verify. They do **not** differ in how they end. Neither is
    /// cleared by elapsed time or by an ordinary commit; both end at the same event, the next
    /// contiguous step that derives a fresh tenure here - an owner change, or the committer's
    /// membership restarting on a new leaf (`OwnerTenure::applied`; pinned by
    /// `owner_tenure_imported_and_unknown_both_end_at_the_next_observed_owner_change`). An earlier
    /// version of this comment said waiting fixes one and not the other, which was false.
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

    /// Prepare a Save against the complete current awaiting-tenure preview. This grants no source
    /// or signing authority: the store proves the installed source is absent before the sync layer
    /// mints the Unconfirmed basis, and proves absence again before deriving the branch.
    ///
    /// Kept crate-private while P5 is false. Native registration must not make an unfinished Save
    /// path reachable merely because its backend preparation seam exists.
    #[allow(dead_code)]
    pub(crate) fn prepare_studio_unconfirmed_overlay(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        preview: &ServerPreparedProvisionalStudioSeed,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioUnconfirmedOverlaySaveTicket, AppError> {
        self.sync.with_registry_context(|group, device, _, _| {
            store.require_studio_source_absent(server, group, target, device, budget)
        })?;
        let basis = self.mint_unconfirmed_overlay_basis(store, server, target, preview)?;
        let branch = self.sync.with_registry_context(|group, device, _, _| {
            store.require_studio_source_absent(server, group, target, device, budget)?;
            store.studio_overlay_request_branch(server, group, target, &basis, budget)
        })?;
        Ok(StudioUnconfirmedOverlaySaveTicket {
            basis: basis.fingerprint(),
            branch,
        })
    }

    /// S0/S1/S1b for an awaiting-tenure Save. Classification runs before preview/source checks so
    /// terminal acknowledgements and exact retries retain V8 behavior. New authoring alone proves
    /// source absence, re-mints the current preview basis, and captures the detached S2 job.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_studio_unconfirmed_overlay(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        preview: &ServerPreparedProvisionalStudioSeed,
        basis: [u8; 32],
        branch: [u8; 32],
        operation: DomainOp,
        budget: &mut EpochStudioBudget,
    ) -> Result<StudioOverlayStart, AppError> {
        let classified = self
            .sync
            .with_registry_context(|group, device, clock, rng| {
                let classified = store.classify_studio_overlay_save(
                    server,
                    group,
                    target,
                    device,
                    basis,
                    branch,
                    operation,
                    clock.now_ms(),
                    rng,
                    budget,
                )?;
                if matches!(classified, StudioOverlayClassification::Authoring(_)) {
                    store.require_studio_source_absent(server, group, target, device, budget)?;
                }
                Ok::<_, AppError>(classified)
            })?;
        let request = match classified {
            StudioOverlayClassification::Settled(saved) => {
                return Ok(StudioOverlayStart::Settled(saved))
            }
            StudioOverlayClassification::Authoring(request) => *request,
        };
        let fresh = self.mint_unconfirmed_overlay_basis(store, server, target, preview)?;
        self.sync.with_registry_context(|group, device, _, _| {
            store
                .authorize_studio_overlay_save(server, group, device, request, fresh.into(), budget)
                .map(|capture| StudioOverlayStart::Captured(Box::new(capture)))
        })
    }

    /// S3 for an awaiting-tenure Save. Source absence is proved before re-entering the current
    /// preview and again inside the final store visit. No async boundary exists between the mint
    /// and that visit; the store still reauthenticates the detached plan and intent bytes.
    pub(crate) fn commit_studio_unconfirmed_overlay(
        &mut self,
        store: &mut ServerStore,
        server: u64,
        target: StudioTarget,
        preview: &ServerPreparedProvisionalStudioSeed,
        plan: crate::store::StudioOverlayPlan,
        budget: &mut EpochStudioBudget,
    ) -> Result<catcoms_replication::studio::StudioLocalDraft, AppError> {
        self.sync.with_registry_context(|group, device, _, _| {
            store.require_studio_source_absent(server, group, target, device, budget)
        })?;
        let fresh = self.mint_unconfirmed_overlay_basis(store, server, target, preview)?;
        self.sync.with_registry_context(|group, device, _, rng| {
            store.commit_studio_unconfirmed_overlay_save(
                server, group, target, device, fresh, plan, rng, budget,
            )
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

/// The public request identity prepared from an awaiting-tenure preview. The private basis does
/// not escape the preparation visit; S1b and S3 each re-mint it from the still-current preview.
#[allow(dead_code)]
pub(crate) struct StudioUnconfirmedOverlaySaveTicket {
    pub(crate) basis: [u8; 32],
    pub(crate) branch: [u8; 32],
}

impl std::fmt::Debug for StudioUnconfirmedOverlaySaveTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioUnconfirmedOverlaySaveTicket { .. }")
    }
}

impl std::fmt::Debug for StudioOverlaySaveTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("StudioOverlaySaveTicket { .. }")
    }
}
