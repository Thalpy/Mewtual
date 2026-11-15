//! Scope durability for activation-seed settlement, separate from dirty DATA maintenance.
//!
//! The cleanup owner and receipts come only from the actual mounted Current family. A
//! successful covering write persists the SAME latest table EVEN CLEAN, then retains its
//! admission serializer through the concrete pins -> canonical -> Root cleanup callback.
//! No row, owner, binding, source allowance, nonce or phase is created or advanced here.
//! This is paired with the actual physical worker and settled cold reader; it supplies
//! neither ordinary message authoring nor native activation.
use super::*;
use crate::store::chat_admission::current_lineage::CurrentFilenameAssociation;
use crate::store::persistence::v2_genesis_activation::settlement::{
    PreparedGenesisSeedSettlement, SavedGenesisSeedSettlement,
};
use crate::store::persistence::GenesisCurrentOwnerRef;

/// Only the Current implementation family constructs the owning loan. These are its actual
/// already-bound scope borrowers, never decoded roles or a caller's replacement owner.
enum ReadScope<'writer> {
    Warm(V2CurrentAdmission<'writer>),
    Cold(ColdOwnedCurrentAdmission<'writer>),
    Repaired(ColdRepairedGenesisCurrentAdmission<'writer>),
}

/// Brief read-only Root verification before/after the scope write. It is not a covering
/// durability receipt. Its private role retains the real Runtime under the same admission
/// writer, and its Root verifier must release Root before returning to the scope serializer.
pub(in crate::store) struct GenesisCurrent2SettlementReadAdmission<'writer> {
    owner: &'writer GenesisCurrentOwnerRef<'writer>,
    runtime: &'writer Runtime,
    scope: ReadScope<'writer>,
}

/// Constructed ONLY after the even-clean write and its post-write full physical proof.
/// The HRTB callback cannot detach this borrow from the continuously held admission writer.
pub(in crate::store) struct DurableGenesisCurrent2SettlementAdmission<'writer> {
    read: GenesisCurrent2SettlementReadAdmission<'writer>,
}

/// Distinct seed-independent final read. An actual saved cleanup receipt selects this role;
/// no optional-seed fallback is added to the old warm/cold mandatory-seed validators.
pub(in crate::store) struct SettledGenesisCurrent2ReadAdmission<'writer> {
    read: GenesisCurrent2SettlementReadAdmission<'writer>,
    settlement: &'writer Arc<SavedGenesisSeedSettlement>,
}

macro_rules! private_debug {
    ($($ty:ident),+ $(,)?) => {$ (
        impl std::fmt::Debug for $ty<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($ty)).finish_non_exhaustive()
            }
        }
    )+};
}
private_debug!(
    GenesisCurrent2SettlementReadAdmission,
    DurableGenesisCurrent2SettlementAdmission,
    SettledGenesisCurrent2ReadAdmission,
);

/// Checked by the NEW settlement metadata invoice before any setup allocation. Existing
/// owner/read/source/SDK/writer payments remain live independently. These borrowers allocate
/// no new owner/watch/key; the32MiB scope encoding/sealing/write overlap is paid separately.
pub(in crate::store) const GENESIS_SEED_SETTLEMENT_SCOPE_METADATA_BYTES: usize =
    std::mem::size_of::<ReadScope<'static>>()
        + std::mem::size_of::<GenesisCurrent2SettlementReadAdmission<'static>>()
        + std::mem::size_of::<DurableGenesisCurrent2SettlementAdmission<'static>>()
        + std::mem::size_of::<SettledGenesisCurrent2ReadAdmission<'static>>()
        + std::mem::size_of::<CurrentFilenameAssociation<'static>>();

