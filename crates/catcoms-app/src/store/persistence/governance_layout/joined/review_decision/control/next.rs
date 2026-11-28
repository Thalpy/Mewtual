//! Bind a later review request to the same native-mounted head and both
//! of its index paths before preparing the next signing operation.
use super::super::{
    selection::{
        append::{IndexedAppend, APPEND_FRAME, MAX_APPEND_BYTES},
        index::{DecisionIndexQuery, DecisionIndexes},
        HEAD_BYTES,
    },
    HEAD_NAME,
};
use super::*;
use crate::store::NativeTextInitialReviewRequest;
use catcoms_replication::chat::cold_index::{MAX_INDEX_FRAME_BYTES, MAX_INDEX_HEIGHT};
use catcoms_replication::chat::owner_header::OwnerHeaderId;
use catcoms_storage::private_chat_page::PrivateChatPageId;
use disk::indexed::{IndexedDecisionSnapshot, IndexedReviewPredecessor, IndexedReviewRequest};
use std::cell::RefCell;

const APPEND_WORK: usize = 8 * MAX_APPEND_BYTES + 4 * MAX_RECORD_BYTES + 256 * 1024;

pub(in crate::store) enum PreparedReviewBase {
    First,
    Saved(ReviewStreamEntry),
    SupersededBeforeAdmission,
    Next(Box<PreparedIndexedReviewSource>),
}

pub(in crate::store) struct PreparedIndexedReviewSource {
    pub(super) snapshot: IndexedDecisionSnapshot,
    pub(super) preflight: IndexedReviewPredecessor,
    original: HeldDirectory,
    _lease: VaultPhysicalWorkLease,
    _fee: CatalogReadReservation,
}

impl PreparedIndexedReviewSource {
    pub(in crate::store) fn next_position(
        &self,
        membership: (OwnerHeaderId, u64),
    ) -> Result<ReviewStreamPosition, AppError> {
        self.snapshot.next_position(membership)
    }

    pub(in crate::store) fn case_data(&self) -> Option<&ExactReviewDecisionBody> {
        self.preflight.case_data().map(|(_, decision)| decision)
    }

    pub(super) fn require_original(&mut self, root: &HeldDirectory) -> Result<(), AppError> {
        if !self.original.same_original(root) {
            return Err(denied());
        }
        self.original.require()?;
        self.snapshot.require_current(root)?;
        self.preflight.require_current()?;
        root.require()
    }
}

impl ServerStore {
    /// Finish only a V3 append whose exact replacement head is already
    /// selected. An old head leaves the pending append to its own request.
    pub(in crate::store) fn finish_selected_review_append(
        &self,
        pin: &FounderGenesisAnchorData,
        mut recheck: impl FnMut(ReviewStreamPosition) -> Result<(), AppError>,
    ) -> Result<(), AppError> {
        let installed = self
            .review_decision_mount
            .try_lock()
            .map_err(|_| denied())?;
        let mount = installed.as_ref().ok_or_else(denied)?;
        mount.require(self)?;
        mount.publication.invalidate()?;
        let root = &mount.directory;
        let path = root.child(disk::SELECTION_WAL);
        if observe(&path)? == Presence::Absent {
            return Ok(());
        }
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(APPEND_WORK)?;
        let _lease = self._session.retain_for_physical_work();
        let mut wal = disk::HeldDecisionFile::read(path, MAX_APPEND_BYTES)?.ok_or_else(denied)?;
        if !wal.bytes.starts_with(APPEND_FRAME) {
            return Ok(());
        }
        let vault_key = Zeroizing::new(self.keys.db_key()?);
        let marker = mount.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let key = JournalKey::new(&vault_key, marker);
        require_control_names(root)?;
        let plain = IndexedAppend::open(&key, &wal.bytes)?;
        let append = IndexedAppend::decode(&plain)?;
        let mut snapshot = IndexedDecisionSnapshot::read(root, &key, pin)?;
        let (_, head, entry) = snapshot.selected_data();
        if head == append.predecessor {
            return Ok(());
        }
        if head != append.head || mount.warm.is_some() {
            return Err(denied());
        }
        let position = entry.position_data();
        recheck(position)?;
        mount.require(self)?;
        let mut files = snapshot.cover_committed_append(root, &key, pin, &append)?;
        drop(super::selected::selected_control_files(root, |frames| {
            append.require_remaining_controls(frames)
        })?);
        wal.sync_complete()?;
        root.sync()?;
        snapshot.retire_old_index_pages(root, &key, &append, |snapshot| {
            recheck(position)?;
            mount.require(self)?;
            files.require_current(snapshot, root)?;
            require_control_names(root)?;
            wal.require()
        })?;
        super::selected::retire_selected_controls(
            root,
            wal,
            |frames| append.require_remaining_controls(frames),
            || {
                recheck(position)?;
                mount.require(self)?;
                files.require_current(&mut snapshot, root)
            },
        )
    }

