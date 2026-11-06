//! Actual cold Current claim and distinct owned/proof negative namespaces. Only concrete
//! original Scope roles enter these locks; no decoded status or namespace callback issues one.
use super::*;
mod settlement;
use crate::store::chat_admission::v2_current::{
    ColdCurrentProofAdmission, ColdOwnedCurrentAdmission, ColdV2CurrentReadAdmission,
    ColdV2GenesisScopeBasis,
};
use crate::store::critical_namespaces::current_layout::OriginalV2GenesisGroupObservation;
use crate::store::persistence::v2_genesis_activation::ColdV2GenesisCurrentHead;

/// Opaque, non-Clone observation issued ONLY after the exact actual pending entry install.
/// Prebuilt payload never escapes PreparedGroupRegistration before that commit. Its original
/// tiny Weak and Root DATA control blocks have their own real payments, no strong task life.
pub(in crate::store) struct RegisteredColdCurrentClaim {
    head: Arc<ColdV2GenesisCurrentHead>,
    basis: ColdV2GenesisScopeBasis,
    original: OriginalV2GenesisGroupObservation,
}
impl RegisteredColdCurrentClaim {
    pub(super) fn prepare(task: &PreparedV2GenesisGroupNamespace) -> Result<Self, AppError> {
        let head = task.cold_current_head_data().ok_or_else(refused)?.clone();
        Ok(Self {
            basis: head.scope_basis_data().clone(),
            head,
            original: task.observe_original(),
        })
    }
    pub(in crate::store) fn require_original(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
        head: &Arc<ColdV2GenesisCurrentHead>,
    ) -> Result<(), AppError> {
        self.original.require_same_original(task)?;
        if !Arc::ptr_eq(&self.head, head) || &self.basis != head.scope_basis_data() {
            return Err(refused());
        }
        Ok(())
    }
}

pub(in crate::store) fn prepare_registration(
    store: &ServerStore,
    pool: &CriticalCatalogReadPool,
    task: &PreparedV2GenesisGroupNamespace,
) -> Result<PreparedGroupRegistration, AppError> {
    task.cold_current_head_data().ok_or_else(refused)?;
    PreparedGroupRegistration::new(store, pool, task)
}

enum ColdAdmission<'task, 'writer> {
    Register(&'task ColdV2CurrentReadAdmission<'writer>),
    Owned(&'task ColdOwnedCurrentAdmission<'writer>),
    Proof(&'task ColdCurrentProofAdmission<'writer>),
}
impl<'task, 'writer> ColdAdmission<'task, 'writer> {
    fn store(&self) -> &'task ServerStore {
        match self {
            Self::Register(a) => a.store(),
            Self::Owned(a) => a.store(),
            Self::Proof(a) => a.store(),
        }
    }
    fn source(&self) -> &GenesisSourceDataRef<'_> {
        match self {
            Self::Register(a) => a.source(),
            Self::Owned(a) => a.source(),
            Self::Proof(a) => a.source(),
        }
    }
    fn validate_task(&self, task: &PreparedV2GenesisGroupNamespace) -> Result<(), AppError> {
        match self {
            Self::Register(a) => {
                a.validate_source_only()?;
                task.retains_original(a.store(), a.source())?;
                let head = task.cold_current_head_data().ok_or_else(refused)?;
                if head.scope_basis_data() != a.scope_basis_data()
                    || task.group() != a.group()
                    || task.signer() != a.signer()
                {
                    return Err(refused());
                }
                head.retains_original(a.store(), a.source())
            }
            Self::Owned(a) => a.require_owned_current_lineage_for_own_group_task(task),
            Self::Proof(a) => a.require_retained_current_lineage_for_own_group_task(task),
        }
    }
    fn association(&self) -> Result<CurrentFilenameAssociation<'task>, AppError> {
        match self {
            Self::Register(a) => {
                let a: &'task ColdV2CurrentReadAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Owned(a) => {
                let a: &'task ColdOwnedCurrentAdmission<'writer> = a;
                a.current_filename_association()
            }
            Self::Proof(a) => {
                let a: &'task ColdCurrentProofAdmission<'writer> = a;
                a.current_filename_association()
            }
        }
    }
}
struct ColdNamespace<'task, 'writer> {
    admission: ColdAdmission<'task, 'writer>,
    task: &'task PreparedV2GenesisGroupNamespace,
    pool: MutexGuard<'task, Runtime>,
    pins: MutexGuard<'task, super::super::super::pins::Runtime>,
    association: CurrentFilenameAssociation<'task>,
    identifier: [u8; 32],
}
impl ColdNamespace<'_, '_> {
    fn validate_source_only(&self) -> Result<(), AppError> {
        self.admission.validate_task(self.task)?;
        self.association.require_original_data()?;
        if !self.pins.managed
            || self.pins.dirty
            || self
                .pins
                .table
                .as_ref()
                .ok_or_else(refused)?
                .contains_key(self.task.group())
            || self.pool.heads.contains_key(&self.identifier)
            || self.identifier != key(self.task.group(), self.task.signer())
        {
            return Err(refused());
        }
        match &self.admission {
            ColdAdmission::Register(_) => {
                if self.pins.pending_groups.excludes_group(self.task.group()) {
                    return Err(refused());
                }
                self.pool
                    .v2_writers
                    .require_no_live_writer(&self.identifier)
            }
            ColdAdmission::Owned(admission) => {
                if self.pins.pending_groups.excludes_group(self.task.group()) {
                    self.pins
                        .pending_groups
                        .require_same_task(self.admission.store(), self.task)
                } else {
                    self.task.require_cold_owned_handoff(admission)
                }
            }
            ColdAdmission::Proof(admission) => {
                // Existing installed receipt proof never accepts registered-only ownership.
                self.task.require_cold_proof_handoff(admission)?;
                if self.pins.pending_groups.excludes_group(self.task.group()) {
                    self.pins
                        .pending_groups
                        .require_same_task(self.admission.store(), self.task)?;
                }
                Ok(())
            }
        }
    }
    fn validate_old_files(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        require_old_owner_files_absent(self.admission.store(), &self.association, self.identifier)?;
        self.validate_source_only()
    }
}

