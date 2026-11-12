//! Existing-only genesis settlement locks. No registration, owner replacement, head write,
//! row transition or raw Runtime escapes this family. The real scope borrow holds admission
//! continuously from its covering barrier through pins/canonical and the later Root guard.
use super::cold_current::ColdOwnedV2CurrentOwnerNamespace;
use super::cold_initializing::ColdRepairedV2GenesisCurrentNamespace;
use super::*;
use crate::store::chat_admission::v2_current::{
    DurableGenesisCurrent2SettlementAdmission, GenesisCurrent2SettlementReadAdmission,
    SettledGenesisCurrent2ReadAdmission,
};

enum ExistingOwnerCore<'guard, 'writer> {
    Warm(CurrentV2OwnerNamespace<'guard, 'writer>),
    Cold(ColdOwnedV2CurrentOwnerNamespace<'guard, 'writer>),
    Repaired(ColdRepairedV2GenesisCurrentNamespace<'guard, 'writer>),
}
pub(super) struct ExistingOwner<'guard, 'writer> {
    core: ExistingOwnerCore<'guard, 'writer>,
    admission: &'guard GenesisCurrent2SettlementReadAdmission<'writer>,
    identifier: [u8; 32],
}
impl ExistingOwner<'_, '_> {
    pub(super) fn validate_source_only(&self) -> Result<(), AppError> {
        self.admission.validate_source_only()?;
        match &self.core {
            ExistingOwnerCore::Warm(n) => n.validate_source_only()?,
            ExistingOwnerCore::Cold(n) => n.validate_source_only()?,
            ExistingOwnerCore::Repaired(n) => n.validate_source_only()?,
        }
        if self.identifier != key(self.admission.group(), self.admission.signer()) {
            return Err(refused());
        }
        Ok(())
    }
    pub(super) fn validate_old_files(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        match &self.core {
            ExistingOwnerCore::Warm(n) => n.validate_old_files()?,
            ExistingOwnerCore::Cold(n) => n.validate_old_files()?,
            ExistingOwnerCore::Repaired(n) => n.validate_old_files()?,
        }
        self.validate_source_only()
    }
    pub(super) fn text_lane_runtime(&self) -> &super::super::text_lane::TextLaneRuntime {
        match &self.core {
            ExistingOwnerCore::Warm(n) => &n.inner.pool.text_lanes,
            ExistingOwnerCore::Cold(n) => n.text_lane_runtime(),
            ExistingOwnerCore::Repaired(n) => n.text_lane_runtime(),
        }
    }
    /// Canonical-family-only negative active-lane storage. No installed writer mutation.
    pub(super) fn text_lane_runtime_mut(
        &mut self,
    ) -> &mut super::super::text_lane::TextLaneRuntime {
        match &mut self.core {
            ExistingOwnerCore::Warm(n) => &mut n.inner.pool.text_lanes,
            ExistingOwnerCore::Cold(n) => n.text_lane_runtime_mut(),
            ExistingOwnerCore::Repaired(n) => n.text_lane_runtime_mut(),
        }
    }
    pub(super) fn registration_parts(
        &self,
    ) -> (&super::super::v2_registration::Runtime, &[u8; 32]) {
        let runtime = match &self.core {
            ExistingOwnerCore::Warm(n) => &n.inner.pool.v2_writers,
            ExistingOwnerCore::Cold(n) => n.settlement_registration_runtime(),
            ExistingOwnerCore::Repaired(n) => n.settlement_registration_runtime(),
        };
        (runtime, &self.identifier)
    }
}

/// Effect-capable only because its actual borrowed Durable admission follows the covering
/// scope2 barrier. Root still requires its own original immutable cleanup state/fee/phase.
pub(in crate::store) struct GenesisSeedSettlementOwnerNamespace<'guard, 'writer> {
    inner: ExistingOwner<'guard, 'writer>,
    admission: &'guard DurableGenesisCurrent2SettlementAdmission<'writer>,
}
/// Read-only actual settled owner2. This distinct type cannot be passed to the unlink worker.
pub(in crate::store) struct SettledGenesisCurrentOwnerNamespace<'guard, 'writer> {
    inner: ExistingOwner<'guard, 'writer>,
    admission: &'guard SettledGenesisCurrent2ReadAdmission<'writer>,
}

macro_rules! existing_namespace {
    ($ty:ident, $admission:ident) => {
        impl<'guard, 'writer> $ty<'guard, 'writer> {
            pub(in crate::store) fn scope_admission(&self) -> &'guard $admission<'writer> {
                self.admission
            }
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.inner.validate_source_only()
            }
            pub(in crate::store) fn validate_old_files(&self) -> Result<(), AppError> {
                self.inner.validate_old_files()
            }
            // Only canonical's kernel child receives its OWN readonly generation map.
            // No namespace/public caller can select a slot or obtain mutable map access.
            pub(in crate::store::owner_evidence::canonical) fn settlement_registration_parts(
                &self,
            ) -> (&super::super::v2_registration::Runtime, &[u8; 32]) {
                self.inner.registration_parts()
            }
        }
    };
}
existing_namespace!(
    GenesisSeedSettlementOwnerNamespace,
    DurableGenesisCurrent2SettlementAdmission
);
existing_namespace!(
    SettledGenesisCurrentOwnerNamespace,
    SettledGenesisCurrent2ReadAdmission
);

pub(super) fn lock_existing<'guard, 'writer>(
    admission: &'guard GenesisCurrent2SettlementReadAdmission<'writer>,
) -> Result<ExistingOwner<'guard, 'writer>, AppError> {
    admission.validate_source_only()?;
    let store = admission.store();
    let task = admission.task();
    let core = match (
        admission.warm_admission(),
        admission.cold_admission(),
        admission.repaired_admission(),
    ) {
        (Some(warm), None, None) => {
            ExistingOwnerCore::Warm(store.lock_current_v2_group_namespace(warm, task)?)
        }
        (None, Some(cold), None) => {
            task.require_cold_owned_handoff(cold)?;
            ExistingOwnerCore::Cold(store.lock_cold_owned_v2_current_namespace(cold, task)?)
        }
        (None, None, Some(repaired)) => {
            task.require_cold_initializing_owned_handoff(repaired.owner_admission())?;
            // This acquires only the actual existing canonical/pins closure. Do NOT call
            // the old Root-bearing repaired guard: it still mandates activation.seed.
            ExistingOwnerCore::Repaired(store.lock_cold_repaired_current_namespace(repaired, task)?)
        }
        _ => return Err(refused()),
    };
    let inner = ExistingOwner {
        core,
        admission,
        identifier: key(admission.group(), admission.signer()),
    };
    inner.validate_old_files()?;
    Ok(inner)
}
impl ServerStore {
    pub(in crate::store) fn lock_genesis_seed_settlement_owner_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard DurableGenesisCurrent2SettlementAdmission<'writer>,
    ) -> Result<GenesisSeedSettlementOwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(GenesisSeedSettlementOwnerNamespace {
            inner: lock_existing(admission.read_admission())?,
            admission,
        })
    }
    pub(in crate::store) fn lock_settled_genesis_current_owner_namespace<'guard, 'writer>(
        &'guard self,
        admission: &'guard SettledGenesisCurrent2ReadAdmission<'writer>,
    ) -> Result<SettledGenesisCurrentOwnerNamespace<'guard, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(SettledGenesisCurrentOwnerNamespace {
            inner: lock_existing(admission.read_admission())?,
            admission,
        })
    }
}
