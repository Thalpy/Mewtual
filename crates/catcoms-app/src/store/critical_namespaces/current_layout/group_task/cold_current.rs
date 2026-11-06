//! Complete cold Current claims. No warm directory state is manufactured here. Actual
//! Scope owns the sole sender; this task retains only a funded original negative association.
use super::*;
use crate::store::chat_admission::v2_current::{
    ColdCurrentProofAdmission, ColdOwnedCurrentAdmission, ColdV2GenesisScopeBasis,
};
use crate::store::owner_evidence::canonical::current_vacancy::cold_current as owners;
use crate::store::owner_evidence::canonical::v2_registration::{
    cold_current::PreparedColdV2WriterRegistration, V2WriterObservation,
};
use crate::store::persistence::v2_genesis_activation::{
    ColdV2GenesisCurrentHead, ColdV2GenesisReadResources,
};
use std::sync::OnceLock;

pub(super) struct ColdCurrentClaim {
    head: Arc<ColdV2GenesisCurrentHead>,
    resources: ColdV2GenesisReadResources,
    // Only the successful actual registry guard may issue this opaque original witness.
    registered: Option<owners::RegisteredColdCurrentClaim>,
    owner: OnceLock<ColdScopeOwnerAssociation>,
}
struct ColdScopeOwnerAssociation {
    identity: Arc<()>,
    basis: ColdV2GenesisScopeBasis,
    // LAST: actual watch and authority tails keep the SAME real cold metadata payment.
    negative: RequestCancellation,
}

/// Fixed prepared negative association. Only canonical's concrete Root-proved handoff can
/// take a slot; this setter alone owns no registry access or Current permission.
pub(in crate::store) struct PreparedColdOwnerAssociation<'task, 'writer> {
    task: &'task PreparedV2GenesisGroupNamespace,
    admission: &'task ColdOwnedCurrentAdmission<'writer>,
    association: Option<ColdScopeOwnerAssociation>,
}
impl PreparedColdOwnerAssociation<'_, '_> {
    pub(in crate::store) fn install(
        self,
        task: &PreparedV2GenesisGroupNamespace,
        admission: &ColdOwnedCurrentAdmission<'_>,
    ) -> Result<(), AppError> {
        if !std::ptr::eq(self.task, task) || !std::ptr::eq(self.admission, admission) {
            return Err(refused());
        }
        task.require_cold_current_scope_owner(admission)?;
        if let Some(association) = self.association {
            if association.negative.is_cancelled() {
                return Err(refused());
            }
            // Err leaves the existing cell unchanged. Success is followed only by the
            // canonical caller's private infallible exact slot move.
            task.cold()?.owner.set(association).map_err(|_| refused())
        } else {
            task.require_cold_owned_handoff(admission)
        }
    }
}

// Separate named task metadata from Root's cold file/receipt loan. Native fixed selectors
// are actually exercised by the owned/proof guards and retain their independent allowance.
const COLD_TASK_FEE: usize = GROUP_TASK_STAMP_FEE
    + std::mem::size_of::<PreparedColdOwnerAssociation<'static, 'static>>()
    + 2 * std::mem::size_of::<ColdCurrentClaim>()
    + 2 * std::mem::size_of::<ColdScopeOwnerAssociation>()
    + 2 * std::mem::size_of::<owners::RegisteredColdCurrentClaim>()
    + std::mem::size_of::<owners::ColdOwnedV2CurrentOwnerNamespace<'static, 'static>>()
    + std::mem::size_of::<owners::ColdV2CurrentProofOwnerNamespace<'static, 'static>>()
    + 4 * (super::super::MAX_OPEN_BASE_PATH + 256)
    + 32 * 1024
    + 2 * std::mem::size_of::<String>()
    + 64
    + 128;

