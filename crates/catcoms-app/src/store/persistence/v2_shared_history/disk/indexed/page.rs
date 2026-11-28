//! One paid bounded page from the exact selected action root. This is a
//! locator query, not serving or authoring authority. Every original still
//! needs its own exact bundle/signature and current-member checks.
use super::*;

pub(in crate::store::persistence) const INDEXED_ACTION_PAGE_SCAN: usize = 8;
// The existing point helper permits at most this many native node reads
// while finding one successor, including any right-boundary descents.
const READS_PER_SUCCESSOR: usize = 4 * MAX_PATH;
const PAGE_NODE_READS: usize = INDEXED_ACTION_PAGE_SCAN * READS_PER_SUCCESSOR;
// Include growth while appending a fixed leaf to the bounded base path.
const PAGE_NAMED_PATH_BYTES: usize =
    crate::store::critical_namespaces::current_layout::MAX_OPEN_BASE_PATH + 256;
const PAGE_PATH_BYTES: usize = 2 * PAGE_NAMED_PATH_BYTES;

/// Scratch is independent of archive count. Keep every exact node File
/// until the final root check, and pay the overlapping raw/copy/AEAD/path
/// buffers before entering the archive gate. This charge ends on return.
pub(in crate::store::persistence) const INDEXED_ACTION_PAGE_WORK_FEE: usize = 4 * INDEXED_HEAD_CAP
    + 4 * PAGE_PATH_BYTES
    + PAGE_NODE_READS
        * (MAX_INDEX_FRAME_BYTES + PAGE_PATH_BYTES + std::mem::size_of::<HeldIndexFile>())
    + 4 * MAX_PATH * (MAX_INDEX_FRAME_BYTES + MAX_INDEX_NODE_BYTES + 1024);

/// Only a small fixed page/cursor charge survives between calls. It also
/// pays the weak-allocation tails and a later exact Head negative check.
pub(in crate::store::persistence) const INDEXED_ACTION_PAGE_RETAINED_FEE: usize = 2
    * std::mem::size_of::<IndexedActionPage>()
    + 2 * std::mem::size_of::<IndexedActionCursor>()
    + INDEXED_ACTION_PAGE_SCAN * std::mem::size_of::<SharedArchiveIndexValueData>()
    + std::mem::size_of::<Mutex<HeldIndexFile>>()
    + std::mem::size_of::<Mutex<u64>>()
    + std::mem::size_of::<Mutex<Option<IndexedRetirement>>>()
    + std::mem::size_of::<HeldDirectory>()
    + 4 * INDEXED_HEAD_CAP
    + 2 * PAGE_PATH_BYTES
    + 256;

/// A narrowing query only. An Admission label is never an eligibility or
/// removal witness; the real provider must retain its independent verifier.
pub(in crate::store::persistence) type IndexedActionPageFilter = Option<OwnerFactId>;

#[derive(Debug)]
pub(in crate::store) enum IndexedActionPageError {
    Stale,
    /// Carries typed Busy only for a classified healthy-account pressure or
    /// contended guard. Other errors are structural; neither may be laundered
    /// into an empty page or a fresh authoritative cursor.
    Unavailable(AppError),
}
impl From<AppError> for IndexedActionPageError {
    fn from(error: AppError) -> Self {
        Self::Unavailable(error)
    }
}
type PageResult<T> = Result<T, IndexedActionPageError>;

struct PageRoot {
    gate: Weak<Mutex<u64>>,
    head: Weak<Mutex<HeldIndexFile>>,
    directory: Weak<HeldDirectory>,
    retiring: Weak<Mutex<Option<IndexedRetirement>>>,
    revision: u64,
    native: SessionId,
    digest: [u8; 32],
    root: Option<NodeRefData>,
    count: u64,
    scope: [u8; 32],
    layout: IndexedHistoryLayout,
}

/// No public constructor, Clone, decoder or owning extraction. Borrowing
/// the next cursor retains the whole paid page. A fresh invocation supplies
/// another independent work debit before advancing this historical query.
pub(in crate::store::persistence) struct IndexedActionCursor {
    selected: PageRoot,
    after: Option<ActionId>,
    scanned: u64,
    filter: IndexedActionPageFilter,
}
catcoms_rt::redacted_debug! { IndexedActionCursor; }

