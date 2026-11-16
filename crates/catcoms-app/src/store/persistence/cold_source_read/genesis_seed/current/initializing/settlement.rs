//! Repaired SAME-node cleanup loan. Its only constructor borrows the actual owning
//! output; no repaired?Warm/Cold conversion or independently adoptable parts are exposed.
use super::super::settlement::{GenesisCurrentOwnerRef, GenesisCurrentOwnerRole};
use super::*;

fn require_repaired_head(
    repair_seed: &Arc<ColdV2GenesisRepairSeed>,
    scope: &ColdV2InitializingPreparation,
    writer: &PreparedColdV2InitializingWriterRegistration,
    repair: &PreparedColdV2GenesisRepair,
) -> Result<(), AppError> {
    let head = scope
        .saved_head()
        .ok_or_else(critical_namespaces::refused)?;
    if !Arc::ptr_eq(repair_seed, scope.seed()) || !Arc::ptr_eq(head.seed(), repair_seed) {
        return Err(critical_namespaces::refused());
    }
    repair.require_same_saved_head(head)?;
    scope.current_binding()?;
    writer.require_same_observation(head.writer_observation())
}

impl<T, R> UnexposedRepairedV2GenesisCurrent<T, R> {
    /// Cached prerequisite only; new operational physical proof is separate. An ambiguous
    /// cleanup must first finish its SAME worker, not adopt file absence.
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn text_owner_ref(
        &self,
    ) -> Result<GenesisCurrentOwnerRef<'_>, AppError> {
        self.seed_settlement
            .as_ref()
            .and_then(|worker| worker.saved_outcome())
            .ok_or_else(critical_namespaces::refused)?;
        self.settlement_owner_ref()
    }
    /// Negative only; explicit close joins real work separately from this revocation.
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn cancel_for_text(
        &self,
    ) {
        self.writer.cancel_original();
        self.scope.cancel_original();
        self.task.cancel_original();
    }

    /// Cached exact association only. In particular no old mandatory-seed physical
    /// validator occurs here: this loan is also needed after an admitted ambiguous unlink.
    pub(in crate::store) fn settlement_owner_ref(
        &self,
    ) -> Result<GenesisCurrentOwnerRef<'_>, AppError> {
        require_repaired_head(&self.repair_seed, &self.scope, &self.writer, &self.repair)?;
        let head = self
            .scope
            .saved_head()
            .ok_or_else(critical_namespaces::refused)?;
        Ok(GenesisCurrentOwnerRef {
            source: self.source(),
            task: &self.task,
            role: GenesisCurrentOwnerRole::Repaired {
                scope: &self.scope,
                head,
                writer: &self.writer,
            },
            wrapper_fee: &self.fee,
        })
    }

    /// Settle THIS original activation seed with the same actual node, scope2 owner,
    /// nonce and installed ColdInitializing writer. A refusal retains all owners and the
    /// exact cleanup phase. No source.seed/private object, history or SDK is changed.
    pub fn retry_seed_settlement(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        // Split the mutable phase slot from actual original owners. There is no graph
        // Option hole, cloning/recreated RNG/transport, or permission callback adapter.
        let Self {
            node,
            resources,
            repair_seed,
            task,
            scope,
            writer,
            repair,
            seed_settlement,
            fee,
        } = self;
        require_repaired_head(repair_seed, scope, writer, repair)?;
        let head = scope
            .saved_head()
            .ok_or_else(critical_namespaces::refused)?;
        let owner = GenesisCurrentOwnerRef {
            source: GenesisSourceDataRef {
                seed: node.borrow_genesis_seed_data(),
                resources,
            },
            task,
            role: GenesisCurrentOwnerRole::Repaired {
                scope,
                head,
                writer,
            },
            wrapper_fee: fee,
        };
        super::super::settlement::retry_owned_settlement(&owner, seed_settlement, layout, rng)
    }
}
