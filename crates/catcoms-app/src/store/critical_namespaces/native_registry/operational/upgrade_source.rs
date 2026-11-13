//! Consuming ownership boundary for the version2 -> version3 upgrade.
//!
//! This proves only exclusive physical custody. Authentication, independent
//! version3 payment, catalog/Stage checks and the original gate claim are still
//! required at the actual upgrade admission. In particular, `idle` is never a
//! saved permission to write or enter an SDK.
use super::super::lifecycle::preparation::{Output, Policy};
use super::*;
use crate::store::persistence::native_registry::lifecycle::{
    ExpectedCurrentInputs, UpgradeSnapshot,
};
use catcoms_rt::CryptoRngCore;
use zeroize::Zeroizing;

/// Preserve the WHOLE original mount through non-writing preparation. Returning
/// its inner Current, rebuilding its Arc or dropping its admission owner here
/// would either lose exact identity or close the original JOIN fence too early.
pub(in crate::store::critical_namespaces::native_registry) struct UpgradeSource {
    original: NativeIndexStore,
}

impl std::fmt::Debug for UpgradeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpgradeSource").finish_non_exhaustive()
    }
}

impl NativeIndexStore {
    /// Refusal has no effects and returns the very same opaque owner. Never use
    /// separate strong/weak counts: a worker's final strong-to-weak transition
    /// can race those snapshots. `get_mut` proves both conditions together.
    pub(in crate::store::critical_namespaces::native_registry) fn into_upgrade_source(
        mut self,
    ) -> Result<UpgradeSource, Self> {
        let ready = (|| {
            let physical = Arc::get_mut(&mut self.physical).ok_or_else(refused)?;
            physical
                .cell
                .get_mut()
                .map_err(|_| refused())?
                .require_idle()?;
            physical.handler.require()?;
            physical.handler.require_index_idle_cached()
        })();
        match ready {
            Ok(()) => Ok(UpgradeSource { original: self }),
            Err(_) => Err(self),
        }
    }
}

