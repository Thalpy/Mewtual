//! Shared bounded discovery of recovery, owner journals and opt-in intents/registry epochs.
//!
//! A scan borrows the mounted store exclusively across steps. It never repairs, promotes or
//! removes a file. Bodies are authenticated one at a time and discarded; only bounded metadata
//! survives. The completed result is a point-in-time observation, not a continuing write lease.

use std::collections::BTreeMap;

use super::*;
use crate::store::epoch_budget::MAX_ACCOUNTED_RECORDS;
use crate::store::{epoch_draft_archive, epoch_intents, epoch_owner};
use catcoms_wire::DocType;
pub(in crate::store) mod cache;

/// Local automatic-service rails, not replicated document acceptance rules.
pub(crate) const STUDIO_RECEIVE_COLD_BYTES: u64 = 256 * 1024;
const STUDIO_RECEIVE_READ_BYTES: u64 = 8 * 1024 * 1024;

/// Explicit file-family coverage, carried unchanged through cleanup, scan and completed result.
/// No variant includes blobs or future non-Studio/non-registry epoch snapshot families.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpochInventoryCoverage {
    /// Compatibility mode: existing recovery-only APIs never inspect/delete owner journals.
    RecoveryOnly,
    /// Compatibility mode: recovery and owner journals only, under one store borrow.
    RecoveryAndOwnerReceipts,
    /// Explicit opt-in to local intent files as well; older APIs retain their narrower coverage.
    RecoveryOwnerReceiptsAndIntents,
    /// Also covers durable registry epochs; earlier modes keep their exact narrower coverage.
    RecoveryOwnerReceiptsIntentsAndRegistry,
    /// Five-family Studio metadata inventory; blobs remain a separate lifecycle.
    RecoveryOwnerReceiptsIntentsRegistryAndStudio,
}

impl EpochInventoryCoverage {
    pub(in crate::store) fn includes_intents(self) -> bool {
        matches!(
            self,
            Self::RecoveryOwnerReceiptsAndIntents
                | Self::RecoveryOwnerReceiptsIntentsAndRegistry
                | Self::RecoveryOwnerReceiptsIntentsRegistryAndStudio
        )
    }
}

/// Physical record family; a digest alone must never identify a temporary's destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EpochRecordKind {
    /// Retained/staged recovery snapshots.
    Recovery,
    /// Owner-local pending and published receipt decisions.
    OwnerReceipts,
    /// Device-local replay instructions, never inbound peer submissions.
    Intents,
    /// Device-local preserved draft archives. A distinct physical family that shares the Intents
    /// accounting class; its contents are Agent 2's manual lifecycle and are not decoded here.
    DraftArchive,
    /// Checked registry document/gate/receipt restart units.
    Registry,
    /// Checked Index/art document/gate/receipt restart units.
    Studio,
}

impl EpochRecordKind {
    /// True for the families charged to `EpochIntentBudget` against `MAX_VAULT_INTENT_BYTES`.
    /// Physical identity is per variant; accounting is per class.
    pub(in crate::store) fn intent_class(self) -> bool {
        matches!(self, Self::Intents | Self::DraftArchive)
    }
    fn suffix(self) -> &'static str {
        match self {
            Self::Recovery => ".recovery",
            Self::OwnerReceipts => ".owner-receipts",
            Self::Intents => ".intents",
            Self::DraftArchive => ".draft-archive",
            Self::Registry => ".registry-epoch",
            Self::Studio => ".studio-epoch",
        }
    }
    fn domain(self) -> &'static [u8] {
        match self {
            Self::Recovery => RECORD_DOMAIN,
            Self::OwnerReceipts => epoch_owner::RECORD_DOMAIN,
            Self::Intents => epoch_intents::RECORD_DOMAIN,
            Self::DraftArchive => epoch_draft_archive::RECORD_DOMAIN,
            Self::Registry => super::super::epoch_registry::RECORD_DOMAIN,
            Self::Studio => super::super::epoch_studio::RECORD_DOMAIN,
        }
    }
    fn scope(self, server: u64, document: &LogicalDocument) -> Result<Vec<u8>, AppError> {
        match self {
            Self::Recovery => scope_bytes(server, document),
            Self::OwnerReceipts => epoch_owner::scope_bytes(server, document),
            Self::Intents => epoch_intents::scope_bytes(server, document),
            Self::DraftArchive => epoch_draft_archive::scope_bytes(server, document),
            Self::Registry => super::super::epoch_registry::scope_bytes(server, document),
            Self::Studio => super::super::epoch_studio::scope_bytes(server, document),
        }
    }
    fn sealed_cap(self) -> usize {
        match self {
            Self::Recovery => MAX_SEALED_BYTES,
            Self::OwnerReceipts => epoch_owner::MAX_SEALED_BYTES,
            Self::Intents => epoch_intents::MAX_SEALED_BYTES,
            Self::DraftArchive => epoch_draft_archive::MAX_DRAFT_ARCHIVE_SEALED_BYTES,
            Self::Registry => super::super::epoch_registry::MAX_SEALED_BYTES,
            Self::Studio => super::super::epoch_studio::MAX_SEALED_BYTES,
        }
    }
    /// A cheap precheck bound that no canonical scope of this family can exceed.
    ///
    /// Every family frames the same document fields, so only the length-prefixed domain differs
    /// between them. A single constant sized for Recovery's 31-byte domain therefore refused any
    /// family with a longer one at the maximum document shape; `DraftArchive`'s 36-byte domain is
    /// the first to have one, and a legal 506-byte archive scope was rejected outright (B-001).
    ///
    /// This is an upper bound, not each family's exact maximum: Registry and Studio additionally
    /// pin their logical key to 32 and 16 bytes, so their real maxima are well below it. Being
    /// loose there costs nothing, because `decode_record_scope` re-derives the canonical scope for
    /// the family and compares it byte for byte. Being *tight* is what must never happen, and
    /// `no_family_can_encode_a_scope_its_own_bound_refuses` proves it against real encodings.
    fn scope_cap(self) -> usize {
        MAX_SCOPE_DOCUMENT_BYTES + 4 + self.domain().len()
    }
}

/// The scope fields after the domain, at the largest shape `LogicalDocument::new` admits: the
/// numeric mount, the length-prefixed group id at 256 bytes, the document type tag, and the
/// length-prefixed logical key at 192 bytes.
const MAX_SCOPE_DOCUMENT_BYTES: usize = 8 + 4 + 256 + 2 + 4 + 192;

// Count ignored legacy names too: a flat directory with hostile clutter must not make one step
// scan indefinitely. These are local discovery rails, not replicated admission rules.
pub(super) const MAX_DIRECTORY_ENTRIES: usize = 2 * MAX_ACCOUNTED_RECORDS;
pub(super) const ENTRIES_PER_STEP: usize = 64;
// One authenticated body per step; its byte ceiling is the addressed reader's existing cap.
// This also bounds aggregate authentication work, independently of caller scheduling.
const MAX_AUTHENTICATED_BYTES: u64 = MAX_ACCOUNTED_RECORDS as u64 * MAX_SEALED_BYTES as u64;

