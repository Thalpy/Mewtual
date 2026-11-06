//! Distinct cold Initializing1 writer. Only the NEW actual covering scope1 role can
//! install it; read-only repaired Head/current2 roles prove the SAME installed generation.
//! No warm Saved receipt, decoded phase or ordinary Current2 preparation enters this family.
use super::*;
use crate::store::chat_admission::v2_current::{
    ColdV2InitializingPreparation, DurableColdGenesisInitializingAdmission,
};
use crate::store::owner_evidence::canonical::current_vacancy::cold_initializing::{
    ColdInitializingV2GenesisReadNamespace, ColdRepairedV2GenesisCurrentNamespace,
    ColdRepairedV2GenesisCurrentProofNamespace, DurableColdInitializingV2GenesisOwnerNamespace,
};
use crate::store::persistence::v2_genesis_activation::initializing_repair::ColdV2GenesisRepairSeed;
use catcoms_sync::local_chat_scope::ManagedChatScopeReservation;

/// An independently paid original registration, not an installed-writer proof. The caller
/// retains it until its node has drained; Drop closes its signal before resource cleanup.
pub(in crate::store) struct PreparedColdV2InitializingWriterRegistration {
    retired: Option<Slot>,
    expansion: Option<SlotBuffer>,
    // Resource cleanup only. An even-clean scope1 failure can precede the FIRST installer;
    // it must not discard the sole prepaid backing needed by this same original retry.
    expansion_redundant: bool,
    stamp: ScopeStamp,
    task: OriginalV2GenesisGroupObservation,
    seed: Arc<ColdV2GenesisRepairSeed>,
    mount: Arc<()>,
    observation: V2WriterObservation,
    life: Arc<WriterLife>,
    // LAST, including old slot/observation weak control blocks and watch metadata.
    _fee: Arc<CatalogReadReservation>,
}
impl std::fmt::Debug for PreparedColdV2InitializingWriterRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedColdV2InitializingWriterRegistration")
            .finish_non_exhaustive()
    }
}
impl Drop for PreparedColdV2InitializingWriterRegistration {
    fn drop(&mut self) {
        self.cancel_original();
    }
}
fn check_initializing_writer_invoice() -> Result<(), AppError> {
    let named = std::mem::size_of::<PreparedColdV2InitializingWriterRegistration>()
        + std::mem::size_of::<ScopeStamp>()
        + MAX_GROUP
        + std::mem::size_of::<WriterLife>()
        + std::mem::size_of::<AccountedAuthority>()
        + std::mem::size_of::<ObservationAccounting>()
        + std::mem::size_of::<V2WriterObservation>()
        + std::mem::size_of::<Slot>()
        + std::mem::size_of::<OriginalV2GenesisGroupObservation>()
        + 16 * 64
        + 1024;
    if named > WRITER_METADATA_FEE {
        return Err(refused());
    }
    Ok(())
}
impl PreparedColdV2InitializingWriterRegistration {
    pub(in crate::store) fn cancel_original(&self) {
        self.life.sender.send_replace(true);
    }

    /// Outside ALL host writers: independently pay before allocating any setup/map/watch
    /// backing. Only this exact actual mounted Initializing1 preparation supplies the original owner.
    pub(in crate::store) fn prepare(
        store: &ServerStore,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        scope: &ColdV2InitializingPreparation,
    ) -> Result<Self, AppError> {
        scope.require_original_source_negative(store, source, task)?;
        check_initializing_writer_invoice()?;
        let pool = &store
            .archive_bootstrap_policy
            .as_ref()
            .ok_or_else(refused)?
            .pending
            .pool;
        let fee = Arc::new(pool.try_reserve(WRITER_METADATA_FEE)?);
        let needs_buffer = store
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?
            .v2_writers
            .buffer
            .is_none();
        let expansion = if needs_buffer {
            Some(SlotBuffer::prepare(pool)?)
        } else {
            None
        };
        let stamp = ScopeStamp {
            group: scope.group().to_vec(),
            signer: *scope.signer(),
            domain: scope.domain(),
            nonce: scope.nonce(),
            owner_identity: scope.owner_identity(),
        };
        let (sender, _) = watch::channel(false);
        let life = Arc::new(WriterLife {
            identity: Arc::new(()),
            sender,
            #[cfg(test)]
            drop_notice: None,
        });
        let observation = V2WriterObservation {
            identity: life.identity.clone(),
            original_life: Arc::downgrade(&life),
            cancellation: RequestCancellation::new(
                life.sender.subscribe(),
                Some(Arc::new(ObservationAccounting { _fee: fee.clone() })),
            )
            .with_authority(Arc::new(AccountedAuthority {
                mount: store.chat_mount.observer(),
                scope: scope.reservation_observer(),
                _fee: fee.clone(),
            })),
        };
        // Map/authority allocations may wait. Their completion supplies no fresh ownership.
        scope.require_original_source_negative(store, source, task)?;
        Ok(Self {
            retired: None,
            expansion,
            expansion_redundant: false,
            stamp,
            task: task.observe_original(),
            seed: scope.seed().clone(),
            mount: store.registry_mount(),
            observation,
            life,
            _fee: fee,
        })
    }