/// Metadata from one proven root. `rows_data` does not prove present-time
/// source authorization. The cursor and weak tails drain before its fee.
pub(in crate::store) struct IndexedActionPage {
    rows: Vec<SharedArchiveIndexValueData>,
    cursor: IndexedActionCursor,
    scanned: usize,
    more: bool,
    _payment: CatalogReadReservation,
}
catcoms_rt::redacted_debug! { IndexedActionPage; }
impl IndexedActionPage {
    /// Selected locator DATA only. The mounted relay still opens each exact
    /// signed original and checks the current author's delegation before offer.
    pub(in crate::store) fn relay_entries_data(
        &self,
    ) -> impl ExactSizeIterator<Item = ArchiveRelayEntry> + '_ {
        self.rows.iter().map(|row| ArchiveRelayEntry {
            action: row.action,
            admission: row.admission,
        })
    }

    pub(in crate::store) fn has_more_data(&self) -> bool {
        self.more
    }

    pub(in crate::store::persistence) fn rows_data(&self) -> &[SharedArchiveIndexValueData] {
        &self.rows
    }
    pub(in crate::store::persistence) fn scanned_rows_data(&self) -> usize {
        self.scanned
    }
    pub(in crate::store::persistence) fn next_cursor(&self) -> Option<&IndexedActionCursor> {
        self.more.then_some(&self.cursor)
    }
    /// Negative observation only, with no raw SDK or caller callback under
    /// the archive gate. Native publication keeps its separate Source proof.
    pub(in crate::store::persistence) fn require_current_data(
        &self,
        session: &VaultSessionGuard,
    ) -> PageResult<()> {
        // The caller supplies the SAME checked Store session; cursor
        // custody itself never keeps a physical job alive between turns.
        let _lease = session.retain_for_physical_work();
        let gate = self
            .cursor
            .selected
            .gate
            .upgrade()
            .ok_or(IndexedActionPageError::Stale)?;
        let revision = gate.try_lock().map_err(lock_error)?;
        self.cursor.selected.require_current(*revision)
    }
}

impl PageRoot {
    fn require_current(&self, revision: u64) -> PageResult<()> {
        if revision != self.revision {
            return Err(IndexedActionPageError::Stale);
        }
        let directory = self
            .directory
            .upgrade()
            .ok_or(IndexedActionPageError::Stale)?;
        let owner = self.head.upgrade().ok_or(IndexedActionPageError::Stale)?;
        let retirement = self
            .retiring
            .upgrade()
            .ok_or(IndexedActionPageError::Stale)?;
        let retiring = retirement.try_lock().map_err(lock_error)?;
        if retiring.is_some() {
            return Err(refused().into());
        }
        let mut head = owner.try_lock().map_err(lock_error)?;
        if head.file.1 != self.native || *blake3::hash(&head.bytes).as_bytes() != self.digest {
            return Err(IndexedActionPageError::Stale);
        }
        directory.require()?;
        if observe(&directory.child(WAL))? != Presence::Absent
            || observe(&directory.child(STAGE))? != Presence::Absent
        {
            return Err(refused().into());
        }
        head.require()?;
        directory.require()?;
        Ok(())
    }
}

impl SharedHistoryDisk {
    pub(in crate::store::persistence) fn require_indexed_relay_page(
        &self,
        key: &[u8; 32],
        page: &IndexedActionPage,
        session: &VaultSessionGuard,
    ) -> PageResult<()> {
        self.require_key(key)?;
        let owner = self.indexed.as_ref().ok_or_else(refused)?;
        let selected = &page.cursor.selected;
        if selected.scope != self.scope.id()
            || selected.layout != owner.layout
            || !selected.gate.ptr_eq(&Arc::downgrade(&self.gate))
            || !selected.head.ptr_eq(&Arc::downgrade(&owner.head))
            || !selected.directory.ptr_eq(&Arc::downgrade(&owner.directory))
            || !selected.retiring.ptr_eq(&Arc::downgrade(&owner.retiring))
        {
            return Err(IndexedActionPageError::Stale);
        }
        page.require_current_data(session)
    }