    pub(in crate::store) fn prepare_review_base(
        &self,
        pin: &FounderGenesisAnchorData,
        request: &NativeTextInitialReviewRequest,
        mut recheck: impl FnMut(Option<ReviewStreamPosition>) -> Result<(), AppError>,
    ) -> Result<PreparedReviewBase, AppError> {
        let installed = self
            .review_decision_mount
            .try_lock()
            .map_err(|_| denied())?;
        let mount = installed.as_ref().ok_or_else(denied)?;
        mount.require(self)?;
        let policy = self.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let probe_fee = policy.current.archive.pending.pool.try_reserve(8192)?;
        let lease = self._session.retain_for_physical_work();
        let root = &mount.directory;
        let vault_key = Zeroizing::new(self.keys.db_key()?);
        let marker = mount.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let key = JournalKey::new(&vault_key, marker);
        require_control_names(root)?;
        let Some(head) = disk::HeldDecisionFile::read(root.child(HEAD_NAME), HEAD_BYTES)? else {
            // A legacy exact pending first transaction may have no head.
            // The original signing lane separately forbids a fresh entry.
            return Ok(PreparedReviewBase::First);
        };
        if head.bytes.as_slice() == key.initial_head()
            || DecisionHead::open(&key, &head.bytes)?.roots.is_none()
        {
            return Ok(PreparedReviewBase::First);
        }
        let fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(APPEND_WORK)?;
        drop(probe_fee);
        let mut snapshot = IndexedDecisionSnapshot::read(root, &key, pin)?;
        if snapshot.selected_data().1 != head.bytes.as_slice() {
            return Err(denied());
        }
        match snapshot.check_request(root, &key, pin, request)? {
            IndexedReviewRequest::Superseded(mut proof) => {
                // The lower proof checked a real same-case revision, an
                // exact nonce miss and absent physical controls. A retained
                // warm continuation is independent evidence and also blocks
                // retiring the caller's retry custody.
                if mount.warm.is_some() {
                    return Err(denied());
                }
                recheck(Some(proof.case_position()?))?;
                mount.require(self)?;
                snapshot.require_superseded(root, &mut proof)?;
                Ok(PreparedReviewBase::SupersededBeforeAdmission)
            }
            IndexedReviewRequest::Saved(mut lookup) => {
                let entry = lookup.record.as_ref().ok_or_else(denied)?.entry.clone();
                recheck(Some(entry.position_data()))?;
                mount.require(self)?;
                snapshot.require_current(root)?;
                lookup.require_current()?;
                Ok(PreparedReviewBase::Saved(entry))
            }
            IndexedReviewRequest::Next(preflight) => {
                let mut source = Box::new(PreparedIndexedReviewSource {
                    snapshot,
                    preflight,
                    original: root.try_clone()?,
                    _lease: lease,
                    _fee: fee,
                });
                recheck(None)?;
                mount.require(self)?;
                source.require_original(root)?;
                Ok(PreparedReviewBase::Next(source))
            }
        }
    }

    pub(in crate::store) fn with_indexed_review_decision<T>(
        &self,
        prepared: &mut PreparedIndexedReviewSource,
        use_control: impl FnOnce(
            &SelectedReviewDecision,
            &mut DecisionControl,
            &mut PreparedIndexedReviewSource,
        ) -> Result<T, AppError>,
    ) -> Result<T, AppError> {
        let mut journal = self
            .review_decision_mount
            .try_lock()
            .map_err(|_| denied())?;
        let journal = journal.as_mut().ok_or_else(denied)?;
        journal.require(self)?;
        prepared.require_original(&journal.directory)?;
        let prior = prepared.snapshot.signing_base(&journal.directory)?;
        let mut control = DecisionControl::capture(self, journal, Some(prior))?;
        let result = use_control(journal, &mut control, prepared);
        journal.warm = control.retained_warm();
        result
    }
}

