//! Cleanup of unpublished record-write temporaries, never saved journals or recovery snapshots.
//!
//! `atomic_write` consumes a temporary name by rename before it reports success. Thus, while
//! the mounted store is exclusively borrowed, a canonical staging sibling cannot be an active
//! write or a published record. The caller must keep source history/intents until the final save
//! succeeds; this cleanup is NOT permission to discard those sources or a settlement journal.

use super::inventory::{
    invalid, is_link, regular_file, storage_name, EpochInventoryCoverage, EpochStorageScan,
    RecoveryName, ENTRIES_PER_STEP, MAX_DIRECTORY_ENTRIES,
};
use super::*;

/// Counts from successful batches only. File lengths are observed ciphertext lengths, NOT an
/// estimate of filesystem free space (hard links, sparse files and storage allocation can differ).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EpochStorageCleanupProgress {
    /// Includes ignored final records and legacy names; all traversal work is charged.
    pub visited_entries: usize,
    /// Canonical temporary names unlinked in batches whose directory-sync step succeeded.
    pub removed_files: usize,
    /// Sum of observed ciphertext lengths for those names, including zero-length files.
    pub removed_ciphertext_bytes: u64,
    /// EOF for this traversal only. Directory iteration during deletion may skip entries;
    /// only a new inventory can identify remaining orphans and actual accounting inputs.
    pub complete: bool,
}

/// Exclusive, bounded cleanup pass. There is no caller-supplied path and no automatic cleanup
/// on Drop. Cancellation/errors can leave partial removals; they never roll files back or refund
/// accounting. A new pass must complete its directory sync, even if no siblings remain.
pub struct EpochStorageCleanup<'a> {
    coverage: EpochInventoryCoverage,
    store: &'a mut ServerStore,
    directory: fs::ReadDir,
    parent: PathBuf,
    progress: EpochStorageCleanupProgress,
    failed: bool,
    entry_limit: usize,
}

impl std::fmt::Debug for EpochStorageCleanup<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Do not expose local vault paths or private record identifiers in generic diagnostics.
        f.debug_struct("EpochStorageCleanup")
            .field("coverage", &self.coverage)
            .field("progress", &self.progress)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl ServerStore {
    /// Begin explicit cleanup of this vault's recovery-write staging siblings. This is a local
    /// storage operation, not file retention/P2, recovery eviction, or a network command.
    ///
    /// Only exact canonical temporary names under the fixed `servers` parent are eligible;
    /// even a corrupt final `.recovery` record is never removed. No final destination is required
    /// because a failed first publication can leave a sibling before any final file exists.
    /// The mutable borrow excludes store writers through every step; the mount guard excludes
    /// other conforming app processes. Malicious concurrent local path replacement is out of scope.
    pub fn cleanup_epoch_recovery_staging(&mut self) -> Result<EpochStorageCleanup<'_>, AppError> {
        self.cleanup_epoch_files(EpochInventoryCoverage::RecoveryOnly)
    }

    /// Explicitly remove unpublished recovery AND owner-journal write attempts. Saved pending
    /// owner decisions are final files and are never removed. This is not P2 retention or pruning.
    /// Success alone neither reconciles a budget nor enables network publication.
    pub fn cleanup_epoch_storage_staging(&mut self) -> Result<EpochStorageCleanup<'_>, AppError> {
        self.cleanup_epoch_files(EpochInventoryCoverage::RecoveryAndOwnerReceipts)
    }

    /// Also remove unpublished intent attempts, never final replay instructions. Source edits
    /// must not have been accepted before the final rename and durability barrier succeeded.
    pub fn cleanup_epoch_storage_staging_with_intents(
        &mut self,
    ) -> Result<EpochStorageCleanup<'_>, AppError> {
        self.cleanup_epoch_files(EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents)
    }

    /// Include unpublished registry attempts, never retained source epochs or receipts. A
    /// completed cleanup still requires a fresh inventory before accounting can be reconciled.
    pub fn cleanup_epoch_storage_staging_with_registry(
        &mut self,
    ) -> Result<EpochStorageCleanup<'_>, AppError> {
        self.cleanup_epoch_files(EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsAndRegistry)
    }

    /// Also remove eligible unpublished Studio temporary siblings. Never removes final epochs
    /// or retained recovery; a fresh five-family inventory is mandatory after this pass.
    pub fn cleanup_epoch_storage_staging_with_studio(
        &mut self,
    ) -> Result<EpochStorageCleanup<'_>, AppError> {
        self.studio_generation = std::sync::Arc::new(());
        self.cleanup_epoch_files(
            EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio,
        )
    }

    fn cleanup_epoch_files(
        &mut self,
        coverage: EpochInventoryCoverage,
    ) -> Result<EpochStorageCleanup<'_>, AppError> {
        let parent = self.dir.join("servers");
        let metadata = fs::symlink_metadata(&parent).map_err(|e| AppError::Io(e.to_string()))?;
        if !metadata.is_dir() || is_link(&metadata) {
            return Err(invalid(
                "epoch storage cleanup parent is not a regular directory",
            ));
        }
        let directory = fs::read_dir(&parent).map_err(|e| AppError::Io(e.to_string()))?;
        Ok(EpochStorageCleanup {
            coverage,
            store: self,
            directory,
            parent,
            progress: EpochStorageCleanupProgress::default(),
            failed: false,
            entry_limit: MAX_DIRECTORY_ENTRIES,
        })
    }
}

