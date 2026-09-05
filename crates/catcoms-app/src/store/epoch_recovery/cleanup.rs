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
        self.step_with_io(|path| fs::remove_file(path), sync_directory)
    }

    // Private seams inject failures on both sides of unlink and directory sync without changing
    // the production ordering. Poison before any I/O so catch_unwind cannot bypass a failure.
    fn step_with_io(
        &mut self,
        mut unlink: impl FnMut(&Path) -> std::io::Result<()>,
        mut sync: impl FnMut(&Path) -> std::io::Result<()>,
    ) -> Result<EpochStorageCleanupProgress, AppError> {
        if self.failed {
            return Err(invalid("epoch storage cleanup failed; start a new pass"));
        }
        if self.progress.complete {
            return Ok(self.progress);
        }
        self.failed = true;
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
            unlink(&path).map_err(|e| {
                AppError::Io(format!(
                    "epoch staging cleanup: {e}; earlier siblings may already be removed"
                ))
            })?;
            next.removed_files += 1;
            next.removed_ciphertext_bytes = removed_bytes;
        }
        // Must also run at EOF with zero removals: a previous pass may have unlinked everything
        // then failed (or crashed) before its sync. There is no speculative counter refund.
        sync(&self.parent).map_err(|e| AppError::CommittedButNotDurable(e.to_string()))?;
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
            fs::write(staging_candidate(&path, 900), b"unpublished").unwrap();
        }
        let mut cleanup = store.cleanup_epoch_storage_staging().unwrap();
        assert_eq!(
            cleanup.coverage(),
            EpochInventoryCoverage::RecoveryAndOwnerReceipts
        );
        let result = cleanup.step_with_io(
            |path| fs::remove_file(path),
            |_| Err(std::io::Error::other("flush failure")),
        );
        assert!(matches!(result, Err(AppError::CommittedButNotDurable(_))));
        assert_eq!(cleanup.progress.removed_files, 0); // Failed batches report no success.
        assert!(cleanup.into_inventory().is_err());
        drop(store);
        let mut store = open(root.path());
        let mut cleanup = store.cleanup_epoch_storage_staging().unwrap();
        let mut syncs = 0;
        let done = cleanup
            .step_with_io(
                |path| fs::remove_file(path),
                |_| {
                    syncs += 1;
                    Ok(())
                },
            )
            .unwrap();
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
        let empty = staging_candidate(&final_path, 800);
        let partial = staging_candidate(&final_path, 801);
        fs::write(&empty, []).unwrap();
        fs::write(&partial, b"partial ciphertext").unwrap();
        let legacy = staging_candidate(&root.path().join("servers/7.net"), 802);
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
        let orphan = staging_candidate(&final_path(&store), 800);
        let result = store.update_epoch_recovery_with_writer(
            7,
            &document(),
            action(1),
            &ManualClock::new(1),
            &mut ChaCha20Rng::seed_from_u64(1),
            |_, bytes| {
                fs::write(&orphan, bytes).unwrap();
                Err(AppError::Io(
                    "simulated interruption before first rename".into(),
                ))
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
        let orphan = staging_candidate(&final_path(&store), 800);
        let failed = store.update_epoch_recovery_accounted_with_writer(
            7,
            &document(),
            action(2),
            &ManualClock::new(2),
            &mut ChaCha20Rng::seed_from_u64(2),
            &mut budget,
            |_, bytes| {
                fs::write(&orphan, bytes).unwrap();
                Err(AppError::Io("interrupted replacement".into()))
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
        let orphan = staging_candidate(&published, 800);
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
        let orphan = staging_candidate(&final_path(&store), 800);
        fs::write(&orphan, b"partial").unwrap();
        let mut job = store.cleanup_epoch_recovery_staging().unwrap();
        let failure = job.step_with_io(
            |p| fs::remove_file(p),
            |_| Err(std::io::Error::other("sync failed")),
        );
        assert!(matches!(failure, Err(AppError::CommittedButNotDurable(_))));
        assert!(!orphan.exists());
        assert_eq!(job.progress.removed_files, 0); // No successful progress published.
        assert!(job.step().is_err());
        assert!(job.into_inventory().is_err());
        drop(store);
        let mut store = open(root.path());
        let mut retry = store.cleanup_epoch_recovery_staging().unwrap();
        let mut syncs = 0;
        let progress = retry
            .step_with_io(
                |_| panic!("no sibling remains"),
                |_| {
                    syncs += 1;
                    Ok(())
                },
            )
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
                fs::write(staging_candidate(&final_path(&store), id), b"partial").unwrap();
            }
            let mut job = store.cleanup_epoch_recovery_staging().unwrap();
            let mut calls = 0;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                job.step_with_io(
                    |path| {
                        calls += 1;
                        if calls == 1 {
                            fs::remove_file(path)?;
                            if panic_after_unlink {
                                panic!("interrupted after unlink");
                            }
                            Ok(())
                        } else {
                            Err(std::io::Error::other("unlink failed"))
                        }
                    },
                    |_| panic!("failed traversal must not report a synced batch"),
                )
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
        let first = job
            .step_with_io(
                |_| panic!("legacy must not be unlinked"),
                |_| {
                    syncs += 1;
                    Ok(())
                },
            )
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
            let canonical = staging_candidate(&final_path(&store), 800);
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
        let orphan = staging_candidate(&final_path(&store), 800);
        fs::write(&orphan, b"x").unwrap();
        let mut job = store.cleanup_epoch_recovery_staging().unwrap();
        let debug = format!("{job:?}");
        assert!(!debug.contains("private-cat") && !debug.contains("mewtual-stage"));
        assert!(!debug.contains(root.path().to_str().unwrap()));
        job.progress.removed_ciphertext_bytes = u64::MAX;
        assert!(job
            .step_with_io(|_| panic!("overflow must precede unlink"), |_| Ok(()))
            .is_err());
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
        let orphan = staging_candidate(&final_path(&store), 800);
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
