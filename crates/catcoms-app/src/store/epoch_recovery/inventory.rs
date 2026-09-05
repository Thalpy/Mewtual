//! Bounded discovery of the recovery namespace, not a complete P1 storage inventory.
//!
//! A scan borrows the mounted store exclusively across steps. It never repairs, promotes or
//! removes a file. Bodies are authenticated one at a time and discarded; only bounded metadata
//! survives. The completed result is a point-in-time observation, not a continuing write lease.

use std::collections::BTreeMap;

use super::*;
use crate::store::epoch_budget::MAX_ACCOUNTED_RECORDS;
use catcoms_wire::DocType;

// Count ignored legacy names too: a flat directory with hostile clutter must not make one step
// scan indefinitely. These are local discovery rails, not replicated admission rules.
pub(super) const MAX_DIRECTORY_ENTRIES: usize = 2 * MAX_ACCOUNTED_RECORDS;
pub(super) const ENTRIES_PER_STEP: usize = 64;
// One authenticated body per step; its byte ceiling is the addressed reader's existing cap.
// This also bounds aggregate authentication work, independently of caller scheduling.
const MAX_AUTHENTICATED_BYTES: u64 = MAX_ACCOUNTED_RECORDS as u64 * MAX_SEALED_BYTES as u64;

/// Counts only. Progress is not evidence that an incomplete inventory is safe to spend against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecoveryScanProgress {
    /// Includes non-recovery entries skipped during traversal.
    pub visited_entries: usize,
    /// Successfully authenticated canonical final records.
    pub recovery_records: usize,
    /// Canonically named staging siblings, including empty or partial files.
    pub orphan_files: usize,
    /// Physical ciphertext bytes read and authenticated (no orphan bodies are read).
    pub authenticated_bytes: u64,
    /// True only after reaching directory EOF without any error or exhausted rail.
    pub complete: bool,
}

/// Authenticated attribution and physical accounting for one final recovery file.
pub struct RecoveryInventoryEntry {
    /// Local server mount id inside the authenticated record.
    pub server: u64,
    /// Full group/type/key scope inside that record, independent of the current server registry.
    pub document: LogicalDocument,
    /// Exact observed physical footprint, including vault framing and staged-slot bytes.
    pub record: StorageRecord,
}

impl std::fmt::Debug for RecoveryInventoryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryInventoryEntry")
            .field("footprint", &self.record.footprint)
            .finish_non_exhaustive()
    }
}

/// Observed staging metadata. Neither the filename nor the bytes authorize a promotion/deletion.
pub struct RecoveryOrphan {
    name: String,
    destination: [u8; 32],
    bytes: u64,
}

impl RecoveryOrphan {
    /// Canonical basename, never an arbitrary path. A future cleanup operation must revalidate
    /// the actual file under exclusive store access, not act on a stale inventory result alone.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Physical length; zero is a real entry, not absence.
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl std::fmt::Debug for RecoveryOrphan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryOrphan")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

/// Complete discovery of recovery files only. Other P1 files, blobs and legacy server snapshots
/// are NOT inventoried here. After releasing the scan guard, writes can make this view stale.
/// It deliberately has no public constructor, including `Default`; completion requires scan EOF.
///
/// ```compile_fail
/// use catcoms_app::store::EpochRecoveryInventory;
/// let unverified_empty = EpochRecoveryInventory::default();
/// ```
pub struct EpochRecoveryInventory {
    records: BTreeMap<[u8; 32], RecoveryInventoryEntry>,
    orphans: BTreeMap<String, RecoveryOrphan>,
}

impl std::fmt::Debug for EpochRecoveryInventory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochRecoveryInventory")
            .field("records", &self.records.len())
            .field("orphans", &self.orphans.len())
            .field("unresolved_orphans", &self.unresolved_orphans())
            .finish_non_exhaustive()
    }
}

impl EpochRecoveryInventory {
    fn empty() -> Self {
        Self {
            records: BTreeMap::new(),
            orphans: BTreeMap::new(),
        }
    }

    /// Authenticated final entries in deterministic storage-key order. No recovery content.
    pub fn records(&self) -> impl ExactSizeIterator<Item = &RecoveryInventoryEntry> {
        self.records.values()
    }

