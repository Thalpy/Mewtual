//! Existing-only genesis settlement locks. No registration, owner replacement, head write,
//! row transition or raw Runtime escapes this family. The real scope borrow holds admission
//! continuously from its covering barrier through pins/canonical and the later Root guard.
use super::cold_current::ColdOwnedV2CurrentOwnerNamespace;
use super::*;
use crate::store::chat_admission::v2_current::{
    DurableGenesisCurrent2SettlementAdmission, GenesisCurrent2SettlementReadAdmission,
    SettledGenesisCurrent2ReadAdmission,
};

enum ExistingOwnerCore<'guard, 'writer> {
    Warm(CurrentV2OwnerNamespace<'guard, 'writer>),
    Cold(ColdOwnedV2CurrentOwnerNamespace<'guard, 'writer>),
}
struct ExistingOwner<'guard, 'writer> {
    core: ExistingOwnerCore<'guard, 'writer>,
    admission: &'guard GenesisCurrent2SettlementReadAdmission<'writer>,
    identifier: [u8; 32],
}
impl ExistingOwner<'_, '_> {
    fn validate_source_only(&self) -> Result<(), AppError> {
        self.admission.validate_source_only()?;
        match &self.core {
            ExistingOwnerCore::Warm(n) => n.validate_source_only()?,
            ExistingOwnerCore::Cold(n) => n.validate_source_only()?,
        }
        if self.identifier != key(self.admission.group(), self.admission.signer()) {
            return Err(refused());
        }
        Ok(())
    }
    fn validate_old_files(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        match &self.core {
            ExistingOwnerCore::Warm(n) => n.validate_old_files()?,
            ExistingOwnerCore::Cold(n) => n.validate_old_files()?,
        }
        self.validate_source_only()
    }
    fn registration_parts(&self) -> (&super::super::v2_registration::Runtime, &[u8; 32]) {
        let runtime = match &self.core {
            ExistingOwnerCore::Warm(n) => &n.inner.pool.v2_writers,
            ExistingOwnerCore::Cold(n) => n.settlement_registration_runtime(),
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

fn lock_existing<'guard, 'writer>(
    admission: &'guard GenesisCurrent2SettlementReadAdmission<'writer>,
) -> Result<ExistingOwner<'guard, 'writer>, AppError> {
    admission.validate_source_only()?;
    let store = admission.store();
    let task = admission.task();
    let core = if let Some(warm) = admission.warm_admission() {
        ExistingOwnerCore::Warm(store.lock_current_v2_group_namespace(warm, task)?)
    } else {
        let cold = admission.cold_admission().ok_or_else(refused)?;
        task.require_cold_owned_handoff(cold)?;
        ExistingOwnerCore::Cold(store.lock_cold_owned_v2_current_namespace(cold, task)?)
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