impl<'a> EpochStorageCleanup<'a> {
    /// Families eligible for this pass. Fixed at creation and preserved by `into_inventory`.
    pub fn coverage(&self) -> EpochInventoryCoverage {
        self.coverage
    }
    /// Visit at most 64 entries, then sync the parent before reporting success (Unix directory
    /// sync; the existing non-Unix primitive is a no-op). Even a zero-removal batch syncs, so a
    /// retry after an uncertain unlink cannot convert absence into unproven free-space credit.
    /// A failed/cancelled pass may already have removed some unpublished siblings. It never
    /// changes a budget: only complete, current inventory reconciliation can release charges.
    pub fn step(&mut self) -> Result<EpochStorageCleanupProgress, AppError> {
        self.step_with_hooks(&mut WriteHooks::None)
    }

    // Tests decide on either side of each unlink and of the directory sync without changing the
    // production ordering and without owning the physical I/O: removal and flush are always the
    // capability's own operations. Poison before any I/O so catch_unwind cannot bypass a failure.
    fn step_with_hooks(
        &mut self,
        hooks: &mut WriteHooks<'_>,
    ) -> Result<EpochStorageCleanupProgress, AppError> {
        if self.failed {
            return Err(invalid("epoch storage cleanup failed; start a new pass"));
        }
        if self.progress.complete {
            return Ok(self.progress);
        }
        self.failed = true;
        // Invalidate all prior intent-budget/scan tokens BEFORE any possible unlink or panic.
        // Even an empty retry must complete syncing and rescan before spending again.
        if self.coverage.includes_intents() {
            self.store.intent_generation = std::sync::Arc::new(());
        }
        // I-4. One capability spans the whole destructive batch, taken before the first possible
        // unlink. That is sound for the same reason a batch of writes is: the guard holds the
        // store exclusively, so no C-3 scan can be captured between the first removal and the
        // parent sync, and a single rotation therefore invalidates every inventory that could
        // have been taken before any of it. Unlinking a temporary sibling is exactly the
        // "unlink or leave a temporary sibling" category I-4 names.
        let mutation = self.store.epoch_mutation_guard();
        let mut next = self.progress;
        for _ in 0..ENTRIES_PER_STEP {
            let Some(entry) = self.directory.next() else {
                next.complete = true;
                break;
            };
            let entry = entry.map_err(|e| AppError::Io(e.to_string()))?;
            next.visited_entries += 1;
            if next.visited_entries > self.entry_limit {
                return Err(invalid("epoch storage cleanup directory limit reached"));
            }
            let Some((_, RecoveryName::Temporary(_))) =
                storage_name(&entry.file_name(), self.coverage)?
            else {
                // Finals (including corrupt ones) and unrelated staging families are not ours
                // to remove. The subsequent inventory, not cleanup, authenticates final records.
                continue;
            };
            let path = self.parent.join(entry.file_name());
            let metadata = fs::symlink_metadata(&path).map_err(|e| AppError::Io(e.to_string()))?;
            if !regular_file(&metadata) {
                return Err(invalid(
                    "epoch storage cleanup candidate is not a regular file",
                ));
            }
            // Check the counter before unlink. Even an artificial huge sparse-file length must
            // not overflow and leave a successful-looking wrapped counter after deletion.
            let removed_bytes = next
                .removed_ciphertext_bytes
                .checked_add(metadata.len())
                .ok_or_else(|| invalid("epoch storage cleanup byte counter overflow"))?;
            hooks.before_unlink(WriteTag::Staging, &path)?;
            mutation.remove_io(&path).map_err(|e| {
                AppError::Io(format!(
                    "epoch staging cleanup: {e}; earlier siblings may already be removed"
                ))
            })?;
            hooks.after_unlink(WriteTag::Staging, &path)?;
            next.removed_files += 1;
            next.removed_ciphertext_bytes = removed_bytes;
        }
        // Must also run at EOF with zero removals: a previous pass may have unlinked everything
        // then failed (or crashed) before its sync. There is no speculative counter refund.
        // A directory sync has no record whose size could be checked, so it reports zero. A
        // refusal here is committed-but-not-durable by construction: siblings are already gone
        // and nothing has made their absence durable.
        hooks
            .before_sync(WriteTag::Staging, &self.parent, 0)
            .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        mutation
            .sync_parent_io(&self.parent)
            .map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
        hooks.after_sync(WriteTag::Staging, &self.parent)?;
        self.progress = next;
        self.failed = false;
        Ok(next)
    }