/// Counts only. Progress is not evidence that an incomplete inventory is safe to spend against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EpochStorageScanProgress {
    /// Includes entries outside this scan's coverage skipped during traversal.
    pub visited_entries: usize,
    /// Successfully authenticated canonical recovery final records (never owner journals).
    pub recovery_records: usize,
    /// Successfully authenticated owner journals; always zero in recovery-only mode.
    pub owner_receipt_records: usize,
    /// Authenticated local intent ledgers; zero unless coverage explicitly includes intents.
    pub intent_records: usize,
    /// Authenticated preserved draft archives; gated by the same coverage as intents.
    pub draft_archive_records: usize,
    /// Authenticated checked registry epochs; zero unless coverage explicitly includes them.
    pub registry_records: usize,
    /// Checked Studio epoch files; zero unless explicitly included by coverage.
    pub studio_records: usize,
    /// Canonically named staging siblings, including empty or partial files.
    pub orphan_files: usize,
    /// Physical ciphertext bytes read and authenticated (no orphan bodies are read).
    pub authenticated_bytes: u64,
    /// Successfully authenticated history records whose exact prior validation was reused.
    pub reused_records: usize,
    /// Physical bytes charged to fresh record parsing/validation (including non-history families).
    pub uncached_bytes: u64,
    /// True only after reaching directory EOF without any error or exhausted rail.
    pub complete: bool,
}

/// Authenticated attribution and physical accounting for one final record in this scan's coverage.
pub struct EpochStorageInventoryEntry {
    /// The authenticated physical namespace, not inferred from a matching digest alone.
    pub kind: EpochRecordKind,
    /// Local server mount id inside the authenticated record.
    pub server: u64,
    /// Full group/type/key scope inside that record, independent of the current server registry.
    pub document: LogicalDocument,
    /// Exact observed physical footprint, including vault framing and staged-slot bytes.
    pub record: StorageRecord,
}

impl std::fmt::Debug for EpochStorageInventoryEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochStorageInventoryEntry")
            .field("kind", &self.kind)
            .field("footprint", &self.record.footprint)
            .finish_non_exhaustive()
    }
}

/// Observed staging metadata. Neither the filename nor the bytes authorize a promotion/deletion.
pub struct EpochStorageOrphan {
    name: String,
    destination: (EpochRecordKind, [u8; 32]),
    bytes: u64,
}

impl EpochStorageOrphan {
    /// Namespace whose canonical staging name was observed; this does not prove ownership.
    pub fn kind(&self) -> EpochRecordKind {
        self.destination.0
    }
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

impl std::fmt::Debug for EpochStorageOrphan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochStorageOrphan")
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

/// Complete discovery of only the families named by `coverage()`. Other P1 files, blobs and
/// legacy server snapshots are NOT inventoried here. Releasing the scan can make this view stale.
/// It deliberately has no public constructor, including `Default`; completion requires scan EOF.
///
/// ```compile_fail
/// use catcoms_app::store::EpochStorageInventory;
/// let unverified_empty = EpochStorageInventory::default();
/// ```
pub struct EpochStorageInventory {
    pub(in crate::store) intent_generation: std::sync::Arc<()>,
    pub(in crate::store) studio_generation: std::sync::Arc<()>,
    coverage: EpochInventoryCoverage,
    records: BTreeMap<(EpochRecordKind, [u8; 32]), EpochStorageInventoryEntry>,
    orphans: BTreeMap<String, EpochStorageOrphan>,
}

impl std::fmt::Debug for EpochStorageInventory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochStorageInventory")
            .field("coverage", &self.coverage)
            .field("records", &self.records.len())
            .field("orphans", &self.orphans.len())
            .field("unresolved_orphans", &self.unresolved_orphans())
            .finish_non_exhaustive()
    }
}

impl EpochStorageInventory {
    fn empty(coverage: EpochInventoryCoverage, intent_generation: std::sync::Arc<()>) -> Self {
        Self {
            intent_generation,
            studio_generation: std::sync::Arc::new(()),
            coverage,
            records: BTreeMap::new(),
            orphans: BTreeMap::new(),
        }
    }

    /// Exact families inventoried. This is not a complete-P1-storage attestation.
    pub fn coverage(&self) -> EpochInventoryCoverage {
        self.coverage
    }

    /// Authenticated final entries in namespace/storage-key order. No content or receipt bodies.
    pub fn records(&self) -> impl ExactSizeIterator<Item = &EpochStorageInventoryEntry> {
        self.records.values()
    }

    /// Staging metadata in deterministic basename order, including unresolved ownership.
    pub fn orphans(&self) -> impl ExactSizeIterator<Item = &EpochStorageOrphan> {
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

    /// Inputs from the reported coverage for a future complete server inventory. Refuses if ANY orphan
    /// is unresolved: an unknown file might belong to this server, even when others do not.
    /// Recovery/owner temporaries charge settlement scratch; intent/registry temporaries charge
    /// content conservatively (the latter might be peer ingress, never inferred from opaque bytes).
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
            return Err(invalid("unresolved epoch staging ownership"));
        }
        let belongs = |entry: &EpochStorageInventoryEntry| {
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
                // Preserve recovery temporary ids. Owner names use a separate domain and the
                // full basename, so identical textual digests cannot alias across namespaces.
                e.put_bytes(match orphan.kind() {
                    EpochRecordKind::Recovery => {
                        b"catcoms/epoch-recovery-temp-inventory/v1".as_slice()
                    }
                    EpochRecordKind::OwnerReceipts => {
                        b"catcoms/epoch-owner-temp-inventory/v1".as_slice()
                    }
                    EpochRecordKind::Intents => {
                        b"catcoms/epoch-intent-temp-inventory/v1".as_slice()
                    }
                    EpochRecordKind::DraftArchive => {
                        b"catcoms/epoch-draft-archive-temp-inventory/v1".as_slice()
                    }
                    EpochRecordKind::Registry => {
                        b"catcoms/epoch-registry-temp-inventory/v1".as_slice()
                    }
                    EpochRecordKind::Studio => b"catcoms/epoch-studio-temp-inventory/v1".as_slice(),
                })
                .expect("constant fits");
                e.put_bytes(orphan.name.as_bytes())
                    .expect("bounded basename fits");
                result.push(StorageRecord {
                    id: *blake3::hash(&e.finish()).as_bytes(),
                    document: entry.record.document,
                    // An opaque registry attempt might be peer ingress, not receipt settlement.
                    // Conservatively charge content until explicit cleanup; never parse or promote
                    // a temporary to guess which allowance it should consume.
                    footprint: if matches!(
                        orphan.kind(),
                        EpochRecordKind::Intents
                            | EpochRecordKind::DraftArchive
                            | EpochRecordKind::Registry
                            | EpochRecordKind::Studio
                    ) {
                        Footprint {
                            content: orphan.bytes,
                            ..Footprint::default()
                        }
                    } else {
                        Footprint {
                            settlement: orphan.bytes,
                            ..Footprint::default()
                        }
                    },
                });
            }
        }
        Ok(result)
    }
}

