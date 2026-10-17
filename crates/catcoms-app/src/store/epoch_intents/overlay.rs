//! Overlay entries share the ordinary writer, budgets and exact-retry flush barrier.
use super::*;
use catcoms_replication::studio::{StudioOverlaySave, StudioTarget};

impl ServerStore {
    /// The exact-retry flush barrier for an accepted overlay operation, and nothing else.
    ///
    /// Design 6.2's S1a: terminal and flush-only. The acknowledgement carries the stored branch's
    /// basis and accepted count, both structural, and **the branch is never reconstructed here**.
    /// An earlier version returned `overlay.read(..)`, which loads the seed graph and replays every
    /// accepted entry, so a retry cost the branch's whole depth under custody, and every retry of
    /// an Unconfirmed Save paid it (design 18.3 review, F1). No caller used the projection.
    /// `studio_overlay_store_exact_retry_rebuilds_no_draft` pins this, counting reconstructions in
    /// the replication crate, so a full decode or a direct `StudioOverlay::read` here is caught too.
    ///
    /// This used to be the new-authoring writer as well, minting a branch with
    /// `unwrap_or_else(StudioOverlayState::new)` and `append`. New authoring moved to the staged
    /// capture/plan/commit long ago and its only remaining caller passed `basis: None`, so that
    /// tail could no longer be reached - but it was still a second place that could open a branch
    /// without going through the branch-generation admission. It is removed rather than left dead:
    /// a branch is now opened only where S1b has admitted it.
    ///
    /// The I-3 protection transfer that tail carried lives on in `commit_studio_overlay_save`,
    /// which is the path that actually accepts new work, and the regressions written for it drive
    /// that path.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::store) fn write_studio_overlay_intent(
        &mut self,
        server: u64,
        document: &LogicalDocument,
        target: StudioTarget,
        device: &MlsDevice,
        group: &ServerGroup,
        expected: [u8; 32],
        operation: DomainOp,
        rng: &mut impl CryptoRngCore,
        budget: &mut EpochStorageBudget,
        intents: &mut EpochIntentBudget,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<StudioOverlaySave, AppError> {
        if document.server_id != group.group_id()
            || group.member_signature_key(&device.device_id()).as_deref()
                != Some(device.public_key_bytes().as_slice())
        {
            return Err(invalid("intent author is not a current local member"));
        }
        let scope = scope_bytes(server, document)?;
        let state = self.checked_epoch_replay_state(server, document, budget, intents)?;
        // Physical size only; the record was authenticated above.
        let old = self
            .read_scoped_intent_plain(&scope)?
            .map(|record| record.physical_bytes);
        let intent = LocalIntent {
            author: device.device_id(),
            operation,
        };
        if let Some(metadata) = &state.overlay {
            if metadata.target() != target {
                return Err(invalid("overlay belongs to another channel"));
            }
        }
        let Some(overlay) = state.overlay() else {
            return Err(invalid("no accepted overlay to retry against"));
        };
        if !overlay.exact_retry(expected, &intent).map_err(invalid)? {
            return Err(invalid(
                "not an exact retry of an accepted overlay operation",
            ));
        }
        let acknowledged = StudioOverlaySave::Acknowledged {
            basis: overlay.basis(),
            accepted: overlay.accepted(),
        };
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
        Ok(acknowledged)
    }
}
