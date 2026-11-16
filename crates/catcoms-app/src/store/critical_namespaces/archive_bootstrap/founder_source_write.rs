//! Synchronous original warm source consumer. A genuine Work is fully prepared before the
//! persistent I/O debit, then runs AFTER the entire admission closure/guards drain. Native
//! scheduling, cold negative repair and the later Current transaction are separate consumers.
use super::*;
use crate::store::persistence::OriginalSourceWriteWork;

impl<T: MeshTransport, R: CryptoRngCore> ArchiveFounderSourcePreparation<T, R> {
    pub fn save_source_files(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        self.save_source_files_with_access(store, rng, FounderAccess::Library)
    }
    pub(in crate::store) fn save_source_files_with_access(
        &mut self,
        store: &ServerStore,
        rng: &mut dyn CryptoRngCore,
        access: FounderAccess<'_>,
    ) -> Result<ArchiveFounderSourceProgress, AppError> {
        access.require(&self.original.creator.origin)?;
        self.save_source_prepared_with_access(store, rng, access)?;
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
        let plan = self.original.plan.as_ref().ok_or_else(refused)?;
        let registered = self.registered.as_ref().ok_or_else(refused)?;
        let saved = &mut self.saved_files;
        let mut work: Option<OriginalSourceWriteWork> = None;
        store.with_original_archive_founder(&source, plan, |admission| {
            let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
            if let Some(saved) = saved.as_ref() {
                return saved.validate_original(store, captured, &vacancy, registered);
            }
            *saved =
                store.completed_original_bootstrap_source_write(captured, &vacancy, registered)?;
            if saved.is_none() {
                work = Some(store.prepare_original_bootstrap_source_write(
                    captured, &vacancy, registered, rng,
                )?);
            }
            Ok(())
        })?;
        if let Some(work) = work {
            #[cfg(test)]
            work.set_before_io(self.before_source_io.take());
            // No admission, pin, canonical, Root, registry or source-cell guard is held.
            // Errors retain the complete Store source and SAME output objects; the actual
            // allowance remains spent. The physical session loan outlives the blocking pass.
            work.run()?;
        }
        store.with_original_archive_founder(&source, plan, |admission| {
            let vacancy = store.lock_archive_vacant_owner_namespace(admission)?;
            if saved.is_none() {
                *saved = store
                    .completed_original_bootstrap_source_write(captured, &vacancy, registered)?;
            }
            saved
                .as_ref()
                .ok_or_else(refused)?
                .validate_original(store, captured, &vacancy, registered)
        })?;
        captured.validate_original(store)?;
        if captured.observer().is_cancelled() {
            return Err(refused());
        }
        Ok(ArchiveFounderSourceProgress::SourceFilesSavedUnexposed)
    }
}

#[cfg(test)]
#[path = "founder_source_write_tests.rs"]
mod tests;
