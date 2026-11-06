//! Installed-cold-writer binding, paired with Factory's actual claim/proof guards and
//! Root's actual receipt/owning caller. No runnable or native Current is exposed here.
//! An old ColdHead predates writer installation. Only the NEW opaque actual writer receipt
//! may enter this latch, and every positive edge repeats its installed generation proof.
use super::*;
use crate::store::persistence::v2_genesis_activation::cold_writer::ValidatedColdV2CurrentWriter;

// The actual cold metadata debit includes this inline binding. Keep phase changes free of
// a new Box allocation after writer validation or at the owner-binding commit edge.
#[allow(clippy::large_enum_variant)]
pub(super) enum ColdProgress {
    Unbound,
    // Latch BEFORE binder allocation. An error cannot replay writer installation.
    WriterValidated(Arc<ValidatedColdV2CurrentWriter>),
    CurrentBound {
        receipt: Arc<ValidatedColdV2CurrentWriter>,
        binding: V2CurrentBinding,
    },
}

impl ColdV2CurrentPreparation {
    pub(in crate::store) fn current_binding(&self) -> Result<&V2CurrentBinding, AppError> {
        match &self.progress {
            ColdProgress::CurrentBound { binding, .. } => Ok(binding),
            _ => Err(refused()),
        }
    }
    fn writer_receipt(&self) -> Result<&Arc<ValidatedColdV2CurrentWriter>, AppError> {
        match &self.progress {
            ColdProgress::Unbound => Err(refused()),
            ColdProgress::WriterValidated(receipt) | ColdProgress::CurrentBound { receipt, .. } => {
                Ok(receipt)
            }
        }
    }
}

fn require_receipt(
    store: &ServerStore,
    source: &GenesisSourceDataRef<'_>,
    task: &PreparedV2GenesisGroupNamespace,
    preparation: &ColdV2CurrentPreparation,
    receipt: &ValidatedColdV2CurrentWriter,
) -> Result<(), AppError> {
    if !Arc::ptr_eq(receipt.head(), &preparation.head) {
        return Err(refused());
    }
    receipt.require_original_source_negative(
        store,
        source,
        task,
        &preparation.head,
        &preparation.inner.owner_identity,
    )
}

fn owned<'writer>(
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer ColdV2CurrentPreparation,
) -> ColdOwnedCurrentAdmission<'writer> {
    ColdOwnedCurrentAdmission {
        store,
        runtime,
        source,
        task,
        preparation,
        _not_send: PhantomData,
    }
}

/// Read-only existing installed-writer proof role. It is deliberately distinct from the
/// CLEAN install/bind role: a failed unrelated scope write may dirty the SAME latest table
/// after our real receipt was latched. No Unbound preparation can construct this borrower.
/// Its concrete Factory guard proves retained canonical/Root objects, never installs a writer.
pub(in crate::store) struct ColdCurrentProofAdmission<'writer> {
    original: ColdOwnedCurrentAdmission<'writer>,
}
impl std::fmt::Debug for ColdCurrentProofAdmission<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColdCurrentProofAdmission")
            .finish_non_exhaustive()
    }
}
impl ColdCurrentProofAdmission<'_> {
    pub(in crate::store) fn store(&self) -> &ServerStore {
        self.original.store()
    }
    pub(in crate::store) fn source(&self) -> &GenesisSourceDataRef<'_> {
        self.original.source()
    }
    pub(in crate::store) fn task(&self) -> &PreparedV2GenesisGroupNamespace {
        self.original.task()
    }
    pub(in crate::store) fn head(&self) -> &Arc<ColdV2GenesisCurrentHead> {
        self.original.head()
    }
    pub(in crate::store) fn scope_basis_data(&self) -> &ColdV2GenesisScopeBasis {
        self.original.scope_basis_data()
    }
    pub(in crate::store) fn writer_receipt(
        &self,
    ) -> Result<&Arc<ValidatedColdV2CurrentWriter>, AppError> {
        self.original.preparation.writer_receipt()
    }
    pub(in crate::store) fn owner_identity(&self) -> &Arc<()> {
        self.original.owner_identity()
    }
    pub(in crate::store) fn owner_observer(&self) -> &RequestCancellation {
        self.original.owner_observer()
    }
    pub(in crate::store) fn group(&self) -> &[u8] {
        self.original.group()
    }
    pub(in crate::store) fn signer(&self) -> &[u8; 32] {
        self.original.signer()
    }
    pub(in crate::store) fn validate_source_only(&self) -> Result<(), AppError> {
        self.original.validate_source_only()?;
        let receipt = self.writer_receipt()?;
        require_receipt(
            self.store(),
            self.source(),
            self.task(),
            self.original.preparation,
            receipt,
        )
    }
    pub(in crate::store) fn require_retained_current_lineage_for_own_group_task(
        &self,
        task: &PreparedV2GenesisGroupNamespace,
    ) -> Result<(), AppError> {
        self.original.preparation.task.require_same_original(task)?;
        self.validate_source_only()?;
        // This cached exact registered/handoff association intentionally accepts a dirty
        // table; the new role grants only proof reads, not install, bind or scope flush.
        task.require_cold_current_scope_owner(&self.original)
    }
    pub(in crate::store) fn current_filename_association(
        &self,
    ) -> Result<
        crate::store::chat_admission::current_lineage::CurrentFilenameAssociation<'_>,
        AppError,
    > {
        self.validate_source_only()?;
        self.original
            .runtime
            .current_filename_association(self.store())
    }
}