pub(in crate::store) struct RegisteringColdV2CurrentNamespace<'task, 'writer> {
    inner: ColdNamespace<'task, 'writer>,
}
pub(in crate::store) struct ColdOwnedV2CurrentOwnerNamespace<'task, 'writer> {
    inner: ColdNamespace<'task, 'writer>,
    admission: &'task ColdOwnedCurrentAdmission<'writer>,
}
pub(in crate::store) struct ColdV2CurrentProofOwnerNamespace<'task, 'writer> {
    inner: ColdNamespace<'task, 'writer>,
    admission: &'task ColdCurrentProofAdmission<'writer>,
}
macro_rules! owned_accessors {
    ($ty:ident, $admission:ident) => {
        impl<'task, 'writer> $ty<'task, 'writer> {
            pub(in crate::store) fn admission(&self) -> &'task $admission<'writer> {
                self.admission
            }
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.inner.validate_source_only()
            }
            pub(in crate::store) fn validate_old_files(&self) -> Result<(), AppError> {
                self.inner.validate_old_files()
            }
        }
    };
}
owned_accessors!(ColdOwnedV2CurrentOwnerNamespace, ColdOwnedCurrentAdmission);
owned_accessors!(ColdV2CurrentProofOwnerNamespace, ColdCurrentProofAdmission);
impl<'task, 'writer> ColdOwnedV2CurrentOwnerNamespace<'task, 'writer> {
    /// Actual install/installed-proof gates require completed exact owner handoff. The
    /// pre-handoff owned guard exists only to finish that transfer, never mint a receipt.
    pub(in crate::store) fn require_completed_cold_handoff(&self) -> Result<(), AppError> {
        self.validate_source_only()?;
        self.inner.task.require_cold_owned_handoff(self.admission)
    }
    pub(in crate::store::owner_evidence::canonical) fn cold_registration_parts(
        &mut self,
    ) -> (
        &ColdOwnedCurrentAdmission<'writer>,
        &mut super::super::v2_registration::Runtime,
    ) {
        (self.admission, &mut self.inner.pool.v2_writers)
    }
    pub(in crate::store) fn handoff_under_archive_root(
        &mut self,
        prepared: crate::store::critical_namespaces::current_layout::group_task::cold_current::PreparedColdOwnerAssociation<'_, '_>,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<Option<RetiredGroupEntry>, AppError> {
        self.validate_old_files()?;
        let position = self
            .inner
            .pins
            .pending_groups
            .buffer
            .as_ref()
            .and_then(|buffer| {
                buffer.slots.iter().position(|slot| {
                    slot.as_ref()
                        .is_some_and(|entry| entry.group() == self.inner.task.group())
                })
            });
        if position.is_some() {
            self.inner
                .pins
                .pending_groups
                .require_same_task(self.admission.store(), self.inner.task)?;
        } else {
            self.inner.task.require_cold_owned_handoff(self.admission)?;
        }
        let store = self.admission.store();
        let head = self.admission.head();
        head.validate_under_archive_root(store, self.admission.source(), root)?;
        store.validate_mounted_current_layout_under_archive_root(root)?;
        crate::store::persistence::require_cold_head_group_directory(head)?;
        self.validate_source_only()?;
        // The only cross-family setter changes negative association, never registry debt.
        // All checks precede it. After successful set this private exact take is infallible,
        // with no callback/allocation/refund. No Store-visible plan can retire early.
        prepared.install(self.inner.task, self.admission)?;
        let entry = position.and_then(|position| {
            self.inner
                .pins
                .pending_groups
                .buffer
                .as_mut()
                .and_then(|buffer| buffer.slots.get_mut(position))
                .and_then(Option::take)
        });
        Ok(entry.map(|entry| RetiredGroupEntry { _entry: entry }))
    }
}
impl<'task, 'writer> ColdV2CurrentProofOwnerNamespace<'task, 'writer> {
    pub(in crate::store::owner_evidence::canonical) fn cold_proof_registration_parts(
        &self,
    ) -> (
        &ColdCurrentProofAdmission<'writer>,
        &super::super::v2_registration::Runtime,
    ) {
        (self.admission, &self.inner.pool.v2_writers)
    }
}

impl RegisteringColdV2CurrentNamespace<'_, '_> {
    pub(in crate::store) fn register_under_archive_root(
        &mut self,
        prepared: &mut PreparedGroupRegistration,
        root: &crate::store::critical_namespaces::archive_bootstrap::MountedRoot,
    ) -> Result<RegisteredColdCurrentClaim, AppError> {
        self.inner.validate_old_files()?;
        let task = self.inner.task;
        let head = task.cold_current_head_data().ok_or_else(refused)?;
        let witness = prepared.cold_witness.as_ref().ok_or_else(refused)?;
        witness.require_original(task, head)?;
        let store = self.inner.admission.store();
        head.validate_under_archive_root(store, self.inner.admission.source(), root)?;
        store.validate_mounted_current_layout_under_archive_root(root)?;
        crate::store::persistence::require_cold_head_group_directory(head)?;
        self.inner.validate_source_only()?;
        // Take the prebuilt observation before the install. On any precommit refusal it
        // returns to the same outside owner; no last fee/data is dropped under the guards.
        let witness = prepared.cold_witness.take().ok_or_else(refused)?;
        match self
            .inner
            .pins
            .pending_groups
            .install(store, task, prepared)
        {
            Ok(()) => Ok(witness),
            Err(error) => {
                prepared.cold_witness = Some(witness);
                Err(error)
            }
        }
    }
}

fn lock_namespace<'task, 'writer>(
    admission: ColdAdmission<'task, 'writer>,
    task: &'task PreparedV2GenesisGroupNamespace,
) -> Result<ColdNamespace<'task, 'writer>, AppError> {
    admission.validate_task(task)?;
    let store = admission.store();
    task.cold_current_resources()?
        .validate_original(store, admission.source())?;
    let pins = store.owner_evidence.pins.lock().map_err(|_| refused())?;
    let pool = store
        .owner_evidence
        .canonical
        .lock()
        .map_err(|_| refused())?;
    let association = admission.association()?;
    let inner = ColdNamespace {
        admission,
        task,
        pool,
        pins,
        association,
        identifier: key(task.group(), task.signer()),
    };
    inner.validate_old_files()?;
    Ok(inner)
}
impl ServerStore {
    pub(in crate::store) fn lock_registering_cold_v2_current_namespace<'task, 'writer>(
        &'task self,
        admission: &'task ColdV2CurrentReadAdmission<'writer>,
        task: &'task PreparedV2GenesisGroupNamespace,
    ) -> Result<RegisteringColdV2CurrentNamespace<'task, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(RegisteringColdV2CurrentNamespace {
            inner: lock_namespace(ColdAdmission::Register(admission), task)?,
        })
    }
    pub(in crate::store) fn lock_cold_owned_v2_current_namespace<'task, 'writer>(
        &'task self,
        admission: &'task ColdOwnedCurrentAdmission<'writer>,
        task: &'task PreparedV2GenesisGroupNamespace,
    ) -> Result<ColdOwnedV2CurrentOwnerNamespace<'task, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(ColdOwnedV2CurrentOwnerNamespace {
            inner: lock_namespace(ColdAdmission::Owned(admission), task)?,
            admission,
        })
    }
    pub(in crate::store) fn lock_cold_v2_current_proof_namespace<'task, 'writer>(
        &'task self,
        admission: &'task ColdCurrentProofAdmission<'writer>,
        task: &'task PreparedV2GenesisGroupNamespace,
    ) -> Result<ColdV2CurrentProofOwnerNamespace<'task, 'writer>, AppError> {
        if !std::ptr::eq(self, admission.store()) {
            return Err(refused());
        }
        Ok(ColdV2CurrentProofOwnerNamespace {
            inner: lock_namespace(ColdAdmission::Proof(admission), task)?,
            admission,
        })
    }
}
