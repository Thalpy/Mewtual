//! Select one checked signed header or freeze on a retained contradiction.
//! Durable review custody is independent of ordinary archive selection.
use super::{
    approval,
    header::ReceiverKey,
    index::{Indexes, Query},
    lane_head::LaneHead,
    manifest::{ManifestKey, ReviewScope, SelectedLane},
    objects::{lane_directory_name, HeldLane, HeldObject, MAX_VIEW_FILES},
    plan::{ApprovalSelection, ReviewLaneRead, ReviewPlan},
    transaction_check::TransactionScope,
    wal::{JournalKey, PreparedEffect, WriteAhead, MAX_WAL_BYTES},
    write::{finish_covered_effect, MANIFEST_STAGE, WAL_NAME},
    *,
};
use crate::store::persistence::v2_member_outbox::fs::{
    finish_exact_write, observe, observe_directory, open_exact_write, remove_listed, Presence,
    WriteMode,
};
use catcoms_replication::chat::{
    cold_index::MAX_INDEX_FRAME_BYTES,
    join_bindings::GenesisAnchorBindingData,
    owner_bodies::{ReviewStreamEntry, ReviewStreamPosition, ReviewStreamRecord},
    owner_facts::OwnerFactId,
    owner_header::{OwnerHeaderId, OwnerStateCommitment, OwnerTenureId},
    ActionId, FounderGenesisAnchorData,
};
use catcoms_rt::CryptoRngCore;

const SELECT_WORK: usize =
    8 * MAX_WAL_BYTES + 3 * MAX_VIEW_FILES * MAX_INDEX_FRAME_BYTES + 128 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextReviewHeaderStatus {
    Covered,
    Duplicate,
    WaitingFor(u64),
    Frozen,
}