    /// Staging metadata in deterministic basename order, including unresolved ownership.
    pub fn orphans(&self) -> impl ExactSizeIterator<Item = &RecoveryOrphan> {
        self.orphans.values()
    }

    /// An empty/partial temporary may precede its destination's first successful publication.
    /// Without an authenticated final record we cannot assign that file to any server safely.
    pub fn unresolved_orphans(&self) -> usize {
        self.orphans
            .values()
            .filter(|orphan| !self.records.contains_key(&orphan.destination))
            .count()
    }

    /// Recovery-namespace inputs for a future complete server inventory. Refuses if ANY orphan
    /// is unresolved: an unknown file might belong to this server, even when others do not.
    /// Known temporaries are charged wholly as settlement scratch to the verified destination.
    /// This does not construct a budget: callers must still inventory every other managed type
    /// and exclude writes across composition. Multiple documents' orphans can pin conflicting
    /// reserves; cleanup must resolve that, never pretend the temporary bytes are absent.
    pub fn records_for_server(
        &self,
        server: u64,
        group: &[u8],
    ) -> Result<Vec<StorageRecord>, AppError> {
        StorageScope::new(server, group).map_err(invalid)?;
        if self.unresolved_orphans() != 0 {
            return Err(invalid("unresolved recovery staging ownership"));
        }
        let belongs = |entry: &RecoveryInventoryEntry| {
            entry.server == server && entry.document.server_id == group
        };
        let mut result: Vec<_> = self
            .records
            .values()
            .filter(|e| belongs(e))
            .map(|e| e.record)
            .collect();
        for orphan in self.orphans.values() {
            let entry = &self.records[&orphan.destination];
            if belongs(entry) {
                let mut e = Encoder::new();
                e.put_bytes(b"catcoms/epoch-recovery-temp-inventory/v1")
                    .expect("constant fits");
                e.put_bytes(orphan.name.as_bytes())
                    .expect("bounded basename fits");
                result.push(StorageRecord {
                    id: *blake3::hash(&e.finish()).as_bytes(),
                    document: entry.record.document,
                    footprint: Footprint {
                        settlement: orphan.bytes,
                        ..Footprint::default()
                    },
                });
            }
        }
        Ok(result)
    }
}

/// Exclusive, incrementally scheduled inventory job. Dropping it cancels discovery; neither
/// cancellation nor error exposes a partial inventory. No store mutation is performed.
pub struct EpochRecoveryScan<'a> {
    store: &'a mut ServerStore,
    directory: fs::ReadDir,
    inventory: EpochRecoveryInventory,
    progress: RecoveryScanProgress,
    failed: bool,
    entry_limit: usize,
    record_limit: usize,
    byte_limit: u64,
}

impl std::fmt::Debug for EpochRecoveryScan<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochRecoveryScan")
            .field("progress", &self.progress)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl ServerStore {
    /// Begin a nonrecursive scan of the reserved recovery filename family. The exclusive borrow
    /// lasts across ALL steps, excluding conforming writers; the mount lock excludes a second app
    /// process. A malicious local process replacing directories concurrently is out of scope.
    pub fn scan_epoch_recovery(&mut self) -> Result<EpochRecoveryScan<'_>, AppError> {
        let path = self.dir.join("servers");
        let metadata = fs::symlink_metadata(&path).map_err(|e| AppError::Io(e.to_string()))?;
        if !metadata.is_dir() || is_link(&metadata) {
            return Err(invalid(
                "recovery inventory parent is not a regular directory",
            ));
        }
        let directory = fs::read_dir(path).map_err(|e| AppError::Io(e.to_string()))?;
        Ok(EpochRecoveryScan {
            store: self,
            directory,
            inventory: EpochRecoveryInventory::empty(),
            progress: RecoveryScanProgress::default(),
            failed: false,
            entry_limit: MAX_DIRECTORY_ENTRIES,
            record_limit: MAX_ACCOUNTED_RECORDS,
            byte_limit: MAX_AUTHENTICATED_BYTES,
        })
    }
}