/// No file has been changed and no claim has been issued. The exact held files
/// and complete sealed candidate survive until admission or whole-owner drop.
/// Later admission MUST repeat these negative checks under the original catalog
/// mutex and retain an original index2 claim before the first file effect.
pub(in crate::store::critical_namespaces::native_registry) struct PreparedUpgrade {
    // Retained separately from mounted Current: cold inspection runs before
    // Current/JOIN openers, and no remembered descriptor substitutes for these
    // original authenticated marker objects at final admission or exposure.
    expected_current: ExpectedCurrentInputs,
    pub(super) snapshot: UpgradeSnapshot,
    pub(super) output: Output,
    // After BOTH real file/buffer fields, before the old mount and new fee.
    #[cfg(test)]
    pub(super) retirement:
        Option<crate::store::persistence::native_registry::operational::RetirementObservation>,
    original: UpgradeSource,
    // LAST: new buffers/files must retire before their independent reservation.
    _fee: CatalogReadReservation,
}
impl std::fmt::Debug for PreparedUpgrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedUpgrade").finish_non_exhaustive()
    }
}
impl Drop for PreparedUpgrade {
    fn drop(&mut self) {
        // Close FIRST, before the actual snapshot/output destructors. The old
        // owner stays whole until those paid buffers and File handles drain.
        self.original.original.close_index_admission();
    }
}
impl UpgradeSource {
    /// Every refusal returns this unchanged original source. Sealing may draw
    /// RNG in an unpublished candidate; no writing role/effect exists here.
    pub(in crate::store::critical_namespaces::native_registry) fn prepare(
        self,
        policy: Policy,
        rng: &mut impl CryptoRngCore,
    ) -> Result<PreparedUpgrade, Self> {
        let parts = (|| -> Result<_, AppError> {
            let physical = &self.original.physical;
            // cached_store only borrows the original allocation and checks its
            // negative close bit; no read/AEAD/candidate precedes the new debit.
            let store = physical.current.cached_store()?;
            let current = store.current_layout_policy.as_ref().ok_or_else(refused)?;
            if policy.pool.same_account(&physical.handler.pool)
                || policy.pool.same_account(&current.archive.pending.pool)
                || current
                    .member_outbox
                    .as_ref()
                    .is_none_or(|member| policy.pool.same_account(&member.pool))
            {
                return Err(refused());
            }
            let fee = policy.reserve()?;
            let store = physical.current.checked_store()?;
            physical.handler.require()?;
            physical.handler.require_index_idle_cached()?;
            let expected_current = ExpectedCurrentInputs::capture(store, &fee, &policy.pool)?;
            let observed_layout = expected_current.authenticate(store)?;
            let snapshot = UpgradeSnapshot::capture(store)?;
            let key = Zeroizing::new(store.keys.db_key()?);
            // Expected Current comes from the original mounted marker, not an
            // asserted context or the intent/catalog being decoded below.
            let layout = {
                let mounted = store.current_layout.try_lock().map_err(|_| refused())?;
                let mounted = mounted.as_ref().ok_or_else(refused)?;
                if mounted.negative.is_cancelled() {
                    return Err(refused());
                }
                mounted.layout
            };
            if observed_layout != layout {
                return Err(refused());
            }
            let expected = super::super::capability::NativeCapabilityData::for_current_data(
                layout,
                &layout.encode_data()?,
            )?;
            let output = Output::prepare(&key, expected, &snapshot, rng)?;
            // No saved readiness flag survives long decoding/sealing. This is
            // still NON-writing preparation; real claim admission repeats it.
            physical.current.checked_store()?;
            {
                let guard = store
                    .pending_join_catalog
                    .try_lock()
                    .map_err(|_| refused())?;
                let catalog = guard.as_ref().ok_or_else(refused)?;
                if catalog.pending.is_some()
                    || catalog.current.framed != *blake3::hash(snapshot.catalog()).as_bytes()
                {
                    return Err(refused());
                }
                physical.handler.require_index_idle_cached()?;
                physical.current.cached_store()?;
                snapshot.require(store)?;
                expected_current.require(store)?;
            }
            Ok((expected_current, snapshot, output, fee))
        })();
        match parts {
            Ok((expected_current, snapshot, output, fee)) => Ok(PreparedUpgrade {
                expected_current,
                snapshot,
                output,
                #[cfg(test)]
                retirement: None,
                original: self,
                _fee: fee,
            }),
            Err(_) => Err(self),
        }
    }
}

#[cfg(test)]
impl PreparedUpgrade {
    pub(super) fn inventory_for_test(
        &self,
        pool: &CriticalCatalogReadPool,
    ) -> crate::store::persistence::native_registry::lifecycle::upgrade::UpgradeInventory {
        self.try_inventory_for_test(pool).unwrap()
    }
    pub(super) fn try_inventory_for_test(
        &self,
        pool: &CriticalCatalogReadPool,
    ) -> Result<
        crate::store::persistence::native_registry::lifecycle::upgrade::UpgradeInventory,
        AppError,
    > {
        crate::store::persistence::native_registry::lifecycle::upgrade::UpgradeInventory::capture(
            self.original_store_for_test(),
            &self._fee,
            pool,
        )
    }
    pub(super) fn expected_current_for_test(&self) -> &ExpectedCurrentInputs {
        &self.expected_current
    }
    pub(super) fn original_store_for_test(&self) -> &crate::store::ServerStore {
        self.original
            .original
            .physical
            .current
            .cached_store()
            .unwrap()
    }
}

#[cfg(test)]
impl UpgradeSource {
    // Tests may recover the unchanged pre-effect owner. Production gets no
    // physical loan, extractor or escape from the consuming upgrade boundary.
    pub(super) fn into_original_for_test(self) -> NativeIndexStore {
        self.original
    }
}
