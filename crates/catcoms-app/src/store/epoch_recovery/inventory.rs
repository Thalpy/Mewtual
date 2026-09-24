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
/// C-3's resumable inventory cursor: everything a scan needs except custody of the store.
///
/// The store is supplied per call rather than borrowed for the cursor's life, which is what
/// lets a scan be parked between visits. Holding `&mut ServerStore` across every step is what
/// made the previous scanner single-visit by construction.
///
/// `generation` is the `inventory_generation` observed when the scan began. Under I-4 that token
/// rotates before any five-family mutation's first possible I/O, so a cursor whose captured
/// value no longer matches the store's has been overtaken by a write it did not see, and must
/// refuse rather than continue or issue an inventory. Over-rotation costs a rescan;
/// under-rotation is the only unsafe direction, which is why the check is equality on the
/// allocation identity rather than a counter comparison.
pub struct EpochStorageCursor {
    directory: fs::ReadDir,
    inventory: EpochStorageInventory,
    progress: EpochStorageScanProgress,
    failed: bool,
    entry_limit: usize,
    record_limit: usize,
    byte_limit: u64,
    cold_byte_limit: Option<u64>,
    references: Option<CreativeReferenceScan>,
    generation: std::sync::Arc<()>,
    /// This cursor's own identity, so a detached validation cannot be installed into a
    /// different cursor that happens to be scanning the same record.
    identity: std::sync::Arc<()>,
    /// The physical mount. A result produced before a reopen must not be installed after one.
    mount: std::sync::Arc<()>,
    /// At most one, which is the existing one-body-per-step rail rather than a new bound.
    parked: Option<ParkedEpochRecord>,
    /// The record whose detached validation is outstanding: parked and not yet installed,
    /// whether or not the caller has taken the body yet.
    ///
    /// Stepping is refused while this is set. Without it, a caller could take a parked body,
    /// drop it, and keep scanning - and the record would be missing from the resulting inventory
    /// with nothing having failed. An inventory that silently omits a record is worse than one
    /// that refuses, because a budget built from it looks usable.
    awaiting: Option<(EpochRecordKind, [u8; 32])>,
}

/// A record whose authenticated plaintext has been read, but whose typed validation the cursor
/// would not run inside the visit's remaining budget.
///
/// Parking is a scheduling decision, not a weakening of any rail: the body was already read
/// under the byte and cold-byte limits, the family cap was already applied, and a genuine
/// size-limit violation still refuses rather than parks.
///
/// The four bindings are what make installing the result safe. Cursor identity stops a
/// validation being installed into a different scan; the mount stops one surviving a reopen;
/// the record id stops it being installed against a different entry; and the generation stops
/// it being installed after a write it never saw. All four are rechecked at install.
pub struct ParkedEpochRecord {
    identity: std::sync::Arc<()>,
    mount: std::sync::Arc<()>,
    generation: std::sync::Arc<()>,
    key: (EpochRecordKind, [u8; 32]),
    plain: Zeroizing<Vec<u8>>,
    server: u64,
    document: LogicalDocument,
    size: u64,
    digest: blake3::Hash,
    references: bool,
}

/// A detached validation's result, still carrying the bindings of the cursor that parked it.
pub struct ValidatedEpochRecord {
    identity: std::sync::Arc<()>,
    mount: std::sync::Arc<()>,
    generation: std::sync::Arc<()>,
    key: (EpochRecordKind, [u8; 32]),
    server: u64,
    document: LogicalDocument,
    size: u64,
    digest: blake3::Hash,
    body: ValidatedRecordBody,
}

impl ParkedEpochRecord {
    /// The detached stage. Runs the same pure validation the inline path runs, with no store, no
    /// device key and no MLS secret, so it needs no custody and can happen in another visit.
    pub fn validate(self) -> Result<ValidatedEpochRecord, AppError> {
        let body = self.run()?;
        Ok(ValidatedEpochRecord {
            identity: self.identity,
            mount: self.mount,
            generation: self.generation,
            key: self.key,
            server: self.server,
            document: self.document,
            size: self.size,
            digest: self.digest,
            body,
        })
    }

    /// The validation itself, borrowing rather than consuming.
    ///
    /// Split out so [`Self::validate`] and the design 13.7 measurement cannot drift apart. A
    /// measurement that timed its own copy of this would stop measuring the production path the
    /// first time one of them changed, and nothing would say so.
    fn run(&self) -> Result<ValidatedRecordBody, AppError> {
        // The scope is the first field of the authenticated plaintext; re-deriving it is a slice
        // read, not a second validation, and keeps the parked body a single owned buffer.
        let mut decoder = Decoder::new(&self.plain);
        let scope = decoder.get_bytes().map_err(invalid)?;
        validate_record_body(
            self.key.0,
            &self.plain,
            scope,
            self.server,
            &self.document,
            self.size,
            self.references,
        )
    }
}

#[cfg(test)]
impl ParkedEpochRecord {
    /// The three facts `validation_fits` classifies on, for a measurement that has to group its
    /// results by them. The cursor reads its own fields directly and needs no accessor.
    pub(in crate::store) fn classification(&self) -> (EpochRecordKind, u64, bool) {
        (self.key.0, self.size, self.references)
    }

    /// Run the detached validation again, without consuming the record.
    ///
    /// [`Self::validate`] takes `self`, which is right for production: a parked body is validated
    /// once and installed. Design 13.7 needs its *cost*, and the only clock available has
    /// millisecond resolution - `scripts/check-no-ambient.sh` forbids `Instant::now` everywhere
    /// under `crates/`, test code included. One record's validation can round to zero against
    /// that, so the measurement times a batch of repetitions and divides, which needs an input it
    /// can run more than once.
    pub(in crate::store) fn revalidate(&self) -> Result<(), AppError> {
        self.run().map(|_| ())
    }
}

impl std::fmt::Debug for ParkedEpochRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No plaintext, no record identifier, no document.
        f.debug_struct("ParkedEpochRecord")
            .field("family", &self.key.0)
            .field("bytes", &self.size)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for ValidatedEpochRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ValidatedEpochRecord")
            .field("family", &self.key.0)
            .field("bytes", &self.size)
            .finish_non_exhaustive()
    }
}

/// A cursor plus custody of the store, for callers that complete a scan in one visit and so do
/// not need to park it. Every method delegates to the cursor; there is one implementation.
pub struct EpochStorageScan<'a> {
    store: &'a mut ServerStore,
    cursor: EpochStorageCursor,
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
            .field("progress", &self.cursor.progress)
            .field("failed", &self.cursor.failed)
            .finish_non_exhaustive()
    }
}

