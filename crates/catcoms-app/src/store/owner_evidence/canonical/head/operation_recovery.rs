//! Exact retained successor settlement shared by original in-process retry and cold restore.
//! All authentication precedes mutation; unknown stage/parent bytes remain preserved.
use super::*;

fn equals_parts(bytes: &[u8], parts: &[&[u8]]) -> bool {
    let mut offset = 0usize;
    for part in parts {
        let Some(end) = offset.checked_add(part.len()) else {
            return false;
        };
        if bytes.get(offset..end) != Some(*part) {
            return false;
        }
        offset = end;
    }
    offset == bytes.len()
}
fn head_hash(bytes: &[u8]) -> [u8; 32] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"catcoms/owner-head-id/v1\0");
    hash.update(bytes);
    *hash.finalize().as_bytes()
}
impl OwnerHeadWriteGuard<'_> {
    /// Return false ONLY for an exact original parent with no stage: the same-process source
    /// may write its already-frozen candidate. Cold recovery requires true, never new sealing.
    pub(super) fn settle_operation_record(
        &self,
        permit: &ChatAdmissionStoragePermit<'_>,
        parent: &Meta,
        candidate: &Meta,
        parts: &[&[u8]],
        cap: usize,
    ) -> Result<bool, AppError> {
        if permit.mode() != ChatAdmissionStorageMode::Current
            || permit.vault_format()? != catcoms_storage::VaultFormat::OwnerOperations
            || candidate.generation != 2
        {
            return Err(refused());
        }
        let store = permit.store();
        let canonical_is_candidate = {
            let (file, raw) = store.read_owner_canonical_record(&self.cell.key, cap)?;
            let plain = Zeroizing::new(unseal(
                &store.keys.db_key()?,
                &crate::store::unframe(&raw)?,
            )?);
            let same = equals_parts(&plain, parts);
            if !same && (parent.generation != 1 || head_hash(&plain) != parent.head) {
                return Err(refused());
            }
            // Prove the actual original parent file barrier BEFORE replacing it, or establish
            // the candidate barrier BEFORE discarding its exact duplicate stage.
            store.sync_opened_owner_canonical_record(permit, self, &file)?;
            same
        }; // All raw/cipher/plain reread scratch drops before opening the second source.
        if store.owner_canonical_stage_exists(&self.cell.key)? {
            let (file, raw) = store.read_owner_restore_record(
                &self.cell.key,
                cap,
                source::OwnerSourceLocation::OperationStage,
            )?;
            let plain = Zeroizing::new(unseal(
                &store.keys.db_key()?,
                &crate::store::unframe(&raw)?,
            )?);
            if !equals_parts(&plain, parts) {
                return Err(refused());
            }
            permit.validate_original(store, permit.group(), permit.signer())?;
            store.settle_opened_owner_operation_stage(
                permit,
                self,
                file,
                canonical_is_candidate,
            )?;
        } else if !canonical_is_candidate {
            return Ok(false);
        }
        let (file, raw) = store.read_owner_canonical_record(&self.cell.key, cap)?;
        let plain = Zeroizing::new(unseal(
            &store.keys.db_key()?,
            &crate::store::unframe(&raw)?,
        )?);
        if !equals_parts(&plain, parts) {
            return Err(refused());
        }
        store.sync_opened_owner_canonical_record(permit, self, &file)?;
        Ok(true)
    }

    pub(super) fn establish_operation_stage_barrier(
        &mut self,
        permit: &ChatAdmissionStoragePermit<'_>,
        source: &ValidatedOwnerSnapshot,
    ) -> Result<SavedOwnerRestore, AppError> {
        let parent = source.operation_canonical.as_ref().ok_or_else(refused)?;
        if source.location != source::OwnerSourceLocation::OperationStage
            || self.state.owner.as_ref().and_then(Weak::upgrade).is_some()
            || self
                .state
                .current
                .as_ref()
                .is_some_and(|known| known != parent && known != &source.meta)
            || self
                .state
                .pending
                .as_ref()
                .is_some_and(|known| known != &source.meta)
            || !matches!(
                source.admission(),
                crate::store::ChatAdmissionResolution::Current(_)
            )
        {
            return Err(refused());
        }
        self.state.pending = Some(source.meta.clone());
        if !self.settle_operation_record(
            permit,
            parent,
            &source.meta,
            &[&source.plain],
            source.file_cap,
        )? {
            // A captured stage disappearing while only its old parent remains grants nothing.
            return Err(refused());
        }
        source.validate_original(permit.store())?;
        self.state.current = Some(source.meta.clone());
        self.state.pending = None;
        Ok(SavedOwnerRestore {
            mount: permit.store().registry_mount(),
            head: self.cell.identity.clone(),
            admission_owner: permit.owner_identity().clone(),
            meta: source.meta.clone(),
        })
    }
}
