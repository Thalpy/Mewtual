//! Exact original S1 stage promotion, not a generic successor or operation-journal recovery.
use super::*;

impl OwnerHeadWriteGuard<'_> {
    /// The private decoder has already checked the externally pinned chain/own credential.
    /// Retain this exact pending identity before any I/O: a failed rename/parent or Current
    /// covering write must never let a different candidate overwrite uncertain first state.
    pub(super) fn establish_initial_stage_barrier(
        &mut self,
        permit: &ChatAdmissionStoragePermit<'_>,
        source: &ValidatedOwnerSnapshot,
    ) -> Result<SavedOwnerRestore, AppError> {
        if permit.vault_format()? != catcoms_storage::VaultFormat::OwnerEvidence
            || source.location != source::OwnerSourceLocation::InitialStage
            || source.meta.generation != 1
            || self.state.owner.as_ref().and_then(Weak::upgrade).is_some()
            || self
                .state
                .pending
                .as_ref()
                .is_some_and(|known| known != &source.meta)
            || self
                .state
                .current
                .as_ref()
                .is_some_and(|known| known != &source.meta)
        {
            return Err(refused());
        }
        let crate::store::ChatAdmissionResolution::Initializing(preparation) = source.admission()
        else {
            return Err(refused());
        };
        preparation.validate_initialization_attempt(&source.meta.attempt)?;
        self.state.pending = Some(source.meta.clone());
        let store = permit.store();
        if !store.owner_canonical_record_exists(&self.cell.key)? {
            // A duplicate preparation can arrive after another exact retry made Current. It
            // may revalidate an existing canonical file below, but Current never grants stage
            // promotion and AlreadyCurrent is never interpreted as an actual saved barrier.
            if permit.mode() != ChatAdmissionStorageMode::Initializing {
                return Err(refused());
            }
            {
                let (file, raw) = store.read_owner_restore_record(
                    &self.cell.key,
                    source.file_cap,
                    source::OwnerSourceLocation::InitialStage,
                )?;
                let plain = Zeroizing::new(unseal(
                    &store.keys.db_key()?,
                    &crate::store::unframe(&raw)?,
                )?);
                if plain.as_slice() != source.plain.as_slice() {
                    return Err(refused());
                }
                source.validate_original(store)?;
                permit.validate_original(store, source.group(), source.signer())?;
                // Only the actual synchronized opened stage crosses the fixed-name move.
                // No saved-state rewrite, new AEAD nonce, MLS Add or signature is generated.
                store.promote_opened_owner_initial_stage(permit, self, file)?;
            } // Release re-read buffers before the canonical read; one paid scratch set.
        }
        let (file, raw) = store.read_owner_canonical_record(&self.cell.key, source.file_cap)?;
        let plain = Zeroizing::new(unseal(
            &store.keys.db_key()?,
            &crate::store::unframe(&raw)?,
        )?);
        if plain.as_slice() != source.plain.as_slice() {
            return Err(refused());
        }
        // This runs on EVERY retry, including a same original Initializing preparation whose
        // independent row has become Current. Visible identical bytes cannot invent Saved.
        store
            .sync_opened_owner_canonical_record(permit, self, &file)
            .map_err(|error| AppError::CommittedButNotDurable(error.to_string()))?;
        source.validate_original(store)?;
        self.state.current = Some(source.meta.clone());
        self.state.pending = None;
        Ok(SavedOwnerRestore {
            mount: store.registry_mount(),
            head: self.cell.identity.clone(),
            admission_owner: permit.owner_identity().clone(),
            meta: source.meta.clone(),
        })
    }
}