/// Same omissions as the scan's: no vault paths, no record identifiers, and not the captured
/// generation, which is an allocation identity and means nothing outside this process.
impl std::fmt::Debug for EpochStorageCursor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochStorageCursor")
            .field("coverage", &self.inventory.coverage)
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
        scan.cursor.entry_limit = 1024;
        scan.cursor.record_limit = 64;
        scan.cursor.byte_limit = STUDIO_RECEIVE_READ_BYTES;
        scan.cursor.cold_byte_limit = Some(STUDIO_RECEIVE_COLD_BYTES);
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
            cursor: EpochStorageCursor {
                directory,
                inventory,
                progress: EpochStorageScanProgress::default(),
                failed: false,
                entry_limit: MAX_DIRECTORY_ENTRIES,
                record_limit: MAX_ACCOUNTED_RECORDS,
                byte_limit: MAX_AUTHENTICATED_BYTES,
                cold_byte_limit: None,
                references: None,
                // Captured before the first entry is read, so any mutation concurrent with even
                // the earliest part of this scan invalidates it.
                generation: self.inventory_generation.clone(),
                identity: std::sync::Arc::new(()),
                mount: self.registry_mount(),
                parked: None,
                awaiting: None,
            },
            store: self,
        })
    }

    /// Begin a scan that can be parked between visits. The cursor owns its progress; custody of
    /// the store is taken again by each `step_epoch_storage_scan` call and released on return.
    pub fn begin_epoch_storage_scan(
        &mut self,
        coverage: EpochInventoryCoverage,
    ) -> Result<EpochStorageCursor, AppError> {
        Ok(self.scan_epoch_files(coverage)?.cursor)
    }

    /// Resume a parked cursor. Invalidation is checked **before** any traversal or record work,
    /// so an overtaken cursor costs nothing to reject.
    ///
    /// `budget` bounds continuous custody. With `None` the cursor never parks, which is right
    /// for a caller that holds the store for the whole scan anyway and could not use a parked
    /// body if it got one. With `Some`, a record whose validation cannot be conservatively
    /// bounded within what remains is parked instead of run: take it with
    /// [`Self::take_parked_record`], validate it detached, and install it before stepping again.
    ///
    /// The deadline is checked between entry-processing units, with a one-entry minimum for a
    /// nonzero step allowance; individual entry work is not preempted. A visit that begins
    /// already past its deadline therefore still processes one entry, and a visit can overrun
    /// its budget by the cost of whichever entry was in flight when it expired. Callers that
    /// need a hard ceiling must bound `steps` as well.
    pub fn step_epoch_storage_scan(
        &mut self,
        cursor: &mut EpochStorageCursor,
        steps: usize,
        budget: Option<(&dyn catcoms_rt::Clock, u64)>,
    ) -> Result<EpochStorageScanProgress, AppError> {
        cursor
            .step_with(self, steps, budget)
            .map_err(CursorFailure::into_error)
    }

    /// Begin a commit attempt's inventory work, with its own restart budget.
    ///
    /// Prefer this over driving a bare cursor: it is what keeps an overtaken scan from either
    /// failing the commit outright or restarting forever.
    pub fn begin_epoch_inventory_job(
        &mut self,
        coverage: EpochInventoryCoverage,
    ) -> Result<EpochInventoryJob, AppError> {
        Ok(EpochInventoryJob {
            cursor: self.begin_epoch_storage_scan(coverage)?,
            coverage,
            restarts: 0,
        })
    }

    /// Step an inventory job, absorbing an invalidation into a restart while the budget allows.
    pub fn step_epoch_inventory_job(
        &mut self,
        job: &mut EpochInventoryJob,
        steps: usize,
        budget: Option<(&dyn catcoms_rt::Clock, u64)>,
    ) -> Result<EpochInventoryStep, AppError> {
        match job.cursor.step_with(self, steps, budget) {
            Ok(progress) => Ok(if job.cursor.parked.is_some() {
                EpochInventoryStep::Parked
            } else {
                EpochInventoryStep::Stepped(progress)
            }),
            Err(CursorFailure::Invalidated(_)) => self.restart_job(job),
            Err(other) => Err(other.into_error()),
        }
    }

    /// Take the record this job's cursor parked.
    pub fn take_parked_job_record(&self, job: &mut EpochInventoryJob) -> Option<ParkedEpochRecord> {
        job.cursor.parked.take()
    }

    /// Install a detached validation into this job's cursor. An invalidation here is absorbed
    /// as a restart like any other: a write landing while a validation ran detached is the
    /// expected case, not an error the caller should have to classify.
    pub fn install_validated_job_record(
        &mut self,
        job: &mut EpochInventoryJob,
        validated: ValidatedEpochRecord,
    ) -> Result<EpochInventoryStep, AppError> {
        match job.cursor.install_validated(self, validated) {
            Ok(()) => Ok(EpochInventoryStep::Stepped(job.cursor.progress)),
            Err(CursorFailure::Invalidated(_)) => self.restart_job(job),
            Err(other) => Err(other.into_error()),
        }
    }

    /// Finish, or restart once more, or report that the vault will not hold still.
    pub fn finish_epoch_inventory_job(
        &mut self,
        job: EpochInventoryJob,
    ) -> Result<EpochInventoryOutcome, AppError> {
        // Destructured rather than moved out of a still-live job, so the success path opens no
        // directory it does not need. Replacing the cursor in place cost a `read_dir` on every
        // finish, and could turn a completed scan into an error if that open happened to fail.
        let EpochInventoryJob {
            cursor,
            coverage,
            restarts,
        } = job;
        match cursor.finish_with(self) {
            Ok(inventory) => Ok(EpochInventoryOutcome::Complete(Box::new(inventory))),
            Err(CursorFailure::Invalidated(_)) => {
                if restarts >= MAX_INVENTORY_RESTARTS {
                    return Ok(EpochInventoryOutcome::Unstable);
                }
                Ok(EpochInventoryOutcome::Restarted(Box::new(
                    EpochInventoryJob {
                        cursor: self.begin_epoch_storage_scan(coverage)?,
                        coverage,
                        restarts: restarts + 1,
                    },
                )))
            }
            Err(other) => Err(other.into_error()),
        }
    }

    /// Replace an overtaken cursor, or report the budget spent.
    fn restart_job(&mut self, job: &mut EpochInventoryJob) -> Result<EpochInventoryStep, AppError> {
        if job.restarts >= MAX_INVENTORY_RESTARTS {
            return Ok(EpochInventoryStep::Unstable);
        }
        job.restarts += 1;
        job.cursor = self.begin_epoch_storage_scan(job.coverage)?;
        Ok(EpochInventoryStep::Restarted)
    }

    /// Turn a cursor into a reference scan, before it has visited anything.
    ///
    /// Reference scans are not driven by [`EpochInventoryJob`], which is for budget inventories
    /// and whose outcome type has nothing to say about a CID set. They drive the cursor.
    ///
    /// `#[cfg(test)]` because no production path drives an *owned* cursor yet: `creative_pinned_cids`
    /// reaches the same two cursor methods through [`EpochStorageScan`], which holds the borrow.
    /// So this is a second entry point to production machinery, not production machinery of its
    /// own, and shipping it ungated would ship dead code. It ungates when the runtime adopts the
    /// cursor - the six call sites listed in the status ledger.
    #[cfg(test)]
    pub(in crate::store) fn collect_cursor_creative_references(
        &self,
        cursor: &mut EpochStorageCursor,
    ) -> Result<(), AppError> {
        cursor.collect_creative_references(self)
    }

    /// Finish a reference scan, installing its protection. `#[cfg(test)]` for the reason given on
    /// [`Self::collect_cursor_creative_references`].
    #[cfg(test)]
    pub(in crate::store) fn finish_cursor_creative_references(
        &self,
        cursor: EpochStorageCursor,
    ) -> Result<super::super::creative_references::CreativeReferences, AppError> {
        cursor.finish_creative_references(self)
    }

    /// Take the record this cursor parked, if any. The cursor refuses to step again until the
    /// result is installed, so this is the only way forward rather than an optional check.
    pub fn take_parked_record(&self, cursor: &mut EpochStorageCursor) -> Option<ParkedEpochRecord> {
        cursor.parked.take()
    }

    /// Install a detached validation. Rechecks the cursor identity, the mount, the record id and
    /// the inventory generation; a failure here is ordinary, not exceptional, because a write
    /// may well have landed while the validation ran outside custody.
    pub fn install_validated_record(
        &mut self,
        cursor: &mut EpochStorageCursor,
        validated: ValidatedEpochRecord,
    ) -> Result<(), AppError> {
        cursor
            .install_validated(self, validated)
            .map_err(CursorFailure::into_error)
    }

    /// Consume a cursor and issue its inventory. Rechecks invalidation: a scan that completed
    /// its traversal before a write landed must not hand out a stale inventory.
    pub fn finish_epoch_storage_scan(
        &self,
        cursor: EpochStorageCursor,
    ) -> Result<EpochStorageInventory, AppError> {
        cursor.finish_with(self).map_err(CursorFailure::into_error)
    }
}

impl EpochStorageCursor {
    /// Refuse if a five-family mutation has landed since this cursor captured its token.
    ///
    /// Called before resuming work and again before issuing an inventory. Both matter: the
    /// first keeps an overtaken cursor from spending custody on results it must discard, and
    /// the second catches a write that lands after the traversal reached EOF but before the
    /// caller consumed the inventory.
    fn check_not_invalidated(&self, store: &ServerStore) -> Result<(), CursorFailure> {
        if !std::sync::Arc::ptr_eq(&self.generation, &store.inventory_generation) {
            return Err(CursorFailure::Invalidated(invalid(
                "epoch storage inventory was invalidated by a concurrent record mutation",
            )));
        }
        Ok(())
    }