fn proof<'writer>(
    store: &'writer ServerStore,
    runtime: &'writer Runtime,
    source: &'writer GenesisSourceDataRef<'writer>,
    task: &'writer PreparedV2GenesisGroupNamespace,
    preparation: &'writer ColdV2CurrentPreparation,
) -> Result<ColdCurrentProofAdmission<'writer>, AppError> {
    let admission = ColdCurrentProofAdmission {
        original: owned(store, runtime, source, task, preparation),
    };
    admission.require_retained_current_lineage_for_own_group_task(task)?;
    Ok(admission)
}

fn require_binding(
    store: &ServerStore,
    preparation: &ColdV2CurrentPreparation,
    binding: &V2CurrentBinding,
) -> Result<(), AppError> {
    let p = &preparation.inner;
    if binding.inner.key != p.key
        || binding.inner.nonce != p.record.nonce
        || binding.inner.revision != 2
        || binding.inner.domain != p.domain
        || !Arc::ptr_eq(&binding.inner.owner_identity, &p.owner_identity)
        || !Arc::ptr_eq(&binding.inner.mount_identity, &store.chat_mount.identity)
        || binding.reservation_observer().is_cancelled()
    {
        return Err(refused());
    }
    Ok(())
}

impl ServerStore {
    /// Only Root's private actual installed-writer caller may supply these closures. Its
    /// install returns the NEW opaque receipt; verify MUST invoke that receipt's concrete
    /// Factory owned guard validator. No public trait, scalar status or ordinary ColdHead
    /// substitutes for this connected source/installed-generation proof.
    pub(in crate::store) fn finish_cold_v2_genesis_current(
        &self,
        preparation: &mut ColdV2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        install: impl for<'writer> FnOnce(
            &ColdOwnedCurrentAdmission<'writer>,
        )
            -> Result<Arc<ValidatedColdV2CurrentWriter>, AppError>,
        mut verify: impl for<'writer> FnMut(
            &ColdOwnedCurrentAdmission<'writer>,
            &Arc<ValidatedColdV2CurrentWriter>,
        ) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        preparation.require_original_source_negative(self, source, task)?;
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        if matches!(&preparation.progress, ColdProgress::Unbound) {
            let receipt = {
                let admission = owned(self, &runtime, source, task, preparation);
                admission.require_owned_current_lineage_for_own_group_task(task)?;
                install(&admission)?
            };
            if !Arc::ptr_eq(receipt.head(), &preparation.head) {
                return Err(refused());
            }
            // Root allocated this receipt BEFORE its last actual proof. Cache by move,
            // BEFORE further negative checks or binder/observer allocation can refuse.
            // A late cancellation retains this exact terminal attempt, never Unbound.
            preparation.progress = ColdProgress::WriterValidated(receipt);
        }
        let receipt = preparation.writer_receipt()?.clone();
        {
            let admission = owned(self, &runtime, source, task, preparation);
            admission.require_owned_current_lineage_for_own_group_task(task)?;
            require_receipt(self, source, task, preparation, &receipt)?;
            verify(&admission, &receipt)?;
            admission.validate_source_only()?;
        }
        if !matches!(&preparation.progress, ColdProgress::CurrentBound { .. }) {
            let inner = self.bind_owned_chat_admission_locked(
                &runtime,
                &preparation.inner.key,
                preparation.inner.nonce(),
                2,
                &preparation.inner.owner_identity,
            )?;
            let binding = V2CurrentBinding {
                inner,
                writer: receipt.writer_observation().clone(),
            };
            // Allocating the binding can outlive the prior proof. Repeat the SAME actual
            // installed registration/source/head after that work, then only cached negatives.
            let admission = owned(self, &runtime, source, task, preparation);
            admission.require_owned_current_lineage_for_own_group_task(task)?;
            verify(&admission, &receipt)?;
            require_receipt(self, source, task, preparation, &receipt)?;
            require_binding(self, preparation, &binding)?;
            admission.validate_source_only()?;
            preparation.progress = ColdProgress::CurrentBound { receipt, binding };
        } else {
            require_binding(self, preparation, preparation.current_binding()?)?;
            require_receipt(self, source, task, preparation, &receipt)?;
        }
        Ok(())
    }

    /// Existing installed proof only: WriterValidated OR CurrentBound may resync the SAME
    /// dirty latest table with actual temporary credit. Unbound cannot enter, and no install
    /// callback/binder/row revision/new owner exists in this path. Root owns the concrete
    /// proof guard callback; it must call validate_under_proof_guard, not return a status.
    pub(in crate::store) fn resync_cold_v2_genesis_current(
        &self,
        preparation: &ColdV2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        scope_buffers: &CatalogReadReservation,
        rng: &mut impl CryptoRngCore,
        mut verify: impl for<'writer> FnMut(
            &ColdCurrentProofAdmission<'writer>,
            &Arc<ValidatedColdV2CurrentWriter>,
        ) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let receipt = preparation.writer_receipt()?;
        require_buffers(self, scope_buffers)?;
        preparation.require_original_source_negative(self, source, task)?;
        let mut runtime = self.chat_mount.locked()?;
        require_buffers(self, scope_buffers)?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        {
            let admission = proof(self, &runtime, source, task, preparation)?;
            verify(&admission, receipt)?;
            admission.validate_source_only()?;
        }
        if runtime.dirty {
            require_buffers(self, scope_buffers)?;
            {
                // Account validation can wait; actual installed/source proofs follow it.
                let admission = proof(self, &runtime, source, task, preparation)?;
                verify(&admission, receipt)?;
                admission.validate_source_only()?;
            }
            self.flush_chat_admissions(&mut runtime, rng)?;
        }
        let admission = proof(self, &runtime, source, task, preparation)?;
        verify(&admission, receipt)?;
        admission.validate_source_only()
    }

    /// FINAL read-only clean proof after the caller has dropped temporary scope credits and
    /// drained retired resources OUTSIDE writers. Dirty preserves the same node/preparation.
    /// No quota/refund, flush, callback replay, binding allocation or new owner is performed.
    pub(in crate::store) fn validate_cold_v2_genesis_current(
        &self,
        preparation: &ColdV2CurrentPreparation,
        source: &GenesisSourceDataRef<'_>,
        task: &PreparedV2GenesisGroupNamespace,
        verify: impl for<'writer> FnOnce(
            &ColdOwnedCurrentAdmission<'writer>,
            &Arc<ValidatedColdV2CurrentWriter>,
        ) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let binding = preparation.current_binding()?;
        let receipt = preparation.writer_receipt()?;
        preparation.require_original_source_negative(self, source, task)?;
        let runtime = self.chat_mount.locked()?;
        self.require_no_pending_checkpoint_join_group(preparation.inner.group())?;
        let admission = owned(self, &runtime, source, task, preparation);
        admission.require_owned_current_lineage_for_own_group_task(task)?;
        require_receipt(self, source, task, preparation, receipt)?;
        verify(&admission, receipt)?;
        require_binding(self, preparation, binding)?;
        require_receipt(self, source, task, preparation, receipt)?;
        admission.validate_source_only()
    }
}
