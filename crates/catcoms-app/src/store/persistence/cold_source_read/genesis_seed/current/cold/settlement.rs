//! The cold owner itself lends its SAME node/resource/scope/writer fields. No Store-visible
//! parts constructor or trait can manufacture this association. Declare only with the
//! actual independent closure, scope barrier and settled cold reader.
use super::super::settlement::{GenesisCurrentOwnerRef, GenesisCurrentOwnerRole};
use super::*;

impl<T, R> UnexposedColdV2GenesisCurrent<T, R> {
    /// Absent is the original actual cold selection, never a failed seed-present proof.
    /// current_writer_receipt requires the real Bound2 covering obligation satisfied.
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn text_owner_ref(
        &self,
    ) -> Result<GenesisCurrentOwnerRef<'_>, AppError> {
        self.scope.current_writer_receipt()?;
        if !self.head.requires_scope_covering() {
            self.seed_settlement
                .as_ref()
                .and_then(|worker| worker.saved_outcome())
                .ok_or_else(critical_namespaces::refused)?;
        }
        self.settlement_owner_ref()
    }
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn cancel_for_text(
        &self,
    ) {
        self.writer.cancel_original();
        self.scope.cancel_original();
        self.task.cancel_original();
    }
    /// Root calls only before promotion. Dispatch actual immutable seed selection first.
    pub(in crate::store::persistence::cold_source_read::genesis_seed::current) fn retry_text_seed_settlement(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        if self.head.requires_scope_covering() {
            self.scope.current_writer_receipt()?;
            self.validate_original(layout)
        } else {
            self.retry_seed_settlement(layout, rng)
        }
    }

    pub(in crate::store) fn settlement_owner_ref(
        &self,
    ) -> Result<GenesisCurrentOwnerRef<'_>, AppError> {
        if !Arc::ptr_eq(&self.head, self.scope.head()) {
            return Err(critical_namespaces::refused());
        }
        Ok(GenesisCurrentOwnerRef {
            source: self.source(),
            task: &self.task,
            role: GenesisCurrentOwnerRole::Cold {
                scope: &self.scope,
                head: &self.head,
                writer: &self.writer,
                receipt: &self.receipt,
            },
            wrapper_fee: &self.fee,
        })
    }

    /// Remove only THIS retained seed-present activation after a new actual scope covering
    /// barrier. An already-settled cold capture has no seed to adopt/remove and refuses this
    /// constructor; its normal immutable Current validation stays in the settled reader.
    pub fn retry_seed_settlement(
        &mut self,
        layout: &V2CurrentLayoutStore,
        rng: &mut impl CryptoRngCore,
    ) -> Result<(), AppError> {
        let Self {
            node,
            resources,
            head,
            task,
            scope,
            writer,
            receipt,
            fee,
            seed_settlement,
            ..
        } = self;
        if !Arc::ptr_eq(head, scope.head()) {
            return Err(critical_namespaces::refused());
        }
        let owner = GenesisCurrentOwnerRef {
            source: GenesisSourceDataRef {
                seed: node.borrow_genesis_seed_data(),
                resources,
            },
            task,
            role: GenesisCurrentOwnerRole::Cold {
                scope,
                head,
                writer,
                receipt,
            },
            wrapper_fee: fee,
        };
        super::super::settlement::retry_owned_settlement(&owner, seed_settlement, layout, rng)
    }
}