impl EpochRecoveryScan<'_> {
    /// Visit at most 64 directory entries and authenticate at most one bounded record. Schedule
    /// another step when `complete` is false. An error permanently poisons this scan, including
    /// errors caused by local corruption, disappearance, aliases or resource exhaustion.
    pub fn step(&mut self) -> Result<RecoveryScanProgress, AppError> {
        self.guarded_step(Self::step_inner)
    }

    fn guarded_step(
        &mut self,
        work: impl FnOnce(&mut Self) -> Result<RecoveryScanProgress, AppError>,
    ) -> Result<RecoveryScanProgress, AppError> {
        if self.failed {
            return Err(invalid("recovery inventory scan failed; restart required"));
        }
        // Poison BEFORE traversing/parsing: even a caught parser panic must not let a caller
        // resume beyond the offending entry and declare an incomplete inventory complete.
        self.failed = true;
        let result = work(self);
        if result.is_ok() {
            self.failed = false;
        }
        result
    }

    fn step_inner(&mut self) -> Result<RecoveryScanProgress, AppError> {
        if self.progress.complete {
            return Ok(self.progress);
        }
        for _ in 0..ENTRIES_PER_STEP {
            let Some(entry) = self.directory.next() else {
                self.progress.complete = true;
                break;
            };
            let entry = entry.map_err(|e| AppError::Io(e.to_string()))?;
            self.progress.visited_entries += 1;
            if self.progress.visited_entries > self.entry_limit {
                return Err(invalid("recovery inventory directory limit reached"));
            }
            let name = entry.file_name();
            let Some(kind) = recovery_name(&name)? else {
                continue;
            };
            if self.inventory.records.len() + self.inventory.orphans.len() >= self.record_limit {
                return Err(invalid("recovery inventory record limit reached"));
            }
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|e| AppError::Io(e.to_string()))?;
            if !regular_file(&metadata) {
                return Err(invalid("recovery inventory entry is not a regular file"));
            }
            match kind {
                RecoveryName::Final(hash) => {
                    // The shared reader independently caps the opened file. Include actual bytes
                    // in the aggregate check as well, in case the metadata changed before open.
                    let peak = self
                        .progress
                        .authenticated_bytes
                        .checked_add(metadata.len())
                        .ok_or_else(|| invalid("recovery inventory byte limit reached"))?;
                    if peak > self.byte_limit {
                        return Err(invalid("recovery inventory byte limit reached"));
                    }
                    let AuthenticatedRecoveryBytes {
                        plain,
                        physical_bytes: size,
                    } = self
                        .store
                        .read_epoch_recovery_plain(&entry.path())?
                        .ok_or_else(|| invalid("recovery record disappeared during inventory"))?;
                    self.progress.authenticated_bytes = self
                        .progress
                        .authenticated_bytes
                        .checked_add(size)
                        .filter(|bytes| *bytes <= self.byte_limit)
                        .ok_or_else(|| invalid("recovery inventory byte limit reached"))?;
                    let mut d = Decoder::new(&plain);
                    let scope = d.get_bytes().map_err(invalid)?;
                    let (server, document) = decode_scope(scope)?;
                    if blake3::hash(scope).as_bytes() != &hash {
                        return Err(invalid(
                            "recovery filename does not match its authenticated scope",
                        ));
                    }
                    let state = EpochRecoveryState::decode(&plain, scope, &document)?;
                    let record = recovery_record(scope, state.footprint(size)?);
                    if self
                        .inventory
                        .records
                        .insert(
                            hash,
                            RecoveryInventoryEntry {
                                server,
                                document,
                                record,
                            },
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate recovery inventory entry"));
                    }
                    self.progress.recovery_records += 1;
                    break;
                }
                RecoveryName::Temporary(destination) => {
                    // No body is parsed: it may be empty or half-written. Attribution occurs only
                    // through an authenticated destination, after the whole directory was seen.
                    let name = name
                        .into_string()
                        .map_err(|_| invalid("non-UTF-8 recovery filename"))?;
                    if self
                        .inventory
                        .orphans
                        .insert(
                            name.clone(),
                            RecoveryOrphan {
                                name,
                                destination,
                                bytes: metadata.len(),
                            },
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate recovery staging entry"));
                    }
                    self.progress.orphan_files += 1;
                }
            }
        }
        Ok(self.progress)
    }

    /// Consume only a successful EOF result. A completed result is metadata, not a storage lease;
    /// future callers must keep the coordinator exclusive until the complete budget is installed.
    pub fn finish(self) -> Result<EpochRecoveryInventory, AppError> {
        if self.failed || !self.progress.complete {
            return Err(invalid("recovery inventory is incomplete"));
        }
        Ok(self.inventory)
    }
}

