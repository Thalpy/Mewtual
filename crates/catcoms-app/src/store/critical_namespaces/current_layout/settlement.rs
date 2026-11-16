//! Separate covering-barrier cleanup and settled-read roles. Neither can write a head,
//! install/rebind a writer, create a group or adopt a cold unowned scope. The real owners,
//! worker and seed-ABSENT cold reader jointly consume these distinct guards.
use super::*;
use crate::store::chat_admission::v2_current::{
    DurableGenesisCurrent2SettlementAdmission, GenesisCurrent2SettlementReadAdmission,
    SettledGenesisCurrent2ReadAdmission,
};
use crate::store::owner_evidence::canonical::current_vacancy::settlement::{
    GenesisSeedSettlementOwnerNamespace, SettledGenesisCurrentOwnerNamespace,
};
use crate::store::owner_evidence::canonical::v2_registration::V2WriterObservation;
use crate::store::persistence::v2_genesis_activation::settlement::GenesisSeedSettlementResources;
use std::sync::MutexGuard;

pub(in crate::store) const GENESIS_SEED_SETTLEMENT_GUARD_BYTES: usize =
    2 * std::mem::size_of::<GenesisSeedSettlementGroupGuard<'static, 'static>>()
        + 2 * std::mem::size_of::<SettledGenesisCurrentGroupGuard<'static, 'static>>()
        + 4 * (MAX_OPEN_BASE_PATH + 256)
        + 2 * std::mem::size_of::<String>()
        + 256;

/// Held Root/canonical/pins plus SAME covering scope writer. Only the actual original
/// cleanup worker accepts this type. The resources loan outlives every selector/guard drop.
pub(in crate::store) struct GenesisSeedSettlementGroupGuard<'guard, 'writer> {
    root: MutexGuard<'guard, Option<super::super::archive_bootstrap::MountedRoot>>,
    owner: GenesisSeedSettlementOwnerNamespace<'guard, 'writer>,
    resources: &'guard GenesisSeedSettlementResources,
}
/// Separate read-only installed-owner closure after actual seed settlement. This cannot
/// enter cleanup or an earlier Seed/Head writer and cannot create a cold claim.
pub(in crate::store) struct SettledGenesisCurrentGroupGuard<'guard, 'writer> {
    root: MutexGuard<'guard, Option<super::super::archive_bootstrap::MountedRoot>>,
    owner: SettledGenesisCurrentOwnerNamespace<'guard, 'writer>,
    resources: &'guard GenesisSeedSettlementResources,
}

fn require_original_group_name(
    admission: &GenesisCurrent2SettlementReadAdmission<'_>,
) -> Result<(), AppError> {
    let task = admission.task();
    match (
        admission.warm_admission(),
        admission.cold_admission(),
        admission.repaired_admission(),
    ) {
        (Some(_), None, None) => {
            let (path, file, id) = task.saved_directory()?.directory_data();
            crate::store::persistence::v2_genesis_activation::require_directory_name(path, file, id)
        }
        (None, Some(cold), None) => {
            task.require_cold_owned_handoff(cold)?;
            crate::store::persistence::require_cold_head_group_directory(cold.head())
        }
        (None, None, Some(repaired)) => {
            task.require_cold_initializing_owned_handoff(repaired.owner_admission())?;
            // Pure original held/named group proof after source/ID3 waits. The separate
            // settlement body owns the seed phase; the old repaired Head proof stays strict.
            let (path, file, id) = repaired.head().seed().directory_data();
            crate::store::persistence::v2_genesis_activation::require_directory_name(path, file, id)
        }
        _ => Err(refused()),
    }
}
macro_rules! settlement_group_guard {
    ($guard:ident, $admission:ident, $method:ident, $owner_method:ident) => {
        impl<'guard, 'writer> $guard<'guard, 'writer> {
            pub(in crate::store) fn scope_admission(&self) -> &$admission<'writer> {
                self.owner.scope_admission()
            }
            pub(in crate::store) fn resources(&self) -> &GenesisSeedSettlementResources {
                self.resources
            }
            pub(in crate::store) fn archive_root(
                &self,
            ) -> &super::super::archive_bootstrap::MountedRoot {
                self.root
                    .as_ref()
                    .expect("actual Root checked by typed settlement constructor")
            }
            /// No account/I/O/lock/SDK/encoding; ONLY actual same-owner/task negatives.
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.owner.validate_source_only()?;
                let admission = self.scope_admission();
                self.resources
                    .require_original_negative(admission.store(), admission.owner_ref())?;
                if self.archive_root().negative.is_cancelled() {
                    return Err(refused());
                }
                Ok(())
            }
            pub(in crate::store) fn require_installed_writer_observation(
                &self,
                original: &V2WriterObservation,
            ) -> Result<(), AppError> {
                self.owner.require_settlement_writer_observation(original)
            }
            fn validate_original_namespace(&self) -> Result<(), AppError> {
                let admission = self.scope_admission();
                let store = admission.store();
                let source = admission.source();
                // All account/full comparer waits precede the final source -> ID3 -> held
                // directory edge. In particular do not call the old named-seed Head proof.
                self.resources
                    .validate_original(store, admission.owner_ref())?;
                self.owner.validate_old_files()?;
                source
                    .source()
                    .validate_under_archive_root(store, self.archive_root())?;
                store.validate_mounted_current_layout_under_archive_root(self.archive_root())?;
                require_original_group_name(admission.read_admission())?;
                self.require_installed_writer_observation(self.resources.writer_observation())?;
                self.validate_source_only()
            }
        }
        impl ServerStore {
            pub(in crate::store) fn $method<'guard, 'writer>(
                &'guard self,
                admission: &'guard $admission<'writer>,
                resources: &'guard GenesisSeedSettlementResources,
            ) -> Result<$guard<'guard, 'writer>, AppError> {
                if !std::ptr::eq(self, admission.store()) {
                    return Err(refused());
                }
                // Actual fee loan validation precedes every old filename selector/lock.
                resources.validate_original(self, admission.owner_ref())?;
                admission.validate_source_only()?;
                let owner = self.$owner_method(admission)?;
                let root = self.archive_bootstrap_root.lock().map_err(|_| refused())?;
                root.as_ref().ok_or_else(refused)?;
                let guard = $guard {
                    root,
                    owner,
                    resources,
                };
                guard.validate_original_namespace()?;
                Ok(guard)
            }
        }
    };
}
settlement_group_guard!(
    GenesisSeedSettlementGroupGuard,
    DurableGenesisCurrent2SettlementAdmission,
    lock_genesis_seed_settlement_group,
    lock_genesis_seed_settlement_owner_namespace
);
settlement_group_guard!(
    SettledGenesisCurrentGroupGuard,
    SettledGenesisCurrent2ReadAdmission,
    lock_settled_genesis_current_group,
    lock_settled_genesis_current_owner_namespace
);