    /// Opt-in only: ordinary budget scans must NOT replace a transient pre-publication hold.
    pub(in crate::store) fn collect_creative_references(
        &mut self,
        store: &ServerStore,
    ) -> Result<(), AppError> {
        if self.progress.visited_entries != 0
            || self.coverage()
                != EpochInventoryCoverage::RecoveryOwnerReceiptsIntentsRegistryAndStudio
        {
            return Err(invalid("reference scan requires fresh full inventory"));
        }
        let generation = store
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
        store: &ServerStore,
    ) -> Result<super::super::creative_references::CreativeReferences, AppError> {
        // A reference scan installs protection, so a stale one is worse than a stale budget.
        // Not job-driven, so its typed failure collapses to the public error here.
        self.check_not_invalidated(store)
            .map_err(CursorFailure::into_error)?;
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
        store
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
    /// Merge one validated record into the inventory.
    ///
    /// Shared by the inline path and by the install of a detached validation, so a parked record
    /// cannot accumulate different accounting from an unparked one. That is the whole reason it
    /// is a method rather than two copies.
    #[allow(clippy::too_many_arguments)]
    fn install_body(
        &mut self,
        store: &mut ServerStore,
        key: (EpochRecordKind, [u8; 32]),
        server: u64,
        document: LogicalDocument,
        size: u64,
        digest: blake3::Hash,
        body: ValidatedRecordBody,
    ) -> Result<(), AppError> {
        let (family, hash) = key;
        if let Some(collected) = self.references.as_mut() {
            collected.refs.add(&document.server_id, body.cids)?;
            if let Some(target) = body.metadata {
                collected
                    .metadata
                    .insert((server, document.clone()), target);
            }
            if let Some(target) = body.required {
                collected
                    .required
                    .insert((server, document.clone()), target);
            }
        }
        if matches!(family, EpochRecordKind::Registry | EpochRecordKind::Studio) {
            // Even an explicit reference scan may warm pure validation metadata, but a later
            // reference scan must still enumerate the actual CIDs.
            store
                .inventory_cache
                .put((family, hash), size, digest, body.record);
        }
        if self
            .inventory
            .records
            .insert(
                (family, hash),
                EpochStorageInventoryEntry {
                    kind: family,
                    server,
                    document,
                    record: body.record,
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
        Ok(())
    }

    /// Install a detached validation, rechecking every binding it was parked with.
    fn install_validated(
        &mut self,
        store: &mut ServerStore,
        validated: ValidatedEpochRecord,
    ) -> Result<(), CursorFailure> {
        // All four, and the generation last so its message is the one a caller sees when a write
        // landed while the validation was detached - which is the expected outcome, not a bug.
        //
        // Only that last one is an invalidation. A result from another scan, another mount or
        // another record is a caller error, and restarting would hide it.
        let fault = |message: &'static str| CursorFailure::Fault(invalid(message));
        if !std::sync::Arc::ptr_eq(&self.identity, &validated.identity) {
            return Err(fault("validated record belongs to a different scan"));
        }
        if !std::sync::Arc::ptr_eq(&self.mount, &validated.mount) {
            return Err(fault(
                "validated record was produced under a different mount",
            ));
        }
        match self.awaiting {
            Some(key) if key == validated.key => {}
            // Left set, so the caller can still install the right one.
            Some(_) => return Err(fault("validated record is not the one this scan parked")),
            None => return Err(fault("this scan has no parked record to install")),
        }
        self.check_not_invalidated(store)?;
        if !std::sync::Arc::ptr_eq(&self.generation, &validated.generation) {
            return Err(CursorFailure::Invalidated(invalid(
                "validated record was produced before a concurrent record mutation",
            )));
        }
        self.install_body(
            store,
            validated.key,
            validated.server,
            validated.document,
            validated.size,
            validated.digest,
            validated.body,
        )
        .map_err(CursorFailure::Fault)?;
        // Only now: a failed merge leaves the record outstanding rather than quietly dropped.
        self.awaiting = None;
        self.parked = None;
        Ok(())
    }

    fn step_with(
        &mut self,
        store: &mut ServerStore,
        steps: usize,
        budget: Option<(&dyn catcoms_rt::Clock, u64)>,
    ) -> Result<EpochStorageScanProgress, CursorFailure> {
        // Before resuming any expensive work, not after. A cursor that has been overtaken must
        // not pay for traversal or record authentication it is going to throw away.
        self.check_not_invalidated(store)?;
        if self.awaiting.is_some() {
            return Err(CursorFailure::Fault(invalid(
                "this scan has a parked record; validate and install it before stepping",
            )));
        }
        // One clock read per visit, at entry. The deadline is then a fixed target rather than a
        // moving one, and the classifier below compares against what is left of it.
        let deadline = budget.map(|(clock, ms)| (clock, clock.monotonic_ms().saturating_add(ms)));
        // Everything the scan itself can fail at is a fault: a corrupt record, an exhausted rail
        // or a vanished file is not fixed by starting again.
        self.guarded_step(store, steps, deadline, Self::step_inner)
            .map_err(CursorFailure::Fault)
    }

    #[allow(clippy::type_complexity)]
    fn guarded_step(
        &mut self,
        store: &mut ServerStore,
        steps: usize,
        deadline: Option<(&dyn catcoms_rt::Clock, u64)>,
        work: impl FnOnce(
            &mut Self,
            &mut ServerStore,
            usize,
            Option<(&dyn catcoms_rt::Clock, u64)>,
        ) -> Result<EpochStorageScanProgress, AppError>,
    ) -> Result<EpochStorageScanProgress, AppError> {
        if self.failed {
            return Err(invalid(
                "epoch storage inventory scan failed; restart required",
            ));
        }
        // Poison BEFORE traversing/parsing: even a caught parser panic must not let a caller
        // resume beyond the offending entry and declare an incomplete inventory complete.
        self.failed = true;
        let result = work(self, store, steps, deadline);
        if result.is_ok() {
            self.failed = false;
        }
        result
    }

    fn step_inner(
        &mut self,
        store: &mut ServerStore,
        steps: usize,
        deadline: Option<(&dyn catcoms_rt::Clock, u64)>,
    ) -> Result<EpochStorageScanProgress, AppError> {
        if self.progress.complete {
            return Ok(self.progress);
        }
        // The budget bounds the visit, not only the choice to detach a validator.
        //
        // An earlier version sampled the clock once and used it solely for that choice, so a
        // step over ignored filenames, orphans or cache hits ran the whole requested entry count
        // no matter how long it had already taken: with no fresh validation to classify, an
        // expired budget had no effect at all. `steps` bounded it; `budget_ms` did not.
        //
        // A single filesystem operation is not preemptible through this API, so this is not a
        // measured latency ceiling. It is the weaker and honest guarantee the design asks for:
        // once the deadline is known to have passed, no further unit of work begins.
        let expired = |deadline: Option<(&dyn catcoms_rt::Clock, u64)>| {
            deadline.is_some_and(|(clock, at)| clock.monotonic_ms() >= at)
        };
        for processed in 0..steps {
            // Never on the first iteration: a visit that begins already past its deadline must
            // still make one unit of progress, or a cursor could be starved forever by a budget
            // it can never satisfy.
            if processed > 0 && expired(deadline) {
                return Ok(self.progress);
            }
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
                        && store
                            .inventory_cache
                            .candidate((family, hash), metadata.len());
                    if !candidate {
                        self.check_cold_bytes(metadata.len())?;
                    }
                    let AuthenticatedEpochFileBytes {
                        plain,
                        physical_bytes: size,
                    } = match family {
                        EpochRecordKind::Recovery => store.read_epoch_recovery_plain(&entry.path()),
                        EpochRecordKind::OwnerReceipts => {
                            store.read_epoch_owner_plain(&entry.path())
                        }
                        EpochRecordKind::Intents => store.read_epoch_intent_plain(&entry.path()),
                        EpochRecordKind::DraftArchive => {
                            store.read_epoch_draft_archive_plain(&entry.path())
                        }
                        EpochRecordKind::Registry => store.read_epoch_registry_plain(&entry.path()),
                        EpochRecordKind::Studio => store.read_epoch_studio_plain(&entry.path()),
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
                        .then(|| store.inventory_cache.get((family, hash), size, digest))
                        .flatten();
                    let body = if let Some(record) = cached {
                        // A cache hit is cheap by construction, so it is never a parking
                        // candidate: the expensive thing is exactly what the cache avoided.
                        self.progress.reused_records += 1;
                        ValidatedRecordBody::accounting_only(record)
                    } else {
                        self.check_cold_bytes(size)?;
                        self.progress.uncached_bytes = self
                            .progress
                            .uncached_bytes
                            .checked_add(size)
                            .ok_or_else(|| {
                                invalid("epoch storage inventory cold byte limit reached")
                            })?;
                        // Classify BEFORE invoking. Measuring after a long synchronous validator
                        // has returned would not have bounded anything, so the decision is made
                        // on facts known now: family, authenticated size, and whether this scan
                        // is collecting references.
                        //
                        // The remaining budget is sampled here rather than at step entry,
                        // because the read and authentication of this record's body have already
                        // consumed some of it. Classifying against a stale figure would admit a
                        // validator on the strength of time that was spent getting to it.
                        let remaining_ms =
                            deadline.map(|(clock, at)| at.saturating_sub(clock.monotonic_ms()));
                        if let Some(remaining) = remaining_ms {
                            if !validation_fits(family, size, self.references.is_some(), remaining)
                            {
                                self.awaiting = Some((family, hash));
                                self.parked = Some(ParkedEpochRecord {
                                    identity: self.identity.clone(),
                                    mount: self.mount.clone(),
                                    generation: self.generation.clone(),
                                    key: (family, hash),
                                    plain,
                                    server,
                                    document,
                                    size,
                                    digest,
                                    references: self.references.is_some(),
                                });
                                return Ok(self.progress);
                            }
                        }
                        validate_record_body(
                            family,
                            &plain,
                            scope,
                            server,
                            &document,
                            size,
                            self.references.is_some(),
                        )?
                    };
                    self.install_body(store, (family, hash), server, document, size, digest, body)?;
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
    fn finish_with(mut self, store: &ServerStore) -> Result<EpochStorageInventory, CursorFailure> {
        // Again before issuing, not only before resuming: a write can land after the traversal
        // reaches EOF and before the caller consumes the result.
        self.check_not_invalidated(store)?;
        if self.failed || !self.progress.complete {
            return Err(CursorFailure::Fault(invalid(
                "epoch storage inventory is incomplete",
            )));
        }
        // Stamp the budget-ownership token at issue, not at begin.
        //
        // `studio_generation` rotates on a budget mint and on budget *entry* - bookkeeping that
        // touches no record and therefore, correctly, does not rotate `inventory_generation`. A
        // cursor that spanned one of those would otherwise survive every invalidation check and
        // then hand back an inventory `studio_storage_budget` refuses as stale, which is the
        // negative property inverted: harmless activity would not kill the cursor but would
        // still waste it.
        //
        // This is not a continuing lease. It stamps the moment of issue, under the same custody
        // that just confirmed the disk state is current; an already-issued inventory still ages
        // exactly as before, and the next mint or entry still invalidates it.
        //
        // `intent_generation` is deliberately *not* refreshed. Every site that rotates it takes
        // the mutation guard immediately afterwards, so it cannot move without
        // `inventory_generation` moving too - and if that ever stopped being true, refreshing
        // here would silently mask the staleness instead of refusing.
        self.inventory.studio_generation = store.studio_generation.clone();
        Ok(self.inventory)
    }
}

/// The single-visit form. Each method holds custody only for the call it delegates, which is
/// the same discipline the parked form uses; the difference is that this one keeps the borrow
/// alive between calls so the caller cannot release custody even if it wanted to.
impl EpochStorageScan<'_> {
    pub(in crate::store) fn collect_creative_references(&mut self) -> Result<(), AppError> {
        self.cursor.collect_creative_references(self.store)
    }
    pub(in crate::store) fn finish_creative_references(
        self,
    ) -> Result<super::super::creative_references::CreativeReferences, AppError> {
        self.cursor.finish_creative_references(self.store)
    }
    /// Fixed coverage for this job, including before it completes.
    pub fn coverage(&self) -> EpochInventoryCoverage {
        self.cursor.coverage()
    }
    /// Visit at most 64 directory entries and authenticate at most one bounded record. Schedule
    /// another step when `complete` is false. An error permanently poisons this scan, including
    /// errors caused by local corruption, disappearance, aliases or resource exhaustion.
    pub fn step(&mut self) -> Result<EpochStorageScanProgress, AppError> {
        // No budget: this form holds the store across every step, so parking a body would
        // strand the scan rather than shorten any custody hold.
        self.cursor
            .step_with(self.store, ENTRIES_PER_STEP, None)
            .map_err(CursorFailure::into_error)
    }
    pub fn finish(self) -> Result<EpochStorageInventory, AppError> {
        self.cursor
            .finish_with(self.store)
            .map_err(CursorFailure::into_error)
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

/// How many times one commit attempt may start an invalidated scan again before giving up.
///
/// The bound exists because restarting is only progress if writes eventually stop. A vault under
/// continuous write pressure would otherwise restart forever, holding the commit open and doing
/// no useful work. Exhausting it is a signal to back off, **not** permission to fall back to a
/// single-visit unbounded scan: that would trade the custody bound for the liveness problem.
pub const MAX_INVENTORY_RESTARTS: usize = 3;

/// One commit attempt's inventory work: a cursor plus the restart budget that bounds it.
///
/// The restart count belongs here rather than in the cursor because a restart *replaces* the
/// cursor. Keeping it in the thing that outlives the cursor is what makes the budget a property
/// of the attempt, as the design specifies, instead of resetting every time a scan is retried.
/// Budget inventories only, deliberately.
///
/// A reference scan does not return an inventory; it installs deletion protection and yields
/// the reference set. An earlier version of this type accepted a `references` flag and, on
/// finishing one, returned `Complete` carrying an **empty** inventory - which a caller could
/// have built a zero budget from without anything having failed. Reference scans therefore
/// drive the cursor directly until their own runtime adoption gives them an outcome type that
/// says what they actually produce.
pub struct EpochInventoryJob {
    cursor: EpochStorageCursor,
    coverage: EpochInventoryCoverage,
    restarts: usize,
}

/// What one step of an inventory job did.
#[derive(Debug)]
pub enum EpochInventoryStep {
    /// Ordinary progress.
    Stepped(EpochStorageScanProgress),
    /// A record is parked. Take it, validate it detached, install it, then step again.
    Parked,
    /// A write overtook the scan and it has been started again. Nothing is lost except the work
    /// already done; the restart budget is one smaller.
    Restarted,
    /// The restart budget is spent. Back off and try the whole attempt later.
    Unstable,
}

/// What finishing an inventory job produced.
pub enum EpochInventoryOutcome {
    Complete(Box<EpochStorageInventory>),
    /// Overtaken between the last step and the inventory being issued. The job comes back with
    /// a fresh cursor and one less restart.
    Restarted(Box<EpochInventoryJob>),
    /// The restart budget is spent.
    Unstable,
}

impl std::fmt::Debug for EpochInventoryJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EpochInventoryJob")
            .field("coverage", &self.coverage)
            .field("restarts", &self.restarts)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for EpochInventoryOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Complete(_) => f.write_str("Complete(..)"),
            Self::Restarted(_) => f.write_str("Restarted(..)"),
            Self::Unstable => f.write_str("Unstable"),
        }
    }
}

/// Why a cursor operation failed, as a type rather than as a message.
///
/// Only an invalidation may be absorbed by a restart. Every other failure is the scan's own and
/// must surface: retrying a corrupt record or an exhausted rail would spend the budget hiding a
/// fault that is not going to fix itself, and would report "try again later" for something no
/// amount of waiting repairs.
///
/// This was a substring match on the error text. That recognised English rather than the event
/// "this cursor's generation no longer matches": an unrelated `Invalid` whose message happened
/// to contain the word would have consumed a restart and eventually become `Unstable`, and
/// rewording the genuine message would have silently disabled automatic restart. The public
/// surface is unchanged - `AppError` is not widened - because the distinction only needs to
/// survive as far as the job layer, which is inside this module.
enum CursorFailure {
    Invalidated(AppError),
    Fault(AppError),
}

impl CursorFailure {
    fn into_error(self) -> AppError {
        match self {
            Self::Invalidated(error) | Self::Fault(error) => error,
        }
    }
}

/// Everything one record's typed validation produces.
///
/// Returned rather than applied, so the validation can run somewhere other than where its
/// results are merged. The reference facts are keyed by the caller, which already holds the
/// record's authenticated `(server, document)`.
pub(in crate::store) struct ValidatedRecordBody {
    record: StorageRecord,
    cids: std::collections::BTreeSet<[u8; 32]>,
    /// A Studio handoff metadata target this record *supplies*.
    metadata: Option<catcoms_replication::studio::StudioTarget>,
    /// A Studio handoff metadata target this record *depends on*.
    required: Option<catcoms_replication::studio::StudioTarget>,
}

impl ValidatedRecordBody {
    /// A cache hit: the record's typed validation already happened, and its reference facts were
    /// deliberately not cached, which is why a reference scan never consults the cache.
    fn accounting_only(record: StorageRecord) -> Self {
        Self {
            record,
            cids: std::collections::BTreeSet::new(),
            metadata: None,
            required: None,
        }
    }
}

/// Whether one record's typed validation can be **conservatively** bounded within `remaining_ms`.
///
/// Design 9.2 is explicit that `budget_ms` is not a preemption mechanism: checking elapsed time
/// after a long synchronous validator has returned does not enforce a boundary. So the decision
/// is made from what is known before the call - family, authenticated physical size, and whether
/// references are being collected - and **where cost cannot be conservatively classified, the
/// default is to detach**.
///
/// Measurement 13.7 is what will make classification possible: the largest single-record step for
/// each family at its accepted ceiling, with and without reference collection. Until those
/// figures exist, nothing can be conservatively classified and this returns false for every
/// fresh validation. That is deliberately pessimistic - it detaches records that would have been
/// cheap - and it is the direction the design names, because the failure mode of detaching a
/// cheap record is an extra visit, while the failure mode of inlining an expensive one is an
/// unbounded custody hold.
fn validation_fits(
    _family: EpochRecordKind,
    _size: u64,
    _references: bool,
    _remaining_ms: u64,
) -> bool {
    false
}

/// The typed validation of one authenticated record body, for every family.
///
/// **Pure by construction**: authenticated plaintext and its already-checked identity in,
/// accounting and reference facts out. No `ServerStore`, no device key, no MLS secret, no I/O.
/// That is not an incidental property - it is what lets C-3 run this stage detached, in a visit
/// other than the one that read the bytes, which is the whole basis for bounding custody when a
/// single cold Registry or Studio record's validation would otherwise dominate a step.
///
/// Extracting it also makes the claim checkable rather than asserted: if this signature ever
/// needs the store back, detachment has stopped being sound and the compiler says so.
#[allow(clippy::too_many_arguments)]
pub(in crate::store) fn validate_record_body(
    family: EpochRecordKind,
    plain: &[u8],
    scope: &[u8],
    server: u64,
    document: &LogicalDocument,
    size: u64,
    references: bool,
) -> Result<ValidatedRecordBody, AppError> {
    let mut cids = std::collections::BTreeSet::new();
    let mut metadata = None;
    let mut required = None;
    let record = match family {
        EpochRecordKind::Recovery => {
            let state = EpochRecoveryState::decode(plain, scope, document)?;
            if references {
                cids.extend(super::super::creative_references::recovery_cids(
                    document, &state,
                )?);
            }
            recovery_record(scope, state.footprint(size)?)
        }
        EpochRecordKind::OwnerReceipts => {
            epoch_owner::EpochOwnerReceiptState::decode(plain, scope, document)?;
            epoch_owner::storage_record(server, document, scope, size)?
        }
        EpochRecordKind::Intents => {
            // Accounting and reference collection need the ledger, the handoff metadata target
            // and the overlay's seed-derived base CIDs, never its replayed projection. A
            // retained branch would otherwise be fully reconstructed on every five-family scan
            // in the vault, including scans for unrelated documents.
            let state = epoch_intents::EpochIntentState::decode_structural(plain, scope, document)?;
            if references {
                if let Some(held) = state.handoff_metadata() {
                    metadata = Some(held.target());
                }
                if let Some(overlay) = state.overlay() {
                    cids.extend(overlay.base_blob_cids().map_err(invalid)?);
                }
                if matches!(
                    document.doc_type,
                    DocType::StudioIndex | DocType::StudioObject
                ) {
                    for (_, intent) in state.pending() {
                        cids.extend(
                            catcoms_replication::studio::operation_blob_cid(&intent.operation)
                                .map_err(invalid)?,
                        );
                    }
                } else if document.doc_type != DocType::DocRegistry {
                    return Err(invalid("unsupported creative reference family"));
                }
            }
            epoch_intents::storage_record(server, document, scope, size)?
        }
        EpochRecordKind::DraftArchive => {
            // The seam authenticates, names and accounts an archive without knowing what is
            // inside it. A reference scan is the one case that cannot proceed on that basis: its
            // deletion-protection set would omit the archive's CIDs and archived pixels would be
            // reclaimed, destroying the preservation guarantee. The seam therefore failed closed
            // for EVERY archive until the collector existed.
            //
            // That refusal is NARROWED here, not removed. `inventory_references` still refuses an
            // archive whose bounded canonical payload will not decode, which is the rule every
            // other family applies to a corrupt record; what it no longer refuses is an archive
            // it can read.
            //
            // An accounting-only scan still reads and authenticates the file, as it does for
            // every family; what it does not do is decode or interpret the archive payload. That
            // distinction matters at an I/O boundary and is not the same as touching nothing.
            if references {
                let inspected = epoch_draft_archive::inventory_references(
                    plain, server, document, scope, size,
                )?;
                cids.extend(inspected.cids);
                inspected.record
            } else {
                epoch_draft_archive::storage_record(server, document, scope, size)?
            }
        }
        EpochRecordKind::Registry => {
            super::super::epoch_registry::inventory_record(plain, server, document, scope, size)?
        }
        EpochRecordKind::Studio => {
            if references {
                let inspected = super::super::epoch_studio::inventory_references(
                    plain, server, document, scope, size,
                )?;
                cids.extend(inspected.cids);
                required = inspected.required_metadata;
                inspected.record
            } else {
                super::super::epoch_studio::inventory_record(plain, server, document, scope, size)?
            }
        }
    };
    Ok(ValidatedRecordBody {
        record,
        cids,
        metadata,
        required,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use catcoms_replication::RecoveryReason;
    use catcoms_rt::ManualClock;
    use rand_chacha::ChaCha20Rng;
    use rand_core::SeedableRng;

    mod performance;

    fn open(path: &Path) -> ServerStore {
        ServerStore::open(path, b"inventory-test", &mut ChaCha20Rng::seed_from_u64(1)).unwrap()
    }

    fn document(group: &[u8], key: &[u8]) -> LogicalDocument {
        LogicalDocument::new(group.to_vec(), DocType::StudioObject, key.to_vec()).unwrap()
    }

    /// I-4's two directions, which are not symmetric.
    ///
    /// Rotation must happen before the first possible I/O of any operation that can touch an
    /// inventoried record, and must **not** happen for anything else. Over-rotation costs a
    /// rescan; under-rotation silently keeps a captured inventory valid across a mutation it
    /// never saw, which is the only unsafe direction and the reason the guard is a type-level
    /// prerequisite rather than a convention.
    ///
    /// The negative half is as load-bearing as the positive one: a token that rotated on reads
    /// would make a cross-visit cursor die on ordinary activity, which is precisely why
    /// `studio_generation` could not be reused for this.
    #[test]
    fn taking_the_mutation_guard_rotates_the_inventory_generation_and_reading_does_not() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());

        let before = store.inventory_generation();
        // A read of the same token is not an operation.
        let again = store.inventory_generation();
        assert!(
            std::sync::Arc::ptr_eq(&before, &again),
            "reading the token rotated it"
        );

        // The guard is the only way to reach an inventoried-family primitive, and taking it
        // rotates before the caller can perform any I/O at all.
        store.epoch_mutation_guard();
        let after = store.inventory_generation();
        assert!(
            !std::sync::Arc::ptr_eq(&before, &after),
            "taking the mutation guard did not rotate the inventory generation, so a captured \
             inventory would survive a write it never saw"
        );

        // Rotation is monotonic: dropping the guard does not restore the previous token, and a
        // second guard moves it again rather than returning to any earlier value.
        store.epoch_mutation_guard();
        let third = store.inventory_generation();
        assert!(!std::sync::Arc::ptr_eq(&after, &third));
        assert!(!std::sync::Arc::ptr_eq(&before, &third));
    }

    /// N17's per-family obligation, for the families converted so far.
    ///
    /// The guard test above proves the mechanism; this proves the mechanism is actually *on* the
    /// path a real writer takes. Those are different claims, and only the second one is what a
    /// cross-visit cursor depends on. A writer that reaches disk some other way is exactly the
    /// case a correct central guard does not cover, which is why the design pairs the choke point
    /// with an audited list rather than treating either as sufficient alone.
    ///
    /// Reads are asserted not to rotate in the same test, because a token that moved on reads
    /// would make a parked cursor die on ordinary activity rather than on a mutation.
    #[test]
    fn a_real_recovery_write_rotates_the_inventory_generation_and_a_scan_does_not() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let document = document(b"group", b"doc");

        let before = store.inventory_generation();
        stage(&mut store, 7, &document, 1);
        let after = store.inventory_generation();
        assert!(
            !std::sync::Arc::ptr_eq(&before, &after),
            "a recovery write did not rotate the inventory generation, so an inventory captured \
             before it would still be treated as current. This exercises `update_epoch_recovery`; \
             the accounted writer is a separate path, covered by its own test"
        );

        // A full scan is a read. It must leave the token alone, or nothing could ever be
        // scanned across visits.
        let quiet = store.inventory_generation();
        let _ = collect_with(&mut store, EpochInventoryCoverage::RecoveryOnly);
        assert!(
            std::sync::Arc::ptr_eq(&quiet, &store.inventory_generation()),
            "scanning rotated the token, which would make every cross-visit scan self-invalidating"
        );

        // A second write moves it again, so the token tracks writes rather than latching once.
        stage(&mut store, 7, &document, 2);
        assert!(!std::sync::Arc::ptr_eq(
            &after,
            &store.inventory_generation()
        ));
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
            let before = scan.cursor.progress;
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
            let empty = staging_candidate_for_test(&final_path, 900);
            let partial = staging_candidate_for_test(&final_path, 901);
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
        let orphan = staging_candidate_for_test(&source, 999);
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
        scan.cursor.record_limit = 0;
        assert!(scan.step().is_err());
        scan.cursor.record_limit = MAX_ACCOUNTED_RECORDS;
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
        scan.cursor.entry_limit = ENTRIES_PER_STEP;
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
        scan.cursor.byte_limit = bytes - 1;
        while scan.step().is_ok() {}
        assert_eq!(scan.cursor.progress.authenticated_bytes, 0);
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
        assert_eq!(scan.cursor.entry_limit, 1024);
        assert_eq!(scan.cursor.record_limit, 64);
        assert_eq!(scan.cursor.byte_limit, STUDIO_RECEIVE_READ_BYTES);
        assert_eq!(scan.cursor.cold_byte_limit, Some(STUDIO_RECEIVE_COLD_BYTES));
        let error = loop {
            match scan.step() {
                Err(error) => break error.to_string(),
                Ok(p) => assert!(!p.complete, "oversized registry record was skipped"),
            }
        };
        assert!(error.contains("byte limit"), "{error}");
        assert_eq!(scan.cursor.progress.authenticated_bytes, 0);
        assert!(scan.finish().is_err());
    }

    #[test]
    fn a_caught_parser_panic_cannot_skip_an_entry_and_finish_the_scan() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let mut scan = store.scan_epoch_recovery().unwrap();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scan.cursor
                .guarded_step(scan.store, ENTRIES_PER_STEP, None, |_, _, _, _| {
                    panic!("injected parser panic")
                })
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
            fs::write(staging_candidate_for_test(&destination, id), []).unwrap();
        }
        let mut scan = store.scan_epoch_recovery().unwrap();
        scan.cursor.record_limit = 2;
        assert!(scan.step().is_err());
        assert_eq!(scan.cursor.progress.orphan_files, 2);
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
            fs::write(staging_candidate_for_test(&path, id), []).unwrap();
        }
        let mut scan = store.scan_epoch_storage().unwrap();
        scan.cursor.record_limit = 2;
        assert!(scan.step().is_err());
        assert_eq!(scan.cursor.progress.orphan_files, 2);
        scan.cursor.record_limit = 3;
        assert!(scan.step().is_err());
        assert!(scan.finish().is_err());
        let mut scan = store.scan_epoch_storage().unwrap();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scan.cursor
                .guarded_step(scan.store, ENTRIES_PER_STEP, None, |_, _, _, _| {
                    panic!("combined parser panic")
                })
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

    /// C-3's central property: a cursor that released custody, was overtaken by an inventoried
    /// mutation, and then resumed must refuse.
    ///
    /// This is the claim the whole of I-4 exists to support, and it is only expressible now that
    /// the scan is an owned cursor: the previous scanner held `&mut ServerStore` for its whole
    /// life, so no write could land between its steps and the question could not be asked.
    ///
    /// Both refusal points are exercised separately, because they fail differently. A cursor
    /// overtaken mid-traversal must refuse at its next step, before paying for work it would
    /// discard. A cursor overtaken after reaching EOF has no step left to refuse at, and must
    /// refuse when it issues the inventory instead - otherwise a scan that finished traversing
    /// a moment before a write would hand out a budget that is already wrong.
    #[test]
    fn a_cursor_overtaken_by_a_write_refuses_at_its_next_step_and_at_finish() {
        let doc = document(b"group", b"cursor");

        // Overtaken mid-traversal.
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 7, &doc, 1);
        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        // One step, then custody is released: the cursor outlives the borrow, which is the
        // whole point of the change.
        let progress = store.step_epoch_storage_scan(&mut cursor, 1, None).unwrap();
        assert!(
            !progress.complete,
            "the fixture must need more than one step"
        );
        stage(&mut store, 7, &doc, 2);
        let refused = store
            .step_epoch_storage_scan(&mut cursor, 1, None)
            .unwrap_err();
        assert!(
            refused.to_string().contains("invalidated"),
            "a cursor resumed across a record mutation continued instead of refusing: {refused}"
        );
        assert!(
            store.finish_epoch_storage_scan(cursor).is_err(),
            "an invalidated cursor still issued an inventory"
        );

        // Overtaken after EOF, with nothing left to refuse at except the inventory itself.
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 7, &doc, 1);
        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        while !store
            .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, None)
            .unwrap()
            .complete
        {}
        stage(&mut store, 7, &doc, 2);
        let refused = store.finish_epoch_storage_scan(cursor).unwrap_err();
        assert!(
            refused.to_string().contains("invalidated"),
            "a completed-but-stale cursor issued its inventory: {refused}"
        );
    }

    /// The negative half, which is as load-bearing as the positive one.
    ///
    /// If reads or budget-only activity rotated the token, a parked cursor would die on ordinary
    /// traffic and the runtime would never finish a scan on a busy vault. That failure mode is
    /// exactly why `studio_generation` could not be reused here, and a cursor that survives a
    /// quiescent vault is the design's stated completion condition.
    #[test]
    fn reads_and_budget_only_activity_do_not_invalidate_a_parked_cursor() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"group", b"quiet");
        stage(&mut store, 7, &doc, 1);

        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        store.step_epoch_storage_scan(&mut cursor, 1, None).unwrap();

        // Ordinary non-mutating traffic between visits.
        store.load_epoch_recovery(7, &doc).unwrap();
        store.epoch_recovery_inventory_record(7, &doc).unwrap();
        let _ = store.inventory_generation();

        while !store
            .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, None)
            .unwrap()
            .complete
        {}
        let inventory = store
            .finish_epoch_storage_scan(cursor)
            .expect("a quiescent vault must let a parked cursor complete");
        assert_eq!(inventory.coverage(), EpochInventoryCoverage::RecoveryOnly);
    }

    /// C-3's custody bound: with a time budget, a record whose validation cannot be
    /// conservatively bounded is parked rather than run, and the scan still completes through
    /// the detached stage.
    ///
    /// The classifier currently detaches every fresh validation, because measurement 13.7 does
    /// not exist yet and 9.2's rule for that case is to default to detaching. So this also
    /// documents the pessimistic default: one record per visit until those figures land.
    #[test]
    fn a_budgeted_cursor_parks_each_record_and_completes_through_the_detached_stage() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"group", b"parked");
        stage(&mut store, 7, &doc, 1);
        let clock = ManualClock::new(0);

        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        let mut parked_bodies = 0;
        loop {
            let progress = store
                .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((&clock, 250)))
                .unwrap();
            let Some(parked) = store.take_parked_record(&mut cursor) else {
                if progress.complete {
                    break;
                }
                continue;
            };
            parked_bodies += 1;
            // A cursor holding a parked body refuses to advance: the caller cannot skip the
            // detached stage and silently drop the record from its inventory.
            assert!(
                store
                    .step_epoch_storage_scan(&mut cursor, 1, Some((&clock, 250)))
                    .is_err(),
                "a cursor stepped past its own parked record"
            );
            // The detached stage: no store, no key, no custody.
            let validated = parked.validate().unwrap();
            store
                .install_validated_record(&mut cursor, validated)
                .unwrap();
        }
        assert_eq!(
            parked_bodies, 1,
            "the fixture's one record must have been parked exactly once"
        );

        let inventory = store.finish_epoch_storage_scan(cursor).unwrap();
        assert_eq!(
            inventory.records().count(),
            1,
            "the parked record never reached the inventory, so detaching lost it"
        );

        // An unbudgeted scan of the same vault must agree, or parking changed the answer.
        let direct = collect(&mut store).unwrap();
        assert_eq!(canonical(&direct), canonical(&inventory));
    }

    /// One record, with every field kept separate.
    ///
    /// The first version packed two footprint components arithmetically as
    /// `protocol + settlement * 1_000_000`, which collides: `protocol = 1_000_000, settlement = 0`
    /// and `protocol = 0, settlement = 1` encode identically. Those are different accounting
    /// pools for the same bytes, and telling them apart is most of the point of comparing
    /// footprints at all. Ordinary tuple fields have no such failure mode.
    ///
    /// Splitting the old format string into fields also dropped `doc_type` on the first attempt,
    /// which a string of nine `|`-separated values hid because the field count still looked
    /// right. It is field three.
    type CanonicalRecord = (
        EpochRecordKind,
        u64,
        catcoms_wire::DocType,
        Vec<u8>,
        Vec<u8>,
        [u8; 32],
        [u8; 32],
        u64,
        u64,
        u64,
    );

    /// Every field a budgeted scan could have got wrong, canonicalised for comparison.
    ///
    /// Counting records was not equivalence. A detached install that preserved an entry but
    /// zeroed its footprint, or attributed it to the wrong document, would have satisfied a
    /// count comparison exactly - and this cursor's output authorises storage accounting, so
    /// those are the fields that matter most.
    fn canonical(
        inventory: &EpochStorageInventory,
    ) -> (Vec<CanonicalRecord>, Vec<(EpochRecordKind, String, u64)>) {
        let mut records: Vec<CanonicalRecord> = inventory
            .records()
            .map(|entry| {
                (
                    entry.kind,
                    entry.server,
                    entry.document.doc_type,
                    entry.document.server_id.clone(),
                    entry.document.logical_key.clone(),
                    entry.record.id,
                    entry.record.document,
                    entry.record.footprint.content,
                    entry.record.footprint.protocol,
                    entry.record.footprint.settlement,
                )
            })
            .collect();
        let mut orphans: Vec<(EpochRecordKind, String, u64)> = inventory
            .orphans()
            .map(|orphan| (orphan.kind(), orphan.name().to_owned(), orphan.bytes()))
            .collect();
        records.sort();
        orphans.sort();
        (records, orphans)
    }

    /// The scan's own counters, which the output records do not carry.
    ///
    /// `authenticated_bytes` and `uncached_bytes` are what the aggregate rails are enforced
    /// against, and nothing in the finished inventory reflects them: an install that reset,
    /// double-counted or forgot them would leave every record, footprint, orphan and per-server
    /// composition identical while the cursor's remaining allowance was wrong.
    fn counters(progress: &EpochStorageScanProgress) -> (u64, u64, usize, usize, usize) {
        (
            progress.authenticated_bytes,
            progress.uncached_bytes,
            progress.recovery_records,
            progress.orphan_files,
            progress.reused_records,
        )
    }

    /// The equivalence the previous version only claimed: a budgeted scan that parks and
    /// detaches every record produces the *same inventory*, not merely the same number of them.
    ///
    /// Multi-record and multi-family, with orphans, so that attribution, per-pool footprints and
    /// staging accounting are all in the comparison. The rails are exercised across a park too:
    /// a record is parked, validated and installed before the limit is reached, so the refusal
    /// happens on a cursor that has already been through the detached path once.
    #[test]
    fn a_budgeted_scan_produces_the_same_inventory_as_an_unbudgeted_one() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for (group, key) in [
            (&b"group-a"[..], &b"one"[..]),
            (&b"group-a"[..], &b"two"[..]),
            (&b"group-b"[..], &b"three"[..]),
        ] {
            let doc = document(group, key);
            stage(&mut store, 7, &doc, 1);
            stage(&mut store, 8, &doc, 2);
        }
        // Staging siblings of a real destination, so attribution resolves.
        let attributed = document(b"group-a", b"one");
        let final_path = store.epoch_recovery_path(&scope_bytes(7, &attributed).unwrap());
        for n in 0..3u64 {
            fs::write(
                staging_candidate_for_test(&final_path, 700 + n),
                vec![7; 16],
            )
            .unwrap();
        }

        let clock = ManualClock::new(0);
        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        let mut parked_bodies = 0;
        loop {
            let progress = store
                .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((&clock, 250)))
                .unwrap();
            if let Some(parked) = store.take_parked_record(&mut cursor) {
                parked_bodies += 1;
                let validated = parked.validate().unwrap();
                store
                    .install_validated_record(&mut cursor, validated)
                    .unwrap();
                continue;
            }
            if progress.complete {
                break;
            }
        }
        assert_eq!(
            parked_bodies, 6,
            "every record should have been parked, so this exercises the detached path for all \
             of them rather than one"
        );
        // Read the cursor's own counters once, here, because `finish` consumes it and the
        // finished inventory carries none of them.
        //
        // An earlier version tracked this in a variable reassigned at three points in the loop.
        // Clippy reported the post-install assignment as never read, which was true: the next
        // iteration's `= progress` always overwrote it first. The comparison below happened to
        // be reading the right value, but by accident of control flow rather than by
        // construction. One read of the cursor cannot be wrong in that way.
        let budgeted_progress = cursor_progress(&cursor);
        let budgeted = store.finish_epoch_storage_scan(cursor).unwrap();
        let (direct, direct_progress) = collect_with_progress(&mut store);

        let (budgeted_records, budgeted_orphans) = canonical(&budgeted);
        let (direct_records, direct_orphans) = canonical(&direct);
        assert_eq!(
            budgeted_records, direct_records,
            "parking changed a record's attribution or accounting"
        );
        assert_eq!(
            budgeted_orphans, direct_orphans,
            "parking changed staging attribution"
        );
        assert_eq!(budgeted_records.len(), 6);
        assert_eq!(budgeted_orphans.len(), 3);

        // Per-server composition, which is what a budget is actually built from. Every
        // combination the fixture creates, not a sample of them: the two that exist for each
        // group and the two that must come back empty.
        for server in [7, 8] {
            for group in [&b"group-a"[..], &b"group-b"[..]] {
                assert_eq!(
                    budgeted.records_for_server(server, group).unwrap(),
                    direct.records_for_server(server, group).unwrap(),
                    "per-server composition differs for {server}/{}",
                    String::from_utf8_lossy(group),
                );
            }
        }

        // The scan's own counters, which nothing in the finished inventory reflects. An install
        // that reset, double-counted or forgot `authenticated_bytes` would leave every
        // comparison above identical while the cursor's remaining aggregate allowance was
        // wrong - and that allowance is what the byte rail is enforced against.
        assert_eq!(
            counters(&budgeted_progress),
            counters(&direct_progress),
            "parking changed the scan's own accounting counters"
        );
        // The comparison above only says the two scans agree; it cannot say they are right.
        // This anchors the figure against a *different* code path - the per-record footprints
        // that validation produced, rather than the counters the scan accumulated - so a
        // mutation that skews both scans identically still fails here. Every record in this
        // fixture is cold, so authenticated and uncached bytes are both that sum. It is not an
        // external constant: it is still derived from this run, just not from its counters.
        let expected: u64 = budgeted
            .records()
            .map(|entry| entry.record.footprint.total().unwrap())
            .sum();
        assert_eq!(
            budgeted_progress.authenticated_bytes, expected,
            "authenticated bytes do not match the records actually authenticated"
        );
        assert_eq!(
            budgeted_progress.uncached_bytes, expected,
            "an all-cold scan reported cached work"
        );
        assert_eq!(budgeted_progress.reused_records, 0);
    }

    fn cursor_progress(cursor: &EpochStorageCursor) -> EpochStorageScanProgress {
        cursor.progress
    }

    /// `collect`, but keeping the final progress so the counters can be compared.
    fn collect_with_progress(
        store: &mut ServerStore,
    ) -> (EpochStorageInventory, EpochStorageScanProgress) {
        let mut scan = store.scan_epoch_recovery().unwrap();
        let mut progress = EpochStorageScanProgress::default();
        while !progress.complete {
            progress = scan.step().unwrap();
        }
        (scan.finish().unwrap(), progress)
    }

    /// A rail violation still refuses on a cursor that has already parked and installed once.
    ///
    /// The existing cardinality and byte-rail tests run through the unbudgeted wrapper, so none
    /// of them reaches a limit on a cursor that has been through the detached path. Parking
    /// bypasses no bound is a claim about exactly that case.
    #[test]
    fn a_rail_violation_still_refuses_after_a_record_has_been_parked_and_installed() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for key in [&b"one"[..], &b"two"[..], &b"three"[..]] {
            stage(&mut store, 7, &document(b"group", key), 1);
        }
        let clock = ManualClock::new(0);
        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        // Room for one record only; the cursor must still refuse the second, after having gone
        // through park, detached validation and install for the first.
        cursor_record_limit(&mut cursor, 1);

        let mut installed = 0;
        let refused = loop {
            match store.step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((&clock, 250)))
            {
                Ok(progress) => {
                    if let Some(parked) = store.take_parked_record(&mut cursor) {
                        let validated = parked.validate().unwrap();
                        match store.install_validated_record(&mut cursor, validated) {
                            Ok(()) => installed += 1,
                            Err(error) => break error,
                        }
                        continue;
                    }
                    assert!(!progress.complete, "the rail was never reached");
                }
                Err(error) => break error,
            }
        };
        assert!(
            installed >= 1,
            "the cursor refused before installing anything, so the refusal did not happen after \
             a detached round trip"
        );
        assert!(
            refused.to_string().contains("record limit"),
            "a parked-and-installed cursor bypassed its record rail: {refused}"
        );
        // Poisoning persists. Lifting the limit that caused the refusal proves this rather than
        // the traversal merely being unfinished: with room to continue, a healthy cursor would
        // step, and a poisoned one must still refuse.
        cursor_record_limit(&mut cursor, MAX_ACCOUNTED_RECORDS);
        let still_refused = store
            .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((&clock, 250)))
            .unwrap_err();
        assert!(
            still_refused.to_string().contains("restart required"),
            "a cursor poisoned after a detached round trip resumed once its limit was lifted: \
             {still_refused}"
        );
        assert!(
            store.finish_epoch_storage_scan(cursor).is_err(),
            "a cursor that hit its rail still issued an inventory"
        );
    }

    /// The aggregate byte rail, reached after a detached round trip.
    ///
    /// The record-count case above bounds cardinality; this bounds bytes, which is the counter a
    /// parked record's read has already spent by the time its validation is installed. The two
    /// are separate rails and a cursor could honour one while losing the other.
    #[test]
    fn the_aggregate_byte_rail_still_refuses_after_a_record_has_been_parked_and_installed() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        for key in [&b"one"[..], &b"two"[..], &b"three"[..]] {
            stage(&mut store, 7, &document(b"group", key), 1);
        }
        // What one record actually costs, measured rather than assumed.
        let sized = collect(&mut store).unwrap();
        let one = sized
            .records()
            .next()
            .unwrap()
            .record
            .footprint
            .total()
            .unwrap();

        let clock = ManualClock::new(0);
        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        // Room for one record's bytes and not the second's.
        cursor_byte_limit(&mut cursor, one + one / 2);

        let mut installed = 0;
        let refused = loop {
            match store.step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((&clock, 250)))
            {
                Ok(progress) => {
                    if let Some(parked) = store.take_parked_record(&mut cursor) {
                        let validated = parked.validate().unwrap();
                        match store.install_validated_record(&mut cursor, validated) {
                            Ok(()) => installed += 1,
                            Err(error) => break error,
                        }
                        continue;
                    }
                    assert!(!progress.complete, "the byte rail was never reached");
                }
                Err(error) => break error,
            }
        };
        assert!(
            installed >= 1,
            "the cursor refused before installing anything, so the refusal did not happen after \
             a detached round trip"
        );
        assert!(
            refused.to_string().contains("byte limit"),
            "a parked-and-installed cursor bypassed its aggregate byte rail: {refused}"
        );
    }

    fn cursor_record_limit(cursor: &mut EpochStorageCursor, limit: usize) {
        cursor.record_limit = limit;
    }

    fn cursor_byte_limit(cursor: &mut EpochStorageCursor, limit: u64) {
        cursor.byte_limit = limit;
    }

    /// A clock that advances a fixed amount on every read, so a deadline is crossed by
    /// construction rather than by hoping the work takes long enough.
    #[derive(Debug)]
    struct SteppingClock {
        ms: std::sync::atomic::AtomicU64,
        step: u64,
    }

    impl catcoms_rt::Clock for SteppingClock {
        fn now_ms(&self) -> u64 {
            self.monotonic_ms()
        }
        fn monotonic_ms(&self) -> u64 {
            self.ms
                .fetch_add(self.step, std::sync::atomic::Ordering::SeqCst)
                + self.step
        }
        fn sleep(
            &self,
            _: std::time::Duration,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
            Box::pin(std::future::ready(()))
        }
    }

    /// The supplied budget must bound the *visit*, not only the choice to detach a validator.
    ///
    /// The fixture deliberately contains nothing that needs fresh typed validation: only
    /// filenames outside the coverage and canonical staging siblings, which are counted without
    /// being read. An earlier implementation sampled the clock once and consulted it solely when
    /// classifying a validator, so with nothing to classify an expired budget had no effect and
    /// the step ran the full requested entry count. `steps` bounded it; `budget_ms` did not.
    #[test]
    fn a_supplied_deadline_stops_traversal_even_with_no_validation_to_classify() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        // Deliberately nothing that can be parked. Parking also ends a step, so a fixture
        // containing one cold record would satisfy a "stopped early" assertion whether or not
        // the deadline did anything - which is exactly how the first version of this test
        // passed against the unfixed code. These names are outside the coverage: they are
        // traversed and classified, and nothing else.
        let parent = root.path().join("servers");
        for n in 0..48 {
            fs::write(parent.join(format!("unrelated-{n}.bin")), []).unwrap();
        }

        // 200 ms per clock read against a 250 ms budget. The clock post-increments, so the entry
        // sample reads 200 and fixes the deadline at 450. The check before the second entry reads
        // 400, which is not yet past it; the check before the third reads 600, which is. Two
        // entries are processed - the one the minimum guarantees, plus one the budget still
        // allowed. That is the arithmetic, not a round number: assert it exactly.
        let clock = SteppingClock {
            ms: std::sync::atomic::AtomicU64::new(0),
            step: 200,
        };
        let mut cursor = store
            .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        let bounded = store
            .step_epoch_storage_scan(&mut cursor, MAX_DIRECTORY_ENTRIES, Some((&clock, 250)))
            .unwrap();
        assert!(!bounded.complete, "an expired budget ran the scan to EOF");
        assert_eq!(
            bounded.visited_entries, 2,
            "the deadline did not stop traversal where the clock arithmetic says it must, with \
             no validation to classify"
        );

        // Still resumable, and an unbudgeted continuation reaches the same place an unbudgeted
        // scan would: the bound yielded, it did not damage or skip anything.
        while !store
            .step_epoch_storage_scan(&mut cursor, MAX_DIRECTORY_ENTRIES, None)
            .unwrap()
            .complete
        {}
        let resumed = store.finish_epoch_storage_scan(cursor).unwrap();
        let direct = collect(&mut store).unwrap();
        assert_eq!(resumed.records().count(), direct.records().count());
        assert_eq!(resumed.orphans().count(), direct.orphans().count());
    }

    /// The four bindings a detached validation is rechecked against.
    ///
    /// Each is a different way for a result to arrive at the wrong place: another scan, another
    /// mount, another record, or after a write the validation never saw. The last is the
    /// expected one - a validation runs outside custody precisely so that writes can happen
    /// while it does - which is why it refuses rather than poisons.
    #[test]
    fn a_detached_validation_is_refused_by_the_wrong_scan_record_or_generation() {
        let doc = document(b"group", b"binding");
        let clock = ManualClock::new(0);
        let park = |store: &mut ServerStore| {
            let mut cursor = store
                .begin_epoch_storage_scan(EpochInventoryCoverage::RecoveryOnly)
                .unwrap();
            store
                .step_epoch_storage_scan(&mut cursor, ENTRIES_PER_STEP, Some((&clock, 250)))
                .unwrap();
            let parked = store
                .take_parked_record(&mut cursor)
                .expect("the classifier must park this record");
            (cursor, parked)
        };

        // Wrong scan: a second cursor over the same vault and the same record.
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 7, &doc, 1);
        let (_first, parked) = park(&mut store);
        let (mut second, _also) = park(&mut store);
        let error = store
            .install_validated_record(&mut second, parked.validate().unwrap())
            .unwrap_err();
        assert!(
            error.to_string().contains("different scan"),
            "a validation was installed into a scan that did not park it: {error}"
        );

        // Wrong record: park one, then offer it to a cursor waiting for another.
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 7, &doc, 1);
        stage(&mut store, 7, &document(b"group", b"other"), 1);
        let (mut cursor, parked) = park(&mut store);
        let mut forged = parked.validate().unwrap();
        forged.identity = cursor_identity(&cursor);
        forged.key.1 = [0xAB; 32];
        let error = store
            .install_validated_record(&mut cursor, forged)
            .unwrap_err();
        assert!(
            error.to_string().contains("not the one this scan parked"),
            "a validation was installed against the wrong record: {error}"
        );
        assert!(
            store
                .step_epoch_storage_scan(&mut cursor, 1, Some((&clock, 250)))
                .is_err(),
            "a refused install let the scan continue with its record still outstanding"
        );

        // Wrong mount: a result produced before a reopen, installed after one. The earlier
        // version of this test discussed four bindings and exercised three; this is the fourth.
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 7, &doc, 1);
        let (_stale_cursor, parked) = park(&mut store);
        let validated = parked.validate().unwrap();
        drop(store);
        let mut store = open(root.path());
        let (mut reopened, _) = park(&mut store);
        let error = store
            .install_validated_record(&mut reopened, validated)
            .unwrap_err();
        // The identity check fires first for a cursor from another mount, which is correct: it
        // is also a different scan. Vary only the mount by keeping the identity.
        assert!(
            error.to_string().contains("different scan"),
            "a validation from a previous mount was installed: {error}"
        );
        let (mut same_mount, parked) = park(&mut store);
        let mut forged = parked.validate().unwrap();
        forged.identity = cursor_identity(&same_mount);
        forged.mount = std::sync::Arc::new(());
        let error = store
            .install_validated_record(&mut same_mount, forged)
            .unwrap_err();
        assert!(
            error.to_string().contains("different mount"),
            "a validation carrying another mount's identity was installed: {error}"
        );

        // Overtaken while detached: the case the design expects to happen in normal running.
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        stage(&mut store, 7, &doc, 1);
        let (mut cursor, parked) = park(&mut store);
        let validated = parked.validate().unwrap();
        stage(&mut store, 7, &doc, 2);
        let error = store
            .install_validated_record(&mut cursor, validated)
            .unwrap_err();
        assert!(
            error.to_string().contains("invalidated"),
            "a validation produced before a write was installed after it: {error}"
        );
    }

    /// Reaches into the cursor so the wrong-record case can vary exactly one binding. Varying
    /// the record id alone is the point: if the test also changed the identity, the identity
    /// check would fire first and the record check would never be exercised.
    fn cursor_identity(cursor: &EpochStorageCursor) -> std::sync::Arc<()> {
        cursor.identity.clone()
    }

    /// The restart budget, and the thing it must not become.
    ///
    /// A vault under continuous write pressure restarts forever without a bound, holding the
    /// commit open and doing no useful work. With one, the attempt gives up and backs off. The
    /// part worth testing is the *shape* of giving up: `Unstable` is a signal to retry the whole
    /// attempt later, never a licence to fall back to a single-visit unbounded scan, because
    /// that would trade the custody bound away for the liveness problem.
    #[test]
    fn an_inventory_job_restarts_a_bounded_number_of_times_then_reports_the_vault_unstable() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"group", b"restart");
        stage(&mut store, 7, &doc, 1);

        let mut job = store
            .begin_epoch_inventory_job(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        let mut restarts = 0;
        let unstable = loop {
            // Rotate before every step, so the scan is overtaken every single time. The guard
            // is the right unit here: this test is about the restart budget's arithmetic, and
            // N17 separately proves each real writer takes the guard. Driving it with real
            // writes would also exhaust their own slots before the budget ran out.
            store.epoch_mutation_guard();
            match store
                .step_epoch_inventory_job(&mut job, ENTRIES_PER_STEP, None)
                .unwrap()
            {
                EpochInventoryStep::Restarted => restarts += 1,
                EpochInventoryStep::Unstable => break true,
                EpochInventoryStep::Stepped(_) | EpochInventoryStep::Parked => {}
            }
            assert!(
                restarts <= MAX_INVENTORY_RESTARTS,
                "the job restarted more times than its budget allows"
            );
        };
        assert!(unstable);
        assert_eq!(
            restarts, MAX_INVENTORY_RESTARTS,
            "the job gave up before spending its budget"
        );

        // Spent, and it stays spent: a further step reports the same thing rather than quietly
        // resuming, so a caller cannot loop its way back to an unbounded scan.
        assert!(matches!(
            store
                .step_epoch_inventory_job(&mut job, ENTRIES_PER_STEP, None)
                .unwrap(),
            EpochInventoryStep::Unstable
        ));
        assert!(matches!(
            store.finish_epoch_inventory_job(job).unwrap(),
            EpochInventoryOutcome::Unstable
        ));

        // And the vault is fine: once writes stop, a fresh attempt completes. `Unstable`
        // described the vault's behaviour, not damage to it.
        let mut job = store
            .begin_epoch_inventory_job(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        loop {
            match store
                .step_epoch_inventory_job(&mut job, ENTRIES_PER_STEP, None)
                .unwrap()
            {
                EpochInventoryStep::Stepped(progress) if progress.complete => break,
                EpochInventoryStep::Unstable => panic!("a quiescent vault reported unstable"),
                _ => {}
            }
        }
        let EpochInventoryOutcome::Complete(inventory) =
            store.finish_epoch_inventory_job(job).unwrap()
        else {
            panic!("a quiescent vault did not complete");
        };
        assert_eq!(inventory.records().count(), 1);
    }

    /// A restart absorbs an invalidation and nothing else.
    ///
    /// Spending the budget on a corrupt record or an exhausted rail would hide a fault that is
    /// not going to fix itself, and would report `Unstable` - "try again later" - for something
    /// no amount of waiting repairs.
    #[test]
    fn an_inventory_job_does_not_spend_restarts_on_faults_that_are_not_invalidation() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        let doc = document(b"group", b"corrupt");
        stage(&mut store, 7, &doc, 1);
        // Corrupt the record in place: authentication fails, which is not an invalidation.
        let path = store.epoch_recovery_path(&scope_bytes(7, &doc).unwrap());
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        fs::write(&path, bytes).unwrap();

        let mut job = store
            .begin_epoch_inventory_job(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        let error = loop {
            match store.step_epoch_inventory_job(&mut job, ENTRIES_PER_STEP, None) {
                Ok(EpochInventoryStep::Unstable) => {
                    panic!("a corrupt record was reported as an unstable vault")
                }
                Ok(EpochInventoryStep::Restarted) => {
                    panic!("a corrupt record spent a restart from the budget")
                }
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        assert!(
            !error.to_string().contains("invalidated"),
            "a corruption fault was misclassified as an invalidation: {error}"
        );
    }

    /// Restart eligibility is decided by `CursorFailure`, not by the error's text.
    ///
    /// It was once a substring match on the message, so a fault whose text happened to contain
    /// "invalidated" would have consumed a restart and eventually reported `Unstable` - "try
    /// again later" for something no waiting repairs - and rewording the genuine message would
    /// have silently disabled restart.
    ///
    /// There is deliberately no "fault whose message contains the word" case here, because with
    /// the typed distinction that bug is **not expressible**: no code path reads the text. What
    /// is testable is that the two classes are still told apart at all - a second, structurally
    /// different fault surfaces rather than restarting, and a real invalidation still restarts -
    /// so the fix did not simply stop absorbing everything.
    #[test]
    fn restart_eligibility_follows_the_failure_kind_and_not_the_error_text() {
        let root = tempfile::tempdir().unwrap();
        let mut store = open(root.path());
        // A name in the covered family whose body is a directory: the traversal refuses it, by
        // a different mechanism from the corruption test above.
        let parent = root.path().join("servers");
        fs::create_dir(parent.join(format!("{}.recovery", hex::encode([9u8; 32])))).unwrap();

        let mut job = store
            .begin_epoch_inventory_job(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        let outcome = loop {
            match store.step_epoch_inventory_job(&mut job, ENTRIES_PER_STEP, None) {
                Ok(EpochInventoryStep::Restarted) => {
                    panic!("a traversal fault consumed a restart from the budget")
                }
                Ok(EpochInventoryStep::Unstable) => {
                    panic!("a traversal fault was reported as an unstable vault")
                }
                Ok(EpochInventoryStep::Stepped(progress)) if progress.complete => {
                    panic!("the fixture did not produce a fault, so this proves nothing")
                }
                Ok(_) => {}
                Err(error) => break error,
            }
        };
        // The point is the classification, not the wording: this is a fault regardless of what
        // its message happens to say.
        assert!(
            outcome.to_string().contains("not a regular file"),
            "unexpected fault, so the classifier was not exercised: {outcome}"
        );

        // And the typed distinction is what decides: an actual invalidation on a fresh job still
        // restarts, proving the classifier did not simply stop absorbing everything.
        let mut job = store
            .begin_epoch_inventory_job(EpochInventoryCoverage::RecoveryOnly)
            .unwrap();
        store.epoch_mutation_guard();
        assert!(matches!(
            store
                .step_epoch_inventory_job(&mut job, ENTRIES_PER_STEP, None)
                .unwrap(),
            EpochInventoryStep::Restarted
        ));
    }
}
