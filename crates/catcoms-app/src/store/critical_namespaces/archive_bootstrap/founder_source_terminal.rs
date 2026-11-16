//! SourceSaved is a NEW closed phase, not a relaxation of the old arm-current SavedFiles
//! receipt. Retain/lookup its actual prepared closure BEFORE calling any old file consumer;
//! otherwise a lost return after terminal promotion could strand the original owner forever.
use super::*;

impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderSourcePreparation<T, R> {
    pub fn save_source_terminal(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        self.save_source_terminal_with_access(store, rng, FounderAccess::Library)
    }
    pub(in crate::store) fn save_source_terminal_with_access(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
        mut access: FounderAccess<'_>,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        access.require(&self.original.creator.origin)?;
        self.register_private_source_with_access(store, rng, access.reborrow())?;
        {
            let key = Zeroizing::new(store.keys.db_key()?);
            let SourcePhase::Transferred {
                source: bootstrap,
                context,
                id,
            } = &self.phase
            else {
                return Err(refused());
            };
            let source = ActualFounderSource {
                creator: &self.original.creator,
                phase: ActualSourcePhase::Transferred {
                    source: bootstrap,
                    key: &key,
                    context,
                    id: *id,
                },
            };
            let captured = &self.original.creator.original;
            let registered = self.registered.as_ref().ok_or_else(refused)?;
            let prepared = &mut self.prepared_terminal;
            store.with_original_archive_founder(
                &source,
                self.original.plan.as_ref().ok_or_else(refused)?,
                |admission| {
                    let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
                    if prepared.is_none() {
                        *prepared = store
                            .lookup_original_source_terminal(captured, &vacancy, registered)?;
                    }
                    Ok(())
                },
            )?;
        }
        if self.prepared_terminal.is_none() {
            self.save_source_files_with_access(store, rng, access.reborrow())?;
            let key = Zeroizing::new(store.keys.db_key()?);
            let SourcePhase::Transferred {
                source: bootstrap,
                context,
                id,
            } = &self.phase
            else {
                return Err(refused());
            };
            let source = ActualFounderSource {
                creator: &self.original.creator,
                phase: ActualSourcePhase::Transferred {
                    source: bootstrap,
                    key: &key,
                    context,
                    id: *id,
                },
            };
            let captured = &self.original.creator.original;
            let registered = self.registered.as_ref().ok_or_else(refused)?;
            let files = self.saved_files.as_ref().ok_or_else(refused)?;
            let prepared = &mut self.prepared_terminal;
            store.with_original_archive_founder(
                &source,
                self.original.plan.as_ref().ok_or_else(refused)?,
                |admission| {
                    let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
                    // Actual Root preparation caches its witness before any phase change and
                    // has no fallible work after the move. Refusal preserves the old arm/files.
                    *prepared =
                        Some(store.prepare_original_source_terminal(
                            captured, &vacancy, registered, files,
                        )?);
                    Ok(())
                },
            )?;
        }
        {
            let key = Zeroizing::new(store.keys.db_key()?);
            let SourcePhase::Transferred {
                source: bootstrap,
                context,
                id,
            } = &self.phase
            else {
                return Err(refused());
            };
            let source = ActualFounderSource {
                creator: &self.original.creator,
                phase: ActualSourcePhase::Transferred {
                    source: bootstrap,
                    key: &key,
                    context,
                    id: *id,
                },
            };
            let captured = &self.original.creator.original;
            let registered = self.registered.as_ref().ok_or_else(refused)?;
            let prepared = self.prepared_terminal.as_ref().ok_or_else(refused)?;
            let saved = &mut self.saved_terminal;
            store.with_original_archive_founder(
                &source,
                self.original.plan.as_ref().ok_or_else(refused)?,
                |admission| {
                    let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
                    if saved.is_none() {
                        *saved = Some(store.save_original_source_terminal(
                            captured, &vacancy, registered, prepared, rng,
                        )?);
                    }
                    saved
                        .as_ref()
                        .ok_or_else(refused)?
                        .validate_original(store, captured, &vacancy, registered)
                },
            )?;
        }
        self.original.creator.original.validate_original(store)?;
        if self.original.creator.original.observer().is_cancelled() {
            return Err(refused());
        }
        Ok(ArchiveFounderSourceProgress::SourceSavedUnexposed)
    }
}

#[cfg(test)]
#[path = "founder_source_terminal_tests.rs"]
mod tests;