impl PreparedV2GenesisGroupNamespace {
    fn cold(&self) -> Result<&ColdCurrentClaim, AppError> {
        match &self.custody {
            GroupTaskCustody::ColdCurrent(cold) => Ok(cold),
            GroupTaskCustody::Warm { .. } | GroupTaskCustody::ColdInitializing(_) => Err(refused()),
        }
    }
    pub(in crate::store) fn cold_current_head_data(
        &self,
    ) -> Option<&Arc<ColdV2GenesisCurrentHead>> {
        match &self.custody {
            GroupTaskCustody::ColdCurrent(cold) => Some(&cold.head),
            GroupTaskCustody::Warm { .. } | GroupTaskCustody::ColdInitializing(_) => None,
        }
    }
    pub(in crate::store) fn cold_current_resources(
        &self,
    ) -> Result<&ColdV2GenesisReadResources, AppError> {
        Ok(&self.cold()?.resources)
    }
    pub(in crate::store) fn require_registered_cold_current_claim(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        head: &Arc<ColdV2GenesisCurrentHead>,
    ) -> Result<(), AppError> {
        self.retains_original(store, source)?;
        let cold = self.cold()?;
        if cold.owner.get().is_some() || !Arc::ptr_eq(&cold.head, head) {
            return Err(refused());
        }
        cold.resources.require_original_negative(store, source)?;
        cold.registered
            .as_ref()
            .ok_or_else(refused)?
            .require_original(self, head)
    }
    // Keep the complete identity tuple explicit at this shared comparison boundary. Each
    // typed entry point supplies actual source/head/basis/owner/observer loans; no field is
    // optional, and omitting one would let an equal label substitute for original custody.
    #[allow(clippy::too_many_arguments)]
    fn require_owner_data(
        &self,
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        head: &Arc<ColdV2GenesisCurrentHead>,
        basis: &ColdV2GenesisScopeBasis,
        identity: &Arc<()>,
        observer: &RequestCancellation,
        require_handoff: bool,
    ) -> Result<(), AppError> {
        self.retains_original(store, source)?;
        let cold = self.cold()?;
        cold.resources.require_original_negative(store, source)?;
        if !Arc::ptr_eq(&cold.head, head)
            || head.scope_basis_data() != basis
            || observer.is_cancelled()
        {
            return Err(refused());
        }
        match cold.owner.get() {
            Some(owner)
                if Arc::ptr_eq(&owner.identity, identity)
                    && &owner.basis == basis
                    && !owner.negative.is_cancelled() =>
            {
                Ok(())
            }
            Some(_) => Err(refused()),
            None if require_handoff => Err(refused()),
            None => self.require_registered_cold_current_claim(store, source, head),
        }
    }
    /// Negative only: actual role owns phase/clean gates. Never call its lineage wrapper
    /// here, which itself conjoins this matcher. The source-only comparator is acyclic.
    pub(in crate::store) fn require_cold_current_scope_owner(
        &self,
        admission: &ColdOwnedCurrentAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        if !std::ptr::eq(self, admission.task()) {
            return Err(refused());
        }
        self.require_owner_data(
            admission.store(),
            admission.source(),
            admission.head(),
            admission.scope_basis_data(),
            admission.owner_identity(),
            admission.owner_observer(),
            false,
        )
    }
    pub(in crate::store) fn require_cold_owned_handoff(
        &self,
        admission: &ColdOwnedCurrentAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        if !std::ptr::eq(self, admission.task()) {
            return Err(refused());
        }
        self.require_owner_data(
            admission.store(),
            admission.source(),
            admission.head(),
            admission.scope_basis_data(),
            admission.owner_identity(),
            admission.owner_observer(),
            true,
        )
    }
    pub(in crate::store) fn require_cold_proof_handoff(
        &self,
        admission: &ColdCurrentProofAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        if !std::ptr::eq(self, admission.task()) {
            return Err(refused());
        }
        self.require_owner_data(
            admission.store(),
            admission.source(),
            admission.head(),
            admission.scope_basis_data(),
            admission.owner_identity(),
            admission.owner_observer(),
            true,
        )
    }

