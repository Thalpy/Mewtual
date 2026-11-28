//! Explicit successor backing. Only a typed companion mount selects this
//! engine; presence of these files never upgrades the original ID8 owner.
//! The initial keyed Head is covered by declaration, not synthesized here.
use super::*;
use crate::store::persistence::v2_text::{
    private_index::{
        encode_private_leaf_data, PrivateIndexContextData, PrivateIndexKindData,
        PrivateIndexLeafRowData, PrivateIndexNodeFrame, PrivateIndexPathData,
        PrivateIndexSuccessorData, PrivateIndexValueData, SharedArchiveIndexValueData,
    },
    TextContextIdData, TextRootData,
};
use catcoms_replication::chat::cold_index::{
    NodePlacementData, NodeRefData, NodeSummaryData, MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT,
    MAX_INDEX_NODE_BYTES,
};
use catcoms_storage::private_chat_page::{open_private_chat_page, PreparedPrivateChatPage};

mod codec;
mod index;
mod page;
pub(in crate::store::persistence) use page::INDEXED_ACTION_PAGE_WORK_FEE;
pub(in crate::store) use page::{IndexedActionPage, IndexedActionPageError};
#[cfg(test)]
use page::{INDEXED_ACTION_PAGE_RETAINED_FEE, INDEXED_ACTION_PAGE_SCAN};
#[cfg(test)]
mod tests;
mod write;

pub(in crate::store) const INDEXED_HEAD_NAME: &str = "archive-indexed.head";
const STAGE: &str = "archive-indexed.stage";
const WAL: &str = "archive-indexed.wal";
pub(in crate::store) const INDEXED_HEAD_CAP: usize = 512;
const MAX_PATH: usize = MAX_INDEX_HEIGHT as usize + 1;
const MAX_OUTPUTS: usize = 2 * MAX_PATH + 1;
const WAL_CAP: usize = 1024
    + 2 * INDEXED_HEAD_CAP
    + MAX_SHARED_HISTORY_BUNDLE
    + MAX_PATH * (118 + 8)
    + MAX_OUTPUTS * (118 + 4 + MAX_INDEX_FRAME_BYTES);
const _: () = assert!(WAL_CAP <= MAX_FILE);
/// Existing DISK_FEE also pays legacy bounded relay/target materialization.
/// This additional class pays cumulative retained path/output/WAL requests;
/// it is reserved before prepare, never borrowed from a returned record.
pub(in crate::store::persistence) const INDEXED_DISK_FEE: usize = DISK_FEE
    + 4 * WAL_CAP
    + 4 * INDEXED_HEAD_CAP
    + 4 * MAX_PATH * (MAX_INDEX_FRAME_BYTES + MAX_INDEX_NODE_BYTES + 1024)
    + 4 * MAX_OUTPUTS * (MAX_INDEX_FRAME_BYTES + MAX_INDEX_NODE_BYTES + 1024);

/// Immutable companion binding DATA. It is selected only by the separately
/// authenticated physical catalog owner; neither this value nor file names
/// constitute a mount capability.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::store::persistence) struct IndexedHistoryLayout(
    pub(in crate::store::persistence) [u8; 32],
);

/// Same paid warm selection across temporary and installed disks. The
/// namespace owner retains the actual selected File, not just its digest.
#[derive(Clone)]
pub(in crate::store::persistence) struct IndexedHeadOwner {
    layout: IndexedHistoryLayout,
    directory: Arc<HeldDirectory>,
    head: Arc<Mutex<HeldIndexFile>>,
    gate: Arc<Mutex<u64>>,
    retiring: Arc<Mutex<Option<IndexedRetirement>>>,
}
impl IndexedHeadOwner {
    pub(in crate::store::persistence) fn require_original_file(
        &self,
        file: &File,
        frame: &[u8],
    ) -> Result<(), AppError> {
        self.directory.require()?;
        let mut original = self.head.try_lock().map_err(|_| refused())?;
        if original.file.1
            != catcoms_storage::kept::session_id(file)
                .map_err(|error| AppError::Io(error.to_string()))?
            || original.bytes.as_slice() != frame
        {
            return Err(refused());
        }
        original.require()
    }
    pub(in crate::store::persistence) fn require(&self) -> Result<(), AppError> {
        self.directory.require()?;
        self.head.try_lock().map_err(|_| refused())?.require()
    }
    pub(in crate::store::persistence) fn capture(
        directory: &HeldDirectory,
        key: &[u8; 32],
        layout: IndexedHistoryLayout,
    ) -> Result<Self, AppError> {
        let mut head = HeldIndexFile::read(directory.child(INDEXED_HEAD_NAME), INDEXED_HEAD_CAP)?;
        verify_indexed_history_head(key, layout.0, &head.bytes)?;
        head.require()?;
        directory.require()?;
        Ok(Self {
            layout,
            directory: Arc::new(directory.try_clone()?),
            head: Arc::new(Mutex::new(head)),
            gate: Arc::new(Mutex::new(0)),
            retiring: Arc::new(Mutex::new(None)),
        })
    }
    pub(in crate::store::persistence) fn matches_original(
        &self,
        other: &Self,
    ) -> Result<bool, AppError> {
        if self.layout != other.layout || !self.directory.same_original(&other.directory) {
            return Ok(false);
        }
        if Arc::ptr_eq(&self.head, &other.head) {
            return Ok(true);
        }
        let mut left = self.head.try_lock().map_err(|_| refused())?;
        let mut right = other.head.try_lock().map_err(|_| refused())?;
        left.require()?;
        right.require()?;
        Ok(left.file.1 == right.file.1 && left.bytes == right.bytes)
    }
    fn require_directory(&self, directory: &HeldDirectory) -> Result<(), AppError> {
        if !self.directory.same_original(directory) {
            return Err(refused());
        }
        self.directory.require()?;
        directory.require()
    }
}