impl<'writer> GenesisCurrent2SettlementReadAdmission<'writer> {
    pub(in crate::store) fn owner_ref(&self) -> &GenesisCurrentOwnerRef<'writer> {
        self.owner
    }
    pub(in crate::store) fn warm_admission(&self) -> Option<&V2CurrentAdmission<'writer>> {
        match &self.scope {
            ReadScope::Warm(admission) => Some(admission),
            ReadScope::Cold(_) | ReadScope::Repaired(_) => None,
        }
    }
    pub(in crate::store) fn cold_admission(&self) -> Option<&ColdOwnedCurrentAdmission<'writer>> {
        match &self.scope {
            ReadScope::Cold(admission) => Some(admission),
            ReadScope::Warm(_) | ReadScope::Repaired(_) => None,
        }
    }
    pub(in crate::store) fn repaired_admission(
        &self,
    ) -> Option<&ColdRepairedGenesisCurrentAdmission<'writer>> {
        match &self.scope {
            ReadScope::Repaired(admission) => Some(admission),
            ReadScope::Warm(_) | ReadScope::Cold(_) => None,
        }
    }
    pub(in crate::store) fn store(&self) -> &ServerStore {
        match &self.scope {
            ReadScope::Warm(admission) => admission.store(),
            ReadScope::Cold(admission) => admission.store(),
            ReadScope::Repaired(admission) => admission.store(),
        }
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'writer> {
        self.owner.source()
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.owner.task()
    }
    fn binding(&self) -> Result<&V2CurrentBinding, AppError> {
        match &self.scope {
            ReadScope::Warm(admission) => admission.preparation.current_binding(),
            ReadScope::Cold(_) => self
                .owner
                .cold_scope()
                .ok_or_else(refused)?
                .current_binding(),
            ReadScope::Repaired(_) => self
                .owner
                .repaired_scope()
                .ok_or_else(refused)?
                .current_binding(),
        }
    }
    pub(in crate::store) fn writer_observation(&self) -> Result<&V2WriterObservation, AppError> {
        Ok(&self.binding()?.writer)
    }
    // Only the Windows physical-settlement tests exercise this observation.
    #[cfg(all(test, any(windows, target_os = "linux")))]
    pub(in crate::store) fn owner_observer(&self) -> &RequestCancellation {
        match &self.scope {
            ReadScope::Warm(admission) => &admission.preparation.inner.admission,
            ReadScope::Cold(admission) => admission.owner_observer(),
            ReadScope::Repaired(admission) => admission.owner_admission().owner_observer(),
        }
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        match &self.scope {
            ReadScope::Warm(admission) => admission.group(),
            ReadScope::Cold(admission) => admission.group(),
            ReadScope::Repaired(admission) => admission.group(),
        }
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        match &self.scope {
            ReadScope::Warm(admission) => admission.signer(),
            ReadScope::Cold(admission) => admission.signer(),
            ReadScope::Repaired(admission) => admission.signer(),
        }
    }
    /// Closed cached negatives only: no lock, file/pool operation, SDK or allocating owner
    /// setup. Mandatory-seed physical validators intentionally do NOT occur here: cleanup
    /// may already have unlinked that exact seed, with the retained prepared phase selecting
    /// its independent proof. The physical verifier still proves selected artifacts in full.
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        match &self.scope {
            ReadScope::Warm(admission) => {
                admission.require_current_lineage_for_own_group_task(self.task())?;
            }
            ReadScope::Cold(admission) => {
                admission.require_owned_current_lineage_for_own_group_task(self.task())?;
            }
            ReadScope::Repaired(admission) => {
                admission.require_current_lineage_for_own_group_task(self.task())?;
            }
        };
        if self.runtime.dirty {
            return Err(refused());
        }
        require_binding(self.store(), self.runtime, self.binding()?)?;
        self.owner
            .require_original_negative(self.store(), self.writer_observation()?)
    }
}

fn require_binding(
    store: &ServerStore,
    runtime: &Runtime,
    binding: &V2CurrentBinding,
) -> Result<(), AppError> {
    let b = &binding.inner;
    let row = runtime
        .table
        .as_ref()
        .ok_or_else(refused)?
        .get(&b.key)
        .ok_or_else(refused)?;
    let owner = runtime.owners.get(&b.key).ok_or_else(refused)?;
    if runtime.dirty
        || row.phase != Phase::Current
        || row.revision != 2
        || row.nonce != b.nonce
        || b.revision != 2
        || b.domain != store.chat_vault_domain()?
        || !Arc::ptr_eq(&b.mount_identity, &store.chat_mount.identity)
        || !Arc::ptr_eq(&b.owner_identity, &owner.life.identity())
        || !owner.life.live()
        || owner.nonce != row.nonce
        || owner.revision != row.revision
        || binding.reservation_observer().is_cancelled()
    {
        return Err(refused());
    }
    Ok(())
}

pub(super) fn read<'writer>(
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    owner: &'writer GenesisCurrentOwnerRef<'writer>,
) -> Result<GenesisCurrent2SettlementReadAdmission<'writer>, AppError> {
    require_base(store, runtime)?;
    if runtime.dirty {
        // DATA maintenance may settle previously authorized debt first, but it never
        // substitutes for this new even-clean covering write or renews a cancelled owner.
        return Err(refused());
    }
    let scope = match (
        owner.warm_scope(),
        owner.cold_scope(),
        owner.repaired_scope(),
    ) {
        (Some(preparation), None, None) => ReadScope::Warm(V2CurrentAdmission {
            store,
            runtime,
            source: owner.source(),
            task: owner.task(),
            preparation,
            _not_send: PhantomData,
        }),
        (None, Some(preparation), None) => ReadScope::Cold(preparation.borrow_current2_admission(
            store,
            runtime,
            owner.source(),
            owner.task(),
        )?),
        (None, None, Some(preparation)) => ReadScope::Repaired(
            preparation.borrow_current2_admission(store, runtime, owner.source(), owner.task())?,
        ),
        _ => return Err(refused()),
    };
    let admission = GenesisCurrent2SettlementReadAdmission {
        owner,
        runtime,
        scope,
    };
    admission.validate_source_only()?;
    Ok(admission)
}