/// Exclusive, incrementally scheduled inventory job. Dropping it cancels discovery; neither
/// cancellation nor error exposes a partial inventory. Only mount-local validation metadata
/// may be memoized; no durable store mutation is performed.
pub struct EpochStorageScan<'a> {
    store: &'a mut ServerStore,
    directory: fs::ReadDir,
    inventory: EpochStorageInventory,
    progress: EpochStorageScanProgress,
    failed: bool,
    entry_limit: usize,
    record_limit: usize,
    byte_limit: u64,
    cold_byte_limit: Option<u64>,
    references: Option<CreativeReferenceScan>,
}

/// At most one requirement or match per already-counted authenticated record. Full scopes
/// retain numeric server, group, type and logical key; targets additionally bind the channel.
/// Bodies are still read only once, and directory order cannot change dependency resolution.
struct CreativeReferenceScan {
    generation: std::sync::Arc<()>,
    refs: super::super::creative_references::CreativeReferences,
    required: BTreeMap<(u64, LogicalDocument), catcoms_replication::studio::StudioTarget>,
    metadata: BTreeMap<(u64, LogicalDocument), catcoms_replication::studio::StudioTarget>,
}

impl CreativeReferenceScan {
    fn check_dependencies(&self) -> Result<(), AppError> {
        for (scope, target) in &self.required {
            if self.metadata.get(scope) != Some(target) {
                return Err(invalid(
                    "reference scan required handoff metadata missing or mismatched",
                ));
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for EpochStorageScan<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochStorageScan")
            .field("progress", &self.progress)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

impl ServerStore {
    /// Begin a nonrecursive scan of the reserved recovery filename family. The exclusive borrow
    /// lasts across ALL steps, excluding conforming writers; the mount lock excludes a second app
    /// process. A malicious local process replacing directories concurrently is out of scope.
    pub fn scan_epoch_recovery(&mut self) -> Result<EpochStorageScan<'_>, AppError> {
        self.scan_epoch_files(EpochInventoryCoverage::RecoveryOnly)
    }

    /// Discover recovery files AND owner journals under one uninterrupted exclusive borrow.
    /// Other record families still need integration before this can bootstrap a production budget.
    pub fn scan_epoch_storage(&mut self) -> Result<EpochStorageScan<'_>, AppError> {
        self.scan_epoch_files(EpochInventoryCoverage::RecoveryAndOwnerReceipts)
    }

    /// Explicit three-family inventory; required to bootstrap/reconcile the vault intent cap.
    /// Still excludes epoch snapshots and other unimplemented P1 file families.
    pub fn scan_epoch_storage_with_intents(&mut self) -> Result<EpochStorageScan<'_>, AppError> {
        self.scan_epoch_files(EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents)
    }

    /// Four-family inventory, including checked registry epochs. Still not all P1/app storage.
    pub fn scan_epoch_storage_with_registry(&mut self) -> Result<EpochStorageScan<'_>, AppError> {
        self.scan_epoch_files(EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsAndRegistry)
    }

    /// All currently implemented P1 metadata families, including Studio. Earlier APIs keep
    /// their exact coverage and cannot bootstrap a Studio write budget. Does not include blobs.
    pub fn scan_epoch_storage_with_studio(&mut self) -> Result<EpochStorageScan<'_>, AppError> {
        self.scan_epoch_files(EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio)
    }

    /// Conservative initial background-service rail, not a document acceptance or convergence
    /// rule. Reuse the SAME authenticated inventory, with lower limits checked before record
    /// reads/reconstruction. Previously verified history gets a larger bounded authentication
    /// allowance, not permission to reconstruct a cold large source. Explicit Save is unchanged.
    pub(crate) fn scan_studio_receive_inventory(
        &mut self,
    ) -> Result<EpochStorageScan<'_>, AppError> {
        let mut scan = self.scan_epoch_storage_with_studio()?;
        scan.entry_limit = 1024;
        scan.record_limit = 64;
        scan.byte_limit = STUDIO_RECEIVE_READ_BYTES;
        scan.cold_byte_limit = Some(STUDIO_RECEIVE_COLD_BYTES);
        Ok(scan)
    }

    pub(in crate::store) fn scan_epoch_files(
        &mut self,
        coverage: EpochInventoryCoverage,
    ) -> Result<EpochStorageScan<'_>, AppError> {
        let path = self.dir.join("servers");
        let metadata = fs::symlink_metadata(&path).map_err(|e| AppError::Io(e.to_string()))?;
        if !metadata.is_dir() || is_link(&metadata) {
            return Err(invalid(
                "epoch storage inventory parent is not a regular directory",
            ));
        }
        let directory = fs::read_dir(path).map_err(|e| AppError::Io(e.to_string()))?;
        let mut inventory = EpochStorageInventory::empty(coverage, self.intent_generation.clone());
        inventory.studio_generation = self.studio_generation.clone();
        Ok(EpochStorageScan {
            store: self,
            directory,
            inventory,
            progress: EpochStorageScanProgress::default(),
            failed: false,
            entry_limit: MAX_DIRECTORY_ENTRIES,
            record_limit: MAX_ACCOUNTED_RECORDS,
            byte_limit: MAX_AUTHENTICATED_BYTES,
            cold_byte_limit: None,
            references: None,
        })
    }
}