    fn require_initializing(
        &self,
        admission: &DurableColdGenesisInitializingAdmission<'_>,
    ) -> Result<(), AppError> {
        admission.validate_source_only()?;
        admission.scope_permit().validate_original(
            admission.store(),
            admission.group(),
            admission.signer(),
        )?;
        self.task.require_same_original(admission.task())?;
        self.seed
            .retains_original(admission.store(), admission.source())?;
        if !Arc::ptr_eq(&self.seed, admission.seed())
            || !Arc::ptr_eq(&self.mount, &admission.store().registry_mount())
            || !self
                .observation
                .original_life
                .ptr_eq(&Arc::downgrade(&self.life))
            || self.stamp.group != admission.group()
            || &self.stamp.signer != admission.signer()
            || self.stamp.domain != admission.owner_admission().domain()
            || self.stamp.nonce != admission.owner_admission().nonce()
            || !Arc::ptr_eq(
                &self.stamp.owner_identity,
                admission.owner_admission().owner_identity(),
            )
            || self.observation.is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(in crate::store) fn observe_original(&self) -> V2WriterObservation {
        self.observation.clone()
    }
    pub(in crate::store) fn require_same_observation(
        &self,
        other: &V2WriterObservation,
    ) -> Result<(), AppError> {
        if !Arc::ptr_eq(&self.observation.identity, &other.identity)
            || !self.observation.original_life.ptr_eq(&other.original_life)
            || self.observation.is_cancelled()
            || other.is_cancelled()
        {
            return Err(refused());
        }
        Ok(())
    }
    /// Drain only after actual admission/pins/canonical/Root guards have all gone. A retired
    /// weak generation's final fee or unused map backing must never refund under a writer.
    pub(in crate::store) fn drain_unused_outside_writers(&mut self) {
        drop(self.retired.take());
        if self.expansion_redundant {
            drop(self.expansion.take());
        }
    }
}

impl DurableColdInitializingV2GenesisOwnerNamespace<'_, '_> {
    pub(in crate::store) fn install_initializing_writer(
        &mut self,
        prepared: &mut PreparedColdV2InitializingWriterRegistration,
    ) -> Result<(), AppError> {
        // Registered-only ownership is sufficient to COMPLETE handoff, never to install.
        // Otherwise receipt -> failed bind -> another group's dirty write could make both
        // clean handoff and dirty existing-writer resync impossible for the same attempt.
        self.require_completed_initializing_handoff()?;
        let (admission, runtime) = self.initializing_registration_parts();
        prepared.require_initializing(admission)?;
        admission.require_initializing_lineage_for_own_group_task(admission.task())?;
        let identifier = key(admission.group(), admission.signer());
        if runtime.buffer.is_none() {
            if prepared.expansion.is_none() {
                return Err(refused());
            }
            prepared.require_initializing(admission)?;
            runtime.buffer = prepared.expansion.take();
        }
        let slots = &mut runtime.buffer.as_mut().ok_or_else(refused)?.slots;
        match slots.binary_search_by_key(&identifier, |slot| slot.key) {
            Ok(index) => {
                if classify_original_owner(&slots[index], &prepared.life)?
                    == ExistingOwner::SameOriginal
                {
                    prepared.expansion_redundant = true;
                    return Ok(());
                }
                if prepared.retired.is_some() {
                    return Err(refused());
                }
                prepared.require_initializing(admission)?;
                let next = Slot {
                    key: identifier,
                    owner: Arc::downgrade(&prepared.life),
                    _fee: prepared._fee.clone(),
                };
                prepared.retired = Some(std::mem::replace(&mut slots[index], next));
            }
            Err(index) => {
                if slots.len() >= MAX_WRITERS || slots.len() == slots.capacity() {
                    return Err(refused());
                }
                prepared.require_initializing(admission)?;
                slots.insert(
                    index,
                    Slot {
                        key: identifier,
                        owner: Arc::downgrade(&prepared.life),
                        _fee: prepared._fee.clone(),
                    },
                );
            }
        }
        prepared.expansion_redundant = true;
        prepared.require_initializing(admission)
    }
    pub(in crate::store) fn require_installed_initializing_writer(
        &mut self,
        prepared: &PreparedColdV2InitializingWriterRegistration,
    ) -> Result<(), AppError> {
        self.require_completed_initializing_handoff()?;
        let (admission, runtime) = self.initializing_registration_parts();
        prepared.require_initializing(admission)?;
        runtime.require_installed_observation(
            &key(admission.group(), admission.signer()),
            &prepared.observation,
        )
    }
}

macro_rules! installed_proof {
    ($namespace:ident, $lineage:ident) => {
        impl $namespace<'_, '_> {
            /// Read-only exact original generation, also valid for SAME dirty row2. No
            /// upgrade, mutable map, insertion/replacement or positive role is exported.
            pub(in crate::store) fn require_installed_initializing_observation(
                &self,
                observation: &V2WriterObservation,
            ) -> Result<(), AppError> {
                let (admission, runtime) = self.initializing_proof_registration_parts();
                admission.validate_source_only()?;
                admission.$lineage(admission.task())?;
                let original = admission.head().writer_observation();
                if !original.original_life.ptr_eq(&observation.original_life)
                    || !Arc::ptr_eq(&original.identity, &observation.identity)
                    || original.is_cancelled()
                    || observation.is_cancelled()
                {
                    return Err(refused());
                }
                runtime.require_installed_observation(
                    &key(admission.group(), admission.signer()),
                    observation,
                )
            }
        }
    };
}
installed_proof!(
    ColdInitializingV2GenesisReadNamespace,
    require_initializing_lineage_for_own_group_task
);
installed_proof!(
    ColdRepairedV2GenesisCurrentProofNamespace,
    require_retained_current_lineage_for_own_group_task
);
installed_proof!(
    ColdRepairedV2GenesisCurrentNamespace,
    require_current_lineage_for_own_group_task
);