/// Fixed cleanup custody survives a lost unlink/barrier return without
/// retaining a full WAL under the mount's small metadata payment.
struct IndexedRetirement {
    wal: Option<(File, SessionId)>,
    wal_bytes: usize,
    wal_digest: [u8; 32],
    head_digest: [u8; 32],
    row: SharedArchiveIndexValueData,
    obsolete: Vec<PrivateChatPageId>,
}

/// Exact initial ciphertext for the declaration's compound recovery source.
/// The declaring physical owner pays before calling this and retains these
/// same bytes through first-head creation and every retry.
pub(in crate::store) fn prepare_empty_indexed_history_head(
    key: &[u8; 32],
    layout: [u8; 32],
    rng: &mut dyn CryptoRngCore,
) -> Result<Zeroizing<Vec<u8>>, AppError> {
    codec::Head::empty().seal(key, IndexedHistoryLayout(layout), rng)
}

pub(in crate::store) fn verify_empty_indexed_history_head(
    key: &[u8; 32],
    layout: [u8; 32],
    bytes: &[u8],
) -> Result<(), AppError> {
    if codec::Head::open(key, IndexedHistoryLayout(layout), bytes)? != codec::Head::empty() {
        return Err(refused());
    }
    Ok(())
}

pub(in crate::store::persistence) fn verify_indexed_history_head(
    key: &[u8; 32],
    layout: [u8; 32],
    bytes: &[u8],
) -> Result<(), AppError> {
    let head = codec::Head::open(key, IndexedHistoryLayout(layout), bytes)?;
    if head.count > MAX_SHARED_HISTORY_RECORDS as u64 || head.bytes > 256 * 1024 * 1024 {
        return Err(refused());
    }
    Ok(())
}

pub(super) struct HeldIndexFile {
    name: std::path::PathBuf,
    file: (File, SessionId),
    bytes: Zeroizing<Vec<u8>>,
}
impl HeldIndexFile {
    fn read(name: std::path::PathBuf, cap: usize) -> Result<Self, AppError> {
        let (file, id, bytes) = read_bounded_held(&name, cap, false)?;
        Ok(Self {
            name,
            file: (file, id),
            bytes,
        })
    }
    pub(super) fn require(&mut self) -> Result<(), AppError> {
        require_exact_held(&self.name, &mut self.file, &self.bytes)
    }
    fn sync(&mut self) -> Result<(), AppError> {
        let mut writer = open_exact_write(&self.name, WriteMode::Existing)?;
        if writer.1 != self.file.1 {
            return Err(refused());
        }
        finish_exact_write(&self.name, &mut writer, &self.bytes)?;
        self.require()
    }
}

pub(super) struct PreparedIndexedSave {
    head: HeldIndexFile,
    dependencies: Vec<HeldIndexFile>,
}

pub(super) struct PendingIndexedWrite {
    pub(super) record: SelectedHistoryRecord,
    wal: HeldIndexFile,
    stage: Option<HeldIndexFile>,
}
impl PendingIndexedWrite {
    pub(super) fn require(&mut self, directory: &HeldDirectory) -> Result<(), AppError> {
        self.wal.require()?;
        if let Some(stage) = self.stage.as_mut() {
            stage.require()?;
        } else if observe(&directory.child(STAGE))? != Presence::Absent {
            return Err(refused());
        }
        directory.require()
    }
}

struct IndexedOriginal {
    record: SelectedHistoryRecord,
    admission: OwnerFactId,
    original: AssembledActionOriginal,
    file: HeldIndexFile,
}