    /// Claim existing complete Current2 under a NEW actual paid read/SDK and physical mount.
    /// Everything prepared before writers; after actual slot installation only moves remain.
    pub(in crate::store) fn from_cold_current(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        head: Arc<ColdV2GenesisCurrentHead>,
    ) -> Result<Self, AppError> {
        source.require_original_negative(store)?;
        head.retains_original(store, source)?;
        let policy = store.current_layout_policy.as_ref().ok_or_else(refused)?;
        let fee = Arc::new(policy.archive.pending.pool.try_reserve(COLD_TASK_FEE)?);
        let layout = store
            .current_layout
            .lock()
            .map_err(|_| refused())?
            .as_ref()
            .ok_or_else(refused)?
            .layout;
        let data = source.data();
        if layout.vault != data.context.spec_data().vault {
            return Err(refused());
        }
        let resources = head.read_resources();
        resources.validate_original(store, source)?;
        let mut task = Self {
            source: OriginalGroupSource {
                id: data.source,
                context: data.context.clone(),
                signer: data.context.spec_data().signer,
                read: source.source().observe_original(),
                sdk: source.seed().observe_original_seed_data(),
                mount: store.registry_mount(),
                negative: store.chat_mount.observer(),
            },
            layout,
            custody: GroupTaskCustody::ColdCurrent(ColdCurrentClaim {
                head,
                resources,
                registered: None,
                owner: OnceLock::new(),
            }),
            retired: None,
            life: Arc::new(GroupTaskLife {
                cancelled: AtomicBool::new(false),
            }),
            fee,
        };
        let mut prepared =
            owners::prepare_registration(store, &policy.archive.pending.pool, &task)?;
        let registered = store.with_cold_v2_genesis_current_read(source, |admission| {
            let mut owner = store.lock_registering_cold_v2_current_namespace(admission, &task)?;
            let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
            owner.register_under_archive_root(&mut prepared, root.as_ref().ok_or_else(refused)?)
        })?;
        // No callback/check/allocation in the commit-to-witness move. Unused prepared
        // backing drains/refunds here OUTSIDE guards before return; a concurrent close
        // merely makes the SAME returned DATA task negative, never a renewed claim.
        if let GroupTaskCustody::ColdCurrent(cold) = &mut task.custody {
            cold.registered = Some(registered);
        }
        Ok(task)
    }

    /// Actual owner handoff, not directory-only retirement. Caller MUST drain returned paid
    /// debt outside its admission callback. Task/owner negatives remain original afterward.
    pub(in crate::store) fn handoff_cold_current_owner(
        &self,
        admission: &ColdOwnedCurrentAdmission<'_>,
    ) -> Result<
        Option<crate::store::owner_evidence::canonical::current_vacancy::RetiredGroupEntry>,
        AppError,
    > {
        let store = admission.store();
        let cold = self.cold()?;
        // These clones do not allocate. All fixed storage and escaping tails are actually
        // prepaid; observe/compare the original owner AFTER clone/setup and full waits.
        let association = if cold.owner.get().is_none() {
            Some(ColdScopeOwnerAssociation {
                identity: admission.owner_identity().clone(),
                basis: admission.scope_basis_data().clone(),
                negative: admission.owner_observer().clone(),
            })
        } else {
            None
        };
        let prepared = PreparedColdOwnerAssociation {
            task: self,
            admission,
            association,
        };
        let mut owner = store.lock_cold_owned_v2_current_namespace(admission, self)?;
        let root = store.archive_bootstrap_root.lock().map_err(|_| refused())?;
        owner.handoff_under_archive_root(prepared, root.as_ref().ok_or_else(refused)?)
    }
}

#[cfg(test)]
mod tests;

pub(in crate::store) struct ColdOwnedV2CurrentOwnerGuard<'task, 'writer> {
    owner: owners::ColdOwnedV2CurrentOwnerNamespace<'task, 'writer>,
}
pub(in crate::store) struct ColdOwnedV2CurrentGroupGuard<'task, 'writer> {
    root: MutexGuard<'task, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdOwnedV2CurrentOwnerNamespace<'task, 'writer>,
    registration: &'task PreparedColdV2WriterRegistration,
}
pub(in crate::store) struct ColdV2CurrentProofGroupGuard<'task, 'writer> {
    root: MutexGuard<'task, Option<super::super::super::archive_bootstrap::MountedRoot>>,
    owner: owners::ColdV2CurrentProofOwnerNamespace<'task, 'writer>,
}