impl EpochStorageScan<'_> {
    /// Opt-in only: ordinary budget scans must NOT replace a transient pre-publication hold.
    pub(in crate::store) fn collect_creative_references(&mut self) -> Result<(), AppError> {
        if self.progress.visited_entries != 0
            || self.coverage()
                != EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio
        {
            return Err(invalid("reference scan requires fresh full inventory"));
        }
        let generation = self
            .store
            .creative_protection
            .lock()
            .map_err(|_| invalid("reference protection poisoned"))?
            .generation
            .clone();
        self.references = Some(CreativeReferenceScan {
            generation,
            refs: Default::default(),
            required: BTreeMap::new(),
            metadata: BTreeMap::new(),
        });
        Ok(())
    }
    pub(in crate::store) fn finish_creative_references(
        self,
    ) -> Result<super::super::creative_references::CreativeReferences, AppError> {
        if self.failed || !self.progress.complete || !self.inventory.orphans.is_empty() {
            // Partial temporary files may contain a not-yet-published reference. Never guess.
            return Err(invalid(
                "reference scan incomplete or unpublished metadata remains",
            ));
        }
        let collected = self
            .references
            .ok_or_else(|| invalid("not a reference scan"))?;
        collected.check_dependencies()?;
        self.store
            .creative_protection
            .lock()
            .map_err(|_| invalid("reference protection poisoned"))?
            .install(&collected.generation, collected.refs.clone())?;
        Ok(collected.refs)
    }
    /// Fixed coverage for this job, including before it completes.
    pub fn coverage(&self) -> EpochInventoryCoverage {
        self.inventory.coverage
    }
    /// Visit at most 64 directory entries and authenticate at most one bounded record. Schedule
    /// another step when `complete` is false. An error permanently poisons this scan, including
    /// errors caused by local corruption, disappearance, aliases or resource exhaustion.
    pub fn step(&mut self) -> Result<EpochStorageScanProgress, AppError> {
        self.guarded_step(Self::step_inner)
    }

    fn guarded_step(
        &mut self,
        work: impl FnOnce(&mut Self) -> Result<EpochStorageScanProgress, AppError>,
    ) -> Result<EpochStorageScanProgress, AppError> {
        if self.failed {
            return Err(invalid(
                "epoch storage inventory scan failed; restart required",
            ));
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

    fn step_inner(&mut self) -> Result<EpochStorageScanProgress, AppError> {
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
                return Err(invalid("epoch storage inventory directory limit reached"));
            }
            let name = entry.file_name();
            let Some((family, kind)) = storage_name(&name, self.coverage())? else {
                continue;
            };
            if self.inventory.records.len() + self.inventory.orphans.len() >= self.record_limit {
                return Err(invalid("epoch storage inventory record limit reached"));
            }
            let metadata =
                fs::symlink_metadata(entry.path()).map_err(|e| AppError::Io(e.to_string()))?;
            if !regular_file(&metadata) {
                return Err(invalid(
                    "epoch storage inventory entry is not a regular file",
                ));
            }
            match kind {
                RecoveryName::Final(hash) => {
                    // Select the small owner-journal cap BEFORE any body read or decrypt. A
                    // combined scan must not silently grant it recovery's much larger allowance.
                    if metadata.len() > family.sealed_cap() as u64 {
                        return Err(invalid("epoch inventory file exceeds its family bound"));
                    }
                    // The shared reader independently caps the opened file. Include actual bytes
                    // in the aggregate check as well, in case the metadata changed before open.
                    let peak = self
                        .progress
                        .authenticated_bytes
                        .checked_add(metadata.len())
                        .ok_or_else(|| invalid("epoch storage inventory byte limit reached"))?;
                    if peak > self.byte_limit {
                        return Err(invalid("epoch storage inventory byte limit reached"));
                    }
                    let cacheable =
                        matches!(family, EpochRecordKind::Registry | EpochRecordKind::Studio)
                            && self.references.is_none();
                    let candidate = cacheable
                        && self
                            .store
                            .inventory_cache
                            .candidate((family, hash), metadata.len());
                    if !candidate {
                        self.check_cold_bytes(metadata.len())?;
                    }
                    let AuthenticatedEpochFileBytes {
                        plain,
                        physical_bytes: size,
                    } = match family {
                        EpochRecordKind::Recovery => {
                            self.store.read_epoch_recovery_plain(&entry.path())
                        }
                        EpochRecordKind::OwnerReceipts => {
                            self.store.read_epoch_owner_plain(&entry.path())
                        }
                        EpochRecordKind::Intents => {
                            self.store.read_epoch_intent_plain(&entry.path())
                        }
                        EpochRecordKind::DraftArchive => {
                            self.store.read_epoch_draft_archive_plain(&entry.path())
                        }
                        EpochRecordKind::Registry => {
                            self.store.read_epoch_registry_plain(&entry.path())
                        }
                        EpochRecordKind::Studio => {
                            self.store.read_epoch_studio_plain(&entry.path())
                        }
                    }?
                    .ok_or_else(|| invalid("epoch record disappeared during inventory"))?;
                    self.progress.authenticated_bytes = self
                        .progress
                        .authenticated_bytes
                        .checked_add(size)
                        .filter(|bytes| *bytes <= self.byte_limit)
                        .ok_or_else(|| invalid("epoch storage inventory byte limit reached"))?;
                    let mut d = Decoder::new(&plain);
                    let scope = d.get_bytes().map_err(invalid)?;
                    let (server, document) = decode_record_scope(scope, family)?;
                    if blake3::hash(scope).as_bytes() != &hash {
                        return Err(invalid(
                            "epoch storage filename does not match its authenticated scope",
                        ));
                    }
                    // The digest includes scope/channel, receipt book, gate, quarantine and ALL
                    // signed history. Stat/filename/snapshot-head equality alone is insufficient.
                    let digest = blake3::hash(&plain);
                    let cached = cacheable
                        .then(|| self.store.inventory_cache.get((family, hash), size, digest))
                        .flatten();
                    let record = if let Some(record) = cached {
                        self.progress.reused_records += 1;
                        record
                    } else {
                        self.check_cold_bytes(size)?;
                        self.progress.uncached_bytes = self
                            .progress
                            .uncached_bytes
                            .checked_add(size)
                            .ok_or_else(|| {
                                invalid("epoch storage inventory cold byte limit reached")
                            })?;
                        let record = match family {
                            EpochRecordKind::Recovery => {
                                let state = EpochRecoveryState::decode(&plain, scope, &document)?;
                                if let Some(collected) = self.references.as_mut() {
                                    collected.refs.add(
                                        &document.server_id,
                                        super::super::creative_references::recovery_cids(
                                            &document, &state,
                                        )?,
                                    )?;
                                }
                                recovery_record(scope, state.footprint(size)?)
                            }
                            EpochRecordKind::OwnerReceipts => {
                                epoch_owner::EpochOwnerReceiptState::decode(
                                    &plain, scope, &document,
                                )?;
                                epoch_owner::storage_record(server, &document, scope, size)?
                            }
                            EpochRecordKind::Intents => {
                                // Accounting and reference collection need the ledger, the
                                // handoff metadata target and the overlay's seed-derived base
                                // CIDs, never its replayed projection. A retained branch would
                                // otherwise be fully reconstructed on every five-family scan in
                                // the vault, including scans for unrelated documents.
                                let state = epoch_intents::EpochIntentState::decode_structural(
                                    &plain, scope, &document,
                                )?;
                                if let Some(collected) = self.references.as_mut() {
                                    if let Some(metadata) = state.handoff_metadata() {
                                        collected
                                            .metadata
                                            .insert((server, document.clone()), metadata.target());
                                    }
                                    if let Some(overlay) = state.overlay() {
                                        collected.refs.add(
                                            &document.server_id,
                                            overlay.base_blob_cids().map_err(invalid)?,
                                        )?;
                                    }
                                    if matches!(
                                        document.doc_type,
                                        DocType::StudioIndex | DocType::StudioObject
                                    ) {
                                        for (_, intent) in state.pending() {
                                            collected.refs.add(
                                                &document.server_id,
                                                catcoms_replication::studio::operation_blob_cid(
                                                    &intent.operation,
                                                )
                                                .map_err(invalid)?,
                                            )?;
                                        }
                                    } else if document.doc_type != DocType::DocRegistry {
                                        return Err(invalid(
                                            "unsupported creative reference family",
                                        ));
                                    }
                                }
                                epoch_intents::storage_record(server, &document, scope, size)?
                            }
                            EpochRecordKind::DraftArchive => {
                                // The seam authenticates, names and accounts an archive without
                                // knowing what is inside it. A reference scan is the one case that
                                // cannot proceed on that basis: its deletion-protection set would
                                // omit the archive's CIDs and archived pixels would be reclaimed,
                                // destroying the preservation guarantee. The seam therefore failed
                                // closed for EVERY archive until the collector existed.
                                //
                                // That refusal is NARROWED here, not removed. `inventory_references`
                                // still refuses an archive whose bounded canonical payload will not
                                // decode, which is the rule every other family applies to a corrupt
                                // record; what it no longer refuses is an archive it can read. An
                                // accounting-only scan still needs no payload, exactly as before.
                                if let Some(collected) = self.references.as_mut() {
                                    let inspected = epoch_draft_archive::inventory_references(
                                        &plain, server, &document, scope, size,
                                    )?;
                                    collected.refs.add(&document.server_id, inspected.cids)?;
                                    inspected.record
                                } else {
                                    epoch_draft_archive::storage_record(
                                        server, &document, scope, size,
                                    )?
                                }
                            }
                            EpochRecordKind::Registry => {
                                super::super::epoch_registry::inventory_record(
                                    &plain, server, &document, scope, size,
                                )?
                            }
                            EpochRecordKind::Studio => {
                                if let Some(collected) = self.references.as_mut() {
                                    let inspected =
                                        super::super::epoch_studio::inventory_references(
                                            &plain, server, &document, scope, size,
                                        )?;
                                    collected.refs.add(&document.server_id, inspected.cids)?;
                                    if let Some(target) = inspected.required_metadata {
                                        collected
                                            .required
                                            .insert((server, document.clone()), target);
                                    }
                                    inspected.record
                                } else {
                                    super::super::epoch_studio::inventory_record(
                                        &plain, server, &document, scope, size,
                                    )?
                                }
                            }
                        };
                        if matches!(family, EpochRecordKind::Registry | EpochRecordKind::Studio) {
                            // Even an explicit reference scan may warm pure validation metadata,
                            // but a later reference scan must still enumerate the actual CIDs.
                            self.store
                                .inventory_cache
                                .put((family, hash), size, digest, record);
                        }
                        record
                    };
                    if self
                        .inventory
                        .records
                        .insert(
                            (family, hash),
                            EpochStorageInventoryEntry {
                                kind: family,
                                server,
                                document,
                                record,
                            },
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate epoch storage inventory entry"));
                    }
                    match family {
                        EpochRecordKind::Recovery => self.progress.recovery_records += 1,
                        EpochRecordKind::OwnerReceipts => self.progress.owner_receipt_records += 1,
                        EpochRecordKind::Intents => self.progress.intent_records += 1,
                        EpochRecordKind::DraftArchive => self.progress.draft_archive_records += 1,
                        EpochRecordKind::Registry => self.progress.registry_records += 1,
                        EpochRecordKind::Studio => self.progress.studio_records += 1,
                    }
                    break;
                }
                RecoveryName::Temporary(destination) => {
                    // No body is parsed: it may be empty or half-written. Attribution occurs only
                    // through an authenticated destination, after the whole directory was seen.
                    let name = name
                        .into_string()
                        .map_err(|_| invalid("non-UTF-8 epoch filename"))?;
                    if self
                        .inventory
                        .orphans
                        .insert(
                            name.clone(),
                            EpochStorageOrphan {
                                name,
                                destination: (family, destination),
                                bytes: metadata.len(),
                            },
                        )
                        .is_some()
                    {
                        return Err(invalid("duplicate epoch storage staging entry"));
                    }
                    self.progress.orphan_files += 1;
                }
            }
        }
        Ok(self.progress)
    }

    fn check_cold_bytes(&self, size: u64) -> Result<(), AppError> {
        if self.cold_byte_limit.is_some_and(|limit| {
            self.progress
                .uncached_bytes
                .checked_add(size)
                .is_none_or(|bytes| bytes > limit)
        }) {
            return Err(invalid("epoch storage inventory cold byte limit reached"));
        }
        Ok(())
    }

    /// Consume only a successful EOF result. A completed result is metadata, not a storage lease;
    /// future callers must keep the coordinator exclusive until the complete budget is installed.
    pub fn finish(self) -> Result<EpochStorageInventory, AppError> {
        if self.failed || !self.progress.complete {
            return Err(invalid("epoch storage inventory is incomplete"));
        }
        Ok(self.inventory)
    }
}