impl SharedHistoryDisk {
    /// Called only by the typed ID13 physical mount. The same ID8 marker,
    /// native directory, session and independent pool remain the owner.
    pub(in crate::store::persistence) fn from_indexed_held(
        directory: HeldDirectory,
        scope: SharedHistoryScope,
        quota: SharedHistoryDiskQuota,
        pool: CriticalCatalogReadPool,
        key: &[u8; 32],
        indexed: IndexedHeadOwner,
    ) -> Result<Self, AppError> {
        indexed.require_directory(&directory)?;
        let disk = Self {
            directory,
            scope,
            quota,
            gate: indexed.gate.clone(),
            pool,
            key_binding: Self::bind_key(&scope, key),
            indexed: Some(indexed),
        };
        // Missing Head is irreplaceable even if the catalog is authentic.
        // A WAL may later settle only against this exact old/new Head.
        let (mut head, selected) = disk.indexed_head(key)?;
        selected.require_scope(scope)?;
        head.require()?;
        Ok(disk)
    }

    pub(in crate::store::persistence) fn disk_fee(&self) -> usize {
        if self.indexed.is_some() {
            INDEXED_DISK_FEE
        } else {
            DISK_FEE
        }
    }

    pub(super) fn wal_name(&self) -> &'static str {
        if self.indexed.is_some() {
            WAL
        } else {
            WAL_NAME
        }
    }

    fn indexed_head(&self, key: &[u8; 32]) -> Result<(HeldIndexFile, codec::Head), AppError> {
        let owner = self.indexed.as_ref().ok_or_else(refused)?;
        owner.require_directory(&self.directory)?;
        let mut original = owner.head.try_lock().map_err(|_| refused())?;
        original.require()?;
        let layout = owner.layout;
        self.directory.require()?;
        let mut file = HeldIndexFile {
            name: original.name.clone(),
            file: (
                original
                    .file
                    .0
                    .try_clone()
                    .map_err(|error| AppError::Io(error.to_string()))?,
                original.file.1,
            ),
            bytes: original.bytes.clone(),
        };
        let head = codec::Head::open(key, layout, &file.bytes)?;
        head.require_scope(self.scope)?;
        if head.count > self.quota.records as u64 || head.bytes > self.quota.bytes as u64 {
            return Err(refused());
        }
        file.require()?;
        Ok((file, head))
    }

    fn indexed_layout(&self) -> Result<IndexedHistoryLayout, AppError> {
        Ok(self.indexed.as_ref().ok_or_else(refused)?.layout)
    }

    fn indexed_page(
        &self,
        id: PrivateChatPageId,
        held: &mut Vec<HeldIndexFile>,
    ) -> Result<Zeroizing<Vec<u8>>, AppError> {
        if held.len() >= 4 * MAX_PATH {
            return Err(refused());
        }
        let file =
            HeldIndexFile::read(self.directory.child(&index_name(id)), MAX_INDEX_FRAME_BYTES)?;
        let bytes = file.bytes.clone();
        held.push(file);
        Ok(bytes)
    }

    pub(super) fn indexed_clean(&self) -> Result<(), AppError> {
        self.directory.require()?;
        if self
            .indexed
            .as_ref()
            .ok_or_else(refused)?
            .retiring
            .try_lock()
            .map_err(|_| refused())?
            .is_some()
        {
            return Err(refused());
        }
        if observe(&self.directory.child(WAL))? != Presence::Absent
            || observe(&self.directory.child(STAGE))? != Presence::Absent
        {
            return Err(refused());
        }
        Ok(())
    }

    pub(super) fn indexed_read_selected(
        &self,
        key: &[u8; 32],
        action: ActionId,
    ) -> Result<Option<(SelectedHistoryRecord, OwnerFactId, AssembledActionOriginal)>, AppError>
    {
        self.indexed_read_selected_checked(key, action, None, || Ok(()))
    }

    pub(super) fn indexed_read_selected_checked(
        &self,
        key: &[u8; 32],
        action: ActionId,
        expected: Option<SharedArchiveIndexValueData>,
        mut final_check: impl FnMut() -> Result<(), AppError>,
    ) -> Result<Option<(SelectedHistoryRecord, OwnerFactId, AssembledActionOriginal)>, AppError>
    {
        let (mut head_file, head) = self.indexed_head(key)?;
        let indexes = index::ActionIndex::new(key, self.indexed_layout()?, self.scope);
        let mut pages = Vec::with_capacity(MAX_PATH);
        let row = indexes.lookup(head.root, action, |id| self.indexed_page(id, &mut pages))?;
        if expected.is_some() && row != expected {
            return Err(refused());
        }
        let mut result = row.map(|row| self.indexed_original(key, row)).transpose()?;
        for page in &mut pages {
            page.require()?;
        }
        head_file.require()?;
        final_check()?;
        // The actual original File survives the caller's final blocking
        // WAL/absence/dir checks, then is checked again before returning.
        for page in &mut pages {
            page.require()?;
        }
        head_file.require()?;
        self.directory.require()?;
        if let Some(opened) = result.as_mut() {
            opened.file.require()?;
        }
        Ok(result.map(|opened| (opened.record, opened.admission, opened.original)))
    }

    fn indexed_original(
        &self,
        key: &[u8; 32],
        row: SharedArchiveIndexValueData,
    ) -> Result<IndexedOriginal, AppError> {
        if row.bundle_bytes > MAX_SHARED_HISTORY_BUNDLE as u64 {
            return Err(refused());
        }
        let mut file =
            HeldIndexFile::read(self.bundle_path(row.manifest), MAX_SHARED_HISTORY_BUNDLE)?;
        if row.bundle_bytes != file.bytes.len() as u64
            || row.bundle_digest != *blake3::hash(&file.bytes).as_bytes()
        {
            return Err(refused());
        }
        let (admission, original) =
            open_shared_history_bundle(&self.scope, key, (row.action, row.manifest), &file.bytes)?;
        if admission != row.admission
            || projection_target(original.action(), row.action) != row.target
        {
            return Err(refused());
        }
        file.require()?;
        Ok(IndexedOriginal {
            record: SelectedHistoryRecord {
                action: row.action,
                manifest: row.manifest,
            },
            admission,
            original,
            file,
        })
    }

    /// Current bounded compatibility view. The indexed successor iterator
    /// visits exact selected keys, not a directory census. The old 2,048-row
    /// bound remains in force until independent target/Admission indexes and
    /// shared paged relay replace these callers' whole-scope working state.
    pub(super) fn indexed_actions(
        &self,
        key: &[u8; 32],
        target: Option<ActionId>,
        session: &VaultSessionGuard,
        scratch: &CatalogReadReservation,
    ) -> Result<Vec<ActionId>, AppError> {
        Ok(self
            .indexed_inventory(key, session, scratch)?
            .into_iter()
            .filter(|row| target.map_or(row.target == row.action, |target| row.target == target))
            .map(|row| row.action)
            .collect())
    }

    pub(super) fn indexed_inventory(
        &self,
        key: &[u8; 32],
        session: &VaultSessionGuard,
        scratch: &CatalogReadReservation,
    ) -> Result<Vec<SharedArchiveIndexValueData>, AppError> {
        scratch.validate(&self.pool, self.disk_fee())?;
        let _lease = session.retain_for_physical_work();
        // Compatibility only: current callers still retain the old bounded
        // whole-scope vector. Every eight scanned rows now gets a FRESH
        // independent page charge. Full paged relay state is a later step.
        let mut actions = Vec::with_capacity(self.quota.records);
        let mut page = self
            .indexed_action_page(
                key,
                None,
                None,
                session,
                self.pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE)?,
            )
            .map_err(page_error)?;
        let mut scanned = 0usize;
        loop {
            scanned = scanned
                .checked_add(page.scanned_rows_data())
                .ok_or_else(refused)?;
            if scanned > self.quota.records
                || actions
                    .len()
                    .checked_add(page.rows_data().len())
                    .is_none_or(|count| count > self.quota.records)
            {
                return Err(refused());
            }
            actions.extend_from_slice(page.rows_data());
            let Some(cursor) = page.next_cursor() else {
                break;
            };
            let next = self
                .indexed_action_page(
                    key,
                    Some(cursor),
                    None,
                    session,
                    self.pool.try_reserve(INDEXED_ACTION_PAGE_WORK_FEE)?,
                )
                .map_err(page_error)?;
            page = next;
        }
        page.require_current_data(session).map_err(page_error)?;
        Ok(actions)
    }
}

fn page_error(error: IndexedActionPageError) -> AppError {
    match error {
        // Existing full-catalogue consumers retry their local maintenance
        // on a new snapshot; never return an assembled mixed-root vector.
        IndexedActionPageError::Stale => AppError::ChatV2ArchiveBusy,
        IndexedActionPageError::Unavailable(error) => error,
    }
}

// Negative-only test cut at the compatibility caller's last physical
// guard. It supplies no selected row, paid debit or successful outcome.
#[cfg(test)]
type PageFinalCut = Option<Box<dyn FnOnce()>>;
#[cfg(test)]
thread_local! { static PAGE_FINAL_CUT: std::cell::RefCell<PageFinalCut> = const { std::cell::RefCell::new(None) }; }
#[cfg(test)]
pub(super) fn run_page_final_cut() {
    let cut = PAGE_FINAL_CUT.with(|slot| slot.borrow_mut().take());
    if let Some(cut) = cut {
        cut();
    }
}

fn index_name(id: PrivateChatPageId) -> String {
    format!("n-{}.bin", hex::encode(id.as_bytes()))
}
