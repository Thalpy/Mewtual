//! Private first-Add overlap capacity. This reserves no new logical head: the original actual
//! writer already owns its current/replacement allowance. Retained lower responses keep their
//! fee after the actor finishes, so dropping a waiter cannot prematurely refund live bytes.
use super::*;

struct OperationFee {
    pool: Weak<Mutex<Runtime>>,
    bytes: usize,
}
impl std::fmt::Debug for OperationFee {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerOperationFee")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}
impl Drop for OperationFee {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            if let Ok(mut pool) = pool.lock() {
                if let Some(bytes) = pool.hot_bytes.checked_sub(self.bytes) {
                    pool.hot_bytes = bytes;
                }
            }
        }
    }
}

impl OperationFee {
    fn into_response(fee: Arc<Self>) -> Result<catcoms_rt::SharedRequestKeepalive, AppError> {
        let mut fee = Arc::try_unwrap(fee).map_err(|_| refused())?;
        let pool = fee.pool.upgrade().ok_or_else(refused)?;
        let resized = (|| {
            let mut runtime = pool.lock().map_err(|_| refused())?;
            let target = catcoms_sync::owner_evidence::OWNER_FIRST_ADD_RESPONSE_PRIVATE_BYTES;
            let budget = runtime.budget.ok_or_else(refused)?;
            if target > fee.bytes {
                return Err(refused());
            }
            let hot = runtime
                .hot_bytes
                .checked_sub(fee.bytes)
                .and_then(|n| n.checked_add(target))
                .filter(|n| *n <= budget.hot_bytes)
                .ok_or_else(refused)?;
            // Compute ALL checked values before changing either side under the SAME pool.
            runtime.hot_bytes = hot;
            fee.bytes = target;
            Ok(())
        })();
        // Release the pool guard even on failure BEFORE fee.Drop returns its old allowance.
        resized?;
        Ok(Arc::new(fee))
    }
}

/// Source/candidate/frozen buffers must be fields BEFORE this fee in their owning transaction.
/// No constructor accepts paid=true, current=true or a caller-created writer identity.
pub(crate) struct OwnerOperationReservation {
    limits: OwnerInitializationSnapshotLimits,
    fee: Arc<OperationFee>,
    mount: Arc<()>,
    negative: RequestCancellation,
    writer: Arc<()>,
    expected: head::Meta,
}
impl OwnerOperationReservation {
    pub(crate) fn limits(&self) -> OwnerInitializationSnapshotLimits {
        self.limits
    }
    /// Resource phase transfer ONLY. Caller drops private candidate/frozen payloads outside
    /// admission/head first; active Server memory has a separate, still-open host admission.
    /// Uniqueness proves no lower copy still owns the old private allowance. Original scope
    /// and effect authority are never reconstructed/renewed by changing this accounting value.
    pub(crate) fn into_response_custody(
        self,
    ) -> Result<catcoms_rt::SharedRequestKeepalive, AppError> {
        OperationFee::into_response(self.fee)
    }
    pub(crate) fn validate_original(
        &self,
        permit: &ChatAdmissionStoragePermit<'_>,
        writer: &OwnerWriterRegistration,
        expected: &head::Meta,
    ) -> Result<(), AppError> {
        if self.negative.is_cancelled()
            || writer.publication_authority().is_cancelled()
            || !Arc::ptr_eq(&self.mount, &permit.store().registry_mount())
            || !Arc::ptr_eq(&self.writer, &writer.identity())
            || &self.expected != expected
            || permit.vault_format()? != catcoms_storage::VaultFormat::OwnerOperations
        {
            return Err(refused());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

impl ServerStore {
    /// Called under the ORIGINAL Current admission, before decoding/cloning the bounded
    /// request or any candidate SDK/signing work. Lock order remains admission -> pool -> head.
    /// The result is retained outside both locks; its Drop can safely return the hot allowance.
    pub(crate) fn reserve_owner_operation(
        &self,
        permit: &ChatAdmissionStoragePermit<'_>,
        writer: &OwnerWriterRegistration,
        binding: &ChatAdmissionBinding,
        expected: &head::Meta,
    ) -> Result<OwnerOperationReservation, AppError> {
        if permit.vault_format()? != catcoms_storage::VaultFormat::OwnerOperations
            || expected.generation != 1
        {
            return Err(refused());
        }
        permit.validate_original(self, binding.group(), binding.signer())?;
        if self.owner_canonical_stage_exists(&key(binding.group(), binding.signer()))? {
            return Err(refused());
        }
        let mut pool = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        let budget = pool.budget.ok_or_else(refused)?;
        let metadata = catcoms_sync::owner_evidence::OWNER_FIRST_ADD_MAX_METADATA;
        if budget
            .mls
            .checked_add(metadata)
            .is_none_or(|n| n > budget.snapshot)
        {
            return Err(refused());
        }
        let bytes = budget
            .buffer_fee()?
            .checked_add(catcoms_sync::owner_evidence::OWNER_FIRST_ADD_EXTRA_PRIVATE_BYTES)
            .ok_or_else(refused)?;
        let hot = pool
            .hot_bytes
            .checked_add(bytes)
            .filter(|n| *n <= budget.hot_bytes)
            .ok_or_else(refused)?;
        writer.with_saved_current_head(permit, binding, expected, || ())?;
        pool.hot_bytes = hot;
        Ok(OwnerOperationReservation {
            limits: OwnerInitializationSnapshotLimits {
                max_snapshot_bytes: budget.snapshot,
                max_mls_bytes: budget.mls,
                max_mls_backing_bytes: budget.mls_backing,
            },
            fee: Arc::new(OperationFee {
                pool: Arc::downgrade(&self.owner_evidence.canonical),
                bytes,
            }),
            mount: self.registry_mount(),
            negative: self.chat_mount.observer(),
            writer: writer.identity(),
            expected: expected.clone(),
        })
    }

    /// One exact retained-result retry, under that invocation's original current head. This
    /// acquires capacity only; the same captured native effect still gates actual driver send.
    pub(crate) fn reserve_owner_response(
        &self,
        permit: &ChatAdmissionStoragePermit<'_>,
        writer: &OwnerWriterRegistration,
        binding: &ChatAdmissionBinding,
        expected: &head::Meta,
    ) -> Result<catcoms_rt::SharedRequestKeepalive, AppError> {
        if permit.vault_format()? != catcoms_storage::VaultFormat::OwnerOperations
            || expected.generation != 2
        {
            return Err(refused());
        }
        permit.validate_original(self, binding.group(), binding.signer())?;
        let mut pool = self
            .owner_evidence
            .canonical
            .lock()
            .map_err(|_| refused())?;
        let budget = pool.budget.ok_or_else(refused)?;
        let bytes = catcoms_sync::owner_evidence::OWNER_FIRST_ADD_RESPONSE_PRIVATE_BYTES;
        let hot = pool
            .hot_bytes
            .checked_add(bytes)
            .filter(|n| *n <= budget.hot_bytes)
            .ok_or_else(refused)?;
        writer.with_saved_current_head(permit, binding, expected, || ())?;
        pool.hot_bytes = hot;
        Ok(Arc::new(OperationFee {
            pool: Arc::downgrade(&self.owner_evidence.canonical),
            bytes,
        }))
    }
}
