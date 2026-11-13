//! Original retained ciphertext -> actual small SourcePrepared save. Root owns the exact
//! cached candidate/seed and all file effects. No parsed counter, supplied frame or bool
//! becomes a saved result, and this child cannot begin a large source-file write.
use super::*;

impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderSourcePreparation<T, R> {
    pub fn save_source_prepared(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        self.save_source_prepared_with_access(store, rng, FounderAccess::Library)
    }
    pub(in crate::store) fn save_source_prepared_with_access(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        access.require(&self.original.creator.origin)?;
        // Complete original cipher+fee custody is a hard predecessor. The registered
        // validator selects its exact current journal state after an interrupted SP save.
        self.register_private_source_with_access(store, rng, access)?;
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
        let plan = self.original.plan.as_ref().ok_or_else(refused)?;
        let captured = &self.original.creator.original;
        let registered = self.registered.as_ref().ok_or_else(refused)?;
        let prepared = &mut self.prepared_journal;
        store.with_original_archive_founder(&source, plan, |admission| {
            let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
            vacancy.validate_transferred_source(captured, context, *id)?;
            if prepared.is_none() {
                // Cache the actual private result BEFORE any subsequent observation can
                // fail. A lost reply retries the same cell/operation, not another source.
                *prepared =
                    Some(store.save_original_source_prepared(captured, &vacancy, registered, rng)?);
            }
            prepared
                .as_ref()
                .ok_or_else(refused)?
                .validate_original(store, captured, &vacancy, registered)
        })?;
        self.original.creator.original.validate_original(store)?;
        if self.original.creator.original.observer().is_cancelled() {
            return Err(refused());
        }
        Ok(ArchiveFounderSourceProgress::SourcePreparedJournalSavedUnexposed)
    }
}

#[cfg(test)]
#[path = "founder_source_prepared_tests.rs"]
mod tests;