/// Durable review state and the current coordinates for a recipient contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextReviewHeaderPollStatus {
    Unavailable,
    Frozen,
    Ready([u8; 32], u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeTextReviewApprovalStatus {
    Covered(ActionId),
    Duplicate(ActionId),
    WaitingForHeader(u64),
    WaitingForReplyParent(ActionId),
    Frozen,
}

pub(in crate::store) fn pinned_founder(
    anchor: &GenesisAnchorBindingData,
) -> FounderGenesisAnchorData {
    FounderGenesisAnchorData {
        group: anchor.group.clone(),
        founder: anchor.founder,
        profile: OwnerFactId::from_bytes(anchor.profile),
        admission: OwnerFactId::from_bytes(anchor.admission),
        tenure: OwnerTenureId::from_bytes(anchor.tenure),
        header: OwnerHeaderId::from_bytes(anchor.header),
        state: OwnerStateCommitment::from_bytes(anchor.state),
    }
}

impl SelectedReviewReceiver {
    pub(super) fn check_approval_header(
        &self,
        store: &ServerStore,
        binding: &GenesisAnchorBindingData,
        header: &ReviewStreamEntry,
    ) -> Result<ApprovalSelection, AppError> {
        self.recover_admitted(store)?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(SELECT_WORK)?;
        let _lease = store._session.retain_for_physical_work();
        let scope = ReviewScope::derive_data(&store.chat_vault_domain()?, binding)?;
        let key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let manifest_key = ManifestKey::new(&key, marker);
        let lane_key = ReceiverKey::new(&key, marker, scope.as_bytes());
        let indexes = Indexes::new(&key, scope);
        let mut manifest = self.read_manifest(store, &manifest_key)?;
        let mut lane = manifest
            .data
            .selected(scope)
            .map(|row| HeldLane::read(&self.directory, (scope, row), &lane_key))
            .transpose()?;
        let read = ReviewLaneRead {
            head: lane.as_ref().map(|lane| lane.head),
            key: &lane_key,
            indexes: &indexes,
        };
        let selected = read.select_approval(
            &pinned_founder(binding),
            header,
            |id| lane.as_ref().ok_or_else(denied)?.page(id),
            |id| lane.as_ref().ok_or_else(denied)?.header(id),
        )?;
        if let Some(lane) = lane.as_mut() {
            lane.require(&self.directory)?;
        }
        manifest.require_current(&self.directory)?;
        self.require(store)?;
        Ok(selected)
    }

    pub(super) fn recover_admitted(&self, store: &ServerStore) -> Result<(), AppError> {
        self.require(store)?;
        let pending = self.directory.child(WAL_NAME);
        if observe(&pending)? == Presence::Absent {
            return absent(&self.directory.child(MANIFEST_STAGE));
        }
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(SELECT_WORK)?;
        let _lease = store._session.retain_for_physical_work();
        let key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let journal = JournalKey::new(&key, marker);
        self.finish_header_transaction(store, &journal, HeldObject::read(pending, MAX_WAL_BYTES)?)
    }

    pub(super) fn select_header(
        &self,
        store: &ServerStore,
        binding: &GenesisAnchorBindingData,
        incoming: &ReviewStreamEntry,
        mut require: impl FnMut(Option<ReviewStreamPosition>) -> Result<(), AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<NativeTextReviewHeaderStatus, AppError> {
        require(None)?;
        self.require(store)?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(SELECT_WORK)?;
        let _lease = store._session.retain_for_physical_work();
        let vault = store.chat_vault_domain()?;
        let scope = ReviewScope::derive_data(&vault, binding)?;
        let vault_key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let manifest_key = ManifestKey::new(&vault_key, marker);
        let pin = pinned_founder(binding);
        let lane_key = ReceiverKey::new(&vault_key, marker, scope.as_bytes());
        let indexes = Indexes::new(&vault_key, scope);
        let journal_key = JournalKey::new(&vault_key, marker);
        let transaction = TransactionScope {
            scope,
            pin: &pin,
            manifest_key: &manifest_key,
            lane_key: &lane_key,
            indexes: &indexes,
        };
        let mut require = |position| {
            require(position)?;
            self.require(store)
        };
        let pending = self.directory.child(WAL_NAME);
        if observe(&pending)? == Presence::Present {
            self.finish_header_transaction(
                store,
                &journal_key,
                HeldObject::read(pending, MAX_WAL_BYTES)?,
            )?;
        } else {
            absent(&self.directory.child(MANIFEST_STAGE))?;
        }
        let mut manifest = self.read_manifest(store, &manifest_key)?;
        let previous = manifest.data.selected(scope);
        let mut selected = previous
            .map(|head| HeldLane::read(&self.directory, (scope, head), &lane_key))
            .transpose()?;
        let read = ReviewLaneRead {
            head: selected.as_ref().map(|lane| lane.head),
            key: &lane_key,
            indexes: &indexes,
        };
        let read_page = |id| selected.as_ref().ok_or_else(denied)?.page(id);
        let read_header = |digest| selected.as_ref().ok_or_else(denied)?.header(digest);
        require(Some(incoming.position_data()))?;
        let plan = read.plan(&pin, incoming, read_page, read_header)?;
        use NativeTextReviewHeaderStatus as Status;
        let scope_empty = previous.is_none();
        if scope_empty
            && observe_directory(&self.directory.child(&lane_directory_name(scope)))?
                != Presence::Absent
        {
            return Err(denied());
        }
        let (head, frame, updates, sibling) = match plan {
            ReviewPlan::Duplicate | ReviewPlan::Frozen | ReviewPlan::Missing(_) => {
                require(None)?;
                if let Some(lane) = selected.as_mut() {
                    lane.require(&self.directory)?;
                }
                manifest.require_current(&self.directory)?;
                return Ok(match plan {
                    ReviewPlan::Duplicate => Status::Duplicate,
                    ReviewPlan::Frozen => Status::Frozen,
                    ReviewPlan::Missing(sequence) => Status::WaitingFor(sequence),
                    _ => unreachable!("this branch has no new effect"),
                });
            }
            ReviewPlan::Append { case } => {
                let frame = lane_key.cover_header_data(&pin, incoming, rng)?;
                let row = Indexes::row(incoming, &frame)?;
                let updates = [
                    indexes.prepare(
                        Query::Ordinal(row.sequence),
                        read.head.map(|head| head.ordinal),
                        (None, row),
                        read_page,
                        rng,
                    )?,
                    indexes.prepare(
                        Query::Case(row.case),
                        read.head.map(|head| head.cases),
                        (case, row),
                        read_page,
                        rng,
                    )?,
                ];
                let head = LaneHead::next(
                    incoming,
                    selected
                        .as_ref()
                        .map(|lane| (lane.head, lane.sealed_head.bytes.as_slice())),
                    updates.each_ref().map(|update| update.root),
                )?;
                (head, frame, Some(updates), None)
            }
            ReviewPlan::Fork(older) => {
                let original = selected.as_ref().ok_or_else(denied)?;
                require(Some(older.position_data()))?;
                let frame = lane_key.cover_fork_data(&pin, &older, incoming, rng)?;
                let head = original.head.freeze(&original.sealed_head.bytes, &frame)?;
                (head, frame, None, Some(older.position_data()))
            }
        };
        let head_frame = head.seal(&lane_key, rng)?;
        let sizes = std::iter::once(frame.len()).chain(
            updates
                .iter()
                .flatten()
                .flat_map(|path| path.new.iter().map(|page| page.frame.as_bytes().len())),
        );
        let replacement = SelectedLane::successor(previous, &head_frame, sizes, head.is_frozen())?;
        manifest.data.replace(scope, previous, replacement)?;
        let next = manifest_key.seal(&manifest.data, rng)?;
        let effect = updates
            .as_ref()
            .map_or(PreparedEffect::Fork(&frame), |paths| {
                PreparedEffect::Header {
                    frame: &frame,
                    paths,
                }
            });
        let covered = journal_key.prepare(
            (scope.as_bytes(), binding),
            [&manifest.bytes, &next],
            &head_frame,
            effect,
            rng,
        )?;
        let plain = journal_key.open_candidate(&covered)?;
        let output = WriteAhead::decode(&plain)?;
        transaction.verify(selected.as_ref(), &output)?;
        let pending = self.cover_pending(scope, &journal_key, covered, || {
            require(Some(incoming.position_data()))?;
            if let Some(sibling) = sibling {
                require(Some(sibling))?;
            }
            if let Some(lane) = selected.as_mut() {
                lane.require(&self.directory)?;
            }
            manifest.require_current(&self.directory)?;
            absent(&self.directory.child(MANIFEST_STAGE))?;
            require(None)
        })?;
        #[cfg(test)]
        super::fault::cut(ReceiverFault::AdmittedHeader)?;
        drop((manifest, selected));
        self.finish_header_transaction(store, &journal_key, pending)?;
        Ok(if head.is_frozen() {
            Status::Frozen
        } else {
            Status::Covered
        })
    }

    /// Cover the exact original only behind its selected public Approve
    /// and an independently checked current source and reply ancestry.
    pub(super) fn select_approval(
        &self,
        store: &ServerStore,
        binding: &GenesisAnchorBindingData,
        raw: &[u8],
        mut require: impl FnMut() -> Result<(), AppError>,
        rng: &mut dyn CryptoRngCore,
    ) -> Result<NativeTextReviewApprovalStatus, AppError> {
        use NativeTextReviewApprovalStatus as Status;
        require()?;
        self.recover_admitted(store)?;
        let policy = store.governance_layout_policy.as_ref().ok_or_else(denied)?;
        let _fee = policy
            .current
            .archive
            .pending
            .pool
            .try_reserve(SELECT_WORK)?;
        let _lease = store._session.retain_for_physical_work();
        let scope = ReviewScope::derive_data(&store.chat_vault_domain()?, binding)?;
        let pin = pinned_founder(binding);
        let vault_key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let manifest_key = ManifestKey::new(&vault_key, marker);
        let lane_key = ReceiverKey::new(&vault_key, marker, scope.as_bytes());
        let indexes = Indexes::new(&vault_key, scope);
        let journal_key = JournalKey::new(&vault_key, marker);
        let transaction = TransactionScope {
            scope,
            pin: &pin,
            manifest_key: &manifest_key,
            lane_key: &lane_key,
            indexes: &indexes,
        };
        let record = ReviewStreamRecord::decode(&pin, raw).map_err(|_| denied())?;
        let action = record.held_data().target_data().spec().action;
        let mut manifest = self.read_manifest(store, &manifest_key)?;
        let previous = manifest.data.selected(scope);
        let mut lane = previous
            .map(|row| HeldLane::read(&self.directory, (scope, row), &lane_key))
            .transpose()?;
        let read = ReviewLaneRead {
            head: lane.as_ref().map(|lane| lane.head),
            key: &lane_key,
            indexes: &indexes,
        };
        let read_page = |id| lane.as_ref().ok_or_else(denied)?.page(id);
        let read_header = |id| lane.as_ref().ok_or_else(denied)?.header(id);
        let header =
            match read.select_approval(&pin, record.entry_data(), read_page, read_header)? {
                ApprovalSelection::WaitingFor(sequence) => {
                    if let Some(lane) = lane.as_mut() {
                        lane.require(&self.directory)?;
                    }
                    manifest.require_current(&self.directory)?;
                    require()?;
                    return Ok(Status::WaitingForHeader(sequence));
                }
                ApprovalSelection::Frozen => {
                    lane.as_mut().ok_or_else(denied)?.require(&self.directory)?;
                    manifest.require_current(&self.directory)?;
                    require()?;
                    return Ok(Status::Frozen);
                }
                ApprovalSelection::Selected { row, .. } => row,
            };
        let mut lane = lane.ok_or_else(denied)?;
        if let Some(root) = lane.head.approvals {
            if let Some(existing) = indexes.lookup_approved(root, action, |id| lane.page(id))? {
                let sealed = lane.approval(&existing.receipt)?;
                let stored = approval::open(&lane_key, &pin, &sealed)?;
                if stored.as_slice() != raw || approval::row(header, &record, &sealed)? != existing
                {
                    return Err(denied());
                }
                lane.require(&self.directory)?;
                manifest.require_current(&self.directory)?;
                require()?;
                return Ok(Status::Duplicate(action));
            }
        }
        let frame = approval::cover(&lane_key, &pin, record.entry_data(), raw, rng)?;
        let row = approval::row(header, &record, &frame)?;
        let path = indexes.prepare_approved(lane.head.approvals, row, |id| lane.page(id), rng)?;
        let next = lane
            .head
            .cover_approval(&lane.sealed_head.bytes, path.root)?;
        let head_frame = next.seal(&lane_key, rng)?;
        let sizes = std::iter::once(frame.len())
            .chain(path.new.iter().map(|page| page.frame.as_bytes().len()));
        let proposed = SelectedLane::successor(previous, &head_frame, sizes, false)?;
        manifest.data.replace(scope, previous, proposed)?;
        let next_manifest = manifest_key.seal(&manifest.data, rng)?;
        let covered = journal_key.prepare(
            (scope.as_bytes(), binding),
            [&manifest.bytes, &next_manifest],
            &head_frame,
            PreparedEffect::Approval {
                frame: &frame,
                path: &path,
            },
            rng,
        )?;
        let plain = journal_key.open_candidate(&covered)?;
        transaction.verify(Some(&lane), &WriteAhead::decode(&plain)?)?;
        let pending = self.cover_pending(scope, &journal_key, covered, || {
            self.require(store)?;
            lane.require(&self.directory)?;
            manifest.require_current(&self.directory)?;
            absent(&self.directory.child(MANIFEST_STAGE))?;
            require()
        })?;
        #[cfg(test)]
        super::fault::cut(ReceiverFault::AdmittedHeader)?;
        drop((manifest, lane));
        self.finish_header_transaction(store, &journal_key, pending)?;
        Ok(Status::Covered(action))
    }

    fn cover_pending(
        &self,
        scope: ReviewScope,
        key: &JournalKey,
        mut covered: Zeroizing<Vec<u8>>,
        mut require: impl FnMut() -> Result<(), AppError>,
    ) -> Result<HeldObject, AppError> {
        let path = self.directory.child(WAL_NAME);
        absent(&path)?;
        let lane = lane_directory_name(scope);
        let mut available = None;
        for suffix in ["scratch", "retry-scratch"] {
            let scratch = self.directory.child(&format!("{lane}.{suffix}"));
            if observe(&scratch)? == Presence::Absent {
                available = Some(scratch);
                break;
            }
            let mut previous = HeldObject::read(scratch.clone(), MAX_WAL_BYTES)?;
            // A short ciphertext cannot authenticate its original owner.
            // Preserve it and allow one scoped alternative for fresh work.
            if JournalKey::is_incomplete_scratch(&previous.bytes) {
                continue;
            }
            let plain = key.open_scratch(&previous.bytes)?;
            if &WriteAhead::decode(&plain)?.scope != scope.as_bytes() {
                return Err(denied());
            }
            require()?;
            absent(&path)?;
            previous.require()?;
            remove_listed(&scratch)?;
            drop(previous);
            self.directory.sync()?;
            absent(&scratch)?;
            available = Some(scratch);
            break;
        }
        let scratch = available.ok_or_else(denied)?;
        let mut file = open_exact_write(&scratch, WriteMode::Create)?;
        finish_exact_write(&scratch, &mut file, &covered)?;
        self.directory.sync()?;
        require_exact_held(&scratch, &mut file, &covered)?;
        require()?;
        absent(&path)?;
        self.publication.invalidate(scope)?;
        key.mark_admitted(&mut covered);
        finish_exact_write(&scratch, &mut file, &covered)?;
        self.directory.require()?;
        require_exact_held(&scratch, &mut file, &covered)?;
        absent(&path)?;
        install_catalog_stage(&scratch, &path).map_err(io)?;
        self.directory.sync()?;
        require_exact_held(&path, &mut file, &covered)?;
        let pending = HeldObject::read(path, MAX_WAL_BYTES)?;
        if pending.identity() != file.1 || pending.bytes != covered {
            return Err(denied());
        }
        Ok(pending)
    }

    fn finish_header_transaction(
        &self,
        store: &ServerStore,
        journal_key: &JournalKey,
        mut pending: HeldObject,
    ) -> Result<(), AppError> {
        pending.require()?;
        let plain = journal_key.open_admitted(&pending.bytes)?;
        let work = WriteAhead::decode(&plain)?;
        let scope = ReviewScope::derive_data(&store.chat_vault_domain()?, &work.pin)?;
        if scope.as_bytes() != &work.scope {
            return Err(denied());
        }
        self.publication.invalidate(scope)?;
        // Only the canonical write-ahead file attests that all live checks
        // passed at admission. Completing these exact metadata writes never
        // relies on the old member still being able to sign or receive.
        let pin = pinned_founder(&work.pin);
        let vault_key = Zeroizing::new(store.keys.db_key()?);
        let marker = self.marker.1.as_slice().try_into().map_err(|_| denied())?;
        let manifest_key = ManifestKey::new(&vault_key, marker);
        let lane_key = ReceiverKey::new(&vault_key, marker, scope.as_bytes());
        let indexes = Indexes::new(&vault_key, scope);
        let transaction = TransactionScope {
            scope,
            pin: &pin,
            manifest_key: &manifest_key,
            lane_key: &lane_key,
            indexes: &indexes,
        };
        let mut current = self.read_manifest(store, transaction.manifest_key)?;
        self.directory.sync()?;
        let original = transaction.manifest_key.open(work.manifests[0])?;
        let previous = original.selected(transaction.scope);
        let mut lane = previous
            .map(|head| {
                HeldLane::read(
                    &self.directory,
                    (transaction.scope, head),
                    transaction.lane_key,
                )
            })
            .transpose()?;
        let approved = transaction.verify(lane.as_ref(), &work)?;
        let head = LaneHead::open(transaction.lane_key, work.head)?;
        finish_covered_effect(
            &self.directory,
            (transaction.scope, approved, head, previous.is_none()),
            &mut current,
            pending,
            &work,
            || {
                self.require(store)?;
                if let Some(source) = lane.as_mut() {
                    source.require(&self.directory)?;
                }
                Ok(())
            },
        )
    }
}