fn decode_scope(scope: &[u8]) -> Result<(u64, LogicalDocument), AppError> {
    if scope.len() > 501 {
        return Err(invalid("recovery inventory scope exceeds its bound"));
    }
    let mut d = Decoder::new(scope);
    if d.get_bytes().map_err(invalid)? != RECORD_DOMAIN {
        return Err(invalid("unknown recovery inventory scope domain"));
    }
    let server = d.get_u64().map_err(invalid)?;
    let group = d.get_bytes().map_err(invalid)?;
    let kind = DocType::from_tag(d.get_u16().map_err(invalid)?)
        .ok_or_else(|| invalid("unknown recovery inventory document type"))?;
    let key = d.get_bytes().map_err(invalid)?;
    d.finish().map_err(invalid)?;
    let document = LogicalDocument::new(group.to_vec(), kind, key.to_vec()).map_err(invalid)?;
    if scope_bytes(server, &document)? != scope {
        return Err(invalid("noncanonical recovery inventory scope"));
    }
    Ok((server, document))
}

pub(super) enum RecoveryName {
    Final([u8; 32]),
    Temporary([u8; 32]),
}

pub(super) fn recovery_name(name: &OsStr) -> Result<Option<RecoveryName>, AppError> {
    // Windows direct opens can resolve case aliases. Recognize the entire reserved family
    // case-insensitively on every OS, then refuse noncanonical spelling instead of omitting bytes.
    if !name
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains(".recovery")
    {
        return Ok(None);
    }
    let name = name
        .to_str()
        .ok_or_else(|| invalid("non-UTF-8 recovery filename"))?;
    if let Some(hash) = name.strip_suffix(".recovery") {
        return Ok(Some(RecoveryName::Final(filename_hash(hash)?)));
    }
    if let Some((hash, tail)) = name
        .strip_prefix('.')
        .and_then(|n| n.split_once(".recovery.mewtual-stage-"))
    {
        if let Some((pid, id)) = tail.strip_suffix(".tmp").and_then(|s| s.split_once('-')) {
            let canonical_u64 =
                |text: &str| text.parse::<u64>().is_ok_and(|n| n.to_string() == text);
            if canonical_u64(pid) && pid.parse::<u32>().is_ok() && canonical_u64(id) {
                return Ok(Some(RecoveryName::Temporary(filename_hash(hash)?)));
            }
        }
    }
    Err(invalid("noncanonical recovery inventory filename"))
}

fn filename_hash(text: &str) -> Result<[u8; 32], AppError> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid("noncanonical recovery inventory filename"));
    }
    let mut hash = [0; 32];
    hex::decode_to_slice(text, &mut hash).map_err(invalid)?;
    Ok(hash)
}

pub(in crate::store) fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // Reparse points include more than symlinks (for example, junctions).
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