    /// Scan at most eight raw selected locator rows, including excluded
    /// rows. An empty filtered result can still have a progressing cursor.
    /// This never inventories the directory, settles a WAL or reads a body.
    pub(in crate::store::persistence) fn indexed_action_page(
        &self,
        key: &[u8; 32],
        cursor: Option<&IndexedActionCursor>,
        filter: IndexedActionPageFilter,
        session: &VaultSessionGuard,
        work: CatalogReadReservation,
    ) -> PageResult<IndexedActionPage> {
        work.validate(&self.pool, INDEXED_ACTION_PAGE_WORK_FEE)?;
        // Reserve the independent retained tail before any read/row/Weak
        // allocation; do not keep the large transient class on every peer.
        let retained = self
            .pool
            .try_reserve_classified(INDEXED_ACTION_PAGE_RETAINED_FEE, || {
                AppError::ChatV2ArchiveBusy
            })?;
        self.require_key(key)?;
        let owner = self.indexed.as_ref().ok_or_else(refused)?;
        // These path capacities are part of this explicit read class. A
        // longer existing vault remains usable through its older APIs.
        if owner
            .head
            .try_lock()
            .map_err(lock_error)?
            .name
            .as_os_str()
            .len()
            > PAGE_NAMED_PATH_BYTES
        {
            return Err(refused().into());
        }
        let _lease = session.retain_for_physical_work();
        let revision = self.gate.try_lock().map_err(lock_error)?;
        self.indexed_clean()?;
        if let Some(cursor) = cursor {
            if !cursor.selected.gate.ptr_eq(&Arc::downgrade(&self.gate))
                || !cursor.selected.head.ptr_eq(&Arc::downgrade(&owner.head))
                || !cursor
                    .selected
                    .directory
                    .ptr_eq(&Arc::downgrade(&owner.directory))
                || !cursor
                    .selected
                    .retiring
                    .ptr_eq(&Arc::downgrade(&owner.retiring))
                || cursor.selected.layout != owner.layout
                || cursor.selected.scope != self.scope.id()
                || cursor.filter != filter
                || cursor.selected.revision != *revision
            {
                return Err(IndexedActionPageError::Stale);
            }
        }
        // Test observation is at the actual paid Head-buffer boundary,
        // after gate/cursor refusal. Production invokes no hook/callback.
        #[cfg(test)]
        work.before_read_buffer();
        let (mut head_file, head) = self.indexed_head(key)?;
        let selected = PageRoot {
            gate: Arc::downgrade(&self.gate),
            head: Arc::downgrade(&owner.head),
            directory: Arc::downgrade(&owner.directory),
            retiring: Arc::downgrade(&owner.retiring),
            revision: *revision,
            native: head_file.file.1,
            digest: *blake3::hash(&head_file.bytes).as_bytes(),
            root: head.root,
            count: head.count,
            scope: self.scope.id(),
            layout: owner.layout,
        };
        let (mut after, mut total) = if let Some(cursor) = cursor {
            if cursor.selected.native != selected.native
                || cursor.selected.digest != selected.digest
                || cursor.selected.root != selected.root
                || cursor.selected.count != selected.count
                || cursor.scanned == 0
                || cursor.scanned >= head.count
                || cursor.after.is_none()
            {
                return Err(IndexedActionPageError::Stale);
            }
            (cursor.after, cursor.scanned)
        } else {
            (None, 0)
        };
        let count = (head.count - total).min(INDEXED_ACTION_PAGE_SCAN as u64) as usize;
        let mut rows = Vec::with_capacity(INDEXED_ACTION_PAGE_SCAN);
        let mut files = Vec::with_capacity(PAGE_NODE_READS);
        let indexes = index::ActionIndex::new(key, owner.layout, self.scope);
        for _ in 0..count {
            let mut reads = 0;
            let row = indexes
                .next(head.root, after, |id| {
                    if reads == READS_PER_SUCCESSOR || files.len() == PAGE_NODE_READS {
                        return Err(refused());
                    }
                    reads += 1;
                    let file = HeldIndexFile::read(
                        self.directory.child(&index_name(id)),
                        MAX_INDEX_FRAME_BYTES,
                    )?;
                    let bytes = file.bytes.clone();
                    files.push(file);
                    Ok(bytes)
                })?
                .ok_or_else(refused)?;
            if after.is_some_and(|previous| row.action <= previous) {
                return Err(refused().into());
            }
            // Advance by scanned keys, never by the filtered returned list.
            after = Some(row.action);
            total = total.checked_add(1).ok_or_else(refused)?;
            if filter.is_none_or(|id| row.admission == id) {
                rows.push(row);
            }
        }
        if total == head.count
            && head.count != 0
            && after.map(|id| *id.as_bytes()) != head.root.map(|root| root.summary.max)
        {
            return Err(refused().into());
        }
        for file in &mut files {
            file.require()?;
        }
        self.indexed_clean()?;
        head_file.require()?;
        self.directory.require()?;
        Ok(IndexedActionPage {
            rows,
            cursor: IndexedActionCursor {
                selected,
                after,
                scanned: total,
                filter,
            },
            scanned: count,
            more: total < head.count,
            _payment: retained,
        })
    }
}

fn lock_error<T>(error: std::sync::TryLockError<T>) -> IndexedActionPageError {
    match error {
        std::sync::TryLockError::WouldBlock => AppError::ChatV2ArchiveBusy.into(),
        std::sync::TryLockError::Poisoned(error) => {
            // The poisoned error owns the actual guard. Release it before
            // constructing a diagnostic or returning structural failure.
            drop(error);
            refused().into()
        }
    }
}