macro_rules! common_root {
    ($ty:ident, $admission:ident) => {
        impl $ty<'_, '_> {
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.owner.admission().store()
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
                self.owner.admission().source()
            }
            pub(in crate::store) fn head(&self) -> &Arc<ColdV2GenesisCurrentHead> {
                self.owner.admission().head()
            }
            pub(in crate::store) fn scope_admission(&self) -> &$admission<'_> {
                self.owner.admission()
            }
            pub(in crate::store) fn archive_root(
                &self,
            ) -> &super::super::super::archive_bootstrap::MountedRoot {
                self.root
                    .as_ref()
                    .expect("Root checked by private cold owner guard")
            }
            pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
                self.owner.validate_source_only()
            }
            fn validate_physical(&self) -> Result<(), AppError> {
                self.owner.validate_old_files()?;
                self.head().validate_under_archive_root(
                    self.store(),
                    self.source(),
                    self.archive_root(),
                )?;
                self.store()
                    .validate_mounted_current_layout_under_archive_root(self.archive_root())?;
                crate::store::persistence::require_cold_head_group_directory(self.head())?;
                self.validate_source_only()
            }
        }
    };
}
common_root!(ColdOwnedV2CurrentGroupGuard, ColdOwnedCurrentAdmission);
common_root!(ColdV2CurrentProofGroupGuard, ColdCurrentProofAdmission);
impl<'task, 'writer> ColdOwnedV2CurrentOwnerGuard<'task, 'writer> {
    pub(in crate::store) fn install_cold_writer(
        &mut self,
        writer: &mut PreparedColdV2WriterRegistration,
    ) -> Result<(), AppError> {
        self.owner.install_cold_writer(writer)
    }
    pub(in crate::store) fn lock_archive_root_for_cold_writer(
        mut self,
        registration: &'task PreparedColdV2WriterRegistration,
    ) -> Result<ColdOwnedV2CurrentGroupGuard<'task, 'writer>, AppError> {
        self.owner.require_installed_cold_writer(registration)?;
        let root = self
            .owner
            .admission()
            .store()
            .archive_bootstrap_root
            .lock()
            .map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let guard = ColdOwnedV2CurrentGroupGuard {
            root,
            owner: self.owner,
            registration,
        };
        guard.validate_physical()?;
        Ok(guard)
    }
}
impl ColdOwnedV2CurrentGroupGuard<'_, '_> {
    pub(in crate::store) fn writer_registration(&self) -> &PreparedColdV2WriterRegistration {
        self.registration
    }
}
impl ColdV2CurrentProofGroupGuard<'_, '_> {
    pub(in crate::store) fn require_installed_cold_observation(
        &self,
        observation: &V2WriterObservation,
    ) -> Result<(), AppError> {
        self.owner.require_installed_cold_observation(observation)
    }
}
impl PreparedV2GenesisGroupNamespace {
    pub(in crate::store) fn lock_cold_owned_current_owner_namespace<'task, 'writer>(
        &'task self,
        admission: &'task ColdOwnedCurrentAdmission<'writer>,
    ) -> Result<ColdOwnedV2CurrentOwnerGuard<'task, 'writer>, AppError> {
        Ok(ColdOwnedV2CurrentOwnerGuard {
            owner: admission
                .store()
                .lock_cold_owned_v2_current_namespace(admission, self)?,
        })
    }
    pub(in crate::store) fn lock_cold_current_proof<'task, 'writer>(
        &'task self,
        admission: &'task ColdCurrentProofAdmission<'writer>,
    ) -> Result<ColdV2CurrentProofGroupGuard<'task, 'writer>, AppError> {
        let owner = admission
            .store()
            .lock_cold_v2_current_proof_namespace(admission, self)?;
        let root = admission
            .store()
            .archive_bootstrap_root
            .lock()
            .map_err(|_| refused())?;
        root.as_ref().ok_or_else(refused)?;
        let guard = ColdV2CurrentProofGroupGuard { root, owner };
        guard.validate_physical()?;
        // The opaque latched receipt is mandatory even before Root's final full verifier.
        guard
            .require_installed_cold_observation(admission.writer_receipt()?.writer_observation())?;
        Ok(guard)
    }
}