pub(in crate::store) fn regular_file(metadata: &fs::Metadata) -> bool {
    metadata.is_file() && !is_link(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_replication::RecoveryReason;
    use catcoms_rt::ManualClock;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    fn open(path: &Path) -> ServerStore {
        ServerStore::open(path, b"inventory-test", &mut ChaCha20Rng::seed_from_u64(1)).unwrap()
    }

    fn document(group: &[u8], key: &[u8]) -> LogicalDocument {
        LogicalDocument::new(group.to_vec(), DocType::StudioObject, key.to_vec()).unwrap()
    }

    fn stage(store: &mut ServerStore, server: u64, document: &LogicalDocument, epoch: u64) {
        let snapshot = RecoverySnapshot {
            doc_type: document.doc_type,
            logical_key: document.logical_key.clone(),
            epoch,
            base_close_record_hash: None,
            reason: RecoveryReason::Excluded,
            projection: b"private recovery picture".to_vec(),
            tombstones: vec![],
            elements: vec![],
            conflicts: vec![],
            applied_ops: vec![],
        };
        store
            .update_epoch_recovery(
                server,
                document,
                EpochRecoveryAction::Stage(snapshot),
                &ManualClock::new(epoch),
                &mut ChaCha20Rng::seed_from_u64(epoch),
            )
            .unwrap();
    }

    fn collect(store: &mut ServerStore) -> Result<EpochRecoveryInventory, AppError> {
        let mut scan = store.scan_epoch_recovery()?;
        loop {
            let before = scan.progress;
            let after = scan.step()?;
            assert!(after.visited_entries - before.visited_entries <= ENTRIES_PER_STEP);
            assert!(after.recovery_records - before.recovery_records <= 1);
            assert!(
                after.authenticated_bytes - before.authenticated_bytes <= MAX_SEALED_BYTES as u64
            );
            if after.complete {
                break;
            }
        }
        scan.finish()
    }

    #[test]
    fn an_empty_vault_becomes_a_completed_empty_inventory_only_at_eof() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        assert!(store.scan_epoch_recovery().unwrap().finish().is_err());
        let mut scan = store.scan_epoch_recovery().unwrap();
        let first = scan.step().unwrap();
        assert!(first.complete);
        assert_eq!(first.visited_entries, 0);
        assert_eq!(scan.step().unwrap(), first);
        let inventory = scan.finish().unwrap();
        assert_eq!(inventory.records().len(), 0);
        assert_eq!(inventory.orphans().len(), 0);
        assert!(inventory
            .records_for_server(7, b"group")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn discovery_recovers_full_scope_and_physical_pools_without_a_registry_after_restart() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"first-group", b"private-key");
        let reused = document(b"different-group", b"private-key");
        for epoch in 1..=3 {
            stage(&mut store, 7, &doc, epoch);
        }
        stage(&mut store, 7, &reused, 1);
        stage(&mut store, 8, &doc, 1);
        // Legacy files need not be valid recovery ciphertext and are never read as such.
        fs::write(root.path().join("servers/7.bin"), b"legacy opaque bytes").unwrap();
        let expected = store
            .epoch_recovery_inventory_record(7, &doc)
            .unwrap()
            .unwrap();
        drop(store);
        let mut store = open(root.path());
        let inventory = collect(&mut store).unwrap();
        assert_eq!(inventory.records().len(), 3);
        assert_eq!(
            inventory.records_for_server(7, b"first-group").unwrap(),
            vec![expected]
        );
        assert_eq!(
            inventory
                .records_for_server(7, b"different-group")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            inventory
                .records_for_server(8, b"first-group")
                .unwrap()
                .len(),
            1
        );
        assert!(expected.footprint.settlement > 0);
        assert!(inventory
            .records_for_server(9, b"first-group")
            .unwrap()
            .is_empty());
        assert!(inventory.records_for_server(7, b"").is_err());
        let debug = format!(
            "{inventory:?} {:?}",
            inventory.records().collect::<Vec<_>>()
        );
        for private in ["private-key", "first-group", "private recovery picture"] {
            assert!(!debug.contains(private));
        }
    }

    #[test]
    fn orphan_attribution_is_order_independent_and_counts_empty_and_partial_copies() {
        for temp_first in [true, false] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let doc = document(b"group", b"cat");
            let final_path = store.epoch_recovery_path(&scope_bytes(7, &doc).unwrap());
            let empty = staging_candidate(&final_path, 900);
            let partial = staging_candidate(&final_path, 901);
            if !temp_first {
                stage(&mut store, 7, &doc, 1);
            }
            fs::write(&empty, []).unwrap();
            fs::write(&partial, b"not a complete sealed record").unwrap();
            if temp_first {
                stage(&mut store, 7, &doc, 1);
            }
            let inventory = collect(&mut store).unwrap();
            assert_eq!(inventory.unresolved_orphans(), 0);
            assert_eq!(inventory.orphans().len(), 2);
            let records = inventory.records_for_server(7, b"group").unwrap();
            assert_eq!(records.len(), 3);
            let final_record = store
                .epoch_recovery_inventory_record(7, &doc)
                .unwrap()
                .unwrap();
            let budget = EpochStorageBudget::from_inventory(
                StorageScope::new(7, b"group").unwrap(),
                records,
            )
            .unwrap();
            assert_eq!(budget.usage().content, final_record.footprint.content);
            assert_eq!(
                budget.usage().settlement,
                b"not a complete sealed record".len() as u64
            );
            assert!(empty.exists());
            assert_eq!(fs::read(&partial).unwrap(), b"not a complete sealed record");
            assert!(
                !format!("{:?}", inventory.orphans().collect::<Vec<_>>()).contains("mewtual-stage")
            );
        }
    }

    #[test]
    fn orphan_without_verified_destination_blocks_all_server_composition_and_is_not_promoted() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"group", b"cat");
        stage(&mut store, 7, &doc, 1);
        let source = store.epoch_recovery_path(&scope_bytes(7, &doc).unwrap());
        let orphan = staging_candidate(&source, 999);
        fs::rename(&source, &orphan).unwrap();
        let inventory = collect(&mut store).unwrap();
        assert_eq!(inventory.records().len(), 0);
        assert_eq!(inventory.unresolved_orphans(), 1);
        for server in [7, 8] {
            assert!(inventory.records_for_server(server, b"group").is_err());
        }
        assert!(!source.exists());
        assert!(orphan.exists());
        assert_eq!(
            inventory.orphans().next().unwrap().name(),
            orphan.file_name().unwrap().to_str().unwrap()
        );
    }

    #[test]
    fn incomplete_cancelled_or_failed_scans_never_return_a_completed_inventory() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"group", b"cat");
        stage(&mut store, 7, &doc, 1);
        assert!(store.scan_epoch_recovery().unwrap().finish().is_err());
        let mut scan = store.scan_epoch_recovery().unwrap();
        assert!(!scan.step().unwrap().complete); // One body is the entire step's parse budget.
        drop(scan);
        assert_eq!(collect(&mut store).unwrap().records().len(), 1);
        let mut scan = store.scan_epoch_recovery().unwrap();
        scan.record_limit = 0;
        assert!(scan.step().is_err());
        scan.record_limit = MAX_ACCOUNTED_RECORDS;
        assert!(scan.step().is_err()); // Increasing a test rail cannot unpoison the job.
        assert!(scan.finish().is_err());
    }

    #[test]
    fn traversal_and_bytes_have_independent_fail_closed_limits() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for id in 0..(ENTRIES_PER_STEP + 1) {
            fs::write(root.path().join("servers").join(format!("{id}.bin")), []).unwrap();
        }
        let mut scan = store.scan_epoch_recovery().unwrap();
        scan.entry_limit = ENTRIES_PER_STEP;
        let step = scan.step().unwrap();
        assert_eq!(step.visited_entries, ENTRIES_PER_STEP);
        assert!(!step.complete);
        assert!(scan.step().is_err());
        assert!(scan.finish().is_err());
        let doc = document(b"group", b"cat");
        stage(&mut store, 7, &doc, 1);
        let bytes = store
            .epoch_recovery_inventory_record(7, &doc)
            .unwrap()
            .unwrap()
            .footprint
            .total()
            .unwrap();
        let mut scan = store.scan_epoch_recovery().unwrap();
        scan.byte_limit = bytes - 1;
        while scan.step().is_ok() {}
        assert_eq!(scan.progress.authenticated_bytes, 0);
        assert!(scan.finish().is_err());
    }

    #[test]
    fn a_caught_parser_panic_cannot_skip_an_entry_and_finish_the_scan() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut scan = store.scan_epoch_recovery().unwrap();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scan.guarded_step(|_| panic!("injected parser panic"))
        }))
        .is_err());
        assert!(scan.step().is_err());
        assert!(scan.finish().is_err());
    }

    #[test]
    fn zero_byte_orphans_still_exhaust_the_metadata_rail() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let destination =
            store.epoch_recovery_path(&scope_bytes(7, &document(b"g", b"k")).unwrap());
        for id in 0..3 {
            fs::write(staging_candidate(&destination, id), []).unwrap();
        }
        let mut scan = store.scan_epoch_recovery().unwrap();
        scan.record_limit = 2;
        assert!(scan.step().is_err());
        assert_eq!(scan.progress.orphan_files, 2);
        assert!(scan.finish().is_err());
    }

    #[test]
    fn malformed_oversized_aliased_and_misattributed_records_are_not_silently_omitted() {
        for mode in ["corrupt", "oversized", "wrong-hash", "alias", "directory"] {
            let root = tempfile::tempdir().unwrap();
            let mut store = open(root.path());
            let doc = document(b"group", b"cat");
            stage(&mut store, 7, &doc, 1);
            let path = store.epoch_recovery_path(&scope_bytes(7, &doc).unwrap());
            match mode {
                "corrupt" => fs::write(&path, [0; 40]).unwrap(),
                "oversized" => OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(MAX_SEALED_BYTES as u64 + 1)
                    .unwrap(),
                "wrong-hash" => fs::rename(
                    &path,
                    path.with_file_name(format!("{}.recovery", "01".repeat(32))),
                )
                .unwrap(),
                "alias" => {
                    // Two renames also exercise Windows' case-preserving filesystem reliably.
                    let temporary = path.with_file_name("rename-intermediate");
                    fs::rename(&path, &temporary).unwrap();
                    fs::rename(
                        temporary,
                        path.with_file_name(
                            path.file_name()
                                .unwrap()
                                .to_str()
                                .unwrap()
                                .to_ascii_uppercase(),
                        ),
                    )
                    .unwrap();
                }
                "directory" => {
                    fs::remove_file(&path).unwrap();
                    fs::create_dir(path).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(collect(&mut store).is_err(), "{mode}");
        }
    }

    #[test]
    fn filename_and_scope_grammars_are_exact_and_bounded() {
        let hash = "ab".repeat(32);
        assert!(matches!(
            recovery_name(OsStr::new(&format!("{hash}.recovery"))).unwrap(),
            Some(RecoveryName::Final(_))
        ));
        assert!(matches!(
            recovery_name(OsStr::new(&format!(
                ".{hash}.recovery.mewtual-stage-4294967295-18446744073709551615.tmp"
            )))
            .unwrap(),
            Some(RecoveryName::Temporary(_))
        ));
        for name in [
            format!("{hash}.RECOVERY"),
            format!("{}.recovery", hash.to_uppercase()),
            format!(".{hash}.recovery.mewtual-stage-01-1.tmp"),
            format!(".{hash}.recovery.mewtual-stage-1-+1.tmp"),
            format!(".{hash}.recovery.mewtual-stage-1-18446744073709551616.tmp"),
            format!(".{hash}.recovery.mewtual-stage-4294967296-1.tmp"),
            format!(".{hash}.recovery.mewtual-stage-1-1.TMP"),
            "unexpected.recovery.backup".into(),
        ] {
            assert!(recovery_name(OsStr::new(&name)).is_err(), "{name}");
        }
        assert!(recovery_name(OsStr::new("7.cache")).unwrap().is_none());
        let doc = document(&[1; 256], &[2; 192]);
        let scope = scope_bytes(u64::MAX, &doc).unwrap();
        assert_eq!(decode_scope(&scope).unwrap(), (u64::MAX, doc));
        let mut trailing = scope.clone();
        trailing.push(0);
        assert!(decode_scope(&trailing).is_err());
        assert!(decode_scope(b"wrong scope domain").is_err());
        let mut wrong = scope;
        wrong[4] ^= 1;
        assert!(decode_scope(&wrong).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn recovery_symlinks_and_redirected_inventory_parents_are_refused() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"group", b"cat");
        let final_path = store.epoch_recovery_path(&scope_bytes(7, &doc).unwrap());
        let target = root.path().join("outside");
        fs::write(&target, b"do not follow").unwrap();
        symlink(&target, &final_path).unwrap();
        assert!(collect(&mut store).is_err());
        assert_eq!(fs::read(target).unwrap(), b"do not follow");
        let parent = root.path().join("servers");
        let moved = root.path().join("redirect-target");
        fs::rename(&parent, &moved).unwrap();
        symlink(&moved, &parent).unwrap();
        assert!(store.scan_epoch_recovery().is_err());
    }
}