impl DecisionControl {
    pub(in crate::store) fn select_later(
        &mut self,
        (store, mount): (&ServerStore, &SelectedReviewDecision),
        (spec, entry): (ReviewSigningSpec<'_>, &ReviewStreamEntry),
        prepared: &mut PreparedIndexedReviewSource,
        mut recheck: impl FnMut(ReviewStreamPosition) -> Result<(), AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<(), AppError> {
        self.cover_captured_files(store, mount)?;
        let position = entry.position_data();
        let (old, prior, tip) = prepared.snapshot.selected_data();
        let prior = Zeroizing::new(prior.to_vec());
        let tip = tip.clone();
        let old_roots = old.roots.ok_or_else(denied)?;
        let prior_case = prepared
            .preflight
            .case_data()
            .map(|(row, decision)| (row, decision.clone()));
        let digests = self.require_covered_entry(spec, entry, head_digest(&prior))?;
        let key = &self.key;
        let root = &self.data.directory;
        let slots = &mut self.data.slots;
        let reads = RefCell::new(AppendPageReads {
            records: prepared.snapshot.records_directory()?,
            pages: Vec::with_capacity(MAX_APPEND_READS),
        });
        let mut require_inputs = || {
            recheck(position)?;
            mount.require(store)?;
            require_control_names(root)?;
            require_synced_controls(root, slots)?;
            reads.borrow_mut().require()?;
            mount.require(store)
        };
        let check_wal = |append: &IndexedAppend<'_>| {
            append.require_controls(digests)?;
            append.verify_next(
                key,
                spec.pin,
                (&prior, &tip),
                (entry, spec.request_digest),
                prior_case.as_ref().map(|(row, decision)| (*row, decision)),
                |id| reads.borrow_mut().read(id),
            )
        };
        let mut wal = disk::cover_indexed_append(
            root,
            position.sequence,
            |sealed| {
                require_inputs()?;
                prepared.require_original(root)?;
                if let Some(sealed) = sealed {
                    let plain = IndexedAppend::open(key, sealed)?;
                    check_wal(&IndexedAppend::decode(&plain)?)?;
                }
                Ok(())
            },
            || {
                let record = RetainedDecisionRecord::prepare(
                    key,
                    spec.pin,
                    (entry, spec.decision, spec.held),
                    (spec.request_digest, digests[2]),
                    rng,
                )?;
                let plain = RetainedDecisionRecord::open(key, &record)?;
                let retained = RetainedDecisionRecord::decode(spec.pin, &plain)?;
                let value = DecisionIndexes::row(&retained, &record);
                let indexes = key.indexes();
                let changes = [
                    indexes.prepare_update(
                        old_roots[0],
                        DecisionIndexQuery::Intent(value.intent),
                        (None, value),
                        |id| reads.borrow_mut().read(id),
                        rng,
                    )?,
                    indexes.prepare_update(
                        old_roots[1],
                        DecisionIndexQuery::Case(value.case),
                        (prior_case.as_ref().map(|(row, _)| *row), value),
                        |id| reads.borrow_mut().read(id),
                        rng,
                    )?,
                ];
                let roots = changes.each_ref().map(|change| change.root);
                let selected =
                    DecisionHead::from_record(entry, &record, Some((old, &prior)), roots)?;
                let selected = selected.seal(key, rng)?;
                IndexedAppend::seal(key, digests, (&prior, &record, &selected), &changes, rng)
            },
        )?;
        let body = IndexedAppend::open(key, &wal.bytes)?;
        let append = IndexedAppend::decode(&body)?;
        let selected = check_wal(&append)?;
        let preflight = &mut prepared.preflight;
        prepared
            .snapshot
            .select_append(root, &mut wal, &append, selected, || {
                require_inputs()?;
                preflight.require_current()
            })
    }
}

const MAX_APPEND_READS: usize = 4 * (MAX_INDEX_HEIGHT as usize + 1);

struct AppendPageReads {
    records: HeldDirectory,
    pages: Vec<(PrivateChatPageId, disk::HeldDecisionFile)>,
}

impl AppendPageReads {
    fn read(&mut self, id: PrivateChatPageId) -> Result<Zeroizing<Vec<u8>>, AppError> {
        self.records.require()?;
        if let Some((_, original)) = self.pages.iter_mut().find(|(page, _)| *page == id) {
            original.require()?;
            return Ok(original.bytes.clone());
        }
        if self.pages.len() >= MAX_APPEND_READS {
            return Err(denied());
        }
        let path = self.records.child(&disk::index_name(id));
        let file = disk::HeldDecisionFile::read(path, MAX_INDEX_FRAME_BYTES)?.ok_or_else(denied)?;
        let bytes = file.bytes.clone();
        self.pages.push((id, file));
        Ok(bytes)
    }

    fn require(&mut self) -> Result<(), AppError> {
        self.records.require()?;
        for (_, page) in &mut self.pages {
            page.require()?;
        }
        self.records.require()
    }
}