/// Only the absence of ALL reserved P1 filenames proves an empty reference cache cheaply.
/// Any final/temporary/alias, traversal error or exhausted rail starts the mount as Unknown.
pub(in crate::store) fn epoch_files_absent(path: &Path) -> Result<bool, AppError> {
    let metadata = fs::symlink_metadata(path).map_err(|e| AppError::Io(e.to_string()))?;
    if !metadata.is_dir() || is_link(&metadata) {
        return Err(invalid("invalid epoch parent"));
    }
    for (n, entry) in fs::read_dir(path)
        .map_err(|e| AppError::Io(e.to_string()))?
        .enumerate()
    {
        if n >= MAX_DIRECTORY_ENTRIES {
            return Err(invalid("epoch directory limit"));
        }
        let entry = entry.map_err(|e| AppError::Io(e.to_string()))?;
        if storage_name(
            &entry.file_name(),
            EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio,
        )?
        .is_some()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
fn decode_scope(scope: &[u8]) -> Result<(u64, LogicalDocument), AppError> {
    decode_record_scope(scope, EpochRecordKind::Recovery)
}

fn decode_record_scope(
    scope: &[u8],
    family: EpochRecordKind,
) -> Result<(u64, LogicalDocument), AppError> {
    if scope.len() > family.scope_cap() {
        return Err(invalid("epoch storage inventory scope exceeds its bound"));
    }
    let mut d = Decoder::new(scope);
    if d.get_bytes().map_err(invalid)? != family.domain() {
        return Err(invalid("unknown epoch storage inventory scope domain"));
    }
    let server = d.get_u64().map_err(invalid)?;
    let group = d.get_bytes().map_err(invalid)?;
    let kind = DocType::from_tag(d.get_u16().map_err(invalid)?)
        .ok_or_else(|| invalid("unknown epoch storage inventory document type"))?;
    let key = d.get_bytes().map_err(invalid)?;
    d.finish().map_err(invalid)?;
    let document = LogicalDocument::new(group.to_vec(), kind, key.to_vec()).map_err(invalid)?;
    if family.scope(server, &document)? != scope {
        return Err(invalid("noncanonical epoch storage inventory scope"));
    }
    Ok((server, document))
}

pub(super) enum RecoveryName {
    Final([u8; 32]),
    Temporary([u8; 32]),
}

#[cfg(test)]
fn recovery_name(name: &OsStr) -> Result<Option<RecoveryName>, AppError> {
    record_name(name, EpochRecordKind::Recovery)
}

pub(super) fn storage_name(
    name: &OsStr,
    coverage: EpochInventoryCoverage,
) -> Result<Option<(EpochRecordKind, RecoveryName)>, AppError> {
    for family in [
        EpochRecordKind::Recovery,
        EpochRecordKind::OwnerReceipts,
        EpochRecordKind::Intents,
        EpochRecordKind::DraftArchive,
        EpochRecordKind::Registry,
        EpochRecordKind::Studio,
    ] {
        // Archives are gated with intents because they share the Intents accounting class. That
        // is only safe while no coverage narrower than the full five-family scan installs a
        // deletion-protection set: a narrower scan would omit archived CIDs from the known set
        // and archived pixels would be reclaimed. Reference scans run at full coverage.
        if family.intent_class() && !coverage.includes_intents() {
            continue;
        }
        if family == EpochRecordKind::Registry
            && !matches!(
                coverage,
                EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsAndRegistry
                    | EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio
            )
        {
            continue;
        }
        if family == EpochRecordKind::Studio
            && coverage != EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio
        {
            continue;
        }
        if family == EpochRecordKind::OwnerReceipts
            && coverage == EpochInventoryCoverage::RecoveryOnly
        {
            continue;
        }
        if let Some(kind) = record_name(name, family)? {
            return Ok(Some((family, kind)));
        }
    }
    Ok(None)
}

fn record_name(name: &OsStr, family: EpochRecordKind) -> Result<Option<RecoveryName>, AppError> {
    // Windows direct opens can resolve case aliases. Recognize the entire reserved family
    // case-insensitively on every OS, then refuse noncanonical spelling instead of omitting bytes.
    if !name
        .to_string_lossy()
        .to_ascii_lowercase()
        .contains(family.suffix())
    {
        return Ok(None);
    }
    let name = name
        .to_str()
        .ok_or_else(|| invalid("non-UTF-8 epoch filename"))?;
    if let Some(hash) = name.strip_suffix(family.suffix()) {
        return Ok(Some(RecoveryName::Final(filename_hash(hash)?)));
    }
    if let Some((hash, tail)) = name
        .strip_prefix('.')
        .and_then(|n| n.split_once(&format!("{}.mewtual-stage-", family.suffix())))
    {
        if let Some((pid, id)) = tail.strip_suffix(".tmp").and_then(|s| s.split_once('-')) {
            let canonical_u64 =
                |text: &str| text.parse::<u64>().is_ok_and(|n| n.to_string() == text);
            if canonical_u64(pid) && pid.parse::<u32>().is_ok() && canonical_u64(id) {
                return Ok(Some(RecoveryName::Temporary(filename_hash(hash)?)));
            }
        }
    }
    Err(invalid("noncanonical epoch storage inventory filename"))
}

fn filename_hash(text: &str) -> Result<[u8; 32], AppError> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid("noncanonical epoch storage inventory filename"));
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

pub(super) fn invalid(error: impl std::fmt::Display) -> AppError {
    AppError::Invalid(format!("epoch storage: {error}"))
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

    /// Drive one scan to EOF at an explicit coverage.
    fn collect_with(
        store: &mut ServerStore,
        coverage: EpochInventoryCoverage,
    ) -> EpochStorageInventory {
        let mut scan = store.scan_epoch_files(coverage).unwrap();
        while !scan.step().unwrap().complete {}
        scan.finish().unwrap()
    }

    fn collect(store: &mut ServerStore) -> Result<EpochStorageInventory, AppError> {
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
    fn studio_automatic_inventory_caps_before_unrelated_source_authentication() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        // An unrelated large registry record must refuse on metadata size, even if its body
        // is corrupt. No decrypt, Automerge reconstruction or partial inventory is allowed.
        let name = format!("{}.registry-epoch", "00".repeat(32));
        fs::write(
            root.path().join("servers").join(name),
            vec![0; 256 * 1024 + 1],
        )
        .unwrap();
        let mut scan = store.scan_studio_receive_inventory().unwrap();
        assert_eq!(scan.entry_limit, 1024);
        assert_eq!(scan.record_limit, 64);
        assert_eq!(scan.byte_limit, STUDIO_RECEIVE_READ_BYTES);
        assert_eq!(scan.cold_byte_limit, Some(STUDIO_RECEIVE_COLD_BYTES));
        let error = loop {
            match scan.step() {
                Err(error) => break error.to_string(),
                Ok(p) => assert!(!p.complete, "oversized registry record was skipped"),
            }
        };
        assert!(error.contains("byte limit"), "{error}");
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
    fn combined_zero_byte_metadata_rail_and_parser_poison_cover_both_namespaces() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for (id, suffix) in [
            (0, "recovery"),
            (1, "owner-receipts"),
            (2, "owner-receipts"),
        ] {
            let path = root
                .path()
                .join("servers")
                .join(format!("{}.{suffix}", "ab".repeat(32)));
            fs::write(staging_candidate(&path, id), []).unwrap();
        }
        let mut scan = store.scan_epoch_storage().unwrap();
        scan.record_limit = 2;
        assert!(scan.step().is_err());
        assert_eq!(scan.progress.orphan_files, 2);
        scan.record_limit = 3;
        assert!(scan.step().is_err());
        assert!(scan.finish().is_err());
        let mut scan = store.scan_epoch_storage().unwrap();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scan.guarded_step(|_| panic!("combined parser panic"))
        }))
        .is_err());
        assert!(scan.finish().is_err());
        // Compatibility mode includes just one of the exact same directory's three orphans.
        let legacy = collect(&mut store).unwrap();
        assert_eq!(legacy.coverage(), EpochInventoryCoverage::RecoveryOnly);
        assert_eq!(legacy.orphans().len(), 1);
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
        for bad in [
            format!("{hash}.OWNER-RECEIPTS"),
            format!(".{hash}.owner-receipts.mewtual-stage-01-1.tmp"),
            format!(".{hash}.owner-receipts.mewtual-stage-1-+1.tmp"),
            format!(".{hash}.owner-receipts.mewtual-stage-4294967296-1.tmp"),
            format!(".{hash}.owner-receipts.mewtual-stage-1-18446744073709551616.tmp"),
        ] {
            assert!(storage_name(
                OsStr::new(&bad),
                EpochInventoryCoverage::RecoveryAndOwnerReceipts
            )
            .is_err());
            assert!(
                storage_name(OsStr::new(&bad), EpochInventoryCoverage::RecoveryOnly)
                    .unwrap()
                    .is_none()
            );
        }
        let doc = document(&[1; 256], &[2; 192]);
        let scope = scope_bytes(u64::MAX, &doc).unwrap();
        let owner_scope = epoch_owner::scope_bytes(u64::MAX, &doc).unwrap();
        assert_eq!(
            decode_record_scope(&owner_scope, EpochRecordKind::OwnerReceipts).unwrap(),
            (u64::MAX, doc.clone())
        );
        assert!(decode_record_scope(&scope, EpochRecordKind::OwnerReceipts).is_err());
        assert!(decode_record_scope(&owner_scope, EpochRecordKind::Recovery).is_err());
        assert_eq!(decode_scope(&scope).unwrap(), (u64::MAX, doc));
        let mut trailing = scope.clone();
        trailing.push(0);
        assert!(decode_scope(&trailing).is_err());
        assert!(decode_scope(b"wrong scope domain").is_err());
        let mut wrong = scope;
        wrong[4] ^= 1;
        assert!(decode_scope(&wrong).is_err());
    }

    /// The `DraftArchive` seam: physical identity of its own, accounting shared with Intents,
    /// and no knowledge of what an archive contains. Agent 2 builds the payload, the writer, the
    /// release path and the reference collector on top of this.
    ///
    /// The first half is the one that matters most: a vault with no archive file must behave
    /// exactly as it did before the variant existed.
    #[test]
    fn draft_archive_is_its_own_physical_family_sharing_the_intent_accounting_class() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"archive-group", b"private-key");
        stage(&mut store, 7, &doc, 1);

        // No archive file: every existing observation is unchanged, and the new counter is zero.
        let before = collect_with(
            &mut store,
            EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents,
        );
        let baseline: Vec<_> = before.records().map(|e| (e.kind, e.record)).collect();
        assert_eq!(baseline.len(), 1);
        assert_eq!(before.orphans().len(), 0);
        let baseline_budget = EpochIntentBudget::from_inventory(&before).unwrap();
        assert_eq!(baseline_budget.bytes(), 0);
        assert!(!epoch_files_absent(&root.path().join("servers")).unwrap());
        // Positive control for the fail-closed claim below: without an archive present, this
        // exact vault completes a full reference scan and installs a known set.

        // A directory holding ONLY an archive is not empty, which is what protects the cheap
        // empty-vault shortcut from starting a mount with an unprotected reference cache. The
        // same isolated vault is where the fail-closed reference claim is tested, because there
        // the only thing that can refuse a reference scan is the archive.
        let bare = tempfile::tempdir().unwrap();
        let mut spare = open(bare.path());
        assert!(epoch_files_absent(&bare.path().join("servers")).unwrap());
        spare
            .creative_pinned_cids()
            .expect("an empty vault must complete a reference scan");
        assert!(spare.creative_references_known());
        epoch_draft_archive::write_draft_archive_for_test(
            &spare,
            7,
            &doc,
            b"opaque archive body",
            &mut ChaCha20Rng::seed_from_u64(3),
        )
        .unwrap();
        assert!(!epoch_files_absent(&bare.path().join("servers")).unwrap());
        // A reference scan must not silently collect nothing from an archive it cannot read: an
        // installed protection set missing the archive's CIDs would make archived pixels
        // reclaimable, which is the preservation guarantee the archive exists to provide. The
        // control above proves this exact vault completed a reference scan before the archive
        // existed.
        //
        // The seam refused EVERY archive, because it had no collector. Agent 2's collector
        // narrowed that to refusing an archive whose bounded canonical payload will not decode,
        // which is what `b"opaque archive body"` is. The property this test guards is unchanged
        // and still holds; only the reason, and so the message, moved. Asserting the refusal
        // rather than its wording keeps this test honest across that narrowing, and
        // `an_undecodable_draft_archive_still_fails_a_reference_scan_closed` covers the same
        // property from the collector's side against a real branch.
        let refused = spare.creative_pinned_cids();
        assert!(
            refused.is_err(),
            "a reference scan installed a protection set for a vault holding an archive it \
             cannot read, so the archive's pixels are reclaimable: {refused:?}"
        );
        assert!(
            !spare.creative_references_known(),
            "a refused reference scan left protection claiming to be known"
        );
        drop(spare);

        // Now the same vault with an archive beside its recovery record.
        let path = epoch_draft_archive::write_draft_archive_for_test(
            &store,
            7,
            &doc,
            b"opaque archive body",
            &mut ChaCha20Rng::seed_from_u64(4),
        )
        .unwrap();
        let physical = fs::metadata(&path).unwrap().len();
        let after = collect_with(
            &mut store,
            EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents,
        );
        let archive = after
            .records()
            .find(|e| e.kind == EpochRecordKind::DraftArchive)
            .expect("the archive was not inventoried");
        assert_eq!(archive.server, 7);
        assert_eq!(archive.document, doc);
        assert_eq!(archive.record.footprint.content, physical);
        assert_eq!(archive.record.footprint.settlement, 0);
        assert_eq!(after.records().count(), baseline.len() + 1);
        // Everything that was there before is byte-identical, so the new family added a record
        // rather than disturbing one.
        for (kind, record) in &baseline {
            assert!(after
                .records()
                .any(|e| e.kind == *kind && e.record == *record));
        }

        // It charges a record slot and its bytes into the Intents class, under the vault cap.
        let budget = EpochIntentBudget::from_inventory(&after).unwrap();
        assert_eq!(
            budget.bytes(),
            baseline_budget.bytes() + physical,
            "an archive's bytes were not charged to the intent accounting class"
        );
        assert!(budget.bytes() < MAX_VAULT_INTENT_BYTES);
        assert_eq!(
            budget.record_slots_for_test(),
            baseline_budget.record_slots_for_test() + 1,
            "an archive did not claim a record slot in the intent accounting class"
        );
        assert!(EpochRecordKind::DraftArchive.intent_class());
        assert!(EpochRecordKind::Intents.intent_class());
        assert!(!EpochRecordKind::Studio.intent_class());

        // A coverage that excludes intents excludes archives, by the same gate.
        let narrow = collect_with(&mut store, EpochInventoryCoverage::RecoveryAndOwnerReceipts);
        assert!(
            narrow
                .records()
                .all(|e| e.kind != EpochRecordKind::DraftArchive),
            "a coverage that excludes intents inventoried an archive"
        );
        assert_eq!(
            narrow.records().count(),
            baseline.len(),
            "a coverage that excludes intents inventoried an archive"
        );
        assert!(EpochIntentBudget::from_inventory(&narrow).is_err());

        // A temporary archive sibling is recognised as temporary, and an archive scope is not an
        // intent scope in either direction even though the two share an accounting class.
        let hash = "ab".repeat(32);
        assert!(matches!(
            storage_name(
                OsStr::new(&format!("{hash}.draft-archive")),
                EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents
            )
            .unwrap(),
            Some((EpochRecordKind::DraftArchive, RecoveryName::Final(_)))
        ));
        assert!(matches!(
            storage_name(
                OsStr::new(&format!(
                    ".{hash}.draft-archive.mewtual-stage-4294967295-18446744073709551615.tmp"
                )),
                EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents
            )
            .unwrap(),
            Some((EpochRecordKind::DraftArchive, RecoveryName::Temporary(_)))
        ));
        for bad in [
            format!("{hash}.DRAFT-ARCHIVE"),
            format!(".{hash}.draft-archive.mewtual-stage-01-1.tmp"),
            format!(".{hash}.draft-archive.mewtual-stage-1-+1.tmp"),
        ] {
            assert!(storage_name(
                OsStr::new(&bad),
                EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents
            )
            .is_err());
            assert!(storage_name(
                OsStr::new(&bad),
                EpochInventoryCoverage::RecoveryAndOwnerReceipts
            )
            .unwrap()
            .is_none());
        }
        let archive_scope = epoch_draft_archive::scope_bytes(7, &doc).unwrap();
        let intent_scope = epoch_intents::scope_bytes(7, &doc).unwrap();
        assert_ne!(archive_scope, intent_scope);
        assert_eq!(
            decode_record_scope(&archive_scope, EpochRecordKind::DraftArchive).unwrap(),
            (7, doc.clone())
        );
        assert!(decode_record_scope(&archive_scope, EpochRecordKind::Intents).is_err());
        assert!(decode_record_scope(&intent_scope, EpochRecordKind::DraftArchive).is_err());

        // The addressed reader reaches the same file the scanner inventoried, under this family's
        // own cap, and an intent scope addresses nothing in it.
        let read = store
            .read_scoped_draft_archive_plain(&archive_scope)
            .unwrap()
            .expect("the archive is not readable at its canonical path");
        assert_eq!(read.physical_bytes, physical);
        assert!(store
            .read_scoped_draft_archive_plain(&intent_scope)
            .unwrap()
            .is_none());
    }

    /// B-001. No family may be able to encode a canonical scope that its own precheck bound then
    /// refuses. `scope_cap()` is arithmetic over `domain().len()`, so this runs it against real
    /// maximal encodings for all six families, and would fail loudly if `LogicalDocument`'s limits
    /// or any family's key rule changed underneath it.
    #[test]
    fn no_family_can_encode_a_scope_its_own_bound_refuses() {
        // Each family's own largest legal document: Registry and Studio pin their logical key to
        // 32 and 16 bytes, the rest take the full 192.
        let widest = |family: EpochRecordKind| match family {
            EpochRecordKind::Registry => {
                LogicalDocument::new(vec![1; 256], DocType::DocRegistry, vec![2; 32]).unwrap()
            }
            EpochRecordKind::Studio => {
                LogicalDocument::new(vec![1; 256], DocType::StudioObject, vec![2; 16]).unwrap()
            }
            _ => LogicalDocument::new(vec![1; 256], DocType::StudioObject, vec![2; 192]).unwrap(),
        };
        for family in [
            EpochRecordKind::Recovery,
            EpochRecordKind::OwnerReceipts,
            EpochRecordKind::Intents,
            EpochRecordKind::DraftArchive,
            EpochRecordKind::Registry,
            EpochRecordKind::Studio,
        ] {
            let doc = widest(family);
            let scope = family.scope(u64::MAX, &doc).unwrap();
            assert!(
                scope.len() <= family.scope_cap(),
                "{family:?} encodes a {}-byte scope its own {}-byte bound refuses",
                scope.len(),
                family.scope_cap()
            );
            let decoded = decode_record_scope(&scope, family).unwrap_or_else(|error| {
                panic!("{family:?} refused its own maximal canonical scope: {error}")
            });
            assert_eq!(decoded, (u64::MAX, doc.clone()));
            // The four families whose logical key is unconstrained reach the bound exactly, so the
            // arithmetic is tight where tightness is observable at all.
            if !matches!(family, EpochRecordKind::Registry | EpochRecordKind::Studio) {
                assert_eq!(
                    scope.len(),
                    family.scope_cap(),
                    "{family:?} no longer reaches its stated bound"
                );
            }
            // One byte past the bound is refused for every family, so the precheck still bounds
            // the work `decode_record_scope` does before it re-derives anything.
            let mut over = scope.clone();
            over.resize(family.scope_cap() + 1, 0);
            assert!(decode_record_scope(&over, family).is_err());
        }
    }

    /// B-001 in the scanner. A maximum-shape archive is 506 bytes because its domain is five bytes
    /// longer than Recovery's, so the old Recovery-sized constant refused a completely legal record
    /// that was canonically scoped, correctly sealed, correctly named and under its family's byte
    /// cap. Once Agent 2's writer exists such an archive would be unreconcilable into the intent
    /// budget and would block any operation needing a complete scan.
    #[test]
    fn a_maximum_shape_draft_archive_is_inventoried_rather_than_refused_by_a_scope_bound() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = LogicalDocument::new(vec![7; 256], DocType::StudioObject, vec![8; 192]).unwrap();

        let archive_scope = epoch_draft_archive::scope_bytes(7, &doc).unwrap();
        let intent_scope = epoch_intents::scope_bytes(7, &doc).unwrap();
        assert_eq!(
            archive_scope.len(),
            506,
            "the maximal archive scope is no longer the shape this regression exists for"
        );
        assert_eq!(intent_scope.len(), 499);
        assert!(
            archive_scope.len() > EpochRecordKind::Recovery.scope_cap(),
            "the archive scope no longer exceeds Recovery's bound, so this proves nothing"
        );

        let decoded = decode_record_scope(&archive_scope, EpochRecordKind::DraftArchive)
            .unwrap_or_else(|error| {
                panic!("a maximum-shape archive scope was refused by a scope bound: {error}")
            });
        assert_eq!(decoded, (7, doc.clone()));
        // Neither family accepts the other's scope even at the maximum shape.
        assert!(decode_record_scope(&archive_scope, EpochRecordKind::Intents).is_err());
        assert!(decode_record_scope(&intent_scope, EpochRecordKind::DraftArchive).is_err());

        let path = epoch_draft_archive::write_draft_archive_for_test(
            &store,
            7,
            &doc,
            b"opaque archive body",
            &mut ChaCha20Rng::seed_from_u64(5),
        )
        .unwrap();
        let physical = fs::metadata(&path).unwrap().len();
        let view = collect_with(
            &mut store,
            EpochInventoryCoverage::RecoveryOwnerReceiptsAndIntents,
        );
        let archive = view
            .records()
            .find(|e| e.kind == EpochRecordKind::DraftArchive)
            .expect("a maximum-shape archive was refused by a scope bound");
        assert_eq!(archive.document, doc);
        assert_eq!(archive.record.footprint.content, physical);
        assert_eq!(
            EpochIntentBudget::from_inventory(&view).unwrap().bytes(),
            physical,
            "a maximum-shape archive could not be reconciled into the intent budget"
        );
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