macro_rules! delegate_read {
    ($ty:ident) => {
        impl<'writer> $ty<'writer> {
            pub(in crate::store) fn store(&self) -> &ServerStore {
                self.read.store()
            }
            pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'writer> {
                self.read.source()
            }
            pub(in crate::store) fn owner_ref(&self) -> &GenesisCurrentOwnerRef<'writer> {
                self.read.owner_ref()
            }
        }
    };
}
delegate_read!(DurableGenesisCurrent2SettlementAdmission);
delegate_read!(SettledGenesisCurrent2ReadAdmission);

impl<'writer> DurableGenesisCurrent2SettlementAdmission<'writer> {
    pub(in crate::store) fn read_admission(
        &self,
    ) -> &GenesisCurrent2SettlementReadAdmission<'writer> {
        &self.read
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.read.validate_source_only()
    }
}
impl<'writer> SettledGenesisCurrent2ReadAdmission<'writer> {
    pub(in crate::store) fn read_admission(
        &self,
    ) -> &GenesisCurrent2SettlementReadAdmission<'writer> {
        &self.read
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.settlement
            .require_same_owner(self.read.store(), self.read.owner_ref())?;
        self.read.validate_source_only()
    }
}

impl ServerStore {
    /// One explicit covering write with the original actual prepaid scope buffers. Even a
    /// loaded CLEAN inventory is written and synced: cold authentication does not prove a
    /// fresh file/parent barrier. No automatic retry, persistent grant or source-unit refund
    /// is created. The SAME AD stays held from table selection through the callback.
    pub(in crate::store) fn with_durable_genesis_current2_settlement<T>(
        &self,
        owner: &GenesisCurrentOwnerRef<'_>,
        prepared: &mut PreparedGenesisSeedSettlement,
        scope_buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
        callback: impl for<'writer> FnOnce(
            &DurableGenesisCurrent2SettlementAdmission<'writer>,
            &mut PreparedGenesisSeedSettlement,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        require_buffers(self, scope_buffers)?;
        prepared.require_same_owner(self, owner)?;
        let mut runtime = self.chat_mount.locked()?;
        require_buffers(self, scope_buffers)?;
        self.require_no_pending_checkpoint_join_group(
            owner.source().data().context.spec_data().group,
        )?;
        {
            let admission = read(self, &runtime, owner)?;
            prepared.validate_under_scope_admission(&admission)?;
            admission.validate_source_only()?;
        }
        // The verifier's function-local Root guard MUST have dropped. Serialize exactly
        // the latest loaded table, without cached image overwrite or a dirty-only shortcut.
        self.flush_chat_admissions(&mut runtime, rng)?;
        require_buffers(self, scope_buffers)?;
        let admission = read(self, &runtime, owner)?;
        prepared.validate_under_scope_admission(&admission)?;
        admission.validate_source_only()?;
        let durable = DurableGenesisCurrent2SettlementAdmission { read: admission };
        let result = callback(&durable, prepared)?;
        // Cleanup may now be past unlink. Do not call an old seed-present proof or acquire
        // refundable buffers here. The worker caches its actual prepared/saved phase before
        // fallible return; only same-owner and closed cached negatives follow this callback.
        prepared.require_same_owner(self, owner)?;
        durable.validate_source_only()?;
        Ok(result)
    }

    /// Seed-independent final proof lane after caller-owned temporary credits have drained.
    /// A dirty latest table refuses; maintenance plus a NEW covering write remains separate.
    /// The private callback may perform only the concrete saved-settlement Root proof.
    pub(in crate::store) fn with_settled_genesis_current2_read<T>(
        &self,
        owner: &GenesisCurrentOwnerRef<'_>,
        settlement: &Arc<SavedGenesisSeedSettlement>,
        callback: impl for<'writer> FnOnce(
            &SettledGenesisCurrent2ReadAdmission<'writer>,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        settlement.require_same_owner(self, owner)?;
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(
            owner.source().data().context.spec_data().group,
        )?;
        let admission = SettledGenesisCurrent2ReadAdmission {
            read: read(self, &runtime, owner)?,
            settlement,
        };
        settlement.validate_under_scope_admission(&admission)?;
        admission.validate_source_only()?;
        let result = callback(&admission)?;
        // The callback may wait in pins/canonical/Root. Nothing it returns substitutes for
        // the selected saved closure; reprove it after those waits, then only negatives.
        settlement.validate_under_scope_admission(&admission)?;
        admission.validate_source_only()?;
        Ok(result)
    }
}