    /// Start a fresh inventory without releasing exclusive store access. Cleanup EOF is not a
    /// claim that no orphan remains: callers must inspect this scan's completed result, repeat
    /// cleanup if necessary, and inventory other P1 types before composing a whole-server budget.
    pub fn into_inventory(self) -> Result<EpochStorageScan<'a>, AppError> {
        if self.failed || !self.progress.complete {
            return Err(invalid("epoch storage cleanup pass is incomplete"));
        }
        self.store.scan_epoch_files(self.coverage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::EpochRecoveryInventory;

    // `WriteHooks` borrows its decisions rather than owning them, so a helper cannot build one
    // and return it with the closure inside. These take the caller's closure by reference and
    // assemble the rest, which keeps each call site to the decision it is actually making.

    /// Decide before the directory sync only.
    fn on_sync<'h>(f: &'h mut dyn FnMut(WriteTag, &Path, u64) -> AfterIntercept) -> WriteHooks<'h> {
        WriteHooks::Hooked {
            before: None,
            before_sync: Some(f),
            before_unlink: None,
            after: None,
        }
    }

    /// Decide before each unlink only. Used with `panic!` to assert an unlink never happens.
    fn on_unlink<'h>(f: &'h mut dyn FnMut(WriteTag, &Path) -> AfterIntercept) -> WriteHooks<'h> {
        WriteHooks::Hooked {
            before: None,
            before_sync: None,
            before_unlink: Some(f),
            after: None,
        }
    }

    use catcoms_replication::RecoveryReason;
    use catcoms_rt::ManualClock;
    use catcoms_wire::DocType;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn open(path: &Path) -> ServerStore {
        ServerStore::open(path, b"cleanup-test", &mut ChaCha20Rng::seed_from_u64(1)).unwrap()
    }

    fn document() -> LogicalDocument {
        LogicalDocument::new(
            b"group".to_vec(),
            DocType::StudioObject,
            b"private-cat".to_vec(),
        )
        .unwrap()
    }

    fn action(epoch: u64) -> EpochRecoveryAction {
        EpochRecoveryAction::Stage(RecoverySnapshot {
            doc_type: DocType::StudioObject,
            logical_key: document().logical_key,
            epoch,
            base_close_record_hash: None,
            reason: RecoveryReason::Excluded,
            projection: b"saved pixels".to_vec(),
            tombstones: vec![],
            elements: vec![],
            conflicts: vec![],
            applied_ops: vec![],
        })
    }

    fn final_path(store: &ServerStore) -> PathBuf {
        store.epoch_recovery_path(&scope_bytes(7, &document()).unwrap())
    }

    fn inventory(mut scan: EpochStorageScan<'_>) -> EpochRecoveryInventory {
        while !scan.step().unwrap().complete {}
        scan.finish().unwrap()
    }

    fn complete(store: &mut ServerStore) -> (EpochStorageCleanupProgress, EpochRecoveryInventory) {
        let mut job = store.cleanup_epoch_recovery_staging().unwrap();
        let mut before = EpochStorageCleanupProgress::default();
        loop {
            let after = job.step().unwrap();
            assert!(after.visited_entries - before.visited_entries <= ENTRIES_PER_STEP);
            before = after;
            if after.complete {
                break;
            }
        }
        (before, inventory(job.into_inventory().unwrap()))
    }

    #[test]
    fn combined_cleanup_flush_failure_retries_at_eof_without_losing_coverage() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for suffix in ["recovery", "owner-receipts"] {
            let path = root
                .path()
                .join("servers")
                .join(format!("{}.{suffix}", "ab".repeat(32)));
            fs::write(staging_candidate_for_test(&path, 900), b"unpublished").unwrap();
        }
        let mut cleanup = store.cleanup_epoch_storage_staging().unwrap();
        assert_eq!(
            cleanup.coverage(),
            EpochInventoryCoverage::RecoveryAndOwnerReceipts
        );
        let mut refuse = |_: WriteTag, _: &Path, _: u64| {
            AfterIntercept::Fail(AppError::Io("flush failure".into()))
        };
        let result = cleanup.step_with_hooks(&mut on_sync(&mut refuse));
        assert!(matches!(result, Err(AppError::CommittedButNotDurable(_))));
        assert_eq!(cleanup.progress.removed_files, 0); // Failed batches report no success.
        assert!(cleanup.into_inventory().is_err());
        drop(store);
        let mut store = open(root.path());
        let mut cleanup = store.cleanup_epoch_storage_staging().unwrap();
        let mut syncs = 0;
        let mut count = |_: WriteTag, _: &Path, _: u64| {
            syncs += 1;
            AfterIntercept::Continue
        };
        let done = cleanup.step_with_hooks(&mut on_sync(&mut count)).unwrap();
        assert!(done.complete);
        assert_eq!(done.removed_files, 0);
        assert_eq!(syncs, 1); // An empty retry must still make earlier unlinks durable.
        let scan = cleanup.into_inventory().unwrap();
        assert_eq!(
            scan.coverage(),
            EpochInventoryCoverage::RecoveryAndOwnerReceipts
        );
        let result = inventory(scan);
        assert_eq!(
            result.coverage(),
            EpochInventoryCoverage::RecoveryAndOwnerReceipts
        );
        assert_eq!(result.orphans().len(), 0);
    }

    /// I4-003. Unlinking a temporary sibling is a five-family mutation, and the pass that does
    /// it must rotate the inventory generation exactly as a write does.
    ///
    /// One capability spans the whole batch rather than one per removal: the guard holds the
    /// store exclusively, so no cursor can be captured between the first unlink and the parent
    /// sync, and a single rotation therefore invalidates every inventory that could have been
    /// taken before any of it.
    #[test]
    fn a_cleanup_pass_that_removes_a_sibling_rotates_the_inventory_generation() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let orphan = staging_candidate_for_test(&final_path(&store), 800);
        fs::write(&orphan, b"partial").unwrap();

        let before = store.inventory_generation();
        // N17's unlink case, with a real cursor parked across the removal.
        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        store.step_epoch_storage_scan(&mut cursor, 1, None).unwrap();

        let mut job = store.cleanup_epoch_recovery_staging().unwrap();
        while !job.step().unwrap().complete {}
        drop(job);
        assert!(
            !orphan.exists(),
            "the pass removed nothing, so this proves nothing"
        );
        assert!(
            !std::sync::Arc::ptr_eq(&before, &store.inventory_generation()),
            "cleanup unlinked an inventoried temporary sibling without rotating, so an inventory              captured before the pass would still be treated as current"
        );
        assert!(
            store.step_epoch_storage_scan(&mut cursor, 1, None).is_err(),
            "a cursor parked across a cleanup unlink resumed anyway"
        );
        assert!(
            store.finish_epoch_storage_scan(cursor).is_err(),
            "a cursor parked across a cleanup unlink still issued an inventory"
        );
    }

    #[test]
    fn cleanup_removes_only_unpublished_siblings_not_the_three_logical_recovery_slots() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for epoch in 1..=3 {
            store
                .update_epoch_recovery(
                    7,
                    &document(),
                    action(epoch),
                    &ManualClock::new(epoch),
                    &mut ChaCha20Rng::seed_from_u64(epoch),
                )
                .unwrap();
        }
        let final_path = final_path(&store);
        let original = fs::read(&final_path).unwrap();
        let empty = staging_candidate_for_test(&final_path, 800);
        let partial = staging_candidate_for_test(&final_path, 801);
        fs::write(&empty, []).unwrap();
        fs::write(&partial, b"partial ciphertext").unwrap();
        let legacy = staging_candidate_for_test(&root.path().join("servers/7.net"), 802);
        fs::write(&legacy, b"do not remove legacy stages").unwrap();
        let (progress, inventory) = complete(&mut store);
        assert_eq!(progress.removed_files, 2);
        assert_eq!(
            progress.removed_ciphertext_bytes,
            b"partial ciphertext".len() as u64
        );
        assert_eq!(fs::read(&final_path).unwrap(), original);
        assert_eq!(fs::read(legacy).unwrap(), b"do not remove legacy stages");
        assert!(!empty.exists() && !partial.exists());
        let state = store.load_epoch_recovery(7, &document()).unwrap();
        assert_eq!(state.retained().len(), 2);
        assert!(state.staged().is_some());
        assert!(state.eviction_pending().unwrap().is_some());
        assert_eq!(inventory.orphans().len(), 0);
        let records = inventory.records_for_server(7, b"group").unwrap();
        assert!(records[0].footprint.settlement > 0); // Logical staged version still pins reserve.
    }

    #[test]
    fn first_publication_orphan_is_discarded_without_promoting_it_or_touching_its_source() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        // The source remains durable until the recovery save actually succeeds. This fixture's
        // legacy snapshot stands in for the still-unpruned epoch; cleanup cannot target it.
        let source = root.path().join("servers/7.bin");
        fs::write(&source, b"unpruned source history").unwrap();
        let orphan = staging_candidate_for_test(&final_path(&store), 800);
        let result = store.update_epoch_recovery_with_writer(
            7,
            &document(),
            action(1),
            &ManualClock::new(1),
            &mut ChaCha20Rng::seed_from_u64(1),
            &mut WriteHooks::Hooked {
                before: Some(&mut |_: WriteTag, _: &Path, bytes: &[u8]| {
                    fs::write(&orphan, bytes).unwrap();
                    Intercept::Fail(AppError::Io(
                        "simulated interruption before first rename".into(),
                    ))
                }),
                before_sync: None,
                before_unlink: None,
                after: None,
            },
        );
        assert!(result.is_err());
        assert!(!final_path(&store).exists());
        assert_eq!(
            inventory(store.scan_epoch_recovery().unwrap()).unresolved_orphans(),
            1
        );
        drop(store);
        let mut store = open(root.path());
        let (progress, inventory) = complete(&mut store);
        assert_eq!(progress.removed_files, 1);
        assert!(!orphan.exists() && !final_path(&store).exists());
        assert_eq!(inventory.unresolved_orphans(), 0);
        assert_eq!(fs::read(source).unwrap(), b"unpruned source history");
        // Retrying the original durable intent can now publish normally.
        store
            .update_epoch_recovery(
                7,
                &document(),
                action(1),
                &ManualClock::new(1),
                &mut ChaCha20Rng::seed_from_u64(1),
            )
            .unwrap();
        assert_eq!(
            store
                .load_epoch_recovery(7, &document())
                .unwrap()
                .retained()
                .len(),
            1
        );
    }

    #[test]
    fn failed_accounted_write_recovers_through_cleanup_fresh_inventory_and_exact_retry() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let scope = StorageScope::new(7, b"group").unwrap();
        // This isolated fixture contains only this recovery namespace. A production coordinator
        // must compose EVERY managed type, never use recovery-only discovery as its whole budget.
        let mut budget = EpochStorageBudget::from_inventory(scope.clone(), []).unwrap();
        store
            .update_epoch_recovery_accounted(
                7,
                &document(),
                action(1),
                &ManualClock::new(1),
                &mut ChaCha20Rng::seed_from_u64(1),
                &mut budget,
            )
            .unwrap();
        let initial_usage = budget.usage();
        let orphan = staging_candidate_for_test(&final_path(&store), 800);
        let failed = store.update_epoch_recovery_accounted_with_writer(
            7,
            &document(),
            action(2),
            &ManualClock::new(2),
            &mut ChaCha20Rng::seed_from_u64(2),
            &mut budget,
            &mut WriteHooks::Hooked {
                before: Some(&mut |_: WriteTag, _: &Path, bytes: &[u8]| {
                    fs::write(&orphan, bytes).unwrap();
                    Intercept::Fail(AppError::Io("interrupted replacement".into()))
                }),
                before_sync: None,
                before_unlink: None,
                after: None,
            },
        );
        assert!(failed.is_err());
        assert!(budget.requires_reconciliation());
        let with_orphan = inventory(store.scan_epoch_recovery().unwrap());
        budget
            .reconcile(&scope, with_orphan.records_for_server(7, b"group").unwrap())
            .unwrap();
        let charged = budget.usage();
        assert!(charged.settlement > 0);
        let (_, fresh) = complete(&mut store);
        assert_eq!(budget.usage(), charged); // Cleanup never refunds a live budget itself.
        budget
            .reconcile(&scope, fresh.records_for_server(7, b"group").unwrap())
            .unwrap();
        assert_eq!(budget.usage(), initial_usage);
        store
            .update_epoch_recovery_accounted(
                7,
                &document(),
                action(2),
                &ManualClock::new(2),
                &mut ChaCha20Rng::seed_from_u64(2),
                &mut budget,
            )
            .unwrap();
        assert_eq!(
            store
                .load_epoch_recovery(7, &document())
                .unwrap()
                .retained()
                .len(),
            2
        );
        assert!(!budget.requires_reconciliation());
    }

    #[test]
    fn unlinking_a_hardlinked_sibling_preserves_the_published_name_and_data() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        store
            .update_epoch_recovery(
                7,
                &document(),
                action(1),
                &ManualClock::new(1),
                &mut ChaCha20Rng::seed_from_u64(1),
            )
            .unwrap();
        let published = final_path(&store);
        let bytes = fs::read(&published).unwrap();
        let orphan = staging_candidate_for_test(&published, 800);
        fs::hard_link(&published, &orphan).unwrap();
        let (progress, fresh) = complete(&mut store);
        assert_eq!(progress.removed_files, 1);
        assert_eq!(progress.removed_ciphertext_bytes, bytes.len() as u64);
        assert!(!orphan.exists());
        assert_eq!(fs::read(published).unwrap(), bytes);
        assert_eq!(fresh.records().len(), 1);
    }

    #[test]
    fn unlink_then_sync_failure_never_finishes_and_zero_deletion_retry_still_flushes() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let orphan = staging_candidate_for_test(&final_path(&store), 800);
        fs::write(&orphan, b"partial").unwrap();
        let mut job = store.cleanup_epoch_recovery_staging().unwrap();
        let mut refuse = |_: WriteTag, _: &Path, _: u64| {
            AfterIntercept::Fail(AppError::Io("sync failed".into()))
        };
        let failure = job.step_with_hooks(&mut on_sync(&mut refuse));
        assert!(matches!(failure, Err(AppError::CommittedButNotDurable(_))));
        assert!(!orphan.exists());
        assert_eq!(job.progress.removed_files, 0); // No successful progress published.
        assert!(job.step().is_err());
        assert!(job.into_inventory().is_err());
        drop(store);
        let mut store = open(root.path());
        let mut retry = store.cleanup_epoch_recovery_staging().unwrap();
        let mut syncs = 0;
        let mut never = |_: WriteTag, _: &Path| panic!("no sibling remains");
        let mut count = |_: WriteTag, _: &Path, _: u64| {
            syncs += 1;
            AfterIntercept::Continue
        };
        let progress = retry
            .step_with_hooks(&mut WriteHooks::Hooked {
                before: None,
                before_sync: Some(&mut count),
                before_unlink: Some(&mut never),
                after: None,
            })
            .unwrap();
        assert!(progress.complete);
        assert_eq!(progress.removed_files, 0);
        assert_eq!(syncs, 1);
        assert_eq!(
            inventory(retry.into_inventory().unwrap()).orphans().len(),
            0
        );
    }

    #[test]
    fn partial_unlink_failure_and_caught_panic_poison_without_rollback_or_accounting_credit() {
        for panic_after_unlink in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            for id in 800..802 {
                fs::write(
                    staging_candidate_for_test(&final_path(&store), id),
                    b"partial",
                )
                .unwrap();
            }
            let mut job = store.cleanup_epoch_recovery_staging().unwrap();
            let mut calls = 0;
            // The first sibling is removed by the capability's own unlink; the decision only
            // says whether to proceed. The second is refused before it happens, so exactly one
            // removal is real and the retry below must find exactly one left to do.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut proceed_once = |_: WriteTag, _: &Path| {
                    calls += 1;
                    if calls == 1 {
                        AfterIntercept::Continue
                    } else {
                        AfterIntercept::Fail(AppError::Io("unlink failed".into()))
                    }
                };
                let mut interrupt = |op: CompletedOperation, _: WriteTag, _: &Path| {
                    // Specifically after a removal, not after the batch's parent sync.
                    if panic_after_unlink && op == CompletedOperation::Unlink {
                        panic!("interrupted after unlink");
                    }
                    AfterIntercept::Continue
                };
                let mut unreached =
                    |_: WriteTag, _: &Path, _: u64| panic!("failed traversal must not sync");
                job.step_with_hooks(&mut WriteHooks::Hooked {
                    before: None,
                    before_sync: Some(&mut unreached),
                    before_unlink: Some(&mut proceed_once),
                    after: Some(&mut interrupt),
                })
            }));
            if panic_after_unlink {
                assert!(result.is_err());
            } else {
                assert!(result.unwrap().is_err());
            }
            assert_eq!(job.progress.removed_files, 0);
            assert!(job.step().is_err());
            assert!(job.into_inventory().is_err());
            let (retried, _) = complete(&mut store);
            assert_eq!(retried.removed_files, 1);
        }
    }

    #[test]
    fn traversal_limit_counts_legacy_names_and_cancellation_cannot_yield_inventory() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for id in 0..=ENTRIES_PER_STEP {
            fs::write(root.path().join(format!("servers/{id}.bin")), []).unwrap();
        }
        assert!(store
            .cleanup_epoch_recovery_staging()
            .unwrap()
            .into_inventory()
            .is_err());
        let mut job = store.cleanup_epoch_recovery_staging().unwrap();
        job.entry_limit = ENTRIES_PER_STEP;
        let mut syncs = 0;
        let mut never = |_: WriteTag, _: &Path| panic!("legacy must not be unlinked");
        let mut count = |_: WriteTag, _: &Path, _: u64| {
            syncs += 1;
            AfterIntercept::Continue
        };
        let first = job
            .step_with_hooks(&mut WriteHooks::Hooked {
                before: None,
                before_sync: Some(&mut count),
                before_unlink: Some(&mut never),
                after: None,
            })
            .unwrap();
        assert_eq!(first.visited_entries, ENTRIES_PER_STEP);
        assert!(!first.complete);
        assert_eq!(syncs, 1);
        assert!(job.step().is_err());
        assert!(job.into_inventory().is_err());
        assert_eq!(complete(&mut store).0.removed_files, 0);
    }

    #[test]
    fn malformed_aliases_directories_and_corrupt_final_records_are_never_deleted() {
        for kind in ["alias", "malformed", "directory", "final"] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let canonical = staging_candidate_for_test(&final_path(&store), 800);
            let path = match kind {
                "alias" => canonical.with_file_name(
                    canonical
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_ascii_uppercase(),
                ),
                "malformed" => canonical.with_extension("tmp.extra"),
                "final" => final_path(&store),
                _ => canonical,
            };
            if kind == "directory" {
                fs::create_dir(&path).unwrap();
            } else {
                fs::write(&path, b"leave intact").unwrap();
            }
            let mut job = store.cleanup_epoch_recovery_staging().unwrap();
            if kind == "final" {
                while !job.step().unwrap().complete {}
                let mut scan = job.into_inventory().unwrap();
                assert!(scan.step().is_err()); // Cleanup is not authentication or corrupt-file repair.
            } else {
                assert!(job.step().is_err());
                assert!(job.into_inventory().is_err());
            }
            assert!(path.exists(), "{kind}");
        }
    }

    #[test]
    fn byte_counter_overflow_refuses_before_unlink_and_debug_omits_paths() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let orphan = staging_candidate_for_test(&final_path(&store), 800);
        fs::write(&orphan, b"x").unwrap();
        let mut job = store.cleanup_epoch_recovery_staging().unwrap();
        let debug = format!("{job:?}");
        assert!(!debug.contains("private-cat") && !debug.contains("mewtual-stage"));
        assert!(!debug.contains(root.path().to_str().unwrap()));
        job.progress.removed_ciphertext_bytes = u64::MAX;
        let mut never = |_: WriteTag, _: &Path| panic!("overflow must precede unlink");
        assert!(job.step_with_hooks(&mut on_unlink(&mut never)).is_err());
        assert!(orphan.exists());
        assert!(job.into_inventory().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_candidates_and_redirected_parent_are_not_followed_or_removed() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let outside = root.path().join("outside");
        fs::write(&outside, b"must survive").unwrap();
        let orphan = staging_candidate_for_test(&final_path(&store), 800);
        symlink(&outside, &orphan).unwrap();
        let mut job = store.cleanup_epoch_recovery_staging().unwrap();
        assert!(job.step().is_err());
        assert!(job.into_inventory().is_err());
        assert!(fs::symlink_metadata(orphan)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(outside).unwrap(), b"must survive");
        let parent = root.path().join("servers");
        let moved = root.path().join("moved-servers");
        fs::rename(&parent, &moved).unwrap();
        symlink(&moved, &parent).unwrap();
        assert!(store.cleanup_epoch_recovery_staging().is_err());
    }
}
